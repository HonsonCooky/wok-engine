//! Request-load entry point and worker-result integration. The system-level glue lives in
//! `crate::system`; this module holds the helper logic that decides whether a load is needed
//! and how to integrate a `WorkerResult::ChunkLoaded`. Plan section 5.1.

use std::sync::Arc;

use crate::chunk::{ChunkGpuHandles, ChunkSlot, ContentEvent, ResidentChunk, SlotState};
use crate::worker::WorkerResult;

/// Decide whether a coord needs a fresh worker dispatch. The plan's idempotency rule (test
/// 7.3 #2): if the slot is already Pending / Loading / Resident, no new dispatch is needed.
/// `Unloading` and `Failed` slots should be replaced by a fresh load attempt. Missing slots
/// always need a dispatch.
pub fn should_dispatch(existing: Option<&ChunkSlot>) -> bool {
    match existing {
        None => true,
        Some(slot) => matches!(
            slot.state,
            SlotState::Unloading(_) | SlotState::Failed(_)
        ),
    }
}

/// Integrate one worker result into the slot map. Returns an optional event for the caller
/// to push onto its event vector. The slot map is mutated in place.
///
/// Cancellation handling (plan section 5.1 "Three failure modes"):
/// - If the slot is missing, the load was cancelled (request_unload deleted the slot);
///   silently drop the result.
/// - If the slot is `Unloading`, similar: the integrator already started releasing; drop.
/// - If the slot is `Failed`, an earlier attempt already failed; the new result is stale
///   if the failed state predates the dispatch. Phase A does not re-dispatch on Failed
///   automatically (`should_dispatch` returns true so the caller can re-issue), so this
///   case only arises when the worker queue had a stale request. Drop.
/// - If the slot is `Pending` or `Loading`, integrate.
pub fn integrate_result<S: std::hash::BuildHasher>(
    slots: &mut std::collections::HashMap<wok_scene::ChunkCoord, ChunkSlot, S>,
    result: WorkerResult,
) -> Option<ContentEvent> {
    match result {
        WorkerResult::ChunkLoaded(payload) => {
            let crate::worker::protocol::ChunkLoadedPayload {
                coord,
                runtime,
                visible_meshes,
                terrain_gpu,
            } = *payload;
            let slot = slots.get_mut(&coord)?;
            match slot.state {
                SlotState::Pending | SlotState::Loading => {
                    slot.state = SlotState::Resident(ResidentChunk {
                        runtime,
                        gpu: ChunkGpuHandles {
                            visible: visible_meshes,
                            terrain: terrain_gpu,
                        },
                    });
                    Some(ContentEvent::ChunkResident(coord))
                }
                SlotState::Resident(_) | SlotState::Unloading(_) | SlotState::Failed(_) => {
                    // Slot already moved on (cancellation, retry, etc.). Drop the result
                    // and let the held resources fall out of scope.
                    drop(runtime);
                    drop(visible_meshes);
                    drop(terrain_gpu);
                    None
                }
            }
        }
        WorkerResult::ChunkFailed { coord, error } => {
            let slot = slots.get_mut(&coord)?;
            match slot.state {
                SlotState::Pending | SlotState::Loading => {
                    slot.state = SlotState::Failed(Arc::clone(&error));
                    Some(ContentEvent::ChunkFailed { coord, error })
                }
                _ => None,
            }
        }
    }
}
