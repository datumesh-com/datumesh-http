//! Helpers of the 0.5.1 tests (CONTRACT-0.5.1 H1-H5): counters on more tables than `h0410::Probe` counts, and a
//! per-statement maximum, on the same PostgreSQL database and role as the `Fx` of `common/h0410.rs`.
//!
//! * [`arm_tables`] counts the rows a table's statements visit, as `Probe::arm` does for its groups, on tables of
//!   the caller's choice (`classes`, `class_bindings`), into a sequence of the caller's name.
//! * [`arm_statement_max`] records, for the tables given, the most rows ONE statement visited (a row-level
//!   security policy with a function that keeps a count per statement, keyed on `statement_timestamp()`, and the
//!   largest in a table). "No read larger than a page" is a bound on that figure; the totals of `Probe` cannot
//!   tell one big read from many small ones.
//!
//! Needs `mod common; #[path = "common/h0410.rs"] mod h0410; #[path = "common/h051.rs"] mod h051;` at the crate
//! root.
#![allow(dead_code)]

use datumesh_http::Config;
use datumesh_store::pg_unstable::postgres::{Client, Config as PgConfig, NoTls};
use serde_json::{json, Value};

use crate::common::lite::*;
use crate::common::on_thread;
use crate::h0410::{config, Fx};

fn admin(url: &str) -> Client {
    let cfg: PgConfig = url.parse().unwrap();
    cfg.connect(NoTls).unwrap()
}

/// Counts the rows visited in `tables` into the sequence `seq` (created here). Call after the fixture is
/// published, and after `Probe::arm` when that is used too.
pub fn arm_tables(fx: &Fx, seq: &str, tables: &[&str]) {
    let (url, role) = (fx.probe().db_url.clone(), fx.probe().role.clone());
    let (seq, tables): (String, Vec<String>) = (seq.to_string(), tables.iter().map(|t| t.to_string()).collect());
    on_thread(move || {
        let mut c = admin(&url);
        c.batch_execute(&format!("CREATE SEQUENCE IF NOT EXISTS {seq}")).unwrap();
        c.batch_execute(&format!("GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO \"{role}\""))
            .unwrap();
        for t in &tables {
            c.batch_execute(&format!(
                "ALTER TABLE {t} ENABLE ROW LEVEL SECURITY;\
                 DROP POLICY IF EXISTS p051_{seq}_sel ON {t};\
                 CREATE POLICY p051_{seq}_sel ON {t} FOR SELECT USING (nextval('{seq}') > 0);\
                 DROP POLICY IF EXISTS p051_ins ON {t}; CREATE POLICY p051_ins ON {t} FOR INSERT WITH CHECK (true);\
                 DROP POLICY IF EXISTS p051_upd ON {t}; CREATE POLICY p051_upd ON {t} FOR UPDATE USING (true) WITH CHECK (true);\
                 DROP POLICY IF EXISTS p051_del ON {t}; CREATE POLICY p051_del ON {t} FOR DELETE USING (true)"
            ))
            .unwrap_or_else(|e| panic!("arming {t}: {e}"));
        }
    });
}

/// Rows visited in the tables of [`arm_tables`] into `seq` so far.
pub fn reads_of(fx: &Fx, seq: &str) -> u64 {
    let (url, seq) = (fx.probe().db_url.clone(), seq.to_string());
    on_thread(move || {
        let mut c = admin(&url);
        let row = c
            .query_one(&format!("SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM {seq}"), &[])
            .unwrap_or_else(|e| panic!("{seq} is not armed: {e}"));
        row.get::<_, i64>(0) as u64
    })
}

/// Records the most rows one statement visits in `tables`. [`statement_max`] reads it, [`statement_reset`] zeroes it.
/// A table that `Probe::arm` has armed keeps its counting policy; one that has no policy gets a catch-all.
pub fn arm_statement_max(fx: &Fx, tables: &[&str]) {
    let (url, role) = (fx.probe().db_url.clone(), fx.probe().role.clone());
    let tables: Vec<String> = tables.iter().map(|t| t.to_string()).collect();
    on_thread(move || {
        let mut c = admin(&url);
        c.batch_execute(
            r#"CREATE TABLE IF NOT EXISTS p051_stmt (id int PRIMARY KEY, n bigint NOT NULL);
INSERT INTO p051_stmt VALUES (1, 0) ON CONFLICT DO NOTHING;
CREATE OR REPLACE FUNCTION p051_row() RETURNS boolean LANGUAGE plpgsql VOLATILE SECURITY DEFINER AS $f$
DECLARE
  ts text := statement_timestamp()::text;
  cur bigint;
BEGIN
  IF coalesce(current_setting('p051.ts', true), '') <> ts THEN
    PERFORM set_config('p051.ts', ts, false);
    PERFORM set_config('p051.n', '0', false);
  END IF;
  cur := coalesce(nullif(current_setting('p051.n', true), ''), '0')::bigint + 1;
  PERFORM set_config('p051.n', cur::text, false);
  IF cur > (SELECT n FROM p051_stmt WHERE id = 1) THEN
    UPDATE p051_stmt SET n = cur WHERE id = 1;
  END IF;
  RETURN true;
END
$f$;
GRANT EXECUTE ON FUNCTION p051_row() TO PUBLIC"#,
        )
        .unwrap_or_else(|e| panic!("arming the statement maximum: {e} {:?}", e.as_db_error()));
        c.batch_execute(&format!("GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO \"{role}\"")).unwrap();
        for t in &tables {
            // a restrictive policy is ANDed with the permissive ones (the counting policies of `Probe::arm`), so
            // every row a statement visits goes through the function
            let armed: i64 = c
                .query_one("SELECT count(*) FROM pg_policies WHERE tablename = $1 AND permissive = 'PERMISSIVE'", &[t])
                .unwrap()
                .get(0);
            if armed == 0 {
                c.batch_execute(&format!(
                    "ALTER TABLE {t} ENABLE ROW LEVEL SECURITY;\
                     CREATE POLICY p051_all ON {t} FOR ALL USING (true) WITH CHECK (true)"
                ))
                .unwrap_or_else(|e| panic!("arming {t}: {e}"));
            }
            c.batch_execute(&format!(
                "DROP POLICY IF EXISTS p051_stmt ON {t};\
                 CREATE POLICY p051_stmt ON {t} AS RESTRICTIVE FOR SELECT USING (p051_row())"
            ))
            .unwrap_or_else(|e| panic!("arming {t}: {e}"));
        }
    });
}

