use crate::registry::{Registry, Route, Service, UpstreamTarget};
use futures::StreamExt;
use k8s_openapi::api::core::v1::Service as K8sService;
use k8s_openapi::api::networking::v1::Ingress as K8sIngress;
use kube::runtime::watcher::{watcher, Config};
use kube::{Api, Client};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{info, warn};

pub struct K8sController {
    registry: Arc<Registry>,
}

impl K8sController {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    pub fn start_auto_discovery(registry: Arc<Registry>) {
        tokio::spawn(async move {
            info!("Initializing Kubernetes Auto-Discovery Controller...");
            let client = match Client::try_default().await {
                Ok(c) => c,
                Err(e) => {
                    warn!(
                        "Kubernetes API Client unavailable ({}); auto-discovery disabled (running in standalone mode)",
                        e
                    );
                    return;
                }
            };

            info!("Connected to Kubernetes API. Watching Services and Ingresses for auto-registration...");

            let controller = Self::new(registry);
            let client_svc = client.clone();
            let client_ing = client.clone();

            // Task 1: Watch Annotated Kubernetes Services
            let registry_svc = controller.registry.clone();
            tokio::spawn(async move {
                let services_api: Api<K8sService> = Api::all(client_svc);
                let mut watcher_stream = watcher(services_api, Config::default()).boxed();

                while let Some(event) = watcher_stream.next().await {
                    match event {
                        Ok(kube::runtime::watcher::Event::Applied(svc)) => {
                            Self::handle_k8s_service_applied(&registry_svc, &svc);
                        }
                        Ok(kube::runtime::watcher::Event::Deleted(svc)) => {
                            Self::handle_k8s_service_deleted(&registry_svc, &svc);
                        }
                        Ok(kube::runtime::watcher::Event::Restarted(svcs)) => {
                            Self::handle_k8s_services_reconcile(&registry_svc, &svcs);
                        }
                        Err(e) => {
                            warn!("K8s Service Watcher error: {:?}", e);
                        }
                    }
                }
            });

            // Task 2: Watch Kubernetes Ingress Resources
            let registry_ing = controller.registry.clone();
            tokio::spawn(async move {
                let ingress_api: Api<K8sIngress> = Api::all(client_ing);
                let mut watcher_stream = watcher(ingress_api, Config::default()).boxed();

                while let Some(event) = watcher_stream.next().await {
                    match event {
                        Ok(kube::runtime::watcher::Event::Applied(ing)) => {
                            Self::handle_k8s_ingress_applied(&registry_ing, &ing);
                        }
                        Ok(kube::runtime::watcher::Event::Deleted(ing)) => {
                            Self::handle_k8s_ingress_deleted(&registry_ing, &ing);
                        }
                        Ok(kube::runtime::watcher::Event::Restarted(ings)) => {
                            Self::handle_k8s_ingresses_reconcile(&registry_ing, &ings);
                        }
                        Err(e) => {
                            warn!("K8s Ingress Watcher error: {:?}", e);
                        }
                    }
                }
            });
        });
    }

    fn handle_k8s_service_applied(registry: &Registry, k8s_svc: &K8sService) {
        let meta = &k8s_svc.metadata;
        let name = meta.name.clone().unwrap_or_default();
        let namespace = meta.namespace.clone().unwrap_or_else(|| "default".to_string());

        // Skip internal kubernetes API service
        if name == "kubernetes" && namespace == "default" {
            return;
        }

        let annotations = meta.annotations.as_ref();

        let enabled = annotations
            .and_then(|a| a.get("htar.gateway/enable"))
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if !enabled {
            return;
        }

        let service_id = format!("k8s_svc_{}_{}", namespace, name);

        let path = annotations
            .and_then(|a| a.get("htar.gateway/path"))
            .cloned()
            .unwrap_or_else(|| format!("/{}", name));

        let strip_path = determine_strip_path(&path, annotations);
        let health_check_path = determine_health_check_path(&path, annotations);

        let port = k8s_svc
            .spec
            .as_ref()
            .and_then(|s| s.ports.as_ref())
            .and_then(|ports| ports.first())
            .map(|p| p.port)
            .unwrap_or(8080);

        let cluster_dns_target = format!("http://{}.{}.svc.cluster.local:{}", name, namespace, port);

        info!(
            "Auto-discovering K8s Service '{}/{}' -> Target: {} | Path: {}",
            namespace, name, cluster_dns_target, path
        );

        let svc = registry.add_service(Service {
            id: service_id.clone(),
            name: format!("{}-{}", namespace, name),
            targets: vec![UpstreamTarget {
                url: cluster_dns_target,
                weight: 10,
                is_healthy: true,
            }],
            connect_timeout_ms: 5000,
            retries: 3,
            health_check_path,
        });

        let is_k8s_dashboard = name.contains("kubernetes-dashboard") || path.starts_with("/kubernetes");
        let route_id = if is_k8s_dashboard {
            "k8s_dashboard_route".to_string()
        } else {
            format!("route_{}", service_id)
        };
        let (enable_cache, cache_ttl_secs, enable_auth) = if let Some(existing) = registry.get_route(&route_id) {
            (existing.enable_cache, existing.cache_ttl_secs, existing.enable_auth)
        } else {
            (true, Some(120), is_k8s_dashboard)
        };

        registry.add_route(Route {
            id: route_id,
            service_id: svc.id,
            hosts: vec![],
            paths: vec![path],
            methods: vec![],
            strip_path,
            enable_cache,
            cache_ttl_secs,
            enable_auth,
        });
    }

