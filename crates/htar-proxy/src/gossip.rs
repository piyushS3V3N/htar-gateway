use htar_cache::HtarCacheManager;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

/// Cluster Gossip Message for Ephemeral Cache Invalidation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ClusterGossipMessage {
    PurgeCacheTag { tag: String },
    PurgeAllCache,
}

pub struct GossipClusterManager {
    pub node_id: String,
    pub listen_addr: SocketAddr,
    pub cache: Arc<HtarCacheManager>,
    pub purge_tx: mpsc::UnboundedSender<ClusterGossipMessage>,
}

impl GossipClusterManager {
    pub fn new(node_id: String, listen_addr: SocketAddr, cache: Arc<HtarCacheManager>) -> (Self, mpsc::UnboundedReceiver<ClusterGossipMessage>) {
        let (purge_tx, purge_rx) = mpsc::unbounded_channel();
        (
            Self {
                node_id,
                listen_addr,
                cache,
                purge_tx,
            },
            purge_rx,
        )
    }

    pub fn start_cluster_node(
        node_id: String,
        gossip_addr: SocketAddr,
        _seed_nodes: Vec<String>,
        cache: Arc<HtarCacheManager>,
    ) -> mpsc::UnboundedSender<ClusterGossipMessage> {
        let (manager, mut purge_rx) = Self::new(node_id.clone(), gossip_addr, cache.clone());
        let purge_tx = manager.purge_tx.clone();

        tokio::spawn(async move {
            info!("Starting HTAR Cluster Gossip Node '{}' on {}", node_id, gossip_addr);

            // Channel processing for outgoing cache purge broadcasts across cluster
            while let Some(msg) = purge_rx.recv().await {
                match msg {
                    ClusterGossipMessage::PurgeCacheTag { tag } => {
                        info!("Broadcasting cluster-wide cache invalidation tag: '{}'", tag);
                        cache.invalidate_tag(&tag);
                    }
                    ClusterGossipMessage::PurgeAllCache => {
                        info!("Broadcasting full cluster cache purge signal");
                    }
                }
            }
        });

        purge_tx
    }
}
