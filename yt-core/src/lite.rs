// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! A light node's store in a database of the SQLite dialect, the application's own: Turso's
//! (crate::turso, feature `turso`) or SQLite's (the yt-sqlite loadable extension). One store,
//! [`Lite`], over a connection of either: [`Conn`] runs its statements.
//!
//! - **Up.** `setup(table)` puts three triggers on a table, after an insert, an update and a
//!   delete, that write the row - every column named, as JSON - to `yt_outbox`. The outbox
//!   row commits or rolls back with the row itself. What goes up is read on a connection of
//!   this node's own, so only what is committed: an outbox row is one position.
//! - **The values,** as the PostgreSQL extension sends them, by each column's declared type:
//!   an integer type as `Int`, a float as `Flt`, a boolean as `Bool`, anything else as text
//!   (`Universal`) - and the column's PostgreSQL type id beside it, so the cluster above
//!   binds the value as it binds one of its own. Declare a table's columns with the types
//!   of its PostgreSQL table (`int`, `bigint`, `text`, `timestamptz`...): SQLite keeps the
//!   declared type as written.
//! - **Names.** A table of schema `s` in the cluster is the table named `s.t` here (a quoted
//!   name with a dot: SQLite has no schemas); `public`'s are named `t`. Its columns are
//!   matched by name, and it needs a primary key.
//! - **Down.** A batch is applied in one transaction with its cursor. The outbox's highest
//!   id is noted first, and the outbox rows above it - the triggers' copies of the rows
//!   just applied - are deleted before the commit: nothing that came down goes back up.
//!   The transaction takes the write lock at its start (BEGIN IMMEDIATE), so no other
//!   writer's rows are in that range.
//! - **What it holds.** A row of a table not here, or of another database of the cluster,
//!   is not this node's: skipped and counted, never an error. A batch that cannot be
//!   applied is refused whole, and tried again.
//! - **Which tables sync** (crate::sync), as on a cluster node: those with an A row in this
//!   database's `yt_config` (`schema.table`, as the cluster names it), each the ways its
//!   value says. `setup` writes one with the table's triggers; `sync` writes one alone - a
//!   table whose rows only come down; `unsync` takes both away. A table with none: what
//!   comes down for it is not applied, its captured rows are not sent; both counted.
//! - **Not followed:** a schema change (run `setup` again after one); `INSERT OR REPLACE`
//!   of a row that is there, which SQLite's triggers see as an insert only - write an
//!   upsert (`ON CONFLICT DO UPDATE`) instead.
//! - **Its connection** is the node's own, beside the application's: each engine opens it
//!   (crate::turso `Turso::open`, yt-sqlite on the extension's database file).
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::Mutex;
use yt_core_proto::base::{var_type::Value as V, ColumnRef, TableRef, VarType};
use yt_core_proto::db::{DbAction, DbInputEntity};
use yt_core_proto::link::{RelayBatch, RelayItem};
use yt_core_proto::MAX_SUBTERM;

use crate::core::pos::{to_pos, Pos};

/// The words of a database with no file, for yt-sqlite: it cannot link.
pub fn no_file(db: &str) -> String {
	t!("light-sqlite-no-file", db = db)
}

/// The words of a link with no cluster above set, for yt-sqlite.
pub fn no_upper() -> String {
	t!("light-sqlite-no-upper")
}

/// The words of `yt_link` called inside a transaction, for yt-sqlite.
pub fn in_transaction() -> String {
	t!("light-sqlite-in-tx")
}

/// The words of an `upper_root` that is no CA fingerprint, for yt-sqlite.
pub fn bad_root(value: &str) -> String {
	t!("light-sqlite-bad-root", value = value)
}

/// A value of a row, as both engines hold one.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
	Null,
	Integer(i64),
	Real(f64),
	Text(String),
	Blob(Vec<u8>),
}

/// A connection of the SQLite dialect the store runs its statements on: a statement with
/// its parameters (how many rows it changed), several with none, a query's rows.
#[tonic::async_trait]
pub trait Conn: Send + Sync + 'static {
	async fn exec(&self, sql: &str, params: Vec<Val>) -> Result<u64, String>;
	async fn batch(&self, sql: &str) -> Result<(), String>;
	async fn rows(&self, sql: &str, params: Vec<Val>) -> Result<Vec<Vec<Val>>, String>;
}

