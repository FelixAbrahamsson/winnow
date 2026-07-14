//! The winnow application: an `App` struct owning the session, widgets, and
//! view state, with methods wired to GTK event controllers.

pub mod desktop;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk4::gdk;
use gtk4::gdk_pixbuf::Pixbuf;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, DragSource, EventControllerKey, EventControllerScroll,
    EventControllerScrollFlags, GestureClick, GestureDrag, Label, Orientation, Picture,
    PropagationPhase, ScrolledWindow,
};
use winnow_core::Session;

const MIN_SCALE: f64 = 0.05; // absolute floor, used before the view is sized
const MAX_SCALE: f64 = 40.0;
const ZOOM_RATE: f64 = 1.1; // per unit of scroll delta (proportional)
const KEY_ZOOM_STEP: f64 = 1.25;
const BRIGHT_STEP: f64 = 0.1;
const GAMMA_STEP: f64 = 0.1;

/// Apply brightness/gamma to a pixbuf via a per-channel LUT. Identity fast-path.
fn adjust_pixbuf(orig: &Pixbuf, brightness: f64, gamma: f64) -> Pixbuf {
    if (brightness - 1.0).abs() < 1e-3 && (gamma - 1.0).abs() < 1e-3 {
        return orig.clone();
    }
    let mut lut = [0u8; 256];
    for (i, l) in lut.iter_mut().enumerate() {
        let mut v = i as f64 / 255.0;
        if (gamma - 1.0).abs() >= 1e-3 {
            v = v.powf(1.0 / gamma.max(0.05));
        }
        v = (v * brightness).clamp(0.0, 1.0);
        *l = (v * 255.0).round() as u8;
    }
    let n = orig.n_channels() as usize;
    let rowstride = orig.rowstride() as usize;
    let width = orig.width() as usize;
    let height = orig.height() as usize;
    let mut data = orig.read_pixel_bytes().to_vec();
    for y in 0..height {
        let row = &mut data[y * rowstride..y * rowstride + width * n];
        for px in row.chunks_mut(n) {
            px[0] = lut[px[0] as usize];
            px[1] = lut[px[1] as usize];
            px[2] = lut[px[2] as usize];
        }
    }
    Pixbuf::from_bytes(
        &glib::Bytes::from_owned(data),
        orig.colorspace(),
        orig.has_alpha(),
        orig.bits_per_sample(),
        orig.width(),
        orig.height(),
        orig.rowstride(),
    )
}

pub struct App {
    session: RefCell<Session>,
    window: ApplicationWindow,
    picture: Picture,
    scroller: ScrolledWindow,
    status: Label,
    orig_pixbuf: RefCell<Option<Pixbuf>>,
    cur_path: RefCell<PathBuf>,
    scale: Cell<f64>,
    fitted: Cell<bool>,
    brightness: Cell<f64>,
    gamma: Cell<f64>,
    msg_gen: Cell<u64>,
    pan_start: Cell<(f64, f64)>,
}

impl App {
    pub fn new(gtkapp: &Application, session: Session, sort: Option<(String, bool)>) -> Rc<App> {
        let picture = Picture::new();
        picture.set_keep_aspect_ratio(true);
        picture.set_can_shrink(true);
        picture.set_halign(gtk4::Align::Center);
        picture.set_valign(gtk4::Align::Center);

        let scroller =
            ScrolledWindow::builder().hexpand(true).vexpand(true).child(&picture).build();
        scroller.set_kinetic_scrolling(false);

        let status = Label::builder()
            .xalign(0.0)
            .margin_start(8)
            .margin_end(8)
            .margin_top(3)
            .margin_bottom(3)
            .build();

        let vbox = gtk4::Box::new(Orientation::Vertical, 0);
        vbox.append(&scroller);
        vbox.append(&status);

        let window = ApplicationWindow::builder()
            .application(gtkapp)
            .title("winnow")
            .default_width(1200)
            .default_height(800)
            .child(&vbox)
            .build();

        let app = Rc::new(App {
            session: RefCell::new(session),
            window,
            picture,
            scroller,
            status,
            orig_pixbuf: RefCell::new(None),
            cur_path: RefCell::new(PathBuf::new()),
            scale: Cell::new(1.0),
            fitted: Cell::new(true),
            brightness: Cell::new(1.0),
            gamma: Cell::new(1.0),
            msg_gen: Cell::new(0),
            pan_start: Cell::new((0.0, 0.0)),
        });

        app.build_controllers();
        app.refresh();
        if let Some((key, desc)) = sort {
            app.session.borrow_mut().apply_sort(&key, desc);
            app.refresh();
        }
        app.window.present();

        // Fit once the viewport has a real size.
        {
            let app = app.clone();
            glib::timeout_add_local_once(Duration::from_millis(30), move || app.fit());
        }
        app
    }

