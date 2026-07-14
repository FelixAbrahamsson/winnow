#!/usr/bin/env bash
#
# Install a "Winnow" launcher so you can right-click a folder or image in your
# file manager and choose Open With -> Winnow. Also adds it to the app menu.
#
# Re-run this after moving the project folder (the launcher stores an absolute
# path to the venv's `winnow` binary).
#
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$HERE/.venv/bin/winnow"

if [[ ! -x "$BIN" ]]; then
    echo "error: $BIN not found." >&2
    echo "Create the venv and install first:  uv venv && uv pip install -e ." >&2
    exit 1
fi

APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor"
mkdir -p "$APPS"

# Install icons into the hicolor theme so menus and dialogs pick them up.
for sz in 48 64 128 256; do
    if [[ "$sz" == 256 ]]; then
        src="$HERE/packaging/winnow.png"
    else
        src="$HERE/packaging/winnow-$sz.png"
    fi
    [[ -f "$src" ]] || continue
    dest="$ICONS/${sz}x${sz}/apps"
    mkdir -p "$dest"
    cp -f "$src" "$dest/winnow.png"
done

DESKTOP="$APPS/winnow.desktop"
cat > "$DESKTOP" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=Winnow
GenericName=Image Culling Tool
Comment=Fast keyboard-driven image culling / selection
Exec=$BIN %f
Icon=winnow
Terminal=false
Categories=Graphics;Viewer;RasterGraphics;
MimeType=inode/directory;image/jpeg;image/png;image/bmp;image/gif;image/tiff;image/webp;
EOF
chmod +x "$DESKTOP"

update-desktop-database "$APPS" 2>/dev/null || true
gtk-update-icon-cache -f "$ICONS" 2>/dev/null || true

echo "Installed launcher:  $DESKTOP"
echo "Right-click a folder or image -> Open With -> Winnow."
echo "If it doesn't show up right away, restart the file manager:  nautilus -q"
