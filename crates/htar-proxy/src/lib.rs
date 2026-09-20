pub mod admin;
pub mod config;
pub mod k8s_controller;
pub mod plugins;
pub mod registry;
pub mod router;
pub mod server;

pub use admin::AdminApi;
pub use config::GatewayConfig;
pub use k8s_controller::K8sController;
pub use plugins::{PluginInstance, PluginPipeline, PluginType};
pub use registry::{Consumer, Registry, Route, Service, UpstreamTarget};
pub use router::Router;
pub use server::GatewayServer;

#[cfg(test)]
mod tests {
    use super::*;
    use plugins::PluginContext;

    #[test]
    fn test_service_route_registry() {
        let registry = Registry::new();

        let service = registry.add_service(Service {
            id: "".to_string(),
            name: "user-service".to_string(),
            targets: vec![UpstreamTarget {
                url: "http://127.0.0.1:9000".to_string(),
                weight: 10,
                is_healthy: true,
            }],
            connect_timeout_ms: 3000,
            retries: 2,
            health_check_path: Some("/health".to_string()),
        });

        assert!(!service.id.is_empty());

        let route = registry.add_route(Route {
            id: "".to_string(),
            service_id: service.id.clone(),
            hosts: vec![],
            paths: vec!["/api/v1/users".to_string()],
            methods: vec!["GET".to_string(), "POST".to_string()],
            strip_path: false,
            enable_cache: true,
            cache_ttl_secs: Some(60),
        });

        assert!(!route.id.is_empty());

        let matched = registry.match_request("*", "GET", "/api/v1/users/profile");
        assert!(matched.is_some());
        let (r, s) = matched.unwrap();
        assert_eq!(r.service_id, service.id);
        assert_eq!(s.name, "user-service");
    }

    #[test]
    fn test_api_key_plugin_validation() {
        let registry = Registry::new();
        let pipeline = PluginPipeline::new();

        let consumer = registry.add_consumer(Consumer {
            id: "".to_string(),
            username: "partner_app".to_string(),
            api_keys: vec!["secret_api_key_123".to_string()],
        });
        assert_eq!(consumer.username, "partner_app");

        pipeline.add_plugin(PluginInstance {
            id: "".to_string(),
            name: "api_key_auth".to_string(),
            route_id: None,
            service_id: None,
            enabled: true,
            config: PluginType::ApiKey {
                header_name: "X-API-Key".to_string(),
            },
        });

        let mut ctx_valid = PluginContext {
            client_ip: "127.0.0.1".to_string(),
            request_headers: vec![("X-API-Key".to_string(), "secret_api_key_123".to_string())],
            response_headers: vec![],
            authenticated_user: None,
        };

        let res = pipeline.execute_request_plugins("route1", "svc1", &registry, &mut ctx_valid);
        assert!(matches!(res, plugins::PluginResult::Continue));
        assert_eq!(ctx_valid.authenticated_user.as_deref(), Some("partner_app"));

        let mut ctx_invalid = PluginContext {
            client_ip: "127.0.0.1".to_string(),
            request_headers: vec![("X-API-Key".to_string(), "wrong_key".to_string())],
            response_headers: vec![],
            authenticated_user: None,
        };

        let res_err = pipeline.execute_request_plugins("route1", "svc1", &registry, &mut ctx_invalid);
        assert!(matches!(res_err, plugins::PluginResult::Forbidden(_)));
    }
}
