use crate::plugins::{PluginInstance, PluginPipeline};
use crate::registry::{Consumer, Registry, Route, Service};
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::header::CONTENT_TYPE;
use hyper::{Method, Request, Response, StatusCode};
use serde_json::json;
use std::sync::Arc;

pub struct AdminApi {
    registry: Arc<Registry>,
    plugins: Arc<PluginPipeline>,
}

impl AdminApi {
    pub fn new(registry: Arc<Registry>, plugins: Arc<PluginPipeline>) -> Self {
        Self { registry, plugins }
    }

    pub async fn handle_admin_request(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        let response_json = match (method, path.as_str()) {
            // --- Services API ---
            (Method::GET, "/admin/v1/services") => {
                json!({ "services": self.registry.list_services() })
            }
            (Method::POST, "/admin/v1/services") => {
                let body_bytes = req.into_body().collect().await?.to_bytes();
                match serde_json::from_slice::<Service>(&body_bytes) {
                    Ok(svc) => {
                        let created = self.registry.add_service(svc);
                        json!({ "status": "created", "service": created })
                    }
                    Err(e) => return Ok(bad_request(&format!("Invalid service JSON: {}", e))),
                }
            }

            // --- Routes API ---
            (Method::GET, "/admin/v1/routes") => {
                json!({ "routes": self.registry.list_routes() })
            }
            (Method::POST, "/admin/v1/routes") => {
                let body_bytes = req.into_body().collect().await?.to_bytes();
                match serde_json::from_slice::<Route>(&body_bytes) {
                    Ok(route) => {
                        let created = self.registry.add_route(route);
                        json!({ "status": "created", "route": created })
                    }
                    Err(e) => return Ok(bad_request(&format!("Invalid route JSON: {}", e))),
                }
            }

            // --- Consumers API ---
            (Method::GET, "/admin/v1/consumers") => {
                json!({ "consumers": self.registry.list_consumers() })
            }
            (Method::POST, "/admin/v1/consumers") => {
                let body_bytes = req.into_body().collect().await?.to_bytes();
                match serde_json::from_slice::<Consumer>(&body_bytes) {
                    Ok(consumer) => {
                        let created = self.registry.add_consumer(consumer);
                        json!({ "status": "created", "consumer": created })
                    }
                    Err(e) => return Ok(bad_request(&format!("Invalid consumer JSON: {}", e))),
                }
            }

            // --- Plugins API ---
            (Method::GET, "/admin/v1/plugins") => {
                json!({ "plugins": self.plugins.list_plugins() })
            }
            (Method::POST, "/admin/v1/plugins") => {
                let body_bytes = req.into_body().collect().await?.to_bytes();
                match serde_json::from_slice::<PluginInstance>(&body_bytes) {
                    Ok(plugin) => {
                        let created = self.plugins.add_plugin(plugin);
                        json!({ "status": "created", "plugin": created })
                    }
                    Err(e) => return Ok(bad_request(&format!("Invalid plugin JSON: {}", e))),
                }
            }

            // --- Endpoints Overview ---
            (Method::GET, "/admin/v1/endpoints") => {
                let routes = self.registry.list_routes();
                let services = self.registry.list_services();

                let mut seen = std::collections::HashSet::new();
                let mut endpoints = Vec::new();
                for r in routes {
                    let target_svc = services.iter().find(|s| s.id == r.service_id);
                    let upstream_targets = target_svc.map(|s| s.targets.clone()).unwrap_or_default();
                    let is_healthy = target_svc.map(|s| s.targets.iter().any(|t| t.is_healthy)).unwrap_or(false);
                    let path_key = format!("{:?}:{:?}:{}", r.hosts, r.paths, r.service_id);

                    if seen.insert(path_key) {
                        endpoints.push(serde_json::json!({
                            "route_id": r.id,
                            "hosts": if r.hosts.is_empty() { vec!["*".to_string()] } else { r.hosts.clone() },
                            "paths": r.paths,
                            "service_id": r.service_id,
                            "service_name": target_svc.map(|s| s.name.clone()).unwrap_or_else(|| r.service_id.clone()),
                            "upstreams": upstream_targets,
                            "healthy": is_healthy,
                            "cache_enabled": r.enable_cache,
                            "cache_ttl_secs": r.cache_ttl_secs,
                        }));
                    }
                }

                json!({
                    "total_endpoints": endpoints.len(),
                    "endpoints": endpoints,
                })
            }

            // --- State Overview ---
            (Method::GET, "/admin/v1/overview") => {
                json!({
                    "services_count": self.registry.list_services().len(),
                    "routes_count": self.registry.list_routes().len(),
                    "consumers_count": self.registry.list_consumers().len(),
                    "plugins_count": self.plugins.list_plugins().len(),
                    "engine": "HTAR-API-Gateway"
                })
            }

            _ => return Ok(not_found()),
        };

        let mut res = Response::new(full_body(Bytes::from(response_json.to_string())));
        res.headers_mut().insert(CONTENT_TYPE, hyper::header::HeaderValue::from_static("application/json"));
        Ok(res)
    }
}

fn full_body(chunk: Bytes) -> BoxBody<Bytes, hyper::Error> {
    BoxBody::new(Full::new(chunk).map_err(|never| match never {}))
}

fn bad_request(msg: &str) -> Response<BoxBody<Bytes, hyper::Error>> {
    let mut res = Response::new(full_body(Bytes::from(json!({ "error": msg }).to_string())));
    *res.status_mut() = StatusCode::BAD_REQUEST;
    res.headers_mut().insert(CONTENT_TYPE, hyper::header::HeaderValue::from_static("application/json"));
    res
}

fn not_found() -> Response<BoxBody<Bytes, hyper::Error>> {
    let mut res = Response::new(full_body(Bytes::from(json!({ "error": "Admin endpoint not found" }).to_string())));
    *res.status_mut() = StatusCode::NOT_FOUND;
    res.headers_mut().insert(CONTENT_TYPE, hyper::header::HeaderValue::from_static("application/json"));
    res
}
