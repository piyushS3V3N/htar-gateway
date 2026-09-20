use crate::admin::AdminApi;
use crate::config::GatewayConfig;
use crate::plugins::{PluginContext, PluginPipeline, PluginResult};
use crate::registry::{Registry, Route, Service, UpstreamTarget};
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, CONTENT_TYPE};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto;
use htar_cache::HtarCacheManager;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing::{error, info, warn};

pub struct GatewayServer {
    config: GatewayConfig,
    registry: Arc<Registry>,
    plugins: Arc<PluginPipeline>,
    admin_api: Arc<AdminApi>,
    cache: Arc<HtarCacheManager>,
    http_client: reqwest::Client,
}

impl GatewayServer {
    pub fn new(config: GatewayConfig) -> Self {
        let registry = Arc::new(Registry::new());
        let plugins = Arc::new(PluginPipeline::new());
        let admin_api = Arc::new(AdminApi::new(registry.clone(), plugins.clone()));

        // Populate initial static routes from config into dynamic registry
        for route_cfg in &config.routes {
            let service = registry.add_service(Service {
                id: format!("svc_{}", route_cfg.id),
                name: route_cfg.id.clone(),
                targets: vec![UpstreamTarget {
                    url: route_cfg.upstream_url.clone(),
                    weight: 10,
                    is_healthy: true,
                }],
                connect_timeout_ms: 5000,
                retries: 3,
                health_check_path: route_cfg.health_check_path.clone().or_else(|| {
                    if route_cfg.path_prefix.starts_with("/v2") {
                        Some("/v2/".to_string())
                    } else {
                        Some("/_health".to_string())
                    }
                }),
            });

            registry.add_route(Route {
                id: format!("route_{}", route_cfg.id),
                service_id: service.id,
                hosts: vec![],
                paths: vec![route_cfg.path_prefix.clone()],
                methods: vec![],
                strip_path: route_cfg.strip_path,
                enable_cache: route_cfg.enable_cache,
                cache_ttl_secs: route_cfg.cache_ttl_secs,
            });
        }

        // Start background health checking loop
        Registry::start_health_checker(registry.clone(), Duration::from_secs(10));

        // Start Kubernetes Auto-Discovery Controller (watching Services and Ingresses)
        crate::k8s_controller::K8sController::start_auto_discovery(registry.clone());

        let mut cache = HtarCacheManager::new();
        if let Some(path) = &config.cache.htar_bundle_path {
            if let Err(e) = cache.load_archive(path) {
                warn!("Failed to load HTAR bundle archive from {:?}: {}", path, e);
            } else {
                info!("Successfully mounted HTAR cold cache bundle from {:?}", path);
            }
        }

        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build proxy HTTP client");

        Self {
            config,
            registry,
            plugins,
            admin_api,
            cache: Arc::new(cache),
            http_client,
        }
    }

    pub fn registry(&self) -> Arc<Registry> {
        self.registry.clone()
    }

    pub fn plugins(&self) -> Arc<PluginPipeline> {
        self.plugins.clone()
    }

    pub fn cache(&self) -> Arc<HtarCacheManager> {
        self.cache.clone()
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let addr: SocketAddr = format!("{}:{}", self.config.server.host, self.config.server.port).parse()?;
        let listener = TcpListener::bind(addr).await?;
        info!("HTAR Enterprise API Gateway listening on http://{}", addr);
        info!("Admin Control Plane API available at http://{}/admin/v1/", addr);

        let server_arc = Arc::new(self);

        loop {
            let (stream, remote_addr) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let server_clone = server_arc.clone();

            tokio::spawn(async move {
                let service = service_fn(move |req: Request<Incoming>| {
                    let server = server_clone.clone();
                    async move { server.handle_request(req, remote_addr).await }
                });

                if let Err(err) = auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                    .serve_connection(io, service)
                    .await
                {
                    error!("Error serving connection from {}: {:?}", remote_addr, err);
                }
            });
        }
    }

    async fn handle_request(
        &self,
        req: Request<Incoming>,
        remote_addr: SocketAddr,
    ) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
        let method = req.method().clone();
        let path = req.uri().path().to_string();
        let req_host = req.headers().get("host").and_then(|h| h.to_str().ok()).unwrap_or("*").to_string();

        // 1. Admin Control Plane REST API (/admin/v1/...)
        if path.starts_with("/admin/v1") {
            return self.admin_api.handle_admin_request(req).await;
        }

