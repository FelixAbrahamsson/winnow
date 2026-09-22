//! Optional per-image metadata (CSV) keyed by image path.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::Path;

/// Candidate column names that hold the image path (relative to root).
pub const PATH_COLUMNS: &[&str] = &[
    "path", "filepath", "file_path", "image_path", "img_path", "file", "filename", "file_name",
    "image", "img", "name",
];

/// Metadata file names picked up automatically from the folder root.
pub const AUTO_METADATA: &[&str] = &["metadata.csv", "metadata.tsv"];

/// The auto-detected metadata file in `root`, if any.
pub fn find_metadata(root: &Path) -> Option<std::path::PathBuf> {
    AUTO_METADATA.iter().map(|n| root.join(n)).find(|p| p.is_file())
}

/// A sortable value: numbers sort before text, missing values sort last.
#[derive(Debug, Clone, PartialEq)]
pub enum SortKey {
    Num(f64),
    Text(String),
    Missing,
}

impl Eq for SortKey {}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> Ordering {
        fn rank(k: &SortKey) -> u8 {
            match k {
                SortKey::Num(_) => 0,
                SortKey::Text(_) => 1,
                SortKey::Missing => 2,
            }
        }
        match (self, other) {
            (SortKey::Num(a), SortKey::Num(b)) => a.total_cmp(b),
            (SortKey::Text(a), SortKey::Text(b)) => a.cmp(b),
            _ => rank(self).cmp(&rank(other)),
        }
    }
}

#[derive(Default)]
pub struct Metadata {
    /// Display order of columns, excluding the path key column.
    pub columns: Vec<String>,
    /// Header of the column used as the image path.
    pub key_column: String,
    /// False when no known path column name was found and the first column
    /// was used as a fallback.
    pub key_column_recognized: bool,
    /// Path keys that appeared on more than one row (the last row wins).
    pub duplicate_keys: Vec<String>,
    by_relpath: HashMap<String, HashMap<String, String>>,
    by_basename: HashMap<String, String>,
}