/// `yt_position`'s row of how far this node applied what came down
const CURSOR: &str = "cursor";

/// A column of a local table.
#[derive(Clone, Debug, PartialEq)]
pub struct Col {
	/// its number: the column's place, from 1
	pub attnum: u32,
	pub name: String,
	/// its type as declared, lower case
	pub decl: String,
	/// part of the primary key
	pub pk: bool,
}

/// What kind of value a declared type holds, and the PostgreSQL type id that goes with
/// it - as the extension sends a value of that type (ytpgc yt_cluster.c, per type).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
	Int,
	Flt,
	Bool,
	Text,
	/// bytea: a BLOB here, `\x<hex>` text on the wire, as PostgreSQL writes it
	Bytes,
}

/// A declared type: its kind and its PostgreSQL type id (0: none known - the cluster above
/// takes the value as text).
pub fn kind(decl: &str) -> (Kind, u32) {
	let d = decl.trim().to_lowercase();
	let base = d.split('(').next().unwrap_or("").trim();
	match base {
		"smallint" | "int2" => (Kind::Int, 21),
		"int" | "integer" | "int4" | "serial" => (Kind::Int, 23),
		"bigint" | "int8" | "bigserial" => (Kind::Int, 20),
		"real" | "float4" => (Kind::Flt, 700),
		"double precision" | "double" | "float8" | "float" => (Kind::Flt, 701),
		"bool" | "boolean" => (Kind::Bool, 16),
		"text" | "" => (Kind::Text, 25),
		"varchar" | "character varying" => (Kind::Text, 1043),
		"char" | "character" | "bpchar" => (Kind::Text, 1042),
		"timestamptz" | "timestamp with time zone" => (Kind::Text, 1184),
		"timestamp" | "timestamp without time zone" => (Kind::Text, 1114),
		"date" => (Kind::Text, 1082),
		"time" => (Kind::Text, 1083),
		"numeric" | "decimal" => (Kind::Text, 1700),
		"json" => (Kind::Text, 114),
		"jsonb" => (Kind::Text, 3802),
		"uuid" => (Kind::Text, 2950),
		"bytea" | "blob" => (Kind::Bytes, 17),
		_ => (Kind::Text, 0),
	}
}

/// The name of a cluster table here: `schema.table`, `public`'s by its name alone.
pub fn local_name(schema: &str, table: &str) -> String {
	if schema == "public" { table.to_string() } else { format!("{}.{}", schema, table) }
}

/// A local table's name as the cluster has it: (schema, table).
pub fn cluster_name(local: &str) -> (String, String) {
	match local.split_once('.') {
		Some((s, t)) => (s.to_string(), t.to_string()),
		None => ("public".to_string(), local.to_string()),
	}
}

/// A name for SQL, quoted.
pub(crate) fn ident(s: &str) -> String {
	format!("\"{}\"", s.replace('"', "\"\""))
}

/// A text for SQL, quoted.
pub(crate) fn lit(s: &str) -> String {
	format!("'{}'", s.replace('\'', "''"))
}

/// The position of outbox row `id` (from 1): a term holds at most MAX_SUBTERM.
pub fn pos_of(id: u64) -> Pos {
	if id == 0 { return (0, 0) }
	let m = MAX_SUBTERM as u64;
	(1 + (id - 1) / m, ((id - 1) % m + 1) as u32)
}

/// The outbox row at position `p`.
pub fn id_of(p: Pos) -> u64 {
	if p.0 == 0 { return 0 }
	(p.0 - 1) * MAX_SUBTERM as u64 + p.1 as u64
}

pub(crate) fn e<E: std::fmt::Display>(what: &'static str) -> impl Fn(E) -> String {
	move |x| t!("light-db", what = what, error = x)
}

