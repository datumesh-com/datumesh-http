"""Shared helpers for the feed jobs: errors, canonical decimals, JSON lines, state, HTTP. Standard library only."""
import argparse
import csv
import datetime as dt
import email.utils
import http.client
import hashlib
import io
import json
import os
import re
import sys
import time
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


class RateLimited(Exception):
    """The source answered 429 or 503: too many requests, try later. `retry_after` is the seconds its Retry-After
    header gave, or None."""

    def __init__(self, url, retry_after=None, code=429):
        super().__init__(f"HTTP {code} from {url}" + (f", retry after {retry_after:g} s" if retry_after is not None else ""))
        self.url, self.retry_after, self.code = url, retry_after, code


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


class UnknownHost(ValueError):
    """The host is not one of the table's hosts, spelled exactly. There is no default and no normalisation."""


_TABLE = None
_NAMES = {}


def _table() -> dict:
    global _TABLE
    if _TABLE is None:
        with open(FEEDS / "names.toml", "rb") as f:
            _TABLE = tomllib.load(f)
    return _TABLE


def _with_host(x, host):
    if isinstance(x, dict):
        return {k: _with_host(v, host) for k, v in x.items()}
    if isinstance(x, list):
        return [_with_host(v, host) for v in x]
    if isinstance(x, str):
        return x.replace("{host}", host)
    return x


def names(host) -> dict:
    """The table for one host. `host` has no default: it must be, character for character, a key of [hosts]
    (the rehearsal host or production). Anything else raises UnknownHost, so production names are produced only
    when the caller says datumesh.com."""
    table = _table()
    hosts = table["hosts"]
    if not isinstance(host, str) or host not in hosts:
        raise UnknownHost(f"host {host!r} is not one of {sorted(hosts)}")
    if host not in _NAMES:
        body = {k: v for k, v in table.items() if k != "hosts"}
        out = _with_host(body, host)
        out["meta"] = {"host": host, "status": hosts[host]["status"], "takeover_base": f"https://{host}/takeover"}
        _NAMES[host] = out
    return _NAMES[host]


def licences() -> list:
    """The licence facts of the register's entries (licences.toml): page, verbatim quotes, and a reason when the
    source is not relayed. Host-independent."""
    global _LICENCES
    if _LICENCES is None:
        with open(FEEDS / "licences.toml", "rb") as f:
            _LICENCES = tomllib.load(f)["licence"]
    return _LICENCES


_LICENCES = None


def series(key: str) -> dict:
    """The series of a relay source (policy_rates): host-independent data, read without a host."""
    return _table()[key]["series"]


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


def warn(message: str) -> None:
    """A line on stderr (the job's log), never on stdout (which carries the path of the file)."""
    print(message, file=sys.stderr)


def _retry_after(headers):
    """Seconds from a Retry-After header (a number, or an HTTP date), or None."""
    value = headers.get("Retry-After") if headers is not None else None
    if value is None:
        return None
    value = value.strip()
    if value.isdigit():
        return float(value)
    try:
        when = email.utils.parsedate_to_datetime(value)
    except (TypeError, ValueError):
        return None
    return max(0.0, when.timestamp() - time.time())


RETRY_WAITS = (5.0, 15.0, 45.0, 120.0)  # a transient network failure is retried after each of these


def _transient(e: BaseException) -> bool:
    """A failure worth another try: no HTTP answer (reset, refused, timeout, name lookup, a dropped connection) or a
    502 or 504. Not 404 (not published), 429 and 503 (rate limited, a different path), nor any other HTTP status."""
    if isinstance(e, urllib.error.HTTPError):
        return e.code in (502, 504)
    return isinstance(e, (OSError, http.client.HTTPException))  # URLError, timeouts and resets are OSErrors


def http_get(url: str, *, sleep=None) -> bytes:
    """The body of url. A transient network failure is retried up to 4 times (waits 5, 15, 45, 120 s) and then
    raised as it was; 404 is NotPublished, 429 and 503 are RateLimited, other HTTP errors are raised at once."""
    sleep = sleep or time.sleep
    for attempt in range(len(RETRY_WAITS) + 1):
        req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        try:
            with urllib.request.urlopen(req, timeout=180) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code == 404:
                raise NotPublished(url) from e
            if e.code in (429, 503):
                raise RateLimited(url, _retry_after(e.headers), e.code) from e
            if not _transient(e) or attempt == len(RETRY_WAITS):
                raise
            err = e
        except (OSError, http.client.HTTPException) as e:
            if attempt == len(RETRY_WAITS):
                raise
            err = e
        wait = RETRY_WAITS[attempt]
        warn(f"http_get {url}: {type(err).__name__}: {err}; retry {attempt + 1} of {len(RETRY_WAITS)} in {wait:g} s")
        sleep(wait)


def _host_arg(value: str) -> str:
    try:
        names(value)
    except UnknownHost as e:
        raise argparse.ArgumentTypeError(str(e)) from e
    return value


def base_parser(modes=()) -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="Fetch one source, check its shape, write a publish-stored file.")
    p.add_argument("--host", required=True, type=_host_arg,
                   help="the mesh host the names are spelled for: a host of the names table, spelled exactly (no default)")
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
