// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! yt: YT Cluster lite as a SQLite loadable extension (README.md). A SQLite database file is
//! a lite node of a YT cluster: the tables set up in it go up to the cluster, and what the
//! cluster holds of them comes down - both ways, as yt-lite does for PostgreSQL and the
//! `turso` feature for an application on Turso. One store for all three SQLite-dialect
//! engines: yt_core::lite.
//!
//! Its functions, on the connection that loaded it:
//!
//! - `yt_setup(table)` capture a table's writes and sync it both ways (its triggers, its A row)
//! - `yt_sync(table [, value])` sync a table with no capture: its rows only come down; the
//!   value is the cluster's encoding of which ways (`ytctl --encode-table-cfg`)
//! - `yt_unsync(table)` stop syncing a table: its A row and its triggers go
//! - `yt_link()` / `yt_link(upper, upper_root)` link the database file to the cluster above,
//!   from its `yt_config` rows of module L (the two given are written there first); returns
//!   the node's CA fingerprint, which the cluster's operator approves
//! - `yt_unlink()` stop the link of this database file
//! - `yt_info()` where the link is, as JSON
mod conn;
mod link;

use std::future::Future;
use std::os::raw::{c_char, c_int};
use std::path::PathBuf;
use std::task::{Context as Cx, Poll, Waker};

use rusqlite::functions::{Context, FunctionFlags};
use rusqlite::{ffi, Connection, Error, OptionalExtension, Result};
use serde_json::json;
use yt_core::db::Db;
use yt_core::lite::Lite;
use yt_core_proto::uuid::{Builder, Uuid};

use crate::conn::Sqlite;

/// SQLite's entry point for `yt.so` / `yt.dylib` / `yt.dll` (`.load yt`).
///
/// # Safety
/// Called by SQLite's loader only, with its own arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite3_yt_init(db: *mut ffi::sqlite3, err: *mut *mut c_char,
	api: *mut ffi::sqlite3_api_routines) -> c_int {
	unsafe { Connection::extension_init2(db, err, api, init) }
}

/// The entry point by its general name: a host that loads the file renamed, or names it.
///
/// # Safety
/// Called by SQLite's loader only, with its own arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite3_extension_init(db: *mut ffi::sqlite3, err: *mut *mut c_char,
	api: *mut ffi::sqlite3_api_routines) -> c_int {
	unsafe { Connection::extension_init2(db, err, api, init) }
}

fn init(db: Connection) -> Result<bool> {
	// side effects: from SQL written by hand only, never from a trigger or a view
	let f = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DIRECTONLY;
	db.create_scalar_function("yt_setup", 1, f, |c| {
		let t: String = c.get(0)?;
		on_caller(c, |s| now(s.setup(&t))).map(|_| t)
	})?;
	db.create_scalar_function("yt_sync", 1, f, |c| {
		let t: String = c.get(0)?;
		on_caller(c, |s| now(s.sync(&t, ""))).map(|_| t)
	})?;
	db.create_scalar_function("yt_sync", 2, f, |c| {
		let (t, v): (String, String) = (c.get(0)?, c.get(1)?);
		on_caller(c, |s| now(s.sync(&t, &v))).map(|_| t)
	})?;
	db.create_scalar_function("yt_unsync", 1, f, |c| {
		let t: String = c.get(0)?;
		on_caller(c, |s| now(s.unsync(&t))).map(|_| t)
	})?;
	db.create_scalar_function("yt_link", 0, f, |c| link(c, None))?;
	db.create_scalar_function("yt_link", 2, f, |c| {
		let upper: String = c.get(0)?;
		let root: Option<String> = c.get(1)?;
		link(c, Some((upper, root)))
	})?;
	db.create_scalar_function("yt_unlink", 0, f, |c| Ok(link::stop(&path(&caller(c)?))))?;
	// a closure, not `info` itself: the function's lifetimes are not the bound's (E0631)
	#[allow(clippy::redundant_closure)]
	db.create_scalar_function("yt_info", 0, f, |c| info(c))?;
	Ok(false)
}

fn user(e: String) -> Error {
	Error::UserFunctionError(e.into())
}

/// A future of the store run to its end here. Its connection answers at once (no I/O
/// waits on a runtime), so no runtime: one would panic in a host that runs inside tokio.
fn now<T>(f: impl Future<Output = T>) -> T {
	let mut f = std::pin::pin!(f);
	let mut cx = Cx::from_waker(Waker::noop());
	loop {
		if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
			return v;
		}
		std::thread::yield_now();
	}
}

/// The calling connection itself - not closed when this handle is dropped.
fn caller(c: &Context<'_>) -> Result<Connection> {
	unsafe { Connection::from_handle(c.get_connection()?.handle()) }
}

/// The main database's file; empty for one in memory or temporary.
fn path(c: &Connection) -> String {
	c.path().unwrap_or("").to_string()
}

/// `f` on a store of the calling connection: the triggers and the A rows are written in the
/// application's own transaction, if it is in one.
fn on_caller(c: &Context<'_>, f: impl FnOnce(&Lite<Sqlite>) -> Result<(), String>) -> Result<()> {
	let s = now(Lite::new(Sqlite::new(caller(c)?), "")).map_err(user)?;
	f(&s).map_err(user)
}

