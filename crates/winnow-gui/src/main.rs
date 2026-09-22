//! winnow — GTK4 image culling tool (Rust rewrite). Entry point + CLI.

mod agent;
mod app;
mod imageview;

use std::path::PathBuf;

use clap::Parser;
use gtk4::prelude::*;
use gtk4::{gio, glib, Application};
use winnow_core::Session;

use app::App;

const APP_ID: &str = "com.github.felixabrahamsson.winnow";

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
    /// Metadata file (.csv/.tsv). Auto-detected as metadata.csv / metadata.tsv
    /// in the folder if omitted.
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
    /// Validate FOLDER's metadata file against its images and exit
    /// (non-zero on errors).
    #[arg(long)]
    check: bool,
    /// Print a guide for AI agents: how to write metadata / buckets.
    #[arg(long)]
    agent_help: bool,
    /// Install the agent guide as a Claude Code skill (~/.claude/skills/winnow).
    #[arg(long)]
    install_skill: bool,
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

    if cli.agent_help {
        print!("{}", agent::guide());
        return glib::ExitCode::SUCCESS;
    }

    if cli.install_skill {
        return match agent::install_skill() {
            Ok(path) => {
                println!("Installed Claude Code skill: {}", path.display());
                println!("Agents will now pick it up when you ask them to prepare images for winnow.");
                glib::ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("winnow: {e}");
                glib::ExitCode::FAILURE
            }
        };
    }

    if cli.check {
        let folder = cli.folder.clone().unwrap_or_else(|| PathBuf::from("."));
        let report = winnow_core::check::check(
            &folder,
            !cli.no_recursive,
            cli.metadata.as_deref(),
            cli.buckets.as_deref(),
        );
        println!("{report}");
        return if report.has_errors() { glib::ExitCode::FAILURE } else { glib::ExitCode::SUCCESS };
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
    let meta = metadata.or_else(|| winnow_core::metadata::find_metadata(&root));
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