        // 2. Gateway Health Check & Endpoints Overview
        if method == Method::GET && path == "/_htar/health" {
            let stats = self.cache.stats.read().await;
            let body_json = serde_json::json!({
                "status": "UP",
                "gateway": "HTAR-Enterprise-API-Gateway",
                "version": "0.1.0",
                "services": self.registry.list_services().len(),
                "routes": self.registry.list_routes().len(),
                "plugins": self.plugins.list_plugins().len(),
                "cache": {
                    "hits": stats.hits,
                    "misses": stats.misses,
                    "bytes_served": stats.bytes_served,
                }
            });
            let mut res = Response::new(full_body(Bytes::from(body_json.to_string())));
            res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            return Ok(res);
        }

        if method == Method::GET && (path == "/_htar/endpoints" || path == "/_htar/routes") {
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

            let body_json = serde_json::json!({
                "total_endpoints": endpoints.len(),
                "endpoints": endpoints,
            });
            let mut res = Response::new(full_body(Bytes::from(body_json.to_string())));
            res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            return Ok(res);
        }

        // 3. Service & Route Matching (with Host header priority)
        let (route, service) = match self.registry.match_request(&req_host, method.as_str(), &path) {
            Some((r, s)) => (r, s),
            None => {
                // Check Referer header to see if un-prefixed request came from a prefix route (e.g. /ollama/...)
                if let Some(referer) = req.headers().get("referer").and_then(|h| h.to_str().ok()) {
                    if let Ok(ref_url) = reqwest::Url::parse(referer) {
                        let ref_path = ref_url.path();
                        let routes = self.registry.list_routes();
                        for r in &routes {
                            for p in &r.paths {
                                let clean_p = p.trim_end_matches('*').trim_end_matches('/');
                                if !clean_p.is_empty() && clean_p != "/" && (ref_path == clean_p || ref_path.starts_with(&format!("{}/", clean_p))) {
                                    let redirect_target = format!("{}/{}", clean_p, path.trim_start_matches('/'));
                                    info!("Smart Referer Reroute: Request '{}' from referer '{}' redirected to '{}'", path, ref_path, redirect_target);
                                    let mut res = Response::new(full_body(Bytes::from(format!("307 Temporary Redirect to {}", redirect_target))));
                                    *res.status_mut() = StatusCode::TEMPORARY_REDIRECT;
                                    if let Ok(loc) = hyper::header::HeaderValue::from_str(&redirect_target) {
                                        res.headers_mut().insert(hyper::header::LOCATION, loc);
                                    }
                                    return Ok(res);
                                }
                            }
                        }
                    }
                }

                let mut res = Response::new(full_body(Bytes::from("404 Not Found in HTAR Router")));
                *res.status_mut() = StatusCode::NOT_FOUND;
                return Ok(res);
            }
        };

