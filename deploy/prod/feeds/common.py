"""Shared helpers for the feed jobs: errors, canonical decimals, JSON lines, state, HTTP. Standard library only."""
import argparse
import csv
import datetime as dt
import hashlib
import io
import json
import os
import re
import tomllib
import unicodedata
import urllib.error
import urllib.request
from pathlib import Path

FEEDS = Path(__file__).resolve().parent
USER_AGENT = "datumesh-feeds/0.1"  # no contact details: the sources see only the project

DECIMAL = re.compile(r"^-?\d+(\.\d+)?$")
ISO_DATE = re.compile(r"^(\d{4})-(\d\d)-(\d\d)$")
INSTANT = re.compile(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{6}Z$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")


class ShapeError(ValueError):
    """The fetched file is not the shape this job knows. Nothing is emitted, no state is kept."""


class ConformanceError(ValueError):
    """Two independent copies of the source disagree. Nothing is published."""


class EmptyRegion(ValueError):
    """A region has no pixel centre inside it with data."""


class NotPublished(Exception):
    """The source answered 404: the file for that period is not published yet."""


def shape(cond, message):
    if not cond:
        raise ShapeError(message)


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def decimal_text(s: str) -> str:
    """The published decimal without insignificant trailing zeros: 0.83290 -> 0.8329, 137.00 -> 137.
    Never rounded, never rescaled."""
    shape(isinstance(s, str) and DECIMAL.match(s) is not None, f"not a decimal number: {s!r}")
    if "." in s:
        s = s.rstrip("0").rstrip(".")
    return s


def is_number(s) -> bool:
    return isinstance(s, str) and DECIMAL.match(s) is not None


def nfc(s: str) -> str:
    return unicodedata.normalize("NFC", s)


def decode(data: bytes) -> str:
    try:
        return data.decode("utf-8-sig")
    except UnicodeDecodeError as e:
        raise ShapeError(f"not UTF-8: {e}") from e


def csv_table(data: bytes):
    """-> (header, rows as lists). Every row must have as many cells as the header."""
    reader = csv.reader(io.StringIO(decode(data), newline=""))
    try:
        header = next(reader)
    except StopIteration:
        raise ShapeError("empty file") from None
    rows = []
    for row in reader:
        if not row:
            continue
        shape(len(row) == len(header), f"row has {len(row)} cells, header has {len(header)}")
        rows.append(row)
    return header, rows


def csv_dicts(data: bytes, required):
    header, rows = csv_table(data)
    missing = set(required) - set(header)
    shape(not missing, f"missing columns: {sorted(missing)}")
    return [dict(zip(header, row)) for row in rows]


def dumps_line(obj) -> str:
    return json.dumps(obj, ensure_ascii=False)


def write_jsonl(path: Path, lines) -> Path:
    with open(path, "x", encoding="utf-8", newline="\n") as f:
        for line in lines:
            f.write(dumps_line(line) + "\n")
    return path


_NAMES = None


def names() -> dict:
    global _NAMES
    if _NAMES is None:
        with open(FEEDS / "names.toml", "rb") as f:
            _NAMES = tomllib.load(f)
    return _NAMES


def now_instant(epoch: float | None = None) -> str:
    t = dt.datetime.now(dt.timezone.utc) if epoch is None else dt.datetime.fromtimestamp(epoch, dt.timezone.utc)
    return t.strftime("%Y-%m-%dT%H:%M:%S.%fZ")


def load_state(path: Path) -> dict:
    try:
        return json.loads(Path(path).read_text(encoding="utf-8"))
    except FileNotFoundError:
        return {}


def save_state(path: Path, state: dict) -> None:
    path = Path(path)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(state, ensure_ascii=False, sort_keys=True), encoding="utf-8")
    os.replace(tmp, path)


def http_get(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(req, timeout=180) as r:
            return r.read()
    except urllib.error.HTTPError as e:
        if e.code == 404:
            raise NotPublished(url) from e
        raise


def base_parser(modes=()) -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="Fetch one source, check its shape, write a publish-stored file.")
    p.add_argument("--state-dir", required=True)
    p.add_argument("--out-dir", required=True)
    if modes:
        p.add_argument("--mode", default=modes[0], choices=modes)
    return p


def finish(path) -> int:
    """The job prints the file it wrote (nothing when there was nothing new); the shell publishes it."""
    if path:
        print(path)
    return 0
