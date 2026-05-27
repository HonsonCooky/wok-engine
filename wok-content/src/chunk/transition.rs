//! `transition_chunk` - flip a Resident slot's runtime eagerness. Plan section 3.1 lists
//! this in the Phase-A ContentSystem API; section 5 describes the semantics. The mutation
//! is local to one slot (no I/O, no worker dispatch); a `ChunkTransitioned` event is queued
//! for the next `poll()`.
//!
//! The runtime eagerness lives on `ChunkRuntime.eagerness` (wok-scene's field). The slicer
//! copies authored eagerness into this field at load time; after a transition, the two may
//! diverge. Consumers that need authored eagerness call `ContentSystem::authored_eagerness`
//! (lands in step 9). Plan section 9.11 codifies this.

use std::collections::HashMap;

use wok_scene::{ChunkCoord, ChunkEagerness};

use crate::chunk::{ChunkSlot, ContentEvent, SlotState};
use crate::error::TransitionError;

/// Apply a transition to the slot map. Returns the queued `ChunkTransitioned` event (the
/// caller pushes it onto the system's pending event queue) on success. The runtime arrays
/// inside `ResidentChunk` are untouched; only the eagerness tag flips. No-op transitions
/// (new == current eagerness) return `Ok(None)` so callers do not emit spurious events.
pub fn apply_transition<S: std::hash::BuildHasher>(
    slots: &mut HashMap<ChunkCoord, ChunkSlot, S>,
    coord: ChunkCoord,
    new: ChunkEagerness,
) -> Result<Option<ContentEvent>, TransitionError> {
    let slot = slots
        .get_mut(&coord)
        .ok_or(TransitionError::UnknownSlot(coord))?;
    match &mut slot.state {
        SlotState::Resident(rc) | SlotState::Unloading(rc) => {
            let from = rc.runtime.eagerness;
            if from == new {
                return Ok(None);
            }
            rc.runtime.eagerness = new;
            Ok(Some(ContentEvent::ChunkTransitioned {
                coord,
                from,
                to: new,
            }))
        }
        other => Err(TransitionError::NotResident {
            coord,
            state_label: other.label(),
        }),
    }
}
