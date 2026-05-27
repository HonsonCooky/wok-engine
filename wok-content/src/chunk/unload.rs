//! Request-unload entry point and the per-tick `finalize_unloads` sweep. Plan section 3.2
//! defers the actual slot removal one tick to give consumers a chance to finish their
//! current iteration.

use wok_scene::ChunkCoord;

use crate::chunk::{ChunkSlot, ContentEvent, SlotState};

/// Apply a request_unload to the slot map. Returns true if the slot now exists (either
/// removed or moved to Unloading); false if the coord had no slot (no-op).
pub fn apply_unload<S: std::hash::BuildHasher>(
    slots: &mut std::collections::HashMap<ChunkCoord, ChunkSlot, S>,
    coord: ChunkCoord,
) -> bool {
    let Some(slot) = slots.get_mut(&coord) else {
        return false;
    };
    // Replace state with a dummy then inspect to take ownership of any payload.
    let prev = std::mem::replace(&mut slot.state, SlotState::Pending);
    match prev {
        SlotState::Pending => {
            // Cancellation: the worker may still produce a result; the integrator will see
            // a missing slot (because we remove below) and drop the result.
            slots.remove(&coord);
        }
        SlotState::Loading => {
            // Phase B reach. Same cancellation handling as Pending: drop the slot, let the
            // worker's eventual result be discarded by the integrator.
            slots.remove(&coord);
        }
        SlotState::Resident(rc) => {
            // Defer release: next poll() removes and emits ChunkUnloaded.
            slot.state = SlotState::Unloading(rc);
        }
        SlotState::Unloading(rc) => {
            // Idempotent: already on the way out.
            slot.state = SlotState::Unloading(rc);
        }
        SlotState::Failed(_) => {
            // Failed slots get removed cleanly; no event because the prior ChunkFailed
            // already informed the game.
            slots.remove(&coord);
        }
    }
    true
}

/// Sweep slots in `Unloading` state and remove them, emitting `ChunkUnloaded` for each.
/// Called from `ContentSystem::poll` after integrating worker results.
pub fn finalize_unloads<S: std::hash::BuildHasher>(
    slots: &mut std::collections::HashMap<ChunkCoord, ChunkSlot, S>,
    events: &mut Vec<ContentEvent>,
) {
    let to_remove: Vec<ChunkCoord> = slots
        .iter()
        .filter_map(|(c, s)| {
            if matches!(s.state, SlotState::Unloading(_)) {
                Some(*c)
            } else {
                None
            }
        })
        .collect();
    for coord in to_remove {
        slots.remove(&coord);
        events.push(ContentEvent::ChunkUnloaded(coord));
    }
}
