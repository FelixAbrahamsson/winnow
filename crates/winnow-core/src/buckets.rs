//! Bucket configuration: where images get moved and by which hotkey.
//!
//! Zero config == a single built-in "reject" bucket bound to Delete. An
//! optional `.winnow.toml` in the scan root adds category buckets.

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

/// Ordered bucket list. Reject is always first. If no config file exists at
/// `config_path` (or `root/.winnow.toml`), returns just the default reject.
pub fn load_buckets(root: &Path, config_path: Option<&Path>) -> Result<Vec<Bucket>, BucketError> {
    let owned = config_path.map(Path::to_path_buf).unwrap_or_else(|| root.join(CONFIG_NAME));
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
        if seen.contains(&key_l) {
            return Err(BucketError::DuplicateKey(b.key));
        }
        seen.push(key_l);
        let folder = b.folder.unwrap_or_else(|| format!("_{}", b.name));
        buckets.push(Bucket { name: b.name, key: b.key, folder, is_reject: false });
    }
    Ok(buckets)
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
