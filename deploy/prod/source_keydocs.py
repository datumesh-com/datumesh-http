#!/usr/bin/env python3
"""The key-document check of the sources the mesh relays (CONTRACT-guard-patch P3).

Runs daily from root's crontab on vmi630775 (17 5 * * *, outside quran-box; see install_keydoc_cron.sh),
as /root/datumesh-prod/bin/source_keydocs.py. Run: python3 -I source_keydocs.py (standard library only).

What it does: reads the mesh's catalogue (/catalog and the /catalog/... pages it links, at most 200 pages)
and takes the domains of its relay links (/fn/<host>/relay/<domain>/<fn>), at most 500 domains; keeps those
whose GET /takeover/<domain> is 200 (the mesh confirms it relays from them); then GETs each such source's
key document (https://<domain>/.well-known/datumesh.json) and records whether it is present (200, a JSON
object whose "host" is the domain), absent (any other answer, or a fetch error such as DNS, TLS or timeout)
or invalid (200 but not such an object). Key-document bodies are read at most 1 MiB; sha256 is of the exact
body bytes. A document that is new, or whose sha256 changed since the last run, is reported.

Environment (defaults):
  DATUMESH_MESH        the mesh origin                         (https://datumesh.com)
  DATUMESH_STATUS_DIR  where the status files go; created 0755 (/root/datumesh-prod/status)
  DATUMESH_SOURCE_URL  a source's key document URL, with {domain} (https://{domain}/.well-known/datumesh.json)

Files written in the status dir (atomically: a temp file, then os.replace):
  source-keydocs.json   {"checked", "mesh", "ok", "domains": [{"domain", "keydoc": present|absent|invalid,
                         "http", "sha256", "first_seen", "error"}], "new": [...], "changed": [...]}
  source-keydocs.txt    one line: "<time> OK: ..." | "<time> NEW: ..." | "<time> WARN: ..."
  source-keydocs.state  the present documents (domain, sha256, first_seen), kept across runs (mode 0600)
The same line is printed to stdout (the cron log, /root/datumesh-prod/source-keydocs.log), as uptime.sh does.
A mesh that cannot be read (the catalogue, or a takeover check that fails) is WARN, exit 1, and the state is
kept. Times are UTC, "%Y-%m-%dT%H:%M:%SZ".

Nothing changes on the mesh: every request to it is a GET, at most one per second, with a 15 s timeout and the
User-Agent "datumesh-keydoc-check/1 (+https://datumesh.com)". Links are followed only within the mesh origin.
"""
import hashlib
import html.parser
import json
import os
import re
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone

UA = "datumesh-keydoc-check/1 (+https://datumesh.com)"
TIMEOUT = 15
MESH_INTERVAL = 1.0
MAX_CATALOGUE_PAGES = 200
MAX_DOMAINS = 500
MAX_KEYDOC = 1024 * 1024
MAX_CATALOGUE_BODY = 4 * 1024 * 1024
DOMAIN_RE = re.compile(r"^(?=.{1,253}$)[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$")
RELAY_RE = re.compile(r"^/fn/[^/]+/relay/([^/]+)/[^/]+$")

_last_mesh = [0.0]


class MeshError(Exception):
    pass


def now():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def short(e):
    text = str(getattr(e, "reason", None) or e) or type(e).__name__
    return " ".join(text.split())[:120]


def write_atomic(path, data, mode):
    d = os.path.dirname(path)
    fd, tmp = tempfile.mkstemp(dir=d, prefix="." + os.path.basename(path) + ".", suffix=".tmp")
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        os.chmod(tmp, mode)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def mesh_get(url, accept):
    """A GET to the mesh, at most one per second. Returns (status, body bytes); raises MeshError."""
    wait = MESH_INTERVAL - (time.monotonic() - _last_mesh[0])
    if wait > 0:
        time.sleep(wait)
    _last_mesh[0] = time.monotonic()
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": accept})
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
            return r.status, r.read(MAX_CATALOGUE_BODY + 1)
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return 404, b""
        raise MeshError(f"HTTP {e.code}")
    except Exception as e:  # DNS, TLS, timeout, reset
        raise MeshError(short(e))


class LinkParser(html.parser.HTMLParser):
    def __init__(self):
        super().__init__()
        self.hrefs = []

    def handle_starttag(self, tag, attrs):
        if tag == "a":
            for k, v in attrs:
                if k == "href" and v:
                    self.hrefs.append(v)


def origin_of(url):
    p = urllib.parse.urlsplit(url)
    return f"{p.scheme}://{p.netloc}"