    // ---- image loading & rendering --------------------------------
    fn refresh(&self) {
        let path = self.session.borrow().current().map(|i| i.abs_path.clone());
        match path {
            Some(p) => {
                *self.cur_path.borrow_mut() = p.clone();
                match Pixbuf::from_file(&p) {
                    Ok(pb) => {
                        let pb = pb.apply_embedded_orientation().unwrap_or(pb);
                        *self.orig_pixbuf.borrow_mut() = Some(pb);
                        self.render();
                        if self.fitted.get() {
                            if let Some(f) = self.viewport_fit() {
                                self.scale.set(f.min(1.0));
                            }
                        }
                        self.apply_scale();
                    }
                    Err(_) => {
                        *self.orig_pixbuf.borrow_mut() = None;
                        self.picture.set_paintable(None::<&gdk::Texture>);
                    }
                }
            }
            None => {
                *self.orig_pixbuf.borrow_mut() = None;
                self.picture.set_paintable(None::<&gdk::Texture>);
            }
        }
        self.update_status();
    }

    /// Re-apply brightness/gamma to the current image (keeps zoom).
    fn render(&self) {
        if let Some(orig) = self.orig_pixbuf.borrow().as_ref() {
            let adj = adjust_pixbuf(orig, self.brightness.get(), self.gamma.get());
            let tex = gdk::Texture::for_pixbuf(&adj);
            self.picture.set_paintable(Some(&tex));
        }
    }

    // ---- status bar -----------------------------------------------
    fn update_status(&self) {
        let s = self.session.borrow();
        let text = match s.current() {
            Some(item) => format!("{}/{} — {}", s.index + 1, s.count(), item.rel_path),
            None => "0/0 — empty".to_string(),
        };
        self.status.set_text(&text);
        self.window.set_title(Some(&text));
    }

    fn flash(self: &Rc<Self>, text: String) {
        let g = self.msg_gen.get().wrapping_add(1);
        self.msg_gen.set(g);
        self.status.set_text(&text);
        let app = self.clone();
        glib::timeout_add_local_once(Duration::from_secs(4), move || {
            if app.msg_gen.get() == g {
                app.update_status();
            }
        });
    }

    // ---- zoom / pan -----------------------------------------------
    fn viewport_fit(&self) -> Option<f64> {
        let p = self.picture.paintable()?;
        let (iw, ih) = (p.intrinsic_width() as f64, p.intrinsic_height() as f64);
        let (vw, vh) = (self.scroller.width() as f64, self.scroller.height() as f64);
        if iw > 0.0 && ih > 0.0 && vw > 0.0 && vh > 0.0 {
            Some((vw / iw).min(vh / ih))
        } else {
            None
        }
    }

    fn min_scale(&self) -> f64 {
        self.viewport_fit().map(|f| f.min(1.0)).unwrap_or(MIN_SCALE)
    }

    fn apply_scale(&self) {
        if let Some(p) = self.picture.paintable() {
            let sc = self.scale.get();
            self.picture.set_size_request(
                (p.intrinsic_width() as f64 * sc).round() as i32,
                (p.intrinsic_height() as f64 * sc).round() as i32,
            );
        }
    }

    fn zoom(&self, factor: f64) {
        let s = (self.scale.get() * factor).clamp(self.min_scale(), MAX_SCALE);
        self.scale.set(s);
        self.fitted.set(false);
        self.apply_scale();
    }

    fn fit(&self) {
        if let Some(f) = self.viewport_fit() {
            self.scale.set(f.min(1.0));
            self.apply_scale();
        }
        self.fitted.set(true);
    }

    fn actual_size(&self) {
        self.scale.set(1.0f64.clamp(self.min_scale(), MAX_SCALE));
        self.fitted.set(false);
        self.apply_scale();
    }

    fn can_pan(&self) -> bool {
        let h = self.scroller.hadjustment();
        let v = self.scroller.vadjustment();
        h.upper() > h.page_size() + 0.5 || v.upper() > v.page_size() + 0.5
    }

    // ---- brightness -----------------------------------------------
    fn bump_brightness(self: &Rc<Self>, delta: f64) {
        self.brightness.set((self.brightness.get() + delta).clamp(0.1, 5.0));
        self.render();
        self.apply_scale();
        self.flash(format!("Brightness {:.0}%", self.brightness.get() * 100.0));
    }

