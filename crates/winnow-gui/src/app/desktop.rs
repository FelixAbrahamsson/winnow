//! `winnow --install-desktop`: register the "Open With -> Winnow" launcher and
//! icons so folders and images get an entry in the file manager. Icons are
//! embedded in the binary, so this works from any install location.

use std::io;
use std::path::PathBuf;
use std::process::Command;

const ICONS: &[(u32, &[u8])] = &[
    (48, include_bytes!("../../icons/winnow-48.png")),
    (64, include_bytes!("../../icons/winnow-64.png")),
    (128, include_bytes!("../../icons/winnow-128.png")),
    (256, include_bytes!("../../icons/winnow-256.png")),
];

const MIME: &str = "inode/directory;image/jpeg;image/png;image/bmp;image/gif;image/tiff;image/webp;";

fn exe_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "winnow".to_string())
}

pub fn install_desktop() -> io::Result<PathBuf> {
    let home = PathBuf::from(std::env::var("HOME").map_err(|_| {
        io::Error::new(io::ErrorKind::NotFound, "HOME not set")
    })?);
    let apps = home.join(".local/share/applications");
    let icons = home.join(".local/share/icons/hicolor");
    std::fs::create_dir_all(&apps)?;

    for (sz, data) in ICONS {
        let dir = icons.join(format!("{sz}x{sz}")).join("apps");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("winnow.png"), data)?;
    }

    let desktop = apps.join("winnow.desktop");
    let contents = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=Winnow\n\
         GenericName=Image Culling Tool\n\
         Comment=Fast keyboard-driven image culling / selection\n\
         Exec={exe} %f\n\
         Icon=winnow\n\
         Terminal=false\n\
         Categories=Graphics;Viewer;2DGraphics;\n\
         MimeType={mime}\n",
        exe = exe_path(),
        mime = MIME,
    );
    std::fs::write(&desktop, contents)?;

    // Refresh caches (best effort; absent on some desktops).
    let _ = Command::new("update-desktop-database").arg(&apps).status();
    let _ = Command::new("gtk-update-icon-cache").arg("-f").arg(&icons).status();

    Ok(desktop)
}
