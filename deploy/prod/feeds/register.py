"""The licence register (answer 3): the class `<host>/register/dataset(source, code)`, a stored method per fact,
and one entry per dataset examined: the sources relayed, the one computed from, and those not relayed.

Input: the register.json of the licence check (every verdict: RELAY and SKIP: the page, hash and date), and
licences.toml (the verbatim clauses of LICENCES.md and the reasons). Every name, the entries, the methods' texts and
where each value comes from are in names.toml. There is no `retrieved` key: `retrieved` is a
method. A new check that finds the same licence text emits nothing; a changed text emits new versions of that
entry's cells (the same cells, a later version), and `asof` gives the history."""
import hashlib
import json
from pathlib import Path

import common
import relay
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


def _clauses(lic) -> str:
    """The verbatim quotes of licences.toml, joined by " | "; a source not relayed starts with its reason."""
    text = " | ".join(lic["quotes"])
    if "reason" in lic:
        text = f"Not relayed. Reason: {lic['reason']} Clauses: {text}"
    return text


def _value(sec, method, d, e, lic, blobs):
    """A string, an instant, or a text blob: {"blob": "blobs/<sha256>", "media_type"} with the bytes put in `blobs`
    (the file goes next to the lines file, named by its hash, as the scripture loader does)."""
    where = sec["value"][method]
    if where == "clauses":
        text = _clauses(lic)
    else:
        scope, field = where.split(".")
        text = {"dataset": d, "entry": e}[scope][field]
    kind = sec.get("returns", {}).get(method, "string")
    if kind == "blob":
        data = text.encode("utf-8")
        path = "blobs/" + hashlib.sha256(data).hexdigest()
        blobs[path] = data
        return {"blob": path, "media_type": sec["blob_media_type"]}
    shape(len(text.encode("utf-8")) <= 1024, f"register: the {method} value is over the 1024-byte string limit")
    return {"type": kind, "value": text}


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


def _expected(value, blobs):
    """The typed scalar a test expects for a cell value: the value itself, or for a blob file reference the blob scalar
    `<sha256>:<length>:<media type>` (spec 3.7), the length in bytes of the file."""
    if "blob" in value:
        sha = value["blob"].rsplit("/", 1)[1]
        return {"type": "blob", "value": f"{sha}:{len(blobs[value['blob']])}:{value['media_type']}"}
    return value


def _test_line(cell, blobs):
    """One test per cell: the method's name, its call (source and code), and the value the cell emitted."""
    fn = cell["cell"]["fn"]
    a = cell["cell"]["args"]
    call = relay.call_text(fn, ["source", "code"], {k: a[k]["value"] for k in ("source", "code")})
    return {"test": fn, "call": call, "expect": _expected(cell["value"], blobs)}


def _cell_lines(sec, cls, d, e, lic, blobs):
    args = {"source": {"type": "string", "value": d["source"]}, "code": {"type": "string", "value": d["code"]}}
    fields = {"source": d["source"], "code": d["code"], "name": d["name"], "status": d["status"],
              "retrieved": e["retrieved"], "licence_page": e["licence_url"]}
    lines = []
    for m in sec["methods"]:
        summary = common.nfc(sec["method_text"][m]["summary"].format(**fields))
        lines.append({"cell": {"fn": f"{cls}/{m}", "args": dict(args)}, "value": _value(sec, m, d, e, lic, blobs),
                      "licence": sec["licence"], "summary": summary})
    return lines


def emit(entries, state, names, licences=None, blobs=None):
    """-> (lines, state). The first call writes the method descriptors, the class and the catalogue; every call
    writes the six cells of each dataset whose licence text (its SHA-256) is not the one in the state."""
    blobs = {} if blobs is None else blobs
    sec = names[KEY]
    cls = f"{sec['namespace']}/{sec['class']}"
    lics = {(x["source"], x["code"]): x for x in (common.licences() if licences is None else licences)}
    by_input = {e["source_domain"]: e for e in entries}
    done = dict(state.get("done", {}))
    todo = []
    for d in sec["datasets"]:
        e = by_input.get(d["input"])
        shape(e is not None, f"register: no entry {d['input']!r} for {d['source']} {d['code']}")
        lic = lics.get((d["source"], d["code"]))
        shape(lic is not None and lic["quotes"], f"register: no licence quotes for {d['source']} {d['code']}")
        shape((e["verdict"] == "SKIP") == ("reason" in lic) and lic["page"] == e["licence_url"],
              f"register: licences.toml does not fit register.json for {d['code']}")
        shape((e["verdict"] == "SKIP") == (d["status"] == "not_relayed"),
              f"register: verdict {e['verdict']} does not fit status {d['status']} for {d['code']}")
        if done.get(f"{d['source']}|{d['code']}") != e["sha256"]:
            todo.append((d, e, lic))
    if not todo:
        return [], state
    lines = [] if state.get("class") else _class_lines(sec, cls)
    cells = []
    for d, e, lic in todo:
        cells += _cell_lines(sec, cls, d, e, lic, blobs)
        done[f"{d['source']}|{d['code']}"] = e["sha256"]
    tests = [_test_line(c, blobs) for c in cells]
    new_state = dict(state)
    new_state["class"] = True
    new_state["done"] = dict(sorted(done.items()))
    return lines + cells + tests, new_state


def run(names, *, path, now, state_dir, out_dir, publish=None):
    state_path = Path(state_dir) / f"{KEY}.json"
    state = common.load_state(state_path)
    blobs = {}
    lines, state = emit(parse(Path(path).read_bytes()), state, names, blobs=blobs)
    if not lines:
        return None
    (Path(out_dir) / "blobs").mkdir(exist_ok=True)
    for rel, data in blobs.items():  # content-addressed: the same bytes under the same name, written once
        target = Path(out_dir) / rel
        if not target.exists():
            target.write_bytes(data)
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
