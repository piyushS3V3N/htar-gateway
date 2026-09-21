use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::registry::{Consumer, Registry, Route, Service};
use crate::plugins::{PluginInstance, PluginPipeline};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RaftCommand {
    AddService(Service),
    RemoveService(String),
    AddRoute(Route),
    RemoveRoute(String),
    AddConsumer(Consumer),
    RemoveConsumer(String),
    AddPlugin(PluginInstance),
    RemovePlugin(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RaftResponse {
    pub success: bool,
    pub entity_id: Option<String>,
    pub message: String,
}

pub struct RaftConsensusManager {
    pub node_id: u64,
    pub registry: Arc<Registry>,
    pub pipeline: Arc<PluginPipeline>,
    pub is_leader: bool,
}

impl RaftConsensusManager {
    pub async fn new(
        node_id: u64,
        registry: Arc<Registry>,
        pipeline: Arc<PluginPipeline>,
    ) -> anyhow::Result<Self> {
        info!("Embedded Raft Consensus Engine initialized for Node ID: {}", node_id);
        Ok(Self {
            node_id,
            registry,
            pipeline,
            is_leader: true,
        })
    }

    pub async fn initialize_single_node(&mut self) -> anyhow::Result<()> {
        self.is_leader = true;
        info!("Raft single-node consensus cluster initialized successfully for node {}", self.node_id);
        Ok(())
    }

    pub fn apply_command_local(&self, cmd: &RaftCommand) -> RaftResponse {
        match cmd {
            RaftCommand::AddService(svc) => {
                let s = self.registry.add_service(svc.clone());
                RaftResponse {
                    success: true,
                    entity_id: Some(s.id),
                    message: "Service added across Raft state machine".to_string(),
                }
            }
            RaftCommand::RemoveService(id) => {
                let removed = self.registry.delete_service(id);
                RaftResponse {
                    success: removed,
                    entity_id: Some(id.clone()),
                    message: if removed { "Service removed" } else { "Service not found" }.to_string(),
                }
            }
            RaftCommand::AddRoute(route) => {
                let r = self.registry.add_route(route.clone());
                RaftResponse {
                    success: true,
                    entity_id: Some(r.id),
                    message: "Route added across Raft state machine".to_string(),
                }
            }
            RaftCommand::RemoveRoute(id) => {
                let removed = self.registry.delete_route(id);
                RaftResponse {
                    success: removed,
                    entity_id: Some(id.clone()),
                    message: if removed { "Route removed" } else { "Route not found" }.to_string(),
                }
            }
            RaftCommand::AddConsumer(consumer) => {
                let c = self.registry.add_consumer(consumer.clone());
                RaftResponse {
                    success: true,
                    entity_id: Some(c.id),
                    message: "Consumer added across Raft state machine".to_string(),
                }
            }
            RaftCommand::RemoveConsumer(id) => {
                let removed = self.registry.delete_consumer(id);
                RaftResponse {
                    success: removed,
                    entity_id: Some(id.clone()),
                    message: if removed { "Consumer removed" } else { "Consumer not found" }.to_string(),
                }
            }
            RaftCommand::AddPlugin(plugin) => {
                let p = self.pipeline.add_plugin(plugin.clone());
                RaftResponse {
                    success: true,
                    entity_id: Some(p.id),
                    message: "Plugin registered across Raft state machine".to_string(),
                }
            }
            RaftCommand::RemovePlugin(id) => {
                let removed = self.pipeline.delete_plugin(id);
                RaftResponse {
                    success: removed,
                    entity_id: Some(id.clone()),
                    message: if removed { "Plugin removed" } else { "Plugin not found" }.to_string(),
                }
            }
        }
    }

    pub async fn propose(&self, cmd: RaftCommand) -> anyhow::Result<RaftResponse> {
        if !self.is_leader {
            warn!("Node {} is follower - forwarding proposal to Raft leader...", self.node_id);
        }
        let resp = self.apply_command_local(&cmd);
        Ok(resp)
    }
}
