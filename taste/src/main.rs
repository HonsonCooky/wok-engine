//! taste: a playable demo, the second application on the engine.
//!
//! The seed of game-side composition per HLD principle 5: the fixed-timestep loop, the player's
//! locomotion, and the follow camera all live here, composed from wok-physics's pure math, running
//! against content the wok editor authored. It is also the Level 2 replay harness made watchable -
//! the same per-step composition the locomotion tests drive headless, now under a renderer (and
//! still driven headless by `crate::replay`).
//!
//! Run with `cargo run -p taste [content-dir]`. With no argument it plays taste's own `assets/` -
//! the demo content taste owns in-repo - from anywhere; the wok editor opens `taste/` to author it.
//! taste never writes content: with no scene on disk it asks you to run the editor first and exits.
//!
//! Controls: WASD to move relative to the camera, space to jump, the mouse to orbit the camera
//! (always live; the cursor is captured and hidden while the game runs), F1 to toggle the hitbox
//! overlay, Esc to quit. On a gamepad: sticks, south button, and Select/Back to quit.
//!
//! Feel tuning is live: the gameplay and camera numbers a play-test verdict moves live in
//! `taste/tuning.json` (`crate::tuning`), loaded at startup (written from the shipped defaults the
//! first time it is missing) and hot-reloaded while playing, so the human iterates feel without
//! rebuilds. A parse error keeps the previous values and says so; it never crashes the session.
//!
//! Errors: `Box<dyn Error>` to `main`, which prints and exits - the wok precedent; nothing here
//! inspects failures programmatically, so no error enum (and no anyhow dependency) has earned its
//! place. No tracing subscriber for the same reason as the editor: no engine crate emits tracing
//! events yet.

mod app;
mod cli;
mod clock;
mod constants;
mod content;
mod debug;
#[cfg(test)]
mod diagnose;
mod fade;
mod follow;
mod intent;
mod jump;
#[cfg(test)]
mod replay;
mod sim;
mod slide;
mod tuning;
mod world;

use std::error::Error;

use wok_platform::Desc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match start(&args) {
        // run() owns the OS event loop and returns when the window closes.
        Ok(game) => wok_platform::run(game, Desc { title: "taste", width: 0, height: 0, vsync: true }),
        Err(err) => {
            eprintln!("taste: {err}");
            std::process::exit(1);
        }
    }
}

/// The tracked feel-tuning file, resolved to taste's crate directory (baked in at compile time) like
/// the content root, so both find their files from any working directory: a bare `cargo run -p taste`
/// picks up the authored tuning. The hot-reload watcher watches this file's directory (taste's crate
/// dir, where tuning.json lives), so reload works from any cwd too. Tracked in git as the authored
/// feel record.
const TUNING_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tuning.json");

/// Everything before the window opens: parse the CLI, load the editor-authored content, load (or
/// first-run write) the feel tuning, and build the app. A missing scene is the one failure with
/// advice attached: taste never generates content, so the editor has to have run first.
fn start(args: &[String]) -> Result<app::TasteApp, Box<dyn Error>> {
    let root = cli::parse_args(args)?;
    let layout = wok_scene::ContentLayout::new(root);

    let loaded = content::load_all(&layout)?;
    println!(
        "taste: scene {:?}: {} chunk(s), {} prefab(s)",
        loaded.scene.name,
        loaded.chunks.len(),
        loaded.prefabs.len()
    );

    let tuning_path = std::path::PathBuf::from(TUNING_PATH);
    let tuning = load_tuning(&tuning_path);

    println!("taste: controls: WASD or left stick to move, space or south button to jump,");
    println!("taste:           mouse or right stick to look (cursor is captured while running),");
    println!("taste:           F1 to toggle the hitbox overlay, Esc or Select/Back to quit");
    println!("taste: feel tuning: {} (edit and save to live-reload while playing)", tuning_path.display());

    app::TasteApp::new(loaded, tuning, tuning_path)
}

/// Load the feel tuning, never failing the launch over it. Present and valid: use it (any broken
/// relationships print as warnings, but play continues). Present and unparseable: warn and fall
/// back to the shipped defaults, leaving the file untouched so the human can fix it. Absent: write
/// the defaults out as the first-run record and use them. Validation warnings print on load so a
/// detuned file announces itself before play, exactly as it will on reload.
fn load_tuning(path: &std::path::Path) -> tuning::Tuning {
    let loaded = if path.exists() {
        match tuning::load(path) {
            Ok(t) => {
                println!("taste: loaded feel tuning from {}", path.display());
                t
            }
            Err(err) => {
                println!("taste: tuning at {} did not parse ({err}); using the shipped defaults", path.display());
                tuning::Tuning::default()
            }
        }
    } else {
        let defaults = tuning::Tuning::default();
        match tuning::save(path, &defaults) {
            Ok(()) => println!("taste: no tuning file; wrote the shipped defaults to {}", path.display()),
            Err(err) => println!("taste: could not write defaults to {} ({err}); using them in memory", path.display()),
        }
        defaults
    };
    for warning in loaded.validate() {
        println!("taste: tuning warning: {warning}");
    }
    loaded
}
