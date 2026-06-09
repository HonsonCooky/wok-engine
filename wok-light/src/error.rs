//! Error types for the load and save surface, mirroring wok-scene's split.
//!
//! `LoadError` covers I/O failure, JSON parse failure, in-file validation failure (a curve with no
//! keyframes, or keyframes whose times are not strictly increasing), and the one path-shaped
//! failure this crate has: a light-state file whose stem cannot be read as a UTF-8 name, since a
//! state's name is its file stem (see `crate::io`).
//!
//! `SaveError` is the symmetric save-side enum: serialization or the write itself. The two are
//! kept separate, as in wok-scene, because load has a validation step save does not, so one shared
//! enum would advertise failure modes a caller cannot encounter on the side it is using.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("I/O error reading {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("JSON parse error in {path:?}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("validation error in {path:?}: {message}")]
    Validation { path: PathBuf, message: String },

    #[error("cannot derive a light-state name from path {path:?}: missing or non-UTF-8 file stem")]
    BadName { path: PathBuf },
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("I/O error writing {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("JSON serialize error for {path:?}: {source}")]
    Serialize {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}
