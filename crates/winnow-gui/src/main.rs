//! winnow — GTK4 image culling tool (Rust rewrite). Entry point + CLI.

mod app;
mod imageview;

use std::path::PathBuf;

use clap::Parser;
use gtk4::prelude::*;
use gtk4::{gio, glib, Application};
use winnow_core::Session;

use app::App;

const APP_ID: &str = "com.github.felixabrahamsson.winnow";
const AUTO_METADATA: &[&str] = &["metadata.csv", "metadata.tsv", "metadata.json"];

#[derive(Parser, Clone)]
#[command(
    name = "winnow",
    version,
    about = "Fast keyboard-driven image culling / selection tool."
)]
struct Cli {
    /// Folder of images, or a single image (opens its folder, starting on it).
    folder: Option<PathBuf>,
    /// Do not descend into subfolders (default: recurse).
    #[arg(long)]
    no_recursive: bool,
    /// Metadata file (.csv/.tsv). Auto-detected as metadata.csv if omitted.
    #[arg(long)]
    metadata: Option<PathBuf>,
    /// Bucket config TOML (default: .winnow.toml in the folder).
    #[arg(long)]
    buckets: Option<PathBuf>,
    /// Initial sort key (name, path, mtime, size, meta:COLUMN).
    #[arg(long)]
    sort: Option<String>,
    /// Sort descending.
    #[arg(long)]
    sort_desc: bool,
    /// Register the "Open With -> Winnow" launcher and exit.
    #[arg(long)]
    install_desktop: bool,
}

fn main() -> glib::ExitCode {
    let cli = Cli::parse();

    if cli.install_desktop {
        match app::desktop::install_desktop() {
            Ok(path) => {
                println!("Installed launcher: {}", path.display());
                println!("Right-click a folder or image -> Open With -> Winnow.");
            }
            Err(e) => {
                eprintln!("winnow: {e}");
                return glib::ExitCode::FAILURE;
            }
        }
        return glib::ExitCode::SUCCESS;
    }

    let application = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application.connect_activate(move |gtkapp| activate(gtkapp, &cli));
    application.run_with_args::<&str>(&[])
}

/// Build a session for a folder or single image, auto-detecting metadata and
/// resolving the start index for a single-image target. Reused by the CLI and
/// the in-app "Open folder" action.
pub fn open_target(
    target: &std::path::Path,
    recursive: bool,
    metadata: Option<PathBuf>,
    buckets: Option<PathBuf>,
) -> Result<(Session, Option<usize>), winnow_core::buckets::BucketError> {
    // Opening a single image (e.g. as the default image viewer) scans only its
    // directory, never recursively — recursing a large tree like $HOME is slow
    // and unwanted when you just want to view one image and its siblings.
    let (root, start_file, recursive) = if target.is_file() {
        let parent = target.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| target.to_path_buf());
        (parent, Some(target.to_path_buf()), false)
    } else {
        (target.to_path_buf(), None, recursive)
    };
    let meta = metadata.or_else(|| AUTO_METADATA.iter().map(|n| root.join(n)).find(|p| p.exists()));
    let session = Session::new(&root, recursive, buckets.as_deref(), meta.as_deref())?;
    let start = start_file.and_then(|f| session.items.iter().position(|it| it.abs_path == f));
    Ok((session, start))
}

fn activate(gtkapp: &Application, cli: &Cli) {
    // No path given (bare `winnow`, or the launcher with no file) -> open empty
    // rather than scanning the current directory.
    let Some(folder) = cli.folder.clone() else {
        App::new(gtkapp, Session::empty(), None);
        return;
    };

    match open_target(&folder, !cli.no_recursive, cli.metadata.clone(), cli.buckets.clone()) {
        Ok((mut session, start)) => {
            if let Some(i) = start {
                session.set_index(i as isize);
            }
            let sort = cli.sort.clone().map(|k| (k, cli.sort_desc));
            App::new(gtkapp, session, sort);
        }
        Err(e) => {
            eprintln!("winnow: {e}");
            std::process::exit(1);
        }
    }
}
