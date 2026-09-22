//! Bucket configuration: where images get moved and by which hotkey.
//!
//! Zero config == a single built-in "reject" bucket bound to Delete, plus any
//! `_name/` folders already in the scan root (see [`discover_buckets`]). An
//! optional `.winnow.toml` in the scan root adds category buckets; once it
//! exists it is authoritative and discovery is skipped.

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const CONFIG_NAME: &str = ".winnow.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    pub name: String,
    /// GDK key name, e.g. "Delete", "1", "c".
    pub key: String,
    /// Folder relative to root (or absolute).
    pub folder: String,
    pub is_reject: bool,
}

impl Bucket {
    pub fn target_dir(&self, root: &Path) -> PathBuf {
        let p = Path::new(&self.folder);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        }
    }
}

pub fn default_reject() -> Bucket {
    Bucket {
        name: "reject".into(),
        key: "Delete".into(),
        folder: "_rejected".into(),
        is_reject: true,
    }
}

#[derive(Deserialize, Default)]
struct RawConfig {
    reject: Option<RawReject>,
    #[serde(default)]
    bucket: Vec<RawBucket>,
}

#[derive(Deserialize)]
struct RawReject {
    name: Option<String>,
    key: Option<String>,
    folder: Option<String>,
}

#[derive(Deserialize)]
struct RawBucket {
    name: String,
    key: String,
    folder: Option<String>,
}

#[derive(Debug)]
pub enum BucketError {
    Toml(String),
    DuplicateKey(String),
}

impl std::fmt::Display for BucketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BucketError::Toml(e) => write!(f, "invalid {CONFIG_NAME}: {e}"),
            BucketError::DuplicateKey(k) => write!(f, "bucket reuses hotkey '{k}'"),
        }
    }
}

impl std::error::Error for BucketError {}

/// Where the bucket config lives: the explicit path, else `root/.winnow.toml`.
pub fn config_file(root: &Path, config_path: Option<&Path>) -> PathBuf {
    config_path.map(Path::to_path_buf).unwrap_or_else(|| root.join(CONFIG_NAME))
}

/// Ordered bucket list. Reject is always first. If no config file exists at
/// `config_path` (or `root/.winnow.toml`), returns just the default reject.
pub fn load_buckets(root: &Path, config_path: Option<&Path>) -> Result<Vec<Bucket>, BucketError> {
    let owned = config_file(root, config_path);
    if !owned.exists() {
        return Ok(vec![default_reject()]);
    }
    let text = std::fs::read_to_string(&owned).map_err(|e| BucketError::Toml(e.to_string()))?;
    let raw: RawConfig = toml::from_str(&text).map_err(|e| BucketError::Toml(e.to_string()))?;

    let reject = match raw.reject {
        Some(r) => Bucket {
            name: r.name.unwrap_or_else(|| "reject".into()),
            key: r.key.unwrap_or_else(|| "Delete".into()),
            folder: r.folder.unwrap_or_else(|| "_rejected".into()),
            is_reject: true,
        },
        None => default_reject(),
    };

    let mut buckets = vec![reject];
    let mut seen: Vec<String> = vec![buckets[0].key.to_ascii_lowercase()];
    for b in raw.bucket {
        let key_l = b.key.to_ascii_lowercase();
        // An empty key means "no hotkey" (click-only); those may repeat.
        if !key_l.is_empty() && seen.contains(&key_l) {
            return Err(BucketError::DuplicateKey(b.key));
        }
        seen.push(key_l);
        let folder = b.folder.unwrap_or_else(|| format!("_{}", b.name));
        buckets.push(Bucket { name: b.name, key: b.key, folder, is_reject: false });
    }
    Ok(buckets)
}

/// First unused digit hotkey 1–9, or "" (no hotkey) when all are taken.
pub fn next_free_key(buckets: &[Bucket]) -> String {
    (1..=9)
        .map(|d| d.to_string())
        .find(|k| !buckets.iter().any(|b| b.key.eq_ignore_ascii_case(k)))
        .unwrap_or_default()
}

/// Append a bucket for every `_name/` folder directly under root that isn't
/// already a bucket, in name order, with the next free digit hotkeys. Lets a
/// sorting session resume from the folders alone.
pub fn discover_buckets(root: &Path, buckets: &mut Vec<Bucket>) {
    let mut names: Vec<String> = match std::fs::read_dir(root) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.len() > 1 && n.starts_with('_'))
            .collect(),
        Err(_) => return,
    };
    names.sort();
    for folder in names {
        if buckets.iter().any(|b| b.folder == folder) {
            continue;
        }
        let key = next_free_key(buckets);
        buckets.push(Bucket { name: folder[1..].to_string(), key, folder, is_reject: false });
    }
}

