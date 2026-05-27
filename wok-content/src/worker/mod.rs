//! Background-worker abstraction. Phase A ships only the `LoopbackWorker` (synchronous,
//! main-thread) per plan section 11 step 6. Phase B brings the real `std::thread`-based
//! worker behind a `crossbeam_channel`. The two share `WorkerRequest` and `WorkerResult` so
//! consumers do not change when the threaded worker arrives.
//!
//! Plan section 1.4 - "stateless requests": each request carries its own borrowed-immutable
//! payload (Arc'd `Chunk`, prefab map, `RegistryReadView`). The worker does not retain any
//! engine state across requests. The LoopbackWorker stores a request queue and runs each
//! request synchronously when the caller invokes `drain`.

pub mod pipeline;
pub mod protocol;

pub use pipeline::run_load_chunk;
pub use protocol::{WorkerRequest, WorkerResult};

use std::collections::VecDeque;

/// Synchronous worker that runs requests on the calling thread. Used by Phase A test code
/// and by editor-preview consumers in wok-shell (plan section 9.19). The main thread submits
/// requests and later calls `drain` to process them; `drain` invokes the supplied closure
/// for each result so the integrator can update slot state without the worker holding a
/// borrow on the system.
#[derive(Debug, Default)]
pub struct LoopbackWorker {
    queue: VecDeque<WorkerRequest>,
}

impl LoopbackWorker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn submit(&mut self, request: WorkerRequest) {
        self.queue.push_back(request);
    }

    pub fn queued_len(&self) -> usize {
        self.queue.len()
    }

    /// Process every queued request. Each request runs synchronously on this thread; the
    /// result is passed to `handle_result`. The queue is empty when `drain` returns.
    pub fn drain<F: FnMut(WorkerResult)>(&mut self, mut handle_result: F) {
        while let Some(request) = self.queue.pop_front() {
            let result = run_load_chunk(request);
            handle_result(result);
        }
    }
}
