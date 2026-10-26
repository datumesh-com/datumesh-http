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
//! datumesh-mesh openapi          print the OpenAPI document
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
        Some("apikey") => {
            let principal = arg(&args, "--principal").unwrap_or_else(|| fail("--principal"));
            let keys = arg(&args, "--keys-file").unwrap_or_else(|| fail("--keys-file"));
            let out = arg(&args, "--out").unwrap_or_else(|| fail("--out"));
            let limit = arg(&args, "--limit").map(|l| l.parse::<u32>().unwrap_or_else(|_| fail("--limit: a number")));
            datumesh_http::apikeys::create_key(&PathBuf::from(&keys), &principal, limit, &PathBuf::from(&out))
                .unwrap_or_else(|e| fail(e));
            println!("key for {principal} written to {out}; its hash appended to {keys}");
        }
        Some("openapi") => {
            println!("{}", datumesh_http::openapi().to_pretty_json().unwrap_or_default());
        }
        Some(other) => fail(format!("unknown command {other} (serve, migrate, keygen, apikey, openapi)")),
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
