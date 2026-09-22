# winnow

A fast, keyboard-driven image culling / selection tool for ML data curation —
built for stepping through folders of images, inspecting them closely, and
sorting them into keep / reject (and optional category) buckets.

Native Linux app (Rust / GTK4). Purpose-built to replace "open the GNOME
image viewer and mash Delete".

> **Note:** winnow was rewritten from Python/PySide6 to Rust/GTK4 for a tiny
> (~1 MB vs ~90 MB) native binary. It's a Cargo workspace: `winnow-core`
> (pure logic + tests) and `winnow-gui` (the GTK front-end). The original
> Python implementation is preserved in the `v0.1.1` git tag.

## Why this instead of the built-in viewer

- **Delete is reversible.** Pressing `Delete` doesn't destroy anything — it
  *moves* the image into a `_rejected/` folder (preserving subfolder structure).
  `Ctrl+Z` moves it back. Everything is recoverable.
- **Buckets.** Reject is just the built-in bucket. Add your own categories
  (e.g. crack / corrosion / spall) with one hotkey each — see below.
- **Two views.** A one-by-one zoom/pan viewer for close inspection, and a
  contact-sheet grid for fast bulk culling. Toggle with `G`.
- **Metadata.** Point it at a `metadata.csv` and see per-image columns in the
  side panel — and sort the whole set on any column.
- **Scales to thousands** of images: the grid is virtualized (only visible
  thumbnails are decoded) and thumbnails are cached in memory.

## Install

**Prebuilt binary (~630 KB download):**

```bash
curl -fsSL https://raw.githubusercontent.com/FelixAbrahamsson/winnow/master/install.sh | sh
```

Downloads the latest Linux release into `~/.local/bin/winnow` and registers the
"Open With → Winnow" launcher. The binary is dynamically linked against the
system **GTK4 runtime**, which is present on GNOME/KDE and otherwise a one-line
install:

```bash
sudo apt-get install -y libgtk-4-1   # Debian/Ubuntu/Pop!_OS  (Fedora: gtk4, Arch: gtk4)
```

