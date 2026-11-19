"""The shell entry points take the host explicitly (no default), and keep state per host, so a rehearsal never
suppresses a production run."""
import subprocess

import pytest

from conftest import FEEDS, PRODUCTION, REHEARSAL


def sh(script, *args, env=None, cwd=None):
    return subprocess.run(["sh", str(FEEDS / script), *args], capture_output=True, text=True, env=env or {"PATH": "/usr/bin:/bin"},
                          cwd=cwd, timeout=30)


def test_run_feed_refuses_without_a_host_before_touching_anything(tmp_path):
    r = sh("run_feed.sh", "estr", env={"PATH": "/usr/bin:/bin", "DATUMESH_ROOT": str(tmp_path)})
    assert r.returncode == 2 and "FEEDS_HOST" in r.stderr
    assert list(tmp_path.iterdir()) == []


@pytest.mark.parametrize("bad", ["", "datumesh.com.", "DATUMESH.COM", "example.com", "prod"])
def test_run_feed_refuses_an_unknown_host(tmp_path, bad):
    r = sh("run_feed.sh", "estr", env={"PATH": "/usr/bin:/bin", "DATUMESH_ROOT": str(tmp_path), "FEEDS_HOST": bad})
    assert r.returncode == 2 and "FEEDS_HOST" in r.stderr
    assert list(tmp_path.iterdir()) == []


def test_run_feed_keeps_state_and_output_per_host():
    text = (FEEDS / "run_feed.sh").read_text()
    assert 'state="$feeds/state/$host"' in text and 'out="$feeds/out/$host"' in text
    assert '--host "$host"' in text


def test_install_cron_needs_a_host_and_prints_it():
    assert sh("install_cron.sh").returncode == 2
    assert sh("install_cron.sh", "example.com").returncode == 2
    for host in (REHEARSAL, PRODUCTION):
        r = sh("install_cron.sh", host)
        assert r.returncode == 0
        lines = [l for l in r.stdout.splitlines() if "run_feed.sh" in l and not l.startswith("#")]
        assert len(lines) == 6 and all(f"FEEDS_HOST={host} " in l for l in lines)
    other = sh("install_cron.sh", REHEARSAL).stdout
    assert "FEEDS_HOST=datumesh.com " not in other
