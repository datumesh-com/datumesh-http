//! Domain validation (since the namespace-validation step): a namespace host is **valid**
//! when `https://<host>/.well-known/datumesh.json` fetches and passes the key-document rules
//! (SPEC 9.2, 9.3: [`datumesh_core::key_document_from_response`], declarations included).
//!
//! [`Validator::validate`] checks one host and records the outcome in the store:
//!
//! * **valid**: the document is recorded with `put_namespace` (so `validated_at` moves
//!   forward and a rotation, SPEC 9.5, replaces the stored key document through the store's
//!   normal path), then [`Store::record_namespace_success`] resets the failure count and
//!   reinstates a suspended namespace;
//! * **invalid** (no answer, not 200, not a key document for the host, a declaration that
//!   fails): [`Store::record_namespace_failure`] adds one to the consecutive failures, and
//!   the [`SUSPEND_AFTER`]th in a row suspends the namespace (`suspended_at`).
//!
//! A suspended namespace refuses **new writes** under it with the store's
//! [`datumesh_store::Error::NamespaceSuspended`]: a receipt signed by the host
//! (`put_receipt`), a version of such a receipt (`put_version`), a name under the host
//! (`bind_name`, so `define` and `publish-stored` too, and the import of a foreign host's
//! bindings and cells). Reads, verification, the key document the mesh serves and the
//! receipts already stored are not affected; a cross-mesh import that the store refuses
//! reaches a request as `E_INPUT_UNAVAILABLE` (the closest code that exists), with the
//! suspension in its detail.
//!
//! The mesh's **own** namespaces ([`Config::namespaces`](crate::Config::namespaces)) are never
//! validated here, so never suspended: a failed fetch of the mesh's own site must not stop the
//! mesh publishing. Hosts with no namespace in the store have nothing to suspend; their
//! failed validation is reported and nothing is recorded.
//!
//! `datumesh-mesh validate <host>` and `validate --all` run this from the command line; the
//! weekly re-check is a cron entry running `validate --all` (`deploy/install_cron.sh`).

use datumesh_core::json::{self, Value};
use datumesh_core::{key_document_from_response, ErrorCode};
use datumesh_store::{Error as StoreError, Store};

use crate::client::Client;

/// The largest key document read (SPEC 9.3's suggested bound).
const MAX_DOC: u64 = 256 << 10;

/// The consecutive failed validations after which a namespace is suspended.
pub const SUSPEND_AFTER: u32 = 2;

/// How one host's validation came out.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The document fetched and passed the rules.
    Valid,
    /// It did not: the code of SPEC 9.3 (`E_KEY_UNAVAILABLE`, `E_KEY_DOC`) and why.
    Invalid {
        /// `E_KEY_UNAVAILABLE` (no answer, or a status other than 200) or `E_KEY_DOC`.
        code: ErrorCode,
        /// Why.
        detail: String,
    },
    /// Not validated: a namespace of this mesh itself.
    Skipped,
}

/// The result of validating one host, with what the store now records.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Validation {
    /// The host.
    pub host: String,
    /// How it came out.
    pub outcome: Outcome,
    /// The consecutive failures now recorded (zero after a success; zero for a host with no
    /// namespace in the store, whose failure is not recorded).
    pub failures: u32,
    /// Whether the namespace is suspended now.
    pub suspended: bool,
    /// Whether this validation suspended it.
    pub suspended_now: bool,
    /// Whether this validation reinstated it.
    pub reinstated: bool,
    /// Whether the document differs from the one stored before (a rotation or another
    /// change), and replaced it.
    pub rotated: bool,
}

impl Validation {
    fn new(host: &str, outcome: Outcome) -> Validation {
        Validation {
            host: host.to_string(),
            outcome,
            failures: 0,
            suspended: false,
            suspended_now: false,
            reinstated: false,
            rotated: false,
        }
    }

    /// Whether the outcome is [`Outcome::Invalid`].
    pub fn failed(&self) -> bool {
        matches!(self.outcome, Outcome::Invalid { .. })
    }

