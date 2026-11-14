// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! yt-sqlite, the SQLite loadable extension (../yt-sqlite), as an application loads it: into
//! a SQLite of this process (YT_SQLITE_EXT, the built `libyt.so`; skipped without it), on a
//! database file, linked to a node above served here. A table set up there goes up, what the
//! node above holds comes down and does not go back up, yt_info() says where the link is,
//! and yt_unlink() stops it.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use yt_core_proto::base::{ColumnRef, TableRef, VarType, var_type::Value};
use yt_core_proto::db::{DbAction, DbInputEntity};
use yt_core_proto::link::link_server::{Link, LinkServer};
use yt_core_proto::link::{LinkGrant, LinkSignIn, RelayAck, RelayBatch, RelayFetch, RelayItem, RelayPos};

#[derive(Default, Clone)]
struct Above {
	down: Arc<Mutex<Vec<RelayItem>>>,
	took: Arc<Mutex<BTreeMap<(u64, u32), RelayItem>>>,
	at: Arc<Mutex<Option<(u64, u32)>>>,
}

#[tonic::async_trait]
impl Link for Above {
	async fn signin(&self, _r: tonic::Request<LinkSignIn>) -> Result<tonic::Response<LinkGrant>, tonic::Status> {
		Ok(tonic::Response::new(LinkGrant { granted: true, az: "top".into(), challenge: b"c".to_vec(), ..Default::default() }))
	}
	async fn fetch(&self, r: tonic::Request<RelayFetch>) -> Result<tonic::Response<RelayBatch>, tonic::Status> {
		let from = r.into_inner().from.map(|p| (p.term, p.subterm));
		let items: Vec<RelayItem> = self.down.lock().unwrap().iter().filter(|i| Some((i.term, i.subterm)) > from).cloned().collect();
		let upto = items.last().map(|i| (i.term, i.subterm)).or(from).unwrap_or((0, 0));
		Ok(tonic::Response::new(RelayBatch { items, upto: Some(RelayPos { term: upto.0, subterm: upto.1 }), ..Default::default() }))
	}
	async fn deliver(&self, r: tonic::Request<RelayBatch>) -> Result<tonic::Response<RelayAck>, tonic::Status> {
		let b = r.into_inner();
		if let Some(u) = b.upto {
			for i in b.items { self.took.lock().unwrap().insert((i.term, i.subterm), i); }
			*self.at.lock().unwrap() = Some((u.term, u.subterm));
		}
		let at = *self.at.lock().unwrap();
		Ok(tonic::Response::new(RelayAck { at: at.map(|p| RelayPos { term: p.0, subterm: p.1 }), ..Default::default() }))
	}
}

fn down_row(id: i64, v: &str) -> DbInputEntity {
	let cols = vec![ColumnRef { attnum: 1, name: "id".into(), pk: true, atttypid: 20, ..Default::default() },
		ColumnRef { attnum: 2, name: "v".into(), atttypid: 25, ..Default::default() }];
	let x = |v: Value| VarType { value: Some(v) };
	DbInputEntity { table: Some(TableRef { db: "appdb".into(), schema: "public".into(), table: "t".into(), columns: cols, ..Default::default() }),
		new_data: [(1, x(Value::Int(id))), (2, x(Value::Universal(v.into())))].into_iter().collect(),
		action: DbAction::Insert as i32, ..Default::default() }
}

