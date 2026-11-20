"""series.py: the series file of the month pages (Part 6, G2), written from names.toml for one host."""
import importlib
import json
import re

import pytest

import common
import series
from conftest import FEEDS, PRODUCTION, REHEARSAL

HOSTS = [REHEARSAL, PRODUCTION]


def expected(host):
    relay = f"{host}/relay"
    return [
        {"function": f"{relay}/ecb.europa.eu/fx_reference", "granularity": "month", "date_arg": "date",
         "column_arg": "currency", "unit": "units of the currency per 1 EUR", "kind": "relayed"},
        {"function": f"{relay}/ecb.europa.eu/estr", "granularity": "month", "date_arg": "date",
         "unit": "percent", "kind": "relayed"},
        {"function": f"{relay}/bankofengland.co.uk/bank_rate", "granularity": "month", "date_arg": "date",
         "unit": "percent", "kind": "relayed"},
        {"function": f"{relay}/treasury.gov/par_yield", "granularity": "month", "date_arg": "date",
         "column_arg": "tenor", "unit": "percent", "kind": "relayed"},
        {"function": f"{relay}/ecb.europa.eu/policy_rate", "granularity": "year", "date_arg": "date",
         "column_arg": "rate", "unit": "percent", "kind": "relayed"},
        {"function": f"{host}/earth/monthly_rainfall", "granularity": "year", "date_arg": "month",
         "column_arg": "region", "fixed": {"dataset": "chirps"}, "unit": "mm", "kind": "computed",
         "source": "CHIRPS v3"},
    ]


def written(host, tmp_path, name="series.json"):
    out = tmp_path / name
    assert series.main(["--host", host, "--out", str(out)]) == 0
    return out


@pytest.mark.parametrize("host", HOSTS)
def test_the_exact_file(host, tmp_path):
    out = written(host, tmp_path)
    assert json.loads(out.read_text(encoding="utf-8")) == expected(host)
    assert out.read_text(encoding="utf-8") == json.dumps(expected(host), indent=2, ensure_ascii=False) + "\n"
    assert series.build(common.names(host)) == expected(host)


@pytest.mark.parametrize("host", HOSTS)
def test_every_function_is_one_of_names_toml_with_its_own_arguments(host):
    names = common.names(host)
    by_fq = {f"{s['namespace']}/{s['function']}": k for k, s in names.items() if isinstance(s, dict) and "function" in s}
    for entry in expected(host):
        key = by_fq[entry["function"]]  # a KeyError: a function that is not in names.toml
        assert entry["date_arg"] == ("month" if key == "chirps" else "date")
        if key != "chirps":
            mod = importlib.import_module(key)
            assert entry.get("column_arg") == (mod.KEYCOLS[0] if mod.KEYCOLS else None)
        else:
            assert entry["column_arg"] == "region" and entry["fixed"] == {"dataset": names["chirps"]["dataset"]}
    assert len({e["function"] for e in expected(host)}) == 6


@pytest.mark.parametrize("host", HOSTS)
def test_no_production_name_unless_the_host_is_production(host, tmp_path):
    text = written(host, tmp_path).read_text(encoding="utf-8")
    functions = [e["function"] for e in json.loads(text)]
    assert all(f.startswith(host + "/") for f in functions)
    if host == REHEARSAL:
        assert not re.search(r"(?<![A-Za-z0-9.-])datumesh\.com", text)
    else:
        assert "mesh-test" not in text


@pytest.mark.parametrize("host", HOSTS)
def test_no_scripture_register_or_test_cells(host, tmp_path):
    text = written(host, tmp_path).read_text(encoding="utf-8").lower()
    for word in ("scripture", "register", "test_case", "edition", "verse"):
        assert word not in text
    kinds = {e["kind"] for e in json.loads(text)}
    assert kinds == {"relayed", "computed"}
    for e in json.loads(text):
        assert ("source" in e) == (e["kind"] == "computed")


def test_a_host_must_be_given_and_known(tmp_path):
    out = tmp_path / "s.json"
    for args in ([], ["--out", str(out)], ["--host", REHEARSAL], ["--host", "example.com", "--out", str(out)],
                 ["--host", "DATUMESH.COM", "--out", str(out)]):
        with pytest.raises(SystemExit) as e:
            series.main(args)
        assert e.value.code not in (0, None)
    assert list(tmp_path.iterdir()) == []


def test_the_file_is_replaced_whole_and_nothing_is_left_beside_it(tmp_path):
    out = tmp_path / "series.mesh-test.datumesh.com.json"
    out.write_text("[]", encoding="utf-8")
    written(REHEARSAL, tmp_path, out.name)
    assert json.loads(out.read_text()) == expected(REHEARSAL)
    first = out.read_bytes()
    written(REHEARSAL, tmp_path, out.name)
    assert out.read_bytes() == first and [p.name for p in tmp_path.iterdir()] == [out.name]


def test_the_series_are_in_names_toml_not_in_the_script():
    text = (FEEDS / "series.py").read_text()
    for word in ("fx_reference", "par_yield", "monthly_rainfall", "bank_rate", "policy_rate", "ecb.europa.eu", "chirps",
                 "mesh-test", "datumesh.com"):
        assert word not in text.replace('"""', "")


def test_the_job_script_and_readme_say_how_to_make_and_use_the_file():
    sh = (FEEDS / "run_feed.sh").read_text()
    assert re.search(r"ecb_fx\|estr\|policy_rates\|treasury\|boe\|chirps\|series\)", sh)
    assert 'series.py --host "$host" --out "/data/series.$host.json"' in sh
    readme = (FEEDS / "README.md").read_text()
    for must in ("series.<host>.json", "/srv/datumesh/feeds/series.", "DATUMESH_SERIES_FILE", "read-only",
                 "run_feed.sh series"):
        assert must in readme, must
