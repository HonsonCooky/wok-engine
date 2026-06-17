//! wok: the editor, the engine's reference application.
//!
//! The Scene view frame: a window, the Zed-style chrome, and a god-cam viewport that renders the
//! open project's scene. The File menu opens a project - a content-root folder - which loads (or
//! first generates) its scene content and draws it through the engine's render path; the content
//! browser lists the scene, prefabs, and lighting, and opening the scene gives a Scene tab. Backtick
//! toggles a free-fly god-cam (Object is the resting mode). The authoring surfaces over the
//! viewport - picking, the inspector, place mode, save - return as later pieces; this is the frame
//! they drop into.
//!
//! egui is a dependency of this application only, never of an engine crate; the engine libraries
//! (wok-scene, wok-content, wok-mesh, wok-render, wok-light) are composed here, never the reverse.
//! Run with `cargo run -p wok [project-dir]`: with a folder argument the editor opens it on startup,
//! otherwise it starts with no project open (open one from the File menu).

mod action;
mod app;
mod camera;
mod cli;
mod content;
mod gui;
mod input;
mod menu;
mod mode;
mod model;
mod place;
mod project;
mod recent;
mod render;
mod sample;
mod scene;
mod theme;
mod view;
mod workspace;

use wok_platform::Desc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let initial = match cli::parse_args(&args) {
        Ok(initial) => initial,
        Err(err) => {
            eprintln!("wok: {err}");
            std::process::exit(1);
        }
    };
    let app = app::EditorApp::new(initial);
    // run() owns the OS event loop and returns when the window closes.
    wok_platform::run(app, Desc { title: "wok", width: 0, height: 0, vsync: true });
}