fn basename(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

impl Metadata {
    pub fn is_empty(&self) -> bool {
        self.by_relpath.is_empty()
    }

    /// Row for `relpath`, falling back to a unique basename match.
    pub fn get(&self, relpath: &str) -> Option<&HashMap<String, String>> {
        self.row_key(relpath).and_then(|(k, _)| self.by_relpath.get(k))
    }

    /// The row key an image resolves to, and whether it matched by full
    /// relative path (true) or only by filename (false).
    pub fn row_key(&self, relpath: &str) -> Option<(&str, bool)> {
        let rel = relpath.replace('\\', "/");
        if let Some((k, _)) = self.by_relpath.get_key_value(&rel) {
            return Some((k.as_str(), true));
        }
        let base = basename(&rel);
        if let Some((k, _)) = self.by_relpath.get_key_value(base) {
            return Some((k.as_str(), false));
        }
        self.by_basename.get(base).map(|r| (r.as_str(), false))
    }

    /// Row keys in the file.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.by_relpath.keys().map(|k| k.as_str())
    }

    /// Raw cell value.
    pub fn value(&self, key: &str, column: &str) -> Option<&str> {
        self.by_relpath.get(key).and_then(|r| r.get(column)).map(|s| s.as_str())
    }

    /// Make keys relative to `root`: absolute paths under root lose the root
    /// prefix (agents often write absolute paths).
    pub fn relativize(&mut self, root: &Path) {
        let root = root.to_string_lossy().replace('\\', "/");
        let prefix = format!("{}/", root.trim_end_matches('/'));
        let rows = std::mem::take(&mut self.by_relpath);
        for (k, v) in rows {
            let k = k.strip_prefix(&prefix).map(str::to_string).unwrap_or(k);
            self.by_relpath.insert(k, v);
        }
        self.by_basename.clear();
        self.build_basename_index();
    }

    pub fn sort_value(&self, relpath: &str, column: &str) -> SortKey {
        let raw = self.get(relpath).and_then(|row| row.get(column)).map(|s| s.as_str()).unwrap_or("");
        if raw.is_empty() {
            return SortKey::Missing;
        }
        match raw.trim().parse::<f64>() {
            Ok(n) => SortKey::Num(n),
            Err(_) => SortKey::Text(raw.to_ascii_lowercase()),
        }
    }

    fn pick_path_column(headers: &[String]) -> (usize, bool) {
        for cand in PATH_COLUMNS {
            if let Some(i) = headers.iter().position(|h| h.eq_ignore_ascii_case(cand)) {
                return (i, true);
            }
        }
        (0, false)
    }

    fn build_basename_index(&mut self) {
        let mut seen_multiple: std::collections::HashSet<String> = Default::default();
        for rel in self.by_relpath.keys() {
            let base = basename(rel).to_string();
            if self.by_basename.contains_key(&base) || seen_multiple.contains(&base) {
                self.by_basename.remove(&base);
                seen_multiple.insert(base);
            } else {
                self.by_basename.insert(base, rel.clone());
            }
        }
    }

    /// Load a CSV (or TSV) metadata file.
    pub fn load_csv(path: &Path) -> Result<Metadata, csv::Error> {
        let delimiter = match path.extension().and_then(|e| e.to_str()) {
            Some("tsv") => b'\t',
            _ => b',',
        };
        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(true)
            .flexible(true)
            .from_path(path)?;

        let headers: Vec<String> = rdr.headers()?.iter().map(|h| h.trim().to_string()).collect();
        if headers.is_empty() {
            return Ok(Metadata::default());
        }
        let (key_col, key_column_recognized) = Self::pick_path_column(&headers);
        let columns: Vec<String> =
            headers.iter().enumerate().filter(|(i, _)| *i != key_col).map(|(_, h)| h.clone()).collect();

        let mut by_relpath: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut duplicate_keys = Vec::new();
        for record in rdr.records() {
            let rec = record?;
            let key = rec.get(key_col).unwrap_or("").trim().replace('\\', "/");
            let key = key.strip_prefix("./").map(str::to_string).unwrap_or(key);
            if key.is_empty() {
                continue;
            }
            if by_relpath.contains_key(&key) {
                duplicate_keys.push(key.clone());
            }
            let mut row = HashMap::new();
            for (i, h) in headers.iter().enumerate() {
                if i == key_col {
                    continue;
                }
                row.insert(h.clone(), rec.get(i).unwrap_or("").to_string());
            }
            by_relpath.insert(key, row);
        }

        let mut meta = Metadata {
            columns,
            key_column: headers[key_col].clone(),
            key_column_recognized,
            duplicate_keys,
            by_relpath,
            by_basename: HashMap::new(),
        };
        meta.build_basename_index();
        Ok(meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn sortkey_orders_num_then_text_then_missing() {
        let mut v = vec![
            SortKey::Missing,
            SortKey::Text("b".into()),
            SortKey::Num(5.0),
            SortKey::Num(2.0),
            SortKey::Text("a".into()),
        ];
        v.sort();
        assert_eq!(
            v,
            vec![
                SortKey::Num(2.0),
                SortKey::Num(5.0),
                SortKey::Text("a".into()),
                SortKey::Text("b".into()),
                SortKey::Missing,
            ]
        );
    }

    #[test]
    fn loads_csv_and_resolves_paths() {
        let mut p = std::env::temp_dir();
        p.push(format!("winnow-meta-{}.csv", std::process::id()));
        fs::write(
            &p,
            "path,severity,note\nline12/a.jpg,3,hairline\nline12/b.jpg,,\n",
        )
        .unwrap();

        let m = Metadata::load_csv(&p).unwrap();
        assert_eq!(m.columns, vec!["severity", "note"]);
        assert_eq!(m.get("line12/a.jpg").unwrap().get("severity").unwrap(), "3");
        // basename fallback
        assert!(m.get("a.jpg").is_some());
        assert_eq!(m.sort_value("line12/a.jpg", "severity"), SortKey::Num(3.0));
        assert_eq!(m.sort_value("line12/b.jpg", "severity"), SortKey::Missing);
        assert_eq!(m.sort_value("line12/a.jpg", "note"), SortKey::Text("hairline".into()));

        let _ = fs::remove_file(&p);
    }

    #[test]
    fn absolute_and_dot_paths_are_made_relative() {
        let p = std::env::temp_dir().join(format!("winnow-meta-abs-{}.csv", std::process::id()));
        fs::write(&p, "file,score\n/data/set/sub/a.jpg,1\n./b.jpg,2\nc.jpg,3\nc.jpg,4\n").unwrap();
        let mut m = Metadata::load_csv(&p).unwrap();
        assert_eq!(m.key_column, "file");
        assert!(m.key_column_recognized);
        assert_eq!(m.duplicate_keys, vec!["c.jpg"]);
        m.relativize(Path::new("/data/set/"));
        assert_eq!(m.row_key("sub/a.jpg"), Some(("sub/a.jpg", true)));
        assert_eq!(m.row_key("b.jpg"), Some(("b.jpg", true)));
        assert_eq!(m.row_key("x/c.jpg"), Some(("c.jpg", false)));
        assert_eq!(m.value("c.jpg", "score"), Some("4"));
        let _ = fs::remove_file(&p);
    }
}
