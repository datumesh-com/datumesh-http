"""The series file of the month pages (Build 2 Part 6, G2): which series get pages, from names.toml, for one host.

  python -B series.py --host H --out FILE

writes a JSON array with one object a series: function, granularity (month or year pages), date_arg, column_arg (if the
series has columns), fixed (arguments held fixed), unit, kind (relayed or computed) and, for a computed series, its
source. The mesh reads it through DATUMESH_SERIES_FILE; without it there are no pages. Every name and text is in
names.toml (the series table and the page_* keys of each section); the host has no default."""
import argparse
import json
import os
from pathlib import Path

import common


def build(names: dict) -> list:
    """The series entries of names (resolved for one host), in the order of names.toml's [series]."""
    out = []
    for key in names["series"]["order"]:
        sec = names[key]
        entry = {"function": f"{sec['namespace']}/{sec['function']}", "granularity": sec["page_granularity"],
                 "date_arg": sec["page_date_arg"]}
        if "page_column_arg" in sec:
            entry["column_arg"] = sec["page_column_arg"]
        if "page_fixed_from" in sec:
            entry["fixed"] = {arg: sec[arg] for arg in sec["page_fixed_from"]}
        entry["unit"] = sec["page_unit"]
        entry["kind"] = sec["page_kind"]
        if "page_source" in sec:
            entry["source"] = sec["page_source"]
        out.append(entry)
    return out


def write(names: dict, path) -> Path:
    """The file, replaced whole (written beside it and moved over it)."""
    path = Path(path)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(json.dumps(build(names), indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(tmp, path)
    return path


def main(argv=None):
    p = argparse.ArgumentParser(description="Write the series file of the month pages from names.toml.")
    p.add_argument("--host", required=True, type=common._host_arg,
                   help="the mesh host the names are spelled for: a host of the names table, spelled exactly (no default)")
    p.add_argument("--out", required=True, help="the file to write, for example /srv/datumesh/feeds/series.<host>.json")
    a = p.parse_args(argv)
    print(write(common.names(a.host), a.out))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