/// The store on a connection `C` of the SQLite dialect: Turso's (crate::turso), SQLite's
/// (the yt-sqlite extension).
pub struct Lite<C: Conn> {
	/// this node's connection: the outbox read, the applies - on its own, so it reads
	/// only what the application committed
	conn: Mutex<C>,
	/// the name the cluster above knows this database by
	pub cluster_db: String,
	/// the local tables' columns, by local name; None for a table not here (asked again)
	tables: Mutex<HashMap<String, Vec<Col>>>,
	/// rows from above that matched no row here (an update or delete)
	pub missed: AtomicU64,
	/// rows from above of a table not here, or of another database: skipped
	pub not_held: AtomicU64,
	/// captured rows of a table that is gone since (dropped, renamed): passed over, not sent
	pub not_sent: AtomicU64,
	/// rows of a table that does not sync that way - no A row, or no_up / no_down in it
	pub not_synced: AtomicU64,
	/// the last outbox row counted in not_sent or not_synced: a range collected again after
	/// a failed send counts nothing twice (review of 2026-11-13)
	counted: AtomicU64,
	/// the last batch from above that was not applied, and why
	pub apply_error: std::sync::Mutex<String>,
}

pub(crate) fn int(v: &Val) -> i64 {
	match v {
		Val::Integer(i) => *i,
		Val::Real(f) => *f as i64,
		Val::Text(s) => s.parse().unwrap_or(0),
		_ => 0,
	}
}

pub(crate) fn text(v: &Val) -> Option<String> {
	match v {
		Val::Text(s) => Some(s.clone()),
		Val::Integer(i) => Some(i.to_string()),
		Val::Real(f) => Some(f.to_string()),
		_ => None,
	}
}

impl<C: Conn> Lite<C> {
	/// The store on `conn`, a connection of this node's own to the database, and its three
	/// tables. `cluster_db`: this database's name in the cluster above.
	pub async fn new(conn: C, cluster_db: &str) -> Result<Lite<C>, String> {
		conn.batch("CREATE TABLE IF NOT EXISTS yt_outbox (
				id INTEGER PRIMARY KEY AUTOINCREMENT,
				tbl TEXT NOT NULL,
				action TEXT NOT NULL,
				old TEXT,
				new TEXT,
				passed_at TEXT);
			CREATE TABLE IF NOT EXISTS yt_position (
				name TEXT PRIMARY KEY,
				term INTEGER NOT NULL,
				subterm INTEGER NOT NULL);
			CREATE TABLE IF NOT EXISTS yt_config (
				name TEXT NOT NULL,
				module TEXT NOT NULL,
				value TEXT NOT NULL DEFAULT '',
				PRIMARY KEY (module, name));").await.map_err(e("yt_outbox, yt_position, yt_config"))?;
		Ok(Lite { conn: Mutex::new(conn), cluster_db: cluster_db.to_string(), tables: Mutex::new(HashMap::new()),
			missed: AtomicU64::new(0), not_held: AtomicU64::new(0), not_sent: AtomicU64::new(0), not_synced: AtomicU64::new(0), counted: AtomicU64::new(0), apply_error: std::sync::Mutex::new(String::new()) })
	}

	/// A local table's columns; None for a table not here.
	async fn columns(&self, c: &C, local: &str) -> Result<Option<Vec<Col>>, String> {
		if let Some(t) = self.tables.lock().await.get(local) {
			return Ok(Some(t.clone()));
		}
		let rows = c.rows(&format!("SELECT cid, name, type, pk FROM pragma_table_info({})", lit(local)), vec![])
			.await.map_err(e("pragma_table_info"))?;
		if rows.is_empty() {
			return Ok(None);
		}
		let cols: Vec<Col> = rows.iter().map(|r| Col { attnum: int(&r[0]) as u32 + 1, name: text(&r[1]).unwrap_or_default(),
			decl: text(&r[2]).unwrap_or_default().to_lowercase(), pk: int(&r[3]) > 0 }).collect();
		self.tables.lock().await.insert(local.to_string(), cols.clone());
		Ok(Some(cols))
	}

	/// The tables that sync and which way: this database's A rows, by `schema.table`.
	async fn syncs(&self, c: &C) -> Result<HashMap<String, crate::sync::Sync>, String> {
		let rows = c.rows("SELECT name, value FROM yt_config WHERE module = 'A'", vec![]).await.map_err(e("yt_config"))?;
		let rows: Vec<(String, String)> = rows.iter().map(|r| (text(&r[0]).unwrap_or_default(), text(&r[1]).unwrap_or_default())).collect();
		Ok(crate::sync::tables(rows.iter().map(|(n, v)| (n.as_str(), v.as_str()))))
	}

