"""common.http_get retries transient network failures (4 times, after 5, 15, 45 and 120 s); 404 and 429/503 keep their
own paths and other HTTP errors are not retried."""
import http.client
import socket
import urllib.error
import urllib.request

import pytest

import common


class Opener:
    """A fake urlopen: each call takes the next step: bytes (an answer), an exception (raised by the call), or
    ("read", exception) (the answer's read() raises)."""

    def __init__(self, *steps):
        self.steps, self.calls, self.requests = list(steps), 0, []

    def __call__(self, req, timeout=None):
        self.requests.append((req, timeout))
        step = self.steps[min(self.calls, len(self.steps) - 1)]
        self.calls += 1
        if isinstance(step, BaseException):
            raise step
        if isinstance(step, tuple):
            return Resp(step[1])
        return Resp(step)


class Resp:
    def __init__(self, data):
        self.data = data

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False

    def read(self):
        if isinstance(self.data, BaseException):
            raise self.data
        return self.data


def get(monkeypatch, opener):
    monkeypatch.setattr(urllib.request, "urlopen", opener)
    sleeps = []
    return lambda: common.http_get("https://example.org/x", sleep=sleeps.append), sleeps


def http_error(code):
    return urllib.error.HTTPError("https://example.org/x", code, "x", {}, None)


def test_the_waits_are_5_15_45_120():
    assert common.RETRY_WAITS == (5.0, 15.0, 45.0, 120.0)


def test_a_connection_reset_is_retried_and_the_answer_returned(monkeypatch, capsys):
    op = Opener(urllib.error.URLError(ConnectionResetError(104, "Connection reset by peer")),
                urllib.error.URLError(ConnectionResetError(104, "Connection reset by peer")), b"data")
    call, sleeps = get(monkeypatch, op)
    assert call() == b"data"
    assert sleeps == [5.0, 15.0] and op.calls == 3
    err = capsys.readouterr().err
    assert "retry 1 of 4" in err and "retry 2 of 4" in err and "Connection reset" in err


def test_after_four_retries_the_error_is_raised_as_before(monkeypatch):
    err = urllib.error.URLError(ConnectionResetError(104, "Connection reset by peer"))
    op = Opener(err)
    call, sleeps = get(monkeypatch, op)
    with pytest.raises(urllib.error.URLError) as e:
        call()
    assert e.value is err and sleeps == [5.0, 15.0, 45.0, 120.0] and op.calls == 5


@pytest.mark.parametrize("failure", [
    urllib.error.URLError(ConnectionRefusedError(111, "refused")),
    urllib.error.URLError("temporary failure in name resolution"),
    urllib.error.URLError(socket.timeout("timed out")),
    ConnectionResetError(104, "reset while reading"),
    TimeoutError("timed out"),
    socket.timeout("timed out"),
    http.client.RemoteDisconnected("Remote end closed connection without response"),
    http.client.IncompleteRead(b"par", 100),
    http_error(502),
    http_error(504),
])
def test_transient_failures_are_retried(failure, monkeypatch):
    call, sleeps = get(monkeypatch, Opener(failure, b"ok"))
    assert call() == b"ok" and sleeps == [5.0]


def test_a_failure_while_reading_the_body_is_retried_too(monkeypatch):
    call, sleeps = get(monkeypatch, Opener(("read", ConnectionResetError(104, "reset")), b"whole"))
    assert call() == b"whole" and sleeps == [5.0]


@pytest.mark.parametrize("code", [400, 401, 403, 410, 500])
def test_other_http_errors_are_not_retried(code, monkeypatch):
    op = Opener(http_error(code))
    call, sleeps = get(monkeypatch, op)
    with pytest.raises(urllib.error.HTTPError) as e:
        call()
    assert e.value.code == code and op.calls == 1 and sleeps == []


def test_404_is_not_published_and_429_503_are_rate_limited_without_these_retries(monkeypatch):
    op = Opener(http_error(404))
    call, sleeps = get(monkeypatch, op)
    with pytest.raises(common.NotPublished):
        call()
    assert op.calls == 1 and sleeps == []
    for code in (429, 503):
        op = Opener(http_error(code))
        call, sleeps = get(monkeypatch, op)
        with pytest.raises(common.RateLimited) as e:
            call()
        assert e.value.code == code and op.calls == 1 and sleeps == []


def test_a_404_after_a_reset_is_still_not_published(monkeypatch):
    op = Opener(urllib.error.URLError(ConnectionResetError(104, "reset")), http_error(404))
    call, sleeps = get(monkeypatch, op)
    with pytest.raises(common.NotPublished):
        call()
    assert sleeps == [5.0] and op.calls == 2


def test_the_request_is_the_same_each_time(monkeypatch):
    op = Opener(TimeoutError("t"), b"x")
    call, _ = get(monkeypatch, op)
    call()
    (r1, t1), (r2, t2) = op.requests
    assert r1.full_url == r2.full_url == "https://example.org/x" and t1 == t2 == 180
    assert r1.get_header("User-agent") == r2.get_header("User-agent") == common.USER_AGENT


def test_the_default_sleep_is_time_sleep(monkeypatch):
    import time
    slept = []
    monkeypatch.setattr(time, "sleep", slept.append)
    monkeypatch.setattr(urllib.request, "urlopen", Opener(TimeoutError("t"), b"x"))
    assert common.http_get("https://example.org/x") == b"x" and slept == [5.0]