    fn bump_gamma(self: &Rc<Self>, delta: f64) {
        self.gamma.set((self.gamma.get() + delta).clamp(0.1, 5.0));
        self.render();
        self.apply_scale();
        self.flash(format!("Gamma {:.2}", self.gamma.get()));
    }

    fn reset_adjustments(self: &Rc<Self>) {
        self.brightness.set(1.0);
        self.gamma.set(1.0);
        self.render();
        self.apply_scale();
        self.flash("Reset brightness & gamma".into());
    }

    // ---- buckets / undo -------------------------------------------
    fn move_to_bucket(self: &Rc<Self>, idx: usize) {
        let msg = self.session.borrow_mut().move_current_to(idx);
        self.refresh();
        if let Some(m) = msg {
            self.flash(m);
        }
    }

    fn undo(self: &Rc<Self>) {
        let msg = self.session.borrow_mut().undo();
        self.refresh();
        if let Some(m) = msg {
            self.flash(m);
        }
    }

    fn redo(self: &Rc<Self>) {
        let msg = self.session.borrow_mut().redo();
        self.refresh();
        if let Some(m) = msg {
            self.flash(m);
        }
    }

    // ---- clipboard ------------------------------------------------
    fn copy_name(self: &Rc<Self>) {
        if let Some(name) = self.session.borrow().current().map(|i| i.name()) {
            self.window.clipboard().set_text(&name);
            self.flash(format!("Copied filename: {name}"));
        }
    }

    fn copy_path(self: &Rc<Self>) {
        let p = self.cur_path.borrow().clone();
        if !p.as_os_str().is_empty() {
            self.window.clipboard().set_text(&p.to_string_lossy());
            self.flash(format!("Copied path: {}", p.display()));
        }
    }

    fn copy_file(self: &Rc<Self>) {
        let p = self.cur_path.borrow().clone();
        if p.as_os_str().is_empty() {
            return;
        }
        let file = gio::File::for_path(&p);
        let uri = file.uri();
        let uri_list = format!("{}\r\n", uri);
        let gnome = format!("copy\n{}", uri);
        let providers = [
            gdk::ContentProvider::for_value(&file.to_value()),
            gdk::ContentProvider::for_bytes(
                "text/uri-list",
                &glib::Bytes::from_owned(uri_list.into_bytes()),
            ),
            gdk::ContentProvider::for_bytes(
                "x-special/gnome-copied-files",
                &glib::Bytes::from_owned(gnome.into_bytes()),
            ),
            gdk::ContentProvider::for_bytes(
                "application/x-kde-cutselection",
                &glib::Bytes::from_owned(b"0".to_vec()),
            ),
        ];
        let provider = gdk::ContentProvider::new_union(&providers);
        let _ = self.window.clipboard().set_content(Some(&provider));
        let name =
            p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.flash(format!("Copied file (paste into a file manager): {name}"));
    }

    fn toggle_fullscreen(&self) {
        if self.window.is_fullscreen() {
            self.window.unfullscreen();
        } else {
            self.window.fullscreen();
        }
    }

    // ---- event controllers ----------------------------------------
    fn build_controllers(self: &Rc<Self>) {
        self.build_keys();
        self.build_scroll();
        self.build_mouse();
        self.build_drag_source();
    }

