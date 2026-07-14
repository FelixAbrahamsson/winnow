//! winnow GTK4 front-end (work in progress — the Rust rewrite).
//!
//! This is currently a skeleton: it opens a folder via winnow-core and shows a
//! window. Zoom/pan viewer, drag-out, thumbnail grid, and buckets land next.

use std::path::PathBuf;

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Label};
use winnow_core::Session;

const APP_ID: &str = "com.github.felixabrahamsson.winnow";

fn main() -> gtk4::glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    // Don't let GTK parse our positional folder arg as a GApplication option.
    app.run_with_args::<&str>(&[])
}

fn folder_arg() -> PathBuf {
    std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn build_ui(app: &Application) {
    let target = folder_arg();
    let (root, start_rel) = if target.is_file() {
        (target.parent().map(|p| p.to_path_buf()).unwrap_or(target.clone()), Some(target.clone()))
    } else {
        (target, None)
    };

    let session = Session::new(&root, true, None, None);

    let text = match &session {
        Ok(s) => format!(
            "winnow (Rust) — {}\n{} images\nbuckets: {}",
            s.root.display(),
            s.count(),
            s.buckets.iter().map(|b| b.name.as_str()).collect::<Vec<_>>().join(", "),
        ),
        Err(e) => format!("error: {e}"),
    };
    let _ = start_rel;

    let label = Label::builder().label(&text).margin_top(24).margin_bottom(24).build();

    let window = ApplicationWindow::builder()
        .application(app)
        .title("winnow")
        .default_width(1200)
        .default_height(800)
        .child(&label)
        .build();
    window.present();
}