	/// The A row of local table `table`: its name as the cluster has it.
	fn a_name(table: &str) -> String {
		let (s, t) = cluster_name(table);
		format!("{}.{}", s, t)
	}

	/// A table named either way - its local name, or the cluster's (`public.t` for `t`) - by
	/// its local name (review of 2026-11-13: unsync("public.t") left t's triggers)
	fn local(table: &str) -> String {
		let (s, t) = cluster_name(table);
		local_name(&s, &t)
	}

	/// Sync a table, with the A row's `value` (the cluster's encoding, `ytctl
	/// --encode-table-cfg`; empty: both ways) and no capture: what comes down for it is
	/// applied here; its writes stay unless `setup` captures them too.
	pub async fn sync(&self, table: &str, value: &str) -> Result<(), String> {
		// a value this node cannot read would sync both ways, unsaid (review of 2026-11-14)
		if !crate::sync::readable(value) {
			return Err(t!("light-sync-value", value = value));
		}
		let table = &Self::local(table);
		let c = self.conn.lock().await;
		c.exec("INSERT INTO yt_config (name, module, value) VALUES (?, 'A', ?) \
			ON CONFLICT (module, name) DO UPDATE SET value = excluded.value",
			vec![Val::Text(Self::a_name(table)), Val::Text(value.to_string())]).await.map_err(e("yt_config"))?;
		Ok(())
	}

	/// Stop syncing a table: its A row and its triggers go.
	pub async fn unsync(&self, table: &str) -> Result<(), String> {
		let table = &Self::local(table);
		let c = self.conn.lock().await;
		c.exec("DELETE FROM yt_config WHERE module = 'A' AND name = ?", vec![Val::Text(Self::a_name(table))])
			.await.map_err(e("yt_config"))?;
		let mut sql = String::new();
		for a in ["i", "u", "d"] {
			sql.push_str(&format!("DROP TRIGGER IF EXISTS {};\n", ident(&format!("yt_{}_{}", a, table))));
		}
		c.batch(&sql).await.map_err(e("the capture triggers"))?;
		Ok(())
	}