pub fn statement_max(fx: &Fx) -> u64 {
    let url = fx.probe().db_url.clone();
    on_thread(move || admin(&url).query_one("SELECT n FROM p051_stmt WHERE id = 1", &[]).unwrap().get::<_, i64>(0) as u64)
}

pub fn statement_reset(fx: &Fx) {
    let url = fx.probe().db_url.clone();
    on_thread(move || {
        admin(&url).batch_execute("UPDATE p051_stmt SET n = 0 WHERE id = 1").unwrap();
    });
}

// ---- a series of 10,000 cells: 100 currencies by 100 days ----------------------------------------------------------------

pub const FXR: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu/fx_reference";
pub const CLOCK: &str = "2026-01-01T00:00:00.000000Z";
pub const CURRENCIES: usize = 100;
pub const DAYS: usize = 100;

pub fn fx_sig() -> String {
    r#"{"args":{"currency":{"type":"string","valid":false},"date":{"type":"date","valid":true}},"order":["currency","date"],"returns":{"type":"number","unit":"1"}}"#.to_string()
}

pub fn currency(i: usize) -> String {
    format!("C{i:03}")
}

/// The i-th day from 2026-01-01 (the first 100 days run to 2026-04-10).
pub fn day(i: usize) -> String {
    let months = [("01", 31), ("02", 28), ("03", 31), ("04", 30), ("05", 31)];
    let mut left = i;
    for (m, n) in months {
        if left < n {
            return format!("2026-{m}-{:02}", left + 1);
        }
        left -= n;
    }
    panic!("day {i} is past the table")
}

fn source() -> Value {
    json!({"name": "ecb.europa.eu", "url": "https://ecb.europa.eu/data", "retrieved": CLOCK,
        "takeover": format!("https://{HOST}/takeover/ecb.europa.eu")})
}

pub fn fx_cell(cur: &str, d: &str) -> String {
    let tx = format!("{d}T12:00:00.000000Z");
    json!({"cell": {"fn": FXR, "args": {"currency": {"type": "string", "value": cur}, "date": {"type": "date", "value": d}}},
        "licence": "CC0-1.0", "summary": "Synthetic test data.", "value": {"type": "number", "value": "1.5"},
        "source": source(), "tx": tx, "asof": tx, "issued": tx})
    .to_string()
}

/// The function, 10,000 cells (currency `C000`..`C099` by the first 100 days of 2026), and the catalogue entry that lists it.
pub fn publish_fx_world(fx: &Fx) {
    fx.publish(&[crate::common::pub031::function_line(FXR, &fx_sig(), None)], CLOCK);
    let mut v = Vec::with_capacity(CURRENCIES * DAYS + 1);
    for c in 0..CURRENCIES {
        for d in 0..DAYS {
            v.push(fx_cell(&currency(c), &day(d)));
        }
    }
    let dict: Vec<Value> =
        (0..CURRENCIES).map(|i| json!({"value": currency(i), "label": format!("Currency {i}")})).collect();
    v.push(crate::common::readpath::catalog_line(
        FXR,
        "function",
        "Euro reference rate",
        "The euro foreign exchange reference rates.",
        json!({"currency": "the currency", "date": "the day"}),
        json!({"currency": dict}),
    ));
    fx.publish(&v, NOW);
}

/// The path of a series file listing [`FXR`] as a month series with a column per currency.
pub fn series_file(fx: &Fx) -> String {
    let p = fx.dir.join("series.json");
    std::fs::write(
        &p,
        json!([{"function": FXR, "granularity": "month", "date_arg": "date", "column_arg": "currency",
            "unit": "units per EUR", "kind": "relayed"}])
        .to_string(),
    )
    .unwrap();
    p.to_string_lossy().into_owned()
}

/// A configuration with the three background builds held back but the ones named in `free`.
pub fn cfg(free: &[&str], extra: &[(&str, &str)]) -> Config {
    let mut vars = crate::h0410::held(free);
    vars.extend(extra.iter().copied());
    config(&vars, &[])
}

/// A bound on the rows one request may visit for a read of `n` members: a member is a handful of rows (its cell, its
/// arguments, its versions), never more than this many.
pub const ROWS_PER_MEMBER: u64 = 20;
