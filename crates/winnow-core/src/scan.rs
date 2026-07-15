//! Folder scanning for image files.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Extensions Qt/pixbuf can typically display. Lowercase, no leading dot.
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "png", "bmp", "gif", "tif", "tiff", "webp", "ppm", "pgm", "pbm", "pnm",
    "xbm", "xpm", "ico",
];

pub fn is_image(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

/// Absolute paths of image files under `root`.
///
/// `exclude_dirs` are directory *names* to skip entirely (e.g. the bucket
/// folders like `_rejected`). Hidden dirs (starting with `.`) are always
/// skipped. Results are sorted for a stable order.
pub fn scan_folder(root: &Path, recursive: bool, exclude_dirs: &[String]) -> Vec<PathBuf> {
    if !recursive {
        let mut out: Vec<PathBuf> = match std::fs::read_dir(root) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file() && is_image(p))
                .collect(),
            Err(_) => Vec::new(),
        };
        out.sort();
        return out;
    }

    let mut out = Vec::new();
    let walker = WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            // Prune excluded/hidden directories (but never the root itself).
            if e.file_type().is_dir() && e.depth() > 0 {
                let name = e.file_name().to_string_lossy();
                if name.starts_with('.') || exclude_dirs.iter().any(|d| d.as_str() == name.as_ref())
                {
                    return false;
                }
            }
            true
        });
    for entry in walker.filter_map(|e| e.ok()) {
        let p = entry.path();
        if entry.file_type().is_file() && is_image(p) {
            out.push(p.to_path_buf());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp() -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!("winnow-scan-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn is_image_checks_extension_case_insensitively() {
        assert!(is_image(Path::new("a/b.JPG")));
        assert!(is_image(Path::new("x.png")));
        assert!(!is_image(Path::new("notes.txt")));
        assert!(!is_image(Path::new("no_ext")));
    }

    #[test]
    fn scan_recurses_and_excludes() {
        let root = tmp();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::create_dir_all(root.join("_rejected")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join("a.jpg"), b"x").unwrap();
        fs::write(root.join("sub/b.png"), b"x").unwrap();
        fs::write(root.join("notes.txt"), b"x").unwrap();
        fs::write(root.join("_rejected/c.jpg"), b"x").unwrap();
        fs::write(root.join(".hidden/d.jpg"), b"x").unwrap();

        let got = scan_folder(&root, true, &["_rejected".to_string()]);
        let names: Vec<String> = got
            .iter()
            .map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.jpg", "sub/b.png"]);

        let flat = scan_folder(&root, false, &[]);
        let flat_names: Vec<String> =
            flat.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(flat_names, vec!["a.jpg"]);

        let _ = fs::remove_dir_all(&root);
    }
}
