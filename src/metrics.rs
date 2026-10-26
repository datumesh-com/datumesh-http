//! Counters and latency histograms, written as Prometheus text by `GET /metrics`. Labels
//! are routes (templates, never paths), outcomes and statuses: no principal, IP or job id.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

/// Upper bounds of the latency buckets, in seconds.
pub(crate) const BUCKETS: &[f64] =
    &[0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0];

#[derive(Default)]
struct Hist {
    counts: Vec<u64>,
    sum: f64,
    n: u64,
}

#[derive(Default)]
pub(crate) struct Metrics {
    requests: Mutex<BTreeMap<(String, u16), u64>>,
    outcomes: Mutex<BTreeMap<(String, String), u64>>,
    latency: Mutex<BTreeMap<String, Hist>>,
}

impl Metrics {
    pub(crate) fn record(&self, route: &str, status: u16, elapsed: Duration) {
        *self.requests.lock().unwrap_or_else(|p| p.into_inner()).entry((route.to_string(), status)).or_default() += 1;
        let mut l = self.latency.lock().unwrap_or_else(|p| p.into_inner());
        let h = l.entry(route.to_string()).or_default();
        if h.counts.is_empty() {
            h.counts = vec![0; BUCKETS.len()];
        }
        let s = elapsed.as_secs_f64();
        for (i, b) in BUCKETS.iter().enumerate() {
            if s <= *b {
                h.counts[i] += 1;
            }
        }
        h.sum += s;
        h.n += 1;
    }

    pub(crate) fn outcome(&self, route: &str, outcome: &str) {
        *self
            .outcomes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry((route.to_string(), outcome.to_string()))
            .or_default() += 1;
    }

    /// The requests counted for `route` (any status).
    pub(crate) fn requests_of(&self, route: &str) -> u64 {
        self.requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|((r, _), _)| r == route)
            .map(|(_, n)| n)
            .sum()
    }

    pub(crate) fn write(&self, out: &mut String) {
        use std::fmt::Write;
        let _ = writeln!(out, "# HELP datumesh_http_requests_total Requests answered, by route and HTTP status.");
        let _ = writeln!(out, "# TYPE datumesh_http_requests_total counter");
        for ((r, s), n) in self.requests.lock().unwrap_or_else(|p| p.into_inner()).iter() {
            let _ = writeln!(out, "datumesh_http_requests_total{{route=\"{r}\",status=\"{s}\"}} {n}");
        }
        let _ = writeln!(out, "# HELP datumesh_envelopes_total Envelopes served, by route and outcome.");
        let _ = writeln!(out, "# TYPE datumesh_envelopes_total counter");
        for ((r, o), n) in self.outcomes.lock().unwrap_or_else(|p| p.into_inner()).iter() {
            let _ = writeln!(out, "datumesh_envelopes_total{{route=\"{r}\",outcome=\"{o}\"}} {n}");
        }
        let _ = writeln!(out, "# HELP datumesh_http_request_seconds Request latency, by route.");
        let _ = writeln!(out, "# TYPE datumesh_http_request_seconds histogram");
        for (r, h) in self.latency.lock().unwrap_or_else(|p| p.into_inner()).iter() {
            for (i, b) in BUCKETS.iter().enumerate() {
                let _ =
                    writeln!(out, "datumesh_http_request_seconds_bucket{{route=\"{r}\",le=\"{b}\"}} {}", h.counts[i]);
            }
            let _ = writeln!(out, "datumesh_http_request_seconds_bucket{{route=\"{r}\",le=\"+Inf\"}} {}", h.n);
            let _ = writeln!(out, "datumesh_http_request_seconds_sum{{route=\"{r}\"}} {}", h.sum);
            let _ = writeln!(out, "datumesh_http_request_seconds_count{{route=\"{r}\"}} {}", h.n);
        }
    }
}
