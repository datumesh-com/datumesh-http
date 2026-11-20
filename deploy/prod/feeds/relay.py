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


TEST_DATES = 20  # the tests of a run: the latest this many dates of each key


def _keystr(r, keycols):
    return "|".join(r[c] for c in keycols)


def _pairs(rows, second_rows, keycols, cap, after):
    """[(row, key tuple, key text)] for the keys and dates both copies have. With `cap`, only the latest `cap`
    dates of each key, and only dates after the key's high-water mark in `after`."""
    second = {_key(r, keycols) for r in second_rows}
    both = [(r, _key(r, keycols), _keystr(r, keycols)) for r in rows if _key(r, keycols) in second]
    if cap is None and not after:
        return both
    after = after or {}
    dates = {}
    for r, _, ks in both:
        if r["date"] > after.get(ks, ""):
            dates.setdefault(ks, set()).add(r["date"])
    keep = {(ks, d) for ks, ds in dates.items() for d in sorted(ds, reverse=True)[:cap]}
    return [(r, k, ks) for r, k, ks in both if (ks, r["date"]) in keep]


def conformance(key, rows, second_rows, names, keycols, *, cap=None, after=None):
    """One test per key and date present in both copies (the latest `cap` dates a key after the marks in `after`,
    when given); a disagreement as numbers among them is refused. In the order of `rows`."""
    sec = names[key]
    fq = fq_of(sec)
    argnames = keycols + ["date"]
    second = {_key(r, keycols): r["value"] for r in second_rows}
    tests = []
    for r, k, _ in _pairs(rows, second_rows, keycols, cap, after):
        if Decimal(r["value"]) != Decimal(second[k]):
            raise ConformanceError(f"{fq} {k}: {r['value']} against {second[k]} in the second copy")
        tests.append({"test": fq, "call": call_text(fq, argnames, dict(zip(argnames, k))),
                      "expect": {"type": "number", "value": decimal_text(second[k])}})
    return tests


def new_tests(key, rows, second_rows, names, keycols, state, *, tested_key="tested", cap=TEST_DATES):
    """-> (tests, state): the tests of the pairs not tested before (dates after each key's high-water mark in
    state[tested_key]), at most the latest `cap` dates a key, and the state with the marks moved up."""
    after = state.get(tested_key, {})
    pairs = _pairs(rows, second_rows, keycols, cap, after)
    if not pairs:
        return [], state
    tests = conformance(key, [r for r, _, _ in pairs], second_rows, names, keycols)
    marks = dict(after)
    for r, _, ks in pairs:
        marks[ks] = max(marks.get(ks, ""), r["date"])
    st = dict(state)
    st[tested_key] = marks
    return tests, st


def advance(state, new, keycols, per_key, cursor="last_date"):
    st = dict(state)
    if per_key:
        ld = dict(st.get(cursor, {}))
        for r in new:
            k = r[keycols[0]]
            ld[k] = max(ld.get(k, ""), r["date"])
        st[cursor] = ld
    else:
        st[cursor] = max([st.get(cursor, "")] + [r["date"] for r in new])
    return st


def emit(key, rows, state, names, source, *, keycols, per_key, text, second_rows=None, cursor="last_date",
         tested_key="tested"):
    """Lines for the rows after the state's `cursor`, the tests of the pairs a second copy newly gives, and the new
    state. The first call (no `last_date`, not `announced`) also emits the function and catalogue lines. A call with
    new tests and no new cells returns only the tests."""
    sec = names[key]
    fq = fq_of(sec)
    first = "last_date" not in state and not state.get("announced")

    def seen(r):
        last = state.get(cursor, {} if per_key else "")
        return r["date"] <= (last.get(r[keycols[0]], "") if per_key else last)

    new = [r for r in rows if not seen(r)]
    tests, st = ([], state) if second_rows is None else new_tests(key, rows, second_rows, names, keycols, state,
                                                                  tested_key=tested_key)
    if not new and not tests:
        return [], state
    lines = []
    if new:
        if first:
            lines.append(function_line(fq, keycols, sec["unit"]))
            lines.append(catalog_line(sec, fq, keycols, rows, text))
        src = {"url": source["url"], "retrieved": source["retrieved"], "sha256": source["sha256"]}
        lines += [cell_line(sec, fq, keycols, r, src, text) for r in new]
        st = advance(st, new, keycols, per_key, cursor)
        st["announced"] = True
    return lines + tests, st


def second_copy(second, fetch):
    """The rows of the second copy, or None (with a warning) when it cannot be had: a second copy that is down or
    has changed its shape does not stop the relay, it only means no new tests this run."""
    url2, parse2 = second
    try:
        return parse2(fetch(url2))
    except (common.ShapeError, OSError) as e:
        common.warn(f"second copy {url2} not used: {type(e).__name__}: {e}")
        return None


def _stamp(now: str) -> str:
    return now.replace(":", "").replace("-", "")


def run_series(key, names, *, fetch, now, state_dir, out_dir, inputs, second, emit_one, publish=None,
               keycols=(), per_key=False, conformance_fn=None):
    """Fetch, parse and check every input first (a ShapeError leaves no file and no state), emit, add the tests of
    the second copy (every run that has one), write one publish-stored file, then either publish it (state saved only
    after that returns) or save the state.

    inputs(state) -> [(url, parse)]; second -> None or (url, parse2) for the conformance copy;
    emit_one(rows, state, source) -> (lines, state)."""
    state_path = Path(state_dir) / f"{key}.json"
    state = load_state(state_path)
    parsed = []
    for url, parse in inputs(state):
        data = fetch(url)
        parsed.append((url, data, parse(data)))
    second_rows = second_copy(second, fetch) if second is not None else None
    lines = []
    for url, data, rows in parsed:
        src = {"url": url, "retrieved": now, "sha256": sha256_hex(data)}
        more, state = emit_one(rows, state, src)
        lines += more
    if second_rows is not None:
        tests, state = new_tests(key, [r for _, _, rows in parsed for r in rows], second_rows, names, keycols, state)
        lines += tests
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
