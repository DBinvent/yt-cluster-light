// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! yt-lite's store: PostgreSQL, the one database it serves (yt-lite.md 16).
//!
//! - **Up.** A transaction's rows, as the extension sent them, wait in memory until its
//!   prepare (the extension's PRE_COMMIT); then they are in `yt_outbox` under the
//!   transaction's id, at this node's next position - taken at the prepare, so two
//!   transactions on one row are in the order the database committed them: the second
//!   cannot prepare before the first committed. A row goes up only once the database
//!   says its transaction committed (`pg_xact_status`), and up to the first one still
//!   undecided; an aborted one's are dropped. The cluster above passing a position marks
//!   what is at or below it, kept a day for a cluster that comes back from a backup.
//! - **Down.** A batch is applied in one transaction with its cursor, under
//!   `yt.node = 'local'`, so the extension's triggers send nothing of it back up. A
//!   value is cast to the local column's type, read from this database's catalog.
//! - **What it holds.** A lite node has no az: the node above sends it what its own az
//!   holds, whatever database and table. This node takes the rows of the tables its one
//!   database has; a row of another database, or of a table that is not here, is not
//!   this node's - skipped and counted (yt_info `rows_not_held_here`), never an error. A
//!   table made here later starts from then: copy its rows first.
//! - **One database.** A write from any other database of this instance is refused,
//!   never skipped, with the sentence that names the full version.
//! - **Which tables sync** (crate::sync): those with an A row in the config table, as on a
//!   cluster node - `yt_setup` writes it with the trigger - each the ways its value says. A
//!   table with none: what comes down for it is not applied, its writes are not sent; both
//!   counted (yt_info `rows_not_synced_here`).
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use prost::Message;
use tokio::sync::{Mutex, MutexGuard};
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, NoTls};
use yt_core_proto::db::{DbAction, DbInputEntity};
use yt_core_proto::link::{RelayBatch, RelayItem};
use yt_core_proto::{base::var_type::Value, MAX_SUBTERM};

use crate::core::pos::{to_pos, Pos};

/// What a write outside the one database is answered with (SQLSTATE 0A000, as the
/// extension maps a refusal's leading code).
pub fn one_database(db: &str) -> String {
	t!("lite-one-database", db = db)
}

/// The same for a feature of the full version.
pub fn full_version(what: &str) -> String {
	t!("lite-full-version", what = what)
}

pub struct Pg {
	/// where the database is, for a session lost (PostgreSQL restarted)
	url: String,
	/// the outbox and the positions
	capture: Mutex<Client>,
	/// the applies of what comes down
	apply: Mutex<Client>,
	/// the schema of the config table, where the lite tables are kept
	pub schema: String,
	/// the config table's name in it
	pub config_table: String,
	/// this database's own name
	pub db: String,
	/// the name the cluster above knows this database by
	pub cluster_db: String,
	/// the position of the last local batch
	last: Mutex<Pos>,
	/// the local tables' column types: (schema, table) -> column -> type
	types: Mutex<HashMap<(String, String), HashMap<String, String>>>,
	/// rows from above that matched no row here (an update or delete): said in yt_info
	pub missed: AtomicU64,
	/// rows from above of a table this database does not hold, or of another database:
	/// skipped, said in yt_info
	pub not_held: AtomicU64,
	/// the last batch from above that was not applied, and why
	pub apply_error: std::sync::Mutex<String>,
	/// prepared transactions the database no longer knows the end of
	pub unknown: AtomicU64,
	/// rows of a table that does not sync that way - no A row, or `no_up` / `no_down` in it:
	/// not sent, not applied
	pub not_synced: AtomicU64,
	/// the tables that sync, by `schema.table`, as read when
	syncs: Mutex<(Option<Instant>, HashMap<String, crate::sync::Sync>)>,
	/// the last outbox position whose rows of a table not syncing up were counted: a range
	/// collected again after a failed send counts nothing twice
	counted: Mutex<Pos>,
}

/// `yt_position`'s row of how far this node applied what came down
const CURSOR: &str = "cursor";

/// A database error, said with what it was about - `what` a name of the database's own
/// (a table, a function, a setting), the same in every language.
fn e<E: std::fmt::Display>(what: &'static str) -> impl Fn(E) -> String {
	move |x| t!("lite-db", what = what, error = x)
}

/// A name for SQL, quoted.
fn ident(s: &str) -> String {
	format!("\"{}\"", s.replace('"', "\"\""))
}

/// The position after `p`: a term holds at most MAX_SUBTERM subterms.
pub fn next_pos(p: Pos) -> Pos {
	if p.1 >= MAX_SUBTERM { (p.0 + 1, 1) } else { (p.0.max(1), p.1 + 1) }
}

