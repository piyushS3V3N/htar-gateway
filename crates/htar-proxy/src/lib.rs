pub mod admin;
pub mod config;
pub mod ebpf_loader;
pub mod gateway_api;
pub mod gossip;
pub mod k8s_controller;
pub mod mysql_storage;
pub mod plugins;
pub mod raft_consensus;
pub mod registry;
pub mod router;
pub mod server;
pub mod vault;
pub mod wasm_engine;

pub use admin::AdminApi;
pub use config::GatewayConfig;
pub use ebpf_loader::EbpfXdpManager;
pub use gateway_api::GatewayApiController;
pub use gossip::GossipClusterManager;
pub use k8s_controller::K8sController;
pub use mysql_storage::{MysqlConfig, MysqlStorageEngine};
pub use plugins::{PluginInstance, PluginPipeline, PluginType};
pub use raft_consensus::{RaftCommand, RaftConsensusManager, RaftResponse};
pub use registry::{Consumer, Registry, Route, Service, UpstreamTarget};
pub use router::Router;
pub use server::GatewayServer;
pub use vault::{VaultClient, VaultConfig};
pub use wasm_engine::WasmPluginEngine;

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
            enable_auth: false,
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

    #[test]
    fn test_v2_catalog_path_computation() {
        use server::compute_target_path;

        let paths = vec!["/v2".to_string()];

        // With strip_path = false (Preserve full path for Docker Registry)
        let path1 = compute_target_path("/v2/_catalog?n=1000", &paths, false);
        assert_eq!(path1, "/v2/_catalog?n=1000");

        // With strip_path = true
        let path2 = compute_target_path("/v2/_catalog?n=1000", &paths, true);
        assert_eq!(path2, "/_catalog?n=1000");

        // Exact match with strip_path = true
        let path3 = compute_target_path("/v2", &paths, true);
        assert_eq!(path3, "/");

        // Trailing slash prefix /v2/ with strip_path = true
        let paths_slash = vec!["/v2/".to_string()];
        let path4 = compute_target_path("/v2/_catalog?n=1000", &paths_slash, true);
        assert_eq!(path4, "/_catalog?n=1000");
    }

    #[test]
    fn test_v2_registry_route_matching() {
        let registry = Registry::new();

        let service = registry.add_service(Service {
            id: "docker_reg_svc".to_string(),
            name: "docker-registry".to_string(),
            targets: vec![UpstreamTarget {
                url: "http://127.0.0.1:5000".to_string(),
                weight: 10,
                is_healthy: true,
            }],
            connect_timeout_ms: 3000,
            retries: 2,
            health_check_path: Some("/v2/".to_string()),
        });

        let route = registry.add_route(Route {
            id: "v2_route".to_string(),
            service_id: service.id.clone(),
            hosts: vec![],
            paths: vec!["/v2".to_string()],
            methods: vec![],
            strip_path: false,
            enable_cache: false,
            cache_ttl_secs: None,
            enable_auth: false,
        });

        let matched = registry.match_request("localhost:8443", "GET", "/v2/_catalog");
        assert!(matched.is_some());
        let (r, s) = matched.unwrap();
        assert_eq!(r.id, route.id);
        assert_eq!(s.id, service.id);
    }
}
