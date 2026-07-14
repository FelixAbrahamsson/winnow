#!/bin/sh
# Install winnow from the latest GitHub Release.
#
#   curl -fsSL https://raw.githubusercontent.com/FelixAbrahamsson/winnow/master/install.sh | sh
#
# Downloads the prebuilt Linux binary (~1-2 MB, Rust/GTK4) to ~/.local/bin and
# registers the "Open With -> Winnow" launcher. Needs the system GTK4 runtime.
set -eu

REPO="FelixAbrahamsson/winnow"

os=$(uname -s)
arch=$(uname -m)

if [ "$os" != "Linux" ]; then
    echo "winnow ships prebuilt binaries for Linux only." >&2
    echo "On other systems build from source:  cargo install --git https://github.com/$REPO winnow-gui" >&2
    exit 1
fi

case "$arch" in
    x86_64 | amd64) arch=x86_64 ;;
    *)
        echo "No prebuilt binary for '$arch'." >&2
        echo "Build from source instead:  cargo install --git https://github.com/$REPO winnow-gui" >&2
        exit 1
        ;;
esac

asset="winnow-linux-${arch}.tar.gz"
url="https://github.com/${REPO}/releases/latest/download/${asset}"
bindir="$HOME/.local/bin"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading ${asset} ..."
curl -fSL "$url" -o "$tmp/$asset"
tar -C "$tmp" -xzf "$tmp/$asset"          # extracts the `winnow` binary

mkdir -p "$bindir"
install -m755 "$tmp/winnow" "$bindir/winnow"

# Register the desktop launcher (best effort).
"$bindir/winnow" --install-desktop >/dev/null 2>&1 || true

echo "Installed:  $bindir/winnow"

# PATH hint.
case ":$PATH:" in
    *":$bindir:"*) ;;
    *) echo "Add this to your shell rc:  export PATH=\"$bindir:\$PATH\"" ;;
esac

# GTK4 runtime check (the binary is dynamically linked against system GTK).
if command -v ldconfig >/dev/null 2>&1 && ! ldconfig -p | grep -q 'libgtk-4'; then
    echo "winnow needs the GTK4 runtime. Install it:"
    echo "    Debian/Ubuntu/Pop!_OS:  sudo apt-get install -y libgtk-4-1"
    echo "    Fedora:                 sudo dnf install gtk4"
    echo "    Arch:                   sudo pacman -S gtk4"
fi

echo "Run:  winnow /path/to/images   (or right-click a folder -> Open With -> Winnow)"