**From source (needs the [Rust toolchain](https://rustup.rs) + GTK4 dev headers):**

```bash
sudo apt-get install -y libgtk-4-dev
cargo install --git https://github.com/FelixAbrahamsson/winnow winnow-gui
winnow --install-desktop      # optional: register the Open With launcher
```

### Development

```bash
sudo apt-get install -y libgtk-4-dev
cargo test -p winnow-core          # pure-logic unit tests
cargo run -p winnow-gui -- /path/to/images
```

## Run

```bash
winnow /path/to/images                 # recurse into subfolders by default
winnow /path/to/images --no-recursive  # top-level folder only
winnow /path/to/images --metadata metadata.csv
winnow /path/to/images --sort meta:severity --sort-desc
winnow /path/to/images/img_0007.jpg    # open a single image (starts on it)
```

(In a dev checkout, prefix with `cargo run -p winnow-gui --`.)

## Right-click "Open With" integration

The `install.sh` binary installer registers this automatically. Otherwise run:

```bash
winnow --install-desktop
```

Then right-click a folder **or** an image in your file manager and pick
*Open With → Winnow* (it also appears in the app menu). Opening a folder loads
all its images; opening a single image loads its folder and starts on that
image. Re-run it if you move the install (the launcher stores an absolute
path). To remove it, delete `~/.local/share/applications/winnow.desktop`.

## Moving or renaming the install

The desktop launcher stores an **absolute path** to the `winnow` binary, so if
you move it, re-run `winnow --install-desktop` to fix the launcher.

## Releasing (maintainer)

Push a version tag; a GitHub Action builds the Linux binary and attaches it to
a Release, which `install.sh` then downloads:

```bash
git tag v0.2.0 && git push origin v0.2.0
```

To build the binary locally: `cargo build --release -p winnow-gui`
(output at `target/release/winnow`, ~1 MB).

## Keybindings

Press `?` (or `F1`) in the app, or right-click for a context menu, to see this
list at any time.

| Key | Action |
| --- | --- |
| `→` / `Space` | Next image |
| `←` | Previous image |
| mouse Back / Forward buttons | Previous / next image |
| `PgDn` / `PgUp` | Jump ±10 |
| `Home` / `End` | First / last |
| `Delete` / `Backspace` / `X` | Reject (move to `_rejected/`) |
| *bucket keys* / click a chip | Move to that bucket |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / redo the last move |
| scroll wheel / pinch | Zoom in / out (toward cursor) |
| `+` / `-` | Zoom in / out |
| `F` / `A` | Fit to window / 100% actual pixels |
| left-drag **when zoomed in** | Pan the image (open-hand cursor) |
| middle-drag | Pan the image |
| double-click | Toggle fullscreen |
| left-drag **when fit** | Drag the file out to another app (copy) |
| `Ctrl`+left-drag | Drag the file out (works even when zoomed) |
| `Ctrl+Shift+X` | Copy image file — paste into a file manager |
| `[` / `]` / `\` | Brightness down / up / reset |
| `{` / `}` | Gamma down / up |
| `B` | Toggle auto brightness (match the previous image's look) |
| `Ctrl+C` / `Ctrl+Shift+C` | Copy filename / full path |
| `G` | Toggle grid ↔ single view |
| `I` | Toggle info panel |
| `F11` | Fullscreen |
| `Ctrl+O` | Open another folder |
| `?` / `F1` | Show the shortcuts list |
| right-click | Context menu of common actions |

In grid view, select multiple thumbnails (click / `Ctrl`+click / `Shift`+click)
and press a bucket key to move them all at once.

Changing the sort key jumps you to the first image of the new order.

**Auto brightness** (`B`, or the checkbox in the ☀ Brightness popover): when
on, each new image gets the brightness that makes it look as bright (same mean
displayed luminance) as the previous image did — handy when exposure swings
between captures. Tune the brightness once, then step through; gamma is kept.

## Buckets (sorting into classes)

Each bucket is a folder + a hotkey. Pressing the key (or clicking the bucket's
chip in the bar under the image) moves the current image — or, in grid view,
all selected images — into that folder, preserving subfolder structure, fully
undoable. Out of the box there is just **reject** (`Delete` → `_rejected/`).

**The bucket bar** shows every bucket as a chip: hotkey, name, and how many
images are in it. The chip flashes when you press its key.

- **Add** a bucket with the `+` chip: type a name and it gets folder `_name/`
  and the next free digit key `1`–`9`.
- **Rename / remove** a bucket by right-clicking its chip. Renaming also
  renames its folder. Removing only forgets the bucket — its folder and images
  stay put (and rejoin the queue the next time you open the folder).
- **Resuming:** with no config file, existing `_name/` folders in the image
  folder are picked up as buckets automatically, so a half-done sort carries on
  where it left off; images already in them are left out of the queue.

Bucket edits are saved to `.winnow.toml` in the image folder (or the file given
with `--buckets FILE`). Once that file exists it is the source of truth — no
more auto-discovery — and you can edit it by hand too (comments aren't kept
when winnow rewrites it):

```toml
[reject]
name = "reject"
key = "Delete"
folder = "_rejected"

[[bucket]]
name = "crack"
key = "1"
folder = "_crack"

[[bucket]]
name = "spall"
key = "2"
folder = "_spall"
```

An empty `key = ""` means the bucket has no hotkey (click its chip instead).

## Metadata format

If a `metadata.csv` (or `.tsv`/`.json`/`.jsonl`/`.parquet`) sits in the image
folder, it is loaded automatically — no `--metadata` flag needed. Use
`--metadata FILE` only to point at one elsewhere.

A `metadata.csv` with one row per image is the recommended format. It must have
a column identifying the image — `path`, `file`, `filename`, `image`, or `name`
— holding the path **relative to the folder root** (e.g. `sub/img_0007.jpg`), so
it works with recursion. Any other columns become sortable/displayable fields.

```csv
path,severity,camera,notes
line12/img_0001.jpg,3,front,hairline crack
line12/img_0002.jpg,0,front,
```

`.tsv`, `.json`, `.jsonl`, and `.parquet` (needs pandas) are also accepted.

## License

For internal use.
