// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! The lite node inside an application, on Turso: the application's own database is the
//! node's store (yt-lite.md 8) - crate::lite's, on a Turso connection of this node's own.
//! What it captures, how rows come down, which tables sync: crate::lite.
use std::time::Duration;

use turso::{Connection, Database, Value as TV};

pub use crate::lite::{cluster_name, id_of, kind, local_name, pos_of, statement, Col, Kind, Lite, Val};
#[allow(unused_imports)]
pub(crate) use crate::lite::{e, int, text, value, var, Val as Value};
#[allow(unused_imports)]
use crate::lite::Conn;
#[allow(unused_imports)]
use yt_core_proto::base::{var_type::Value as V, ColumnRef, TableRef, VarType};
#[allow(unused_imports)]
use yt_core_proto::db::{DbAction, DbInputEntity};
#[allow(unused_imports)]
use yt_core_proto::MAX_SUBTERM;
#[allow(unused_imports)]
use yt_core_proto::link::{RelayBatch, RelayItem};
#[allow(unused_imports)]
use std::sync::atomic::Ordering;

/// The store on Turso.
pub type Turso = Lite<Connection>;

fn tv(v: Val) -> TV {
	match v {
		Val::Null => TV::Null,
		Val::Integer(i) => TV::Integer(i),
		Val::Real(f) => TV::Real(f),
		Val::Text(s) => TV::Text(s),
		Val::Blob(b) => TV::Blob(b),
	}
}

fn val(v: TV) -> Val {
	match v {
		TV::Null => Val::Null,
		TV::Integer(i) => Val::Integer(i),
		TV::Real(f) => Val::Real(f),
		TV::Text(s) => Val::Text(s),
		TV::Blob(b) => Val::Blob(b),
	}
}

#[tonic::async_trait]
impl Conn for Connection {
	async fn exec(&self, sql: &str, params: Vec<Val>) -> Result<u64, String> {
		self.execute(sql, params.into_iter().map(tv).collect::<Vec<_>>()).await.map_err(|x| x.to_string())
	}
	async fn batch(&self, sql: &str) -> Result<(), String> {
		self.execute_batch(sql).await.map_err(|x| x.to_string())
	}
	async fn rows(&self, sql: &str, params: Vec<Val>) -> Result<Vec<Vec<Val>>, String> {
		let mut r = self.query(sql, params.into_iter().map(tv).collect::<Vec<_>>()).await.map_err(|x| x.to_string())?;
		let mut out = Vec::new();
		while let Some(row) = r.next().await.map_err(|x| x.to_string())? {
			let mut v = Vec::with_capacity(row.column_count());
			for i in 0..row.column_count() {
				v.push(val(row.get_value(i).map_err(|x| x.to_string())?));
			}
			out.push(v);
		}
		Ok(out)
	}
}

/// A query's rows on a Turso connection.
#[allow(dead_code)]
pub(crate) async fn all(c: &Connection, sql: &str, params: Vec<Val>) -> Result<Vec<Vec<Val>>, String> {
	c.rows(sql, params).await
}

impl Lite<Connection> {
	/// This node's store in `db`, the application's database: a connection of its own, and
	/// its tables. `cluster_db`: this database's name in the cluster above.
	pub async fn open(db: &Database, cluster_db: &str) -> Result<Turso, String> {
		let conn = db.connect().map_err(e("connect"))?;
		// one writer at a time: the application's writes and this node's applies wait for each other
		conn.busy_timeout(Duration::from_secs(10)).map_err(e("busy_timeout"))?;
		Lite::new(conn, cluster_db).await
	}
}

