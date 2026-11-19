"""What each relay source must do on its fixture. Values are the sources' published decimals (see ../fixtures/SOURCES.md)."""
from conftest import fixture
from helpers import RETRIEVED

POLICY_RATES = ["mro", "dfr", "mlfr"]
POLICY_FILES = {"mro": "policy_MRR_FR.csv", "dfr": "policy_DFR.csv", "mlfr": "policy_MLFR.csv"}
TENORS = ["1M", "1.5M", "2M", "3M", "4M", "6M", "1Y", "2Y", "3Y", "5Y", "7Y", "10Y", "20Y", "30Y"]


def _rep(old: bytes, new: bytes, count: int = -1):
    return lambda b: b.replace(old, new, count)


# key: the section of names.toml and the module name (the same word)
SPECS = {
    "ecb_fx": dict(
        keycols=["currency"],
        inputs=lambda n: [("ecb_daily.xml", n["ecb_fx"]["url"])],
        nrows=29,
        present={("USD", "2026-11-18"): "1.0968", ("GBP", "2026-11-18"): "0.83290", ("HUF", "2026-11-18"): "368.40",
                 ("IDR", "2026-11-18"): "20020.71", ("ISK", "2026-11-18"): "137.00"},
        absent=[("USD", "2026-11-17"), ("EUR", "2026-11-18")],
        mutations=[_rep(b"currency=", b"ccy="), _rep(b"rate='1.0968'", b"rate='abc'"),
                   _rep(b"<Cube time=", b"<Cube day="), lambda b: b"", lambda b: b[: len(b) // 2]],
        dictionary={"currency": {"USD": "US dollar", "JPY": "Japanese yen", "GBP": "Pound sterling"}},
        dictionary_complete=None,
    ),
    "estr": dict(
        keycols=[],
        inputs=lambda n: [("estr.csv", n["estr"]["url"])],
        nrows=8,
        present={("2026-11-09",): "2.442", ("2026-11-11",): "2.439", ("2026-11-16",): "2.44", ("2026-11-18",): "2.442"},
        absent=[("2026-11-14",), ("2026-11-15",)],
        mutations=[_rep(b"OBS_VALUE", b"OBS_VAL"), _rep(b"TIME_PERIOD", b"PERIOD"), _rep(b"2.442", b"abc", 1),
                   _rep(b"EST.B.EU000A2X2A25.WT", b"EST.B.OTHER.WT"), lambda b: b""],
        dictionary={},
        dictionary_complete=None,
    ),
    "policy_rates": dict(
        keycols=["rate"],
        inputs=lambda n: [(POLICY_FILES[r], n["policy_rates"]["urls"][r]) for r in POLICY_RATES],
        nrows=18,
        present={("mro", "2025-02-05"): "2.9", ("mro", "2026-09-16"): "2.65", ("dfr", "2025-06-11"): "2",
                 ("dfr", "2026-06-17"): "2.25", ("mlfr", "2025-03-12"): "2.9", ("mlfr", "2026-09-16"): "2.9"},
        absent=[("mro", "2026-01-01"), ("dfr", "2026-09-17")],
        mutations=[_rep(b"OBS_VALUE", b"OBS_VAL"), _rep(b"TIME_PERIOD", b"PERIOD"),
                   _rep(b"FM.B.U2.EUR.4F.KR.", b"FM.B.U2.EUR.4F.ZZ."), _rep(b",2.", b",abc.", 1), lambda b: b""],
        dictionary={"rate": {"mro": "main refinancing operations rate"}},
        dictionary_complete={"rate": POLICY_RATES},
    ),
    "treasury": dict(
        keycols=["tenor"],
        inputs=lambda n: [("treasury_202611.xml", n["treasury"]["url"].format(month="202611"))],
        nrows=168,
        present={("1M", "2026-11-18"): "4.10", ("10Y", "2026-11-18"): "5.07", ("30Y", "2026-11-18"): "5.41",
                 ("1.5M", "2026-11-02"): "4.09", ("2Y", "2026-11-09"): "4.56"},
        absent=[("1M", "2026-11-11"), ("10Y", "2026-11-14")],
        mutations=[_rep(b"d:BC_10YEAR", b"d:BC_10YR"), _rep(b">4.05<", b">abc<", 1), _rep(b"NEW_DATE", b"NEWDATE"),
                   lambda b: b"", lambda b: b[: len(b) // 2]],
        dictionary={"tenor": {"1M": "1-month", "10Y": "10-year"}},
        dictionary_complete={"tenor": TENORS},
    ),
    "boe": dict(
        keycols=[],
        inputs=lambda n: [("boe_iudbedr.csv", n["boe"]["url"].format(**{"from": "02/Nov/2026", "to": "10/Nov/2026"}))],
        nrows=7,
        present={("2026-11-02",): "3.75", ("2026-11-04",): "3.75", ("2026-11-05",): "4", ("2026-11-10",): "4"},
        absent=[("2026-11-07",), ("2026-11-08",)],
        mutations=[_rep(b"IUDBEDR", b"XYZ"), _rep(b"05 Nov 2026", b"2026-11-05"), _rep(b"3.75", b"abc", 1),
                   _rep(b"DATE", b"DAY"), lambda b: b""],
        dictionary={},
        dictionary_complete=None,
    ),
}