    /// One canonical JSON object (the `validate` command prints one per host): `host`,
    /// `outcome` (`valid`, `invalid`, `skipped`), `failures`, `suspended`, `suspended_now`,
    /// `reinstated`, `rotated`, and for an invalid host `code` and `detail`.
    pub fn to_json(&self) -> Vec<u8> {
        let b = |v: bool| Value::Bool(v);
        let mut m = vec![
            ("failures".to_string(), json::parse(self.failures.to_string().as_bytes()).unwrap_or(Value::Null)),
            ("host".to_string(), Value::String(self.host.clone())),
            ("reinstated".to_string(), b(self.reinstated)),
            ("rotated".to_string(), b(self.rotated)),
            ("suspended".to_string(), b(self.suspended)),
            ("suspended_now".to_string(), b(self.suspended_now)),
        ];
        let outcome = match &self.outcome {
            Outcome::Valid => "valid",
            Outcome::Skipped => "skipped",
            Outcome::Invalid { code, detail } => {
                m.push(("code".to_string(), Value::String(code.as_str().to_string())));
                m.push(("detail".to_string(), Value::String(detail.clone())));
                "invalid"
            }
        };
        m.push(("outcome".to_string(), Value::String(outcome.to_string())));
        Value::Object(m).to_jcs()
    }
}

/// Validates namespace hosts against their own sites and records the outcome in a store.
#[derive(Debug)]
pub struct Validator {
    client: Client,
    own: Vec<String>,
}

impl Validator {
    /// A validator fetching with `client` (its peers map `https://<host>` to where a host
    /// answers; its allowed-host list, if any, applies), skipping `own`, the hosts this mesh
    /// owns.
    pub fn new(client: Client, own: Vec<String>) -> Validator {
        Validator { client, own }
    }

    /// Fetches `host`'s key document and checks it.
    fn fetch(&self, host: &str) -> Result<Vec<u8>, (ErrorCode, String)> {
        let url = format!("https://{host}/.well-known/datumesh.json");
        let (status, body) = self.client.get(&url, MAX_DOC).map_err(|e| (ErrorCode::KeyUnavailable, e))?;
        key_document_from_response(host, status, if status == 200 { Some(&body) } else { None })
            .map_err(|e| (e.code(), e.to_string()))?;
        Ok(body)
    }

    /// Validates `host` at `now` (an instant, not earlier than what the store recorded for
    /// it) and records the outcome as the module documentation says.
    ///
    /// Errors are the store's: a host that is not a DNS name, an instant earlier than the
    /// latest check, a database failure. A failed validation is a [`Validation`], not an error.
    pub fn validate(&self, store: &dyn Store, host: &str, now: &str) -> Result<Validation, StoreError> {
        datumesh_core::lexical::check_dns_name(host, "host")?;
        if self.own.iter().any(|h| h == host) {
            return Ok(Validation::new(host, Outcome::Skipped));
        }
        let before = store.namespace(host)?;
        match self.fetch(host) {
            Ok(body) => {
                let canonical = json::parse(&body)?.to_jcs();
                let rotated = before.as_ref().is_some_and(|n| n.keydoc != canonical);
                store.put_namespace(host, &body, now)?;
                store.record_namespace_success(host, now)?;
                let mut v = Validation::new(host, Outcome::Valid);
                v.rotated = rotated;
                v.reinstated = before.as_ref().is_some_and(|n| n.suspended_at.is_some());
                Ok(v)
            }
            Err((code, detail)) => {
                let mut v = Validation::new(host, Outcome::Invalid { code, detail });
                if let Some(after) = store.record_namespace_failure(host, now, SUSPEND_AFTER)? {
                    v.failures = after.consecutive_failures;
                    v.suspended = after.suspended_at.is_some();
                    v.suspended_now = v.suspended && before.as_ref().is_some_and(|n| n.suspended_at.is_none());
                }
                Ok(v)
            }
        }
    }

    /// Validates every namespace in the store, by host, at `now`: [`validate`](Validator::validate)
    /// for each (the mesh's own are [`Outcome::Skipped`]).
    pub fn validate_all(&self, store: &dyn Store, now: &str) -> Result<Vec<Validation>, StoreError> {
        store.namespaces()?.iter().map(|n| self.validate(store, &n.host, now)).collect()
    }
}
