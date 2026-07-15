//! Thumbnail contact-sheet grid: a virtualized GridView with lazy thumbnails,
//! multi-select bucket moves, and per-cell drag-out.

use std::path::Path;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::gdk_pixbuf::Pixbuf;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{DragSource, Picture};

use super::App;

const THUMB: i32 = 200;

impl App {
    pub(super) fn build_grid(self: &Rc<Self>) {
        // Each cell is a Picture; a drag source reads the path stashed in the
        // widget name (updated per bind) so drag-out survives widget recycling.
        self.grid_factory.connect_setup(|_f, item| {
            let li = match item.downcast_ref::<gtk4::ListItem>() {
                Some(li) => li,
                None => return,
            };
            let picture = Picture::new();
            picture.set_size_request(THUMB, THUMB);
            picture.set_can_shrink(true);
            picture.set_keep_aspect_ratio(true);
            picture.set_margin_top(4);
            picture.set_margin_bottom(4);
            picture.set_margin_start(4);
            picture.set_margin_end(4);

            let drag = DragSource::new();
            drag.set_actions(gdk::DragAction::COPY);
            drag.connect_prepare(|src, _x, _y| {
                let w = src.widget()?;
                let name = w.widget_name();
                if name.is_empty() {
                    return None;
                }
                let file = gio::File::for_path(name.as_str());
                let uri = format!("{}\r\n", file.uri());
                let up = gdk::ContentProvider::for_bytes(
                    "text/uri-list",
                    &glib::Bytes::from_owned(uri.into_bytes()),
                );
                let fp = gdk::ContentProvider::for_value(&file.to_value());
                Some(gdk::ContentProvider::new_union(&[fp, up]))
            });
            drag.connect_drag_begin(|src, _drag| {
                if let Some(w) = src.widget() {
                    let name = w.widget_name();
                    if let Ok(pb) = Pixbuf::from_file_at_scale(name.as_str(), 160, 160, true) {
                        let tex = gdk::Texture::for_pixbuf(&pb);
                        src.set_icon(Some(&tex), tex.width() / 2, tex.height() / 2);
                    }
                }
            });
            picture.add_controller(drag);
            li.set_child(Some(&picture));
        });

        {
            let app = self.clone();
            self.grid_factory.connect_bind(move |_f, item| {
                let li = match item.downcast_ref::<gtk4::ListItem>() {
                    Some(li) => li,
                    None => return,
                };
                let picture = match li.child().and_downcast::<Picture>() {
                    Some(p) => p,
                    None => return,
                };
                let pos = li.position() as usize;
                let path = app.session.borrow().items.get(pos).map(|it| it.abs_path.clone());
                if let Some(path) = path {
                    picture.set_widget_name(&path.to_string_lossy());
                    match app.load_thumb(&path) {
                        Some(tex) => picture.set_paintable(Some(&tex)),
                        None => picture.set_paintable(None::<&gdk::Texture>),
                    }
                }
            });
        }

        {
            let app = self.clone();
            self.grid_view.connect_activate(move |_gv, pos| app.open_single(pos as usize));
        }
    }

    fn load_thumb(&self, path: &Path) -> Option<gdk::Texture> {
        if let Some(t) = self.thumb_cache.borrow().get(path) {
            return Some(t.clone());
        }
        let pb = Pixbuf::from_file_at_scale(path, THUMB, THUMB, true).ok()?;
        let pb = pb.apply_embedded_orientation().unwrap_or(pb);
        let tex = gdk::Texture::for_pixbuf(&pb);
        self.thumb_cache.borrow_mut().insert(path.to_path_buf(), tex.clone());
        Some(tex)
    }

    pub(super) fn sync_grid_model(&self) {
        let n = self.grid_model.n_items();
        if n > 0 {
            self.grid_model.splice(0, n, &[]);
        }
        let s = self.session.borrow();
        let strs: Vec<&str> = s.items.iter().map(|i| i.rel_path.as_str()).collect();
        self.grid_model.splice(0, 0, &strs);
    }

    pub(super) fn toggle_view(self: &Rc<Self>) {
        if self.in_grid.get() {
            self.in_grid.set(false);
            self.stack.set_visible_child_name("single");
            self.refresh();
        } else {
            self.sync_grid_model();
            self.in_grid.set(true);
            self.stack.set_visible_child_name("grid");
            let idx = self.session.borrow().index as u32;
            self.grid_selection.select_item(idx, true);
            self.grid_view.grab_focus();
        }
    }

    pub(super) fn open_single(self: &Rc<Self>, pos: usize) {
        self.session.borrow_mut().set_index(pos as isize);
        self.in_grid.set(false);
        self.stack.set_visible_child_name("single");
        self.refresh();
    }

    pub(super) fn open_selected(self: &Rc<Self>) {
        let pos =
            self.selected_positions().into_iter().next().unwrap_or(self.session.borrow().index);
        self.open_single(pos);
    }

    fn selected_positions(&self) -> Vec<usize> {
        let sel = self.grid_selection.selection();
        (0..sel.size()).map(|i| sel.nth(i as u32) as usize).collect()
    }

    pub(super) fn move_selected(self: &Rc<Self>, bucket_idx: usize) {
        let positions = self.selected_positions();
        if positions.is_empty() {
            return;
        }
        let moved = self.session.borrow_mut().move_positions(&positions, bucket_idx);
        self.sync_grid_model();
        if moved > 0 {
            let (is_reject, name) = {
                let s = self.session.borrow();
                match s.buckets.get(bucket_idx) {
                    Some(b) => (b.is_reject, b.name.clone()),
                    None => (false, String::new()),
                }
            };
            let verb = if is_reject { "Rejected".to_string() } else { format!("→ {name}") };
            self.flash(format!("{verb} {moved} image(s)"));
        }
    }
}