/// Write the bucket list as a `.winnow.toml` (comments in an existing file
/// are not preserved).
pub fn save_buckets(path: &Path, buckets: &[Bucket]) -> std::io::Result<()> {
    let q = |s: &str| toml::Value::String(s.to_string()).to_string();
    let mut out = String::from("# Buckets for winnow. Edited by the app; hand edits are fine too.\n");
    for b in buckets {
        out.push_str(if b.is_reject { "\n[reject]\n" } else { "\n[[bucket]]\n" });
        out.push_str(&format!("name = {}\nkey = {}\nfolder = {}\n", q(&b.name), q(&b.key), q(&b.folder)));
    }
    std::fs::write(path, out)
}

/// Names of bucket folders sitting directly under root, to exclude from scanning.
pub fn bucket_folder_names(buckets: &[Bucket], root: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let root_c = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    for b in buckets {
        let target = b.target_dir(root);
        let parent_is_root = target
            .canonicalize()
            .ok()
            .and_then(|t| t.parent().map(|p| p.to_path_buf()))
            .map(|p| p == root_c)
            .unwrap_or(true);
        if parent_is_root {
            if let Some(name) = Path::new(&b.folder).file_name() {
                names.push(name.to_string_lossy().into_owned());
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("winnow-bk-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn discovers_underscore_folders_with_free_keys() {
        let root = tmp_root("disc");
        for d in ["_rejected", "_spall", "_crack", "plain", "_"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        let mut b = vec![default_reject()];
        discover_buckets(&root, &mut b);
        let got: Vec<(&str, &str)> = b.iter().map(|b| (b.name.as_str(), b.key.as_str())).collect();
        assert_eq!(got, vec![("reject", "Delete"), ("crack", "1"), ("spall", "2")]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn next_free_key_skips_taken_and_runs_out() {
        let mut b = vec![default_reject()];
        for k in ["1", "3"] {
            b.push(Bucket { name: k.into(), key: k.into(), folder: format!("_{k}"), is_reject: false });
        }
        assert_eq!(next_free_key(&b), "2");
        for k in 1..=9 {
            b.push(Bucket { name: format!("x{k}"), key: k.to_string(), folder: String::new(), is_reject: false });
        }
        assert_eq!(next_free_key(&b), "");
    }

    #[test]
    fn save_then_load_roundtrip() {
        let root = tmp_root("save");
        let cfg = root.join(CONFIG_NAME);
        let b = vec![
            default_reject(),
            Bucket { name: "cr\"ack".into(), key: "1".into(), folder: "_crack".into(), is_reject: false },
            Bucket { name: "a".into(), key: "".into(), folder: "_a".into(), is_reject: false },
            Bucket { name: "b".into(), key: "".into(), folder: "_b".into(), is_reject: false },
        ];
        save_buckets(&cfg, &b).unwrap();
        assert_eq!(load_buckets(&root, None).unwrap(), b);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn no_config_gives_reject_only() {
        let root = std::env::temp_dir();
        let buckets = load_buckets(&root, Some(Path::new("/nonexistent/.winnow.toml"))).unwrap();
        assert_eq!(buckets.len(), 1);
        assert!(buckets[0].is_reject);
        assert_eq!(buckets[0].key, "Delete");
    }

    #[test]
    fn parses_config_with_buckets() {
        let mut p = std::env::temp_dir();
        p.push(format!("winnow-buckets-{}.toml", std::process::id()));
        fs::write(
            &p,
            "[[bucket]]\nname=\"crack\"\nkey=\"1\"\nfolder=\"_crack\"\n\n[[bucket]]\nname=\"spall\"\nkey=\"2\"\n",
        )
        .unwrap();
        let buckets = load_buckets(Path::new("/tmp"), Some(&p)).unwrap();
        assert_eq!(buckets.len(), 3);
        assert_eq!(buckets[1].name, "crack");
        assert_eq!(buckets[1].key, "1");
        assert_eq!(buckets[2].folder, "_spall"); // defaulted from name
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn duplicate_key_errors() {
        let mut p = std::env::temp_dir();
        p.push(format!("winnow-dup-{}.toml", std::process::id()));
        fs::write(&p, "[[bucket]]\nname=\"a\"\nkey=\"Delete\"\nfolder=\"_a\"\n").unwrap();
        assert!(load_buckets(Path::new("/tmp"), Some(&p)).is_err());
        let _ = fs::remove_file(&p);
    }
}
