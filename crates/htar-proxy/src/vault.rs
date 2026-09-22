use serde::{Deserialize, Serialize};
use tracing::info;
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultConfig {
    pub address: String,
    pub token: String,
    pub secret_path: String,
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            address: std::env::var("VAULT_ADDR").unwrap_or_else(|_| "http://vault.dev-tools.svc.cluster.local:8200".to_string()),
            token: std::env::var("VAULT_TOKEN").unwrap_or_default(),
            secret_path: std::env::var("VAULT_SECRET_PATH").unwrap_or_else(|_| "secret/data/htargw".to_string()),
        }
    }
}

pub struct VaultClient {
    config: VaultConfig,
    client: reqwest::Client,
}

impl VaultClient {
    pub fn new(config: VaultConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        Self { config, client }
    }

    /// Fetch environment secrets from Vault Agent sidecar file or HashiCorp Vault REST API
    pub async fn fetch_secrets(&self) -> anyhow::Result<HashMap<String, String>> {
        let mut secrets = HashMap::new();

        // 1. Check if Vault Agent sidecar injected secret file exists on disk
        let secret_file_paths = ["/vault/secrets/env", "/vault/secrets/config", "/vault/secrets/htargw"];
        for path in secret_file_paths {
            if std::path::Path::new(path).exists() {
                if let Ok(contents) = std::fs::read_to_string(path) {
                    info!("Reading Vault secrets from Agent sidecar file at: {}", path);
                    for line in contents.lines() {
                        let trimmed = line.trim();
                        if trimmed.is_empty() || trimmed.starts_with('#') {
                            continue;
                        }
                        let line_clean = trimmed.strip_prefix("export ").unwrap_or(trimmed);
                        if let Some((k, v)) = line_clean.split_once('=') {
                            let key = k.trim().to_string();
                            let val = v.trim().trim_matches('"').trim_matches('\'').to_string();
                            secrets.insert(key, val);
                        }
                    }
                    if !secrets.is_empty() {
                        info!("Successfully loaded {} secrets from Vault Agent sidecar file", secrets.len());
                        return Ok(secrets);
                    }
                }
            }
        }

        // 2. Direct HTTP REST query to Vault server
        let url = format!("{}/v1/{}", self.config.address.trim_end_matches('/'), self.config.secret_path.trim_start_matches('/'));
        info!("Fetching secrets from HashiCorp Vault API at: {}", url);

        let mut req = self.client.get(&url);
        if !self.config.token.is_empty() {
            req = req.header("X-Vault-Token", &self.config.token);
        }

        let res = req.send().await?;
        if !res.status().is_success() {
            anyhow::bail!("Vault request failed with status: {}", res.status());
        }

        let json_val: serde_json::Value = res.json().await?;

        // Parse KV v2 (data.data) or KV v1 (data) payload format
        if let Some(data_obj) = json_val.get("data").and_then(|d| d.get("data")).and_then(|d| d.as_object()) {
            for (k, v) in data_obj {
                if let Some(val_str) = v.as_str() {
                    secrets.insert(k.clone(), val_str.to_string());
                }
            }
        } else if let Some(data_obj) = json_val.get("data").and_then(|d| d.as_object()) {
            for (k, v) in data_obj {
                if let Some(val_str) = v.as_str() {
                    secrets.insert(k.clone(), val_str.to_string());
                }
            }
        }

        info!("Successfully retrieved {} secrets from HashiCorp Vault REST API", secrets.len());
        Ok(secrets)
    }

    pub fn config(&self) -> &VaultConfig {
        &self.config
    }
}
