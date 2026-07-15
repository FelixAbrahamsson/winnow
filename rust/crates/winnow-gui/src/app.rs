//! The winnow application: an `App` struct owning the session, widgets, and
//! view state, with methods wired to GTK event controllers.

pub mod desktop;
mod grid;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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
    info_panel: gtk4::Box,
    info_rows: gtk4::Box,
    sort_keys: Vec<(String, String)>,
    sort_dropdown: gtk4::DropDown,
    desc_check: gtk4::CheckButton,
    stack: gtk4::Stack,
    grid_view: gtk4::GridView,
    grid_model: gtk4::StringList,
    grid_selection: gtk4::MultiSelection,
    grid_factory: gtk4::SignalListItemFactory,
    thumb_cache: RefCell<HashMap<PathBuf, gdk::Texture>>,
    in_grid: Cell<bool>,
    orig_pixbuf: RefCell<Option<Pixbuf>>,
    cur_path: RefCell<PathBuf>,
    scale: Cell<f64>,
    fitted: Cell<bool>,
    brightness: Cell<f64>,
    gamma: Cell<f64>,
    msg_gen: Cell<u64>,
    pan_start: Cell<(f64, f64)>,
    pan_active: Cell<bool>,
    pointer: Cell<(f64, f64)>,            // last pointer pos over the viewport
    pending_scroll: Cell<Option<(f64, f64)>>, // zoom-to-cursor target, applied on relayout
}

fn event_has_ctrl(ev: Option<gdk::Event>) -> bool {
    ev.map(|e| e.modifier_state().contains(gdk::ModifierType::CONTROL_MASK)).unwrap_or(false)
}

fn human_size(n: u64) -> String {
    let mut f = n as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if f < 1024.0 || unit == "GB" {
            return if unit == "B" { format!("{n} B") } else { format!("{f:.1} {unit}") };
        }
        f /= 1024.0;
    }
    format!("{n} B")
}

fn format_mtime(secs: f64) -> Option<String> {
    if secs <= 0.0 {
        return None;
    }
    glib::DateTime::from_unix_local(secs as i64)
        .ok()
        .and_then(|dt| dt.format("%Y-%m-%d %H:%M").ok())
        .map(|g| g.to_string())
}

fn is_url(t: &str) -> bool {
    ["http://", "https://", "ftp://", "file://", "www."].iter().any(|p| t.starts_with(p))
}

/// Wrap URLs in Pango `<a href>` markup, escaping the rest. Returns (markup, found).
fn linkify_markup(text: &str) -> (String, bool) {
    let mut found = false;
    let parts: Vec<String> = text
        .split(' ')
        .map(|tok| {
            let trimmed = tok.trim_end_matches(|c: char| ".,;:!?)]}>\"'".contains(c));
            let trail = &tok[trimmed.len()..];
            if !trimmed.is_empty() && is_url(trimmed) {
                found = true;
                let href = if trimmed.contains("://") {
                    trimmed.to_string()
                } else {
                    format!("https://{trimmed}")
                };
                format!(
                    "<a href=\"{}\">{}</a>{}",
                    glib::markup_escape_text(&href),
                    glib::markup_escape_text(trimmed),
                    glib::markup_escape_text(trail)
                )
            } else {
                glib::markup_escape_text(tok).to_string()
            }
        })
        .collect();
    (parts.join(" "), found)
}

#[derive(Clone, Copy)]
enum MenuAction {
    Next,
    Prev,
    Bucket(usize),
    Undo,
    Fit,
    Actual,
    Fullscreen,
    CopyName,
    CopyPath,
    CopyFile,
    Help,
}

