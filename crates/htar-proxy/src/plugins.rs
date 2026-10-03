use crate::registry::Registry;
use dashmap::DashMap;
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tracing::warn;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PluginType {
    ApiKey {
        header_name: String,
    },
    JwtAuth {
        secret: String,
        algorithm: String,
    },
    RateLimiter {
        limit_per_minute: u64,
    },
    HeaderTransform {
        add_request_headers: Vec<(String, String)>,
        remove_request_headers: Vec<String>,
        add_response_headers: Vec<(String, String)>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInstance {
    pub id: String,
    pub name: String,
    pub route_id: Option<String>,
    pub service_id: Option<String>,
    pub enabled: bool,
    pub config: PluginType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

pub struct PluginContext {
    pub client_ip: String,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub authenticated_user: Option<String>,
}

pub struct PluginPipeline {
    plugins: DashMap<String, PluginInstance>,
    rate_limiter_buckets: DashMap<String, Arc<RateBucket>>,
}

struct RateBucket {
    count: AtomicU64,
    reset_at: std::sync::RwLock<Instant>,
}

pub enum PluginResult {
    Continue,
    Forbidden(String),
    TooManyRequests(String),
}

impl Default for PluginPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginPipeline {
    pub fn new() -> Self {
        Self {
            plugins: DashMap::new(),
            rate_limiter_buckets: DashMap::new(),
        }
    }

    pub fn add_plugin(&self, mut instance: PluginInstance) -> PluginInstance {
        if instance.id.is_empty() {
            instance.id = Uuid::new_v4().to_string();
        }
        self.plugins.insert(instance.id.clone(), instance.clone());
        instance
    }

    pub fn list_plugins(&self) -> Vec<PluginInstance> {
        self.plugins.iter().map(|p| p.value().clone()).collect()
    }

    pub fn delete_plugin(&self, id: &str) -> bool {
        self.plugins.remove(id).is_some()
    }

    /// Execute dynamic plugin chain for a matched route & service
    pub fn execute_request_plugins(
        &self,
        route_id: &str,
        service_id: &str,
        registry: &Registry,
        context: &mut PluginContext,
    ) -> PluginResult {
        let active_plugins: Vec<PluginInstance> = self
            .plugins
            .iter()
            .filter_map(|p| {
                let inst = p.value();
                if !inst.enabled {
                    return None;
                }
                let matches_route = inst.route_id.as_deref() == Some(route_id);
                let matches_service = inst.service_id.as_deref() == Some(service_id);
                let is_global = inst.route_id.is_none() && inst.service_id.is_none();

                if matches_route || matches_service || is_global {
                    Some(inst.clone())
                } else {
                    None
                }
            })
            .collect();

        for plugin in active_plugins {
            match &plugin.config {
                PluginType::ApiKey { header_name } => {
                    let key_val = context.request_headers.iter().find_map(|(k, v)| {
                        if k.eq_ignore_ascii_case(header_name) {
                            Some(v.as_str())
                        } else {
                            None
                        }
                    });

                    match key_val {
                        Some(key) => {
                            if let Some(consumer) = registry.validate_api_key(key) {
                                context.authenticated_user = Some(consumer.username);
                            } else {
                                warn!("API Key Plugin: Invalid key '{}'", key);
                                return PluginResult::Forbidden("Invalid API Key".to_string());
                            }
                        }
                        None => {
                            return PluginResult::Forbidden(format!(
                                "Missing required header '{}'",
                                header_name
                            ));
                        }
                    }
                }
                PluginType::JwtAuth { secret, .. } => {
                    let auth_header = context.request_headers.iter().find_map(|(k, v)| {
                        if k.eq_ignore_ascii_case("authorization") {
                            Some(v.as_str())
                        } else {
                            None
                        }
                    });

                    match auth_header {
                        Some(auth) if auth.starts_with("Bearer ") => {
                            let token = &auth[7..];
                            let key = DecodingKey::from_secret(secret.as_bytes());
                            let validation = Validation::default();

                            match decode::<Claims>(token, &key, &validation) {
                                Ok(token_data) => {
                                    context.authenticated_user = Some(token_data.claims.sub.clone());
                                    context
                                        .request_headers
                                        .push(("X-User-Id".to_string(), token_data.claims.sub));
                                }
                                Err(e) => {
                                    warn!("JWT Plugin: Token decode failed: {:?}", e);
                                    return PluginResult::Forbidden("Invalid JWT Signature".to_string());
                                }
                            }
                        }
                        _ => return PluginResult::Forbidden("Missing or Malformed Bearer Token".to_string()),
                    }
                }
                PluginType::RateLimiter { limit_per_minute } => {
                    let rate_key = context
                        .authenticated_user
                        .clone()
                        .unwrap_or_else(|| context.client_ip.clone());

                    let bucket = self
                        .rate_limiter_buckets
                        .entry(rate_key)
                        .or_insert_with(|| {
                            Arc::new(RateBucket {
                                count: AtomicU64::new(0),
                                reset_at: std::sync::RwLock::new(Instant::now()),
                            })
                        });

                    let mut reset_lock = bucket.reset_at.write().unwrap();
                    if reset_lock.elapsed() >= std::time::Duration::from_secs(60) {
                        bucket.count.store(0, Ordering::Relaxed);
                        *reset_lock = Instant::now();
                    }

                    let current = bucket.count.fetch_add(1, Ordering::Relaxed) + 1;
                    if current > *limit_per_minute {
                        return PluginResult::TooManyRequests(format!(
                            "Rate limit exceeded: {} requests/min",
                            limit_per_minute
                        ));
                    }
                }
                PluginType::HeaderTransform {
                    add_request_headers,
                    remove_request_headers,
                    add_response_headers,
                } => {
                    for remove_key in remove_request_headers {
                        context
                            .request_headers
                            .retain(|(k, _)| !k.eq_ignore_ascii_case(remove_key));
                    }

                    for (k, v) in add_request_headers {
                        context.request_headers.push((k.clone(), v.clone()));
                    }

                    for (k, v) in add_response_headers {
                        context.response_headers.push((k.clone(), v.clone()));
                    }
                }
            }
        }

        PluginResult::Continue
    }
}
