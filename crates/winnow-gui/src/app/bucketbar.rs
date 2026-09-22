//! Bucket bar: one chip per bucket (hotkey, name, image count) under the
//! views. Click a chip to move the current image / grid selection there, `+`
//! adds a bucket, right-click a chip to rename or remove it. Edits are saved
//! to the bucket config by the session.

use std::rc::Rc;
use std::time::Duration;

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{GestureClick, Label, Orientation};

use super::App;

const CSS: &str = "
.bucket-chip { padding: 1px 8px; min-height: 0; }
.bucket-chip.bucket-flash { background: alpha(@theme_selected_bg_color, 0.6); }
";

/// Short form of a GDK key name for the chip.
fn key_label(key: &str) -> String {
    match key {
        "Delete" => "Del".into(),
        "BackSpace" => "⌫".into(),
        k => k.to_string(),
    }
}

/// One chip in the bar; `sig` identifies the bucket it was built for.
pub(super) struct Chip {
    button: gtk4::Button,
    label: Label,
    sig: (String, String, String),
}

fn chip_markup(key: &str, name: &str, count: usize) -> String {
    let key = if key.is_empty() {
        String::new()
    } else {
        format!("<b>{}</b>  ", glib::markup_escape_text(&key_label(key)))
    };
    format!("{key}{}  <span alpha=\"60%\">{count}</span>", glib::markup_escape_text(name))
}

/// Chips must never hold keyboard focus: a focused chip would be pressed by
/// Enter, silently moving an image. Also covers the FlowBox slot around it.
fn unfocusable_slot(w: &impl IsA<gtk4::Widget>) {
    if let Some(slot) = w.parent() {
        slot.set_focusable(false);
    }
}

/// A popover attached to `anchor` that detaches itself once closed.
fn transient_popover(anchor: &impl IsA<gtk4::Widget>) -> gtk4::Popover {
    let pop = gtk4::Popover::new();
    pop.set_parent(anchor);
    pop.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    pop
}

