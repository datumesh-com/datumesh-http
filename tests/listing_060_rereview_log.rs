//! http 0.6.0, the re-review's N5 (decided 27 Nov): the start-up drop of a host the setting does not name (C5) stays, and
//! each drop is logged with the host and the count of its listings dropped. The event: target `datumesh_http`, fields
//! `host` = the host, `count` = the number of listings dropped, `cause` = `unlisted`. In a file of its own: it installs a
//! global tracing subscriber that writes into a buffer.

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/listing060.rs"]
mod listing060;

use std::io::Write;
use std::sync::{Arc, Mutex};

use catworld::*;
use listing060::*;

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn n5_each_unlisted_drop_is_logged_with_the_host_and_the_count() {
    let buf = Buf::default();
    let w = buf.clone();
    let sub = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || w.clone())
        .finish();
    tracing::subscriber::set_global_default(sub).unwrap();
    let lite = listing_lite("r3-n5");
    put_listing(&lite, "gone.example", "gone.example/a_rate", vec![gb()], RUN1);
    put_listing(&lite, "gone.example", "gone.example/b_rate", vec![gb()], RUN1);
    put_listing(&lite, SRC, &bank_rate(), vec![gb()], RUN1);
    let _m = serve(lite, &kept(SRC)).await;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let line = loop {
        let text = String::from_utf8_lossy(&buf.0.lock().unwrap()).into_owned();
        if let Some(l) = text.lines().find(|l| l.contains("unlisted") && l.contains("gone.example")) {
            break Some(l.to_string());
        }
        if std::time::Instant::now() > end {
            break None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    let line = line.expect("a log line for the unlisted host's drop");
    assert!(line.contains("host=gone.example"), "{line}");
    assert!(line.contains("count=2"), "the count of listings dropped: {line}");
    let text = String::from_utf8_lossy(&buf.0.lock().unwrap()).into_owned();
    assert!(
        !text.lines().any(|l| l.contains("unlisted") && l.contains(&format!("host={SRC}"))),
        "a listed host is not dropped"
    );
}
