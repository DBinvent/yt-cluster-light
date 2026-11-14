// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! yt-turso-load: an application on Turso with the lite node inside it (yt-core, feature
//! `turso`) - a leaf of a YT Cluster - and a load on it like pgbench's. The end-to-end test of
//! a Turso leaf drives it (yt-cluster-tests hier/87_load.sh with YX_LOAD_LEAF=turso).
//!
//! ```text
//! yt-turso-load init --db <file> --rows <n> --accounts <n> --schemas common,s_a,...
//!     the database before the load, as on every PostgreSQL node of the test: `common.big`
//!     with <rows> rows, and in each other schema `accounts` (<accounts> rows) and `history`
//! yt-turso-load run --db <file> --dir <dir> --uuid <uuid> --upper <hosts> --root <SHA256:...>
//!                   --cluster-db <name> --schema <s> --tx <n> [--key-dir <dir>]
//!     the lite node on <file>, its writes those of schema <s>; in <dir>: `status`, rewritten
//!     each second; `go`, made by the test: <n> transactions on <s> then; `stop`: once
//!     nothing waits to go up and nothing new comes down, each table dumped to
//!     <dir>/dump/<table>.tsv - PostgreSQL's COPY text, in key order - and the end
//! yt-turso-load dump --db <file> --dir <dir>
//!     each table to <dir>/<table>.tsv, as `run` dumps them: the database as it is
//! ```
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use turso::{Connection, Value};
use yt_core::core::store::Node;
use yt_core::db::Db;
use yt_core::leaf::{self, Status};
use yt_core::turso::{kind, Kind, Turso};

fn args() -> (String, BTreeMap<String, String>) {
	let mut a = std::env::args().skip(1);
	let cmd = a.next().unwrap_or_default();
	let mut m = BTreeMap::new();
	while let Some(k) = a.next() {
		if let Some(k) = k.strip_prefix("--") {
			m.insert(k.to_string(), a.next().unwrap_or_default());
		}
	}
	(cmd, m)
}

fn need<'a>(m: &'a BTreeMap<String, String>, k: &str) -> &'a str {
	m.get(k).map(|s| s.as_str()).unwrap_or_else(|| {
		eprintln!("yt-turso-load: --{} is missing", k);
		std::process::exit(2)
	})
}

fn say(s: &str) {
	println!("{} {}", now_text(), s);
}

/// A time as PostgreSQL writes a timestamptz in UTC, to the second: `2026-11-13 18:00:00+00`.
fn utc_text(secs: u64) -> String {
	let days = (secs / 86400) as i64;
	let rem = secs % 86400;
	// days since 1970-01-01 to a date (H. Hinnant's civil_from_days)
	let z = days + 719468;
	let era = z.div_euclid(146097);
	let doe = z - era * 146097;
	let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
	let y = yoe + era * 400;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let d = doy - (153 * mp + 2) / 5 + 1;
	let m = if mp < 10 { mp + 3 } else { mp - 9 };
	let y = if m <= 2 { y + 1 } else { y };
	format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}+00", y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