async fn session(url: &str) -> Result<Client, String> {
	let mut cfg = tokio_postgres::Config::from_str(url).map_err(|x| t!("lite-db-url", error = x))?;
	cfg.connect_timeout(Duration::from_secs(10));
	let (client, conn) = cfg.connect(NoTls).await.map_err(|x| t!("lite-db-connect", error = x))?;
	tokio::spawn(async move {
		let _ = conn.await;
	});
	// nothing this service writes is captured by the extension's triggers; and no
	// statement holds the outbox for long
	client.batch_execute("SET yt.node = 'local'; SET statement_timeout = '30s'").await.map_err(e("SET yt.node, statement_timeout"))?;
	Ok(client)
}

/// How the database decided transaction `xid`: committed, aborted, in progress, or None
/// when it no longer knows.
async fn status(c: &Client, xid: u64) -> Result<Option<String>, String> {
	Ok(c.query_one("SELECT pg_xact_status($1::text::xid8)::text", &[&xid.to_string()]).await.map_err(e("pg_xact_status"))?.get(0))
}

impl Pg {
	/// Two sessions to the database `url` names, and the lite tables beside its config
	/// table: `config` (`schema.table`) when it is there, else the extension's
	/// `yt_config`. `cluster_db`: the database's name in the cluster above; its own by
	/// default.
	pub async fn connect(url: &str, config: Option<&str>, cluster_db: Option<String>) -> Result<Pg, String> {
		let capture = session(url).await?;
		let apply = session(url).await?;
		let db: String = capture.query_one("SELECT current_database()::text", &[]).await.map_err(e("current_database"))?.get(0);
		let named = match config.and_then(|c| c.split_once('.')) {
			Some((s, t)) => capture.query_opt("SELECT n.nspname::text, c.relname::text FROM pg_class c \
				JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind = 'r'", &[&s, &t])
				.await.map_err(e("yt_config"))?.map(|r| (r.get::<_, String>(0), r.get::<_, String>(1))),
			None => None,
		};
		let (schema, config_table) = match named {
			Some(n) => n,
			None => capture.query_opt(
				"SELECT n.nspname::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
				 WHERE c.relname = 'yt_config' AND c.relkind = 'r' ORDER BY n.nspname = 'public' DESC LIMIT 1", &[])
				.await.map_err(e("yt_config"))?.map(|r| r.get(0)).map(|s| (s, "yt_config".to_string()))
				.unwrap_or_else(|| ("public".to_string(), "yt_config".to_string())),
		};
		let s = ident(&schema);
		capture.batch_execute(&format!(
			"CREATE TABLE IF NOT EXISTS {s}.yt_outbox (
				xid bigint PRIMARY KEY,
				term bigint,
				subterm integer,
				rows bytea NOT NULL,
				at timestamptz NOT NULL DEFAULT now());
			ALTER TABLE {s}.yt_outbox ADD COLUMN IF NOT EXISTS done boolean;
			ALTER TABLE {s}.yt_outbox ADD COLUMN IF NOT EXISTS passed_at timestamptz;
			CREATE UNIQUE INDEX IF NOT EXISTS yt_outbox_pos ON {s}.yt_outbox (term, subterm);
			CREATE TABLE IF NOT EXISTS {s}.yt_position (
				name text PRIMARY KEY,
				term bigint NOT NULL,
				subterm integer NOT NULL)")).await.map_err(e("yt_outbox, yt_position"))?;
		let pg = Pg { url: url.to_string(), capture: Mutex::new(capture), apply: Mutex::new(apply), schema, config_table,
			cluster_db: cluster_db.unwrap_or_else(|| db.clone()), db, last: Mutex::new((1, 0)), types: Mutex::new(HashMap::new()),
			missed: AtomicU64::new(0), not_held: AtomicU64::new(0), apply_error: std::sync::Mutex::new(String::new()),
			unknown: AtomicU64::new(0), not_synced: AtomicU64::new(0), syncs: Mutex::new((None, HashMap::new())),
			counted: Mutex::new((0, 0)) };
		let last = pg.position("last").await?.unwrap_or((1, 0));
		*pg.last.lock().await = last;
		Ok(pg)
	}

	/// A session, opened again when it was lost: PostgreSQL restarted under this service.
	async fn session<'a>(&self, m: &'a Mutex<Client>) -> Result<MutexGuard<'a, Client>, String> {
		let mut c = m.lock().await;
		if c.is_closed() {
			*c = session(&self.url).await?;
		}
		Ok(c)
	}

	/// The extension's socket for this instance, as the extension finds it
	/// (load_socket_path): `<first unix_socket_directories entry>/.s.YT.<port>`.
	pub async fn socket_path(&self) -> Result<String, String> {
		let c = self.session(&self.capture).await?;
		let dirs: String = c.query_one("SELECT current_setting('unix_socket_directories')", &[]).await.map_err(e("unix_socket_directories"))?.get(0);
		let port: String = c.query_one("SELECT current_setting('port')", &[]).await.map_err(e("port"))?.get(0);
		let dir = dirs.split(',').next().unwrap_or("").trim_end();
		Ok(format!("{}/.s.YT.{}", dir, port))
	}

