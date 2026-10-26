//! `datumesh-mesh`: the Datumesh mesh server and its tools.
//!
//! ```text
//! datumesh-mesh [serve]          serve (configuration from the environment; see the README)
//! datumesh-mesh migrate          apply the store's schema to DATABASE_URL
//! datumesh-mesh keygen --out <file> [--publish] [--valid-from <instant>]
//!                                a new signing key (seed, mode 0600); --publish adds it to the
//!                                key document of DATUMESH_HOST in the store (rotating the
//!                                current key out at --valid-from, default now)
//! datumesh-mesh apikey --principal <id> --keys-file <file> --out <file> [--limit <per minute>]
//!                                a new API key: its hash appended to the keys file, the key
//!                                itself written to --out (mode 0600), never printed
//! datumesh-mesh import-vectors <dir>
//!                                the spec's vectors' key documents, descriptors, receipts,
//!                                manifests, name bindings and versions into the store
//! datumesh-mesh openapi          print the OpenAPI document
//! datumesh-mesh health [url]     exit 0 when GET <url> (default http://127.0.0.1:8080/health,
//!                                or DATUMESH_LISTEN's) answers 200: a container health check
//! ```

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use datumesh_core::SigningKey;
use datumesh_engine::{Clock, SystemClock};
use datumesh_http::config::read_secret_file;
use datumesh_http::{Config, Mesh};
use datumesh_store::{migrate, PgStore, PoolOptions, Store};

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("datumesh-mesh: {msg}");
    std::process::exit(1);
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn store(pool: usize) -> PgStore {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| fail("DATABASE_URL is not set"));
    let mut opts = PoolOptions::new().max_size(pool.max(2));
    if let Ok(ca) = std::env::var("DATABASE_CA_FILE") {
        let pem = std::fs::read(&ca).unwrap_or_else(|e| fail(format!("{ca}: {e}")));
        opts = opts.tls_root_certificates_pem(&pem).unwrap_or_else(|e| fail(e));
    }
    PgStore::connect_with(&url, opts).unwrap_or_else(|e| fail(format!("connecting to the store: {e}")))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("serve") => serve(),
        Some("migrate") => {
            let s = store(2);
            migrate::apply(&s).unwrap_or_else(|e| fail(e));
            println!("migrated");
        }
        Some("keygen") => keygen(&args),
        Some("import-vectors") => {
            let dir = args.get(1).unwrap_or_else(|| fail("import-vectors <spec vectors directory>"));
            let s = store(4);
            let report = vectors::import(&s, std::path::Path::new(dir), &SystemClock.now());
            println!("{}", String::from_utf8_lossy(&report.to_jcs()));
        }
        Some("apikey") => {
            let principal = arg(&args, "--principal").unwrap_or_else(|| fail("--principal"));
            let keys = arg(&args, "--keys-file").unwrap_or_else(|| fail("--keys-file"));
            let out = arg(&args, "--out").unwrap_or_else(|| fail("--out"));
            let limit = arg(&args, "--limit").map(|l| l.parse::<u32>().unwrap_or_else(|_| fail("--limit: a number")));
            datumesh_http::apikeys::create_key(&PathBuf::from(&keys), &principal, limit, &PathBuf::from(&out))
                .unwrap_or_else(|e| fail(e));
            println!("key for {principal} written to {out}; its hash appended to {keys}");
        }
        Some("health") => {
            let url = args.get(1).cloned().unwrap_or_else(|| {
                let l = std::env::var("DATUMESH_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".into());
                let port = l.rsplit(':').next().unwrap_or("8080").to_string();
                format!("http://127.0.0.1:{port}/health")
            });
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(std::time::Duration::from_secs(5)))
                .build()
                .into();
            match agent.get(&url).call() {
                Ok(r) if r.status().as_u16() == 200 => println!("healthy"),
                Ok(r) => fail(format!("{url}: status {}", r.status().as_u16())),
                Err(e) => fail(format!("{url}: {e}")),
            }
        }
        Some("openapi") => {
            println!("{}", datumesh_http::openapi().to_pretty_json().unwrap_or_default());
        }
        Some(other) => fail(format!("unknown command {other} (serve, migrate, keygen, apikey, openapi, health)")),
    }
}