	/// Capture a table's writes and sync it: its three triggers, made again - after a schema
	/// change too - and its A row, as `yt_setup` writes one (both ways), when it has none.
	/// `table` is its local name (`schema.table`, or the name alone for `public`).
	pub async fn setup(&self, table: &str) -> Result<(), String> {
		let table = &Self::local(table);
		let c = self.conn.lock().await;
		self.tables.lock().await.remove(table);
		let cols = self.columns(&c, table).await?.ok_or_else(|| t!("light-turso-no-table", table = table))?;
		if !cols.iter().any(|c| c.pk) {
			return Err(t!("light-turso-no-key", table = table));
		}
		// a BLOB as PostgreSQL writes a bytea, an infinite REAL as its word: json_object refuses
		// a BLOB and writes infinity as 9e999, which no JSON reader takes; any other REAL in 17
		// digits, as text - json_object writes 15, and 0.1 + 0.2 would go up as 0.3; Turso's
		// printf keeps them with the `!` flag only (review of 2026-11-13)
		let image = |of: &str| format!("json_object({})", cols.iter().map(|c| {
			let v = format!("{}.{}", of, ident(&c.name));
			format!("{}, CASE WHEN typeof({v}) = 'blob' THEN '\\x' || lower(hex({v})) \
				WHEN typeof({v}) = 'real' AND {v} > 1.7976931348623157e308 THEN 'Infinity' \
				WHEN typeof({v}) = 'real' AND {v} < -1.7976931348623157e308 THEN '-Infinity' \
				WHEN typeof({v}) = 'real' THEN printf('%!.17g', {v}) ELSE {v} END", lit(&c.name))
		}).collect::<Vec<_>>().join(", "));
		let (t, q) = (ident(table), lit(table));
		let mut sql = String::new();
		for (a, when, old, new) in [("i", "INSERT", "NULL".to_string(), image("NEW")), ("u", "UPDATE", image("OLD"), image("NEW")),
			("d", "DELETE", image("OLD"), "NULL".to_string())] {
			let name = ident(&format!("yt_{}_{}", a, table));
			sql.push_str(&format!("DROP TRIGGER IF EXISTS {name};
				CREATE TRIGGER {name} AFTER {when} ON {t} BEGIN
				INSERT INTO yt_outbox (tbl, action, old, new) VALUES ({q}, '{}', {old}, {new}); END;\n", a.to_uppercase()));
		}
		c.batch(&sql).await.map_err(e("the capture triggers"))?;
		c.exec("INSERT INTO yt_config (name, module, value) VALUES (?, 'A', '') ON CONFLICT (module, name) DO NOTHING",
			vec![Val::Text(Self::a_name(table))]).await.map_err(e("yt_config"))?;
		Ok(())
	}

	/// A captured row, as the extension sends one.
	fn entity(&self, id: u64, local: &str, action: &str, old: Option<String>, new: Option<String>, cols: &[Col]) -> Result<DbInputEntity, String> {
		let (schema, table) = cluster_name(local);
		let columns = cols.iter().map(|c| ColumnRef { attnum: c.attnum, name: c.name.clone(), pk: c.pk,
			atttypid: kind(&c.decl).1, ..Default::default() }).collect();
		let image = |j: Option<String>| -> Result<HashMap<u32, VarType>, String> {
			let Some(j) = j else { return Ok(HashMap::new()) };
			let m: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&j).map_err(e("yt_outbox, a row's JSON"))?;
			Ok(cols.iter().filter_map(|c| m.get(&c.name).map(|v| (c.attnum, var(v, kind(&c.decl).0)))).collect())
		};
		let action = match action { "I" => DbAction::Insert, "U" => DbAction::Update, _ => DbAction::Delete };
		Ok(DbInputEntity { tx: id, table: Some(TableRef { db: self.cluster_db.clone(), schema, table, columns, ..Default::default() }),
			old_data: image(old)?, new_data: image(new)?, action: action as i32, ..Default::default() })
	}

	async fn position(&self, c: &C, name: &str) -> Result<Option<Pos>, String> {
		let r = c.rows("SELECT term, subterm FROM yt_position WHERE name = ?", vec![Val::Text(name.to_string())])
			.await.map_err(e("yt_position"))?;
		Ok(r.first().map(|r| (int(&r[0]) as u64, int(&r[1]) as u32)))
	}

	async fn set_position(c: &C, name: &str, p: Pos) -> Result<(), String> {
		c.exec("INSERT INTO yt_position (name, term, subterm) VALUES (?, ?, ?) \
			ON CONFLICT (name) DO UPDATE SET term = excluded.term, subterm = excluded.subterm",
			vec![Val::Text(name.to_string()), Val::Integer(p.0 as i64), Val::Integer(p.1 as i64)])
			.await.map_err(e("yt_position"))?;
		Ok(())
	}

	async fn apply_batch(&self, batch: &RelayBatch) -> Result<Option<Pos>, String> {
		let c = self.conn.lock().await;
		let at = self.position(&c, CURSOR).await?;
		let Some(upto) = to_pos(&batch.upto) else { return Ok(at) };
		if Some(upto) <= at {
			return Ok(at);
		}
		let mut plans = Vec::new();
		let (mut not_held, mut not_synced) = (0, 0);
		let syncs = self.syncs(&c).await?;
		for item in batch.items.iter().filter(|i| Some((i.term, i.subterm)) > at && (i.term, i.subterm) <= upto) {
			for row in &item.data {
				let t = row.table.as_ref().ok_or_else(|| t!("light-row-no-table"))?;
				if t.db != self.cluster_db {
					not_held += 1;
					continue;
				}
				// a table that syncs down here: an A row, no no_down in it
				if !syncs.get(&format!("{}.{}", t.schema, t.table)).is_some_and(|s| s.down) {
					not_synced += 1;
					continue;
				}
				let local = local_name(&t.schema, &t.table);
				match self.columns(&c, &local).await? {
					Some(cols) => plans.push(statement(row, &local, &cols)?),
					None => not_held += 1,
				}
			}
		}
		c.exec("BEGIN IMMEDIATE", vec![]).await.map_err(e("the apply's transaction"))?;
		let r: Result<u64, String> = async {
			let top = c.rows("SELECT coalesce(max(id), 0) FROM yt_outbox", vec![]).await.map_err(e("yt_outbox"))?;
			let top = top.first().map(|r| int(&r[0])).unwrap_or(0);
			let mut missed = 0;
			for (sql, params, must_match) in plans {
				let n = c.exec(&sql, params).await.map_err(|x| t!("light-statement", sql = sql, error = x))?;
				if must_match && n == 0 {
					missed += 1;
				}
			}
			// the triggers' copies of what was just applied: it does not go back up
			c.exec("DELETE FROM yt_outbox WHERE id > ?", vec![Val::Integer(top)]).await.map_err(e("yt_outbox"))?;
			Self::set_position(&*c, CURSOR, upto).await?;
			Ok(missed)
		}.await;
		match r {
			Ok(missed) => {
				// a COMMIT that failed leaves the transaction open: rolled back, or every next
				// apply would fail at its BEGIN (review of 2026-11-13)
				if let Err(x) = c.exec("COMMIT", vec![]).await {
					let _ = c.exec("ROLLBACK", vec![]).await;
					return Err(e("the apply's transaction")(x));
				}
				self.missed.fetch_add(missed, Ordering::Relaxed);
				self.not_held.fetch_add(not_held, Ordering::Relaxed);
				self.not_synced.fetch_add(not_synced, Ordering::Relaxed);
				Ok(Some(upto))
			}
			Err(x) => {
				let _ = c.exec("ROLLBACK", vec![]).await;
				Err(x)
			}
		}
	}
}

/// A captured JSON value as the extension would send it for a column of `kind`.
pub(crate) fn var(v: &serde_json::Value, kind: Kind) -> VarType {
	use serde_json::Value as J;
	let value = match (v, kind) {
		(J::Null, _) => None,
		(J::Number(n), Kind::Int) => n.as_i64().map(V::Int).or_else(|| n.as_f64().map(|f| V::Int(f as i64))),
		(J::String(s), Kind::Int) => Some(s.trim().parse().map(V::Int).unwrap_or_else(|_| V::Universal(s.clone()))),
		(J::Number(n), Kind::Flt) => n.as_f64().map(V::Flt),
		// a REAL as the capture writes it, in 17 digits
		(J::String(s), Kind::Flt) if s.trim().parse::<f64>().is_ok_and(|f| f.is_finite()) => s.trim().parse().ok().map(V::Flt),
		(J::Number(n), Kind::Bool) => Some(V::Bool(n.as_f64() != Some(0.0))),
		(J::Bool(b), _) => Some(if kind == Kind::Text { V::Universal(b.to_string()) } else { V::Bool(*b) }),
		(J::String(s), _) => Some(V::Universal(s.clone())),
		(J::Number(n), _) => Some(V::Universal(n.to_string())),
		(other, _) => Some(V::Universal(other.to_string())),
	};
	VarType { value }
}

/// A value from above, for a column declared `decl` here.
pub(crate) fn value(v: &VarType, decl: &str) -> Result<Val, String> {
	Ok(match &v.value {
		None => Val::Null,
		Some(V::Int(i)) => Val::Integer(*i),
		Some(V::UInt(u)) => Val::Integer(*u as i64),
		// SQLite keeps a NaN as NULL: a value that is not finite is kept as PostgreSQL's word for
		// it, which goes back up as that (review of 2026-11-13)
		Some(V::Flt(f)) if f.is_nan() => Val::Text("NaN".to_string()),
		Some(V::Flt(f)) if f.is_infinite() => Val::Text(if *f > 0.0 { "Infinity" } else { "-Infinity" }.to_string()),
		Some(V::Flt(f)) => Val::Real(*f),
		Some(V::Bool(b)) => Val::Integer(*b as i64),
		// a number that does not read as one is refused, as PostgreSQL refuses it (review of
		// 2026-11-13): SQLite would keep the text in a numeric column
		Some(V::Universal(s)) => match kind(decl).0 {
			Kind::Int => Val::Integer(s.trim().parse().map_err(|_| t!("light-turso-value", value = s, decl = decl))?),
			Kind::Flt => match s.trim() {
				"NaN" | "Infinity" | "-Infinity" => Val::Text(s.trim().to_string()),
				x => Val::Real(x.parse().map_err(|_| t!("light-turso-value", value = s, decl = decl))?),
			},
			Kind::Bool => Val::Integer(matches!(s.trim(), "t" | "true" | "1" | "yes" | "on") as i64),
			Kind::Bytes => match s.strip_prefix("\\x") {
				Some(h) if h.len() % 2 == 0 => Val::Blob((0..h.len()).step_by(2)
					.map(|i| u8::from_str_radix(&h[i..i + 2], 16)).collect::<Result<Vec<u8>, _>>()
					.map_err(|_| t!("light-turso-value", value = s, decl = decl))?),
				_ => return Err(t!("light-turso-value", value = s, decl = decl)),
			},
			Kind::Text => Val::Text(s.clone()),
		},
		Some(V::Diff(_)) => return Err(t!("light-full-version", what = "a document merged path by path")),
	})
}

/// The statement that applies one row to the local table `local`, its parameters, and
/// whether it must match a row (an update or a delete). A row is found by its key; an
/// insert of a key already here is a batch sent again: nothing.
pub fn statement(row: &DbInputEntity, local: &str, cols: &[Col]) -> Result<(String, Vec<Val>, bool), String> {
	let t = row.table.as_ref().ok_or_else(|| t!("light-row-no-table"))?;
	let name = |attnum: &u32| t.columns.iter().find(|c| c.attnum == *attnum).map(|c| c.name.clone())
		.ok_or_else(|| t!("light-no-attnum", table = local, attnum = attnum));
	let decl = |n: &str| cols.iter().find(|c| c.name == n).map(|c| c.decl.clone())
		.ok_or_else(|| t!("light-no-column", table = local, column = n));
	let sorted = |m: &HashMap<u32, VarType>| { let mut k: Vec<u32> = m.keys().copied().collect(); k.sort(); k };
	let pk: Vec<&Col> = cols.iter().filter(|c| c.pk).collect();
	let tq = ident(local);
	let mut params = Vec::new();
	let action = DbAction::try_from(row.action).map_err(|_| DbAction::to_str(row.action))?;
	let key = |params: &mut Vec<Val>| -> Result<String, String> {
		let mut wh = Vec::new();
		// a table with no key here: the row is found by every column of its old image, one of
		// the equal rows by its rowid - as PostgreSQL's side does by ctid (review of 2026-11-13)
		if pk.is_empty() {
			if row.old_data.is_empty() {
				return Err(t!("light-keyless-no-old", table = local));
			}
			for k in sorted(&row.old_data) {
				let n = name(&k)?;
				params.push(value(&row.old_data[&k], &decl(&n)?)?);
				wh.push(format!("{} IS ?", ident(&n)));
			}
			return Ok(format!("rowid = (SELECT rowid FROM {} WHERE {} LIMIT 1)", tq, wh.join(" AND ")));
		}
		for c in &pk {
			let v = t.columns.iter().find(|r| r.name == c.name)
				.and_then(|r| row.old_data.get(&r.attnum).or_else(|| row.new_data.get(&r.attnum)))
				.ok_or_else(|| t!("light-no-key-value", table = local, column = c.name))?;
			params.push(value(v, &c.decl)?);
			wh.push(format!("{} = ?", ident(&c.name)));
		}
		Ok(wh.join(" AND "))
	};
	Ok(match action {
		DbAction::Insert => {
			let (mut names, mut marks) = (Vec::new(), Vec::new());
			for k in sorted(&row.new_data) {
				let n = name(&k)?;
				params.push(value(&row.new_data[&k], &decl(&n)?)?);
				names.push(ident(&n));
				marks.push("?");
			}
			// a conflict on the key only: a batch sent again inserts nothing
			let conflict = if pk.is_empty() { String::new() } else {
				format!(" ON CONFLICT ({}) DO NOTHING", pk.iter().map(|c| ident(&c.name)).collect::<Vec<_>>().join(", "))
			};
			(format!("INSERT INTO {} ({}) VALUES ({}){}", tq, names.join(", "), marks.join(", "), conflict), params, false)
		}
		DbAction::Update => {
			let mut set = Vec::new();
			for k in sorted(&row.new_data) {
				let n = name(&k)?;
				// a key column is set only when the update changed it
				if pk.iter().any(|c| c.name == n) && row.old_data.get(&k).is_none_or(|o| *o == row.new_data[&k]) {
					continue;
				}
				params.push(value(&row.new_data[&k], &decl(&n)?)?);
				set.push(format!("{} = ?", ident(&n)));
			}
			if set.is_empty() {
				return Ok(("SELECT 1".to_string(), Vec::new(), false));
			}
			let wh = key(&mut params)?;
			(format!("UPDATE {} SET {} WHERE {}", tq, set.join(", "), wh), params, true)
		}
		DbAction::Delete => {
			let wh = key(&mut params)?;
			(format!("DELETE FROM {} WHERE {}", tq, wh), params, true)
		}
		DbAction::Truncate => (format!("DELETE FROM {}", tq), params, false),
	})
}

#[tonic::async_trait]
impl<C: Conn> crate::db::Db for Lite<C> {
	async fn waiting(&self) -> Result<i64, String> {
		let c = self.conn.lock().await;
		let r = c.rows("SELECT count(*) FROM yt_outbox WHERE passed_at IS NULL", vec![]).await.map_err(e("yt_outbox"))?;
		Ok(r.first().map(|r| int(&r[0])).unwrap_or(0))
	}

