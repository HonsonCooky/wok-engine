use wok_scene::{InvalidSlug, Slug};

#[test]
fn empty_slug_rejected() {
    assert_eq!(Slug::new(""), Err(InvalidSlug::Empty));
}

#[test]
fn leading_slash_rejected() {
    assert_eq!(Slug::new("/enemies/grunt"), Err(InvalidSlug::LeadingSlash));
}

#[test]
fn uppercase_rejected() {
    assert!(matches!(
        Slug::new("Wooden-Crate"),
        Err(InvalidSlug::InvalidChar('W'))
    ));
}

#[test]
fn space_rejected() {
    assert!(matches!(
        Slug::new("wooden crate"),
        Err(InvalidSlug::InvalidChar(' '))
    ));
}

#[test]
fn lowercase_alphanumeric_underscore_hyphen_slash_accepted() {
    Slug::new("wooden-crate").unwrap();
    Slug::new("enemies/grunt").unwrap();
    Slug::new("version-2").unwrap();
    Slug::new("snake_case_thing").unwrap();
    Slug::new("a/b/c").unwrap();
    Slug::new("a").unwrap();
    Slug::new("0").unwrap();
}

#[test]
fn other_punctuation_rejected() {
    assert!(matches!(
        Slug::new("foo.bar"),
        Err(InvalidSlug::InvalidChar('.'))
    ));
    assert!(matches!(
        Slug::new("foo:bar"),
        Err(InvalidSlug::InvalidChar(':'))
    ));
}
