use std::sync::Arc;

use pantry::serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Validated path-safe identifier. Lowercase ASCII alphanumeric plus `_`, `-`, `/`. No leading
/// slash. Cloned cheaply via `Arc`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Slug(Arc<str>);

impl Slug {
    pub fn new(s: &str) -> Result<Self, InvalidSlug> {
        if s.is_empty() {
            return Err(InvalidSlug::Empty);
        }
        if s.starts_with('/') {
            return Err(InvalidSlug::LeadingSlash);
        }
        for ch in s.chars() {
            if !is_slug_char(ch) {
                return Err(InvalidSlug::InvalidChar(ch));
            }
        }
        Ok(Slug(Arc::from(s)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Slug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn is_slug_char(ch: char) -> bool {
    ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-' || ch == '/'
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidSlug {
    Empty,
    LeadingSlash,
    InvalidChar(char),
}

impl std::fmt::Display for InvalidSlug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvalidSlug::Empty => f.write_str("slug is empty"),
            InvalidSlug::LeadingSlash => f.write_str("slug has leading slash"),
            InvalidSlug::InvalidChar(ch) => write!(f, "slug contains invalid character {ch:?}"),
        }
    }
}

impl std::error::Error for InvalidSlug {}

impl Serialize for Slug {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Slug {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use pantry::serde::de::Error;
        let s = String::deserialize(deserializer)?;
        Slug::new(&s).map_err(D::Error::custom)
    }
}