	async fn cursor(&self) -> Result<Option<Pos>, String> {
		let c = self.conn.lock().await;
		self.position(&c, CURSOR).await
	}

	async fn collect(&self, from: Option<Pos>, max_rows: usize) -> Result<(Vec<RelayItem>, Pos, bool), String> {
		let f = from.unwrap_or((0, 0));
		let c = self.conn.lock().await;
		let rows = c.rows("SELECT id, tbl, action, old, new FROM yt_outbox WHERE id > ? ORDER BY id LIMIT ?",
			vec![Val::Integer(id_of(f) as i64), Val::Integer(max_rows.max(1) as i64 + 1)]).await.map_err(e("yt_outbox"))?;
		let more = rows.len() > max_rows.max(1);
		let (mut items, mut upto) = (Vec::new(), f);
		let syncs = self.syncs(&c).await?;
		for r in rows.iter().take(max_rows.max(1)) {
			let id = int(&r[0]) as u64;
			let local = text(&r[1]).unwrap_or_default();
			let p = pos_of(id);
			// counted once, whatever is collected again after a failed send
			let first = id > self.counted.fetch_max(id, Ordering::Relaxed);
			// a table that does not sync up (any more): its row stays here, its position passed over
			if !syncs.get(&Self::a_name(&local)).is_some_and(|s| s.up) {
				if first {
					self.not_synced.fetch_add(1, Ordering::Relaxed);
				}
				items.push(RelayItem { term: p.0, subterm: p.1, data: vec![] });
				upto = p;
				continue;
			}
			// a table gone since the row was captured (dropped, renamed): nothing to send it
			// as - its position passed over, counted, so the rows after it still go
			// (review of 2026-11-13)
			let Some(cols) = self.columns(&c, &local).await? else {
				if first {
					self.not_sent.fetch_add(1, Ordering::Relaxed);
				}
				items.push(RelayItem { term: p.0, subterm: p.1, data: vec![] });
				upto = p;
				continue;
			};
			let ent = self.entity(id, &local, &text(&r[2]).unwrap_or_default(), text(&r[3]), text(&r[4]), &cols)?;
			items.push(RelayItem { term: p.0, subterm: p.1, data: vec![ent] });
			upto = p;
		}
		Ok((items, upto, more))
	}

	async fn apply(&self, batch: &RelayBatch) -> Result<Option<Pos>, String> {
		let r = self.apply_batch(batch).await;
		*self.apply_error.lock().unwrap_or_else(|e| e.into_inner()) = r.as_ref().err().cloned().unwrap_or_default();
		if r.is_err() {
			self.tables.lock().await.clear();
		}
		r
	}

	async fn passed(&self, upto: Pos) -> Result<(), String> {
		let c = self.conn.lock().await;
		c.exec("UPDATE yt_outbox SET passed_at = datetime('now') WHERE passed_at IS NULL AND id <= ?",
			vec![Val::Integer(id_of(upto) as i64)]).await.map_err(e("yt_outbox"))?;
		// kept a day: a cluster above restored from a backup asks for them again
		c.exec("DELETE FROM yt_outbox WHERE passed_at < datetime('now', '-1 day')", vec![]).await.map_err(e("yt_outbox"))?;
		Self::set_position(&*c, "passed", upto).await
	}
}