impl App {
    pub(super) fn build_bucket_bar(self: &Rc<Self>) {
        let provider = gtk4::CssProvider::new();
        provider.load_from_data(CSS);
        gtk4::style_context_add_provider_for_display(
            &WidgetExt::display(&self.window),
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        self.update_bucket_bar();
    }

    /// Sync the chips with the session. Counts are updated in place; the chips
    /// are only rebuilt when the bucket list itself changed (so a chip is never
    /// destroyed from inside its own click handler).
    pub(super) fn update_bucket_bar(self: &Rc<Self>) {
        {
            let s = self.session.borrow();
            let chips = self.bucket_chips.borrow();
            let same = chips.len() == s.buckets.len()
                && chips
                    .iter()
                    .zip(&s.buckets)
                    .all(|(c, b)| c.sig == (b.name.clone(), b.key.clone(), b.folder.clone()));
            if same {
                for (i, c) in chips.iter().enumerate() {
                    let b = &s.buckets[i];
                    c.label.set_markup(&chip_markup(&b.key, &b.name, s.bucket_counts[i]));
                }
                return;
            }
        }
        while let Some(child) = self.bucket_bar.first_child() {
            self.bucket_bar.remove(&child);
        }
        let mut chips = Vec::new();
        {
            let s = self.session.borrow();
            for (i, b) in s.buckets.iter().enumerate() {
                let count = s.bucket_counts.get(i).copied().unwrap_or(0);
                let lbl = Label::new(None);
                lbl.set_markup(&chip_markup(&b.key, &b.name, count));
                let chip = gtk4::Button::builder().child(&lbl).focusable(false).build();
                chip.add_css_class("bucket-chip");
                let hint = if b.is_reject { "" } else { " · right-click to rename / remove" };
                chip.set_tooltip_text(Some(&format!("Move to {}/{hint}", b.folder)));
                {
                    let app = self.clone();
                    chip.connect_clicked(move |_| {
                        if app.in_grid.get() {
                            app.move_selected(i);
                        } else {
                            app.move_to_bucket(i);
                        }
                    });
                }
                if !b.is_reject {
                    let click = GestureClick::new();
                    click.set_button(gdk::BUTTON_SECONDARY);
                    let app = self.clone();
                    let c = chip.clone();
                    click.connect_pressed(move |_, _, _, _| app.show_chip_menu(&c, i));
                    chip.add_controller(click);
                }
                self.bucket_bar.append(&chip);
                unfocusable_slot(&chip);
                chips.push(Chip {
                    button: chip,
                    label: lbl,
                    sig: (b.name.clone(), b.key.clone(), b.folder.clone()),
                });
            }
        }
        let add = self.add_bucket_button();
        self.bucket_bar.append(&add);
        unfocusable_slot(&add);
        *self.bucket_chips.borrow_mut() = chips;
    }

    /// Briefly highlight a chip so a keypress shows where the image went.
    pub(super) fn flash_chip(&self, idx: usize) {
        if let Some(chip) = self.bucket_chips.borrow().get(idx).map(|c| c.button.clone()) {
            chip.add_css_class("bucket-flash");
            glib::timeout_add_local_once(Duration::from_millis(350), move || {
                chip.remove_css_class("bucket-flash");
            });
        }
    }

    /// After buckets change: rebuild the bar (deferred, as the edit comes from
    /// a popover owned by a chip) and reload the views, since adding a bucket
    /// over an existing folder can shrink the queue.
    fn buckets_changed(self: &Rc<Self>) {
        let app = self.clone();
        glib::idle_add_local_once(move || {
            app.update_bucket_bar();
            app.refresh();
            if app.in_grid.get() {
                app.sync_grid_model();
            }
        });
    }

    fn add_bucket_button(self: &Rc<Self>) -> gtk4::Button {
        let btn = gtk4::Button::builder().icon_name("list-add-symbolic").focusable(false).build();
        btn.add_css_class("bucket-chip");
        btn.set_tooltip_text(Some("Add a bucket (gets the next free number key)"));
        let app = self.clone();
        btn.connect_clicked(move |b| app.show_add_popover(b));
        btn
    }

    fn show_add_popover(self: &Rc<Self>, anchor: &gtk4::Button) {
        let entry = gtk4::Entry::builder().placeholder_text("New bucket name").build();
        let add = gtk4::Button::with_label("Add");
        let row = gtk4::Box::new(Orientation::Horizontal, 6);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        row.set_margin_start(6);
        row.set_margin_end(6);
        row.append(&entry);
        row.append(&add);
        let pop = transient_popover(anchor);
        pop.set_child(Some(&row));

        let submit = {
            let app = self.clone();
            let (e, p) = (entry.clone(), pop.clone());
            Rc::new(move || {
                let res = app.session.borrow_mut().add_bucket(&e.text());
                match res {
                    Ok(i) => {
                        p.popdown();
                        let (name, key) = {
                            let s = app.session.borrow();
                            (s.buckets[i].name.clone(), s.buckets[i].key.clone())
                        };
                        app.buckets_changed();
                        let key = if key.is_empty() { "click its chip".into() } else { format!("key {key}") };
                        app.flash(format!("Added bucket “{name}” ({key})"));
                    }
                    Err(msg) => app.flash(msg),
                }
            })
        };
        {
            let f = submit.clone();
            entry.connect_activate(move |_| f());
        }
        add.connect_clicked(move |_| submit());
        pop.popup();
        entry.grab_focus();
    }

    /// Right-click menu on a category chip: rename, or remove (confirming
    /// first when the folder still holds images).
    fn show_chip_menu(self: &Rc<Self>, chip: &gtk4::Button, idx: usize) {
        let (name, folder, count) = {
            let s = self.session.borrow();
            match s.buckets.get(idx) {
                Some(b) => (b.name.clone(), b.folder.clone(), s.bucket_counts[idx]),
                None => return,
            }
        };
        let pop = transient_popover(chip);

        let vbox = gtk4::Box::new(Orientation::Vertical, 6);
        vbox.set_margin_top(6);
        vbox.set_margin_bottom(6);
        vbox.set_margin_start(6);
        vbox.set_margin_end(6);
        let entry = gtk4::Entry::builder().text(&name).build();
        let rename = gtk4::Button::with_label("Rename");
        let row = gtk4::Box::new(Orientation::Horizontal, 6);
        row.append(&entry);
        row.append(&rename);
        let remove = gtk4::Button::with_label("Remove bucket…");
        remove.add_css_class("flat");
        vbox.append(&row);
        vbox.append(&remove);
        pop.set_child(Some(&vbox));

        let do_rename = {
            let app = self.clone();
            let (e, p) = (entry.clone(), pop.clone());
            Rc::new(move || {
                let res = app.session.borrow_mut().rename_bucket(idx, &e.text());
                match res {
                    Ok(()) => {
                        p.popdown();
                        app.buckets_changed();
                        app.flash(format!("Renamed bucket to “{}”", e.text().trim()));
                    }
                    Err(msg) => app.flash(msg),
                }
            })
        };
        {
            let f = do_rename.clone();
            entry.connect_activate(move |_| f());
        }
        rename.connect_clicked(move |_| do_rename());

        let do_remove = {
            let app = self.clone();
            let (p, name) = (pop.clone(), name.clone());
            Rc::new(move || {
                p.popdown();
                let res = app.session.borrow_mut().remove_bucket(idx);
                match res {
                    Ok(()) => {
                        app.buckets_changed();
                        app.flash(format!("Removed bucket “{name}”"));
                    }
                    Err(msg) => app.flash(msg),
                }
            })
        };
        {
            let (p, name) = (pop.clone(), name.clone());
            remove.connect_clicked(move |_| {
                if count == 0 {
                    do_remove();
                    return;
                }
                // Swap in a confirmation: the files stay where they are.
                let vbox = gtk4::Box::new(Orientation::Vertical, 6);
                vbox.set_margin_top(6);
                vbox.set_margin_bottom(6);
                vbox.set_margin_start(6);
                vbox.set_margin_end(6);
                let msg = Label::builder()
                    .label(format!(
                        "Remove “{name}”? Its {count} image(s) stay in {folder}/ and \
                         rejoin the queue next time this folder is opened."
                    ))
                    .wrap(true)
                    .max_width_chars(36)
                    .xalign(0.0)
                    .build();
                let yes = gtk4::Button::with_label("Remove");
                yes.add_css_class("destructive-action");
                let f = do_remove.clone();
                yes.connect_clicked(move |_| f());
                vbox.append(&msg);
                vbox.append(&yes);
                p.set_child(Some(&vbox));
            });
        }
        pop.popup();
        entry.grab_focus();
    }
}
