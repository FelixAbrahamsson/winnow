"""Command-line entry point: winnow [FOLDER] [options]."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from .app import create_app, launch_window
from .model import Session


# Metadata filenames auto-detected in the image folder, in priority order.
_AUTO_METADATA_NAMES = (
    "metadata.csv", "metadata.tsv", "metadata.json",
    "metadata.jsonl", "metadata.parquet",
)


def _auto_metadata(root: Path) -> Path | None:
    for name in _AUTO_METADATA_NAMES:
        cand = root / name
        if cand.exists():
            return cand
    return None


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="winnow",
        description="Fast keyboard-driven image culling / selection tool.",
    )
    p.add_argument("folder", nargs="?", default=".", help="folder of images (default: current dir)")
    p.add_argument(
        "--no-recursive",
        dest="recursive",
        action="store_false",
        help="do not descend into subfolders (default: recurse)",
    )
    p.add_argument(
        "--metadata",
        metavar="FILE",
        help="metadata file (.csv/.tsv/.json/.parquet); "
        "auto-detected as metadata.csv in the folder if omitted",
    )
    p.add_argument("--buckets", metavar="FILE", help="bucket config TOML (default: .winnow.toml in folder)")
    p.add_argument("--sort", metavar="KEY", help="initial sort key (e.g. name, mtime, size, meta:COLUMN)")
    p.add_argument("--sort-desc", action="store_true", help="sort descending")
    return p


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)

    target = Path(args.folder).expanduser().resolve()
    # Accept either a folder or a single image file (opens its folder, and
    # starts on that image) so "Open with winnow" works for both.
    start_file: Path | None = None
    if target.is_file():
        start_file = target
        root = target.parent
    else:
        root = target
    if not root.is_dir():
        print(f"error: not a directory: {root}", file=sys.stderr)
        return 2

    if args.metadata:
        meta_path = Path(args.metadata).expanduser()
        if not meta_path.exists():
            print(f"error: metadata file not found: {meta_path}", file=sys.stderr)
            return 2
    else:
        meta_path = _auto_metadata(root)
        if meta_path:
            print(f"using metadata: {meta_path.name}", file=sys.stderr)
    buckets_path = Path(args.buckets).expanduser() if args.buckets else None

    app = create_app(sys.argv)
    try:
        session = Session(
            root,
            recursive=args.recursive,
            buckets_config=buckets_path,
            metadata_path=meta_path,
        )
    except Exception as e:  # config / metadata errors
        print(f"error: {e}", file=sys.stderr)
        return 1

    if args.sort:
        session.sort_key = args.sort
        session.sort_reverse = args.sort_desc

    if session.count() == 0:
        print(f"warning: no images found in {root}", file=sys.stderr)

    win = launch_window(session)
    if args.sort:
        session.apply_sort(args.sort, args.sort_desc)
    if start_file is not None:
        for i, item in enumerate(session.items):
            if item.abs_path == start_file:
                session.set_index(i)
                break
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
