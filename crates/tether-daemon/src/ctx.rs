//! Shared daemon state. The lock is a plain `std::sync::Mutex` and is never
//! held across an `.await`: every critical section is a few map operations.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use quinn::Connection;
use tether_core::{ApplyResult, Clock, DeviceId, Op, OpKind, OpLog};

pub struct State {
    pub me: DeviceId,
    pub name: String,
    pub log: OpLog,
    /// Open connections, keyed by the peer's id (learned from its `Hello`).
    pub peers: HashMap<DeviceId, Connection>,
    /// Peers we are currently dialing, so repeated mDNS events don't open
    /// duplicate connections.
    pub dialing: HashSet<DeviceId>,
}

#[derive(Default, Debug)]
pub struct ApplyOutcome {
    pub applied: u32,
    pub gap: bool,
}

#[derive(Clone)]
pub struct Ctx {
    pub state: Arc<Mutex<State>>,
    pub clock: Arc<dyn Clock>,
    /// Text received from peers, handed to the clipboard (or stdout) thread.
    deliver: Sender<String>,
}

impl Ctx {
    pub fn new(me: DeviceId, name: String, clock: Arc<dyn Clock>, deliver: Sender<String>) -> Self {
        let state = State { me, name, log: OpLog::new(me), peers: HashMap::new(), dialing: HashSet::new() };
        Self { state: Arc::new(Mutex::new(state)), clock, deliver }
    }

    pub fn me(&self) -> DeviceId {
        self.state.lock().unwrap().me
    }

    pub fn peers(&self) -> Vec<(DeviceId, Connection)> {
        let s = self.state.lock().unwrap();
        s.peers.iter().map(|(id, c)| (*id, c.clone())).collect()
    }

    /// Record something copied on this device.
    pub fn local_copy(&self, text: String) -> Op {
        let now = self.clock.now_ms();
        self.state.lock().unwrap().log.append_local(OpKind::ClipboardText(text), now)
    }

    /// Apply operations from a peer and surface the newest clipboard item.
    pub fn apply(&self, ops: Vec<Op>) -> ApplyOutcome {
        let now = self.clock.now_ms();
        let mut out = ApplyOutcome::default();
        let mut newest: Option<Op> = None;
        {
            let mut s = self.state.lock().unwrap();
            for op in ops {
                match s.log.apply_remote(op.clone(), now) {
                    ApplyResult::Applied => {
                        out.applied += 1;
                        if newest.as_ref().map_or(true, |n| op.order_key() > n.order_key()) {
                            newest = Some(op);
                        }
                    }
                    ApplyResult::Duplicate => {}
                    ApplyResult::Gap { .. } => out.gap = true,
                }
            }
            // An old operation arriving late must not overwrite a newer clipboard.
            let top = s.log.clipboard_history().first().map(|op| op.id());
            if newest.as_ref().map(Op::id) != top {
                newest = None;
            }
        }
        if let Some(op) = newest {
            match op.kind {
                OpKind::ClipboardText(text) => {
                    let _ = self.deliver.send(text);
                }
            }
        }
        out
    }
}