	/// The config table, quoted for SQL.
	pub fn config(&self) -> String {
		format!("{}.{}", ident(&self.schema), ident(&self.config_table))
	}

	/// yt-lite's settings: the config table's module-L rows, by name.
	pub async fn settings(&self) -> Result<HashMap<String, String>, String> {
		let c = self.session(&self.capture).await?;
		Ok(c.query(&format!("SELECT name::text, value FROM {} WHERE module = 'L'", self.config()), &[]).await
			.map_err(e("yt_config"))?.iter().map(|r| (r.get(0), r.get(1))).collect())
	}

	/// The language the database says its messages in (`lc_messages`), for yt-lite's own
	/// (i18n.rs `pick`); none when it cannot be asked.
	pub async fn lc_messages(&self) -> Option<String> {
		let c = self.session(&self.capture).await.ok()?;
		c.query_one("SELECT current_setting('lc_messages')", &[]).await.ok().map(|r| r.get(0))
	}

	/// This node's uuid: kept in the config table (module L), made at the first start.
	pub async fn node_uuid(&self) -> Result<u128, String> {
		let c = self.session(&self.capture).await?;
		let t = self.config();
		let have = c.query_opt(&format!("SELECT value FROM {t} WHERE module = 'L' AND name = 'uuid'"), &[]).await;
		if let Ok(Some(r)) = have {
			let v: String = r.get(0);
			return yt_core_proto::uuid::Uuid::parse_str(v.trim()).map(|u| u.as_u128()).map_err(|x| t!("lite-uuid-row", error = x));
		}
		let u = yt_core_proto::uuid::Uuid::new_v4();
		c.execute(&format!("INSERT INTO {t} (name, module, value) VALUES ('uuid', 'L', $1) ON CONFLICT DO NOTHING"), &[&u.to_string()])
			.await.map_err(e("yt_config, the row uuid"))?;
		Ok(u.as_u128())
	}

	async fn position(&self, name: &str) -> Result<Option<Pos>, String> {
		let c = self.session(&self.capture).await?;
		let r = c.query_opt(&format!("SELECT term, subterm FROM {}.yt_position WHERE name = $1", ident(&self.schema)), &[&name])
			.await.map_err(e("yt_position"))?;
		Ok(r.map(|r| (r.get::<_, i64>(0) as u64, r.get::<_, i32>(1) as u32)))
	}

	/// How the database decided transaction `xid` (`status`).
	pub async fn xact_status(&self, xid: u64) -> Result<Option<String>, String> {
		let c = self.session(&self.capture).await?;
		status(&c, xid).await
	}

	/// A transaction's rows, at its prepare: in the outbox at the next position, not yet
	/// decided. A transaction prepared again keeps its position.
	pub async fn prepare(&self, xid: u64, rows: Vec<DbInputEntity>) -> Result<Pos, String> {
		let bytes = RelayItem { term: 0, subterm: 0, data: rows }.encode_to_vec();
		let mut last = self.last.lock().await;
		let s = ident(&self.schema);
		let mut c = self.session(&self.capture).await?;
		let tx = c.transaction().await.map_err(e("yt_outbox"))?;
		if let Some(r) = tx.query_opt(&format!("UPDATE {s}.yt_outbox SET rows = $2, at = now() WHERE xid = $1 RETURNING term, subterm"),
			&[&(xid as i64), &bytes]).await.map_err(e("yt_outbox"))? {
			tx.commit().await.map_err(e("yt_outbox"))?;
			return Ok((r.get::<_, i64>(0) as u64, r.get::<_, i32>(1) as u32));
		}
		// the last position as the database holds it, locked until this commit
		let held = tx.query_opt(&format!("SELECT term, subterm FROM {s}.yt_position WHERE name = 'last' FOR UPDATE"), &[])
			.await.map_err(e("yt_position"))?.map(|r| (r.get::<_, i64>(0) as u64, r.get::<_, i32>(1) as u32));
		let next = next_pos(held.unwrap_or(*last).max(*last));
		tx.execute(&format!("INSERT INTO {s}.yt_outbox (xid, term, subterm, rows) VALUES ($1, $2, $3, $4)"),
			&[&(xid as i64), &(next.0 as i64), &(next.1 as i32), &bytes]).await.map_err(e("yt_outbox"))?;
		tx.execute(&format!("INSERT INTO {s}.yt_position (name, term, subterm) VALUES ('last', $1, $2) \
			ON CONFLICT (name) DO UPDATE SET term = EXCLUDED.term, subterm = EXCLUDED.subterm"),
			&[&(next.0 as i64), &(next.1 as i32)]).await.map_err(e("yt_position"))?;
		tx.commit().await.map_err(e("yt_outbox"))?;
		*last = next;
		Ok(next)
	}

