use std::sync::Arc;

use triumvirate_proto::{AgentId, FabricMessage, Payload, Topic};

use crate::fabric::MessageBus;

#[derive(Debug, Clone)]
pub struct FleetPeerMessage {
    pub fleet_id: String,
    pub from_member: String,
    pub to_member: String,
    pub content: String,
}

pub async fn emit_peer_message(bus: Arc<MessageBus>, message: FleetPeerMessage) {
    let payload = format!(
        "[PEER_MESSAGE fleet={} from={} to={}] {}",
        message.fleet_id, message.from_member, message.to_member, message.content
    );
    bus.emit(FabricMessage::new(
        AgentId::System,
        Topic::TaskProgress,
        Payload::HumanMessage { content: payload },
    ))
    .await;
}
