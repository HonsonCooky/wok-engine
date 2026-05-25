use std::path::PathBuf;

use pantry::serde_json;

use crate::ids::PrefabId;

/// Errors produced by `slice_chunk`. Slicing is fail-fast: one error aborts the whole chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SliceError {
    UnknownPrefab(PrefabId),
    UnknownState {
        prefab: PrefabId,
        state: String,
    },
    InvalidShape {
        placement_index: usize,
        shape_index: usize,
        reason: String,
    },
}

impl std::fmt::Display for SliceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SliceError::UnknownPrefab(id) => write!(f, "unknown prefab {}", id.0),
            SliceError::UnknownState { prefab, state } => {
                write!(f, "prefab {} has no state {state:?}", prefab.0)
            }
            SliceError::InvalidShape {
                placement_index,
                shape_index,
                reason,
            } => write!(
                f,
                "invalid shape at placement {placement_index} shape {shape_index}: {reason}"
            ),
        }
    }
}

impl std::error::Error for SliceError {}

/// Errors produced by `load_*` functions. The `path` field on every variant aids
/// diagnostics by pointing at the file that failed.
///
/// `MissingFormat` and `UnsupportedVersion` are deliberately distinct: a typo like
/// `"_formate": 1` is valid JSON but does not declare our header (`MissingFormat`), while
/// `"_format": 99` declares an explicit version we do not support (`UnsupportedVersion`).
#[derive(Debug)]
pub enum LoadError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    MissingFormat {
        path: PathBuf,
    },
    UnsupportedVersion {
        path: PathBuf,
        found: u32,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io { path, source } => {
                write!(f, "I/O error reading {}: {source}", path.display())
            }
            LoadError::Parse { path, source } => {
                write!(f, "parse error in {}: {source}", path.display())
            }
            LoadError::MissingFormat { path } => write!(
                f,
                "missing or non-integer `_format` field in {}",
                path.display()
            ),
            LoadError::UnsupportedVersion { path, found } => write!(
                f,
                "unsupported _format {found} in {} (expected 1)",
                path.display()
            ),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Io { source, .. } => Some(source),
            LoadError::Parse { source, .. } => Some(source),
            LoadError::MissingFormat { .. } | LoadError::UnsupportedVersion { .. } => None,
        }
    }
}

/// Errors produced by `save_*` functions.
#[derive(Debug)]
pub enum SaveError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Encode {
        source: serde_json::Error,
    },
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Io { path, source } => {
                write!(f, "I/O error writing {}: {source}", path.display())
            }
            SaveError::Encode { source } => write!(f, "encode error: {source}"),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SaveError::Io { source, .. } => Some(source),
            SaveError::Encode { source } => Some(source),
        }
    }
}