	/// The extension says transaction `xid` committed: its rows go up once the database
	/// says so too (a commit asked by yt_complete(true) inside the transaction is still in
	/// progress: settled later, `resolve`). Its position, when it is decided now.
	pub async fn commit(&self, xid: u64) -> Result<Option<Pos>, String> {
		self.decide(xid).await
	}

	/// The transaction's end as the database holds it: committed - its rows go up;
	/// aborted - they go; otherwise nothing yet.
	async fn decide(&self, xid: u64) -> Result<Option<Pos>, String> {
		let s = ident(&self.schema);
		let c = self.session(&self.capture).await?;
		match status(&c, xid).await?.as_deref() {
			Some("committed") => Ok(c.query_opt(&format!("UPDATE {s}.yt_outbox SET done = true WHERE xid = $1 AND done IS NULL \
				RETURNING term, subterm"), &[&(xid as i64)]).await.map_err(e("yt_outbox"))?
				.map(|r| (r.get::<_, i64>(0) as u64, r.get::<_, i32>(1) as u32))),
			Some("aborted") => {
				c.execute(&format!("DELETE FROM {s}.yt_outbox WHERE xid = $1 AND done IS NULL"), &[&(xid as i64)]).await.map_err(e("yt_outbox"))?;
				Ok(None)
			}
			_ => Ok(None),
		}
	}

	/// A transaction rolled back: its rows go.
	pub async fn rollback(&self, xid: u64) -> Result<(), String> {
		let c = self.session(&self.capture).await?;
		c.execute(&format!("DELETE FROM {}.yt_outbox WHERE xid = $1 AND done IS NULL", ident(&self.schema)), &[&(xid as i64)])
			.await.map_err(e("yt_outbox"))?;
		Ok(())
	}

	/// The prepared transactions still undecided here (older than `secs`): taken as their
	/// database decided them. How many were settled; those it no longer knows the end of
	/// are counted in `unknown`.
	pub async fn resolve(&self, secs: f64) -> Result<usize, String> {
		let open: Vec<i64> = {
			let c = self.session(&self.capture).await?;
			c.query(&format!("SELECT xid FROM {}.yt_outbox WHERE done IS NULL AND at < now() - make_interval(secs => $1) ORDER BY term, subterm",
				ident(&self.schema)), &[&secs]).await.map_err(e("yt_outbox"))?.iter().map(|r| r.get(0)).collect()
		};
		let (mut n, mut unknown) = (0, 0);
		for xid in open {
			match self.xact_status(xid as u64).await?.as_deref() {
				Some("committed") | Some("aborted") => {
					self.decide(xid as u64).await?;
					n += 1;
				}
				None => unknown += 1,
				_ => {}
			}
		}
		self.unknown.store(unknown, Ordering::Relaxed);
		Ok(n)
	}

	/// What goes up after `from`: whole committed batches in position order, at most
	/// `max_rows` rows, up to the first still undecided - `more` when some are left. The
	/// position is the last batch taken, or `from` with none.
	pub async fn collect(&self, from: Option<Pos>, max_rows: usize) -> Result<(Vec<RelayItem>, Pos, bool), String> {
		let f = from.unwrap_or((0, 0));
		let rows = {
			let c = self.session(&self.capture).await?;
			c.query(&format!("SELECT term, subterm, rows, done FROM {}.yt_outbox \
				WHERE (term, subterm) > ($1, $2) ORDER BY term, subterm LIMIT 1000", ident(&self.schema)),
				&[&(f.0 as i64), &(f.1 as i32)]).await.map_err(e("yt_outbox"))?
		};
		let (mut items, mut n, mut upto, mut more) = (Vec::new(), 0usize, f, false);
		let total = rows.len();
		// a row goes up only while its table syncs up, decided when it is sent - as the Turso
		// leaf decides it (review of 2026-11-13): one whose table stopped syncing up since it
		// was written stays here, its position passed over, counted once
		let syncs = self.syncs_now().await?;
		let mut counted = self.counted.lock().await;
		for (i, r) in rows.iter().enumerate() {
			if r.get::<_, Option<bool>>(3) != Some(true) {
				break; // undecided: nothing after it goes before it does
			}
			let pos = (r.get::<_, i64>(0) as u64, r.get::<_, i32>(1) as u32);
			let bytes: Vec<u8> = r.get(2);
			let all = RelayItem::decode(bytes.as_slice()).map_err(e("yt_outbox, a row's bytes"))?.data;
			let before = all.len();
			let data: Vec<DbInputEntity> = all.into_iter().filter(|row| row.table.as_ref()
				.is_some_and(|t| syncs.get(&format!("{}.{}", t.schema, t.table)).is_some_and(|s| s.up))).collect();
			if pos > *counted {
				self.not_synced.fetch_add((before - data.len()) as u64, Ordering::Relaxed);
				*counted = pos;
			}
			if !items.is_empty() && n + data.len() > max_rows {
				more = true;
				break;
			}
			n += data.len();
			items.push(RelayItem { term: pos.0, subterm: pos.1, data });
			upto = pos;
			if i + 1 == total && total == 1000 {
				more = true;
			}
		}
		Ok((items, upto, more))
	}

