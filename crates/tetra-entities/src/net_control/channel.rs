use std::cell::Cell;
use std::collections::HashMap;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded};
use tetra_core::tetra_entities::TetraEntity;

use crate::net_control::commands::{ControlCommand, ControlResponse};

pub const CONTROL_QUEUE_CAPACITY: usize = 64;

// ---------------------------------------------------------------------------
// CommandDispatcher  (worker side of a per‑entity link)
//
// The CommandWorker holds one CommandDispatcher per connected entity.
// It sends Commands toward the entity and collects CommandResponses back.
// ---------------------------------------------------------------------------

pub struct CommandDispatcher {
    cmd_tx: Sender<ControlCommand>,
    resp_rx: Receiver<ControlResponse>,
    resp_closed: Cell<bool>,
}

impl CommandDispatcher {
    /// Submit a command, logging failure for local callers without a reply path.
    #[inline]
    pub fn send(&self, command: ControlCommand) {
        if self.try_send(command).is_err() {
            tracing::warn!("Control command rejected: entity queue full or closed");
        }
    }

    pub fn try_send(&self, command: ControlCommand) -> Result<(), TrySendError<ControlCommand>> {
        self.cmd_tx.try_send(command)
    }

    /// Non-blocking: collect all pending responses from the entity.
    pub fn try_recv_responses(&self) -> Vec<ControlResponse> {
        let mut responses = Vec::new();
        while let Some(resp) = self.try_recv_response() {
            responses.push(resp);
        }
        responses
    }

    /// Non-blocking: collect a single pending response, if any.
    pub fn try_recv_response(&self) -> Option<ControlResponse> {
        match self.resp_rx.try_recv() {
            Ok(response) => Some(response),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.resp_closed.set(true);
                None
            }
        }
    }

    /// True only after all final responses were drained and the entity dropped.
    pub fn is_closed(&self) -> bool {
        self.resp_closed.get()
    }
}

// ---------------------------------------------------------------------------
// ControlEndpoint  (entity side of a per‑entity link)
//
// Each TetraEntity that participates in the control subsystem holds one
// ControlEndpoint.  It receives ControlCommands and sends ControlResponses back
// toward the worker.
// ---------------------------------------------------------------------------

pub struct ControlEndpoint {
    cmd_rx: Receiver<ControlCommand>,
    resp_tx: Sender<ControlResponse>,
}

impl ControlEndpoint {
    /// Non-blocking: receive a pending command, if any.
    pub fn try_recv(&self) -> Option<ControlCommand> {
        self.cmd_rx.try_recv().ok()
    }

    /// Blocking receive.  Returns `None` when the dispatcher has been dropped.
    pub fn recv(&self) -> Option<ControlCommand> {
        self.cmd_rx.recv().ok()
    }

    /// Send a response with bounded backpressure; report an unavailable worker.
    #[inline]
    pub fn respond(&self, response: ControlResponse) {
        if let Err(error) = self.try_respond(response) {
            let reason = if error.is_timeout() {
                "response queue full"
            } else {
                "worker closed"
            };
            tracing::error!("Control response could not be queued: {reason}");
        }
    }

    pub fn try_respond(&self, response: ControlResponse) -> Result<(), crossbeam_channel::SendTimeoutError<ControlResponse>> {
        self.resp_tx.send_timeout(response, Duration::from_millis(250))
    }
}

// ---------------------------------------------------------------------------
// Link constructor
// ---------------------------------------------------------------------------

/// Create a bidirectional (dispatcher, endpoint) pair.
///
/// - The **dispatcher** (worker side) sends [`ControlCommand`]s and receives [`ControlResponse`]s.
/// - The **endpoint** (entity side) receives [`ControlCommand`]s and sends [`ControlResponse`]s.
pub fn make_control_link() -> (CommandDispatcher, ControlEndpoint) {
    let (cmd_tx, cmd_rx) = bounded(CONTROL_QUEUE_CAPACITY);
    let (resp_tx, resp_rx) = bounded(CONTROL_QUEUE_CAPACITY * 2);
    (
        CommandDispatcher {
            cmd_tx,
            resp_rx,
            resp_closed: Cell::new(false),
        },
        ControlEndpoint { cmd_rx, resp_tx },
    )
}

