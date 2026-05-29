//! Error types for the load and save surface.
//!
//! `LoadError` is the load-side enum the brief names; it covers I/O failure, JSON parse
//! failure, and in-file validation failures (e.g. a prefab whose `default_state` does not
//! match any of its states). Validation here is strictly intra-file - cross-file resolution
//! is the scanner's job in wok-content/wok-shell, not this crate's.
//!
//! `SaveError` is the symmetric save-side enum. The two are kept separate because the
//! canon's preference is one narrow error per failure domain rather than a god-enum; load
//! includes a validation step that save does not, so a shared enum would mislead callers.
//!
//! The heightmap binary format (see `crate::heightmap_io`) adds load-side failures JSON never
//! has - a bad magic number, an unsupported version, a truncated file, a non-UTF-8 surface
//! name - plus `Heightmap`, which wraps a `HeightmapError` once the loader has a path to attach.
//! Save still fails only on I/O, so `SaveError` is unchanged: the binary encoder builds a
//! `Vec<u8>` by hand and cannot fail short of the write itself.

use std::path::PathBuf;

use crate::heightmap::HeightmapError;

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

    #[error("{path:?} is not a wok heightmap file (bad magic)")]
    BadMagic { path: PathBuf },

    #[error("unsupported heightmap version {version} in {path:?}")]
    UnsupportedVersion { path: PathBuf, version: u16 },

    #[error("truncated heightmap file {path:?}: ran out of bytes mid-record")]
    Truncated { path: PathBuf },

    #[error("invalid UTF-8 in a heightmap surface tag in {path:?}: {source}")]
    Utf8 {
        path: PathBuf,
        #[source]
        source: std::str::Utf8Error,
    },

    #[error("invalid heightmap data in {path:?}: {source}")]
    Heightmap {
        path: PathBuf,
        #[source]
        source: HeightmapError,
    },
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
