use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use wok_scene::pantry::serde_json;
use wok_scene::{AnimationId, AudioCueId, LightStateRef, MeshId, Slug, VoiceLineId};

fn parse(token: &str) -> Result<MeshId, serde_json::Error> {
    let json = format!("\"{token}\"");
    serde_json::from_str::<MeshId>(&json)
}

#[test]
fn parses_simple_slug_and_serial() {
    let id = parse("wooden-crate-267").unwrap();
    assert_eq!(id.serial(), 267);
    assert_eq!(id.slug().as_str(), "wooden-crate");
}

#[test]
fn parses_with_hyphens_in_slug_via_last_hyphen_split() {
    let id = parse("version-2-5").unwrap();
    assert_eq!(id.serial(), 5);
    assert_eq!(id.slug().as_str(), "version-2");
}

#[test]
fn parses_zero_serial() {
    let id = parse("foo-0").unwrap();
    assert_eq!(id.serial(), 0);
    assert_eq!(id.slug().as_str(), "foo");
}

#[test]
fn rejects_token_without_separator() {
    assert!(parse("foo").is_err());
}

#[test]
fn rejects_non_numeric_serial() {
    assert!(parse("foo-bar").is_err());
}

#[test]
fn rejects_uppercase_slug_after_split() {
    assert!(parse("WOODEN-CRATE-267").is_err());
}

#[test]
fn round_trip_through_json() {
    let id = MeshId::new(Slug::new("rusty-pipe").unwrap(), 42);
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, "\"rusty-pipe-42\"");
    let back: MeshId = serde_json::from_str(&json).unwrap();
    assert_eq!(back, id);
}

#[test]
fn equality_is_serial_only() {
    let a = MeshId::new(Slug::new("apple").unwrap(), 7);
    let b = MeshId::new(Slug::new("banana").unwrap(), 7);
    let c = MeshId::new(Slug::new("apple").unwrap(), 8);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn hashing_is_serial_only() {
    let a = MeshId::new(Slug::new("apple").unwrap(), 7);
    let b = MeshId::new(Slug::new("zucchini").unwrap(), 7);
    let mut ha = DefaultHasher::new();
    let mut hb = DefaultHasher::new();
    a.hash(&mut ha);
    b.hash(&mut hb);
    assert_eq!(ha.finish(), hb.finish());

    let mut map: HashMap<MeshId, &str> = HashMap::new();
    map.insert(a, "first");
    map.insert(b, "second");
    assert_eq!(map.len(), 1);
}

#[test]
fn debug_format_includes_kind_name() {
    let id = MeshId::new(Slug::new("wooden-crate").unwrap(), 42);
    assert_eq!(format!("{id:?}"), "MeshId(wooden-crate-42)");
}

#[test]
fn display_format_is_slug_dash_serial() {
    let id = MeshId::new(Slug::new("wooden-crate").unwrap(), 42);
    assert_eq!(format!("{id}"), "wooden-crate-42");
}

// The four tests below guard against a macro-generated regression where one asset ID kind
// drifts from MeshId's behavior - e.g. someone derives PartialEq on AnimationId directly
// instead of using define_asset_id!, accidentally bringing slug into identity.

#[test]
fn audio_cue_id_equality_and_hash_are_serial_only() {
    let a = AudioCueId::new(Slug::new("alpha").unwrap(), 9);
    let b = AudioCueId::new(Slug::new("beta").unwrap(), 9);
    assert_eq!(a, b);
    let mut ha = DefaultHasher::new();
    let mut hb = DefaultHasher::new();
    a.hash(&mut ha);
    b.hash(&mut hb);
    assert_eq!(ha.finish(), hb.finish());
}

#[test]
fn animation_id_equality_and_hash_are_serial_only() {
    let a = AnimationId::new(Slug::new("idle").unwrap(), 3);
    let b = AnimationId::new(Slug::new("run").unwrap(), 3);
    assert_eq!(a, b);
    let mut ha = DefaultHasher::new();
    let mut hb = DefaultHasher::new();
    a.hash(&mut ha);
    b.hash(&mut hb);
    assert_eq!(ha.finish(), hb.finish());
}

#[test]
fn voice_line_id_equality_and_hash_are_serial_only() {
    let a = VoiceLineId::new(Slug::new("greet-friendly").unwrap(), 11);
    let b = VoiceLineId::new(Slug::new("greet-hostile").unwrap(), 11);
    assert_eq!(a, b);
    let mut ha = DefaultHasher::new();
    let mut hb = DefaultHasher::new();
    a.hash(&mut ha);
    b.hash(&mut hb);
    assert_eq!(ha.finish(), hb.finish());
}

#[test]
fn light_state_ref_equality_and_hash_are_serial_only() {
    let a = LightStateRef::new(Slug::new("warehouse-day").unwrap(), 4);
    let b = LightStateRef::new(Slug::new("warehouse-night").unwrap(), 4);
    assert_eq!(a, b);
    let mut ha = DefaultHasher::new();
    let mut hb = DefaultHasher::new();
    a.hash(&mut ha);
    b.hash(&mut hb);
    assert_eq!(ha.finish(), hb.finish());
}