fn info(c: &rusqlite::Connection) -> serde_json::Value {
	let s: String = c.query_row("SELECT yt_info()", [], |r| r.get(0)).unwrap();
	serde_json::from_str(&s).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sqlite_database_links_and_its_table_syncs_both_ways() {
	let Ok(ext) = std::env::var("YT_SQLITE_EXT") else {
		eprintln!("YT_SQLITE_EXT (yt-sqlite's built libyt.so) is not set: skipped");
		return;
	};
	let above = Above::default();
	let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let host = format!("http://{}", l.local_addr().unwrap());
	tokio::spawn(tonic::transport::Server::builder().add_service(LinkServer::new(above.clone()))
		.serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(l)));
	let dir = std::env::temp_dir().join(format!("yt-sqlite-{}-{}", std::process::id(),
		std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
	std::fs::create_dir_all(&dir).unwrap();
	// the file's name is the database's name in the cluster above when cluster_db is not set
	let c = rusqlite::Connection::open(dir.join("appdb.db")).unwrap();
	unsafe {
		c.load_extension_enable().unwrap();
		c.load_extension(&ext, Some("sqlite3_yt_init")).unwrap();
	}
	c.execute_batch("CREATE TABLE t (id bigint PRIMARY KEY, v text);").unwrap();
	let set: String = c.query_row("SELECT yt_setup('t')", [], |r| r.get(0)).unwrap();
	assert_eq!(set, "t");
	assert_eq!(info(&c)["linked"], false, "not linked before yt_link");
	c.execute("INSERT INTO yt_config (name, module, value) VALUES ('key_dir', 'L', ?1)", [dir.join("key").to_str().unwrap()]).unwrap();
	let fp: String = c.query_row("SELECT yt_link(?1, NULL)", [&host], |r| r.get(0)).unwrap();
	assert!(fp.starts_with("SHA256:"), "{fp}");
	// a second yt_link is the same link
	let again: String = c.query_row("SELECT yt_link()", [], |r| r.get(0)).unwrap();
	assert_eq!(again, fp);

	// up: a committed row goes to the node above; down: what it holds comes into the table
	c.execute("INSERT INTO t VALUES (1, 'up')", []).unwrap();
	above.down.lock().unwrap().push(RelayItem { term: 3, subterm: 1, data: vec![down_row(2, "down")] });
	let mut ok = (false, false);
	for _ in 0..150 {
		ok.0 = above.took.lock().unwrap().values().flat_map(|i| i.data.iter())
			.any(|e| e.table.as_ref().is_some_and(|t| t.db == "appdb" && t.schema == "public" && t.table == "t"));
		ok.1 = c.query_row("SELECT count(*) FROM t WHERE id = 2 AND v = 'down'", [], |r| r.get::<_, i64>(0)).unwrap() == 1;
		if ok == (true, true) { break }
		tokio::time::sleep(Duration::from_millis(200)).await;
	}
	assert_eq!(ok, (true, true), "a row went up, one came down");
	// what came down did not go back up: the outbox holds the application's row alone
	let n: i64 = c.query_row("SELECT count(*) FROM yt_outbox", [], |r| r.get(0)).unwrap();
	assert_eq!(n, 1);
	// the link's status is written at the end of its turn: the rows can cross in the turn
	// that signs in, a moment before yt_info says it is in
	let mut i = info(&c);
	for _ in 0..50 {
		if i["granted"] == true { break }
		tokio::time::sleep(Duration::from_millis(100)).await;
		i = info(&c);
	}
	assert_eq!(i["linked"], true, "{i}");
	assert_eq!(i["granted"], true, "{i}");
	assert_eq!(i["cluster_db"], "appdb", "{i}");
	assert_eq!(i["fingerprint"], fp.as_str(), "{i}");
	assert_eq!(i["not_held"], 0, "{i}");
	// settings given again: the link made again with them - the same CA, its key_dir and uuid kept
	let re: String = c.query_row("SELECT yt_link(?1, NULL)", [&host], |r| r.get(0)).unwrap();
	assert_eq!(re, fp);
	assert_eq!(info(&c)["linked"], true);
	c.execute("INSERT INTO t VALUES (3, 'after the relink')", []).unwrap();
	let mut up = false;
	for _ in 0..150 {
		up = above.took.lock().unwrap().values().flat_map(|i| i.data.iter())
			.any(|e| e.new_data.values().any(|v| v.value == Some(Value::Universal("after the relink".into()))));
		if up { break }
		tokio::time::sleep(Duration::from_millis(200)).await;
	}
	assert!(up, "a row went up after the relink");

	let stopped: bool = c.query_row("SELECT yt_unlink()", [], |r| r.get(0)).unwrap();
	assert!(stopped);
	assert_eq!(info(&c)["linked"], false);
	let twice: bool = c.query_row("SELECT yt_unlink()", [], |r| r.get(0)).unwrap();
	assert!(!twice, "not linked any more");
	let _ = std::fs::remove_dir_all(&dir);
}

fn err(c: &rusqlite::Connection, sql: &str) -> String {
	c.query_row(sql, [], |r| r.get::<_, rusqlite::types::Value>(0)).err().map(|e| e.to_string())
		.unwrap_or_else(|| panic!("{sql}: no error"))
}

/// what it refuses, with its code: an A row value it cannot read, a link with no cluster
/// above, a root that is no fingerprint, a link inside a transaction (a rollback would take
/// its settings and uuid), a database with no file, a table with no key, a call from a trigger
#[test]
fn its_mistakes_are_refused_with_their_codes() {
	let Ok(ext) = std::env::var("YT_SQLITE_EXT") else {
		eprintln!("YT_SQLITE_EXT (yt-sqlite's built libyt.so) is not set: skipped");
		return;
	};
	let dir = std::env::temp_dir().join(format!("yt-sqlite-no-{}-{}", std::process::id(),
		std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
	std::fs::create_dir_all(&dir).unwrap();
	let load = |c: &rusqlite::Connection| unsafe {
		c.load_extension_enable().unwrap();
		c.load_extension(&ext, Some("sqlite3_yt_init")).unwrap();
	};
	let c = rusqlite::Connection::open(dir.join("app.db")).unwrap();
	load(&c);
	c.execute_batch("CREATE TABLE t (id bigint PRIMARY KEY, v text); CREATE TABLE nk (a int);").unwrap();
	let e = err(&c, "SELECT yt_sync('t', 'not base64 at all!')");
	assert!(e.contains("YT-E085"), "{e}");
	let n: i64 = c.query_row("SELECT count(*) FROM yt_config WHERE module = 'A'", [], |r| r.get(0)).unwrap();
	assert_eq!(n, 0, "nothing written for it");
	let e = err(&c, "SELECT yt_link()");
	assert!(e.contains("YT-E083"), "{e}");
	let e = err(&c, "SELECT yt_link('http://127.0.0.1:1', 'SHA256:zzz')");
	assert!(e.contains("YT-E086"), "{e}");
	c.execute_batch("BEGIN").unwrap();
	let e = err(&c, "SELECT yt_link('http://127.0.0.1:1', NULL)");
	assert!(e.contains("YT-E084"), "{e}");
	c.execute_batch("ROLLBACK").unwrap();
	assert_eq!(info(&c)["linked"], false);
	let e = err(&c, "SELECT yt_setup('nk')");
	assert!(e.contains("YT-E080"), "{e}");
	let e = err(&c, "SELECT yt_setup('absent')");
	assert!(e.contains("YT-E079"), "{e}");
	// from a trigger: not allowed (SQLITE_DIRECTONLY), the function has side effects
	c.execute_batch("CREATE TABLE tr (id int PRIMARY KEY); CREATE TRIGGER tr_i AFTER INSERT ON tr BEGIN SELECT yt_setup('t'); END;").unwrap();
	assert!(c.execute("INSERT INTO tr VALUES (1)", []).is_err(), "yt_setup from a trigger");
	let m = rusqlite::Connection::open_in_memory().unwrap();
	load(&m);
	let e = err(&m, "SELECT yt_link('http://127.0.0.1:1', NULL)");
	assert!(e.contains("YT-E082"), "{e}");
	drop(c);
	let _ = std::fs::remove_dir_all(&dir);
}