fn now_text() -> String {
	utc_text(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

fn md5hex(s: &str) -> String {
	format!("{:x}", md5::compute(s.as_bytes()))
}

fn q(s: &str) -> String {
	format!("\"{}\"", s.replace('"', "\"\""))
}

async fn rows(c: &Connection, sql: &str) -> Result<Vec<Vec<Value>>, turso::Error> {
	let mut r = c.query(sql, ()).await?;
	let mut out = Vec::new();
	while let Some(row) = r.next().await? {
		out.push((0..row.column_count()).map(|i| row.get_value(i)).collect::<Result<Vec<_>, _>>()?);
	}
	Ok(out)
}

/// The database before the load: the same tables and rows as the PostgreSQL nodes' (the
/// test's init_sql), the columns declared with their PostgreSQL types.
async fn init(m: &BTreeMap<String, String>) -> Result<(), String> {
	let db = turso::Builder::new_local(need(m, "db")).build().await.map_err(|e| e.to_string())?;
	let c = db.connect().map_err(|e| e.to_string())?;
	let n_rows: i64 = need(m, "rows").parse().map_err(|_| "--rows: a number".to_string())?;
	let accounts: i64 = need(m, "accounts").parse().map_err(|_| "--accounts: a number".to_string())?;
	for s in need(m, "schemas").split(',').filter(|s| !s.is_empty()) {
		if s == "common" {
			c.execute_batch("DROP TABLE IF EXISTS \"common.big\";
				CREATE TABLE \"common.big\" (id bigint PRIMARY KEY, v text NOT NULL, n int NOT NULL);").await.map_err(|e| e.to_string())?;
			let mut g = 1;
			while g <= n_rows {
				let end = (g + 9999).min(n_rows);
				let vals: Vec<String> = (g..=end).map(|i| format!("({}, '{}', {})", i, md5hex(&i.to_string()), (i * 7919) % 100003)).collect();
				c.execute(&format!("INSERT INTO \"common.big\" VALUES {}", vals.join(",")), ()).await.map_err(|e| e.to_string())?;
				g = end + 1;
			}
			continue;
		}
		c.execute_batch(&format!("DROP TABLE IF EXISTS {a}; DROP TABLE IF EXISTS {h};
			CREATE TABLE {a} (aid int PRIMARY KEY, abalance int NOT NULL DEFAULT 0, filler text NOT NULL);
			CREATE TABLE {h} (hid bigint PRIMARY KEY, aid int NOT NULL, delta int NOT NULL, mtime timestamptz NOT NULL);",
			a = q(&format!("{s}.accounts")), h = q(&format!("{s}.history")))).await.map_err(|e| e.to_string())?;
		let mut g = 1;
		while g <= accounts {
			let end = (g + 9999).min(accounts);
			let vals: Vec<String> = (g..=end).map(|i| format!("({}, 0, '{}')", i, md5hex(&format!("{}{}", i, s)))).collect();
			c.execute(&format!("INSERT INTO {} VALUES {}", q(&format!("{s}.accounts")), vals.join(",")), ()).await.map_err(|e| e.to_string())?;
			g = end + 1;
		}
	}
	say("init: done");
	Ok(())
}

/// A value as PostgreSQL's COPY writes it in text form, for a column declared `decl`.
fn copy_text(v: &Value, decl: &str) -> String {
	match v {
		Value::Null => "\\N".to_string(),
		Value::Integer(i) if kind(decl).0 == Kind::Bool => if *i != 0 { "t" } else { "f" }.to_string(),
		Value::Integer(i) => i.to_string(),
		Value::Real(f) => float_text(*f),
		// COPY's escapes: backslash, and the control characters it names (review of 2026-11-13)
		Value::Text(s) => s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n").replace('\r', "\\r")
			.replace('\u{8}', "\\b").replace('\u{c}', "\\f").replace('\u{b}', "\\v"),
		Value::Blob(b) => format!("\\\\x{}", b.iter().map(|x| format!("{:02x}", x)).collect::<String>()),
	}
}

/// A float as PostgreSQL writes a float8: the shortest that reads back, an exponent past 1e15
/// or under 1e-4 written `1e+20`, `1e-05` (review of 2026-11-13).
fn float_text(f: f64) -> String {
	if f.is_nan() { return "NaN".to_string() }
	if f.is_infinite() { return if f > 0.0 { "Infinity" } else { "-Infinity" }.to_string() }
	let a = f.abs();
	if a != 0.0 && !(1e-4..1e15).contains(&a) {
		let e = format!("{:e}", f);
		let (m, x) = e.split_once('e').unwrap_or((&e, "0"));
		let x: i32 = x.parse().unwrap_or(0);
		return format!("{}e{}{:02}", m, if x < 0 { "-" } else { "+" }, x.abs());
	}
	f.to_string()
}

/// The database's tables but the node's own.
async fn tables(c: &Connection) -> Result<Vec<String>, String> {
	Ok(rows(c, "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'yt\\_%' ESCAPE '\\' \
		AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name").await.map_err(|e| e.to_string())?
		.into_iter().filter_map(|r| match &r[0] { Value::Text(t) => Some(t.clone()), _ => None }).collect())
}

/// Every table of the database but the node's own, to <dir>/<table>.tsv in key order.
async fn dump(c: &Connection, dir: &Path) -> Result<usize, String> {
	std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
	let tables = tables(c).await?;
	for t in &tables {
		std::fs::write(dir.join(format!("{}.tsv", t)), table_text(c, t).await?).map_err(|e| e.to_string())?;
	}
	Ok(tables.len())
}

/// A table as COPY writes it in text form, its rows in key order.
async fn table_text(c: &Connection, t: &str) -> Result<String, String> {
	{
		let cols = rows(c, &format!("SELECT name, type, pk FROM pragma_table_info('{}') ORDER BY cid", t.replace('\'', "''")))
			.await.map_err(|e| e.to_string())?;
		let decl: Vec<String> = cols.iter().map(|r| match &r[1] { Value::Text(s) => s.clone(), _ => String::new() }).collect();
		let mut keys: Vec<(i64, String)> = cols.iter().filter_map(|r| match (&r[0], &r[2]) {
			(Value::Text(n), Value::Integer(k)) if *k > 0 => Some((*k, q(n))), _ => None }).collect();
		keys.sort();
		let order = keys.iter().map(|(_, n)| n.clone()).collect::<Vec<_>>().join(", ");
		let mut out = String::new();
		let order = if order.is_empty() { "rowid".to_string() } else { order };
		for r in rows(c, &format!("SELECT * FROM {} ORDER BY {}", q(t), order)).await.map_err(|e| e.to_string())? {
			out.push_str(&r.iter().zip(decl.iter()).map(|(v, d)| copy_text(v, d)).collect::<Vec<_>>().join("\t"));
			out.push('\n');
		}
		Ok(out)
	}
}

/// A small generator of the load's values: the same for the same seed.
struct Lcg(u64);
impl Lcg {
	fn next(&mut self, n: u64) -> u64 {
		self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
		(self.0 >> 33) % n
	}
}

async fn run(m: &BTreeMap<String, String>) -> Result<(), String> {
	let dir = PathBuf::from(need(m, "dir"));
	std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
	let uuid = yt_core_proto::uuid::Uuid::parse_str(need(m, "uuid")).map_err(|e| format!("--uuid: {}", e))?;
	let uuid_s = uuid.to_string();
	let schema = need(m, "schema").to_string();
	let tx: u64 = need(m, "tx").parse().map_err(|_| "--tx: a number".to_string())?;
	let hosts: Vec<String> = need(m, "upper").split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
	let db = turso::Builder::new_local(need(m, "db")).build().await.map_err(|e| e.to_string())?;
	let store = Arc::new(Turso::open(&db, need(m, "cluster-db")).await?);
	// its own schema's tables are captured; the rest only come down
	// its own schema's tables are captured and sync both ways; every other table of the
	// database syncs too - an A row, no capture: what the cluster holds comes down to it
	let mut captured = 0;
	for t in ["accounts", "history"] {
		store.setup(&format!("{schema}.{t}")).await?;
		captured += 1;
	}
	{
		let c = db.connect().map_err(|e| e.to_string())?;
		let own = [format!("{schema}.accounts"), format!("{schema}.history")];
		for t in tables(&c).await? {
			// the two captured have their A rows from setup; every other table, of its own
			// schema too, syncs down (review of 2026-11-13)
			if !own.contains(&t) {
				store.sync(&t, "").await?;
			}
		}
	}
	let key_dir = m.get("key-dir").map(PathBuf::from).unwrap_or_else(|| dir.join("key"));
	let tls = Arc::new(yt_core::tls::Tls::open(&key_dir, &uuid_s, m.get("root").map(|s| s.as_str()))?);
	say(&format!("fingerprint {}", tls.fingerprint()));
	say(&format!("approve: ytctl --skv-approve {} --fingerprint {}", uuid_s, tls.fingerprint()));
	let node = Arc::new(Node::with_db(uuid.as_u128(), hosts, store.clone()).with_tls(tls.clone()));
	let status = Arc::new(Mutex::new(Status::default()));
	let stop = Arc::new(AtomicBool::new(false));
	let linker = tokio::spawn(leaf::run_until(node.clone(), status.clone(), |s| say(&s), stop.clone()));

	let app = db.connect().map_err(|e| e.to_string())?;
	app.busy_timeout(Duration::from_secs(10)).map_err(|e| e.to_string())?;
	let (a, h) = (q(&format!("{schema}.accounts")), q(&format!("{schema}.history")));
	let accounts = match rows(&app, &format!("SELECT count(*) FROM {a}")).await.map_err(|e| e.to_string())?.first() {
		Some(r) => match r[0] { Value::Integer(n) => n.max(1) as u64, _ => 1 }, None => 1 };
	let mut hid = match rows(&app, &format!("SELECT coalesce(max(hid), 0) FROM {h}")).await.map_err(|e| e.to_string())?.first() {
		Some(r) => match r[0] { Value::Integer(n) => n, _ => 0 }, None => 0 };
	let mut rng = Lcg(uuid.as_u128() as u64);
	let mut load = "waiting".to_string();
	let mut done = 0u64;
	let mut sums: BTreeMap<String, String> = BTreeMap::new();
	let mut turn = 0u64;
	loop {
		tokio::time::sleep(Duration::from_secs(1)).await;
		turn += 1;
		let st = status.lock().unwrap_or_else(|e| e.into_inner()).clone();
		// the load: once the test says go - it checked this node pulled what the cluster had -
		// and the node is linked over TLS. Not "caught up": under the cluster's own load a turn
		// down seldom finds nothing new (review of 2026-11-13)
		if done < tx && dir.join("go").exists() && st.tls {
			say(&format!("load: {} transactions on {}", tx, schema));
			while done < tx {
				let (aid, delta) = (rng.next(accounts) + 1, rng.next(10001) as i64 - 5000);
				hid += 1;
				let r = async {
					app.execute("BEGIN", ()).await?;
					app.execute(&format!("UPDATE {a} SET abalance = abalance + ? WHERE aid = ?"),
						vec![Value::Integer(delta), Value::Integer(aid as i64)]).await?;
					app.execute(&format!("INSERT INTO {h} (hid, aid, delta, mtime) VALUES (?, ?, ?, ?)"),
						vec![Value::Integer(hid), Value::Integer(aid as i64), Value::Integer(delta), Value::Text(now_text())]).await?;
					app.execute("COMMIT", ()).await
				}.await;
				if let Err(e) = r {
					let _ = app.execute("ROLLBACK", ()).await;
					say(&format!("load: a transaction failed: {}", e));
					// the history's last id and its rows as the database has them: a COMMIT that
					// failed may still have gone through - then it counts (review of 2026-11-13;
					// each transaction writes one history row, and the schema's history is this
					// load's alone)
					if let Some(r) = rows(&app, &format!("SELECT coalesce(max(hid), 0), count(*) FROM {h}")).await.map_err(|e| e.to_string())?.first() {
						if let (Value::Integer(m), Value::Integer(n)) = (&r[0], &r[1]) {
							hid = *m;
							done = (*n as u64).min(tx);
						}
					}
					tokio::time::sleep(Duration::from_millis(200)).await;
					continue;
				}
				done += 1;
			}
			load = format!("done {}", done);
			say(&format!("load: {} transactions done", done));
		}
		let mut s = format!("granted={}\ntls={}\ncaught_up={}\nwaiting={}\nturns={}\nload={}\nfingerprint={}\ncaptured={}\nnot_held={}\nnot_sent={}\nnot_synced={}\nmissed={}\napply_error={}\nproblem={}\n",
			st.granted, st.tls, st.caught_up, st.waiting, st.turns, load, tls.fingerprint(), captured,
			store.not_held.load(Ordering::Relaxed), store.not_sent.load(Ordering::Relaxed),
			store.not_synced.load(Ordering::Relaxed), store.missed.load(Ordering::Relaxed),
			store.apply_error.lock().map(|e| e.clone()).unwrap_or_default(), st.problem.unwrap_or_default().replace('\n', " "));
		for t in rows(&app, "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'yt\\_%' ESCAPE '\\' \
			AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' AND name <> 'common.big' ORDER BY name").await.map_err(|e| e.to_string())? {
			if let Value::Text(t) = &t[0]
				&& let Some(r) = rows(&app, &format!("SELECT count(*) FROM {}", q(t))).await.map_err(|e| e.to_string())?.first() {
				s.push_str(&format!("rows {}={}\n", t, match r[0] { Value::Integer(n) => n, _ => -1 }));
				// an accounts table's row 1 filler: what the test's closing update changes
				if t.ends_with(".accounts")
					&& let Some(f) = rows(&app, &format!("SELECT filler FROM {} WHERE aid = 1", q(t))).await.map_err(|e| e.to_string())?.first()
					&& let Value::Text(f) = &f[0] {
					s.push_str(&format!("filler1 {}={}\n", t, f.replace('\n', " ")));
				}
			}
		}
		// each table's digest - the md5 of its dump - every 5 seconds: what the test compares to
		// see it settled (review of 2026-11-13); not the big unchanging one
		if turn % 5 == 1 {
			for t in tables(&app).await? {
				if t != "common.big" {
					sums.insert(t.clone(), format!("{:x}", md5::compute(table_text(&app, &t).await?.as_bytes())));
				}
			}
		}
		for (t, m) in &sums {
			s.push_str(&format!("sum {}={}\n", t, m));
		}
		let tmp = dir.join("status.tmp");
		if std::fs::write(&tmp, s).is_ok() {
			let _ = std::fs::rename(&tmp, dir.join("status"));
		}
		if dir.join("stop").exists() {
			// what it wrote passed above, and what the cluster had applied here: up to 2 minutes
			for _ in 0..120 {
				let st = status.lock().unwrap_or_else(|e| e.into_inner()).clone();
				if store.waiting().await.unwrap_or(1) == 0 && st.caught_up {
					break;
				}
				tokio::time::sleep(Duration::from_secs(1)).await;
			}
			// stopped between two turns, never in an apply (review of 2026-11-13)
			stop.store(true, Ordering::Relaxed);
			let _ = linker.await;
			let n = dump(&app, &dir.join("dump")).await?;
			std::fs::write(dir.join("dumped"), format!("{}\n", n)).map_err(|e| e.to_string())?;
			say(&format!("stopped: {} tables dumped to {}", n, dir.join("dump").display()));
			return Ok(());
		}
	}
}

#[tokio::main]
async fn main() {
	let (cmd, m) = args();
	let r = match cmd.as_str() {
		"init" => init(&m).await,
		"run" => run(&m).await,
		"dump" => async {
			let db = turso::Builder::new_local(need(&m, "db")).build().await.map_err(|e| e.to_string())?;
			let c = db.connect().map_err(|e| e.to_string())?;
			let n = dump(&c, Path::new(need(&m, "dir"))).await?;
			say(&format!("dump: {} tables", n));
			Ok(())
		}.await,
		_ => Err("usage: yt-turso-load init|run --db <file> ... (src/main.rs says the rest)".to_string()),
	};
	if let Err(e) = r {
		eprintln!("yt-turso-load: {}", e);
		std::process::exit(1);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_time_is_written_as_postgresql_writes_a_timestamptz_in_utc() {
		assert_eq!(utc_text(0), "1970-01-01 00:00:00+00");
		assert_eq!(utc_text(1_794_600_000), "2026-11-13 20:00:00+00");
		assert_eq!(utc_text(951_782_400), "2000-02-29 00:00:00+00");
	}

	#[test]
	fn a_value_is_written_as_copy_writes_it() {
		assert_eq!(copy_text(&Value::Null, "text"), "\\N");
		assert_eq!(copy_text(&Value::Integer(1), "boolean"), "t");
		assert_eq!(copy_text(&Value::Integer(-7), "int"), "-7");
		assert_eq!(copy_text(&Value::Text("a\tb\\c\nd".into()), "text"), "a\\tb\\\\c\\nd");
		assert_eq!(md5hex("1"), "c4ca4238a0b923820dcc509a6f75849b", "md5('1') in PostgreSQL");
		assert_eq!(copy_text(&Value::Text("\u{8}\u{c}\u{b}".into()), "text"), "\\b\\f\\v");
		// as PostgreSQL writes a float8
		assert_eq!(float_text(1.5), "1.5");
		assert_eq!(float_text(3.0), "3");
		assert_eq!(float_text(1e20), "1e+20");
		assert_eq!(float_text(1.25e-5), "1.25e-05");
		assert_eq!(float_text(f64::NAN), "NaN");
		assert_eq!(copy_text(&Value::Real(-0.5), "double precision"), "-0.5");
	}

	/// the initial database as on the PostgreSQL nodes, and its dump as COPY writes theirs: the
	/// first row of each table by the same formulas (md5 of the id, the id x 7919 mod 100003)
	#[tokio::test]
	async fn init_makes_the_initial_database_and_dump_writes_it_as_copy() {
		let dir = std::env::temp_dir().join(format!("ytt-init-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let db = dir.join("x.db");
		let m: BTreeMap<String, String> = [("db", db.to_str().unwrap()), ("rows", "50"), ("accounts", "10"), ("schemas", "common,s_a,s_b")]
			.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
		init(&m).await.unwrap();
		let d = turso::Builder::new_local(db.to_str().unwrap()).build().await.unwrap();
		let c = d.connect().unwrap();
		assert_eq!(tables(&c).await.unwrap(), vec!["common.big", "s_a.accounts", "s_a.history", "s_b.accounts", "s_b.history"]);
		assert_eq!(dump(&c, &dir.join("dump")).await.unwrap(), 5);
		let big = std::fs::read_to_string(dir.join("dump/common.big.tsv")).unwrap();
		assert_eq!(big.lines().count(), 50);
		assert_eq!(big.lines().next().unwrap(), format!("1\t{}\t7919", md5hex("1")));
		let acc = std::fs::read_to_string(dir.join("dump/s_a.accounts.tsv")).unwrap();
		assert_eq!(acc.lines().count(), 10);
		assert_eq!(acc.lines().next().unwrap(), format!("1\t0\t{}", md5hex("1s_a")));
		assert_eq!(std::fs::read_to_string(dir.join("dump/s_b.history.tsv")).unwrap(), "", "no history before a load");
		// init again: the tables made anew, not added to
		init(&m).await.unwrap();
		assert_eq!(table_text(&c, "common.big").await.unwrap().lines().count(), 50);
		let _ = std::fs::remove_dir_all(&dir);
	}
}
