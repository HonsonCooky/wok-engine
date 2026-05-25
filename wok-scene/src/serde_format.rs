//! File-format version handling. Authored files carry a top-level `_format` integer that
//! lives outside the typed authored data; this module wraps that field on the write side
//! and strips it on the read side so the authored types stay clean.
//!
//! Note on the serde flatten + deny_unknown_fields interaction: serde's `flatten` does not
//! cooperate with `deny_unknown_fields` on either the outer or the flattened struct (this
//! is documented in serde). So we use flatten on the SERIALIZE side only (where deny is
//! irrelevant) and a two-step parse-then-strip on the DESERIALIZE side, which lets the
//! authored types keep their `deny_unknown_fields` posture and still tolerate the file's
//! `_format` field.

use pantry::serde::Serialize;

/// Current file-format version. Bumped together with the authored data; loader rejects any
/// other value. A future bump would ship with a one-shot migration helper.
pub(crate) const CURRENT_FORMAT: u32 = 1;

/// Serialization-only wrapper that prepends `_format` to an inner struct's fields. The
/// `flatten` attribute inlines the inner fields at the JSON top level so the resulting
/// file looks like `{"_format": 1, ...inner...}`.
#[derive(Serialize)]
#[serde(crate = "pantry::serde")]
pub(crate) struct Versioned<T> {
    #[serde(rename = "_format")]
    pub format: u32,
    #[serde(flatten)]
    pub inner: T,
}
