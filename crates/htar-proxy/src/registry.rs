use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use uuid::Uuid;

/// Upstream target instance with health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamTarget {
    pub url: String,
    pub weight: u16,
    pub is_healthy: bool,
}

/// Upstream Service definition (resembling Kong Service)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub id: String,
    pub name: String,
    pub targets: Vec<UpstreamTarget>,
    pub connect_timeout_ms: u64,
    pub retries: u8,
    pub health_check_path: Option<String>,
}

/// Route definition (resembling Kong Route)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub id: String,
    pub service_id: String,
    #[serde(default)]
    pub hosts: Vec<String>,
    pub paths: Vec<String>,
    pub methods: Vec<String>,
    pub strip_path: bool,
    pub enable_cache: bool,
    pub cache_ttl_secs: Option<u64>,
    #[serde(default)]
    pub enable_auth: bool,
}

/// Dynamic Consumer & API Key mapping
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consumer {
    pub id: String,
    pub username: String,
    pub api_keys: Vec<String>,
}

/// Global In-Memory Gateway Service & Route Registry
#[derive(Clone)]
pub struct Registry {
    services: Arc<DashMap<String, Service>>,
    routes: Arc<DashMap<String, Route>>,
    consumers: Arc<DashMap<String, Consumer>>,
    lb_counters: Arc<DashMap<String, Arc<AtomicUsize>>>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            services: Arc::new(DashMap::new()),
            routes: Arc::new(DashMap::new()),
            consumers: Arc::new(DashMap::new()),
            lb_counters: Arc::new(DashMap::new()),
        }
    }

    // --- Service Operations ---
    pub fn add_service(&self, mut service: Service) -> Service {
        if service.id.is_empty() {
            service.id = Uuid::new_v4().to_string();
        }
        info!("Registered Service: {} ({})", service.name, service.id);
        self.services.insert(service.id.clone(), service.clone());
        self.lb_counters.insert(service.id.clone(), Arc::new(AtomicUsize::new(0)));
        service
    }

    pub fn get_service(&self, id: &str) -> Option<Service> {
        self.services.get(id).map(|r| r.clone())
    }

    pub fn list_services(&self) -> Vec<Service> {
        self.services.iter().map(|r| r.value().clone()).collect()
    }

    pub fn delete_service(&self, id: &str) -> bool {
        self.services.remove(id).is_some()
    }

    // --- Route Operations ---
    pub fn add_route(&self, mut route: Route) -> Route {
        if route.id.is_empty() {
            route.id = Uuid::new_v4().to_string();
        }
        self.routes.insert(route.id.clone(), route.clone());
        info!("Registered Route: {} -> Service {}", route.id, route.service_id);
        route
    }

    pub fn get_route(&self, id: &str) -> Option<Route> {
        self.routes.get(id).map(|r| r.clone())
    }

    pub fn list_routes(&self) -> Vec<Route> {
        self.routes.iter().map(|r| r.value().clone()).collect()
    }

    pub fn delete_route(&self, id: &str) -> bool {
        self.routes.remove(id).is_some()
    }

    // --- Consumer Operations ---
    pub fn add_consumer(&self, mut consumer: Consumer) -> Consumer {
        if consumer.id.is_empty() {
            consumer.id = Uuid::new_v4().to_string();
        }
        self.consumers.insert(consumer.id.clone(), consumer.clone());
        consumer
    }

    pub fn list_consumers(&self) -> Vec<Consumer> {
        self.consumers.iter().map(|r| r.value().clone()).collect()
    }

    pub fn validate_api_key(&self, api_key: &str) -> Option<Consumer> {
        self.consumers.iter().find_map(|c| {
            if c.api_keys.contains(&api_key.to_string()) {
                Some(c.value().clone())
            } else {
                None
            }
        })
    }

    pub fn delete_consumer(&self, id: &str) -> bool {
        self.consumers.remove(id).is_some()
    }

    // --- Path & Method Router with Host Support ---
    pub fn match_request(&self, request_host: &str, method: &str, path: &str) -> Option<(Route, Service)> {
        // Reserved system prefixes must never be matched by dynamic user routes
        if path.starts_with("/_htar") || path.starts_with("/admin") {
            return None;
        }

        let clean_req_host = request_host.split(':').next().unwrap_or(request_host).to_lowercase();

        let mut matched_routes: Vec<(Route, usize, bool)> = self
            .routes
            .iter()
            .filter_map(|r| {
                let route = r.value();
                let method_matches = route.methods.is_empty()
                    || route.methods.iter().any(|m| m.eq_ignore_ascii_case(method));

                let is_exact_host_match = route.hosts.iter().any(|h| h.eq_ignore_ascii_case(&clean_req_host));
                let host_matches = route.hosts.is_empty()
                    || route.hosts.iter().any(|h| h == "*")
                    || is_exact_host_match;

                let matching_prefix_len = route.paths.iter().find_map(|p| {
                    let clean_p = p.trim_end_matches('*').trim_end_matches('/');
                    if clean_p.is_empty() || p == "/" || p == "/*" {
                        Some(1)
                    } else if path == clean_p || path.starts_with(&format!("{}/", clean_p)) {
                        Some(clean_p.len())
                    } else {
                        None
                    }
                });

                if method_matches && host_matches {
                    matching_prefix_len.map(|len| (route.clone(), len, is_exact_host_match))
                } else {
                    None
                }
            })
            .collect();

        // Sort priority:
        // 1. Exact host match over wildcard host match
        // 2. Longest path prefix length descending
        // 3. Deterministic route_id ascending
        matched_routes.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.0.id.cmp(&b.0.id))
        });

        if let Some((route, _, _)) = matched_routes.first() {
            if let Some(service) = self.get_service(&route.service_id) {
                return Some((route.clone(), service));
            }
        }
        None
    }

    // --- Round-Robin Upstream Load Balancer ---
    pub fn select_upstream_target(&self, service: &Service) -> Option<String> {
        let healthy_targets: Vec<&UpstreamTarget> = service
            .targets
            .iter()
            .filter(|t| t.is_healthy)
            .collect();

        if healthy_targets.is_empty() {
            // Fallback to any target if health status isn't updated yet
            return service.targets.first().map(|t| t.url.clone());
        }

        let counter = self
            .lb_counters
            .entry(service.id.clone())
            .or_insert_with(|| Arc::new(AtomicUsize::new(0)));

        let idx = counter.fetch_add(1, Ordering::Relaxed) % healthy_targets.len();
        Some(healthy_targets[idx].url.clone())
    }

    // --- Active Health Check Task ---
    pub fn start_health_checker(registry: Arc<Self>, interval: Duration) {
        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap();

            loop {
                tokio::time::sleep(interval).await;

                // 1. Collect targets to check without holding DashMap locks long-term
                let targets_to_check: Vec<(String, String, String, String)> = registry
                    .list_services()
                    .into_iter()
                    .filter_map(|s| {
                        let path = s.health_check_path?;
                        Some(
                            s.targets
                                .into_iter()
                                .map(move |t| (s.id.clone(), s.name.clone(), t.url, path.clone())),
                        )
                    })
                    .flatten()
                    .collect();

                // 2. Perform async health check requests without holding any registry locks
                for (service_id, service_name, target_url, health_path) in targets_to_check {
                    let health_url = format!("{}{}", target_url, health_path);
                    let is_healthy = match client.get(&health_url).send().await {
                        Ok(res) => {
                            res.status().is_success()
                                || res.status().is_redirection()
                                || res.status() == reqwest::StatusCode::UNAUTHORIZED
                                || res.status() == reqwest::StatusCode::FORBIDDEN
                        }
                        Err(_) => false,
                    };

                    // 3. Briefly update status in registry
                    if let Some(mut entry) = registry.services.get_mut(&service_id) {
                        let service = entry.value_mut();
                        if let Some(target) = service.targets.iter_mut().find(|t| t.url == target_url) {
                            if is_healthy && !target.is_healthy {
                                info!("Upstream target {} for service {} is HEALTHY", target_url, service_name);
                            } else if !is_healthy && target.is_healthy {
                                warn!("Upstream target {} for service {} is UNHEALTHY (Health Check Failed)", target_url, service_name);
                            }
                            target.is_healthy = is_healthy;
                        }
                    }
                }
            }
        });
    }
}
