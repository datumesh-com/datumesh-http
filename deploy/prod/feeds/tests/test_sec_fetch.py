"""Fair access (K1, D34): the declared User-Agent and nothing else about us, a token bucket at 5 requests a second,
a stop on 403, and no production host."""
import io
import urllib.error
import urllib.request

import pytest

import common
import sec
from sec_helpers import FakeClock, max_in_any_second

UA = "Datumesh mesh-test relay (https://datumesh.com)"
URL = "https://data.sec.gov/api/xbrl/companyfacts/CIK0000018230.json"
SEC_403 = (b"<html><head><title>SEC.gov | Request Rate Threshold Exceeded</title></head><body>Your request has been "
           b"identified as part of a network of automated tools outside of the acceptable policy and will be managed "
           b"until action is taken to declare your traffic.</body></html>")


class Resp:
    def __init__(self, data):
        self.data = data

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False

    def read(self):
        return self.data


class Opener:
    def __init__(self, *steps):
        self.steps, self.requests = list(steps), []

    def __call__(self, req, timeout=None):
        self.requests.append((req, timeout))
        step = self.steps[min(len(self.requests) - 1, len(self.steps) - 1)]
        if isinstance(step, BaseException):
            raise step
        return Resp(step)


def http_error(code, body=b""):
    return urllib.error.HTTPError(URL, code, "x", {}, io.BytesIO(body))


# ---------- the declared identity ----------

def test_user_agent_is_exactly_the_declared_one_and_has_no_email():
    assert sec.USER_AGENT == UA
    assert "@" not in sec.USER_AGENT and "amin" not in sec.USER_AGENT.lower() and "bentoak" not in sec.USER_AGENT.lower()
    assert sec.USER_AGENT != common.USER_AGENT, "the SEC sees the declared identity, not the generic feeds one"


def test_a_request_carries_the_user_agent_and_no_other_identifying_header(monkeypatch):
    op = Opener(b"{}")
    monkeypatch.setattr(urllib.request, "urlopen", op)
    assert sec.http_get(URL) == b"{}"
    (req, timeout), = op.requests
    assert req.full_url == URL and req.get_method() == "GET" and req.data is None
    headers = {k.lower(): v for k, v in req.header_items()}
    assert headers["user-agent"] == UA
    assert set(headers) <= {"user-agent", "accept-encoding", "accept"}
    for v in headers.values():
        assert "@" not in v
    assert timeout is not None and timeout <= 180


# ---------- 403: stop, say so, never retry harder ----------

def test_403_raises_sec_refused_with_the_secs_words_and_is_not_retried(monkeypatch):
    op = Opener(http_error(403, SEC_403))
    monkeypatch.setattr(urllib.request, "urlopen", op)
    with pytest.raises(sec.SecRefused) as e:
        sec.http_get(URL)
    assert len(op.requests) == 1, "never retried"
    assert e.value.status == 403 and e.value.url == URL and e.value.body == SEC_403.decode()
    assert "automated tools" in str(e.value) and "403" in str(e.value) and URL in str(e.value)
    assert not isinstance(e.value, common.RateLimited)


def test_429_and_503_are_rate_limited_not_retried_and_404_is_not_published(monkeypatch):
    for code in (429, 503):
        op = Opener(http_error(code))
        monkeypatch.setattr(urllib.request, "urlopen", op)
        with pytest.raises(common.RateLimited):
            sec.http_get(URL)
        assert len(op.requests) == 1
    op = Opener(http_error(404))
    monkeypatch.setattr(urllib.request, "urlopen", op)
    with pytest.raises(common.NotPublished):
        sec.http_get(URL)
    assert len(op.requests) == 1


def test_only_https_sec_hosts_are_fetched(monkeypatch):
    op = Opener(b"{}")
    monkeypatch.setattr(urllib.request, "urlopen", op)
    for bad in ("http://data.sec.gov/x.json", "https://example.org/x.json", "https://sec.gov.evil.org/x", "file:///etc/passwd"):
        with pytest.raises(ValueError):
            sec.http_get(bad)
    assert op.requests == []
    sec.http_get("https://www.sec.gov/files/company_tickers_exchange.json")
    sec.http_get("https://data.sec.gov/api/xbrl/frames/us-gaap/Assets/USD/CY2023Q4I.json")
    assert len(op.requests) == 2


