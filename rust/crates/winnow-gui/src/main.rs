//! winnow GTK4 front-end (work in progress — the Rust rewrite).
//!
//! Prototype stage: proves the two riskiest pieces before building the rest —
//!   * native OS drag-out (drag the image into a file manager to copy it), and
//!   * a zoom/pan image viewer.
//! Plus arrow-key navigation so it's testable. Grid, buckets, brightness,
//! metadata panel, and desktop integration come next.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, DragSource, EventControllerKey, EventControllerScroll,
    EventControllerScrollFlags, Picture, ScrolledWindow,
};
use winnow_core::Session;

const APP_ID: &str = "com.github.felixabrahamsson.winnow";
const MIN_SCALE: f64 = 0.05;
const MAX_SCALE: f64 = 40.0;

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    // Handle our own positional folder arg rather than GApplication options.
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
    let (root, start_file) = if target.is_file() {
        (target.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| target.clone()), Some(target.clone()))
    } else {
        (target, None)
    };

    let session = match Session::new(&root, true, None, None) {
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

    let session = Rc::new(RefCell::new(session));
    let scale = Rc::new(Cell::new(1.0f64));
    let cur_path = Rc::new(RefCell::new(PathBuf::new()));

    let picture = Picture::new();
    picture.set_keep_aspect_ratio(true);
    picture.set_can_shrink(true);
    picture.set_halign(gtk4::Align::Center);
    picture.set_valign(gtk4::Align::Center);

    let scroller = ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&picture)
        .build();

    let window = ApplicationWindow::builder()
        .application(app)
        .title("winnow")
        .default_width(1200)
        .default_height(800)
        .child(&scroller)
        .build();

    // ---- refresh helpers ------------------------------------------
    let refresh = {
        let session = session.clone();
        let scale = scale.clone();
        let cur_path = cur_path.clone();
        let picture = picture.clone();
        let window = window.clone();
        Rc::new(move || {
            let s = session.borrow();
            if let Some(item) = s.current() {
                *cur_path.borrow_mut() = item.abs_path.clone();
                match gdk::Texture::from_filename(&item.abs_path) {
                    Ok(tex) => {
                        picture.set_paintable(Some(&tex));
                        let sc = scale.get();
                        picture.set_size_request(
                            (tex.width() as f64 * sc) as i32,
                            (tex.height() as f64 * sc) as i32,
                        );
                    }
                    Err(_) => picture.set_paintable(None::<&gdk::Texture>),
                }
                window.set_title(Some(&format!("{}/{} — {}", s.index + 1, s.count(), item.name())));
            } else {
                picture.set_paintable(None::<&gdk::Texture>);
                window.set_title(Some("winnow — empty"));
            }
        })
    };

    let apply_scale = {
        let picture = picture.clone();
        let scale = scale.clone();
        Rc::new(move || {
            if let Some(p) = picture.paintable() {
                let sc = scale.get();
                picture.set_size_request(
                    (p.intrinsic_width() as f64 * sc) as i32,
                    (p.intrinsic_height() as f64 * sc) as i32,
                );
            }
        })
    };

    // ---- keyboard navigation & zoom -------------------------------
    let keys = EventControllerKey::new();
    {
        let session = session.clone();
        let scale = scale.clone();
        let refresh = refresh.clone();
        let apply_scale = apply_scale.clone();
        keys.connect_key_pressed(move |_c, keyval, _code, _mods| {
            match keyval {
                gdk::Key::Right | gdk::Key::space => {
                    session.borrow_mut().next();
                    refresh();
                }
                gdk::Key::Left => {
                    session.borrow_mut().prev();
                    refresh();
                }
                gdk::Key::plus | gdk::Key::equal => {
                    scale.set((scale.get() * 1.25).min(MAX_SCALE));
                    apply_scale();
                }
                gdk::Key::minus => {
                    scale.set((scale.get() / 1.25).max(MIN_SCALE));
                    apply_scale();
                }
                gdk::Key::_1 => {
                    scale.set(1.0);
                    apply_scale();
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(keys);

    // ---- scroll-to-zoom -------------------------------------------
    let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
    {
        let scale = scale.clone();
        let apply_scale = apply_scale.clone();
        scroll.connect_scroll(move |_c, _dx, dy| {
            let factor = if dy < 0.0 { 1.15 } else { 1.0 / 1.15 };
            scale.set((scale.get() * factor).clamp(MIN_SCALE, MAX_SCALE));
            apply_scale();
            glib::Propagation::Stop
        });
    }
    picture.add_controller(scroll);

    // ---- native drag-out (the critical capability) ----------------
    let drag = DragSource::new();
    drag.set_actions(gdk::DragAction::COPY);
    {
        let cur_path = cur_path.clone();
        drag.connect_prepare(move |_src, _x, _y| {
            let path = cur_path.borrow().clone();
            if path.as_os_str().is_empty() {
                return None;
            }
            let file = gio::File::for_path(&path);
            // Offer both a GFile value and a text/uri-list payload so GNOME
            // (Nautilus) and KDE (Dolphin) both accept the drop as a copy.
            let uri = format!("{}\r\n", file.uri());
            let uri_provider =
                gdk::ContentProvider::for_bytes("text/uri-list", &glib::Bytes::from_owned(uri.into_bytes()));
            let file_provider = gdk::ContentProvider::for_value(&file.to_value());
            Some(gdk::ContentProvider::new_union(&[file_provider, uri_provider]))
        });
    }
    {
        // Use the current image as the drag cursor icon.
        let picture = picture.clone();
        drag.connect_drag_begin(move |src, _drag| {
            if let Some(p) = picture.paintable() {
                src.set_icon(Some(&p), 0, 0);
            }
        });
    }
    picture.add_controller(drag);

    refresh();
    window.present();
}