fn keygen(args: &[String]) {
    let out = arg(args, "--out").unwrap_or_else(|| fail("--out"));
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).unwrap_or_else(|e| fail(e));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&out).unwrap_or_else(|e| fail(format!("{out}: {e}")));
    f.write_all(format!("{}\n", datumesh_core::lexical::to_hex(&seed)).as_bytes()).unwrap_or_else(|e| fail(e));
    let key = SigningKey::from_seed(&seed);
    println!("key id {} public {}", key.key_id(), datumesh_core::lexical::to_hex(&key.public_key()));
    if args.iter().any(|a| a == "--publish") {
        let host = std::env::var("DATUMESH_HOST").unwrap_or_else(|_| fail("DATUMESH_HOST is not set"));
        let now = SystemClock.now();
        let from = arg(args, "--valid-from").unwrap_or(now.clone());
        let s = store(2);
        let existing = s.namespace(&host).unwrap_or_else(|e| fail(e)).map(|n| n.keydoc);
        let doc = datumesh_http::keydoc::with_key(&host, existing.as_deref(), &key, &from).unwrap_or_else(|e| fail(e));
        s.put_namespace(&host, &doc, &now).unwrap_or_else(|e| fail(e));
        println!("published in the key document of {host} from {from}");
    }
}

fn serve() {
    let json_logs = std::env::var("DATUMESH_LOG_FORMAT").map(|v| v != "text").unwrap_or(true);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    if json_logs {
        tracing_subscriber::fmt().json().with_env_filter(filter).init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
    let config = Config::from_env().unwrap_or_else(|e| fail(e));
    let key_file = std::env::var("DATUMESH_KEY_FILE").unwrap_or_else(|_| fail("DATUMESH_KEY_FILE is not set"));
    read_secret_file(std::path::Path::new(&key_file)).unwrap_or_else(|e| fail(e));
    let listen: SocketAddr = std::env::var("DATUMESH_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()
        .unwrap_or_else(|e| fail(format!("DATUMESH_LISTEN: {e}")));
    let pg = store(config.pool_size);
    if std::env::var("DATUMESH_MIGRATE").is_ok_and(|v| v == "1" || v == "true") {
        migrate::apply(&pg).unwrap_or_else(|e| fail(e));
    }
    let store: Arc<dyn Store + Send + Sync> = Arc::new(pg);
    let mut b = Mesh::builder(config, store).signing_key_file(&key_file);
    if let Ok(l) = std::env::var("DATUMESH_LICENCE") {
        b = b.licence(l);
    }
    if let Ok(f) = std::env::var("DATUMESH_FUEL") {
        b = b.fuel(f.parse().unwrap_or_else(|_| fail("DATUMESH_FUEL: a number")));
    }
    let mesh = b.build().unwrap_or_else(|e| fail(e));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(mesh.config().pool_size + 16)
        .build()
        .unwrap_or_else(|e| fail(e));
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(listen).await.unwrap_or_else(|e| fail(format!("{listen}: {e}")));
        tracing::info!(target: "datumesh_http", %listen, host = %mesh.config().host, pool = mesh.config().pool_size, "serving");
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        #[cfg(unix)]
        let shutdown = async move {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap_or_else(|e| fail(e));
            tokio::select! {
                _ = shutdown => {}
                _ = term.recv() => {}
            }
        };
        mesh.serve(listener, shutdown).await.unwrap_or_else(|e| fail(e));
    });
}

/// `import-vectors <dir>`: the spec's vectors' cells into the store (see `deploy/README.md`).
mod vectors {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    use datumesh_core::json::{self, Value};
    use datumesh_core::{keys_from_response, FunctionDescriptor, Manifest, Receipt, SigningKey};
    use datumesh_store::{PgStore, Store};

    /// The formula vectors' conventions (SPEC 11): each host's key from the seed
    /// SHA-256("datumesh formula test key: <host>"), valid from 2020-01-01.
    const KEY_FROM: &str = "2020-01-01T00:00:00.000000Z";