    fn handle_k8s_service_deleted(registry: &Registry, k8s_svc: &K8sService) {
        let meta = &k8s_svc.metadata;
        let name = meta.name.clone().unwrap_or_default();
        let namespace = meta.namespace.clone().unwrap_or_else(|| "default".to_string());
        let service_id = format!("k8s_svc_{}_{}", namespace, name);

        if registry.delete_service(&service_id) {
            registry.delete_route(&format!("route_{}", service_id));
            info!("Auto-deregistered deleted K8s Service '{}/{}' from Registry", namespace, name);
        }
    }

    fn handle_k8s_services_reconcile(registry: &Registry, current_svcs: &[K8sService]) {
        info!("Reconciling active K8s Services with Gateway Registry...");
        let mut active_service_ids = HashSet::new();

        for svc in current_svcs {
            Self::handle_k8s_service_applied(registry, svc);
            let meta = &svc.metadata;
            let name = meta.name.clone().unwrap_or_default();
            let namespace = meta.namespace.clone().unwrap_or_else(|| "default".to_string());
            active_service_ids.insert(format!("k8s_svc_{}_{}", namespace, name));
        }

        // Purge any registered k8s_svc_* services that no longer exist in the cluster
        let existing = registry.list_services();
        for svc in existing {
            if svc.id.starts_with("k8s_svc_") && !active_service_ids.contains(&svc.id) {
                registry.delete_service(&svc.id);
                registry.delete_route(&format!("route_{}", svc.id));
                info!("Purged stale deleted K8s Service '{}' during reconciliation", svc.id);
            }
        }
    }

