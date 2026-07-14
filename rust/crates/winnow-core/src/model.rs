//! Session state: the working image list, current position, sorting, and the
//! reversible move/undo engine shared by 'reject' and every category bucket.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::buckets::{bucket_folder_names, load_buckets, Bucket, BucketError};
use crate::metadata::{Metadata, SortKey};
use crate::scan::scan_folder;

pub struct ImageItem {
    pub abs_path: PathBuf,
    pub rel_path: String,
}

impl ImageItem {
    pub fn new(abs_path: PathBuf, root: &Path) -> Self {
        let rel = abs_path
            .strip_prefix(root)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| abs_path.clone());
        ImageItem { abs_path, rel_path: rel.to_string_lossy().replace('\\', "/") }
    }

    pub fn name(&self) -> String {
        self.abs_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn size_bytes(&self) -> u64 {
        std::fs::metadata(&self.abs_path).map(|m| m.len()).unwrap_or(0)
    }

    pub fn mtime(&self) -> f64 {
        std::fs::metadata(&self.abs_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }
}

struct MoveOp {
    item: ImageItem,
    from_abs: PathBuf,
    to_abs: PathBuf,
    list_index: usize,
    bucket_name: String,
}

/// Built-in sort keys: (id, label).
pub const BUILTIN_SORTS: &[(&str, &str)] = &[
    ("name", "Name"),
    ("path", "Path"),
    ("mtime", "Date modified"),
    ("size", "File size"),
];

fn unique_dest(dest: &Path) -> PathBuf {
    if !dest.exists() {
        return dest.to_path_buf();
    }
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let ext = dest.extension().and_then(|s| s.to_str());
    let mut i = 1;
    loop {
        let name = match ext {
            Some(e) => format!("{stem}__{i}.{e}"),
            None => format!("{stem}__{i}"),
        };
        let cand = parent.join(name);
        if !cand.exists() {
            return cand;
        }
        i += 1;
    }
}

fn move_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p)?;
    }
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Cross-filesystem: copy then remove.
            std::fs::copy(src, dst)?;
            std::fs::remove_file(src)
        }
    }
}

pub struct Session {
    pub root: PathBuf,
    pub recursive: bool,
    pub buckets: Vec<Bucket>,
    pub metadata: Metadata,
    pub items: Vec<ImageItem>,
    pub index: usize,
    undo_stack: Vec<MoveOp>,
    redo_stack: Vec<MoveOp>,
    pub sort_key: String,
    pub sort_reverse: bool,
}