    fn files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut es: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
        es.sort();
        for p in es {
            if p.is_dir() {
                files(&p, out);
            } else if p.extension().is_some_and(|e| e == "json") {
                out.push(p);
            }
        }
    }

    fn s(v: &str) -> Value {
        Value::String(v.to_string())
    }

    fn num(n: usize) -> Value {
        json::parse(n.to_string().as_bytes()).unwrap_or(Value::Null)
    }

    pub fn import(store: &PgStore, dir: &Path, now: &str) -> Value {
        let mut paths = Vec::new();
        files(dir, &mut paths);
        let vs: Vec<Value> = paths.iter().filter_map(|p| json::parse(&std::fs::read(p).ok()?).ok()).collect();
        let valid = |v: &Value| v.get("valid") == Some(&Value::Bool(true));
        let kind = |v: &Value| v.get("kind").and_then(Value::as_str).unwrap_or("").to_string();

        // Key documents: every valid receipt vector's retrieved documents and the formula
        // vectors' hosts' test keys, merged per host by key id.
        let mut keys: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
        let mut world_hosts = BTreeSet::new();
        for v in &vs {
            if kind(v) == "receipt" && valid(v) {
                for (host, env) in v.get("keys").and_then(Value::as_object).unwrap_or(&[]) {
                    let https = env.get("https");
                    if https.and_then(|h| h.get("status")).and_then(Value::as_number_text) != Some("200") {
                        continue;
                    }
                    let Some(body) = https.and_then(|h| h.get("body")).and_then(Value::as_str) else { continue };
                    if keys_from_response(host, 200, Some(body.as_bytes())).is_err() {
                        continue;
                    }
                    let doc = json::parse(body.as_bytes()).unwrap_or(Value::Null);
                    if let Some(Value::Array(ks)) = doc.get("keys") {
                        for k in ks {
                            if let Some(id) = k.get("id").and_then(Value::as_str) {
                                keys.entry(host.clone())
                                    .or_default()
                                    .entry(id.to_string())
                                    .or_insert_with(|| k.clone());
                            }
                        }
                    }
                }
            }
            if matches!(kind(v).as_str(), "formula-eval" | "excel") && valid(v) {
                if let Some(w) = v.get("world") {
                    for (name, _) in w.get("functions").and_then(Value::as_object).unwrap_or(&[]) {
                        world_hosts.insert(name.split('/').next().unwrap_or("").to_string());
                    }
                    if let Some(Value::Array(rs)) = w.get("receipts") {
                        for r in rs {
                            if let Some(h) = r.get("computed_by").and_then(Value::as_str) {
                                world_hosts.insert(h.split('#').next().unwrap_or("").to_string());
                            }
                        }
                    }
                }
            }
        }
        for host in &world_hosts {
            let key =
                SigningKey::from_seed(&datumesh_core::sha256(format!("datumesh formula test key: {host}").as_bytes()));
            let k = Value::Object(vec![
                ("alg".into(), s("ed25519")),
                ("id".into(), s(key.key_id())),
                ("public".into(), s(&datumesh_core::lexical::to_hex(&key.public_key()))),
                ("valid_from".into(), s(KEY_FROM)),
            ]);
            keys.entry(host.clone()).or_default().entry(key.key_id().to_string()).or_insert(k);
        }
        let mut namespaces = 0;
        for (host, ks) in &keys {
            let doc = Value::Object(vec![
                ("host".into(), s(host)),
                ("keys".into(), Value::Array(ks.values().take(64).cloned().collect())),
                ("proto".into(), s("datumesh/1")),
            ])
            .to_jcs();
            if store.namespace(host).ok().flatten().map(|n| n.keydoc) != Some(doc.clone())
                && store.put_namespace(host, &doc, now).is_ok()
            {
                namespaces += 1;
            }
        }

        // Descriptors, receipts, manifests; candidate bindings and versions.
        let mut descriptors: BTreeMap<String, String> = BTreeMap::new(); // fn_hash -> lang
        let mut bindings: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new(); // name -> (since, fn_hash)
        let mut versions: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new(); // cell id -> (tx, hash, fn_hash)
        let (mut receipts, mut refused, mut manifests) = (0usize, Vec::new(), 0usize);
        let put_descriptor = |d: &Value, descriptors: &mut BTreeMap<String, String>| -> Option<String> {
            let fd = FunctionDescriptor::from_value(d).ok()?;
            let h = store.put_descriptor(&fd.canonical()).ok()?;
            descriptors.insert(h.clone(), fd.lang.clone());
            Some(h)
        };
        let mut put_receipt =
            |bytes: &[u8], id: &str, versions: &mut BTreeMap<String, Vec<(String, String, String)>>| match (
                store.put_receipt(bytes),
                Receipt::from_json(bytes),
            ) {
                (Ok(h), Ok(r)) => {
                    receipts += 1;
                    versions.entry(r.cell().id()).or_default().push((r.tx().to_string(), h, r.cell().fn_hash.clone()));
                }
                (Err(e), _) => refused.push(format!("{id}: {e}")),
                _ => {}
            };
        for v in &vs {
            let id = v.get("id").and_then(Value::as_str).unwrap_or("?").to_string();
            match kind(v).as_str() {
                "receipt" if valid(v) => {
                    let Some(text) = v.get("input").and_then(Value::as_str) else { continue };
                    let rc = v.get("recompute");
                    if let (Some(d), Ok(r)) =
                        (rc.and_then(|r| r.get("descriptor")), Receipt::from_json(text.as_bytes()))
                    {
                        if let Some(h) = put_descriptor(d, &mut descriptors) {
                            if h == r.cell().fn_hash {
                                bindings.entry(r.cell().function_name()).or_default().push((r.tx().to_string(), h));
                            }
                        }
                    }
                    for (h, t) in rc.and_then(|r| r.get("inputs")).and_then(Value::as_object).unwrap_or(&[]) {
                        if let Some(t) = t.as_str() {
                            put_receipt(t.as_bytes(), &format!("{id} input {h}"), &mut versions);
                        }
                    }
                    for (_, t) in rc.and_then(|r| r.get("manifests")).and_then(Value::as_object).unwrap_or(&[]) {
                        if t.as_str().is_some_and(|t| store.put_manifest(t.as_bytes()).is_ok()) {
                            manifests += 1;
                        }
                    }
                    put_receipt(text.as_bytes(), &id, &mut versions);
                }
                "slice" if valid(v) => {
                    if let Some(t) = v.get("input").and_then(Value::as_str) {
                        if Manifest::from_json(t.as_bytes()).is_ok() && store.put_manifest(t.as_bytes()).is_ok() {
                            manifests += 1;
                        }
                    }
                }
                "formula-eval" | "excel" if valid(v) => {
                    let Some(w) = v.get("world") else { continue };
                    for (name, f) in w.get("functions").and_then(Value::as_object).unwrap_or(&[]) {
                        let (Some(d), Some(b)) = (f.get("descriptor"), f.get("bound").and_then(Value::as_str)) else {
                            continue;
                        };
                        if let Some(h) = put_descriptor(d, &mut descriptors) {
                            bindings.entry(name.clone()).or_default().push((b.to_string(), h));
                        }
                    }
                    if let Some(Value::Array(rs)) = w.get("receipts") {
                        for r in rs {
                            put_receipt(&r.to_jcs(), &format!("{id} world"), &mut versions);
                        }
                    }
                }
                _ => {}
            }
        }

        // Bindings: a name with one function is bound from its earliest instant; a name the
        // vectors bind to several functions keeps the first (one name means one function at a
        // time) and the others are reported.
        let (mut bound, mut skipped) = (0usize, Vec::new());
        for (name, cs) in &bindings {
            let hashes: BTreeSet<&String> = cs.iter().map(|(_, h)| h).collect();
            let (since, h) = cs.iter().filter(|(_, h)| *h == cs[0].1).min().cloned().unwrap_or_default();
            if hashes.len() > 1 {
                skipped.push(format!("{name}: {} functions, the first bound", hashes.len()));
            }
            match store.resolve_binding(name, "9999-12-31T23:59:59.999999Z") {
                Ok(Some(b)) if b.fn_hash == h => {}
                Ok(None) => {
                    if store.bind_name(name, &h, &since).is_ok() {
                        bound += 1;
                    }
                }
                _ => skipped.push(format!("{name}: already bound otherwise")),
            }
        }
        // Versions of stored functions' cells, in tx order (a second receipt at one tx is not a
        // version).
        let mut stored_versions = 0usize;
        for (cell_id, mut vs) in versions {
            vs.sort();
            vs.dedup_by(|a, b| a.0 == b.0);
            for (tx, hash, fn_hash) in vs {
                if descriptors.get(&fn_hash).map(String::as_str) != Some("stored") {
                    continue;
                }
                match store.latest(&cell_id) {
                    Ok(Some(l)) if l.tx >= tx => continue,
                    _ => {}
                }
                if store.put_version(&cell_id, &tx, &hash).is_ok() {
                    stored_versions += 1;
                }
            }
        }
        Value::Object(vec![
            ("bindings".into(), num(bound)),
            ("bindings_skipped".into(), Value::Array(skipped.iter().map(|x| s(x)).collect())),
            ("descriptors".into(), num(descriptors.len())),
            ("manifests".into(), num(manifests)),
            ("namespaces".into(), num(namespaces)),
            ("receipts".into(), num(receipts)),
            ("receipts_refused".into(), Value::Array(refused.iter().map(|x| s(x)).collect())),
            ("versions".into(), num(stored_versions)),
        ])
    }
}