	/// The cluster above holds everything through `upto`: marked, and kept a day - a
	/// cluster restored from a backup asks for it again.
	pub async fn passed(&self, upto: Pos) -> Result<(), String> {
		let s = ident(&self.schema);
		let c = self.session(&self.capture).await?;
		c.execute(&format!("UPDATE {s}.yt_outbox SET passed_at = now() WHERE done AND passed_at IS NULL AND (term, subterm) <= ($1, $2)"),
			&[&(upto.0 as i64), &(upto.1 as i32)]).await.map_err(e("yt_outbox"))?;
		c.execute(&format!("DELETE FROM {s}.yt_outbox WHERE passed_at < now() - interval '1 day'"), &[]).await.map_err(e("yt_outbox"))?;
		c.execute(&format!("INSERT INTO {s}.yt_position (name, term, subterm) VALUES ($1, $2, $3) \
			ON CONFLICT (name) DO UPDATE SET term = EXCLUDED.term, subterm = EXCLUDED.subterm"),
			&[&"passed", &(upto.0 as i64), &(upto.1 as i32)]).await.map_err(e("yt_position"))?;
		Ok(())
	}

	/// How far this node applied what came down: one cursor - a lite node has one
	/// cluster above, named by no az.
	pub async fn cursor(&self) -> Result<Option<Pos>, String> {
		self.position(CURSOR).await
	}

	/// The tables that sync and which way: the config table's A rows, read again after two
	/// seconds - every row the extension sends asks; that is an early answer only. What is
	/// sent and what is applied is decided on them as they are (`syncs_now`).
	pub async fn syncs(&self) -> Result<HashMap<String, crate::sync::Sync>, String> {
		{
			let g = self.syncs.lock().await;
			if g.0.is_some_and(|at| at.elapsed() < Duration::from_secs(2)) {
				return Ok(g.1.clone());
			}
		}
		self.syncs_now().await
	}

	/// The A rows as they are now (review of 2026-11-13: a row that came down within two
	/// seconds of its table's A row was skipped for good).
	pub async fn syncs_now(&self) -> Result<HashMap<String, crate::sync::Sync>, String> {
		let rows = {
			let c = self.session(&self.capture).await?;
			c.query(&format!("SELECT name::text, coalesce(value::text, '') FROM {}.{} WHERE module = 'A'",
				ident(&self.schema), ident(&self.config_table)), &[]).await.map_err(e("yt_config"))?
		};
		let t = crate::sync::tables(rows.iter().map(|r| (r.get::<_, &str>(0), r.get::<_, &str>(1))));
		*self.syncs.lock().await = (Some(Instant::now()), t.clone());
		Ok(t)
	}

	/// Local batches not yet passed by the cluster above.
	pub async fn waiting(&self) -> Result<i64, String> {
		let c = self.session(&self.capture).await?;
		Ok(c.query_one(&format!("SELECT count(*) FROM {}.yt_outbox WHERE passed_at IS NULL", ident(&self.schema)), &[])
			.await.map_err(e("yt_outbox"))?.get(0))
	}

