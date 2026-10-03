use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub worker_threads: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8443,
            worker_threads: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub enabled: bool,
    pub htar_bundle_path: Option<String>,
    pub max_hot_memory_mb: usize,
    pub default_ttl_secs: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            htar_bundle_path: None,
            max_hot_memory_mb: 256,
            default_ttl_secs: 300,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteConfig {
    pub id: String,
    pub path_prefix: String,
    pub upstream_url: String,
    #[serde(default)]
    pub strip_path: bool,
    #[serde(default)]
    pub health_check_path: Option<String>,
    pub enable_cache: bool,
    pub cache_ttl_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KubernetesConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayConfig {
    pub server: ServerConfig,
    pub cache: CacheConfig,
    #[serde(default)]
    pub kubernetes: KubernetesConfig,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
}

impl GatewayConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: GatewayConfig = toml::from_str(&content)?;
        Ok(config)
    }
}
