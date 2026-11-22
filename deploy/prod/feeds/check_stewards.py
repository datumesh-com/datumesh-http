"""The steward's change-process check (CONTRACT-classes-round K12): before any class reaches production, the `steward.process`
of each class line must be a real page.

    python check_stewards.py FILE [FILE ...]

Reads each publish file (JSON lines). For each class line (a line with a `class` member), it takes `descriptor.steward.process`
and fetches it with a GET. The page must answer HTTP 200 with a `text/html` content type; anything else is a failure, which is
printed with the file, the line, the URL and the reason, and every failure is reported (no stop at the first). The exit code
is 0 when every class line passes, 1 when one does not, and 2 for a usage error (no file).

Rehearsal hosts (the hosts with status "rehearsal" in names.toml and sec_names.toml) are exempt: their addresses are not real
pages, no request is made to them, and they are listed in the output. Run it before any production publish of a class.

Standard library only."""
import argparse
import json
import pathlib
import sys
import tomllib
import urllib.error
import urllib.parse
import urllib.request

FEEDS = pathlib.Path(__file__).resolve().parent
TIMEOUT_SECONDS = 20


def rehearsal_hosts():
    """The hosts the name tables list as rehearsal hosts (spelled exactly)."""
    hosts = set()
    for name in ("names.toml", "sec_names.toml"):
        with open(FEEDS / name, "rb") as fh:
            hosts |= {h for h, v in tomllib.load(fh)["hosts"].items() if v.get("status") == "rehearsal"}
    return hosts


def fetch(url):
    """The reason the page at `url` is not a 200 text/html page, or None when it is one. One GET."""
    request = urllib.request.Request(url, method="GET", headers={"User-Agent": "datumesh-check-stewards"})
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:
            status = response.status
            ctype = response.headers.get_content_type()
    except urllib.error.HTTPError as e:
        return f"HTTP {e.code}"
    except (urllib.error.URLError, OSError, ValueError) as e:
        return f"not fetched: {getattr(e, 'reason', e)}"
    if status != 200:
        return f"HTTP {status}"
    if ctype != "text/html":
        return f"content type {ctype}, not text/html"
    return None


def class_lines(path):
    """The class lines of a publish file, with their line numbers, as (line number, parsed line)."""
    with open(path, encoding="utf-8") as fh:
        for n, text in enumerate(fh, start=1):
            if not text.strip():
                continue
            v = json.loads(text)
            if isinstance(v, dict) and "class" in v:
                yield n, v


def main(argv):
    parser = argparse.ArgumentParser(description="Checks that each class line's steward.process is a real page.")
    parser.add_argument("files", nargs="+", metavar="FILE", help="a publish file (JSON lines)")
    args = parser.parse_args(argv)
    exempt = rehearsal_hosts()
    failures = 0
    for f in args.files:
        for n, line in class_lines(f):
            process = line.get("descriptor", {}).get("steward", {}).get("process")
            where = f"{f}:{n}"
            if not isinstance(process, str) or not process:
                failures += 1
                print(f"FAIL {where}: class {line['class']} has no steward.process")
                continue
            host = urllib.parse.urlsplit(process).hostname or ""
            if host in exempt:
                print(f"exempt (rehearsal host, not a real page): {process} ({where})")
                continue
            reason = fetch(process)
            if reason is None:
                print(f"OK {pathlib.Path(f).name} line {n}: {process}")
            else:
                failures += 1
                print(f"FAIL {where}: {process}: {reason}")
    if failures:
        print(f"{failures} steward address(es) are not real pages: do not publish the classes", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
