//! The recent-projects list: the most-recently-opened content roots, persisted across runs.
//!
//! Two layers, split so the list logic stays testable without a filesystem:
//! - [`Recents`], a pure most-recent-first list with push / dedup / cap / clear and JSON
//!   (de)serialization. No egui, no I/O - `action::handle` mutates it (the single writer) and the
//!   File menu reads it.
//! - the edge: [`load`] and [`save`], which derive an OS config path from the environment and use
//!   `std::fs`. Dependency-free (no serde, no `directories` crate): the file is a flat JSON array of
//!   path strings, hand-encoded below.
//!
//! The file lives at an OS config path - `%APPDATA%\wok\recent.json` on Windows, then
//! `$XDG_CONFIG_HOME/wok` or `~/.config/wok` elsewhere. Persistence is best-effort: a missing or
//! malformed file reads as an empty list and a failed write is dropped, so the editor never fails to
//! start over its MRU list.

use std::fmt::Write as _;
use std::path::PathBuf;

/// The most-recent-first list never grows past this; the oldest entries fall off the end.
const CAP: usize = 10;

/// The editor's config directory name and the recents file within it.
const APP_DIR: &str = "wok";
const RECENT_FILE: &str = "recent.json";

/// The recent project paths, most-recent first, deduplicated by path and capped at [`CAP`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recents {
    paths: Vec<PathBuf>,
}

impl Recents {
    /// Build from an ordered list (most-recent first), applying dedup (keeping the first, most-recent
    /// occurrence) and the cap. A list read from disk is normalized the same way one grown by [`push`]
    /// is, so a hand-edited file with duplicates or too many entries lands in a valid state.
    pub fn from_paths(paths: impl IntoIterator<Item = PathBuf>) -> Recents {
        let mut out: Vec<PathBuf> = Vec::new();
        for path in paths {
            if !out.contains(&path) {
                out.push(path);
            }
        }
        out.truncate(CAP);
        Recents { paths: out }
    }

    /// Record `path` as the most recently opened: move it to the front, dropping any earlier copy, and
    /// trim the oldest entries past the cap. Re-pushing the front path is a no-op on order.
    pub fn push(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        self.paths.retain(|p| p != &path);
        self.paths.insert(0, path);
        self.paths.truncate(CAP);
    }

    /// The recent paths, most-recent first.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Whether nothing has been opened yet (the menu shows a disabled placeholder when so).
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Encode as a JSON array of path strings, one per line for a readable, diff-friendly file. A path
    /// that is not valid UTF-8 cannot be encoded and is skipped; in practice the folder picker yields
    /// encodable paths.
    pub fn to_json(&self) -> String {
        let encodable: Vec<&str> = self.paths.iter().filter_map(|p| p.to_str()).collect();
        let mut s = String::from("[\n");
        for (i, path) in encodable.iter().enumerate() {
            s.push_str("  ");
            encode_json_string(path, &mut s);
            if i + 1 < encodable.len() {
                s.push(',');
            }
            s.push('\n');
        }
        s.push_str("]\n");
        s
    }

    /// Parse a JSON array of strings into a normalized list. Tolerant: anything that is not a
    /// well-formed array of strings yields an empty list, so a corrupt file never breaks startup.
    pub fn from_json(text: &str) -> Recents {
        match parse_string_array(text) {
            Some(strings) => Recents::from_paths(strings.into_iter().map(PathBuf::from)),
            None => Recents::default(),
        }
    }
}

// ---- the edge: file I/O and the OS config path ----

/// Load the recent-projects list from disk, normalized. A missing or malformed file, or no config
/// directory at all, reads as an empty list.
pub fn load() -> Recents {
    let Some(path) = file_path() else { return Recents::default() };
    match std::fs::read_to_string(&path) {
        Ok(text) => Recents::from_json(&text),
        Err(_) => Recents::default(),
    }
}

/// Save the recent-projects list to disk, best-effort. Creates the config directory if needed; any
/// failure (no config dir, a write error) is dropped - persistence is a convenience, not a guarantee.
pub fn save(recents: &Recents) {
    let Some(path) = file_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, recents.to_json());
}

/// The recents file path, or `None` when no config directory can be derived from the environment (in
/// which case the editor runs without persisting recents).
fn file_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(RECENT_FILE))
}