	/// A local table's column types - `None` for a table this database does not have,
	/// which is asked again each time: it may be made later.
	async fn columns(&self, c: &Client, schema: &str, table: &str) -> Result<Option<HashMap<String, String>>, String> {
		let key = (schema.to_string(), table.to_string());
		if let Some(t) = self.types.lock().await.get(&key) {
			return Ok(Some(t.clone()));
		}
		let rows = c.query("SELECT a.attname::text, format_type(a.atttypid, a.atttypmod) FROM pg_attribute a \
			JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
			WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped", &[&schema, &table])
			.await.map_err(e("pg_attribute"))?;
		if rows.is_empty() {
			return Ok(None);
		}
		let t: HashMap<String, String> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
		self.types.lock().await.insert(key, t.clone());
		Ok(Some(t))
	}

	/// Apply a batch that came down, with its cursor, in one transaction: what is at or
	/// below the cursor is skipped, so a batch sent twice changes nothing. A row of a
	/// table this database does not hold - or of another database - is not this node's:
	/// left out, counted. A batch that cannot be applied is refused whole - never a row
	/// of a held table skipped - said in yt_info, and tried again at the next turn with
	/// the tables read again (a schema changed).
	pub async fn apply(&self, batch: &RelayBatch) -> Result<Option<Pos>, String> {
		let r = self.apply_batch(batch).await;
		{
			let mut said = self.apply_error.lock().unwrap_or_else(|e| e.into_inner());
			*said = r.as_ref().err().cloned().unwrap_or_default();
		}
		if r.is_err() {
			self.types.lock().await.clear();
		}
		r
	}

	async fn apply_batch(&self, batch: &RelayBatch) -> Result<Option<Pos>, String> {
		// the apply session first, held to the end: one apply at a time, so the cursor read
		// under it is the one this batch is applied past (review of 2026-11-13)
		let mut c = self.session(&self.apply).await?;
		let at = self.cursor().await?;
		let Some(upto) = to_pos(&batch.upto) else { return Ok(at) };
		if Some(upto) <= at {
			return Ok(at);
		}
		let mut plans = Vec::new();
		let (mut not_held, mut not_synced) = (0, 0);
		let syncs = self.syncs_now().await?;
		for item in batch.items.iter().filter(|i| Some((i.term, i.subterm)) > at && (i.term, i.subterm) <= upto) {
			for row in &item.data {
				let t = row.table.as_ref().ok_or_else(|| t!("lite-row-no-table"))?;
				// what the az of the node above holds, in every database of its cluster: this
				// node's are the tables its one database has
				if t.db != self.cluster_db {
					not_held += 1;
					continue;
				}
				// ... and of those, the ones that sync down here (an A row, no no_down)
				if !syncs.get(&format!("{}.{}", t.schema, t.table)).is_some_and(|s| s.down) {
					not_synced += 1;
					continue;
				}
				match self.columns(&c, &t.schema, &t.table).await? {
					Some(types) => plans.push(statement(row, &types)?),
					None => not_held += 1,
				}
			}
		}
		let tx = c.transaction().await.map_err(e("the apply's transaction"))?;
		tx.batch_execute("SET LOCAL yt.node = 'local'").await.map_err(e("the apply's transaction"))?;
		let mut missed = 0;
		for (sql, params, must_match) in &plans {
			let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
			let n = tx.execute(sql.as_str(), &refs).await.map_err(|x| t!("lite-statement", sql = sql, error = x))?;
			if *must_match && n == 0 {
				missed += 1;
			}
		}
		tx.execute(&format!("INSERT INTO {}.yt_position (name, term, subterm) VALUES ($1, $2, $3) \
			ON CONFLICT (name) DO UPDATE SET term = EXCLUDED.term, subterm = EXCLUDED.subterm", ident(&self.schema)),
			&[&CURSOR, &(upto.0 as i64), &(upto.1 as i32)]).await.map_err(e("yt_position"))?;
		tx.commit().await.map_err(e("the apply's transaction"))?;
		self.missed.fetch_add(missed, Ordering::Relaxed);
		self.not_held.fetch_add(not_held, Ordering::Relaxed);
		self.not_synced.fetch_add(not_synced, Ordering::Relaxed);
		Ok(Some(upto))
	}
}

#[tonic::async_trait]
impl crate::db::Db for Pg {
	async fn waiting(&self) -> Result<i64, String> {
		Pg::waiting(self).await
	}
	async fn cursor(&self) -> Result<Option<Pos>, String> {
		Pg::cursor(self).await
	}
	async fn collect(&self, from: Option<Pos>, max_rows: usize) -> Result<(Vec<RelayItem>, Pos, bool), String> {
		Pg::collect(self, from, max_rows).await
	}
	async fn apply(&self, batch: &RelayBatch) -> Result<Option<Pos>, String> {
		Pg::apply(self, batch).await
	}
	async fn passed(&self, upto: Pos) -> Result<(), String> {
		Pg::passed(self, upto).await
	}
}

/// A value as text, for a cast to the local column's type; None for NULL. A float in the
/// spelling PostgreSQL reads: NaN, Infinity, -Infinity.
fn text(v: &yt_core_proto::base::VarType) -> Result<Option<String>, String> {
	Ok(match &v.value {
		None => None,
		Some(Value::Int(i)) => Some(i.to_string()),
		Some(Value::UInt(u)) => Some(u.to_string()),
		Some(Value::Flt(f)) if f.is_nan() => Some("NaN".to_string()),
		Some(Value::Flt(f)) if f.is_infinite() => Some(if *f > 0.0 { "Infinity" } else { "-Infinity" }.to_string()),
		Some(Value::Flt(f)) => Some(f.to_string()),
		Some(Value::Bool(b)) => Some(b.to_string()),
		Some(Value::Universal(s)) => Some(s.clone()),
		Some(Value::Diff(_)) => return Err(full_version("a document merged path by path")),
	})
}

/// The statement that applies one row here, its parameters (text, cast to each column's
/// local type), and whether it must match a row (an update or a delete).
///
/// A row is found by its key; a row of a table without one by every column of its old
/// image, one row of the equal ones (`ctid`), as the extension sends the whole old row
/// for such a table. An insert of a key already here is a batch sent again: nothing.
pub fn statement(row: &DbInputEntity, types: &HashMap<String, String>) -> Result<(String, Vec<Option<String>>, bool), String> {
	let t = row.table.as_ref().ok_or_else(|| t!("lite-row-no-table"))?;
	let table = format!("{}.{}", ident(&t.schema), ident(&t.table));
	let name = |attnum: &u32| t.columns.iter().find(|c| c.attnum == *attnum).map(|c| c.name.clone())
		.ok_or_else(|| t!("lite-no-attnum", table = table, attnum = attnum));
	let ty = |col: &str| types.get(col).cloned().ok_or_else(|| t!("lite-no-column", table = table, column = col));
	let mut params: Vec<Option<String>> = Vec::new();
	let mut arg = |v: Option<String>, ty: String| { params.push(v); format!("${}::text::{}", params.len(), ty) };
	let sorted = |m: &HashMap<u32, yt_core_proto::base::VarType>| { let mut k: Vec<u32> = m.keys().copied().collect(); k.sort(); k };
	let pk: Vec<&yt_core_proto::base::ColumnRef> = t.columns.iter().filter(|c| c.pk).collect();
	let action = DbAction::try_from(row.action).map_err(|_| DbAction::to_str(row.action))?;
	let (sql, must_match) = match action {
		DbAction::Insert => {
			let keys = sorted(&row.new_data);
			let (mut cols, mut vals) = (Vec::new(), Vec::new());
			for k in &keys {
				let c = name(k)?;
				vals.push(arg(text(&row.new_data[k])?, ty(&c)?));
				cols.push(ident(&c));
			}
			// a conflict on the key only: another unique column's is an error, never skipped
			let conflict = if pk.is_empty() { String::new() } else {
				format!(" ON CONFLICT ({}) DO NOTHING", pk.iter().map(|c| ident(&c.name)).collect::<Vec<_>>().join(", "))
			};
			(format!("INSERT INTO {} ({}) VALUES ({}){}", table, cols.join(", "), vals.join(", "), conflict), false)
		}
		DbAction::Update | DbAction::Delete => {
			let mut set = Vec::new();
			if action == DbAction::Update {
				for k in sorted(&row.new_data) {
					// a key column is set only when the update changed it
					if pk.iter().any(|c| c.attnum == k) && row.old_data.get(&k).is_none_or(|o| *o == row.new_data[&k]) {
						continue;
					}
					let c = name(&k)?;
					set.push(format!("{} = {}", ident(&c), arg(text(&row.new_data[&k])?, ty(&c)?)));
				}
				if set.is_empty() {
					return Ok(("SELECT 1".to_string(), Vec::new(), false)); // nothing changed
				}
			}
			let mut wh = Vec::new();
			if pk.is_empty() {
				if row.old_data.is_empty() {
					return Err(t!("lite-keyless-no-old", table = table));
				}
				for k in sorted(&row.old_data) {
					let c = name(&k)?;
					wh.push(format!("{} IS NOT DISTINCT FROM {}", ident(&c), arg(text(&row.old_data[&k])?, ty(&c)?)));
				}
			} else {
				for c in &pk {
					let v = row.old_data.get(&c.attnum).or_else(|| row.new_data.get(&c.attnum))
						.ok_or_else(|| t!("lite-no-key-value", table = table, column = c.name))?;
					wh.push(format!("{} = {}", ident(&c.name), arg(text(v)?, ty(&c.name)?)));
				}
			}
			let wh = if pk.is_empty() {
				format!("ctid = (SELECT ctid FROM {} WHERE {} LIMIT 1)", table, wh.join(" AND "))
			} else {
				wh.join(" AND ")
			};
			if action == DbAction::Update {
				(format!("UPDATE {} SET {} WHERE {}", table, set.join(", "), wh), true)
			} else {
				(format!("DELETE FROM {} WHERE {}", table, wh), true)
			}
		}
		DbAction::Truncate => (format!("TRUNCATE {}", table), false),
	};
	Ok((sql, params, must_match))
}

#[cfg(test)]
mod tests {
	use super::*;
	use yt_core_proto::base::{ColumnRef, TableRef, VarType};