/// Build one CommandDispatcher / CommandEndpoint pair per TetraEntity
pub fn build_all_control_links() -> (HashMap<TetraEntity, CommandDispatcher>, HashMap<TetraEntity, ControlEndpoint>) {
    let mut dispatchers = HashMap::new();
    let mut endpoints = HashMap::new();

    let (dispatcher, endpoint) = make_control_link();
    dispatchers.insert(TetraEntity::Phy, dispatcher);
    endpoints.insert(TetraEntity::Phy, endpoint);

    let (dispatcher, endpoint) = make_control_link();
    dispatchers.insert(TetraEntity::Lmac, dispatcher);
    endpoints.insert(TetraEntity::Lmac, endpoint);

    let (dispatcher, endpoint) = make_control_link();
    dispatchers.insert(TetraEntity::Umac, dispatcher);
    endpoints.insert(TetraEntity::Umac, endpoint);

    let (dispatcher, endpoint) = make_control_link();
    dispatchers.insert(TetraEntity::Llc, dispatcher);
    endpoints.insert(TetraEntity::Llc, endpoint);

    let (dispatcher, endpoint) = make_control_link();
    dispatchers.insert(TetraEntity::Mle, dispatcher);
    endpoints.insert(TetraEntity::Mle, endpoint);

    let (dispatcher, endpoint) = make_control_link();
    dispatchers.insert(TetraEntity::Mm, dispatcher);
    endpoints.insert(TetraEntity::Mm, endpoint);

    let (dispatcher, endpoint) = make_control_link();
    dispatchers.insert(TetraEntity::Cmce, dispatcher);
    endpoints.insert(TetraEntity::Cmce, endpoint);

    let (dispatcher, endpoint) = make_control_link();
    dispatchers.insert(TetraEntity::Sndcp, dispatcher);
    endpoints.insert(TetraEntity::Sndcp, endpoint);

    (dispatchers, endpoints)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_command_and_receive_response() {
        let (dispatcher, endpoint) = make_control_link();

        // Worker sends a command
        dispatcher.send(ControlCommand::CommandA { handle: 1, parameter: 42 });

        // Entity receives it
        let cmd = endpoint.try_recv().unwrap();
        assert!(matches!(cmd, ControlCommand::CommandA { handle: 1, parameter: 42 }));

        // Entity sends a response
        endpoint.respond(ControlResponse::CommandAResponse { handle: 1, result: 42 });

        // Worker receives the response
        let resp = dispatcher.try_recv_response().unwrap();
        let ControlResponse::CommandAResponse { handle, result } = resp else {
            panic!("expected CommandAResponse");
        };
        assert_eq!(handle, 1);
        assert_eq!(result, 42);
    }

    #[test]
    fn test_multiple_commands_and_responses() {
        let (dispatcher, endpoint) = make_control_link();

        dispatcher.send(ControlCommand::CommandA { handle: 1, parameter: 1 });
        dispatcher.send(ControlCommand::TestCmdB {
            handle: 2,
            source_ssi: 0,
            is_group: false,
            payload: vec![0xFF],
        });

        // Entity drains both
        let a = endpoint.try_recv().unwrap();
        assert!(matches!(a, ControlCommand::CommandA { handle: 1, parameter: 1 }));
        let b = endpoint.try_recv().unwrap();
        if let ControlCommand::TestCmdB {
            handle,
            source_ssi: ssi,
            is_group,
            payload,
        } = &b
        {
            assert_eq!(*handle, 2);
            assert_eq!(*ssi, 0);
            assert!(!(*is_group));
            assert_eq!(*payload, vec![0xFF]);
        } else {
            panic!("expected Command::SendSds variant");
        }
        assert!(endpoint.try_recv().is_none());

        // Entity responds to both
        endpoint.respond(ControlResponse::CommandAResponse { handle: 1, result: 1 });
        endpoint.respond(ControlResponse::SendSdsResponse { handle: 2, success: true });

        // Worker drains all responses at once
        let responses = dispatcher.try_recv_responses();
        assert_eq!(responses.len(), 2);
    }
}
