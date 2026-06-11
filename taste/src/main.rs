//! taste: a playable demo, the second application on the engine.
//!
//! The seed of game-side composition per HLD principle 5: the fixed-timestep loop, the player's
//! locomotion, and the follow camera all live here, composed from wok-physics's pure math, running
//! against content the wok editor authored. It is also the Level 2 replay harness made watchable -
//! the same per-step composition the locomotion tests drive headless, now under a renderer (and
//! still driven headless by `crate::replay`).
//!
//! Run with `cargo run -p taste [content-dir]` (default `./content`, the editor's convention).
//! taste never writes content: with no scene on disk it asks you to run the editor first and exits.
//!
//! Controls: WASD to move relative to the camera, space to jump, the mouse to orbit the camera
//! (always live; the cursor is captured and hidden while the game runs), F1 to toggle the hitbox
//! overlay, Esc to quit. On a gamepad: sticks, south button, and Select/Back to quit.
//!
//! Errors: `Box<dyn Error>` to `main`, which prints and exits - the wok precedent; nothing here
//! inspects failures programmatically, so no error enum (and no anyhow dependency) has earned its
//! place. No tracing subscriber for the same reason as the editor: no engine crate emits tracing
//! events yet.

mod air;
#[cfg(test)]
mod air_feel;
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
mod landing;
#[cfg(test)]
mod precision;
#[cfg(test)]
mod replay;
mod sim;
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

/// Everything before the window opens: parse the CLI, load the editor-authored content, and build
/// the app. A missing scene is the one failure with advice attached: taste never generates content,
/// so the editor has to have run first.
fn start(args: &[String]) -> Result<app::TasteApp, Box<dyn Error>> {
    let root = cli::parse_args(args)?;
    let paths = content::ContentPaths::new(root);

    if !paths.scene().exists() {
        return Err(format!(
            "no scene at {}; run the wok editor first to generate content",
            paths.scene().display()
        )
        .into());
    }

    let loaded = content::load_all(&paths)?;
    println!(
        "taste: scene {:?}: {} chunk(s), {} prefab(s)",
        loaded.scene.name,
        loaded.chunks.len(),
        loaded.prefabs.len()
    );
    println!("taste: controls: WASD or left stick to move, space or south button to jump,");
    println!("taste:           mouse or right stick to look (cursor is captured while running),");
    println!("taste:           F1 to toggle the hitbox overlay, Esc or Select/Back to quit");

    app::TasteApp::new(loaded)
}
