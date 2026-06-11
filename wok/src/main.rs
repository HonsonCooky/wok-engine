//! wok: the editor, the engine's reference application.
//!
//! v1 is the authoring loop over v0's viewport: see the scene's structure (scene tree), select
//! (tree click or viewport pick), edit (inspector), place and delete, and save - all against the
//! authored in-memory forms, re-transformed through wok-content per edit so the viewport always
//! draws the authored truth. The UI is egui, composed as a final render pass; egui is a
//! dependency of this application only, never of an engine crate.
//!
//! Run with `cargo run -p wok [content-dir]` (default `./content`). A first run against an empty
//! directory generates the sample scene through the engine's save paths; every later run loads
//! what is on disk. Editing the authored JSON or heightmap binaries while the editor runs updates
//! the viewport live.
//!
//! Errors: the application propagates `Box<dyn Error>` to `main`, which prints and exits. Per
//! canon an application may use `anyhow` instead, but nothing here inspects failures
//! programmatically, so the standard library's boxed error is the same capability without a new
//! dependency. There is no tracing subscriber yet for the same reason: no engine crate emits
//! tracing events today, so a subscriber would observe nothing; it arrives when the first crate
//! takes the tracing dependency.

mod app;
mod camera;
mod cli;
mod content;
mod details;
mod drag;
mod edit_ops;
mod glyphs;
mod gui;
mod input;
mod library;
mod lines;
mod model;
mod outline;
mod pages;
mod panels;
mod pick;
mod place;
mod reload;
mod sample;
mod status;
mod sync;
mod theme;
mod tree;

use std::error::Error;

use wok_platform::Desc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match start(&args) {
        // run() owns the OS event loop and returns when the window closes.
        Ok(editor) => wok_platform::run(editor, Desc { title: "wok", width: 0, height: 0, vsync: true }),
        Err(err) => {
            eprintln!("wok: {err}");
            std::process::exit(1);
        }
    }
}

/// Everything before the window opens: parse the CLI, generate sample content if the directory
/// has no scene, load and transform all of it, and build the app.
fn start(args: &[String]) -> Result<app::EditorApp, Box<dyn Error>> {
    let root = cli::parse_args(args)?;
    let paths = content::ContentPaths::new(root);

    if !paths.scene().exists() {
        println!("wok: no scene at {}; generating sample content", paths.scene().display());
        sample::generate(&paths)?;
    }

    // Canonicalize the root after it exists, so the watcher registration and the changed paths it
    // reports share one base and hot-reload classification can strip it exactly. Without this a
    // relative root never prefix-matches the absolute paths the OS watcher reports on Windows.
    let paths = content::ContentPaths::new(paths.root.canonicalize()?);

    let loaded = content::load_all(&paths)?;
    println!(
        "wok: scene {:?}: {} chunk(s), {} prefab(s)",
        loaded.scene.name,
        loaded.chunks.len(),
        loaded.prefabs.len()
    );
    println!("wok: controls: WASD move, Q/E down/up, hold right mouse to look, scroll for speed");
    println!("wok: editing: click selects, drag the selection moves it (Shift: vertical), Delete removes");
    println!("wok: Ctrl+S saves, Esc cancels/deselects");

    app::EditorApp::new(paths, loaded)
}
