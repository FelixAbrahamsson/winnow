# winnow

A fast, keyboard-driven image culling / selection tool for ML data curation —
built for stepping through folders of images, inspecting them closely, and
sorting them into keep / reject (and optional category) buckets.

Native Linux app (PySide6 / Qt6). Purpose-built to replace "open the GNOME
image viewer and mash Delete".

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
- **Scales to thousands** of images with lazy thumbnails, an on-disk thumbnail
  cache, and full-image prefetch.

## Install

```bash
uv venv --python 3.10
uv pip install -e .
```

On X11 sessions, Qt 6.5+ needs one system library (a one-time install):

```bash
sudo apt-get install -y libxcb-cursor0
```

## Run

```bash
uv run winnow /path/to/images                 # recurse into subfolders by default
uv run winnow /path/to/images --no-recursive  # top-level folder only
uv run winnow /path/to/images --metadata metadata.csv
uv run winnow /path/to/images --sort meta:severity --sort-desc
```

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
| *bucket keys* | Move to that category (configurable) |
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

## Buckets (optional categories)

With **no config**, you get plain keep/reject. To add categories, drop an
`.winnow.toml` in the image folder (or pass `--buckets FILE`):

```toml
# optional: override the built-in reject bucket
[reject]
folder = "_rejected"
key = "Delete"

[[bucket]]
name = "crack"
key = "1"
folder = "_crack"

[[bucket]]
name = "corrosion"
key = "2"
folder = "_corrosion"

[[bucket]]
name = "spall"
key = "3"
folder = "_spall"
```

Each bucket is a folder + a hotkey. Pressing the key moves the current image
(or, in grid view, all selected images) into that folder, preserving subfolder
structure, fully undoable. Recommended hotkeys: digits `1`–`9`.

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