    fn handle_k8s_ingress_applied(registry: &Registry, ing: &K8sIngress) {
        let meta = &ing.metadata;
        let name = meta.name.clone().unwrap_or_default();
        let namespace = meta.namespace.clone().unwrap_or_else(|| "default".to_string());

        if let Some(spec) = &ing.spec {
            if let Some(rules) = &spec.rules {
                for (_rule_idx, rule) in rules.iter().enumerate() {
                    if let Some(http) = &rule.http {
                        for (_path_idx, p) in http.paths.iter().enumerate() {
                            let path_str = p.path.clone().unwrap_or_else(|| "/".to_string());
                            if let Some(backend_svc) = &p.backend.service {
                                let target_svc_name = &backend_svc.name;
                                let port_num = backend_svc
                                    .port
                                    .as_ref()
                                    .and_then(|pt| pt.number)
                                    .unwrap_or(80);

                                let cluster_target = format!(
                                    "http://{}.{}.svc.cluster.local:{}",
                                    target_svc_name, namespace, port_num
                                );

                                let svc_id = format!("k8s_ing_{}_{}_{}", namespace, name, target_svc_name);

                                info!(
                                    "Auto-registering K8s Ingress Rule '{}/{}' -> Path '{}' -> Backend '{}'",
                                    namespace, name, path_str, cluster_target
                                );

                                let strip_path = determine_strip_path(&path_str, meta.annotations.as_ref());
                                let health_check_path = determine_health_check_path(&path_str, meta.annotations.as_ref());

                                let svc = registry.add_service(Service {
                                    id: svc_id.clone(),
                                    name: format!("{}-ing-{}", namespace, target_svc_name),
                                    targets: vec![UpstreamTarget {
                                        url: cluster_target,
                                        weight: 10,
                                        is_healthy: true,
                                    }],
                                    connect_timeout_ms: 5000,
                                    retries: 3,
                                    health_check_path,
                                });

                                let host_list = rule.host.as_ref().map(|h| vec![h.clone()]).unwrap_or_default();
                                let clean_host_id = rule.host.as_deref().unwrap_or("wildcard").replace('.', "_");
                                let clean_path_id = path_str.replace('/', "_").replace('*', "");

                                let is_k8s_dashboard = target_svc_name.contains("kubernetes-dashboard") || path_str.starts_with("/kubernetes");

                                let route_id = if is_k8s_dashboard {
                                    "k8s_dashboard_route".to_string()
                                } else {
                                    format!("route_ing_{}_{}_{}", svc_id, clean_host_id, clean_path_id)
                                };

                                let (enable_cache, cache_ttl_secs, enable_auth) = if let Some(existing) = registry.get_route(&route_id) {
                                    (existing.enable_cache, existing.cache_ttl_secs, existing.enable_auth)
                                } else {
                                    (true, Some(60), is_k8s_dashboard)
                                };

                                registry.add_route(Route {
                                    id: route_id,
                                    service_id: svc.id,
                                    hosts: host_list,
                                    paths: if is_k8s_dashboard {
                                        vec![
                                            "/kubernetes".to_string(),
                                            "/kubernetes/".to_string(),
                                            "/k8s-dashboard".to_string(),
                                            "/k8s-dashboard/".to_string(),
                                            "/kubernetes-dashboard".to_string(),
                                            "/kubernetes-dashboard/".to_string(),
                                        ]
                                    } else {
                                        vec![path_str]
                                    },
                                    methods: vec![],
                                    strip_path,
                                    enable_cache,
                                    cache_ttl_secs,
                                    enable_auth,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_k8s_ingress_deleted(registry: &Registry, ing: &K8sIngress) {
        let meta = &ing.metadata;
        let name = meta.name.clone().unwrap_or_default();
        let namespace = meta.namespace.clone().unwrap_or_else(|| "default".to_string());
        
        let prefix = format!("k8s_ing_{}_{}_", namespace, name);
        let existing = registry.list_services();
        for svc in existing {
            if svc.id.starts_with(&prefix) {
                registry.delete_service(&svc.id);
                info!("Auto-deregistered deleted K8s Ingress Service '{}' from Registry", svc.id);
            }
        }

        let existing_routes = registry.list_routes();
        for r in existing_routes {
            if r.service_id.starts_with(&prefix) && r.id != "k8s_dashboard_route" {
                registry.delete_route(&r.id);
            }
        }
    }

    fn handle_k8s_ingresses_reconcile(registry: &Registry, current_ings: &[K8sIngress]) {
        info!("Reconciling active K8s Ingresses with Gateway Registry...");
        let mut active_prefixes = HashSet::new();

        for ing in current_ings {
            Self::handle_k8s_ingress_applied(registry, ing);
            let meta = &ing.metadata;
            let name = meta.name.clone().unwrap_or_default();
            let namespace = meta.namespace.clone().unwrap_or_else(|| "default".to_string());
            active_prefixes.insert(format!("k8s_ing_{}_{}_", namespace, name));
        }

        // Purge stale ingresses
        let existing = registry.list_services();
        for svc in existing {
            if svc.id.starts_with("k8s_ing_") {
                let matches = active_prefixes.iter().any(|prefix| svc.id.starts_with(prefix));
                if !matches {
                    registry.delete_service(&svc.id);
                    info!("Purged stale deleted K8s Ingress Service '{}' during reconciliation", svc.id);
                }
            }
        }
    }
}

fn determine_strip_path(path_str: &str, annotations: Option<&std::collections::BTreeMap<String, String>>) -> bool {
    if let Some(annos) = annotations {
        if let Some(val) = annos.get("htar.gateway/strip-path") {
            return val.eq_ignore_ascii_case("true");
        }
    }

    if path_str == "/" || path_str.starts_with("/v2") {
        return false;
    }

    true
}

fn determine_health_check_path(path_str: &str, annotations: Option<&std::collections::BTreeMap<String, String>>) -> Option<String> {
    if let Some(annos) = annotations {
        if let Some(val) = annos.get("htar.gateway/health-check-path") {
            return Some(val.clone());
        }
    }

    if path_str.starts_with("/v2") {
        Some("/v2/".to_string())
    } else {
        Some("/".to_string())
    }
}
