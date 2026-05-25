use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use pantry::serde::de::Error as _;
use pantry::serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::slug::{InvalidSlug, Slug};

// Generate a per-kind asset ID type with `{ serial, slug }` shape. Equality, hashing, and
// ordering are based on the `serial` only; the `slug` is debug surface, not identity. That
// keeps asset renames safe: an existing reference compares equal across a rename because the
// serial does not change. The macro is what guarantees these impls stay consistent across all
// five asset ID kinds; deriving `PartialEq`/`Hash`/`Ord` on any of them would be a regression.
macro_rules! define_asset_id {
    ($name:ident) => {
        #[derive(Clone)]
        pub struct $name {
            serial: u32,
            slug: Slug,
        }

        impl $name {
            pub fn new(slug: Slug, serial: u32) -> Self {
                Self { serial, slug }
            }

            pub fn serial(&self) -> u32 {
                self.serial
            }

            pub fn slug(&self) -> &Slug {
                &self.slug
            }
        }

        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                self.serial == other.serial
            }
        }

        impl Eq for $name {}

        impl Hash for $name {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.serial.hash(state);
            }
        }

        impl PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        impl Ord for $name {
            fn cmp(&self, other: &Self) -> Ordering {
                self.serial.cmp(&other.serial)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "{}({}-{})",
                    stringify!($name),
                    self.slug.as_str(),
                    self.serial
                )
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}-{}", self.slug.as_str(), self.serial)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let token = format!("{}-{}", self.slug.as_str(), self.serial);
                serializer.serialize_str(&token)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let token = String::deserialize(deserializer)?;
                let (slug, serial) = parse_asset_token(&token).map_err(D::Error::custom)?;
                Ok($name::new(slug, serial))
            }
        }
    };
}

define_asset_id!(MeshId);
define_asset_id!(AudioCueId);
define_asset_id!(AnimationId);
define_asset_id!(VoiceLineId);
define_asset_id!(LightStateRef);

/// Parse an asset reference token of shape `slug-serial`. The split happens at the last
/// hyphen because the slug part may itself contain hyphens; the suffix is parsed as `u32`,
/// and the prefix is validated against `Slug::new`.
fn parse_asset_token(token: &str) -> Result<(Slug, u32), AssetTokenError> {
    let (slug_part, serial_part) = token
        .rsplit_once('-')
        .ok_or_else(|| AssetTokenError::NoSeparator(token.to_string()))?;
    let serial: u32 =
        serial_part
            .parse()
            .map_err(|_| AssetTokenError::NonNumericSerial {
                token: token.to_string(),
                serial: serial_part.to_string(),
            })?;
    let slug = Slug::new(slug_part).map_err(|reason| AssetTokenError::InvalidSlug {
        token: token.to_string(),
        reason,
    })?;
    Ok((slug, serial))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AssetTokenError {
    NoSeparator(String),
    NonNumericSerial { token: String, serial: String },
    InvalidSlug { token: String, reason: InvalidSlug },
}

impl fmt::Display for AssetTokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetTokenError::NoSeparator(token) => {
                write!(f, "asset token {token:?} has no '-' separator")
            }
            AssetTokenError::NonNumericSerial { token, serial } => {
                write!(
                    f,
                    "asset token {token:?} has non-numeric serial {serial:?}"
                )
            }
            AssetTokenError::InvalidSlug { token, reason } => {
                write!(f, "asset token {token:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for AssetTokenError {}
