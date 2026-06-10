//! Command-line parsing for the wok editor.
//!
//! One optional positional argument: the content directory, defaulting to `./content`. No flags
//! yet; anything that looks like one is rejected so a future flag cannot be silently swallowed as
//! a directory name today. Pure (slice of strings in, path or message out) so the parse is unit
//! testable without a process.

use std::path::PathBuf;

/// Default content directory, relative to the working directory.
const DEFAULT_CONTENT_DIR: &str = "content";

/// Parse the arguments after the program name into the content-directory path.
pub fn parse_args(args: &[String]) -> Result<PathBuf, String> {
    match args {
        [] => Ok(PathBuf::from(DEFAULT_CONTENT_DIR)),
        [dir] if !dir.starts_with('-') => Ok(PathBuf::from(dir)),
        [flag] => Err(format!("unknown flag {flag:?}; usage: wok [content-dir]")),
        _ => Err(format!("expected at most one argument, got {}; usage: wok [content-dir]", args.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn no_args_defaults_to_content() {
        assert_eq!(parse_args(&[]).unwrap(), PathBuf::from("content"));
    }

    #[test]
    fn one_arg_is_the_content_dir() {
        let args = strings(&["assets/world"]);
        assert_eq!(parse_args(&args).unwrap(), PathBuf::from("assets/world"));
    }

    #[test]
    fn a_flag_is_rejected() {
        let args = strings(&["--content"]);
        let err = parse_args(&args).unwrap_err();
        assert!(err.contains("--content"), "message should name the flag: {err}");
    }

    #[test]
    fn two_args_are_rejected() {
        let args = strings(&["a", "b"]);
        let err = parse_args(&args).unwrap_err();
        assert!(err.contains("at most one"), "message should explain the arity: {err}");
    }
}
