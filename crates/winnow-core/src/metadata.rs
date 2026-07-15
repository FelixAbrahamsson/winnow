//! Optional per-image metadata (CSV) keyed by image path.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::Path;

/// Candidate column names that hold the image path (relative to root).
const PATH_COLUMNS: &[&str] = &["path", "filepath", "file", "filename", "image", "img", "name"];

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
        let rel = relpath.replace('\\', "/");
        if let Some(row) = self.by_relpath.get(&rel) {
            return Some(row);
        }
        let base = basename(&rel);
        if let Some(row) = self.by_relpath.get(base) {
            return Some(row);
        }
        self.by_basename.get(base).and_then(|r| self.by_relpath.get(r))
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

    fn pick_path_column(headers: &[String]) -> usize {
        for cand in PATH_COLUMNS {
            if let Some(i) = headers.iter().position(|h| h.eq_ignore_ascii_case(cand)) {
                return i;
            }
        }
        0
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
        let key_col = Self::pick_path_column(&headers);
        let columns: Vec<String> =
            headers.iter().enumerate().filter(|(i, _)| *i != key_col).map(|(_, h)| h.clone()).collect();

        let mut by_relpath: HashMap<String, HashMap<String, String>> = HashMap::new();
        for record in rdr.records() {
            let rec = record?;
            let key = rec.get(key_col).unwrap_or("").trim().replace('\\', "/");
            if key.is_empty() {
                continue;
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

        let mut meta = Metadata { columns, by_relpath, by_basename: HashMap::new() };
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
}