	fn row(action: DbAction, old: &[(u32, &str)], new: &[(u32, &str)]) -> DbInputEntity {
		let v = |s: &str| VarType { value: Some(Value::Universal(s.into())) };
		DbInputEntity {
			table: Some(TableRef { db: "appdb".into(), schema: "public".into(), table: "t".into(), columns: vec![
				ColumnRef { attnum: 1, name: "id".into(), pk: true, ..Default::default() },
				ColumnRef { attnum: 2, name: "name".into(), ..Default::default() }], ..Default::default() }),
			old_data: old.iter().map(|(k, s)| (*k, v(s))).collect(),
			new_data: new.iter().map(|(k, s)| (*k, v(s))).collect(),
			action: action as i32,
			..Default::default()
		}
	}

	fn types() -> HashMap<String, String> {
		HashMap::from([("id".to_string(), "bigint".to_string()), ("name".to_string(), "text".to_string())])
	}

	#[test]
	fn a_row_is_applied_by_its_key_with_the_local_types() {
		let (sql, p, _) = statement(&row(DbAction::Insert, &[], &[(1, "7"), (2, "x")]), &types()).unwrap();
		assert_eq!(sql, "INSERT INTO \"public\".\"t\" (\"id\", \"name\") VALUES ($1::text::bigint, $2::text::text) ON CONFLICT (\"id\") DO NOTHING");
		assert_eq!(p, vec![Some("7".into()), Some("x".into())]);
		let (sql, p, m) = statement(&row(DbAction::Update, &[(1, "7")], &[(1, "7"), (2, "y")]), &types()).unwrap();
		assert!(m, "an update must match a row");
		assert_eq!(sql, "UPDATE \"public\".\"t\" SET \"name\" = $1::text::text WHERE \"id\" = $2::text::bigint");
		assert_eq!(p, vec![Some("y".into()), Some("7".into())]);
		let (sql, _, _) = statement(&row(DbAction::Update, &[(1, "7")], &[(1, "8")]), &types()).unwrap();
		assert_eq!(sql, "UPDATE \"public\".\"t\" SET \"id\" = $1::text::bigint WHERE \"id\" = $2::text::bigint", "a key changed");
		let (sql, _, _) = statement(&row(DbAction::Delete, &[(1, "7")], &[]), &types()).unwrap();
		assert_eq!(sql, "DELETE FROM \"public\".\"t\" WHERE \"id\" = $1::text::bigint");
		let (sql, _, m) = statement(&row(DbAction::Truncate, &[], &[]), &types()).unwrap();
		assert!(!m);
		assert_eq!(sql, "TRUNCATE \"public\".\"t\"");
	}

