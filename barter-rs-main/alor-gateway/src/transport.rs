use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

use alor_protocol::CommandAck;
use alor_protocol::OrderCommand;

use crate::health::HealthState;
use crate::models::{BarEvent, OrderEvent, OrdersSnapshot, PositionEvent, PositionsSnapshot};

#[derive(Debug, Clone)]
pub enum EventMessage {
    Bar(BarEvent),
    Order(OrderEvent),
    Position(PositionEvent),
    Health(HealthState),
    SnapshotOrders(OrdersSnapshot),
    SnapshotPositions(PositionsSnapshot),
}

#[async_trait]
pub trait EventSink: Send + Sync {
    async fn publish_bar(&self, event: BarEvent) -> Result<()>;
    async fn publish_order(&self, event: OrderEvent) -> Result<()>;
    async fn publish_position(&self, event: PositionEvent) -> Result<()>;
    async fn publish_health(&self, health: HealthState) -> Result<()>;
    async fn publish_snapshot_orders(&self, snapshot: OrdersSnapshot) -> Result<()>;
    async fn publish_snapshot_positions(&self, snapshot: PositionsSnapshot) -> Result<()>;
}

#[async_trait]
pub trait CommandSource: Send + Sync {
    async fn next_command(&mut self) -> Option<OrderCommand>;
}

#[async_trait]
pub trait CommandSink: Send + Sync {
    async fn publish_command(&self, command: OrderCommand) -> Result<()>;
    async fn publish_ack(&self, ack: CommandAck) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct InprocEventSink {
    tx: mpsc::Sender<EventMessage>,
}

#[derive(Debug)]
pub struct InprocEventSource {
    rx: mpsc::Receiver<EventMessage>,
}

#[derive(Debug, Clone)]
pub struct InprocCommandSink {
    tx: mpsc::Sender<OrderCommand>,
    ack_tx: mpsc::Sender<CommandAck>,
}

#[derive(Debug)]
pub struct InprocCommandSource {
    rx: mpsc::Receiver<OrderCommand>,
}

#[derive(Debug)]
pub struct InprocAckSource {
    rx: mpsc::Receiver<CommandAck>,
}

impl InprocEventSource {
    pub async fn next_event(&mut self) -> Option<EventMessage> {
        self.rx.recv().await
    }
}

impl InprocCommandSource {
    pub async fn next_command(&mut self) -> Option<OrderCommand> {
        self.rx.recv().await
    }
}

impl InprocAckSource {
    pub async fn next_ack(&mut self) -> Option<CommandAck> {
        self.rx.recv().await
    }
}

pub struct InprocTransport {
    events_tx: mpsc::Sender<EventMessage>,
    events_rx: mpsc::Receiver<EventMessage>,
    commands_tx: mpsc::Sender<OrderCommand>,
    commands_rx: mpsc::Receiver<OrderCommand>,
    ack_tx: mpsc::Sender<CommandAck>,
    ack_rx: mpsc::Receiver<CommandAck>,
}

impl InprocTransport {
    pub fn new(buffer: usize) -> Self {
        let (events_tx, events_rx) = mpsc::channel(buffer);
        let (commands_tx, commands_rx) = mpsc::channel(buffer);
        let (ack_tx, ack_rx) = mpsc::channel(buffer);
        Self {
            events_tx,
            events_rx,
            commands_tx,
            commands_rx,
            ack_tx,
            ack_rx,
        }
    }

    pub fn split(self) -> (
        InprocEventSink,
        InprocEventSource,
        InprocCommandSink,
        InprocCommandSource,
        InprocAckSource,
    ) {
        (
            InprocEventSink { tx: self.events_tx },
            InprocEventSource { rx: self.events_rx },
            InprocCommandSink {
                tx: self.commands_tx,
                ack_tx: self.ack_tx,
            },
            InprocCommandSource {
                rx: self.commands_rx,
            },
            InprocAckSource { rx: self.ack_rx },
        )
    }
}

#[async_trait]
impl EventSink for InprocEventSink {
    async fn publish_bar(&self, event: BarEvent) -> Result<()> {
        self.tx
            .send(EventMessage::Bar(event))
            .await
            .map_err(|_| anyhow::anyhow!("inproc event channel closed"))
    }

    async fn publish_order(&self, event: OrderEvent) -> Result<()> {
        self.tx
            .send(EventMessage::Order(event))
            .await
            .map_err(|_| anyhow::anyhow!("inproc event channel closed"))
    }

    async fn publish_position(&self, event: PositionEvent) -> Result<()> {
        self.tx
            .send(EventMessage::Position(event))
            .await
            .map_err(|_| anyhow::anyhow!("inproc event channel closed"))
    }

    async fn publish_health(&self, health: HealthState) -> Result<()> {
        self.tx
            .send(EventMessage::Health(health))
            .await
            .map_err(|_| anyhow::anyhow!("inproc event channel closed"))
    }

    async fn publish_snapshot_orders(&self, snapshot: OrdersSnapshot) -> Result<()> {
        self.tx
            .send(EventMessage::SnapshotOrders(snapshot))
            .await
            .map_err(|_| anyhow::anyhow!("inproc event channel closed"))
    }

    async fn publish_snapshot_positions(&self, snapshot: PositionsSnapshot) -> Result<()> {
        self.tx
            .send(EventMessage::SnapshotPositions(snapshot))
            .await
            .map_err(|_| anyhow::anyhow!("inproc event channel closed"))
    }
}

#[async_trait]
impl CommandSource for InprocCommandSource {
    async fn next_command(&mut self) -> Option<OrderCommand> {
        self.rx.recv().await
    }
}

#[async_trait]
impl CommandSink for InprocCommandSink {
    async fn publish_command(&self, command: OrderCommand) -> Result<()> {
        self.tx
            .send(command)
            .await
            .map_err(|_| anyhow::anyhow!("inproc command channel closed"))
    }

    async fn publish_ack(&self, ack: CommandAck) -> Result<()> {
        self.ack_tx
            .send(ack)
            .await
            .map_err(|_| anyhow::anyhow!("inproc ack channel closed"))
    }
}