impl App {
    pub fn new(gtkapp: &Application, session: Session, sort: Option<(String, bool)>) -> Rc<App> {
        let picture = Picture::new();
        picture.set_keep_aspect_ratio(true);
        picture.set_can_shrink(true);
        // halign/valign are set per-mode in apply_scale (Fill to fit, Center to zoom).

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

        // ---- info / sort side panel ----
        let sort_keys: Vec<(String, String)> = session.sortable_keys();
        let labels: Vec<&str> = sort_keys.iter().map(|(_, l)| l.as_str()).collect();
        let sort_dropdown = gtk4::DropDown::from_strings(&labels);
        let desc_check = gtk4::CheckButton::with_label("desc");
        let sort_row = gtk4::Box::new(Orientation::Horizontal, 6);
        sort_row.set_margin_top(8);
        sort_row.set_margin_bottom(4);
        sort_row.set_margin_start(8);
        sort_row.set_margin_end(8);
        sort_row.append(&Label::new(Some("Sort:")));
        sort_dropdown.set_hexpand(true);
        sort_row.append(&sort_dropdown);
        sort_row.append(&desc_check);

        let info_rows = gtk4::Box::new(Orientation::Vertical, 2);
        info_rows.set_margin_start(8);
        info_rows.set_margin_end(8);
        let info_scroll =
            ScrolledWindow::builder().hexpand(true).vexpand(true).child(&info_rows).build();

        let info_panel = gtk4::Box::new(Orientation::Vertical, 0);
        info_panel.set_width_request(200);
        info_panel.append(&sort_row);
        info_panel.append(&gtk4::Separator::new(Orientation::Horizontal));
        info_panel.append(&info_scroll);

        // Resizable split: drag the divider to size the details panel.
        let paned = gtk4::Paned::new(Orientation::Horizontal);
        paned.set_start_child(Some(&vbox));
        paned.set_end_child(Some(&info_panel));
        paned.set_resize_start_child(true);
        paned.set_resize_end_child(false);
        paned.set_shrink_start_child(false);
        paned.set_shrink_end_child(true);
        paned.set_position(960);
        paned.set_wide_handle(true);
        let hbox = paned;

        // ---- thumbnail grid page ----
        let grid_model = gtk4::StringList::new(&[]);
        let grid_selection = gtk4::MultiSelection::new(Some(grid_model.clone()));
        let grid_factory = gtk4::SignalListItemFactory::new();
        let grid_view =
            gtk4::GridView::new(Some(grid_selection.clone()), Some(grid_factory.clone()));
        grid_view.set_min_columns(2);
        grid_view.set_max_columns(16);
        grid_view.set_enable_rubberband(true);
        let grid_scroll =
            ScrolledWindow::builder().hexpand(true).vexpand(true).child(&grid_view).build();

        let stack = gtk4::Stack::new();
        stack.add_named(&hbox, Some("single"));
        stack.add_named(&grid_scroll, Some("grid"));

        let window = ApplicationWindow::builder()
            .application(gtkapp)
            .title("winnow")
            .default_width(1280)
            .default_height(820)
            .child(&stack)
            .build();

        let app = Rc::new(App {
            session: RefCell::new(session),
            window,
            picture,
            scroller,
            status,
            info_panel,
            info_rows,
            sort_keys,
            sort_dropdown,
            desc_check,
            stack,
            grid_view,
            grid_model,
            grid_selection,
            grid_factory,
            thumb_cache: RefCell::new(HashMap::new()),
            in_grid: Cell::new(false),
            orig_pixbuf: RefCell::new(None),
            cur_path: RefCell::new(PathBuf::new()),
            scale: Cell::new(1.0),
            fitted: Cell::new(true),
            brightness: Cell::new(1.0),
            gamma: Cell::new(1.0),
            msg_gen: Cell::new(0),
            pan_start: Cell::new((0.0, 0.0)),
            pan_active: Cell::new(false),
            pointer: Cell::new((0.0, 0.0)),
            pending_scroll: Cell::new(None),
        });

        app.build_controllers();
        app.refresh();
        if let Some((key, desc)) = sort {
            // Setting the widgets fires the handlers, which apply the sort.
            app.desc_check.set_active(desc);
            if let Some(pos) = app.sort_keys.iter().position(|(k, _)| k == &key) {
                app.sort_dropdown.set_selected(pos as u32);
            }
            app.apply_sort_from_ui();
        }
        app.window.present();
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
        self.update_info();
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
    // Fit mode leaves the Picture with no explicit size, so GtkPicture fills
    // the viewport and auto-scales (upscaling small images, following resizes).
    // Zooming sets an explicit size the ScrolledWindow can pan around.
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

    fn effective_scale(&self) -> f64 {
        if self.fitted.get() {
            self.viewport_fit().unwrap_or(1.0)
        } else {
            self.scale.get()
        }
    }

    fn apply_scale(&self) {
        if self.fitted.get() {
            self.picture.set_halign(gtk4::Align::Fill);
            self.picture.set_valign(gtk4::Align::Fill);
            self.picture.set_size_request(-1, -1);
        } else {
            self.picture.set_halign(gtk4::Align::Center);
            self.picture.set_valign(gtk4::Align::Center);
            if let Some(p) = self.picture.paintable() {
                let s = self.scale.get();
                self.picture.set_size_request(
                    (p.intrinsic_width() as f64 * s).round() as i32,
                    (p.intrinsic_height() as f64 * s).round() as i32,
                );
            }
        }
        self.update_cursor();
    }

    fn update_cursor(&self) {
        let name = if self.pan_active.get() {
            "grabbing"
        } else if self.can_pan() {
            "grab"
        } else {
            "default"
        };
        self.picture.set_cursor_from_name(Some(name));
    }

    fn fit(&self) {
        self.fitted.set(true);
        self.apply_scale();
    }

    fn actual_size(&self) {
        self.scale.set(1.0);
        self.fitted.set(false);
        self.apply_scale();
    }

    /// Zoom by `factor`, keeping the point at viewport coords (fx, fy) fixed.
    fn zoom_at(&self, factor: f64, fx: f64, fy: f64) {
        let fit = self.viewport_fit().unwrap_or(0.0);
        let old = self.effective_scale();
        let lo = if fit > 0.0 { fit } else { MIN_SCALE };
        let new = (old * factor).clamp(lo, MAX_SCALE);
        if new <= fit + 1e-6 {
            self.fit(); // zoomed back out to fit -> fill mode
            return;
        }
        if (new - old).abs() < 1e-9 {
            return;
        }
        let hadj = self.scroller.hadjustment();
        let vadj = self.scroller.vadjustment();
        let ix = (hadj.value() + fx) / old;
        let iy = (vadj.value() + fy) / old;
        self.pending_scroll.set(Some((ix * new - fx, iy * new - fy)));
        self.scale.set(new);
        self.fitted.set(false);
        self.apply_scale();
    }

    fn zoom(&self, factor: f64) {
        let (w, h) = (self.scroller.width() as f64, self.scroller.height() as f64);
        self.zoom_at(factor, w / 2.0, h / 2.0);
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

    // Absolute setters used by the header-bar sliders (no status flash).
    fn set_brightness(&self, v: f64) {
        self.brightness.set(v.clamp(0.1, 5.0));
        self.render();
        self.apply_scale();
    }

    fn set_gamma(&self, v: f64) {
        self.gamma.set(v.clamp(0.1, 5.0));
        self.render();
        self.apply_scale();
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

    // ---- help ------------------------------------------------------
    fn help_markup(&self) -> String {
        fn row(key: &str, desc: &str) -> String {
            format!("  <tt>{:<26}</tt> {}\n", key, glib::markup_escape_text(desc))
        }
        let mut buckets = String::new();
        for b in &self.session.borrow().buckets {
            if b.is_reject {
                buckets.push_str(&row("Delete / Backspace / x", "Reject → move to _rejected/"));
            } else {
                buckets.push_str(&row(
                    &glib::markup_escape_text(&b.key),
                    &format!("Move to “{}” ({}/)", b.name, b.folder),
                ));
            }
        }
        format!(
            "<b>Navigation</b>\n{nav}\n<b>Sort into buckets</b>\n{buckets}{undo}\n\
             <b>Zoom &amp; pan</b>\n{zoom}\n<b>Image adjust</b>\n{adjust}\n\
             <b>Files &amp; views</b>\n{files}",
            nav = [
                row("→ / Space", "Next image"),
                row("←", "Previous image"),
                row("Mouse Back / Forward", "Previous / next"),
                row("Page Down / Page Up", "Jump ±10"),
                row("Home / End", "First / last"),
            ]
            .concat(),
            buckets = buckets,
            undo = row("Ctrl+Z / Ctrl+Shift+Z", "Undo / redo the last move"),
            zoom = [
                row("Scroll / pinch", "Zoom (toward cursor)"),
                row("+ / -", "Zoom in / out"),
                row("f / a", "Fit to window / 100%"),
                row("Left-drag (zoomed)", "Pan the image"),
                row("Middle-drag", "Pan the image"),
                row("Double-click", "Toggle fullscreen"),
            ]
            .concat(),
            adjust = [
                row("] / [", "Brightness up / down"),
                row("} / {", "Gamma up / down"),
                row("\\", "Reset brightness & gamma"),
            ]
            .concat(),
            files = [
                row("Left-drag (fit) / Ctrl+drag", "Drag file out (copy)"),
                row("Ctrl+C / Ctrl+Shift+C", "Copy filename / path"),
                row("Ctrl+Shift+X", "Copy image file to clipboard"),
                row("F11", "Fullscreen"),
                row("? / F1", "This shortcuts list"),
            ]
            .concat(),
        )
    }

    fn show_help(self: &Rc<Self>) {
        let label = Label::builder()
            .use_markup(true)
            .wrap(true)
            .xalign(0.0)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(14)
            .margin_end(14)
            .build();
        label.set_markup(&self.help_markup());
        let scroll =
            ScrolledWindow::builder().hexpand(true).vexpand(true).child(&label).build();
        let win = gtk4::Window::builder()
            .title("winnow — shortcuts")
            .transient_for(&self.window)
            .modal(true)
            .default_width(540)
            .default_height(680)
            .child(&scroll)
            .build();
        let key = EventControllerKey::new();
        let w = win.clone();
        key.connect_key_pressed(move |_c, k, _code, _s| {
            if k == gdk::Key::Escape || k == gdk::Key::question || k == gdk::Key::F1 {
                w.close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        win.add_controller(key);
        win.present();
    }

    // ---- event controllers ----------------------------------------
    fn build_controllers(self: &Rc<Self>) {
        self.build_keys();
        self.build_scroll();
        self.build_mouse();
        self.build_drag_source();
        self.build_context_menu();
        self.build_info_controls();
        self.build_grid();
        self.build_header();
    }

    fn build_header(self: &Rc<Self>) {
        let header = gtk4::HeaderBar::new();

        let grid_btn = gtk4::Button::with_label("Grid");
        grid_btn.set_tooltip_text(Some("Grid / single view (G)"));
        {
            let app = self.clone();
            grid_btn.connect_clicked(move |_| app.toggle_view());
        }
        header.pack_start(&grid_btn);

        let info_btn = gtk4::Button::with_label("Details");
        info_btn.set_tooltip_text(Some("Toggle details panel (I)"));
        {
            let app = self.clone();
            info_btn.connect_clicked(move |_| app.toggle_info());
        }
        header.pack_start(&info_btn);

        // Brightness / gamma sliders in a popover.
        let bri_btn = gtk4::MenuButton::new();
        bri_btn.set_label("☀ Brightness");
        bri_btn.set_tooltip_text(Some("Brightness / gamma"));
        let pop = gtk4::Popover::new();
        let pbox = gtk4::Box::new(Orientation::Vertical, 4);
        pbox.set_margin_top(8);
        pbox.set_margin_bottom(8);
        pbox.set_margin_start(8);
        pbox.set_margin_end(8);
        let bri_scale = gtk4::Scale::with_range(Orientation::Horizontal, 0.2, 3.0, 0.05);
        bri_scale.set_value(1.0);
        bri_scale.set_size_request(220, -1);
        let gam_scale = gtk4::Scale::with_range(Orientation::Horizontal, 0.3, 3.0, 0.05);
        gam_scale.set_value(1.0);
        {
            let app = self.clone();
            bri_scale.connect_value_changed(move |s| app.set_brightness(s.value()));
        }
        {
            let app = self.clone();
            gam_scale.connect_value_changed(move |s| app.set_gamma(s.value()));
        }
        let reset = gtk4::Button::with_label("Reset");
        {
            let bri = bri_scale.clone();
            let gam = gam_scale.clone();
            reset.connect_clicked(move |_| {
                bri.set_value(1.0);
                gam.set_value(1.0);
            });
        }
        let bl = Label::builder().label("Brightness").xalign(0.0).build();
        let gl = Label::builder().label("Gamma").xalign(0.0).build();
        pbox.append(&bl);
        pbox.append(&bri_scale);
        pbox.append(&gl);
        pbox.append(&gam_scale);
        pbox.append(&reset);
        pop.set_child(Some(&pbox));
        bri_btn.set_popover(Some(&pop));
        header.pack_start(&bri_btn);

        // Right side (packed in reverse visual order).
        let help_btn = gtk4::Button::with_label("?");
        help_btn.set_tooltip_text(Some("Shortcuts (?)"));
        {
            let app = self.clone();
            help_btn.connect_clicked(move |_| app.show_help());
        }
        header.pack_end(&help_btn);

        let fs_btn = gtk4::Button::with_label("Fullscreen");
        fs_btn.set_tooltip_text(Some("Fullscreen (F11)"));
        {
            let app = self.clone();
            fs_btn.connect_clicked(move |_| app.toggle_fullscreen());
        }
        header.pack_end(&fs_btn);

        let fit_btn = gtk4::Button::with_label("Fit");
        fit_btn.set_tooltip_text(Some("Fit to window (F)"));
        {
            let app = self.clone();
            fit_btn.connect_clicked(move |_| app.fit());
        }
        header.pack_end(&fit_btn);

        self.window.set_titlebar(Some(&header));
    }

    fn build_info_controls(self: &Rc<Self>) {
        {
            let app = self.clone();
            self.sort_dropdown.connect_selected_notify(move |_| app.apply_sort_from_ui());
        }
        {
            let app = self.clone();
            self.desc_check.connect_toggled(move |_| app.apply_sort_from_ui());
        }
    }

    fn apply_sort_from_ui(self: &Rc<Self>) {
        let idx = self.sort_dropdown.selected() as usize;
        if let Some((key, _)) = self.sort_keys.get(idx) {
            let key = key.clone();
            let desc = self.desc_check.is_active();
            self.session.borrow_mut().apply_sort(&key, desc);
            self.refresh();
        }
    }

    fn toggle_info(&self) {
        self.info_panel.set_visible(!self.info_panel.is_visible());
    }

    fn update_info(&self) {
        while let Some(child) = self.info_rows.first_child() {
            self.info_rows.remove(&child);
        }
        let s = self.session.borrow();
        let item = match s.current() {
            Some(i) => i,
            None => return,
        };
        self.add_section("Image");
        if let Some(pb) = self.orig_pixbuf.borrow().as_ref() {
            let (w, h) = (pb.width(), pb.height());
            let mp = (w as f64 * h as f64) / 1_000_000.0;
            self.add_row("Resolution", &format!("{w} × {h}  ({mp:.1} MP)"), false);
        }
        self.add_row("File size", &human_size(item.size_bytes()), false);
        if let Some(m) = format_mtime(item.mtime()) {
            self.add_row("Modified", &m, false);
        }
        self.add_row("Path", &item.rel_path, false);

        if !s.metadata.is_empty() {
            self.add_section("Metadata");
            if let Some(row) = s.metadata.get(&item.rel_path) {
                for col in &s.metadata.columns {
                    let val = row.get(col).map(|v| v.as_str()).unwrap_or("");
                    self.add_row(col, val, true);
                }
            }
        }
    }

    fn add_section(&self, title: &str) {
        let l = Label::builder().use_markup(true).xalign(0.0).margin_top(8).margin_bottom(2).build();
        l.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(title)));
        self.info_rows.append(&l);
    }

    fn add_row(&self, key: &str, value: &str, linkify: bool) {
        let row = gtk4::Box::new(Orientation::Horizontal, 6);
        let k = Label::builder()
            .label(format!("{key}:"))
            .xalign(0.0)
            .valign(gtk4::Align::Start)
            .width_request(92)
            .wrap(true)
            .build();
        k.add_css_class("dim-label");
        // wrap(WordChar) + bounded max width so a long, space-free value folds
        // into multiple rows instead of forcing the panel wide.
        let v = Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .max_width_chars(28)
            .hexpand(true)
            .selectable(true)
            .build();
        if linkify {
            let (markup, has) = linkify_markup(value);
            if has {
                v.set_markup(&markup);
                let win = self.window.clone();
                v.connect_activate_link(move |_l, uri| {
                    gtk4::show_uri(Some(&win), uri, 0);
                    glib::Propagation::Stop
                });
            } else {
                v.set_text(value);
            }
        } else {
            v.set_text(value);
        }
        row.append(&k);
        row.append(&v);
        self.info_rows.append(&row);
    }

    fn menu_action(self: &Rc<Self>, a: MenuAction) {
        match a {
            MenuAction::Next => {
                self.session.borrow_mut().next();
                self.refresh();
            }
            MenuAction::Prev => {
                self.session.borrow_mut().prev();
                self.refresh();
            }
            MenuAction::Bucket(i) => self.move_to_bucket(i),
            MenuAction::Undo => self.undo(),
            MenuAction::Fit => self.fit(),
            MenuAction::Actual => self.actual_size(),
            MenuAction::Fullscreen => self.toggle_fullscreen(),
            MenuAction::CopyName => self.copy_name(),
            MenuAction::CopyPath => self.copy_path(),
            MenuAction::CopyFile => self.copy_file(),
            MenuAction::Help => self.show_help(),
        }
    }

    fn build_context_menu(self: &Rc<Self>) {
        let popover = gtk4::Popover::new();
        popover.set_parent(&self.picture);
        popover.set_has_arrow(false);
        popover.set_halign(gtk4::Align::Start);

        let vbox = gtk4::Box::new(Orientation::Vertical, 0);
        vbox.set_width_request(240);

        let mut items: Vec<(String, Option<MenuAction>)> =
            vec![("Next  →".into(), Some(MenuAction::Next)), ("Previous  ←".into(), Some(MenuAction::Prev)), (String::new(), None)];
        for (i, b) in self.session.borrow().buckets.iter().enumerate() {
            let label = if b.is_reject {
                format!("Reject  ({})", b.key)
            } else {
                format!("Move to “{}”  ({})", b.name, b.key)
            };
            items.push((label, Some(MenuAction::Bucket(i))));
        }
        items.push(("Undo  (Ctrl+Z)".into(), Some(MenuAction::Undo)));
        items.push((String::new(), None));
        items.push(("Fit to window  (f)".into(), Some(MenuAction::Fit)));
        items.push(("Actual size  (a)".into(), Some(MenuAction::Actual)));
        items.push(("Fullscreen  (F11)".into(), Some(MenuAction::Fullscreen)));
        items.push((String::new(), None));
        items.push(("Copy filename  (Ctrl+C)".into(), Some(MenuAction::CopyName)));
        items.push(("Copy path  (Ctrl+Shift+C)".into(), Some(MenuAction::CopyPath)));
        items.push(("Copy image file  (Ctrl+Shift+X)".into(), Some(MenuAction::CopyFile)));
        items.push((String::new(), None));
        items.push(("Shortcuts…  (?)".into(), Some(MenuAction::Help)));

        for (label, action) in items {
            match action {
                None => vbox.append(&gtk4::Separator::new(Orientation::Horizontal)),
                Some(action) => {
                    let lbl = Label::builder().label(&label).xalign(0.0).build();
                    let btn = gtk4::Button::builder().child(&lbl).build();
                    btn.add_css_class("flat");
                    let app = self.clone();
                    let pop = popover.clone();
                    btn.connect_clicked(move |_| {
                        app.menu_action(action);
                        pop.popdown();
                    });
                    vbox.append(&btn);
                }
            }
        }
        popover.set_child(Some(&vbox));

        let click = GestureClick::new();
        click.set_button(gdk::BUTTON_SECONDARY);
        click.connect_pressed(move |_g, _n, x, y| {
            popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        });
        self.picture.add_controller(click);
    }

    fn build_keys(self: &Rc<Self>) {
        let keys = EventControllerKey::new();
        let app = self.clone();
        keys.connect_key_pressed(move |_c, keyval, _code, state| {
            use glib::Propagation::{Proceed, Stop};
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);

            // Grid mode: bucket keys move the selection; arrows/rubber-band are
            // left to the GridView.
            if app.in_grid.get() {
                if ctrl && keyval == gdk::Key::z {
                    if shift {
                        app.redo();
                    } else {
                        app.undo();
                    }
                    app.sync_grid_model();
                    return Stop;
                }
                if !ctrl {
                    if let Some(kn) = keyval.name() {
                        let kn = kn.to_string();
                        let bidx = app
                            .session
                            .borrow()
                            .buckets
                            .iter()
                            .position(|b| b.key.eq_ignore_ascii_case(&kn))
                            .or_else(|| {
                                if kn.eq_ignore_ascii_case("BackSpace")
                                    || kn.eq_ignore_ascii_case("x")
                                {
                                    Some(0)
                                } else {
                                    None
                                }
                            });
                        if let Some(i) = bidx {
                            app.move_selected(i);
                            return Stop;
                        }
                    }
                }
                match keyval {
                    gdk::Key::g => app.toggle_view(),
                    gdk::Key::Return | gdk::Key::KP_Enter => app.open_selected(),
                    gdk::Key::Escape => app.toggle_view(),
                    _ => return Proceed,
                }
                return Stop;
            }

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
                gdk::Key::i => app.toggle_info(),
                gdk::Key::g => app.toggle_view(),
                gdk::Key::question | gdk::Key::F1 => app.show_help(),
                _ => return Proceed,
            }
            Stop
        });
        self.window.add_controller(keys);
    }

    fn build_scroll(self: &Rc<Self>) {
        // Track the pointer over the viewport for zoom-to-cursor.
        let motion = gtk4::EventControllerMotion::new();
        {
            let app = self.clone();
            motion.connect_motion(move |_c, x, y| app.pointer.set((x, y)));
        }
        self.scroller.add_controller(motion);

        let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(PropagationPhase::Capture);
        {
            let app = self.clone();
            scroll.connect_scroll(move |_c, _dx, dy| {
                let (px, py) = app.pointer.get();
                app.zoom_at(ZOOM_RATE.powf(-dy), px, py);
                glib::Propagation::Stop
            });
        }
        self.scroller.add_controller(scroll);

        // A zoom changes the content size asynchronously; apply the pending
        // zoom-to-cursor scroll position once the adjustments have updated.
        {
            let app = self.clone();
            let vadj = self.scroller.vadjustment();
            self.scroller.hadjustment().connect_changed(move |hadj| {
                if let Some((th, tv)) = app.pending_scroll.take() {
                    hadj.set_value(th);
                    vadj.set_value(tv);
                }
            });
        }
    }

    fn build_mouse(self: &Rc<Self>) {
        // Middle-drag always pans.
        let mid = GestureDrag::new();
        mid.set_button(gdk::BUTTON_MIDDLE);
        self.wire_pan_gesture(&mid, false);
        self.picture.add_controller(mid);

        // Left-drag pans when zoomed in (else it becomes an OS drag-out; see
        // build_drag_source). Ctrl forces drag-out even when zoomed.
        let left = GestureDrag::new();
        left.set_button(gdk::BUTTON_PRIMARY);
        self.wire_pan_gesture(&left, true);
        self.picture.add_controller(left);

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

    /// Wire a GestureDrag to pan the scroller. `conditional` gestures (left
    /// button) only pan when the image is zoomed and Ctrl isn't held, yielding
    /// the sequence otherwise so the drag source can start an OS drag-out.
    fn wire_pan_gesture(self: &Rc<Self>, g: &GestureDrag, conditional: bool) {
        {
            let app = self.clone();
            g.connect_drag_begin(move |g, _x, _y| {
                let active = !conditional || (app.can_pan() && !event_has_ctrl(g.current_event()));
                app.pan_active.set(active);
                if active {
                    g.set_state(gtk4::EventSequenceState::Claimed);
                    app.pan_start.set((
                        app.scroller.hadjustment().value(),
                        app.scroller.vadjustment().value(),
                    ));
                    app.update_cursor();
                } else {
                    g.set_state(gtk4::EventSequenceState::Denied);
                }
            });
        }
        {
            let app = self.clone();
            g.connect_drag_update(move |_g, ox, oy| {
                if app.pan_active.get() {
                    let (sh, sv) = app.pan_start.get();
                    app.scroller.hadjustment().set_value(sh - ox);
                    app.scroller.vadjustment().set_value(sv - oy);
                }
            });
        }
        {
            let app = self.clone();
            g.connect_drag_end(move |_g, _ox, _oy| {
                if app.pan_active.get() {
                    app.pan_active.set(false);
                    app.update_cursor();
                }
            });
        }
    }

    fn build_drag_source(self: &Rc<Self>) {
        let drag = DragSource::new();
        drag.set_actions(gdk::DragAction::COPY);
        {
            let app = self.clone();
            drag.connect_prepare(move |src, _x, _y| {
                // When zoomed and no Ctrl, left-drag pans instead of dragging out.
                if app.can_pan() && !event_has_ctrl(src.current_event()) {
                    return None;
                }
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
