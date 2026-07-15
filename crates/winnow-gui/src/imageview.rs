//! A custom image widget that paints a GdkTexture with a scale + offset
//! transform via the GPU snapshot API. Zoom and pan just change numbers and
//! queue a redraw — no relayout, no scroll-area — giving smooth, jump-free
//! interaction (the equivalent of Qt's QGraphicsView).

use std::cell::{Cell, RefCell};

use gtk4::gdk;
use gtk4::glib;
use gtk4::graphene;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ImageView {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub scale: Cell<f64>,         // display scale when not fitted
        pub offset: Cell<(f64, f64)>, // image top-left in widget coords (not fitted)
        pub fitted: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImageView {
        const NAME: &'static str = "WinnowImageView";
        type Type = super::ImageView;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for ImageView {
        fn constructed(&self) {
            self.parent_constructed();
            self.scale.set(1.0);
            self.fitted.set(true);
        }
    }

    impl WidgetImpl for ImageView {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let tex = match self.texture.borrow().clone() {
                Some(t) => t,
                None => return,
            };
            let obj = self.obj();
            let (ww, wh) = (obj.width() as f32, obj.height() as f32);
            let (tw, th) = (tex.width() as f32, tex.height() as f32);
            if tw <= 0.0 || th <= 0.0 || ww <= 0.0 || wh <= 0.0 {
                return;
            }
            let (dx, dy, dw, dh) = if self.fitted.get() {
                let s = (ww / tw).min(wh / th);
                let (dw, dh) = (tw * s, th * s);
                ((ww - dw) / 2.0, (wh - dh) / 2.0, dw, dh)
            } else {
                let s = self.scale.get() as f32;
                let (ox, oy) = self.offset.get();
                (ox as f32, oy as f32, tw * s, th * s)
            };
            // Clip to the widget so the image never overdraws siblings.
            snapshot.push_clip(&graphene::Rect::new(0.0, 0.0, ww, wh));
            snapshot.append_texture(&tex, &graphene::Rect::new(dx, dy, dw, dh));
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    pub struct ImageView(ObjectSubclass<imp::ImageView>) @extends gtk4::Widget;
}

impl Default for ImageView {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageView {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_texture(&self, t: Option<gdk::Texture>) {
        *self.imp().texture.borrow_mut() = t;
        self.queue_draw();
    }

    pub fn texture(&self) -> Option<gdk::Texture> {
        self.imp().texture.borrow().clone()
    }

    pub fn tex_size(&self) -> Option<(f64, f64)> {
        self.imp().texture.borrow().as_ref().map(|t| (t.width() as f64, t.height() as f64))
    }

    pub fn is_fitted(&self) -> bool {
        self.imp().fitted.get()
    }

    pub fn set_fitted(&self, f: bool) {
        self.imp().fitted.set(f);
        self.queue_draw();
    }

    pub fn scale(&self) -> f64 {
        self.imp().scale.get()
    }

    pub fn set_scale(&self, s: f64) {
        self.imp().scale.set(s);
        self.queue_draw();
    }

    pub fn offset(&self) -> (f64, f64) {
        self.imp().offset.get()
    }

    pub fn set_offset(&self, o: (f64, f64)) {
        self.imp().offset.set(o);
        self.queue_draw();
    }

    /// Scale at which the image exactly fits the widget (None if unsized).
    pub fn fit_scale(&self) -> Option<f64> {
        let (tw, th) = self.tex_size()?;
        let (ww, wh) = (self.width() as f64, self.height() as f64);
        if tw > 0.0 && th > 0.0 && ww > 0.0 && wh > 0.0 {
            Some((ww / tw).min(wh / th))
        } else {
            None
        }
    }

    /// Current (scale, dx, dy) actually being displayed, accounting for fit.
    pub fn display(&self) -> Option<(f64, f64, f64)> {
        let (tw, th) = self.tex_size()?;
        if self.is_fitted() {
            let s = self.fit_scale()?;
            let (ww, wh) = (self.width() as f64, self.height() as f64);
            Some((s, (ww - tw * s) / 2.0, (wh - th * s) / 2.0))
        } else {
            let (ox, oy) = self.offset();
            Some((self.scale(), ox, oy))
        }
    }
}