	#[test]
	fn a_row_that_does_not_fit_here_is_refused() {
		let mut r = row(DbAction::Insert, &[], &[(1, "7"), (3, "z")]);
		assert!(statement(&r, &types()).is_err(), "no column of attnum 3");
		r.table.as_mut().unwrap().columns.push(yt_core_proto::base::ColumnRef { attnum: 3, name: "gone".into(), ..Default::default() });
		assert!(statement(&r, &types()).unwrap_err().contains("no column gone here"));
		let mut keyless = row(DbAction::Delete, &[], &[]);
		keyless.table.as_mut().unwrap().columns[0].pk = false;
		assert!(statement(&keyless, &types()).is_err(), "no key and no old image");
		assert!(one_database("appdb").starts_with("0A000: "));
	}

	#[test]
	fn a_row_of_a_keyless_table_is_found_by_its_whole_old_image_one_row_of_the_equal_ones() {
		let mut r = row(DbAction::Delete, &[(1, "7"), (2, "x")], &[]);
		r.table.as_mut().unwrap().columns[0].pk = false;
		let (sql, p, m) = statement(&r, &types()).unwrap();
		assert_eq!(sql, "DELETE FROM \"public\".\"t\" WHERE ctid = (SELECT ctid FROM \"public\".\"t\" WHERE \"id\" IS NOT DISTINCT FROM $1::text::bigint AND \"name\" IS NOT DISTINCT FROM $2::text::text LIMIT 1)");
		assert_eq!(p, vec![Some("7".into()), Some("x".into())]);
		assert!(m);
		let mut keyless_insert = row(DbAction::Insert, &[], &[(1, "7")]);
		keyless_insert.table.as_mut().unwrap().columns[0].pk = false;
		assert!(!statement(&keyless_insert, &types()).unwrap().0.contains("ON CONFLICT"));
	}

	#[test]
	fn a_float_is_spelled_as_postgresql_reads_it() {
		let f = |x: f64| text(&VarType { value: Some(Value::Flt(x)) }).unwrap().unwrap();
		assert_eq!((f(f64::NAN), f(f64::INFINITY), f(f64::NEG_INFINITY), f(1.5)), ("NaN".into(), "Infinity".into(), "-Infinity".into(), "1.5".into()));
	}

	#[test]
	fn a_term_holds_its_subterms_then_the_next_term_starts() {
		assert_eq!(next_pos((1, 0)), (1, 1));
		assert_eq!(next_pos((1, 5)), (1, 6));
		assert_eq!(next_pos((3, MAX_SUBTERM)), (4, 1));
		assert_eq!(next_pos((0, 0)), (1, 1));
	}
}
