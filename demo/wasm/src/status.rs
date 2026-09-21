//! Where the routine stopped.

use effect_routine_host::status::Status as MachineStatus;
use wasm_bindgen::prelude::*;

/// Where the routine stopped.
#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Suspended; at least one effect in the batch awaits a reply.
    Awaiting,
    /// Ran to completion. Further replies throw.
    Complete,
    /// Suspended on something the host cannot answer. A bug in the routine.
    Stalled,
}

impl From<MachineStatus> for Status {
    fn from(status: MachineStatus) -> Self {
        match status {
            MachineStatus::Awaiting => Status::Awaiting,
            MachineStatus::Complete => Status::Complete,
            MachineStatus::Stalled => Status::Stalled,
        }
    }
}
