"""The class line of <host>/sec/company (CONTRACT-sec-classes C5), our subclass of <host>/relay/sec.gov/company.

A class line holds method hashes, and only the mesh's define computes a formula's hash, so this is a separate step, run
once the views and the company file are published. It asks the mesh (`GET <mesh>/describe/<name>`) for the parent's
class_hash and for each method's fn_hash, and writes two JSON lines: the class line and its catalogue line.

    python sec_class.py --host H --mesh URL --out FILE

The host is a rehearsal host of sec_names.toml, spelled exactly; datumesh.com is refused. An unbound name, or a mesh that
cannot be reached, is an error that names it: nothing is written. Standard library only."""
import argparse
import json
import sys
import tomllib
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

FEEDS = Path(__file__).resolve().parent
SEC_NAMES = FEEDS / "sec_names.toml"
PRODUCTION = "datumesh.com"
TIMEOUT = 30

# the methods of the subclass that are stored (C3) and the formulas of the views (C4), in the order of the views file
STORED = ["fact", "annual_report", "loaded", "from_label"]
FORMULAS = ["fact_in", "reported", "latest", "revenue", "cost_of_revenue", "net_income", "operating_income",
            "total_assets", "current_assets", "current_liabilities", "inventory", "ppe_net", "accounts_payable",
            "accounts_receivable", "current_ratio", "return_on_assets", "inventory_turnover", "fixed_asset_turnover",
            "days_payable_outstanding", "cash_conversion_cycle", "gross_margin", "revenue_growth"]
METHODS = STORED + FORMULAS


class Failed(Exception):
    """A name is unbound, or the mesh cannot answer: the message names it, and nothing is written."""


def _host_arg(value: str) -> str:
    """A rehearsal host of sec_names.toml, spelled exactly. datumesh.com, and any host the file does not list as a
    rehearsal host, is refused before any request."""
    with open(SEC_NAMES, "rb") as f:
        hosts = tomllib.load(f)["hosts"]
    if value == PRODUCTION or value not in hosts or hosts[value].get("status") != "rehearsal":
        raise argparse.ArgumentTypeError(f"host {value!r} is not a rehearsal host of sec_names.toml; datumesh.com is refused")
    return value


def describe(mesh: str, name: str) -> dict:
    """The mesh's describe of one name. An unbound name (404) or an unreachable mesh is Failed, naming the name."""
    url = mesh + "/describe/" + urllib.parse.quote(name, safe="/")
    try:
        with urllib.request.urlopen(url, timeout=TIMEOUT) as r:
            raw = r.read()
    except urllib.error.HTTPError as e:
        raise Failed(f"{name}: the mesh answered HTTP {e.code} (is it bound?)") from e
    except (OSError, ValueError) as e:
        raise Failed(f"{name}: the mesh at {mesh} could not be reached: {e}") from e
    try:
        return json.loads(raw.decode("utf-8"))
    except ValueError as e:
        raise Failed(f"{name}: the mesh's answer is not JSON") from e


def _hash(mesh: str, name: str, key: str) -> str:
    h = describe(mesh, name).get(key)
    if not isinstance(h, str) or not h:
        raise Failed(f"{name}: the mesh's answer has no {key} (is it bound?)")
    return h


def build(host: str, mesh: str) -> list:
    """The class line and the catalogue line of <host>/sec/company, from the mesh's hashes."""
    with open(SEC_NAMES, "rb") as f:
        s = tomllib.load(f)["sec"]
    namespace = s["namespace"].replace("{host}", host)
    parent = f"{namespace}/{s['company_class']}"
    sub = f"{host}/sec/company"
    parent_hash = _hash(mesh, parent, "class_hash")
    hashes = {m: _hash(mesh, f"{sub}/{m}", "fn_hash") for m in METHODS}
    cls = {"class": sub, "descriptor": {
        "keys": {}, "order": [], "parent": parent_hash, "methods": hashes, "lookup": "from_label",
        "extent": {"keys": hashes["loaded"]},
        "steward": {"namespace": f"{host}/sec", "process": f"https://{host}/classes/change"}}}
    catalog = {
        "catalog": sub, "kind": "class", "label": "SEC company (Datumesh views over the SEC relay)",
        "description": ("A subclass of the SEC relay's company class, one company per cik: its figures as filed, the "
                        "annual report of a fiscal year (our selection), the companies in scope, the company by its name "
                        "or ticker, and the ratios and money views. Labels are exact: a name or ticker is looked up as the "
                        "SEC lists it, case-sensitive."),
        "args": {"cik": "the company's CIK, 10 digits with leading zeros (labels are exact and case-sensitive)"},
        "open_keys": ["cik"], "examples": [], "dictionary": {}}
    return [cls, catalog]


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description="Writes the class line of <host>/sec/company (rehearsal hosts only).")
    p.add_argument("--host", required=True, type=_host_arg,
                   help="a rehearsal host of sec_names.toml, spelled exactly (datumesh.com is refused)")
    p.add_argument("--mesh", required=True, help="the mesh's base URL, for GET <mesh>/describe/<name>")
    p.add_argument("--out", required=True, help="the file to write: the class line, then its catalogue line")
    a = p.parse_args(argv)
    try:
        lines = build(a.host, a.mesh.rstrip("/"))
    except Failed as e:
        print(f"sec_class: {e}; nothing is written", file=sys.stderr)
        return 1
    text = "".join(json.dumps(l, ensure_ascii=False, sort_keys=True) + "\n" for l in lines)
    Path(a.out).write_text(text, encoding="utf-8", newline="\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
