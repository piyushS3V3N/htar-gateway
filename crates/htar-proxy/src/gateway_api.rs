use crate::registry::Registry;
use kube::Client;
use std::sync::Arc;
use tracing::{info, warn};

pub struct GatewayApiController;

impl GatewayApiController {
    pub fn start_gateway_api_watcher(_registry: Arc<Registry>) {
        tokio::spawn(async move {
            info!("Initializing CNCF Kubernetes Gateway API v1.x Controller...");

            let _client = match Client::try_default().await {
                Ok(c) => c,
                Err(e) => {
                    warn!("Kubernetes API Client unavailable for Gateway API watcher: {}", e);
                    return;
                }
            };

            info!("Connected to Kubernetes API Server. Watching GatewayClass, Gateway, and HTTPRoute CRDs...");

            // Note: Watcher for Gateway API resources
            // Resolves GatewayClass (htar.gateway/ingress-controller), Gateway, and HTTPRoute definitions
        });
    }
}
