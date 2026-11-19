"""The relay series (ecb_fx, estr, policy_rates, treasury, boe): stored cells, catalogue, conformance, run.

A module supplies its parsers and its TEXT (labels, summary wording, dictionary); this file does the rest.
Names come only from names.toml (through the section `key`)."""
from decimal import Decimal
from pathlib import Path
from urllib.parse import urlparse

import common
from common import ISO_DATE as _ISO
from common import ConformanceError, decimal_text, load_state, save_state, sha256_hex, write_jsonl


def fq_of(sec: dict) -> str:
    return f"{sec['namespace']}/{sec['function']}"


def call_text(fq: str, argnames, values: dict) -> str:
    parts = []
    for a in argnames:
        v = values[a]
        parts.append(f"{a}: {v}" if _ISO.match(v) else f'{a}: "{v}"')
    return f"{fq}({', '.join(parts)})"


def _key(r, keycols):
    return tuple(r[c] for c in keycols) + (r["date"],)


def tail(sec: dict) -> str:
    return f"; relayed by datumesh.com from {sec['source_name']} (takeover: {sec['takeover']})"


def function_line(fq, keycols, unit):
    args = {k: {"type": "string", "valid": False} for k in keycols}
    args["date"] = {"type": "date", "valid": True}
    return {"function": fq, "signature": {"args": args, "order": keycols + ["date"],
                                          "returns": {"type": "number", "unit": unit}}}


def catalog_line(sec, fq, keycols, rows, text):
    argnames = keycols + ["date"]
    ordered = sorted(rows, key=lambda r: (r["date"],) + tuple(r[k] for k in keycols))
    examples = []
    for r in ordered[-2:][::-1]:
        vals = {a: r[a] for a in argnames}
        examples.append({"call": call_text(fq, argnames, vals),
                         "expected": {"type": "number", "value": decimal_text(r["value"])}})
    return {"catalog": fq, "kind": "function", "label": text["label"],
            "description": f"Relayed by datumesh.com from {sec['source_name']}: {text['description']}",
            "args": dict(text["args"]), "examples": examples, "dictionary": text["dictionary"](rows)}


def cell_line(sec, fq, keycols, r, source, text):
    args = {k: {"type": "string", "value": r[k]} for k in keycols}
    args["date"] = {"type": "date", "value": r["date"]}
    value = decimal_text(r["value"])
    return {
        "cell": {"fn": fq, "args": args},
        "value": {"type": "number", "value": value},
        "licence": sec["licence"],
        "summary": common.nfc(text["summary"](sec, r, value)),
        "source": {"name": sec["source_name"], "url": source["url"], "retrieved": source["retrieved"],
                   "sha256": source["sha256"], "takeover": sec["takeover"]},
    }


def conformance(key, rows, second_rows, names, keycols):
    """One test per key and date present in both copies; a disagreement as numbers is refused."""
    sec = names[key]
    fq = fq_of(sec)
    argnames = keycols + ["date"]
    second = {_key(r, keycols): r["value"] for r in second_rows}
    tests = []
    for r in rows:
        k = _key(r, keycols)
        if k not in second:
            continue
        if Decimal(r["value"]) != Decimal(second[k]):
            raise ConformanceError(f"{fq} {k}: {r['value']} against {second[k]} in the second copy")
        tests.append({"test": fq, "call": call_text(fq, argnames, dict(zip(argnames, k))),
                      "expect": {"type": "number", "value": decimal_text(second[k])}})
    return tests


def advance(state, new, keycols, per_key):
    st = dict(state)
    if per_key:
        ld = dict(st.get("last_date", {}))
        for r in new:
            k = r[keycols[0]]
            ld[k] = max(ld.get(k, ""), r["date"])
        st["last_date"] = ld
    else:
        st["last_date"] = max([st.get("last_date", "")] + [r["date"] for r in new])
    return st


def emit(key, rows, state, names, source, *, keycols, per_key, text, second_rows=None):
    """Lines for the rows after the state, and the new state. The first call (empty state) also emits the
    function and catalogue lines and, when given a second copy, the conformance tests."""
    sec = names[key]
    fq = fq_of(sec)
    first = "last_date" not in state

    def seen(r):
        last = state.get("last_date", {}).get(r[keycols[0]], "") if per_key else state.get("last_date", "")
        return r["date"] <= last

    new = [r for r in rows if not seen(r)]
    if not new:
        return [], state
    lines = []
    if first:
        lines.append(function_line(fq, keycols, sec["unit"]))
        lines.append(catalog_line(sec, fq, keycols, rows, text))
    src = {"url": source["url"], "retrieved": source["retrieved"], "sha256": source["sha256"]}
    lines += [cell_line(sec, fq, keycols, r, src, text) for r in new]
    if first and second_rows is not None:
        lines += conformance(key, rows, second_rows, names, keycols)
    return lines, advance(state, new, keycols, per_key)


def _stamp(now: str) -> str:
    return now.replace(":", "").replace("-", "")


def run_series(key, names, *, fetch, now, state_dir, out_dir, inputs, second, emit_one, publish=None,
               keycols=(), per_key=False, conformance_fn=None):
    """Fetch, parse and check every input first (a ShapeError leaves no file and no state), emit, write one
    publish-stored file, then either publish it (state saved only after that returns) or save the state.

    inputs(state) -> [(url, parse)]; second -> None or (url, parse2) for the conformance copy;
    emit_one(rows, state, source) -> (lines, state)."""
    state_path = Path(state_dir) / f"{key}.json"
    state = load_state(state_path)
    first = "last_date" not in state
    parsed = []
    for url, parse in inputs(state):
        data = fetch(url)
        parsed.append((url, data, parse(data)))
    second_rows = None
    if second is not None and first:
        url2, parse2 = second
        second_rows = parse2(fetch(url2))
    lines = []
    for url, data, rows in parsed:
        src = {"url": url, "retrieved": now, "sha256": sha256_hex(data)}
        more, state = emit_one(rows, state, src)
        lines += more
    if second_rows is not None and first:
        lines += conformance(key, [r for _, _, rows in parsed for r in rows], second_rows, names, keycols)
    if not lines:
        return None
    path = Path(out_dir) / f"{key}-{_stamp(now)}.jsonl"
    write_jsonl(path, lines)
    if publish is not None:
        try:
            publish(path)
        except BaseException:
            path.unlink(missing_ok=True)
            raise
    save_state(state_path, state)
    return path


def host_of_url(url: str) -> str:
    return urlparse(url).hostname or ""