fn setting(c: &Connection, name: &str) -> Result<Option<String>> {
	c.query_row("SELECT value FROM yt_config WHERE module = 'L' AND name = ?1", [name], |r| r.get(0))
		.optional().map(|v: Option<String>| v.filter(|v| !v.trim().is_empty()))
}

fn put(c: &Connection, name: &str, value: &str) -> Result<()> {
	c.execute("INSERT INTO yt_config (name, module, value) VALUES (?1, 'L', ?2) \
		ON CONFLICT (module, name) DO UPDATE SET value = excluded.value", [name, value]).map(|_| ())
}

/// An `upper_root` as `ytctl --skv-status` prints one: `SHA256:` and 43 characters of base64.
fn is_root(v: &str) -> bool {
	v.strip_prefix("SHA256:").is_some_and(|b| b.len() == 43 && b.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/'))
}

/// `yt_link`: the settings from module L (`upper` and `upper_root` written there first when
/// given), a uuid made and kept there on the first link, then the link's thread. Outside a
/// transaction only: a rollback would take the settings and the uuid while the link runs
/// (review of 2026-11-14). Given settings relink a file already linked, with them.
fn link(c: &Context<'_>, given: Option<(String, Option<String>)>) -> Result<String> {
	let db = caller(c)?;
	let file = path(&db);
	if file.is_empty() {
		return Err(user(yt_core::lite::no_file("main")));
	}
	if !db.is_autocommit() {
		return Err(user(yt_core::lite::in_transaction()));
	}
	if let Some(r) = given.as_ref().and_then(|g| g.1.as_deref()).filter(|r| !r.trim().is_empty() && !is_root(r.trim())) {
		return Err(user(yt_core::lite::bad_root(r)));
	}
	// the store's tables, yt_config among them, before any setting is read or written
	now(Lite::new(Sqlite::new(caller(c)?), "")).map_err(user)?;
	if let Some((upper, root)) = given {
		put(&db, "upper", &upper)?;
		if let Some(r) = root.filter(|r| !r.trim().is_empty()) {
			put(&db, "upper_root", r.trim())?;
		}
		// the link running already, with the settings before these: stopped, made again below
		link::stop(&file);
	}
	let root = setting(&db, "upper_root")?;
	if let Some(r) = root.as_deref().filter(|r| !is_root(r.trim())) {
		return Err(user(yt_core::lite::bad_root(r)));
	}
	let upper: Vec<String> = setting(&db, "upper")?.unwrap_or_default().split(',')
		.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
	if upper.is_empty() {
		return Err(user(yt_core::lite::no_upper()));
	}
	let uuid = match setting(&db, "uuid")?.and_then(|u| Uuid::parse_str(u.trim()).ok()) {
		Some(u) => u,
		None => {
			let mut b = [0u8; 16];
			openssl::rand::rand_bytes(&mut b).map_err(|e| user(e.to_string()))?;
			let u = Builder::from_random_bytes(b).into_uuid();
			put(&db, "uuid", &u.to_string())?;
			u
		}
	};
	yt_core::i18n::set(yt_core::i18n::pick(setting(&db, "lang")?.as_deref(), None,
		yt_core::i18n::system_locale().as_deref()));
	// this database's name in the cluster above: as set, else the file's name
	let cluster_db = setting(&db, "cluster_db")?.unwrap_or_else(|| {
		let p = PathBuf::from(&file);
		p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "main".into())
	});
	let key_dir = setting(&db, "key_dir")?.map(PathBuf::from).unwrap_or_else(|| {
		let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
			.unwrap_or_else(std::env::temp_dir);
		home.join(".yt-lite").join(uuid.to_string())
	});
	link::start(link::Settings { path: file, upper, upper_root: root.map(|r| r.trim().to_string()), cluster_db, key_dir, uuid })
		.map_err(user)
}

/// `yt_info()`: the link's state and counts, as JSON; `linked: false` for a file not linked.
fn info(c: &Context<'_>) -> Result<String> {
	let db = caller(c)?;
	let file = path(&db);
	// what it needs taken out of the registry, its lock let go: the count below may wait on
	// the link's connection (its busy_timeout), and no other connection waits on that
	let v = link::with(&file, |l| l.view()).map(|l| {
		let s = l.status.lock().unwrap_or_else(|e| e.into_inner()).clone();
		let waiting = now(l.store.waiting()).unwrap_or(-1);
		let n = |a: &std::sync::atomic::AtomicU64| a.load(std::sync::atomic::Ordering::Relaxed);
		json!({
			"version": env!("CARGO_PKG_VERSION"),
			"database": file,
			"linked": true,
			"uuid": l.uuid.to_string(),
			"fingerprint": l.fingerprint,
			"cluster_db": l.cluster_db,
			"upper": l.upper,
			"granted": s.granted,
			"az": s.az,
			"tls": s.tls,
			"caught_up": s.caught_up,
			"waiting": waiting,
			"problem": s.problem,
			"turns": s.turns,
			"missed": n(&l.store.missed),
			"not_held": n(&l.store.not_held),
			"not_sent": n(&l.store.not_sent),
			"not_synced": n(&l.store.not_synced),
			"apply_error": l.store.apply_error.lock().unwrap_or_else(|e| e.into_inner()).clone(),
			"said": l.said.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect::<Vec<_>>(),
		})
	});
	Ok(v.unwrap_or_else(|| json!({ "version": env!("CARGO_PKG_VERSION"), "database": file, "linked": false })).to_string())
}
