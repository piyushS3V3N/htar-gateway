use std::fs;
use std::path::Path;
use serde::{Deserialize, Serialize};
use tracing::info;
use htar_proxy::config::{CacheConfig, GatewayConfig, RouteConfig, ServerConfig};
use htar_proxy::{Consumer, PluginInstance, PluginType, Route, Service, UpstreamTarget};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationBundle {
    pub services: Vec<Service>,
    pub routes: Vec<Route>,
    pub consumers: Vec<Consumer>,
    pub plugins: Vec<PluginInstance>,
}

pub struct MigrateEngine;

impl MigrateEngine {
    /// Ingest Layer7 XML policies or Kong Gateway JSON configurations into a unified MigrationBundle
    pub fn parse_input_bundle(input_path: &Path) -> anyhow::Result<MigrationBundle> {
        info!("Migrate Engine ingesting configuration bundle from {:?}", input_path);
        let content = fs::read_to_string(input_path)?;

        if input_path.extension().and_then(|e| e.to_str()) == Some("xml") || content.contains("<wsp:Policy") {
            Self::parse_layer7_xml(&content)
        } else {
            Self::parse_kong_json(&content)
        }
    }

    /// Ingest Layer7 XML assertions & policies into HTAR structures
    pub fn parse_layer7_xml(xml: &str) -> anyhow::Result<MigrationBundle> {
        info!("Parsing Layer7 API Gateway XML Policy bundle...");

        let mut services = Vec::new();
        let mut routes = Vec::new();
        let mut plugins = Vec::new();
        let mut consumers = Vec::new();

        let service_name = if xml.contains("<L7p:StrProp key=\"serviceName\" strValue=\"") {
            xml.split("<L7p:StrProp key=\"serviceName\" strValue=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or("payments-service-v1")
                .to_string()
        } else {
            "payments-service-v1".to_string()
        };

        let service_id = format!("{}_id", service_name.replace('-', "_"));

        let target_url = if xml.contains("<L7p:StrProp key=\"protectedUrl\" strValue=\"") {
            xml.split("<L7p:StrProp key=\"protectedUrl\" strValue=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or("http://payments-backend.internal:8080")
                .to_string()
        } else {
            "http://payments-backend.internal:8080".to_string()
        };

        let route_path = format!("/api/v1/{}", service_name.replace("-service-v1", "").replace("-service", ""));

        let service = Service {
            id: service_id.clone(),
            name: service_name.clone(),
            targets: vec![UpstreamTarget {
                url: target_url,
                weight: 10,
                is_healthy: true,
            }],
            connect_timeout_ms: 5000,
            retries: 3,
            health_check_path: Some("/health".to_string()),
        };
        services.push(service);

        let has_auth = xml.contains("RequireHttpBasicAuth") || xml.contains("ApiKey") || xml.contains("OAuth");

        let route = Route {
            id: format!("route_{}", service_id),
            service_id: service_id.clone(),
            hosts: vec![],
            paths: vec![route_path],
            methods: vec!["GET".to_string(), "POST".to_string(), "PUT".to_string()],
            strip_path: true,
            enable_cache: true,
            cache_ttl_secs: Some(300),
            enable_auth: has_auth,
        };
        routes.push(route);

        if has_auth {
            plugins.push(PluginInstance {
                id: format!("plugin_{}_auth", service_id),
                name: "api_key_auth".to_string(),
                route_id: Some(format!("route_{}", service_id)),
                service_id: None,
                enabled: true,
                config: PluginType::ApiKey {
                    header_name: "Authorization".to_string(),
                },
            });

            consumers.push(Consumer {
                id: format!("consumer_{}", service_id),
                username: "l7_migrated_client".to_string(),
                api_keys: vec!["bearer_token_rotational_key".to_string()],
            });
        }

        Ok(MigrationBundle {
            services,
            routes,
            consumers,
            plugins,
        })
    }

    /// Ingest Kong Gateway JSON export matrix into HTAR structures
    pub fn parse_kong_json(json_str: &str) -> anyhow::Result<MigrationBundle> {
        info!("Parsing Kong Gateway JSON export matrix...");
        let val: serde_json::Value = serde_json::from_str(json_str)?;

        let mut services = Vec::new();
        let mut routes = Vec::new();
        let mut consumers = Vec::new();
        let plugins = Vec::new();

        if let Some(svcs) = val.get("services").and_then(|v| v.as_array()) {
            for s in svcs {
                let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("svc_1").to_string();
                let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("kong-service").to_string();
                let host = s.get("host").and_then(|v| v.as_str()).unwrap_or("localhost");
                let port = s.get("port").and_then(|v| v.as_u64()).unwrap_or(8080);
                let protocol = s.get("protocol").and_then(|v| v.as_str()).unwrap_or("http");

                services.push(Service {
                    id,
                    name,
                    targets: vec![UpstreamTarget {
                        url: format!("{}://{}:{}", protocol, host, port),
                        weight: 10,
                        is_healthy: true,
                    }],
                    connect_timeout_ms: 3000,
                    retries: 2,
                    health_check_path: Some("/health".to_string()),
                });
            }
        }

        if let Some(rts) = val.get("routes").and_then(|v| v.as_array()) {
            for r in rts {
                let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("rt_1").to_string();
                let service_id = r.get("service").and_then(|v| v.get("id")).and_then(|v| v.as_str()).unwrap_or("svc_1").to_string();
                let paths = r.get("paths").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter().filter_map(|p| p.as_str().map(|s| s.to_string())).collect()
                }).unwrap_or_else(|| vec!["/".to_string()]);

                routes.push(Route {
                    id,
                    service_id,
                    hosts: vec![],
                    paths,
                    methods: vec!["GET".to_string(), "POST".to_string()],
                    strip_path: r.get("strip_path").and_then(|v| v.as_bool()).unwrap_or(true),
                    enable_cache: true,
                    cache_ttl_secs: Some(60),
                    enable_auth: false,
                });
            }
        }

        if let Some(cons) = val.get("consumers").and_then(|v| v.as_array()) {
            for c in cons {
                let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("cons_1").to_string();
                let username = c.get("username").and_then(|v| v.as_str()).unwrap_or("kong_user").to_string();
                consumers.push(Consumer {
                    id,
                    username,
                    api_keys: vec!["kong_secret_key_123".to_string()],
                });
            }
        }

        Ok(MigrationBundle {
            services,
            routes,
            consumers,
            plugins,
        })
    }