/// The editor's OS config directory, read from the environment.
fn config_dir() -> Option<PathBuf> {
    config_dir_from(
        std::env::var_os("APPDATA").map(PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// The config-directory precedence, pure over its inputs so the Windows-first ordering is testable
/// without touching the process environment: `%APPDATA%` wins (Windows), then `$XDG_CONFIG_HOME`, then
/// `~/.config` (other platforms).
fn config_dir_from(appdata: Option<PathBuf>, xdg: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(appdata) = appdata {
        return Some(appdata.join(APP_DIR));
    }
    if let Some(xdg) = xdg {
        return Some(xdg.join(APP_DIR));
    }
    home.map(|home| home.join(".config").join(APP_DIR))
}

// ---- minimal JSON for an array of strings (no serde) ----

/// Append `s` as a quoted, escaped JSON string. Escapes the two structural characters (quote and
/// backslash - the latter pervasive in Windows paths) and the C0 control range, which keeps the
/// output valid JSON for any UTF-8 input.
fn encode_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Writing to a String is infallible, so the formatting Result is discarded.
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Parse `[ "a", "b", ... ]` into its strings, tolerant of surrounding whitespace. Returns `None`
/// unless the text is exactly an array of strings, so the caller falls back to an empty list. Only the
/// subset the recents file uses is supported (arrays, strings, JSON string escapes), not full JSON.
fn parse_string_array(text: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    skip_ws(&chars, &mut i);
    if chars.get(i) != Some(&'[') {
        return None;
    }
    i += 1;
    let mut out = Vec::new();
    skip_ws(&chars, &mut i);
    if chars.get(i) == Some(&']') {
        i += 1;
    } else {
        loop {
            skip_ws(&chars, &mut i);
            if chars.get(i) != Some(&'"') {
                return None;
            }
            out.push(parse_string(&chars, &mut i)?);
            skip_ws(&chars, &mut i);
            match chars.get(i) {
                Some(&',') => i += 1,
                Some(&']') => {
                    i += 1;
                    break;
                }
                _ => return None,
            }
        }
    }
    skip_ws(&chars, &mut i);
    // Trailing content after the array means this is not the file we wrote; reject it.
    if i == chars.len() { Some(out) } else { None }
}

/// Consume the JSON string starting at `chars[*i]` (which must be the opening quote), returning its
/// decoded value and leaving `*i` past the closing quote. `None` on an unterminated or malformed
/// string.
fn parse_string(chars: &[char], i: &mut usize) -> Option<String> {
    *i += 1; // opening quote
    let mut s = String::new();
    loop {
        let c = *chars.get(*i)?;
        *i += 1;
        match c {
            '"' => return Some(s),
            '\\' => {
                let e = *chars.get(*i)?;
                *i += 1;
                match e {
                    '"' => s.push('"'),
                    '\\' => s.push('\\'),
                    '/' => s.push('/'),
                    'n' => s.push('\n'),
                    'r' => s.push('\r'),
                    't' => s.push('\t'),
                    'b' => s.push('\u{0008}'),
                    'f' => s.push('\u{000c}'),
                    // Basic multilingual plane only; a lone surrogate or invalid scalar is dropped to
                    // keep parsing total. Paths do not carry these.
                    'u' => {
                        if let Some(ch) = char::from_u32(parse_hex4(chars, i)?) {
                            s.push(ch);
                        }
                    }
                    _ => return None,
                }
            }
            c => s.push(c),
        }
    }
}

/// Read exactly four hex digits as a code point, advancing `*i`. `None` on a non-hex digit or a short
/// read.
fn parse_hex4(chars: &[char], i: &mut usize) -> Option<u32> {
    let mut v = 0u32;
    for _ in 0..4 {
        let c = *chars.get(*i)?;
        *i += 1;
        v = v * 16 + c.to_digit(16)?;
    }
    Some(v)
}

/// Advance `*i` past any JSON whitespace.
fn skip_ws(chars: &[char], i: &mut usize) {
    while matches!(chars.get(*i), Some(' ' | '\t' | '\n' | '\r')) {
        *i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a list from string slices, most-recent first, through the same normalization a load uses.
    fn recents_of(paths: &[&str]) -> Recents {
        Recents::from_paths(paths.iter().map(PathBuf::from))
    }

    // ---- list logic ----

    #[test]
    fn push_puts_the_newest_at_the_front() {
        let mut r = Recents::default();
        r.push("a");
        r.push("b");
        assert_eq!(r.paths(), &[PathBuf::from("b"), PathBuf::from("a")]);
    }

    #[test]
    fn push_dedups_moving_an_existing_path_to_the_front() {
        let mut r = recents_of(&["a", "b", "c"]); // most-recent first: a, b, c
        r.push("c");
        assert_eq!(r.paths(), &[PathBuf::from("c"), PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(r.paths().len(), 3, "the re-pushed path moves, it does not duplicate");
    }

    #[test]
    fn push_caps_the_list_dropping_the_oldest() {
        let mut r = Recents::default();
        for i in 0..(CAP + 5) {
            r.push(format!("p{i}"));
        }
        assert_eq!(r.paths().len(), CAP);
        // The newest sits at the front; everything older than the cap window has fallen off the back.
        assert_eq!(r.paths()[0], PathBuf::from(format!("p{}", CAP + 4)));
        assert_eq!(r.paths().last().unwrap(), &PathBuf::from("p5"));
    }

    #[test]
    fn from_paths_dedups_and_caps_preserving_order() {
        let mut input: Vec<String> = vec!["a".into(), "a".into(), "b".into()];
        for i in 0..CAP {
            input.push(format!("x{i}"));
        }
        let r = Recents::from_paths(input.into_iter().map(PathBuf::from));
        assert_eq!(r.paths().len(), CAP);
        assert_eq!(r.paths()[0], PathBuf::from("a"));
        assert_eq!(r.paths()[1], PathBuf::from("b"));
    }

    // ---- JSON round-trip ----

    #[test]
    fn json_round_trips_including_backslash_and_quote() {
        // Windows-style backslashes and an embedded quote exercise the escaping both ways.
        let r = recents_of(&[r"C:\Users\dev\game", r#"weird"name"#, "plain/unix"]);
        assert_eq!(Recents::from_json(&r.to_json()), r);
    }

    #[test]
    fn to_json_escapes_backslashes() {
        let json = recents_of(&[r"C:\game"]).to_json();
        assert!(json.contains(r"C:\\game"), "a backslash should be JSON-escaped: {json}");
    }

    #[test]
    fn from_json_reads_an_empty_array_as_empty() {
        assert_eq!(Recents::from_json("[]"), Recents::default());
        assert_eq!(Recents::from_json("[\n]\n"), Recents::default());
    }

    #[test]
    fn from_json_is_tolerant_of_garbage() {
        for bad in ["", "not json", "{}", "[1, 2]", "[\"unterminated", "[\"a\" \"b\"]"] {
            assert_eq!(Recents::from_json(bad), Recents::default(), "garbage {bad:?} should read as empty");
        }
    }

    #[test]
    fn from_json_normalizes_dedup_and_cap() {
        // A file with a duplicate and over-cap entries is normalized on load.
        let mut entries = vec![String::from("a"), String::from("a")];
        for i in 0..CAP {
            entries.push(format!("x{i}"));
        }
        let mut json = String::from("[");
        for (i, e) in entries.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json.push('"');
            json.push_str(e);
            json.push('"');
        }
        json.push(']');
        let r = Recents::from_json(&json);
        assert_eq!(r.paths().len(), CAP);
        assert_eq!(r.paths()[0], PathBuf::from("a"));
    }

    // ---- config path precedence (pure over the environment) ----

    #[test]
    fn config_dir_prefers_appdata_on_windows() {
        let appdata = PathBuf::from(r"C:\Users\dev\AppData\Roaming");
        let dir = config_dir_from(
            Some(appdata.clone()),
            Some(PathBuf::from("/home/dev/.config")),
            Some(PathBuf::from("/home/dev")),
        );
        assert_eq!(dir, Some(appdata.join("wok")));
    }

    #[test]
    fn config_dir_falls_back_to_xdg_then_home() {
        let xdg = config_dir_from(None, Some(PathBuf::from("/cfg")), Some(PathBuf::from("/home/dev")));
        assert_eq!(xdg, Some(PathBuf::from("/cfg/wok")));
        let home = config_dir_from(None, None, Some(PathBuf::from("/home/dev")));
        assert_eq!(home, Some(PathBuf::from("/home/dev/.config/wok")));
        assert_eq!(config_dir_from(None, None, None), None);
    }
}
