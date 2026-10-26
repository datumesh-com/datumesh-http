//! API keys: `Authorization: Bearer dm_<random>`.
//!
//! The keys file (mode 0600) holds one line per key: the SHA-256 of the whole key text
//! (`dm_...`) as 64 lowercase hex digits, the principal id, and optionally the key's own
//! limit in requests per minute; `#` starts a comment. The plaintext of a key is never stored
//! by the mesh: [`create_key`] writes it to a file of its own (mode 0600) and nowhere else.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use datumesh_core::lexical;

use crate::config::ConfigError;

/// A principal: the owner of an API key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    /// The principal id (letters, digits, `-`, `_`, `.`, `@`; at most 64).
    pub id: String,
    /// The key's limit in requests per minute, when it has its own.
    pub per_minute: Option<u32>,
}

/// The API keys a mesh accepts, by hash.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApiKeys {
    by_hash: HashMap<String, Principal>,
}

/// The prefix of every key.
pub const KEY_PREFIX: &str = "dm_";

fn valid_principal(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'@'))
}

impl ApiKeys {
    /// Parses a keys file.
    pub fn parse(bytes: &[u8]) -> Result<ApiKeys, ConfigError> {
        let text = std::str::from_utf8(bytes).map_err(|_| ConfigError::new("API keys", "not UTF-8"))?;
        let mut by_hash = HashMap::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let bad = || ConfigError::new("API keys", format!("line {}: <sha256> <principal> [per minute]", i + 1));
            let mut parts = line.split_whitespace();
            let hash = parts.next().ok_or_else(bad)?;
            let id = parts.next().ok_or_else(bad)?;
            let per_minute = match parts.next() {
                Some(n) => Some(n.parse::<u32>().map_err(|_| bad())?),
                None => None,
            };
            if parts.next().is_some() || lexical::check_hex(hash, 64, "$").is_err() || !valid_principal(id) {
                return Err(bad());
            }
            by_hash.insert(hash.to_string(), Principal { id: id.to_string(), per_minute });
        }
        Ok(ApiKeys { by_hash })
    }

    /// The principal of a presented key, if it is one of these.
    pub fn principal(&self, key: &str) -> Option<&Principal> {
        if !key.starts_with(KEY_PREFIX) || key.len() > 256 {
            return None;
        }
        self.by_hash.get(&datumesh_core::sha256_hex(key.as_bytes()))
    }

    /// Adds a key by its hash (for tests and embedders).
    pub fn insert_hash(&mut self, sha256: &str, principal: Principal) {
        self.by_hash.insert(sha256.to_string(), principal);
    }

    /// The number of keys.
    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }
}

/// A new key: `dm_` and 64 hex digits of 256 bits from the operating system's CSPRNG.
pub fn new_key() -> Result<String, ConfigError> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| ConfigError::new("random", e.to_string()))?;
    Ok(format!("{KEY_PREFIX}{}", lexical::to_hex(&b)))
}

/// Creates a key for `principal`: appends its hash line to `keys_file` (created with mode
/// 0600 if missing; refused if it exists with a wider mode) and writes the key itself, with a
/// final newline, to `out` (created new, mode 0600; refused if it exists). Nothing is printed.
pub fn create_key(keys_file: &Path, principal: &str, per_minute: Option<u32>, out: &Path) -> Result<(), ConfigError> {
    if !valid_principal(principal) {
        return Err(ConfigError::new("principal", "letters, digits, -, _, ., @; at most 64"));
    }
    if keys_file.exists() {
        crate::config::read_secret_file(keys_file)?;
    }
    let key = new_key()?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(out).map_err(|e| ConfigError::new(&out.display().to_string(), e.to_string()))?;
    f.write_all(format!("{key}\n").as_bytes()).map_err(|e| ConfigError::new("key file", e.to_string()))?;
    let mut opts = std::fs::OpenOptions::new();
    opts.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut k = opts.open(keys_file).map_err(|e| ConfigError::new(&keys_file.display().to_string(), e.to_string()))?;
    let line = match per_minute {
        Some(n) => format!("{} {principal} {n}\n", datumesh_core::sha256_hex(key.as_bytes())),
        None => format!("{} {principal}\n", datumesh_core::sha256_hex(key.as_bytes())),
    };
    k.write_all(line.as_bytes()).map_err(|e| ConfigError::new("keys file", e.to_string()))?;
    Ok(())
}
