"""Optional metadata loading (CSV / Parquet) keyed by image path."""

from __future__ import annotations

import csv
from pathlib import Path

# Candidate column names that hold the image path (relative to root).
PATH_COLUMNS = ("path", "filepath", "file", "filename", "image", "img", "name")


class Metadata:
    """Per-image metadata table.

    Rows are keyed by the image path relative to the scan root. Lookups also
    fall back to basename so a metadata file that only lists ``img_0001.jpg``
    still matches ``sub/img_0001.jpg`` when unambiguous.
    """

    def __init__(self, columns: list[str], by_relpath: dict[str, dict[str, str]]):
        self.columns = columns  # display order, excluding the path key column
        self._by_relpath = by_relpath
        # basename -> relpath, only when unique
        self._by_basename: dict[str, str] = {}
        seen_multiple: set[str] = set()
        for rel in by_relpath:
            base = Path(rel).name
            if base in self._by_basename or base in seen_multiple:
                self._by_basename.pop(base, None)
                seen_multiple.add(base)
            else:
                self._by_basename[base] = rel

    def __bool__(self) -> bool:
        return bool(self._by_relpath)

    def get(self, relpath: str) -> dict[str, str]:
        rel = relpath.replace("\\", "/")
        row = self._by_relpath.get(rel)
        if row is None:
            row = self._by_relpath.get(Path(rel).name)
        if row is None:
            mapped = self._by_basename.get(Path(rel).name)
            if mapped is not None:
                row = self._by_relpath.get(mapped)
        return row or {}

    def sort_value(self, relpath: str, column: str):
        """Return a sortable value for ``column``: numbers as float when
        possible, else lowercased string, with missing values sorting last."""
        row = self.get(relpath)
        raw = row.get(column, "")
        if raw is None or raw == "":
            return (1, 0.0, "")  # missing sorts last
        try:
            return (0, float(raw), "")
        except (ValueError, TypeError):
            return (0, 0.0, str(raw).lower())

    @classmethod
    def empty(cls) -> "Metadata":
        return cls([], {})

    @classmethod
    def load(cls, path: Path, root: Path) -> "Metadata":
        suffix = path.suffix.lower()
        if suffix == ".csv":
            return cls._load_csv(path)
        if suffix in (".tsv",):
            return cls._load_csv(path, delimiter="\t")
        if suffix in (".parquet", ".pq"):
            return cls._load_parquet(path)
        if suffix in (".json", ".jsonl"):
            return cls._load_json(path)
        raise ValueError(f"Unsupported metadata format: {suffix}")

    @staticmethod
    def _pick_path_column(fieldnames: list[str]) -> str:
        lowered = {c.lower(): c for c in fieldnames}
        for cand in PATH_COLUMNS:
            if cand in lowered:
                return lowered[cand]
        # Fall back to the first column.
        return fieldnames[0]

    @classmethod
    def _load_csv(cls, path: Path, delimiter: str = ",") -> "Metadata":
        with path.open("r", newline="", encoding="utf-8-sig") as f:
            reader = csv.DictReader(f, delimiter=delimiter)
            fieldnames = reader.fieldnames or []
            if not fieldnames:
                return cls.empty()
            key_col = cls._pick_path_column(fieldnames)
            columns = [c for c in fieldnames if c != key_col]
            by_relpath: dict[str, dict[str, str]] = {}
            for row in reader:
                key = (row.get(key_col) or "").strip().replace("\\", "/")
                if not key:
                    continue
                by_relpath[key] = {c: (row.get(c) or "") for c in columns}
        return cls(columns, by_relpath)

    @classmethod
    def _load_parquet(cls, path: Path) -> "Metadata":
        try:
            import pandas as pd
        except ImportError as e:  # pragma: no cover
            raise RuntimeError("Reading .parquet requires pandas") from e
        df = pd.read_parquet(path)
        fieldnames = list(df.columns)
        key_col = cls._pick_path_column(fieldnames)
        columns = [c for c in fieldnames if c != key_col]
        by_relpath: dict[str, dict[str, str]] = {}
        for _, row in df.iterrows():
            key = str(row[key_col]).strip().replace("\\", "/")
            if not key or key == "nan":
                continue
            by_relpath[key] = {c: ("" if pd.isna(row[c]) else str(row[c])) for c in columns}
        return cls(columns, by_relpath)

    @classmethod
    def _load_json(cls, path: Path) -> "Metadata":
        import json

        text = path.read_text(encoding="utf-8")
        if path.suffix.lower() == ".jsonl":
            records = [json.loads(line) for line in text.splitlines() if line.strip()]
        else:
            data = json.loads(text)
            records = data if isinstance(data, list) else list(data.values())
        if not records:
            return cls.empty()
        fieldnames: list[str] = []
        for rec in records:
            for k in rec:
                if k not in fieldnames:
                    fieldnames.append(k)
        key_col = cls._pick_path_column(fieldnames)
        columns = [c for c in fieldnames if c != key_col]
        by_relpath: dict[str, dict[str, str]] = {}
        for rec in records:
            key = str(rec.get(key_col, "")).strip().replace("\\", "/")
            if not key:
                continue
            by_relpath[key] = {c: ("" if rec.get(c) is None else str(rec.get(c))) for c in columns}
        return cls(columns, by_relpath)
