//! wok: the editor, the engine's reference application.
//!
//! Reset to a minimal shell: a window, an egui pass, and an empty viewport with a working menu bar.
//! The File menu opens a project - a content-root folder - and the editor shows the project's name
//! in the title bar and status bar. Loading and rendering the project's scene content, and the
//! authoring surfaces over it, return as later pieces; this is the frame they drop into.
//!
//! egui is a dependency of this application only, never of an engine crate. Run with
//! `cargo run -p wok [project-dir]`: with a folder argument the editor opens it on startup,
//! otherwise it starts with no project open (open one from the File menu).

mod action;
mod app;
mod cli;
mod gui;
mod menu;
mod model;
mod project;
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
