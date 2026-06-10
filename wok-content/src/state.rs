//! The chunk lifecycle state machine: four states, five legal arcs, illegal arcs as errors.
//!
//! ```text
//! Unloaded -> Loading -> Loaded -> Unloading -> Unloaded
//!                |
//!                +-> Unloaded (failed or canceled load)
//! ```
//!
//! Why each arc:
//!
//! - `Unloaded -> Loading`: a load begins.
//! - `Loading -> Loaded`: the transform finished and the runtime is stored.
//! - `Loading -> Unloaded`: the transform failed, or a pending load was canceled; nothing was stored,
//!   so there is nothing to unload.
//! - `Loaded -> Unloading`: a release begins.
//! - `Unloading -> Unloaded`: the runtime arrays are dropped.
//!
//! Deliberately absent: `Unloading -> Loading` (a chunk wanted again mid-release finishes unloading
//! first, then loads fresh; one path, no half-released state to reason about) and self-arcs (a
//! transition is a change of state; "stay put" is not driven through the machine). If part 2's worker
//! policy needs another arc, it is added here as a reviewed change, not worked around in the store.
//!
//! Part 1 drives the machine synchronously: `Loading` and `Unloading` are passed through atomically
//! inside `ChunkStore::load` / `ChunkStore::release`, never observable at rest. The machine is still
//! defined in full now so part 2's background worker, which does hold chunks in `Loading`, extends the
//! store rather than retrofitting the states.
//!
//! An illegal transition is an error, not a panic: lifecycle misuse (double load, releasing an
//! unloaded chunk) is a caller mistake the caller can handle, not a broken internal invariant.

/// Lifecycle state of one chunk. See the module docs for the legal arcs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChunkState {
    Unloaded,
    Loading,
    Loaded,
    Unloading,
}

/// The error for an arc the machine does not define. Carries both endpoints so the caller (and the
/// error message) can name the misuse precisely.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("illegal chunk state transition: {from:?} -> {to:?}")]
pub struct TransitionError {
    pub from: ChunkState,
    pub to: ChunkState,
}

impl ChunkState {
    /// Step the machine to `to`, returning the new state, or `TransitionError` if the arc is not one
    /// of the five legal ones. Consuming self and returning the next state keeps every state change
    /// flowing through this single check.
    pub fn transition(self, to: ChunkState) -> Result<ChunkState, TransitionError> {
        use ChunkState::{Loaded, Loading, Unloaded, Unloading};
        let legal = matches!(
            (self, to),
            (Unloaded, Loading) | (Loading, Loaded | Unloaded) | (Loaded, Unloading) | (Unloading, Unloaded)
        );
        if legal { Ok(to) } else { Err(TransitionError { from: self, to }) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ChunkState::{Loaded, Loading, Unloaded, Unloading};

    const ALL: [ChunkState; 4] = [Unloaded, Loading, Loaded, Unloading];
    const LEGAL: [(ChunkState, ChunkState); 5] = [
        (Unloaded, Loading),
        (Loading, Loaded),
        (Loading, Unloaded),
        (Loaded, Unloading),
        (Unloading, Unloaded),
    ];

    #[test]
    fn every_legal_transition_succeeds_and_yields_the_target() {
        for (from, to) in LEGAL {
            assert_eq!(from.transition(to).unwrap(), to, "{from:?} -> {to:?}");
        }
    }

    #[test]
    fn every_illegal_transition_errors() {
        // All 16 ordered pairs minus the 5 legal arcs: the 11 illegal ones, self-arcs included.
        let mut illegal_seen = 0;
        for from in ALL {
            for to in ALL {
                if LEGAL.contains(&(from, to)) {
                    continue;
                }
                let err = from.transition(to).unwrap_err();
                assert_eq!(err, TransitionError { from, to });
                illegal_seen += 1;
            }
        }
        assert_eq!(illegal_seen, 11);
    }

    #[test]
    fn transition_error_message_names_both_endpoints() {
        let err = Loaded.transition(Loading).unwrap_err();
        assert_eq!(err.to_string(), "illegal chunk state transition: Loaded -> Loading");
    }
}
