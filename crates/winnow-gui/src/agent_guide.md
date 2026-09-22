# Preparing images for review in winnow

winnow is a keyboard-driven image viewer for culling and sorting images into
folders ("buckets"). You prepare a folder; the user reviews it in the app.
Your job is usually one or both of:

1. **Write a metadata file** so the user can sort/rank images by your
   criteria (model confidence, damage score, …) and see per-image values.
2. **Set up buckets** (classes the user sorts images into).

After writing either, **always run `winnow --check FOLDER`** and fix every
ERROR and WARN it reports (the check reads nothing but the folder, and exits
non-zero on errors).

## Metadata file

Put a CSV named `metadata.csv` in the folder the user will open (the review
root). It is picked up automatically. (`metadata.tsv`, tab-separated, also
works; elsewhere, open with `--metadata FILE`.) JSON / Parquet are NOT read.

- One row per image, with a header row.
- One column holds the image path. Name it `path` (also recognised:
  filepath, file_path, image_path, img_path, file, filename, file_name,
  image, img, name). It should be **relative to the review
  root**, using `/`, e.g. `line12/img_0007.jpg` (subfolders are scanned
  recursively). Absolute paths under the root are accepted too. A bare
  filename also matches, but applies to every image with that name in any
  subfolder, so prefer full relative paths.
- Every other column is shown in the info panel (in file order) and is
  sortable.
- A column whose non-blank cells all parse as numbers sorts numerically;
  otherwise it sorts as text (A→Z, case-insensitive). Numbers always sort
  before text, and blank cells / images without a row sort last. So for a
  numeric column, leave unknown values **blank** — don't write `n/a` or `-`.
- A cell that is a URL is shown as a clickable link.
- Images without a row are still shown (with no values). Rows for images that
  don't exist are ignored.
- Folders used as buckets (`_rejected/`, `_<class>/`) are not scanned; their
  images are already sorted.

Tips:

- Put the ordering you want reviewed first in a column, e.g. `priority`
  (1 = look first) or a score to sort descending. Several criteria → several
  columns; the user switches between them in the app's Sort dropdown.
- Keep values short; long text wraps in the side panel.
- Quote cells containing commas (standard CSV; Python's `csv` / pandas
  `to_csv(index=False)` do this).

Example:

```csv
path,priority,confidence,damage_area,pred_class,source
line12/img_0001.jpg,1,0.31,0.042,crack,https://example.com/runs/88
line12/img_0002.jpg,2,0.47,,spall,
line13/img_0107.jpg,3,0.52,0.310,crack,
```

Open it sorted (for the user, or to tell them the command):

```sh
winnow FOLDER --sort meta:priority            # ascending
winnow FOLDER --sort meta:confidence          # lowest confidence first
winnow FOLDER --sort meta:damage_area --sort-desc
```

Built-in sort keys: `name`, `path`, `mtime`, `size`, `meta:COLUMN`.

## Buckets (classes)

Pressing a bucket's hotkey (or clicking its chip) **moves** the image into the
bucket's folder under the review root, keeping its subfolder path
(`line12/img_0001.jpg` → `_crack/line12/img_0001.jpg`). Undoable in the app.
`Delete` → `_rejected/` always exists.

Easiest setup: create empty folders named `_<class>` in the review root (e.g.
`_crack/`, `_spall/`); with no config file, winnow turns each into a bucket
with digit hotkeys 1, 2, … in name order. For explicit hotkeys, write
`.winnow.toml` in the review root instead (once it exists, folder discovery
is off):

```toml
[[bucket]]
name = "crack"
key = "1"          # GDK key name; digits 1-9 recommended; "" = no hotkey
folder = "_crack"  # relative to the review root

[[bucket]]
name = "spall"
key = "2"
folder = "_spall"
```

Hotkeys must be unique. The user can also add / rename / remove buckets in the
app (it rewrites `.winnow.toml`).

## Results

After review, images the user sorted are in the bucket folders; what's left
in place was kept (or not yet reviewed). The metadata file is never modified.
