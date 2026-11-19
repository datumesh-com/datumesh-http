"""The licence register (answer 3): the class `<host>/register/dataset(source, code)`, a stored method per fact,
and one entry per dataset examined: the sources relayed, the one computed from, and those not relayed.

Input: the register.json of the licence check (every verdict: RELAY and SKIP). Every name, the entries, the
methods' texts and where each value comes from are in names.toml. There is no `retrieved` key: `retrieved` is a
method. A new check that finds the same licence text emits nothing; a changed text emits new versions of that
entry's cells (the same cells, a later version), and `asof` gives the history."""
import hashlib
import json
from pathlib import Path

import common
from common import INSTANT, SHA256, ShapeError, shape, write_jsonl

KEY = "register"
REQUIRED = {"source_domain", "licence_url", "retrieved", "sha256", "verdict", "spdx_or_url", "attribution",
            "conditions"}
VERDICTS = {"RELAY", "SKIP"}


def parse(data: bytes) -> list:
    """Every entry of register.json, checked: the shape, the hash, the instant, the verdict."""
    try:
        entries = json.loads(common.decode(data))
    except json.JSONDecodeError as e:
        raise ShapeError(f"register does not parse: {e}") from e
    shape(isinstance(entries, list) and entries, "register: not a list of entries")
    for e in entries:
        shape(isinstance(e, dict), "register: an entry is not an object")
        missing = REQUIRED - set(e)
        shape(not missing, f"register: entry lacks {sorted(missing)}")
        who = e["source_domain"]
        shape(isinstance(e["sha256"], str) and SHA256.match(e["sha256"]), f"register: bad sha256 for {who}")
        shape(isinstance(e["retrieved"], str) and INSTANT.match(e["retrieved"]), "register: bad retrieved instant")
        shape(e["verdict"] in VERDICTS, f"register: verdict of {who} is not one of {sorted(VERDICTS)}")
        shape(isinstance(e["licence_url"], str) and e["licence_url"].startswith("https://"), f"register: bad URL for {who}")
        shape(isinstance(e["conditions"], list) and all(isinstance(c, str) for c in e["conditions"]),
              f"register: conditions of {who} are not a list of text")
        shape(e["attribution"] is None or isinstance(e["attribution"], str), f"register: bad attribution for {who}")
    return entries


def jcs(obj) -> bytes:
    """Canonical JSON for text and integers (what a descriptor holds): sorted keys, no spaces, UTF-8."""
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def _clauses(d, e) -> str:
    parts = []
    if d["status"] == "not_relayed":
        parts.append(f"Not relayed. Reason: {d['reason']}")
    parts.append("Clauses: " + " | ".join(e["conditions"]))
    if e["attribution"]:
        parts.append(f"Attribution: {e['attribution']}")
    return " ".join(parts)


def _value(sec, method, d, e):
    where = sec["value"][method]
    if where == "clauses":
        text = _clauses(d, e)
    else:
        scope, field = where.split(".")
        text = {"dataset": d, "entry": e}[scope][field]
    return {"type": sec.get("returns", {}).get(method, "string"), "value": text}


def _method_descriptor(sec, method) -> dict:
    args = {k: {"type": "string", "valid": False} for k in sec["keys"]}
    return {"code": {"body": "", "lang": "stored"}, "inputs": [],
            "signature": {"args": args, "order": list(sec["keys"]), "returns": sec.get("returns", {}).get(method, "string")}}


def _class_lines(sec, cls):
    """Descriptor lines for the methods, the class line (its keys, steward and methods by hash), the catalogue."""
    lines, hashes = [], {}
    for m in sec["methods"]:
        d = _method_descriptor(sec, m)
        lines.append({"descriptor": d})
        hashes[m] = hashlib.sha256(jcs(d)).hexdigest()
    none = {"type": "string", "key_normalisation": "none"}
    lines.append({"class": cls, "descriptor": {
        "keys": {k: dict(none) for k in sec["keys"]}, "order": list(sec["keys"]), "methods": hashes,
        "steward": {"namespace": sec["namespace"], "process": sec["steward_process"]}}})
    sources = list(dict.fromkeys(d["source"] for d in sec["datasets"]))
    lines.append({"catalog": cls, "kind": "class", "label": sec["class_label"], "description": sec["class_description"],
                  "args": dict(sec["key_text"]), "examples": [],
                  "dictionary": {"source": [{"value": s, "label": sec["source_labels"][s]} for s in sources],
                                 "code": [{"value": d["code"], "label": d["name"]} for d in sec["datasets"]]}})
    for m in sec["methods"]:
        t = sec["method_text"][m]
        lines.append({"catalog": f"{cls}/{m}", "kind": "function", "label": t["label"], "description": t["description"],
                      "args": dict(sec["key_text"]), "examples": [], "dictionary": {}})
    return lines


def _cell_lines(sec, cls, d, e):
    args = {"source": {"type": "string", "value": d["source"]}, "code": {"type": "string", "value": d["code"]}}
    fields = {"source": d["source"], "code": d["code"], "name": d["name"], "status": d["status"],
              "retrieved": e["retrieved"], "licence_page": e["licence_url"]}
    lines = []
    for m in sec["methods"]:
        summary = common.nfc(sec["method_text"][m]["summary"].format(**fields))
        lines.append({"cell": {"fn": f"{cls}/{m}", "args": dict(args)}, "value": _value(sec, m, d, e),
                      "licence": sec["licence"], "summary": summary})
    return lines


def emit(entries, state, names):
    """-> (lines, state). The first call writes the method descriptors, the class and the catalogue; every call
    writes the six cells of each dataset whose licence text (its SHA-256) is not the one in the state."""
    sec = names[KEY]
    cls = f"{sec['namespace']}/{sec['class']}"
    by_input = {e["source_domain"]: e for e in entries}
    done = dict(state.get("done", {}))
    todo = []
    for d in sec["datasets"]:
        e = by_input.get(d["input"])
        shape(e is not None, f"register: no entry {d['input']!r} for {d['source']} {d['code']}")
        shape((e["verdict"] == "SKIP") == (d["status"] == "not_relayed"),
              f"register: verdict {e['verdict']} does not fit status {d['status']} for {d['code']}")
        if done.get(f"{d['source']}|{d['code']}") != e["sha256"]:
            todo.append((d, e))
    if not todo:
        return [], state
    lines = [] if state.get("class") else _class_lines(sec, cls)
    for d, e in todo:
        lines += _cell_lines(sec, cls, d, e)
        done[f"{d['source']}|{d['code']}"] = e["sha256"]
    new_state = dict(state)
    new_state["class"] = True
    new_state["done"] = dict(sorted(done.items()))
    return lines, new_state


def run(names, *, path, now, state_dir, out_dir, publish=None):
    state_path = Path(state_dir) / f"{KEY}.json"
    state = common.load_state(state_path)
    lines, state = emit(parse(Path(path).read_bytes()), state, names)
    if not lines:
        return None
    out = Path(out_dir) / f"{KEY}-{now.replace(':', '').replace('-', '')}.jsonl"
    write_jsonl(out, lines)
    if publish is not None:
        try:
            publish(out)
        except BaseException:
            out.unlink(missing_ok=True)
            raise
    common.save_state(state_path, state)
    return out


def main(argv=None):
    p = common.base_parser()
    p.add_argument("--register", required=True, help="the register.json of the licence check")
    a = p.parse_args(argv)
    return common.finish(run(common.names(a.host), path=a.register, now=common.now_instant(),
                             state_dir=a.state_dir, out_dir=a.out_dir))


if __name__ == "__main__":
    raise SystemExit(main())