def discover(mesh):
    """The relayed domains the catalogue links to, as a sorted list (the catalogue crawl is breadth-first)."""
    start = mesh + "/catalog"
    queue = [start]
    seen = {start}
    domains = set()
    pages = 0
    while queue and pages < MAX_CATALOGUE_PAGES:
        url = queue.pop(0)
        status, body = mesh_get(url, "text/html")
        pages += 1
        if status == 404 and url != start:
            continue  # a linked catalogue page the mesh does not serve is skipped, not a failure
        if status != 200:
            raise MeshError(f"HTTP {status} for {urllib.parse.urlsplit(url).path}")
        if len(body) > MAX_CATALOGUE_BODY:
            raise MeshError("a catalogue page is too large")
        parser = LinkParser()
        parser.feed(body.decode("utf-8", "replace"))
        for href in parser.hrefs:
            absolute, _ = urllib.parse.urldefrag(urllib.parse.urljoin(url, href))
            if origin_of(absolute) != mesh:
                continue
            p = urllib.parse.urlsplit(absolute)
            m = RELAY_RE.match(p.path)
            if m:
                if DOMAIN_RE.match(m.group(1)):
                    domains.add(m.group(1))
                continue
            if (p.path == "/catalog" or p.path.startswith("/catalog/")) and absolute not in seen:
                seen.add(absolute)
                queue.append(absolute)
        if len(domains) >= MAX_DOMAINS:
            break
    return sorted(domains)[:MAX_DOMAINS]


def confirmed(mesh, domains):
    """The domains whose takeover page says the mesh relays from them (200); 404 is not confirmed."""
    keep = []
    for d in domains:
        status, _ = mesh_get(f"{mesh}/takeover/{d}", "application/json")
        if status == 200:
            keep.append(d)
    return keep


def fetch_keydoc(source_template, domain):
    """(keydoc, http, sha256, error) for one source. Never raises."""
    url = source_template.replace("{domain}", domain)
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
            code = r.status
            body = r.read(MAX_KEYDOC + 1)
    except urllib.error.HTTPError as e:
        return "absent", e.code, None, None
    except Exception as e:
        return "absent", None, None, short(e)
    if code != 200:
        return "absent", code, None, None
    if len(body) > MAX_KEYDOC:
        return "invalid", code, None, "body over 1 MiB"
    try:
        obj = json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        return "invalid", code, None, "not JSON"
    if not isinstance(obj, dict) or obj.get("host") != domain:
        return "invalid", code, None, f"not a key document for {domain}"
    return "present", code, hashlib.sha256(body).hexdigest(), None


def load_state(path):
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
        return {e["domain"]: e for e in data.get("domains", [])}
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        return {}


def plural(n, word):
    return f"{n} {word}" + ("" if n == 1 else "s")


def main():
    mesh = os.environ.get("DATUMESH_MESH", "https://datumesh.com").rstrip("/")
    status_dir = os.environ.get("DATUMESH_STATUS_DIR", "/root/datumesh-prod/status")
    source = os.environ.get("DATUMESH_SOURCE_URL", "https://{domain}/.well-known/datumesh.json")
    os.makedirs(status_dir, mode=0o755, exist_ok=True)
    state_path = os.path.join(status_dir, "source-keydocs.state")
    json_path = os.path.join(status_dir, "source-keydocs.json")
    txt_path = os.path.join(status_dir, "source-keydocs.txt")
    checked = now()

    try:
        domains = confirmed(mesh, discover(mesh))
    except MeshError as e:
        line = f"{checked} WARN: the mesh's catalogue could not be read: {e}"
        status = {"checked": checked, "mesh": mesh, "ok": False, "domains": [], "new": [], "changed": []}
        write_atomic(json_path, (json.dumps(status, indent=2) + "\n").encode(), 0o644)
        write_atomic(txt_path, (line + "\n").encode(), 0o644)
        print(line)
        return 1

    prev = load_state(state_path)
    rows, new, changed, keep = [], [], [], {}
    for d in domains:
        keydoc, http, sha, error = fetch_keydoc(source, d)
        first = None
        if keydoc == "present":
            old = prev.get(d)
            if old is None:
                new.append(d)
                first = checked
            elif old.get("sha256") == sha:
                first = old.get("first_seen") or checked
            else:
                changed.append(d)
                first = checked
            keep[d] = {"domain": d, "sha256": sha, "first_seen": first}
        rows.append({"domain": d, "keydoc": keydoc, "http": http, "sha256": sha if keydoc == "present" else None,
                     "first_seen": first, "error": error})

    present = sorted(keep)
    n = plural(len(domains), "relayed domain")
    if new or changed:
        parts = []
        if new:
            parts.append("key document at " + ", ".join(new))
        if changed:
            parts.append(", ".join(f"{d} changed" for d in changed))
        body = "NEW: " + "; ".join(parts) + f" ({n})"
    elif present:
        body = f"OK: {n}; key documents at {', '.join(present)}"
    else:
        body = f"OK: {n}, no key document"
    line = f"{checked} {body}"

    status = {"checked": checked, "mesh": mesh, "ok": True, "domains": rows, "new": new, "changed": changed}
    state = {"domains": [keep[d] for d in present]}
    write_atomic(json_path, (json.dumps(status, indent=2) + "\n").encode(), 0o644)
    write_atomic(state_path, (json.dumps(state, indent=2) + "\n").encode(), 0o600)
    write_atomic(txt_path, (line + "\n").encode(), 0o644)
    print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
