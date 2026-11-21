"""F2 (CONTRACT-0.4.9): register.py emits `test` lines for the six methods of `register/dataset`.

Specified here: after the cells, one `{"test", "call", "expect"}` line per entry per method (8 x 6 = 48 on
the first run). `test` is the method's full name (`<host>/register/dataset/<method>`), `call` the method
called with `source` and `code`, and `expect` is exactly the `value` of the cell emitted for that call (for
`licence_clauses`, whose cell value is a blob file reference, the typed blob scalar of spec 3.7,
`{"type": "blob", "value": "<sha256>:<length>:text/plain"}`, which a test's `expect` can carry). A changed licence text emits the six new cells of that entry and six tests pinning them; a
rerun with unchanged text emits nothing."""
import copy
import json

import register as reg
from conftest import REHEARSAL, fixture
from helpers import call_text, check_order, split_lines

METHODS = ["name", "status", "licence_page", "licence_hash", "licence_clauses", "retrieved"]
CLS = f"{REHEARSAL}/register/dataset"


def entries():
    return reg.parse(fixture("register.json"))


def _split(lines):
    check_order([l for l in lines if "cell" in l or "test" in l])
    cells = [l for l in lines if "cell" in l]
    tests = [l for l in lines if "test" in l]
    assert lines[len(lines) - len(tests):] == tests, "the tests come last"
    return cells, tests


def _expected(value, blobs):
    """The typed scalar a test expects for a cell value: the value itself, or for a blob file reference the
    blob scalar `<sha256>:<length>:<media type>` (spec 3.7)."""
    if "blob" in value:
        sha = value["blob"].rsplit("/", 1)[1]
        return {"type": "blob", "value": f"{sha}:{len(blobs[value['blob']])}:{value['media_type']}"}
    return value


def _call(cell):
    a = cell["cell"]["args"]
    return call_text(cell["cell"]["fn"], ["source", "code"], {k: a[k]["value"] for k in ("source", "code")})


def test_one_test_per_entry_per_method(names):
    lines, _ = reg.emit(entries(), {}, names)
    cells, tests = _split(lines)
    assert len(cells) == 48 and len(tests) == 48
    for t in tests:
        assert set(t) == {"test", "call", "expect"}, t
        assert t["test"].startswith(CLS + "/") and t["test"][len(CLS) + 1:] in METHODS
        assert t["call"].startswith(t["test"] + "("), "the call names the method by its full name"
    assert len({t["call"] for t in tests}) == 48
    per_method = {m: sum(1 for t in tests if t["test"] == f"{CLS}/{m}") for m in METHODS}
    assert per_method == {m: 8 for m in METHODS}


def test_each_test_pins_the_value_the_cell_emitted(names):
    blobs = {}
    lines, _ = reg.emit(entries(), {}, names, blobs=blobs)
    cells, tests = _split(lines)
    by_call = {_call(c): c for c in cells}
    assert len(by_call) == 48
    assert {t["call"] for t in tests} == set(by_call)
    for t in tests:
        assert t["expect"] == _expected(by_call[t["call"]]["value"], blobs), t["call"]


def test_the_pinned_values_are_the_register_facts(names):
    blobs = {}
    lines, _ = reg.emit(entries(), {}, names, blobs=blobs)
    _, tests = _split(lines)
    assert len(tests) == 48
    sec = names["register"]
    inputs = {r["source_domain"]: r for r in entries()}
    exp = {t["call"]: t["expect"] for t in tests}
    for d in sec["datasets"]:
        r = inputs[d["input"]]
        pick = lambda m: exp[f'{CLS}/{m}(source: "{d["source"]}", code: "{d["code"]}")']
        assert pick("licence_hash") == {"type": "string", "value": r["sha256"]}
        assert pick("retrieved") == {"type": "instant", "value": r["retrieved"]}
        assert pick("licence_page") == {"type": "string", "value": r["licence_url"]}
        assert pick("name") == {"type": "string", "value": d["name"]}
        assert pick("status") == {"type": "string", "value": d["status"]}
        clauses = pick("licence_clauses")
        assert clauses["type"] == "blob" and clauses["value"].endswith(":text/plain")
        sha, length, _ = clauses["value"].split(":", 2)
        assert int(length) == len(blobs["blobs/" + sha])


def test_a_rerun_with_unchanged_licence_text_emits_no_tests_either(names):
    _, state = reg.emit(entries(), {}, names)
    state = json.loads(json.dumps(state))
    assert reg.emit(entries(), state, names) == ([], state)
    later = copy.deepcopy(entries())
    for r in later:
        r["retrieved"] = "2026-11-20T05:00:00.000000Z"
    assert reg.emit(later, state, names)[0] == []


def test_a_changed_text_emits_six_new_cells_and_six_tests_pinning_them(names):
    _, state = reg.emit(entries(), {}, names)
    state = json.loads(json.dumps(state))
    changed = copy.deepcopy(entries())
    (e,) = [r for r in changed if r["source_domain"] == "ecb.europa.eu/estr"]
    e["sha256"] = "ab" * 32
    e["retrieved"] = "2026-12-01T05:00:00.000000Z"
    blobs = {}
    lines, state2 = reg.emit(changed, state, names, blobs=blobs)
    cells, tests = _split(lines)
    assert len(cells) == 6 and len(tests) == 6
    assert {t["test"] for t in tests} == {f"{CLS}/{m}" for m in METHODS}
    by_call = {_call(c): c for c in cells}
    for t in tests:
        assert t["call"] in by_call and t["expect"] == _expected(by_call[t["call"]]["value"], blobs)
    hash_test = next(t for t in tests if t["test"].endswith("/licence_hash"))
    assert hash_test["expect"] == {"type": "string", "value": "ab" * 32}
    assert reg.emit(changed, json.loads(json.dumps(state2)), names)[0] == []


def test_run_writes_the_tests_into_the_file_then_nothing(names, tmp_path):
    state_dir, out_dir = tmp_path / "s", tmp_path / "o"
    state_dir.mkdir(), out_dir.mkdir()
    path = tmp_path / "register.json"
    path.write_bytes(fixture("register.json"))
    out = reg.run(names, path=path, now="2026-11-19T12:00:00.000000Z", state_dir=state_dir, out_dir=out_dir)
    lines = [json.loads(l) for l in out.read_text(encoding="utf-8").splitlines()]
    assert len([l for l in lines if "test" in l]) == 48
    assert len(lines) == 6 + 1 + 7 + 48 + 48
    assert reg.run(names, path=path, now="2026-11-19T12:00:00.000000Z", state_dir=state_dir, out_dir=out_dir) is None
