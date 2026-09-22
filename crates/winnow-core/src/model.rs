//! Session state: the working image list, current position, sorting, and the
//! reversible move/undo engine shared by 'reject' and every category bucket.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::buckets::{
    bucket_folder_names, config_file, discover_buckets, load_buckets, next_free_key, save_buckets,
    Bucket, BucketError,
};
use crate::metadata::{Metadata, SortKey};
use crate::scan::{is_image, scan_folder};

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
    /// Images currently in each bucket's folder (parallel to `buckets`).
    pub bucket_counts: Vec<usize>,
    /// Where bucket edits are saved.
    pub config_path: PathBuf,
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
        let config_path = config_file(&root, buckets_config);
        let mut buckets = load_buckets(&root, buckets_config)?;
        if !config_path.exists() {
            discover_buckets(&root, &mut buckets);
        }
        let bucket_counts = buckets.iter().map(|b| count_images(&b.target_dir(&root))).collect();
        let metadata = match metadata_path {
            Some(p) => {
                let mut m = Metadata::load_csv(p).unwrap_or_default();
                m.relativize(&root);
                m
            }
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
            bucket_counts,
            config_path,
            metadata,
            items,
            index: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            sort_key: "name".into(),
            sort_reverse: false,
        })
    }

    /// An empty session (no folder open). Used when winnow is launched with no
    /// path so it doesn't scan the current directory.
    pub fn empty() -> Session {
        Session {
            root: PathBuf::new(),
            recursive: true,
            buckets: vec![crate::buckets::default_reject()],
            bucket_counts: vec![0],
            config_path: PathBuf::new(),
            metadata: Metadata::default(),
            items: Vec::new(),
            index: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            sort_key: "name".into(),
            sort_reverse: false,
        }
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

    // ---- bucket editing --------------------------------------------
    fn bump_count(&mut self, bucket_name: &str, delta: isize) {
        if let Some(i) = self.bucket_index_by_name(bucket_name) {
            self.bucket_counts[i] = self.bucket_counts[i].saturating_add_signed(delta);
        }
    }

    fn check_name(&self, name: &str, except: Option<usize>) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Bucket name is empty".into());
        }
        if name.starts_with('.') || name.contains(['/', '\\']) {
            return Err(format!("“{name}” can't be used as a folder name"));
        }
        let taken = self.buckets.iter().enumerate().any(|(i, b)| {
            Some(i) != except
                && (b.name.eq_ignore_ascii_case(name) || b.folder.eq_ignore_ascii_case(&format!("_{name}")))
        });
        if taken {
            return Err(format!("A bucket named “{name}” already exists"));
        }
        Ok(name.to_string())
    }

    fn save_buckets(&self) -> Result<(), String> {
        save_buckets(&self.config_path, &self.buckets)
            .map_err(|e| format!("Couldn't save {}: {e}", self.config_path.display()))
    }

    /// Add a bucket `name` (folder `_name`, next free digit hotkey) and save
    /// the config. Images already inside that folder leave the queue.
    pub fn add_bucket(&mut self, name: &str) -> Result<usize, String> {
        if self.root.as_os_str().is_empty() {
            return Err("Open a folder first".into());
        }
        let name = self.check_name(name, None)?;
        let bucket =
            Bucket { key: next_free_key(&self.buckets), folder: format!("_{name}"), name, is_reject: false };
        let dir = bucket.target_dir(&self.root);
        let before = self.items.len();
        self.items.retain(|it| !it.abs_path.starts_with(&dir));
        if self.items.len() != before {
            self.clamp_index();
        }
        self.bucket_counts.push(count_images(&dir));
        self.buckets.push(bucket);
        self.save_buckets()?;
        Ok(self.buckets.len() - 1)
    }

    /// Rename a category bucket. Its folder is renamed too when it follows the
    /// `_name` convention; pending undo/redo steps are pointed at the new
    /// folder.
    pub fn rename_bucket(&mut self, idx: usize, new_name: &str) -> Result<(), String> {
        match self.buckets.get(idx) {
            Some(b) if !b.is_reject => {}
            _ => return Err("That bucket can't be renamed".into()),
        }
        let new_name = self.check_name(new_name, Some(idx))?;
        let old = self.buckets[idx].clone();
        let mut new = old.clone();
        new.name = new_name;
        if old.folder == format!("_{}", old.name) {
            new.folder = format!("_{}", new.name);
            let (from, to) = (old.target_dir(&self.root), new.target_dir(&self.root));
            if to.exists() {
                return Err(format!("{} already exists", to.display()));
            }
            if from.exists() {
                std::fs::rename(&from, &to).map_err(|e| format!("Couldn't rename folder: {e}"))?;
            }
            for op in self.undo_stack.iter_mut().chain(self.redo_stack.iter_mut()) {
                if let Ok(rest) = op.to_abs.strip_prefix(&from) {
                    op.to_abs = to.join(rest);
                }
            }
        }
        for op in self.undo_stack.iter_mut().chain(self.redo_stack.iter_mut()) {
            if op.bucket_name == old.name {
                op.bucket_name = new.name.clone();
            }
        }
        self.buckets[idx] = new;
        self.save_buckets()
    }

    /// Drop a category bucket from the config. Its folder and files are left
    /// untouched (they rejoin the queue the next time the folder is opened).
    pub fn remove_bucket(&mut self, idx: usize) -> Result<(), String> {
        match self.buckets.get(idx) {
            Some(b) if !b.is_reject => {}
            _ => return Err("That bucket can't be removed".into()),
        }
        self.buckets.remove(idx);
        self.bucket_counts.remove(idx);
        self.save_buckets()
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
        self.bucket_counts[bucket_idx] += 1;
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

    /// Move several images (by list position) into a bucket. Each is an
    /// independent undo step. Returns the number moved.
    pub fn move_positions(&mut self, positions: &[usize], bucket_idx: usize) -> usize {
        let mut sorted: Vec<usize> = positions.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let mut moved = 0;
        // Move highest index first so earlier indices stay valid.
        for &pos in sorted.iter().rev() {
            if let Some(op) = self.do_move(pos, bucket_idx) {
                self.undo_stack.push(op);
                moved += 1;
            }
        }
        if moved > 0 {
            self.redo_stack.clear();
            self.clamp_index();
        }
        moved
    }

    pub fn undo(&mut self) -> Option<String> {
        let op = self.undo_stack.pop()?;
        let restore = unique_dest(&op.from_abs);
        if move_file(&op.to_abs, &restore).is_err() {
            self.undo_stack.push(op);
            return None;
        }
        self.bump_count(&op.bucket_name, -1);
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
        self.bump_count(&bucket_name, 1);
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

/// Number of image files anywhere under `dir` (0 if it doesn't exist).
fn count_images(dir: &Path) -> usize {
    if !dir.is_dir() {
        return 0;
    }
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && is_image(e.path()))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buckets::CONFIG_NAME;
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
    fn move_positions_bulk() {
        let (root, mut s) = make_session(6);
        let moved = s.move_positions(&[0, 2, 4], 0);
        assert_eq!(moved, 3);
        assert_eq!(s.count(), 3);
        assert!(root.join("_rejected/img_000.jpg").exists());
        assert!(root.join("_rejected/img_002.jpg").exists());
        assert!(root.join("_rejected/img_004.jpg").exists());
        // remaining are the odd ones
        let names: Vec<String> = s.items.iter().map(|i| i.name()).collect();
        assert_eq!(names, vec!["img_001.jpg", "img_003.jpg", "img_005.jpg"]);
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

    fn tmp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("winnow-model-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn existing_bucket_folders_are_discovered_counted_and_skipped() {
        let root = tmp_root("disc");
        fs::create_dir_all(root.join("_crack/sub")).unwrap();
        fs::write(root.join("_crack/sub/a.jpg"), b"x").unwrap();
        fs::write(root.join("_crack/b.png"), b"x").unwrap();
        fs::write(root.join("todo.jpg"), b"x").unwrap();
        let s = Session::new(&root, true, None, None).unwrap();
        assert_eq!(s.buckets[1].name, "crack");
        assert_eq!(s.buckets[1].key, "1");
        assert_eq!(s.bucket_counts, vec![0, 2]);
        assert_eq!(s.count(), 1); // only todo.jpg is left to sort
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn config_disables_discovery() {
        let root = tmp_root("cfg");
        fs::create_dir_all(root.join("_cache")).unwrap();
        fs::write(root.join(CONFIG_NAME), "").unwrap();
        let s = Session::new(&root, true, None, None).unwrap();
        assert_eq!(s.buckets.len(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn add_move_undo_updates_counts_and_config() {
        let root = tmp_root("add");
        for i in 0..3 {
            fs::write(root.join(format!("img_{i}.jpg")), b"x").unwrap();
        }
        let mut s = Session::new(&root, true, None, None).unwrap();
        assert!(s.add_bucket("  ").is_err());
        let i = s.add_bucket("crack").unwrap();
        assert!(s.add_bucket("Crack").is_err());
        assert_eq!(s.buckets[i].key, "1");
        assert_eq!(load_buckets(&root, None).unwrap(), s.buckets);

        s.move_current_to(i).unwrap();
        assert_eq!(s.bucket_counts[i], 1);
        s.undo().unwrap();
        assert_eq!(s.bucket_counts[i], 0);
        s.redo().unwrap();
        assert_eq!(s.bucket_counts[i], 1);
        assert!(root.join("_crack/img_0.jpg").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn adding_bucket_over_existing_folder_pulls_its_images_from_queue() {
        let root = tmp_root("addex");
        fs::write(root.join(CONFIG_NAME), "").unwrap(); // no discovery
        fs::create_dir_all(root.join("_spall")).unwrap();
        fs::write(root.join("_spall/a.jpg"), b"x").unwrap();
        fs::write(root.join("b.jpg"), b"x").unwrap();
        let mut s = Session::new(&root, true, None, None).unwrap();
        assert_eq!(s.count(), 2);
        let i = s.add_bucket("spall").unwrap();
        assert_eq!(s.count(), 1);
        assert_eq!(s.bucket_counts[i], 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_moves_folder_and_keeps_undo_working() {
        let root = tmp_root("ren");
        fs::write(root.join("a.jpg"), b"x").unwrap();
        let mut s = Session::new(&root, true, None, None).unwrap();
        let i = s.add_bucket("crak").unwrap();
        s.move_current_to(i).unwrap();
        s.rename_bucket(i, "crack").unwrap();
        assert!(root.join("_crack/a.jpg").exists());
        assert!(!root.join("_crak").exists());
        assert!(s.rename_bucket(0, "nope").is_err());
        s.undo().unwrap();
        assert!(root.join("a.jpg").exists());
        assert_eq!(s.bucket_counts[i], 0);
        assert_eq!(load_buckets(&root, None).unwrap()[1].name, "crack");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn remove_keeps_files_and_updates_config() {
        let root = tmp_root("rm");
        fs::write(root.join("a.jpg"), b"x").unwrap();
        let mut s = Session::new(&root, true, None, None).unwrap();
        let i = s.add_bucket("x").unwrap();
        s.move_current_to(i).unwrap();
        assert!(s.remove_bucket(0).is_err());
        s.remove_bucket(i).unwrap();
        assert_eq!(s.buckets.len(), 1);
        assert_eq!(s.bucket_counts.len(), 1);
        assert!(root.join("_x/a.jpg").exists());
        assert_eq!(load_buckets(&root, None).unwrap().len(), 1);
        let _ = fs::remove_dir_all(&root);
    }
}
