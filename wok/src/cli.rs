//! Command-line parsing for the wok editor.
//!
//! One optional positional argument: a project folder to open on startup. With no argument the
//! editor opens to no project (open one from the File menu). No flags yet; anything that looks like
//! one is rejected so a future flag cannot be silently swallowed as a folder name today. Pure
//! (slice of strings in, optional path or message out) so the parse is unit testable without a
//! process.

use std::path::PathBuf;

/// Parse the arguments after the program name into an optional project folder. `None` means no
/// folder was given, so the editor starts with no project open.
pub fn parse_args(args: &[String]) -> Result<Option<PathBuf>, String> {
    match args {
        [] => Ok(None),
        [dir] if !dir.starts_with('-') => Ok(Some(PathBuf::from(dir))),
        [flag] => Err(format!("unknown flag {flag:?}; usage: wok [project-dir]")),
        _ => Err(format!("expected at most one argument, got {}; usage: wok [project-dir]", args.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn no_args_opens_no_project() {
        assert_eq!(parse_args(&[]).unwrap(), None);
    }

    #[test]
    fn one_arg_is_the_project_dir() {
        let args = strings(&["assets/world"]);
        assert_eq!(parse_args(&args).unwrap(), Some(PathBuf::from("assets/world")));
    }

    #[test]
    fn a_flag_is_rejected() {
        let args = strings(&["--project"]);
        let err = parse_args(&args).unwrap_err();
        assert!(err.contains("--project"), "message should name the flag: {err}");
    }

    #[test]
    fn two_args_are_rejected() {
        let args = strings(&["a", "b"]);
        let err = parse_args(&args).unwrap_err();
        assert!(err.contains("at most one"), "message should explain the arity: {err}");
    }
}
