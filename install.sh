#!/bin/sh
# Install winnow from the latest GitHub Release.
#
#   curl -fsSL https://raw.githubusercontent.com/FelixAbrahamsson/winnow/master/install.sh | sh
#
# Downloads the prebuilt Linux binary, installs it to ~/.local/bin, and
# registers the "Open With -> Winnow" launcher. No Python required.
set -eu

REPO="FelixAbrahamsson/winnow"

os=$(uname -s)
arch=$(uname -m)

if [ "$os" != "Linux" ]; then
    echo "winnow ships prebuilt binaries for Linux only." >&2
    echo "On other systems install from source:  uv tool install git+https://github.com/$REPO" >&2
    exit 1
fi

case "$arch" in
    x86_64 | amd64) arch=x86_64 ;;
    *)
        echo "No prebuilt binary for '$arch'." >&2
        echo "Install from source instead:  uv tool install git+https://github.com/$REPO" >&2
        exit 1
        ;;
esac

asset="winnow-linux-${arch}.tar.gz"
url="https://github.com/${REPO}/releases/latest/download/${asset}"
libdir="$HOME/.local/lib/winnow"
bindir="$HOME/.local/bin"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading ${asset} ..."
curl -fSL "$url" -o "$tmp/$asset"

echo "Installing to ${libdir} ..."
rm -rf "$libdir"
mkdir -p "$libdir" "$bindir"
tar -C "$libdir" -xzf "$tmp/$asset"          # creates $libdir/winnow/
ln -sf "$libdir/winnow/winnow" "$bindir/winnow"

# Register the desktop launcher (best effort).
"$bindir/winnow" --install-desktop >/dev/null 2>&1 || true

echo "Installed:  $bindir/winnow"

# PATH hint.
case ":$PATH:" in
    *":$bindir:"*) ;;
    *) echo "Add this to your shell rc:  export PATH=\"$bindir:\$PATH\"" ;;
esac

# Qt runtime hint.
if command -v ldconfig >/dev/null 2>&1 && ! ldconfig -p | grep -q libxcb-cursor; then
    echo "If winnow fails to start, install the Qt runtime lib:"
    echo "    sudo apt-get install -y libxcb-cursor0"
fi

echo "Run:  winnow /path/to/images   (or right-click a folder -> Open With -> Winnow)"
