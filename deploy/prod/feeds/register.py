"""The licence register (F4): for each relayed source, the SHA-256 of its licence text and its licence page URL,
as stored cells under datumesh.com. Input: the register.json of the licence check (verdict RELAY entries only)."""
import json
from pathlib import Path
from urllib.parse import urlparse

import common
from common import INSTANT, SHA256, ShapeError, shape, write_jsonl

KEY = "register"
REQUIRED = {"source_domain", "licence_url", "retrieved", "sha256", "verdict", "spdx_or_url", "attribution",
            "conditions"}


def parse(data: bytes) -> list:
    try:
        entries = json.loads(common.decode(data))
    except json.JSONDecodeError as e:
        raise ShapeError(f"register does not parse: {e}") from e
    shape(isinstance(entries, list), "register: not a list")
    out = []
    for e in entries:
        shape(isinstance(e, dict), "register: an entry is not an object")
        missing = REQUIRED - set(e)
        shape(not missing, f"register: entry lacks {sorted(missing)}")
        shape(isinstance(e["sha256"], str) and SHA256.match(e["sha256"]), f"register: bad sha256 for {e['source_domain']}")
        shape(isinstance(e["retrieved"], str) and INSTANT.match(e["retrieved"]), "register: bad retrieved instant")
        shape(isinstance(e["verdict"], str), "register: verdict is not text")
        if e["verdict"] == "RELAY":
            out.append(e)
    shape(out, "register: no RELAY entries")
    return out


def _catalog(sec, fq, label, description, entries, returns_example):
    dictionary = {"source": [{"value": e["source_domain"], "label": e["attribution"] or e["source_domain"]}
                             for e in entries]}
    first = entries[0]
    return {"catalog": fq, "kind": "function", "label": label,
            "description": f"{description} Source: the register of datumesh.com, RELAY verdicts.",
            "args": {"source": "the source's domain, for example ecb.europa.eu/fx",
                     "retrieved": "the day the licence page was fetched"},
            "examples": [{"call": f'{fq}(source: "{first["source_domain"]}", retrieved: {first["retrieved"][:10]})',
                          "expected": {"type": "string", "value": returns_example(first)}}],
            "dictionary": dictionary}


def emit(entries, state, names):
    """-> (lines, state). The functions and catalogue come once (state key `done` absent); then two cells per
    entry not yet done: the licence text's SHA-256 and the licence page URL."""
    sec = names[KEY]
    ft = f"{sec['namespace']}/{sec['text_function']}"
    fp = f"{sec['namespace']}/{sec['page_function']}"
    done = set(state.get("done", []))
    todo = [e for e in entries if f"{e['source_domain']}|{e['retrieved']}" not in done]
    if not todo:
        return [], state
    lines = []
    first = "done" not in state
    args = {"source": {"type": "string", "valid": False}, "retrieved": {"type": "date", "valid": True}}
    if first:
        for fq in (ft, fp):
            lines.append({"function": fq, "signature": {"args": args, "order": ["source", "retrieved"],
                                                        "returns": "string"}})
        lines.append(_catalog(sec, ft, "Licence text SHA-256 (register)",
                              "SHA-256 of the licence text of a relayed source, as fetched on the day given.",
                              entries, lambda e: e["sha256"]))
        lines.append(_catalog(sec, fp, "Licence page (register)",
                              "The URL of the licence page of a relayed source, as fetched on the day given.",
                              entries, lambda e: e["licence_url"]))
    for e in todo:
        day = e["retrieved"][:10]
        cargs = {"source": {"type": "string", "value": e["source_domain"]}, "retrieved": {"type": "date", "value": day}}
        extra = f" Attribution: {e['attribution']}." if e["attribution"] else ""
        head = f"licence of {e['source_domain']} as fetched on {day} (verdict {e['verdict']}).{extra}"
        lines.append({"cell": {"fn": ft, "args": cargs}, "value": {"type": "string", "value": e["sha256"]},
                      "licence": sec["licence"], "summary": f"SHA-256 of the {head}",
                      "source": _source(e, e["licence_url"], e["sha256"])})
        lines.append({"cell": {"fn": fp, "args": cargs}, "value": {"type": "string", "value": e["licence_url"]},
                      "licence": sec["licence"], "summary": f"Licence page URL of the {head}",
                      "source": _source(e, e["licence_url"], e["sha256"])})
    new_state = dict(state)
    new_state["done"] = sorted(done | {f"{e['source_domain']}|{e['retrieved']}" for e in todo})
    return lines, new_state


def _source(e, url, sha):
    return {"name": urlparse(url).hostname, "url": url, "retrieved": e["retrieved"], "sha256": sha}


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
    return common.finish(run(common.names(), path=a.register, now=common.now_instant(),
                             state_dir=a.state_dir, out_dir=a.out_dir))


if __name__ == "__main__":
    raise SystemExit(main())
