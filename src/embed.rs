//! The embedding client (Build 2 Part 2, R2): `POST <url>/embed` with `{"inputs": [...]}` to the
//! `datumesh-embed` service (Text Embeddings Inference serving `intfloat/multilingual-e5-small`).
//! Queries take the `query: ` prefix and indexed text the `passage: ` prefix; the vectors come back
//! normalised. A query has a 2 s timeout; any failure is an [`EmbedError`], which `find` answers
//! as `"vector": false`. Indexing ([`EmbedClient::embed_passages`]) has [`INDEX_TIMEOUT`]: a batch
//! of passages on the service's one CPU takes over 2 s (since 0.4.1).

use std::time::Duration;

/// The model the service serves, and the name its embeddings are stored under.
pub const MODEL: &str = "intfloat/multilingual-e5-small";

/// The dimension of the model's vectors.
pub const DIM: usize = 384;

/// The time limit of one query call.
pub const TIMEOUT: Duration = Duration::from_secs(2);

/// The time limit of one indexing call (since 0.4.1).
pub const INDEX_TIMEOUT: Duration = Duration::from_secs(60);

/// Why an embedding call failed (transport, status, or a malformed answer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedError(pub String);

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "embed: {}", self.0)
    }
}

impl std::error::Error for EmbedError {}

/// A client of the embedding service at one base URL.
pub struct EmbedClient {
    url: String,
    agent: ureq::Agent,
    index_agent: ureq::Agent,
}

impl std::fmt::Debug for EmbedClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbedClient").field("url", &self.url).finish_non_exhaustive()
    }
}

impl EmbedClient {
    /// A client for `url` (its trailing slash removed), with the 2 s timeout for queries and
    /// [`INDEX_TIMEOUT`] for indexing.
    pub fn new(url: &str) -> EmbedClient {
        let agent = |limit: Duration| -> ureq::Agent {
            ureq::Agent::config_builder()
                .max_redirects(0)
                .http_status_as_error(true)
                .timeout_global(Some(limit))
                .user_agent(concat!("datumesh-http/", env!("CARGO_PKG_VERSION")))
                .build()
                .into()
        };
        EmbedClient { url: url.trim_end_matches('/').to_string(), agent: agent(TIMEOUT), index_agent: agent(INDEX_TIMEOUT) }
    }

    /// The normalised vectors of `texts` with the `passage: ` prefix, in order, under
    /// [`INDEX_TIMEOUT`] (since 0.4.1).
    pub fn embed_passages(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.call(&self.index_agent, "passage: ", texts)
    }

    /// The normalised vectors of `texts`, each with `prefix` (`query: ` or `passage: `), in order.
    pub fn embed(&self, prefix: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.call(&self.agent, prefix, texts)
    }

    fn call(&self, agent: &ureq::Agent, prefix: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let inputs: Vec<String> = texts.iter().map(|t| format!("{prefix}{t}")).collect();
        let body = serde_json::json!({ "inputs": inputs }).to_string();
        let mut resp = agent
            .post(format!("{}/embed", self.url))
            .header("content-type", "application/json")
            .send(body.as_str())
            .map_err(|e| EmbedError(e.to_string()))?;
        let text = resp.body_mut().read_to_string().map_err(|e| EmbedError(e.to_string()))?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| EmbedError(e.to_string()))?;
        let rows = v.as_array().ok_or_else(|| EmbedError("the answer is not an array".into()))?;
        if rows.len() != texts.len() {
            return Err(EmbedError(format!("{} vectors for {} inputs", rows.len(), texts.len())));
        }
        rows.iter().map(parse_row).collect()
    }
}

/// One answered vector: 384 finite values, normalised to unit length.
fn parse_row(row: &serde_json::Value) -> Result<Vec<f32>, EmbedError> {
    let xs = row.as_array().ok_or_else(|| EmbedError("a vector is not an array".into()))?;
    if xs.len() != DIM {
        return Err(EmbedError(format!("a vector has {} values, not {DIM}", xs.len())));
    }
    let mut v = Vec::with_capacity(DIM);
    for x in xs {
        let f = x.as_f64().ok_or_else(|| EmbedError("a vector value is not a number".into()))? as f32;
        if !f.is_finite() {
            return Err(EmbedError("a vector value is not finite".into()));
        }
        v.push(f);
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        return Err(EmbedError("a vector is all zero".into()));
    }
    v.iter_mut().for_each(|x| *x /= norm);
    Ok(v)
}