# ---------- the token bucket ----------

def test_the_rate_is_5_a_second():
    assert sec.MAX_REQUESTS_PER_SECOND == 5


def test_bucket_never_lets_more_than_5_requests_into_any_second():
    fc = FakeClock()
    b = sec.TokenBucket(5, clock=fc.clock, sleep=fc.sleep)
    times = []
    for _ in range(60):
        b.wait()
        times.append(fc.clock())
    assert max_in_any_second(times) <= 5
    assert fc.clock() - 1000.0 >= 59 / 5 - 1e-9, "60 requests take at least 11.8 s at 5 a second"
    assert fc.clock() - 1000.0 <= 59 / 5 + 0.5, "and it does not wait much longer than it must"


def test_bucket_allows_no_burst_beyond_one_token():
    fc = FakeClock()
    b = sec.TokenBucket(5, clock=fc.clock, sleep=fc.sleep)
    t0 = fc.clock()
    b.wait(), b.wait()
    assert fc.clock() - t0 >= 0.2 - 1e-9


def test_bucket_does_not_sleep_when_the_caller_was_slow_and_does_not_bank_tokens():
    fc = FakeClock()
    sleeps = []
    b = sec.TokenBucket(5, clock=fc.clock, sleep=lambda s: (sleeps.append(s), fc.sleep(s)))
    b.wait()
    fc.now += 30.0  # an idle half minute
    b.wait()
    assert sleeps == [], "idle time is not waited for"
    times = []
    for _ in range(10):
        b.wait()
        times.append(fc.clock())
    assert max_in_any_second(times) <= 5, "idle time is not banked into a burst"


def test_bucket_rate_is_a_parameter():
    fc = FakeClock()
    b = sec.TokenBucket(1, clock=fc.clock, sleep=fc.sleep)
    t0 = fc.clock()
    for _ in range(4):
        b.wait()
    assert fc.clock() - t0 >= 3 - 1e-9


def test_a_rate_above_the_secs_limit_is_refused():
    for bad in (0, -1, 10.5, 11):
        with pytest.raises(ValueError):
            sec.TokenBucket(bad)
    sec.TokenBucket(10)  # the SEC's own maximum is allowed to construct; the job uses 5


# ---------- the host ----------

def test_production_host_is_refused_by_the_cli_before_any_fetch(monkeypatch, tmp_path, capsys):
    called = []
    monkeypatch.setattr(urllib.request, "urlopen", lambda *a, **k: called.append(a))
    s, o = tmp_path / "s", tmp_path / "o"
    s.mkdir(), o.mkdir()
    with pytest.raises(SystemExit) as e:
        sec.main(["--host", "datumesh.com", "--state-dir", str(s), "--out-dir", str(o)])
    assert e.value.code not in (0, None)
    assert called == [] and list(o.iterdir()) == [] and list(s.iterdir()) == []
    err = capsys.readouterr().err
    assert "datumesh.com" in err and "mesh-test" in err


def test_unknown_hosts_are_refused_too(tmp_path):
    with pytest.raises(SystemExit):
        sec.main(["--host", "example.org", "--state-dir", str(tmp_path), "--out-dir", str(tmp_path)])
    with pytest.raises(SystemExit):
        sec.main(["--state-dir", str(tmp_path), "--out-dir", str(tmp_path)])  # no default host


def test_run_refuses_the_production_names(tmp_path):
    prod = common.names("datumesh.com")
    s, o = tmp_path / "s", tmp_path / "o"
    s.mkdir(), o.mkdir()

    def fetch(url):
        raise AssertionError("fetched " + url)
    with pytest.raises(sec.NotForProduction):
        sec.run(prod, fetch=fetch, now="2026-11-20T12:00:00.000000Z", state_dir=s, out_dir=o)
    assert issubclass(sec.NotForProduction, ValueError)


def test_production_host_table_has_no_part_7_names(names):
    prod = common.names("datumesh.com")
    assert "sec" not in prod, "K7: nothing for Part 7 in the datumesh.com table; the section is rehearsal-only"
    assert prod["meta"]["status"] == "production"
    assert names["sec"]["namespace"] == "mesh-test.datumesh.com/relay/sec.gov"
