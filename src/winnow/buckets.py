"""Bucket configuration: where images get moved and by which hotkey.

Zero config == a single built-in "reject" bucket bound to Delete, giving the
plain keep/reject workflow. An optional ``.winnow.toml`` in the scan root
(or a path passed via ``--buckets``) adds extra categories.

Example ``.winnow.toml``::

    # optionally override the built-in reject bucket
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
"""

from __future__ import annotations

import sys
from dataclasses import dataclass
from pathlib import Path

if sys.version_info >= (3, 11):
    import tomllib
else:  # pragma: no cover
    import tomli as tomllib

CONFIG_NAME = ".winnow.toml"


@dataclass(frozen=True)
class Bucket:
    name: str
    key: str          # Qt-parseable key string, e.g. "Delete", "1", "c"
    folder: str       # relative to root (or absolute)
    is_reject: bool = False

    def target_dir(self, root: Path) -> Path:
        p = Path(self.folder)
        return p if p.is_absolute() else (root / p)


DEFAULT_REJECT = Bucket(name="reject", key="Delete", folder="_rejected", is_reject=True)


def load_buckets(root: Path, config_path: Path | None = None) -> list[Bucket]:
    """Return the ordered bucket list. Reject is always first."""
    path = config_path or (root / CONFIG_NAME)
    if not path.exists():
        return [DEFAULT_REJECT]

    with path.open("rb") as f:
        data = tomllib.load(f)

    reject = DEFAULT_REJECT
    rj = data.get("reject")
    if isinstance(rj, dict):
        reject = Bucket(
            name=rj.get("name", "reject"),
            key=str(rj.get("key", "Delete")),
            folder=rj.get("folder", "_rejected"),
            is_reject=True,
        )

    buckets = [reject]
    seen_keys = {reject.key.lower()}
    for entry in data.get("bucket", []):
        if not isinstance(entry, dict):
            continue
        name = entry.get("name")
        key = str(entry.get("key", "")).strip()
        folder = entry.get("folder") or (f"_{name}" if name else None)
        if not name or not key or not folder:
            continue
        if key.lower() in seen_keys:
            raise ValueError(f"Bucket '{name}' reuses hotkey '{key}'")
        seen_keys.add(key.lower())
        buckets.append(Bucket(name=name, key=key, folder=folder))
    return buckets


def bucket_folder_names(buckets: list[Bucket], root: Path) -> set[str]:
    """Names of bucket folders that sit directly under root, to exclude from
    scanning so moved images aren't re-discovered."""
    names: set[str] = set()
    for b in buckets:
        target = b.target_dir(root)
        try:
            if target.resolve().parent == root.resolve():
                names.add(target.name)
        except OSError:
            names.add(Path(b.folder).name)
    return names
