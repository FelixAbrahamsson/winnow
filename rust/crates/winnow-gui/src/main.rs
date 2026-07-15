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
#[command(name = "winnow", about = "Fast keyboard-driven image culling / selection tool.")]
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

fn activate(gtkapp: &Application, cli: &Cli) {
    let target = cli
        .folder
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let (root, start_file) = if target.is_file() {
        (target.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| target.clone()), Some(target.clone()))
    } else {
        (target, None)
    };

    let meta = cli
        .metadata
        .clone()
        .or_else(|| AUTO_METADATA.iter().map(|n| root.join(n)).find(|p| p.exists()));
    if cli.metadata.is_none() {
        if let Some(m) = &meta {
            eprintln!("using metadata: {}", m.file_name().unwrap_or_default().to_string_lossy());
        }
    }

    let session = match Session::new(&root, !cli.no_recursive, cli.buckets.as_deref(), meta.as_deref()) {
        Ok(mut s) => {
            if let Some(f) = &start_file {
                if let Some(i) = s.items.iter().position(|it| &it.abs_path == f) {
                    s.set_index(i as isize);
                }
            }
            s
        }
        Err(e) => {
            eprintln!("winnow: {e}");
            std::process::exit(1);
        }
    };

    let sort = cli.sort.clone().map(|k| (k, cli.sort_desc));
    App::new(gtkapp, session, sort);
}