/// [migrate] the Turso form's database for a migration: SQLite 3, deployed by schema_guard_tokio
#[tonic::async_trait]
impl crate::migrate::Target for Lite<Connection> {
	async fn source(&self) -> Result<schema_guard_tokio::mapping::Source, String> {
		schema_guard_tokio::mapping::Source::parse("sqlite.3")
	}
	async fn deploy(&self, y: yaml_rust2::Yaml, file: &str) -> Result<Vec<String>, String> {
		let c = self.conn().await;
		schema_guard_tokio::sqlite::migrate(y, &*c, false, file, &Default::default()).await.map(|p| p.statements)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::db::Db;
	use yt_core_proto::link::RelayPos;

	#[test]
	fn a_position_is_an_outbox_row_and_back() {
		assert_eq!(pos_of(0), (0, 0));
		assert_eq!(pos_of(1), (1, 1));
		let m = MAX_SUBTERM as u64;
		assert_eq!(pos_of(m), (1, MAX_SUBTERM));
		assert_eq!(pos_of(m + 1), (2, 1));
		for id in [1, 2, 7, m - 1, m, m + 1, 3 * m + 5] {
			assert_eq!(id_of(pos_of(id)), id);
			assert!(pos_of(id) < pos_of(id + 1), "in order");
		}
	}

	#[test]
	fn a_table_of_a_schema_is_named_with_it_here() {
		assert_eq!(local_name("s_t1", "accounts"), "s_t1.accounts");
		assert_eq!(local_name("public", "t"), "t");
		assert_eq!(cluster_name("s_t1.accounts"), ("s_t1".to_string(), "accounts".to_string()));
		assert_eq!(cluster_name("t"), ("public".to_string(), "t".to_string()));
	}

	/// as the extension sends a value of each type (ytpgc yt_cluster.c), and the type id
	/// the cluster above binds it by
	#[test]
	fn a_declared_type_gives_the_value_and_the_type_id_postgresql_has() {
		assert_eq!(kind("int"), (Kind::Int, 23));
		assert_eq!(kind("INTEGER"), (Kind::Int, 23));
		assert_eq!(kind("bigint"), (Kind::Int, 20));
		assert_eq!(kind("smallint"), (Kind::Int, 21));
		assert_eq!(kind("double precision"), (Kind::Flt, 701));
		assert_eq!(kind("boolean"), (Kind::Bool, 16));
		assert_eq!(kind("text"), (Kind::Text, 25));
		assert_eq!(kind("varchar(20)"), (Kind::Text, 1043));
		assert_eq!(kind("timestamptz"), (Kind::Text, 1184));
		assert_eq!(kind("numeric(10,2)"), (Kind::Text, 1700));
		assert_eq!(kind("whatever"), (Kind::Text, 0), "unknown: text, no type id - the cluster above takes it as text");
		assert_eq!(var(&serde_json::json!(5), Kind::Int).value, Some(V::Int(5)));
		assert_eq!(var(&serde_json::json!("7"), Kind::Int).value, Some(V::Int(7)));
		assert_eq!(var(&serde_json::json!(1), Kind::Bool).value, Some(V::Bool(true)));
		assert_eq!(var(&serde_json::json!(2.5), Kind::Flt).value, Some(V::Flt(2.5)));
		assert_eq!(var(&serde_json::json!(3), Kind::Text).value, Some(V::Universal("3".into())));
		assert_eq!(var(&serde_json::Value::Null, Kind::Int).value, None);
		assert_eq!(value(&VarType { value: Some(V::Universal("t".into())) }, "boolean").unwrap(), Value::Integer(1));
		assert_eq!(value(&VarType { value: Some(V::Int(4)) }, "int").unwrap(), Value::Integer(4));
		assert_eq!(value(&VarType { value: None }, "text").unwrap(), Value::Null);
	}

	async fn db() -> (Database, std::path::PathBuf) {
		let path = std::env::temp_dir().join(format!("yt-turso-{}-{}.db", std::process::id(),
			std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
		let db = turso::Builder::new_local(path.to_str().unwrap()).build().await.unwrap();
		(db, path)
	}

	async fn q(c: &Connection, sql: &str) -> Vec<Vec<Value>> {
		all(c, sql, vec![]).await.unwrap()
	}

	fn row(schema: &str, table: &str, db: &str, action: DbAction, cols: &[(&str, u32, bool, u32)], old: &[(u32, V)], new: &[(u32, V)]) -> DbInputEntity {
		DbInputEntity { table: Some(TableRef { db: db.into(), schema: schema.into(), table: table.into(),
			columns: cols.iter().map(|(n, a, pk, ty)| ColumnRef { attnum: *a, name: n.to_string(), pk: *pk, atttypid: *ty, ..Default::default() }).collect(),
			..Default::default() }),
			old_data: old.iter().map(|(k, v)| (*k, VarType { value: Some(v.clone()) })).collect(),
			new_data: new.iter().map(|(k, v)| (*k, VarType { value: Some(v.clone()) })).collect(),
			action: action as i32, ..Default::default() }
	}

	/// what the application writes is captured, committed only, and goes up as the extension
	/// would send it; what comes down is applied once, the triggers' copies of it not sent
	/// back; a table not here, or another database's, is skipped and counted
	#[tokio::test]
	async fn rows_go_up_as_the_extension_sends_them_and_come_down_once() {
		let (db, path) = db().await;
		let app = db.connect().unwrap();
		app.execute_batch("CREATE TABLE \"s_t.accounts\" (aid int PRIMARY KEY, abalance int NOT NULL DEFAULT 0, filler text NOT NULL);
			CREATE TABLE \"s_t.history\" (hid integer PRIMARY KEY, aid int NOT NULL, delta int NOT NULL, mtime timestamptz);
			CREATE TABLE \"s_u.accounts\" (aid int PRIMARY KEY, abalance int NOT NULL DEFAULT 0, filler text NOT NULL);
			CREATE TABLE nokey (v text);").await.unwrap();
		let s = Turso::open(&db, "appdb").await.unwrap();
		s.setup("s_t.accounts").await.unwrap();
		s.setup("s_t.history").await.unwrap();
		// the tables rows come down to sync: an A row each (s_x.absent's names a table not here)
		s.sync("s_u.accounts", "").await.unwrap();
		s.sync("s_x.absent", "").await.unwrap();
		assert!(s.setup("nokey").await.unwrap_err().starts_with("YT-E080"), "a table without a key is refused");
		assert!(s.setup("absent").await.unwrap_err().starts_with("YT-E079"));
		s.setup("s_t.accounts").await.unwrap(); // again: made again, not twice

		// the application's writes: a transaction, and one rolled back
		app.execute_batch("BEGIN; INSERT INTO \"s_t.accounts\" VALUES (1, 0, 'a'), (2, 0, 'b');
			UPDATE \"s_t.accounts\" SET abalance = 5 WHERE aid = 1;
			INSERT INTO \"s_t.history\" VALUES (1, 1, 5, '2026-11-13 18:00:00+00');
			DELETE FROM \"s_t.accounts\" WHERE aid = 2; COMMIT;").await.unwrap();
		app.execute_batch("BEGIN; INSERT INTO \"s_t.accounts\" VALUES (3, 0, 'gone'); ROLLBACK;").await.unwrap();
		assert_eq!(s.waiting().await.unwrap(), 5, "five rows, the rolled back one not among them");

		let (items, upto, more) = s.collect(None, 3).await.unwrap();
		assert_eq!((items.len(), upto, more), (3, (1, 3), true), "three, and more left");
		let (items, upto, more) = s.collect(None, 500).await.unwrap();
		assert_eq!((items.len(), upto, more), (5, (1, 5), false));
		let first = &items[0].data[0];
		let t = first.table.as_ref().unwrap();
		assert_eq!((t.db.as_str(), t.schema.as_str(), t.table.as_str()), ("appdb", "s_t", "accounts"));
		assert_eq!(t.columns.iter().map(|c| (c.attnum, c.name.as_str(), c.pk, c.atttypid)).collect::<Vec<_>>(),
			vec![(1, "aid", true, 23), (2, "abalance", false, 23), (3, "filler", false, 25)]);
		assert_eq!(first.action, DbAction::Insert as i32);
		assert_eq!(first.new_data[&1].value, Some(V::Int(1)));
		assert_eq!(first.new_data[&3].value, Some(V::Universal("a".into())));
		let upd = &items[2].data[0];
		assert_eq!(upd.action, DbAction::Update as i32);
		assert_eq!((upd.old_data[&2].value.clone(), upd.new_data[&2].value.clone()), (Some(V::Int(0)), Some(V::Int(5))));
		let hist = &items[3].data[0];
		assert_eq!(hist.table.as_ref().unwrap().columns[3].atttypid, 1184, "timestamptz");
		assert_eq!(hist.new_data[&4].value, Some(V::Universal("2026-11-13 18:00:00+00".into())));
		assert_eq!(items[4].data[0].action, DbAction::Delete as i32);
		let (none, upto, _) = s.collect(Some((1, 5)), 500).await.unwrap();
		assert!(none.is_empty() && upto == (1, 5), "nothing after the last");
		s.passed((1, 3)).await.unwrap();
		assert_eq!(s.waiting().await.unwrap(), 2, "three passed above");

		// down: rows of a table here (captured or not), of a table not here, of another database
		let cols = [("aid", 1, true, 23), ("abalance", 2, false, 23), ("filler", 3, false, 25)];
		let batch = RelayBatch { items: vec![
			RelayItem { term: 7, subterm: 1, data: vec![
				row("s_u", "accounts", "appdb", DbAction::Insert, &cols, &[], &[(1, V::Int(10)), (2, V::Int(0)), (3, V::Universal("u".into()))]),
				row("s_u", "accounts", "appdb", DbAction::Update, &cols, &[(1, V::Int(10))], &[(1, V::Int(10)), (2, V::Int(7))]),
				row("s_t", "accounts", "appdb", DbAction::Insert, &cols, &[], &[(1, V::Int(20)), (2, V::Int(1)), (3, V::Universal("down".into()))]),
				row("s_x", "absent", "appdb", DbAction::Insert, &cols, &[], &[(1, V::Int(1))]),
				row("s_u", "accounts", "otherdb", DbAction::Insert, &cols, &[], &[(1, V::Int(99))]),
			]},
			RelayItem { term: 7, subterm: 2, data: vec![
				row("s_u", "accounts", "appdb", DbAction::Delete, &cols, &[(1, V::Int(11))], &[]),
			]},
		], upto: Some(RelayPos { term: 7, subterm: 2 }), ..Default::default() };
		assert_eq!(s.apply(&batch).await.unwrap(), Some((7, 2)));
		assert_eq!(s.cursor().await.unwrap(), Some((7, 2)));
		assert_eq!(q(&app, "SELECT aid, abalance, filler FROM \"s_u.accounts\"").await,
			vec![vec![Value::Integer(10), Value::Integer(7), Value::Text("u".into())]]);
		assert_eq!(q(&app, "SELECT filler FROM \"s_t.accounts\" WHERE aid = 20").await, vec![vec![Value::Text("down".into())]]);
		assert_eq!(s.not_held.load(Ordering::Relaxed), 2, "a table not here, another database's row");
		assert_eq!(s.missed.load(Ordering::Relaxed), 1, "the delete of a row not here");
		let (up, _, _) = s.collect(Some((1, 5)), 500).await.unwrap();
		assert!(up.is_empty(), "what came down to a captured table does not go back up: {:?}", up);
		// the same batch again: nothing
		assert_eq!(s.apply(&batch).await.unwrap(), Some((7, 2)));
		assert_eq!(q(&app, "SELECT count(*) FROM \"s_t.accounts\" WHERE aid = 20").await, vec![vec![Value::Integer(1)]]);
		assert_eq!(s.missed.load(Ordering::Relaxed), 1);
		// the application's next write goes up after what is there
		app.execute("UPDATE \"s_t.accounts\" SET abalance = 9 WHERE aid = 1", ()).await.unwrap();
		let (up, upto, _) = s.collect(Some((1, 5)), 500).await.unwrap();
		assert_eq!(up.len(), 1);
		assert!(upto > (1, 5), "{:?}", upto);
		drop(app);
		let _ = std::fs::remove_file(&path);
	}

	/// a batch that cannot be applied is refused whole: its first rows are not left behind
	#[tokio::test]
	async fn a_batch_that_fails_is_applied_not_at_all() {
		let (db, path) = db().await;
		let app = db.connect().unwrap();
		app.execute("CREATE TABLE \"s_u.accounts\" (aid int PRIMARY KEY, abalance int NOT NULL, filler text NOT NULL)", ()).await.unwrap();
		let s = Turso::open(&db, "appdb").await.unwrap();
		s.sync("s_u.accounts", "").await.unwrap();
		let cols = [("aid", 1, true, 23), ("abalance", 2, false, 23), ("filler", 3, false, 25), ("nothere", 4, false, 25)];
		let batch = RelayBatch { items: vec![RelayItem { term: 1, subterm: 1, data: vec![
			row("s_u", "accounts", "appdb", DbAction::Insert, &cols, &[], &[(1, V::Int(1)), (2, V::Int(0)), (3, V::Universal("a".into()))]),
			row("s_u", "accounts", "appdb", DbAction::Insert, &cols, &[], &[(1, V::Int(2)), (4, V::Universal("x".into()))]),
		]}], upto: Some(RelayPos { term: 1, subterm: 1 }), ..Default::default() };
		let err = s.apply(&batch).await.unwrap_err();
		assert!(err.contains("nothere"), "{err}");
		assert_eq!(s.cursor().await.unwrap(), None);
		assert_eq!(q(&app, "SELECT count(*) FROM \"s_u.accounts\"").await, vec![vec![Value::Integer(0)]]);
		assert!(!s.apply_error.lock().unwrap().is_empty());
		drop(app);
		let _ = std::fs::remove_file(&path);
	}

	/// a row that fails when it runs - not while the batch is planned - takes the rows before
	/// it in the batch back with it: the rollback runs (review of 2026-11-13)
	#[tokio::test]
	async fn a_batch_that_fails_as_it_runs_is_rolled_back_whole() {
		let (db, path) = db().await;
		let app = db.connect().unwrap();
		app.execute("CREATE TABLE \"s_u.accounts\" (aid int PRIMARY KEY, abalance int NOT NULL, filler text NOT NULL)", ()).await.unwrap();
		let s = Turso::open(&db, "appdb").await.unwrap();
		s.sync("s_u.accounts", "").await.unwrap();
		let cols = [("aid", 1, true, 23), ("abalance", 2, false, 23), ("filler", 3, false, 25)];
		let batch = RelayBatch { items: vec![RelayItem { term: 1, subterm: 1, data: vec![
			row("s_u", "accounts", "appdb", DbAction::Insert, &cols, &[], &[(1, V::Int(1)), (2, V::Int(0)), (3, V::Universal("a".into()))]),
			// no abalance: NOT NULL refuses it when it runs
			row("s_u", "accounts", "appdb", DbAction::Insert, &cols, &[], &[(1, V::Int(2)), (3, V::Universal("b".into()))]),
		]}], upto: Some(RelayPos { term: 1, subterm: 1 }), ..Default::default() };
		assert!(s.apply(&batch).await.is_err());
		assert_eq!(s.cursor().await.unwrap(), None);
		assert_eq!(q(&app, "SELECT count(*) FROM \"s_u.accounts\"").await, vec![vec![Value::Integer(0)]], "the first row went with it");
		// and the connection is out of the transaction: the next batch applies
		let ok = RelayBatch { items: vec![RelayItem { term: 1, subterm: 1, data: vec![
			row("s_u", "accounts", "appdb", DbAction::Insert, &cols, &[], &[(1, V::Int(1)), (2, V::Int(0)), (3, V::Universal("a".into()))]),
		]}], upto: Some(RelayPos { term: 1, subterm: 1 }), ..Default::default() };
		assert_eq!(s.apply(&ok).await.unwrap(), Some((1, 1)));
		drop(app);
		let _ = std::fs::remove_file(&path);
	}

	/// what PostgreSQL's side applies, this one does too: a table with no key (by rowid), a
	/// float that is not finite, a bytea; a number that does not read as one is refused
	#[tokio::test]
	async fn values_and_tables_postgresql_takes_are_taken_here() {
		let (db, path) = db().await;
		let app = db.connect().unwrap();
		app.execute_batch("CREATE TABLE \"s_u.log\" (n int, note text);
			CREATE TABLE \"s_u.f\" (id int PRIMARY KEY, x double precision, b bytea);").await.unwrap();
		app.execute_batch("INSERT INTO \"s_u.log\" VALUES (1, 'a'), (1, 'a'), (2, 'b');").await.unwrap();
		let s = Turso::open(&db, "appdb").await.unwrap();
		s.sync("s_u.log", "").await.unwrap();
		s.sync("s_u.f", "").await.unwrap();
		let lc = [("n", 1, false, 23), ("note", 2, false, 25)];
		let fc = [("id", 1, true, 23), ("x", 2, false, 701), ("b", 3, false, 17)];
		let batch = RelayBatch { items: vec![RelayItem { term: 1, subterm: 1, data: vec![
			row("s_u", "log", "appdb", DbAction::Delete, &lc, &[(1, V::Int(1)), (2, V::Universal("a".into()))], &[]),
			row("s_u", "log", "appdb", DbAction::Update, &lc, &[(1, V::Int(2)), (2, V::Universal("b".into()))], &[(1, V::Int(2)), (2, V::Universal("c".into()))]),
			row("s_u", "f", "appdb", DbAction::Insert, &fc, &[], &[(1, V::Int(1)), (2, V::Flt(f64::NAN)), (3, V::Universal("\\x00ff".into()))]),
			row("s_u", "f", "appdb", DbAction::Insert, &fc, &[], &[(1, V::Int(2)), (2, V::Flt(f64::INFINITY))]),
		]}], upto: Some(RelayPos { term: 1, subterm: 1 }), ..Default::default() };
		assert_eq!(s.apply(&batch).await.unwrap(), Some((1, 1)));
		assert_eq!(q(&app, "SELECT n, note FROM \"s_u.log\" ORDER BY n, note").await,
			vec![vec![Value::Integer(1), Value::Text("a".into())], vec![Value::Integer(2), Value::Text("c".into())]], "one of the two equal rows deleted");
		assert_eq!(q(&app, "SELECT x, b FROM \"s_u.f\" ORDER BY id").await, vec![
			vec![Value::Text("NaN".into()), Value::Blob(vec![0, 255])],
			vec![Value::Text("Infinity".into()), Value::Null]]);
		let bad = RelayBatch { items: vec![RelayItem { term: 1, subterm: 2, data: vec![
			row("s_u", "f", "appdb", DbAction::Insert, &fc, &[], &[(1, V::Int(3)), (2, V::Universal("not a number".into()))]),
		]}], upto: Some(RelayPos { term: 1, subterm: 2 }), ..Default::default() };
		assert!(s.apply(&bad).await.unwrap_err().starts_with("YT-E081 not a number"));
		drop(app);
		let _ = std::fs::remove_file(&path);
	}

	/// a BLOB and an infinite REAL are captured as PostgreSQL writes them; a captured row of a
	/// table dropped since is passed over, and the rows after it still go up
	#[tokio::test]
	async fn what_json_cannot_hold_is_captured_and_a_dropped_table_holds_nothing_back() {
		let (db, path) = db().await;
		let app = db.connect().unwrap();
		app.execute_batch("CREATE TABLE \"s_t.f\" (id int PRIMARY KEY, x double precision, b bytea);
			CREATE TABLE \"s_t.gone\" (id int PRIMARY KEY);").await.unwrap();
		let s = Turso::open(&db, "appdb").await.unwrap();
		s.setup("s_t.f").await.unwrap();
		s.setup("s_t.gone").await.unwrap();
		app.execute("INSERT INTO \"s_t.f\" VALUES (1, 9e999, x'00ff')", ()).await.unwrap();
		app.execute("INSERT INTO \"s_t.gone\" VALUES (1)", ()).await.unwrap();
		app.execute_batch("DROP TABLE \"s_t.gone\";").await.unwrap();
		app.execute("INSERT INTO \"s_t.f\" VALUES (2, 1.5, NULL)", ()).await.unwrap();
		let (items, upto, _) = s.collect(None, 500).await.unwrap();
		assert_eq!(upto, (1, 3), "all three positions");
		assert_eq!(items[0].data[0].new_data[&2].value, Some(V::Universal("Infinity".into())));
		assert_eq!(items[0].data[0].new_data[&3].value, Some(V::Universal("\\x00ff".into())));
		// this node still knows the dropped table's columns: its row, committed before the
		// drop, goes up as it is
		assert_eq!(items[1].data.len(), 1);
		assert_eq!(items[2].data[0].new_data[&2].value, Some(V::Flt(1.5)));
		// after a restart nothing knows them: its position is passed over, counted, and the
		// row after it still goes
		let fresh = Turso::open(&db, "appdb").await.unwrap();
		let (items, upto, _) = fresh.collect(None, 500).await.unwrap();
		assert_eq!(upto, (1, 3));
		assert!(items[1].data.is_empty(), "the dropped table's row: passed over");
		assert_eq!(fresh.not_sent.load(Ordering::Relaxed), 1);
		assert_eq!(items[2].data[0].new_data[&2].value, Some(V::Flt(1.5)));
		drop(app);
		let _ = std::fs::remove_file(&path);
	}

	/// a table syncs as on a cluster node: by its A row, the ways the row's value says; with
	/// none, nothing of it moves - each such row counted
	#[tokio::test]
	async fn a_table_syncs_the_ways_its_a_row_says() {
		use base64::Engine;
		use prost::Message;
		let enc = |no_up: bool, no_down: bool| base64::engine::general_purpose::STANDARD.encode(
			yt_core_proto::link::TableLink { az_routing: Some(yt_core_proto::link::AzRouting { no_up, no_down, ..Default::default() }) }.encode_to_vec());
		let (db, path) = db().await;
		let app = db.connect().unwrap();
		for t in ["s_a.t", "s_b.t", "s_c.t", "s_d.t"] {
			app.execute(&format!("CREATE TABLE \"{t}\" (id int PRIMARY KEY, v text)"), ()).await.unwrap();
		}
		let s = Turso::open(&db, "appdb").await.unwrap();
		s.setup("s_a.t").await.unwrap();                       // both ways
		s.sync("s_b.t", &enc(false, true)).await.unwrap();     // no capture, and no_down
		s.setup("s_c.t").await.unwrap();
		s.sync("s_c.t", &enc(true, false)).await.unwrap();     // captured, but no_up
		// s_d.t: no A row
		assert_eq!(q(&app, "SELECT name, module FROM yt_config ORDER BY name").await.len(), 3);
		for t in ["s_a.t", "s_b.t", "s_c.t", "s_d.t"] {
			app.execute(&format!("INSERT INTO \"{t}\" VALUES (1, 'here')"), ()).await.unwrap();
		}
		let (items, upto, _) = s.collect(None, 500).await.unwrap();
		let sent: Vec<&str> = items.iter().flat_map(|i| i.data.iter()).map(|r| r.table.as_ref().unwrap().schema.as_str()).collect();
		assert_eq!(sent, vec!["s_a"], "only the table that syncs up; s_b and s_d capture nothing, s_c's row stays");
		assert_eq!(upto, (1, 2), "s_c's position passed over");
		assert_eq!(s.not_synced.load(Ordering::Relaxed), 1);
		let cols = [("id", 1, true, 23), ("v", 2, false, 25)];
		let down = |schema: &str| row(schema, "t", "appdb", DbAction::Insert, &cols, &[], &[(1, V::Int(2)), (2, V::Universal("down".into()))]);
		let batch = RelayBatch { items: vec![RelayItem { term: 1, subterm: 1, data: vec![down("s_a"), down("s_b"), down("s_c"), down("s_d")] }],
			upto: Some(RelayPos { term: 1, subterm: 1 }), ..Default::default() };
		assert_eq!(s.apply(&batch).await.unwrap(), Some((1, 1)));
		for (t, n) in [("s_a.t", 2), ("s_b.t", 1), ("s_c.t", 2), ("s_d.t", 1)] {
			assert_eq!(q(&app, &format!("SELECT count(*) FROM \"{t}\"")).await, vec![vec![Value::Integer(n)]], "{t}");
		}
		assert_eq!(s.not_synced.load(Ordering::Relaxed), 3, "s_b (no_down) and s_d (no A row) not applied, besides s_c's row");
		// unsync: neither way any more
		s.unsync("s_a.t").await.unwrap();
		app.execute("INSERT INTO \"s_a.t\" VALUES (3, 'after')", ()).await.unwrap();
		let (items, _, _) = s.collect(Some(upto), 500).await.unwrap();
		assert!(items.iter().all(|i| i.data.is_empty()), "no trigger, no row");
		let batch = RelayBatch { items: vec![RelayItem { term: 1, subterm: 2, data: vec![row("s_a", "t", "appdb", DbAction::Insert, &cols, &[],
			&[(1, V::Int(4)), (2, V::Universal("down".into()))])] }], upto: Some(RelayPos { term: 1, subterm: 2 }), ..Default::default() };
		s.apply(&batch).await.unwrap();
		assert_eq!(q(&app, "SELECT count(*) FROM \"s_a.t\" WHERE id = 4").await, vec![vec![Value::Integer(0)]]);
		drop(app);
		let _ = std::fs::remove_file(&path);
	}

	/// a REAL goes up as it is, not rounded to the 15 digits json_object writes (review of
	/// 2026-11-13); and a table named the cluster's way is the same table
	#[tokio::test]
	async fn a_real_goes_up_whole_and_a_table_answers_to_either_name() {
		let (db, path) = db().await;
		let app = db.connect().unwrap();
		app.execute("CREATE TABLE t (id int PRIMARY KEY, x double precision)", ()).await.unwrap();
		let s = Turso::open(&db, "appdb").await.unwrap();
		s.setup("public.t").await.unwrap();
		app.execute("INSERT INTO t VALUES (1, 0.1 + 0.2)", ()).await.unwrap();
		let (items, _, _) = s.collect(None, 500).await.unwrap();
		assert_eq!(items[0].data[0].new_data[&2].value, Some(V::Flt(0.1 + 0.2)), "0.30000000000000004, not 0.3");
		// unsync by the cluster's name takes the local table's triggers too
		s.unsync("public.t").await.unwrap();
		app.execute("INSERT INTO t VALUES (2, 1.0)", ()).await.unwrap();
		assert_eq!(q(&app, "SELECT count(*) FROM yt_outbox WHERE id > 1").await, vec![vec![Value::Integer(0)]], "no trigger left");
		assert_eq!(q(&app, "SELECT count(*) FROM yt_config").await, vec![vec![Value::Integer(0)]]);
		drop(app);
		let _ = std::fs::remove_file(&path);
	}

	/// an A row value given by hand that this node cannot read is refused, not taken as both
	/// ways unsaid (review of 2026-11-14); the cluster's encoding and none are taken
	#[tokio::test]
	async fn an_a_row_value_it_cannot_read_is_refused() {
		let (db, path) = db().await;
		let app = db.connect().unwrap();
		app.execute("CREATE TABLE t (id int PRIMARY KEY)", ()).await.unwrap();
		let s = Turso::open(&db, "appdb").await.unwrap();
		let e = s.sync("t", "not base64 at all!").await.unwrap_err();
		assert!(e.starts_with("YT-E085"), "{e}");
		assert_eq!(q(&app, "SELECT count(*) FROM yt_config").await, vec![vec![Value::Integer(0)]], "nothing written");
		s.sync("t", "").await.unwrap();
		assert_eq!(q(&app, "SELECT count(*) FROM yt_config WHERE module = 'A'").await, vec![vec![Value::Integer(1)]]);
		drop(app);
		let _ = std::fs::remove_file(&path);
	}

	/// [migrate] a directory of the cluster's schemas deployed here (mapped for SQLite) and its
	/// tables synced: a row written here goes up with the cluster's types (from `yt_columns`), a
	/// second run changes nothing, a versioned script changed after it ran is refused
	#[tokio::test]
	async fn a_migration_deploys_the_clusters_schema_and_syncs_its_tables() {
		let (db, path) = db().await;
		let dir = path.with_extension("dir");
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("S1__accounts.yaml"), "database:
  - schemaName: s_app
    tables:
      - table:
          tableName: accounts
          columns:
            - column:
                name: id
                type: bigserial
                constraint:
                  primaryKey: true
            - column:
                name: name
                type: varchar(80)
            - column:
                name: at
                type: timestamptz
                defaultValue: now()
      - table:
          tableName: notes
          columns:
            - column:
                name: line
                type: text
").unwrap();
		std::fs::write(dir.join("V1_1__seed.sql"), "INSERT INTO \"s_app.accounts\" (id, name, at) VALUES (1, 'seed', '2026-11-19 10:00:00+00');").unwrap();
		let s = Turso::open(&db, "appdb").await.unwrap();
		let r = s.migrate_dir(&s, &dir).await.unwrap();
		assert_eq!(r.ran.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["S1__accounts.yaml", "V1_1__seed.sql"]);
		assert_eq!(r.synced, ["s_app.accounts"]);
		assert!(r.notes.iter().any(|n| n.contains("s_app.notes: no primary key")), "{:?}", r.notes);
		// the seed ran before the triggers were made: not captured; a row written now is
		let app = db.connect().unwrap();
		app.execute("INSERT INTO \"s_app.accounts\" (id, name, at) VALUES (2, 'b', '2026-11-19 11:00:00+00')", ()).await.unwrap();
		let (items, _, _) = s.collect(None, 10).await.unwrap();
		assert_eq!(items.len(), 1, "{:?}", items);
		let row = &items[0].data[0];
		let t = row.table.as_ref().unwrap();
		assert_eq!((t.schema.as_str(), t.table.as_str()), ("s_app", "accounts"));
		let ty = |n: &str| t.columns.iter().find(|c| c.name == n).unwrap().atttypid;
		assert_eq!((ty("id"), ty("name"), ty("at")), (20, 1043, 1184), "the cluster's types, not SQLite's");
		// the second run: nothing
		let again = s.migrate_dir(&s, &dir).await.unwrap();
		assert!(again.ran.is_empty() && again.synced.is_empty(), "{:?}", again);
		std::fs::write(dir.join("V1_1__seed.sql"), "SELECT 1;").unwrap();
		assert!(s.migrate_dir(&s, &dir).await.unwrap_err().contains("changed after it ran"));
		drop(app);
		let _ = std::fs::remove_dir_all(&dir);
		let _ = std::fs::remove_file(&path);
	}

	#[test]
	fn a_files_md5_is_sges() {
		// RFC 1321's own examples
		let hex = |b: &[u8]| { use base64::Engine; base64::engine::general_purpose::STANDARD.decode(crate::migrate::checksum(b)).unwrap()
			.iter().map(|x| format!("{:02x}", x)).collect::<String>() };
		assert_eq!(hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
		assert_eq!(hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
		assert_eq!(hex(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"), "57edf4a22be3c955ac49da2e2107b67a");
	}
}
