// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! The link of a database file to the cluster above: one a process for each file, run on a
//! thread of its own, with a connection of its own to the file - what goes up is read there,
//! so only what the application committed, and what comes down is applied there. It runs
//! until `yt_unlink()` or the process ends; the connection that started it may close.
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use rusqlite::Connection;
use yt_core::core::store::Node;
use yt_core::leaf::{self, Status};
use yt_core::lite::Lite;
use yt_core::tls::Tls;
use yt_core_proto::uuid::Uuid;

use crate::conn::Sqlite;

/// how many of the link's messages `yt_info()` keeps, the newest last
const SAID: usize = 50;

/// What a link needs, from the database's `yt_config` rows of module L.
pub struct Settings {
	/// the database file
	pub path: String,
	/// the nodes of the cluster above, tried in turn
	pub upper: Vec<String>,
	/// the cluster's root CA, `SHA256:...`: only what chains to it is taken
	pub upper_root: Option<String>,
	/// this database's name in the cluster above
	pub cluster_db: String,
	/// where this node keeps its CA
	pub key_dir: PathBuf,
	pub uuid: Uuid,
}

/// What `yt_info()` reads of a link, taken out of the registry: no lock held while it reads.
pub struct View {
	pub status: Arc<Mutex<Status>>,
	pub said: Arc<Mutex<VecDeque<String>>>,
	pub store: Arc<Lite<Sqlite>>,
	pub uuid: Uuid,
	pub fingerprint: String,
	pub cluster_db: String,
	pub upper: Vec<String>,
}

/// A running link.
pub struct Link {
	stop: Arc<AtomicBool>,
	thread: Option<JoinHandle<()>>,
	pub status: Arc<Mutex<Status>>,
	pub said: Arc<Mutex<VecDeque<String>>>,
	pub store: Arc<Lite<Sqlite>>,
	pub uuid: Uuid,
	pub fingerprint: String,
	pub cluster_db: String,
	pub upper: Vec<String>,
}

impl Link {
	pub fn view(&self) -> View {
		View { status: self.status.clone(), said: self.said.clone(), store: self.store.clone(), uuid: self.uuid,
			fingerprint: self.fingerprint.clone(), cluster_db: self.cluster_db.clone(), upper: self.upper.clone() }
	}
}

fn links() -> &'static Mutex<HashMap<String, Link>> {
	static L: OnceLock<Mutex<HashMap<String, Link>>> = OnceLock::new();
	L.get_or_init(|| Mutex::new(HashMap::new()))
}

fn said(s: &Mutex<VecDeque<String>>, m: String) {
	let mut s = s.lock().unwrap_or_else(|e| e.into_inner());
	if s.len() == SAID {
		s.pop_front();
	}
	s.push_back(m);
}

/// Link the database file `s.path`: its store's connection, its CA, its thread. A file
/// already linked in this process is not linked twice: its CA's fingerprint, as it is.
pub fn start(s: Settings) -> Result<String, String> {
	let mut all = links().lock().unwrap_or_else(|e| e.into_inner());
	if let Some(l) = all.get(&s.path) {
		return Ok(l.fingerprint.clone());
	}
	let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("yt-link")
		.enable_all().build().map_err(|e| e.to_string())?;
	let conn = Connection::open(&s.path).map_err(|e| format!("{}: {}", s.path, e))?;
	// one writer at a time: the application's writes and this node's applies wait for each other
	conn.busy_timeout(Duration::from_secs(10)).map_err(|e| e.to_string())?;
	// WAL: the application reads while the link writes, and the other way round. Kept as
	// the file has it when it cannot change now (another connection in a transaction).
	let _ = conn.pragma_update(None, "journal_mode", "WAL");
	let store = Arc::new(rt.block_on(Lite::new(Sqlite::new(conn), &s.cluster_db))?);
	let tls = Arc::new(Tls::open(&s.key_dir, &s.uuid.to_string(), s.upper_root.as_deref())?);
	let fingerprint = tls.fingerprint().to_string();
	let node = Arc::new(Node::with_db(s.uuid.as_u128(), s.upper.clone(), store.clone()).with_tls(tls));
	let status = Arc::new(Mutex::new(Status::default()));
	let words = Arc::new(Mutex::new(VecDeque::new()));
	let stop = Arc::new(AtomicBool::new(false));
	let (st, w, sp) = (status.clone(), words.clone(), stop.clone());
	let thread = std::thread::Builder::new().name("yt-link".into())
		.spawn(move || rt.block_on(leaf::run_until(node, st, move |m| said(&w, m), sp)))
		.map_err(|e| e.to_string())?;
	all.insert(s.path.clone(), Link { stop, thread: Some(thread), status, said: words, store, uuid: s.uuid,
		fingerprint: fingerprint.clone(), cluster_db: s.cluster_db, upper: s.upper });
	Ok(fingerprint)
}

/// Stop the link of `path`, once its turn ends (a second at most, never in the middle of
/// an apply). False: it was not linked.
pub fn stop(path: &str) -> bool {
	let l = links().lock().unwrap_or_else(|e| e.into_inner()).remove(path);
	match l {
		Some(mut l) => {
			l.stop.store(true, Ordering::Relaxed);
			if let Some(t) = l.thread.take() {
				let _ = t.join();
			}
			true
		}
		None => false,
	}
}

/// What `f` takes from the link of `path`, if it is linked.
pub fn with<T>(path: &str, f: impl FnOnce(&Link) -> T) -> Option<T> {
	links().lock().unwrap_or_else(|e| e.into_inner()).get(path).map(f)
}
