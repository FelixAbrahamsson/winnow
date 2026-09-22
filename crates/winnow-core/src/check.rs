//! `winnow --check`: validate a folder's metadata file against its images and
//! explain what winnow will make of it — a feedback loop for whoever (often an
//! AI agent) wrote the file.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

use crate::metadata::{find_metadata, Metadata, AUTO_METADATA, PATH_COLUMNS};
use crate::model::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Default)]
pub struct CheckReport {
    pub lines: Vec<(Level, String)>,
}

impl CheckReport {
    fn info(&mut self, s: impl Into<String>) {
        self.lines.push((Level::Info, s.into()));
    }
    fn warn(&mut self, s: impl Into<String>) {
        self.lines.push((Level::Warn, s.into()));
    }
    fn error(&mut self, s: impl Into<String>) {
        self.lines.push((Level::Error, s.into()));
    }
    pub fn count(&self, level: Level) -> usize {
        self.lines.iter().filter(|(l, _)| *l == level).count()
    }
    pub fn has_errors(&self) -> bool {
        self.count(Level::Error) > 0
    }
}

impl fmt::Display for CheckReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (level, line) in &self.lines {
            let tag = match level {
                Level::Info => "     ",
                Level::Warn => "WARN ",
                Level::Error => "ERROR",
            };
            writeln!(f, "{tag} {line}")?;
        }
        let (e, w) = (self.count(Level::Error), self.count(Level::Warn));
        if e > 0 {
            write!(f, "Result: {e} error(s), {w} warning(s)")
        } else {
            write!(f, "Result: OK, {w} warning(s)")
        }
    }
}

/// Up to `n` items as a quoted, comma-separated list.
fn examples<'a>(items: impl IntoIterator<Item = &'a str>, n: usize) -> String {
    let mut v: Vec<&str> = items.into_iter().collect();
    v.sort_unstable();
    let more = v.len().saturating_sub(n);
    let mut s = v.iter().take(n).map(|x| format!("'{x}'")).collect::<Vec<_>>().join(", ");
    if more > 0 {
        s.push_str(&format!(", … (+{more})"));
    }
    s
}