    fn build_keys(self: &Rc<Self>) {
        let keys = EventControllerKey::new();
        let app = self.clone();
        keys.connect_key_pressed(move |_c, keyval, _code, state| {
            use glib::Propagation::{Proceed, Stop};
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);

            if ctrl {
                match keyval {
                    gdk::Key::z if shift => app.redo(),
                    gdk::Key::z => app.undo(),
                    gdk::Key::y => app.redo(),
                    gdk::Key::c if shift => app.copy_path(),
                    gdk::Key::c => app.copy_name(),
                    gdk::Key::x if shift => app.copy_file(),
                    _ => return Proceed,
                }
                return Stop;
            }

            // Bucket hotkeys by GDK key name; reject also answers Backspace / x.
            if let Some(kn) = keyval.name() {
                let kn = kn.to_string();
                let bidx = app
                    .session
                    .borrow()
                    .buckets
                    .iter()
                    .position(|b| b.key.eq_ignore_ascii_case(&kn))
                    .or_else(|| {
                        if kn.eq_ignore_ascii_case("BackSpace") || kn.eq_ignore_ascii_case("x") {
                            Some(0)
                        } else {
                            None
                        }
                    });
                if let Some(i) = bidx {
                    app.move_to_bucket(i);
                    return Stop;
                }
            }

            match keyval {
                gdk::Key::Right | gdk::Key::space => {
                    app.session.borrow_mut().next();
                    app.refresh();
                }
                gdk::Key::Left => {
                    app.session.borrow_mut().prev();
                    app.refresh();
                }
                gdk::Key::Page_Down => {
                    app.session.borrow_mut().jump(10);
                    app.refresh();
                }
                gdk::Key::Page_Up => {
                    app.session.borrow_mut().jump(-10);
                    app.refresh();
                }
                gdk::Key::Home => {
                    app.session.borrow_mut().set_index(0);
                    app.refresh();
                }
                gdk::Key::End => {
                    let last = app.session.borrow().count() as isize - 1;
                    app.session.borrow_mut().set_index(last);
                    app.refresh();
                }
                gdk::Key::plus | gdk::Key::equal => app.zoom(KEY_ZOOM_STEP),
                gdk::Key::minus => app.zoom(1.0 / KEY_ZOOM_STEP),
                gdk::Key::f => app.fit(),
                gdk::Key::a => app.actual_size(),
                gdk::Key::bracketright => app.bump_brightness(BRIGHT_STEP),
                gdk::Key::bracketleft => app.bump_brightness(-BRIGHT_STEP),
                gdk::Key::braceright => app.bump_gamma(GAMMA_STEP),
                gdk::Key::braceleft => app.bump_gamma(-GAMMA_STEP),
                gdk::Key::backslash => app.reset_adjustments(),
                gdk::Key::F11 => app.toggle_fullscreen(),
                _ => return Proceed,
            }
            Stop
        });
        self.window.add_controller(keys);
    }

    fn build_scroll(self: &Rc<Self>) {
        let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(PropagationPhase::Capture);
        let app = self.clone();
        scroll.connect_scroll(move |_c, _dx, dy| {
            app.zoom(ZOOM_RATE.powf(-dy));
            glib::Propagation::Stop
        });
        self.scroller.add_controller(scroll);
    }

    fn build_mouse(self: &Rc<Self>) {
        // Middle-drag pans the zoomed image.
        let pan = GestureDrag::new();
        pan.set_button(gdk::BUTTON_MIDDLE);
        {
            let app = self.clone();
            pan.connect_drag_begin(move |_g, _x, _y| {
                app.pan_start.set((app.scroller.hadjustment().value(), app.scroller.vadjustment().value()));
            });
        }
        {
            let app = self.clone();
            pan.connect_drag_update(move |_g, ox, oy| {
                let (sh, sv) = app.pan_start.get();
                app.scroller.hadjustment().set_value(sh - ox);
                app.scroller.vadjustment().set_value(sv - oy);
            });
        }
        self.picture.add_controller(pan);

        // Double-click toggles fullscreen; mouse Back/Forward navigate.
        let click = GestureClick::new();
        click.set_button(0); // any button
        let app = self.clone();
        click.connect_pressed(move |g, n_press, _x, _y| {
            match g.current_button() {
                1 if n_press == 2 => app.toggle_fullscreen(),
                8 => {
                    app.session.borrow_mut().prev();
                    app.refresh();
                }
                9 => {
                    app.session.borrow_mut().next();
                    app.refresh();
                }
                _ => {}
            }
        });
        self.picture.add_controller(click);
    }

    fn build_drag_source(self: &Rc<Self>) {
        let drag = DragSource::new();
        drag.set_actions(gdk::DragAction::COPY);
        {
            let app = self.clone();
            drag.connect_prepare(move |_src, _x, _y| {
                let path = app.cur_path.borrow().clone();
                if path.as_os_str().is_empty() {
                    return None;
                }
                let file = gio::File::for_path(&path);
                let uri = format!("{}\r\n", file.uri());
                let uri_provider = gdk::ContentProvider::for_bytes(
                    "text/uri-list",
                    &glib::Bytes::from_owned(uri.into_bytes()),
                );
                let file_provider = gdk::ContentProvider::for_value(&file.to_value());
                Some(gdk::ContentProvider::new_union(&[file_provider, uri_provider]))
            });
        }
        {
            let app = self.clone();
            drag.connect_drag_begin(move |src, _drag| {
                let path = app.cur_path.borrow().clone();
                if let Ok(pb) = Pixbuf::from_file_at_scale(&path, 160, 160, true) {
                    let tex = gdk::Texture::for_pixbuf(&pb);
                    src.set_icon(Some(&tex), tex.width() / 2, tex.height() / 2);
                }
            });
        }
        self.picture.add_controller(drag);
    }
}