        // Extract Request Headers for Plugin Pipeline
        let req_headers: Vec<(String, String)> = req
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|val| (k.as_str().to_string(), val.to_string())))
            .collect();

        let mut plugin_ctx = PluginContext {
            client_ip: remote_addr.ip().to_string(),
            request_headers: req_headers,
            response_headers: Vec::new(),
            authenticated_user: None,
        };

        // 4. Execute Middleware Plugin Pipeline (Auth, Rate Limiting, Header Transforms)
        match self.plugins.execute_request_plugins(
            &route.id,
            &service.id,
            &self.registry,
            &mut plugin_ctx,
        ) {
            PluginResult::Forbidden(msg) => {
                let mut res = Response::new(full_body(Bytes::from(format!("403 Forbidden: {}", msg))));
                *res.status_mut() = StatusCode::FORBIDDEN;
                return Ok(res);
            }
            PluginResult::TooManyRequests(msg) => {
                let mut res = Response::new(full_body(Bytes::from(format!("429 Too Many Requests: {}", msg))));
                *res.status_mut() = StatusCode::TOO_MANY_REQUESTS;
                return Ok(res);
            }
            PluginResult::Continue => {}
        }

        // 5. HTAR O(1) Cache Lookup (if enabled)
        let query = req.uri().query().unwrap_or("");
        let cache_key = if query.is_empty() {
            format!("{}:{}", method, path)
        } else {
            format!("{}:{}?{}", method, path, query)
        };

        if method == Method::GET && route.enable_cache && self.config.cache.enabled {
            if let Some(cached) = self.cache.get(&cache_key).await {
                let mut res = Response::new(full_body(cached.payload));
                res.headers_mut().insert("X-HTAR-Cache", HeaderValue::from_static("HIT"));
                res.headers_mut().insert("X-HTAR-Gateway", HeaderValue::from_static("v0.1.0"));
                if let Ok(ct) = HeaderValue::from_str(&cached.content_type) {
                    res.headers_mut().insert(CONTENT_TYPE, ct);
                }
                for (k, v) in &plugin_ctx.response_headers {
                    if let (Ok(hk), Ok(hv)) = (hyper::header::HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v)) {
                        res.headers_mut().insert(hk, hv);
                    }
                }
                return Ok(res);
            }
        }

        // 6. Upstream Load Balancer Selection
        let upstream_base_url = match self.registry.select_upstream_target(&service) {
            Some(url) => url,
            None => {
                let mut res = Response::new(full_body(Bytes::from("503 Service Unavailable: No healthy upstream targets")));
                *res.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
                return Ok(res);
            }
        };

        // Compute Target Path (supporting strip_path)
        let request_path = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
        let target_path = compute_target_path(request_path, &route.paths, route.strip_path);

        let target_url = format!("{}{}", upstream_base_url, target_path);

        // Proxy Request to Selected Upstream
        let mut client_req = self.http_client.request(method, &target_url);
        for (k, v) in &plugin_ctx.request_headers {
            if k.to_lowercase() != "host" {
                client_req = client_req.header(k, v);
            }
        }

        match client_req.send().await {
            Ok(upstream_res) => {
                let status = upstream_res.status();
                let content_type = upstream_res
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("application/octet-stream")
                    .to_string();

                let mut upstream_headers = Vec::new();
                for (hk, hv) in upstream_res.headers() {
                    let name = hk.as_str().to_lowercase();
                    if name != "connection"
                        && name != "keep-alive"
                        && name != "proxy-authenticate"
                        && name != "proxy-authorization"
                        && name != "te"
                        && name != "trailers"
                        && name != "transfer-encoding"
                        && name != "upgrade"
                        && name != "content-length"
                    {
                        upstream_headers.push((hk.clone(), hv.clone()));
                    }
                }

                let body_bytes = upstream_res.bytes().await.unwrap_or_default();

                // Save to cache if cacheable GET request
                if status.is_success() && route.enable_cache && self.config.cache.enabled {
                    let ttl = route.cache_ttl_secs.map(Duration::from_secs);
                    self.cache.put(cache_key, body_bytes.clone(), content_type.clone(), ttl);
                }

                let mut res = Response::new(full_body(body_bytes));
                *res.status_mut() = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK);

                // Forward preserved upstream headers
                for (hk, hv) in upstream_headers {
                    res.headers_mut().insert(hk, hv);
                }

                res.headers_mut().insert("X-HTAR-Cache", HeaderValue::from_static("MISS"));
                res.headers_mut().insert("X-HTAR-Gateway", HeaderValue::from_static("v0.1.0"));

                for (k, v) in &plugin_ctx.response_headers {
                    if let (Ok(hk), Ok(hv)) = (hyper::header::HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v)) {
                        res.headers_mut().insert(hk, hv);
                    }
                }

                Ok(res)
            }
            Err(e) => {
                error!("Upstream proxy error for {}: {:?}", target_url, e);
                let mut res = Response::new(full_body(Bytes::from(format!("502 Bad Gateway: Upstream error ({})", e))));
                *res.status_mut() = StatusCode::BAD_GATEWAY;
                Ok(res)
            }
        }
    }
}

pub fn compute_target_path(request_path: &str, route_paths: &[String], strip_path: bool) -> String {
    if !strip_path {
        return request_path.to_string();
    }

    for p in route_paths {
        let clean_p = p.trim_end_matches('*').trim_end_matches('/');
        if clean_p.is_empty() || clean_p == "/" {
            return request_path.to_string();
        }

        if request_path == clean_p {
            return "/".to_string();
        }

        let with_slash = format!("{}/", clean_p);
        if request_path.starts_with(&with_slash) {
            let remainder = &request_path[clean_p.len()..];
            return if remainder.is_empty() {
                "/".to_string()
            } else {
                remainder.to_string()
            };
        }

        let with_query = format!("{}?", clean_p);
        if request_path.starts_with(&with_query) {
            let remainder = &request_path[clean_p.len()..];
            return format!("/{}", remainder);
        }
    }

    request_path.to_string()
}

fn full_body(chunk: Bytes) -> BoxBody<Bytes, hyper::Error> {
    BoxBody::new(Full::new(chunk).map_err(|never| match never {}))
}