pub fn check(
    root: &Path,
    recursive: bool,
    metadata_path: Option<&Path>,
    buckets: Option<&Path>,
) -> CheckReport {
    let mut r = CheckReport::default();
    if !root.is_dir() {
        r.error(format!("{} is not a folder", root.display()));
        return r;
    }
    let session = match Session::new(root, recursive, buckets, None) {
        Ok(s) => s,
        Err(e) => {
            r.error(format!("Bucket config: {e}"));
            return r;
        }
    };
    let root = session.root.clone();
    r.info(format!("Folder: {}", root.display()));
    let bucket_dirs: Vec<&str> = session.buckets.iter().map(|b| b.folder.as_str()).collect();
    r.info(format!(
        "Images to review: {} ({}; bucket folders skipped: {})",
        session.count(),
        if recursive { "recursive" } else { "top level only" },
        bucket_dirs.join(", ")
    ));

    // ---- locate + parse ----
    let Some(meta_path) = metadata_path.map(Path::to_path_buf).or_else(|| find_metadata(&root)) else {
        r.error(format!(
            "No metadata file: looked for {} in the folder (or pass --metadata FILE)",
            AUTO_METADATA.join(" / ")
        ));
        return r;
    };
    r.info(format!("Metadata: {}", meta_path.display()));
    let ext = meta_path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(ext.as_str(), "csv" | "tsv") {
        r.error(format!("Unsupported metadata format '.{ext}': use .csv (or tab-separated .tsv)"));
        return r;
    }
    if !meta_path.is_file() {
        r.error(format!("{} does not exist", meta_path.display()));
        return r;
    }
    let mut meta = match Metadata::load_csv(&meta_path) {
        Ok(m) => m,
        Err(e) => {
            r.error(format!("Could not parse: {e}"));
            return r;
        }
    };
    meta.relativize(&root);
    let n_rows = meta.keys().count();
    if meta.key_column_recognized {
        r.info(format!("Path column: '{}'", meta.key_column));
    } else {
        r.warn(format!(
            "No path column found (expected a header named one of: {}); using the first \
             column '{}'",
            PATH_COLUMNS.join(", "),
            meta.key_column
        ));
    }
    r.info(format!("Rows: {n_rows}, other columns: {}", meta.columns.join(", ")));
    if n_rows == 0 {
        r.error("The file has no data rows");
        return r;
    }
    if !meta.duplicate_keys.is_empty() {
        r.warn(format!(
            "{} path(s) appear on more than one row; the last row wins: {}",
            meta.duplicate_keys.len(),
            examples(meta.duplicate_keys.iter().map(|s| s.as_str()), 5)
        ));
    }

    // ---- match rows <-> images ----
    let mut used: HashSet<&str> = HashSet::new();
    let mut per_key: HashMap<&str, usize> = HashMap::new();
    let mut unmatched: Vec<&str> = Vec::new();
    let mut by_name = 0;
    let mut wrong_prefix: Vec<(&str, &str)> = Vec::new();
    for it in &session.items {
        match meta.row_key(&it.rel_path) {
            Some((k, exact)) => {
                used.insert(k);
                if !exact {
                    by_name += 1;
                    if k.contains('/') {
                        wrong_prefix.push((k, it.rel_path.as_str()));
                    }
                    *per_key.entry(k).or_default() += 1;
                }
            }
            None => unmatched.push(it.rel_path.as_str()),
        }
    }
    let matched = session.count() - unmatched.len();
    if session.count() > 0 && matched == 0 {
        r.error(format!(
            "No image matched any row. Paths must be relative to the folder, like '{}'; the \
             file has keys like {}",
            session.items[0].rel_path,
            examples(meta.keys(), 3)
        ));
    } else {
        r.info(format!("Images with a row: {matched} of {}", session.count()));
    }
    if !unmatched.is_empty() && matched > 0 {
        r.warn(format!(
            "{} image(s) have no row (shown with blank values, sorted last): {}",
            unmatched.len(),
            examples(unmatched.iter().copied(), 5)
        ));
    }
    if !wrong_prefix.is_empty() {
        let (k, rel) = wrong_prefix[0];
        r.warn(format!(
            "{} row(s) only matched by filename because their folder part is wrong, e.g. \
             '{k}' for image '{rel}' — write paths relative to the folder",
            wrong_prefix.len()
        ));
    } else if by_name > 0 {
        r.info(format!("{by_name} image(s) matched by filename only (key without subfolder)"));
    }
    let shared: Vec<&str> = per_key.iter().filter(|(_, &n)| n > 1).map(|(k, _)| *k).collect();
    if !shared.is_empty() {
        r.warn(format!(
            "{} filename-only key(s) apply to several images in different subfolders — use \
             the full relative path: {}",
            shared.len(),
            examples(shared.iter().copied(), 5)
        ));
    }
    let orphans: Vec<&str> = meta.keys().filter(|k| !used.contains(k)).collect();
    if !orphans.is_empty() {
        // Rows for images already moved into a bucket are expected.
        let in_bucket = |k: &str| session.buckets.iter().any(|b| b.target_dir(&root).join(k).is_file());
        let (sorted, missing): (Vec<&str>, Vec<&str>) = orphans.iter().partition(|k| in_bucket(k));
        if !sorted.is_empty() {
            r.info(format!("{} row(s) belong to images already moved into a bucket", sorted.len()));
        }
        if !missing.is_empty() {
            r.warn(format!(
                "{} row(s) match no image: {}",
                missing.len(),
                examples(missing.iter().copied(), 5)
            ));
        }
    }

    // ---- columns ----
    for col in &meta.columns {
        let (mut num, mut blank) = (0, 0);
        let mut text: Vec<&str> = Vec::new();
        for k in meta.keys() {
            let v = meta.value(k, col).unwrap_or("").trim();
            if v.is_empty() {
                blank += 1;
            } else if v.parse::<f64>().is_ok() {
                num += 1;
            } else {
                text.push(v);
            }
        }
        let blanks = if blank > 0 { format!(", {blank} blank (sorted last)") } else { String::new() };
        if num > 0 && !text.is_empty() {
            let uniq: HashSet<&str> = text.iter().copied().collect();
            r.warn(format!(
                "Column '{col}': {num} numbers but {} text value(s) such as {} — text sorts \
                 after all numbers; leave cells blank instead{blanks}",
                text.len(),
                examples(uniq, 3)
            ));
        } else if num > 0 {
            r.info(format!("Column '{col}': numeric (sorts by value){blanks}"));
        } else if !text.is_empty() {
            r.info(format!("Column '{col}': text (sorts A→Z, case-insensitive){blanks}"));
        } else {
            r.warn(format!("Column '{col}': every cell is blank"));
        }
    }
    if let Some(col) = meta.columns.first() {
        r.info(format!("Try: winnow {} --sort meta:{col} --sort-desc", root.display()));
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("winnow-check-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        for p in ["a/x.jpg", "a/y.jpg", "b/x.jpg", "b/z.jpg"] {
            fs::write(root.join(p), b"x").unwrap();
        }
        root
    }

    fn has(r: &CheckReport, level: Level, needle: &str) -> bool {
        r.lines.iter().any(|(l, s)| *l == level && s.contains(needle))
    }

    #[test]
    fn good_file_passes_with_useful_notes() {
        let root = tmp_root("good");
        let abs = root.canonicalize().unwrap().join("a/y.jpg");
        fs::write(
            root.join("metadata.csv"),
            format!("path,conf,label\na/x.jpg,0.9,crack\n{},0.2,\nb/x.jpg,0.5,ok\n", abs.display()),
        )
        .unwrap();
        let r = check(&root, true, None, None);
        assert!(!r.has_errors(), "{r}");
        assert!(has(&r, Level::Info, "Images with a row: 3 of 4"), "{r}");
        assert!(has(&r, Level::Warn, "'b/z.jpg'"), "{r}");
        assert!(has(&r, Level::Info, "Column 'conf': numeric"), "{r}");
        assert!(has(&r, Level::Info, "Column 'label': text"), "{r}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn wrong_relative_base_is_an_error() {
        let root = tmp_root("base");
        fs::write(root.join("metadata.csv"), "path,conf\ndata/a/q.jpg,1\n").unwrap();
        let r = check(&root, true, None, None);
        assert!(has(&r, Level::Error, "No image matched any row"), "{r}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn flags_ambiguous_names_mixed_columns_orphans_and_missing_path_column() {
        let root = tmp_root("warn");
        fs::create_dir_all(root.join("_rejected/a")).unwrap();
        fs::rename(root.join("a/y.jpg"), root.join("_rejected/a/y.jpg")).unwrap();
        fs::write(root.join("metadata.csv"), "img_id,score\nx.jpg,1\nz.jpg,n/a\na/y.jpg,2\ngone.jpg,3\n")
            .unwrap();
        let r = check(&root, true, None, None);
        assert!(has(&r, Level::Warn, "No path column found"), "{r}");
        assert!(has(&r, Level::Warn, "apply to several images"), "{r}");
        assert!(has(&r, Level::Warn, "'n/a'"), "{r}");
        assert!(has(&r, Level::Info, "1 row(s) belong to images already moved"), "{r}");
        assert!(has(&r, Level::Warn, "'gone.jpg'"), "{r}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn wrong_folder_prefix_is_flagged_even_when_filename_matches() {
        let root = tmp_root("prefix");
        fs::write(root.join("metadata.csv"), "image_path,s\nold/a/y.jpg,1\n").unwrap();
        let r = check(&root, true, None, None);
        assert!(has(&r, Level::Info, "Path column: 'image_path'"), "{r}");
        assert!(has(&r, Level::Warn, "'old/a/y.jpg' for image 'a/y.jpg'"), "{r}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_and_unsupported_files_are_errors() {
        let root = tmp_root("none");
        assert!(has(&check(&root, true, None, None), Level::Error, "No metadata file"));
        fs::write(root.join("m.json"), "[]").unwrap();
        let r = check(&root, true, Some(&root.join("m.json")), None);
        assert!(has(&r, Level::Error, "Unsupported"), "{r}");
        let _ = fs::remove_dir_all(&root);
    }
}