    /// Generate an executable HTAR Gateway TOML configuration (gateway.toml) from the migrated bundle
    pub fn generate_gateway_config(bundle: &MigrationBundle, output_file: &Path) -> anyhow::Result<()> {
        info!("Generating executable HTAR Gateway TOML configuration from bundle...");
        let mut routes = Vec::new();

        for route in &bundle.routes {
            let upstream_url = bundle.services
                .iter()
                .find(|s| s.id == route.service_id)
                .and_then(|s| s.targets.first())
                .map(|t| t.url.clone())
                .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());

            let path_prefix = route.paths.first().cloned().unwrap_or_else(|| "/".to_string());

            routes.push(RouteConfig {
                id: route.id.clone(),
                path_prefix,
                upstream_url,
                strip_path: route.strip_path,
                health_check_path: Some("/health".to_string()),
                enable_cache: route.enable_cache,
                cache_ttl_secs: route.cache_ttl_secs,
            });
        }

        let config = GatewayConfig {
            server: ServerConfig::default(),
            cache: CacheConfig::default(),
            kubernetes: Default::default(),
            routes,
        };

        let toml_str = toml::to_string_pretty(&config)?;
        if let Some(parent) = output_file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output_file, toml_str)?;
        info!("Generated executable HTAR Gateway configuration at {:?}", output_file);
        Ok(())
    }

    /// Automate CNCF Gateway API CRD Generation (Gateway & HTTPRoute resources)
    pub fn generate_gateway_api_crds(bundle: &MigrationBundle, output_file: &Path) -> anyhow::Result<()> {
        info!("Generating CNCF Gateway API CRD resources from migrated bundle...");
        let mut yaml_output = String::new();

        yaml_output.push_str("apiVersion: gateway.networking.k8s.io/v1\n");
        yaml_output.push_str("kind: Gateway\n");
        yaml_output.push_str("metadata:\n");
        yaml_output.push_str("  name: htar-ingress-gateway\n");
        yaml_output.push_str("  namespace: htar-system\n");
        yaml_output.push_str("  annotations:\n");
        yaml_output.push_str("    htar.gateway/enable: \"true\"\n");
        yaml_output.push_str("spec:\n");
        yaml_output.push_str("  gatewayClassName: htar\n");
        yaml_output.push_str("  listeners:\n");
        yaml_output.push_str("  - name: http\n");
        yaml_output.push_str("    protocol: HTTP\n");
        yaml_output.push_str("    port: 80\n");
        yaml_output.push_str("  - name: https\n");
        yaml_output.push_str("    protocol: HTTPS\n");
        yaml_output.push_str("    port: 443\n");
        yaml_output.push_str("---\n");

        for route in &bundle.routes {
            yaml_output.push_str("apiVersion: gateway.networking.k8s.io/v1\n");
            yaml_output.push_str("kind: HTTPRoute\n");
            yaml_output.push_str("metadata:\n");
            yaml_output.push_str(&format!("  name: httproute-{}\n", route.id.replace('_', "-")));
            yaml_output.push_str("  namespace: htar-system\n");
            yaml_output.push_str("  annotations:\n");
            yaml_output.push_str("    htar.gateway/enable: \"true\"\n");
            if let Some(first_path) = route.paths.first() {
                yaml_output.push_str(&format!("    htar.gateway/path: \"{}\"\n", first_path));
            }
            yaml_output.push_str("spec:\n");
            yaml_output.push_str("  parentRefs:\n");
            yaml_output.push_str("  - name: htar-ingress-gateway\n");
            yaml_output.push_str("  rules:\n");
            yaml_output.push_str("  - matches:\n");
            for path in &route.paths {
                yaml_output.push_str("    - path:\n");
                yaml_output.push_str("        type: PathPrefix\n");
                yaml_output.push_str(&format!("        value: {}\n", path));
            }
            yaml_output.push_str("    backendRefs:\n");
            yaml_output.push_str(&format!("    - name: {}\n", route.service_id.replace('_', "-")));
            yaml_output.push_str("      port: 8080\n");
            yaml_output.push_str("---\n");
        }

        if let Some(parent) = output_file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output_file, yaml_output)?;
        info!("Generated CNCF Gateway API CRDs at {:?}", output_file);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_layer7_xml() {
        let sample_xml = r#"<wsp:Policy xmlns:wsp="http://schemas.xmlsoap.org/ws/2004/09/policy">
            <L7p:StrProp key="serviceName" strValue="orders-service-v1"/>
            <L7p:StrProp key="protectedUrl" strValue="http://orders.backend:8080"/>
            <L7p:RequireHttpBasicAuth/>
        </wsp:Policy>"#;

        let bundle = MigrateEngine::parse_layer7_xml(sample_xml).unwrap();
        assert_eq!(bundle.services.len(), 1);
        assert_eq!(bundle.services[0].name, "orders-service-v1");
        assert_eq!(bundle.services[0].targets[0].url, "http://orders.backend:8080");
        assert_eq!(bundle.routes.len(), 1);
        assert!(bundle.routes[0].enable_auth);
        assert_eq!(bundle.plugins.len(), 1);
        assert_eq!(bundle.consumers.len(), 1);
    }

    #[test]
    fn test_parse_kong_json() {
        let sample_json = r#"{
            "services": [
                {
                    "id": "svc_inventory",
                    "name": "inventory-svc",
                    "host": "inventory.internal",
                    "port": 9000,
                    "protocol": "http"
                }
            ],
            "routes": [
                {
                    "id": "rt_inventory",
                    "service": { "id": "svc_inventory" },
                    "paths": ["/api/v1/inventory"],
                    "strip_path": true
                }
            ],
            "consumers": [
                {
                    "id": "cons_app",
                    "username": "client_app"
                }
            ]
        }"#;

        let bundle = MigrateEngine::parse_kong_json(sample_json).unwrap();
        assert_eq!(bundle.services.len(), 1);
        assert_eq!(bundle.services[0].name, "inventory-svc");
        assert_eq!(bundle.services[0].targets[0].url, "http://inventory.internal:9000");
        assert_eq!(bundle.routes.len(), 1);
        assert_eq!(bundle.routes[0].paths, vec!["/api/v1/inventory"]);
        assert_eq!(bundle.consumers.len(), 1);
    }

    #[test]
    fn test_generate_gateway_config() {
        let bundle = MigrationBundle {
            services: vec![Service {
                id: "svc_test".to_string(),
                name: "test-service".to_string(),
                targets: vec![UpstreamTarget {
                    url: "http://10.0.0.1:8080".to_string(),
                    weight: 10,
                    is_healthy: true,
                }],
                connect_timeout_ms: 3000,
                retries: 2,
                health_check_path: Some("/health".to_string()),
            }],
            routes: vec![Route {
                id: "route_test".to_string(),
                service_id: "svc_test".to_string(),
                hosts: vec![],
                paths: vec!["/test".to_string()],
                methods: vec!["GET".to_string()],
                strip_path: false,
                enable_cache: true,
                cache_ttl_secs: Some(120),
                enable_auth: false,
            }],
            consumers: vec![],
            plugins: vec![],
        };

        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();
        MigrateEngine::generate_gateway_config(&bundle, path).unwrap();

        let loaded = GatewayConfig::load_from_file(path).unwrap();
        assert_eq!(loaded.routes.len(), 1);
        assert_eq!(loaded.routes[0].id, "route_test");
        assert_eq!(loaded.routes[0].path_prefix, "/test");
        assert_eq!(loaded.routes[0].upstream_url, "http://10.0.0.1:8080");
        assert_eq!(loaded.routes[0].cache_ttl_secs, Some(120));
    }

    #[test]
    fn test_generate_gateway_api_crds() {
        let bundle = MigrationBundle {
            services: vec![Service {
                id: "svc_k8s".to_string(),
                name: "k8s-service".to_string(),
                targets: vec![],
                connect_timeout_ms: 3000,
                retries: 2,
                health_check_path: None,
            }],
            routes: vec![Route {
                id: "rt_k8s".to_string(),
                service_id: "svc_k8s".to_string(),
                hosts: vec![],
                paths: vec!["/k8s-route".to_string()],
                methods: vec![],
                strip_path: false,
                enable_cache: false,
                cache_ttl_secs: None,
                enable_auth: false,
            }],
            consumers: vec![],
            plugins: vec![],
        };

        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();
        MigrateEngine::generate_gateway_api_crds(&bundle, path).unwrap();

        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("kind: Gateway"));
        assert!(content.contains("kind: HTTPRoute"));
        assert!(content.contains("/k8s-route"));
    }
}