impl Session {
    pub fn new(
        root: &Path,
        recursive: bool,
        buckets_config: Option<&Path>,
        metadata_path: Option<&Path>,
    ) -> Result<Session, BucketError> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let buckets = load_buckets(&root, buckets_config)?;
        let metadata = match metadata_path {
            Some(p) => Metadata::load_csv(p).unwrap_or_default(),
            None => Metadata::default(),
        };
        let exclude = bucket_folder_names(&buckets, &root);
        let items = scan_folder(&root, recursive, &exclude)
            .into_iter()
            .map(|p| ImageItem::new(p, &root))
            .collect();
        Ok(Session {
            root,
            recursive,
            buckets,
            metadata,
            items,
            index: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            sort_key: "name".into(),
            sort_reverse: false,
        })
    }

    // ---- navigation ------------------------------------------------
    pub fn count(&self) -> usize {
        self.items.len()
    }

    pub fn current(&self) -> Option<&ImageItem> {
        self.items.get(self.index)
    }

    pub fn set_index(&mut self, i: isize) {
        if self.items.is_empty() {
            self.index = 0;
            return;
        }
        let max = self.items.len() as isize - 1;
        self.index = i.clamp(0, max) as usize;
    }

    pub fn next(&mut self) {
        self.set_index(self.index as isize + 1);
    }

    pub fn prev(&mut self) {
        self.set_index(self.index as isize - 1);
    }

    pub fn jump(&mut self, delta: isize) {
        self.set_index(self.index as isize + delta);
    }

    fn clamp_index(&mut self) {
        if self.index >= self.items.len() {
            self.index = self.items.len().saturating_sub(1);
        }
    }

    // ---- sorting ---------------------------------------------------
    pub fn sortable_keys(&self) -> Vec<(String, String)> {
        let mut keys: Vec<(String, String)> =
            BUILTIN_SORTS.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect();
        for col in &self.metadata.columns {
            keys.push((format!("meta:{col}"), format!("[meta] {col}")));
        }
        keys
    }

    fn sort_value(&self, item: &ImageItem, key: &str) -> SortKey {
        match key {
            "name" => SortKey::Text(item.name().to_ascii_lowercase()),
            "path" => SortKey::Text(item.rel_path.to_ascii_lowercase()),
            "mtime" => SortKey::Num(item.mtime()),
            "size" => SortKey::Num(item.size_bytes() as f64),
            _ => {
                if let Some(col) = key.strip_prefix("meta:") {
                    self.metadata.sort_value(&item.rel_path, col)
                } else {
                    SortKey::Text(item.name().to_ascii_lowercase())
                }
            }
        }
    }

    pub fn apply_sort(&mut self, key: &str, reverse: bool) {
        self.sort_key = key.to_string();
        self.sort_reverse = reverse;
        // Precompute keys to avoid repeated fs stats during comparison.
        let mut decorated: Vec<(SortKey, usize)> =
            self.items.iter().enumerate().map(|(i, it)| (self.sort_value(it, key), i)).collect();
        decorated.sort_by(|a, b| a.0.cmp(&b.0));
        if reverse {
            decorated.reverse();
        }
        let order: Vec<usize> = decorated.into_iter().map(|(_, i)| i).collect();
        let mut taken: Vec<Option<ImageItem>> = self.items.drain(..).map(Some).collect();
        self.items = order.into_iter().map(|i| taken[i].take().unwrap()).collect();
        self.index = 0; // jump to the first image of the new ordering
    }

    // ---- move / undo engine ---------------------------------------
    pub fn bucket_index_by_name(&self, name: &str) -> Option<usize> {
        self.buckets.iter().position(|b| b.name == name)
    }

    fn do_move(&mut self, item_pos: usize, bucket_idx: usize) -> Option<MoveOp> {
        let bucket = self.buckets.get(bucket_idx)?.clone();
        let item = self.items.get(item_pos)?;
        let dest = unique_dest(&bucket.target_dir(&self.root).join(&item.rel_path));
        if move_file(&item.abs_path, &dest).is_err() {
            return None;
        }
        let item = self.items.remove(item_pos);
        Some(MoveOp {
            from_abs: item.abs_path.clone(),
            to_abs: dest,
            list_index: item_pos,
            bucket_name: bucket.name.clone(),
            item,
        })
    }

    /// Move the current image into `bucket_idx`. Returns a status message.
    pub fn move_current_to(&mut self, bucket_idx: usize) -> Option<String> {
        if self.items.is_empty() {
            return None;
        }
        let pos = self.index;
        let op = self.do_move(pos, bucket_idx)?;
        let is_reject = self.buckets[bucket_idx].is_reject;
        let name = op.item.name();
        self.undo_stack.push(op);
        self.redo_stack.clear();
        self.clamp_index();
        Some(if is_reject {
            format!("Rejected: {name}")
        } else {
            format!("→ {}: {name}", self.buckets[bucket_idx].name)
        })
    }

    pub fn undo(&mut self) -> Option<String> {
        let op = self.undo_stack.pop()?;
        let restore = unique_dest(&op.from_abs);
        if move_file(&op.to_abs, &restore).is_err() {
            self.undo_stack.push(op);
            return None;
        }
        let mut item = op.item;
        item.abs_path = restore;
        let name = item.name();
        let idx = op.list_index.min(self.items.len());
        self.items.insert(idx, item);
        self.index = idx;
        self.redo_stack.push(MoveOp {
            item: ImageItem::new(self.items[idx].abs_path.clone(), &self.root),
            from_abs: op.from_abs,
            to_abs: op.to_abs,
            list_index: op.list_index,
            bucket_name: op.bucket_name,
        });
        Some(format!("Undo: restored {name}"))
    }

    pub fn redo(&mut self) -> Option<String> {
        let op = self.redo_stack.pop()?;
        let dest = unique_dest(&op.to_abs);
        let pos = self.items.iter().position(|it| it.abs_path == op.from_abs).unwrap_or(self.index);
        if pos >= self.items.len() {
            self.redo_stack.push(op);
            return None;
        }
        if move_file(&self.items[pos].abs_path, &dest).is_err() {
            self.redo_stack.push(op);
            return None;
        }
        let item = self.items.remove(pos);
        let name = item.name();
        let bucket_name = op.bucket_name.clone();
        self.undo_stack.push(MoveOp {
            from_abs: op.from_abs,
            to_abs: dest,
            list_index: pos,
            bucket_name: op.bucket_name,
            item,
        });
        self.clamp_index();
        Some(format!("Redo: {bucket_name} {name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_session(n: usize) -> (PathBuf, Session) {
        let mut root = std::env::temp_dir();
        root.push(format!("winnow-model-{}-{:p}", std::process::id(), &n as *const _));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for i in 0..n {
            fs::write(root.join(format!("img_{i:03}.jpg")), b"x").unwrap();
        }
        let s = Session::new(&root, true, Some(Path::new("/none")), None).unwrap();
        (root, s)
    }

    #[test]
    fn reject_and_undo_roundtrip() {
        let (root, mut s) = make_session(5);
        assert_eq!(s.count(), 5);
        s.set_index(2);
        let rejected = s.current().unwrap().rel_path.clone();

        let msg = s.move_current_to(0).unwrap();
        assert!(msg.starts_with("Rejected"));
        assert_eq!(s.count(), 4);
        assert!(root.join("_rejected").join(&rejected).exists());
        assert!(!root.join(&rejected).exists());

        s.undo().unwrap();
        assert_eq!(s.count(), 5);
        assert!(root.join(&rejected).exists());
        assert!(!root.join("_rejected").join(&rejected).exists());

        s.redo().unwrap();
        assert_eq!(s.count(), 4);
        assert!(root.join("_rejected").join(&rejected).exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sort_by_name_desc_and_index_resets() {
        let (root, mut s) = make_session(4);
        s.set_index(3);
        s.apply_sort("name", true);
        assert_eq!(s.index, 0);
        assert_eq!(s.items[0].name(), "img_003.jpg");
        assert_eq!(s.items[3].name(), "img_000.jpg");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn navigation_clamps() {
        let (root, mut s) = make_session(3);
        s.prev();
        assert_eq!(s.index, 0);
        s.set_index(99);
        assert_eq!(s.index, 2);
        s.jump(-10);
        assert_eq!(s.index, 0);
        let _ = fs::remove_dir_all(&root);
    }
}
