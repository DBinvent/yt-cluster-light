// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! The light node's version of a replaceable file: the node itself, its rows and
//! its cursor. (ytserv has its own `store.rs`: the cluster log and its apply lane.)
//!
//! **A light node has no az.** An az places a cluster's nodes and relates clusters to
//! each other - which leader signs in above, which neighbours it dials. A light node is
//! none of that: a leaf of one cluster, the lowest there is, with nothing below it, never
//! a leader, never dialling a neighbour. So it names no az, for itself or for the cluster
//! above - wherever the shared link files ask for one (`my_az`, the `upper` of a relay
//! turn) it answers with none. The node it is linked to takes it for a node of its own
//! az and says which (`told_az`); a light node cannot set that.
//!
//! A node's store (`crate::db::Db`) holds both: in yt-light, the service beside one
//! PostgreSQL instance, that database (`crate::pg`); inside an application, its own
//! database (`crate::turso`), in one transaction with the cursor. Without one the rows live
//! in memory: committed local batches wait in the outbox, and what comes down is kept as it
//! arrived.
use std::collections::BTreeMap;
use tokio::sync::Mutex;
use yt_core_proto::base::{BaseApi, PeerUuid};
use yt_core_proto::db::DbInputEntity;
use yt_core_proto::link::{RelayBatch, RelayItem};

use super::pos::{to_pos, Pos};
use super::route::Flow;

/// The cluster above, where a shared link file asks which az that is: a light node has
/// one cluster above and names no az for it.
pub const ABOVE: &str = "";

/// A light node: a leaf of one cluster, known there by its uuid. It has no az (see the
/// head of this file).
pub struct Node {
	/// its own uuid: what the cluster above knows it by
	pub uuid: u128,
	/// nodes of the cluster above to dial: `http(s)://host:port`, tried in turn
	pub hosts: Vec<String>,
	/// the next host to try (calls.rs)
	pub(crate) next_host: Mutex<usize>,
	/// how far this node applied what came down: one cursor, the cluster above's history
	pub(crate) cursor: Mutex<Option<Pos>>,
	/// the az the node above takes this one to be in, as its last grant said - its own;
	/// `None` until a sign-in was granted. Told, never set here
	pub(crate) az: Mutex<Option<String>>,
	/// local batches committed and not yet passed by the cluster above: by position
	pub(crate) outbox: Mutex<BTreeMap<Pos, Vec<DbInputEntity>>>,
	/// what came down, as it arrived
	pub(crate) applied: Mutex<Vec<RelayItem>>,
	/// the last challenge the cluster above handed (offer.rs)
	pub(crate) challenge: Mutex<Vec<u8>>,
	/// the channel to the target, kept between calls: its target, and - when it is TLS -
	/// which change of the chain it was made with (calls.rs)
	pub(crate) channel: Mutex<Option<(String, Option<u64>, tonic::transport::Channel)>>,
	/// the database that is the store: yt-light's PostgreSQL, an application's Turso
	pub db: Option<std::sync::Arc<dyn crate::db::Db>>,
	/// its CA, offered at every sign-in, and the TLS of its calls once the cluster above
	/// countersigned it (crate::tls)
	#[cfg(feature = "tls")]
	pub tls: Option<std::sync::Arc<crate::tls::Tls>>,
}

impl Node {
	pub fn new(uuid: u128, hosts: Vec<String>) -> Self {
		Node { uuid, hosts, next_host: Mutex::new(0), cursor: Mutex::new(None), az: Mutex::new(None),
			outbox: Mutex::new(BTreeMap::new()), applied: Mutex::new(Vec::new()), challenge: Mutex::new(Vec::new()),
			channel: Mutex::new(None),
			db: None,
			#[cfg(feature = "tls")]
			tls: None }
	}

	/// yt-light's node: its store the database `pg` serves.
	#[cfg(feature = "service")]
	pub fn with_pg(uuid: u128, hosts: Vec<String>, pg: std::sync::Arc<crate::pg::Pg>) -> Self {
		Node { db: Some(pg), ..Node::new(uuid, hosts) }
	}

	/// A node whose store is `db`: an application's database.
	pub fn with_db(uuid: u128, hosts: Vec<String>, db: std::sync::Arc<dyn crate::db::Db>) -> Self {
		Node { db: Some(db), ..Node::new(uuid, hosts) }
	}

	/// The node with its CA (crate::tls).
	#[cfg(feature = "tls")]
	pub fn with_tls(self, tls: std::sync::Arc<crate::tls::Tls>) -> Self {
		Node { tls: Some(tls), ..self }
	}

	/// The az the node above takes this one to be in - its own, shared with this node - as
	/// the last granted sign-in said: `None` before one, `Some("")` in a cluster with no
	/// azs. For showing only: nothing here reads it to decide anything.
	pub async fn told_az(&self) -> Option<String> {
		self.az.lock().await.clone()
	}

	/// No sign-in is granted now - refused, or no answer: the az told before is not this
	/// node's to show any more (review of 2026-11-13).
	pub async fn not_granted(&self) {
		*self.az.lock().await = None;
	}

	/// A local batch, committed: it goes up from `pos`, this node's next position.
	pub async fn committed(&self, pos: Pos, rows: Vec<DbInputEntity>) {
		self.outbox.lock().await.insert(pos, rows);
	}

	/// Local batches not yet passed by the cluster above.
	pub async fn waiting(&self) -> usize {
		if let Some(db) = &self.db {
			return db.waiting().await.unwrap_or(0) as usize;
		}
		self.outbox.lock().await.len()
	}

	/// What came down so far.
	pub async fn received(&self) -> Vec<RelayItem> {
		self.applied.lock().await.clone()
	}
}

pub async fn api(node: &Node) -> Option<BaseApi> {
	Some(BaseApi { version: 1, peer: Some(PeerUuid::from(node.uuid)), ..Default::default() })
}

pub async fn me(node: &Node) -> Option<PeerUuid> {
	Some(PeerUuid::from(node.uuid))
}

/// The az of a light node: none. The shared link files put it in a sign-in, a fetch and
/// a batch - a light node leaves all three empty, and the node above reads none of them
/// from it.
pub async fn my_az(_node: &Node) -> String {
	String::new()
}

/// How far this node applied what came down. One cursor, whatever `_src` names: a light
/// node has one cluster above, and the positions are that cluster's.
pub async fn cursor(node: &Node, _src: &str) -> Option<Pos> {
	if let Some(db) = &node.db {
		return db.cursor().await.ok().flatten();
	}
	*node.cursor.lock().await
}

/// What goes up after `from`: the outbox, every row of it - a light node writes
/// only its own tables. At most `max_rows` rows, whole batches; `more` when some
/// are left. The position is the last batch taken, or `from` with none.
pub async fn collect(node: &Node, _to_az: &str, flow: Flow, from: Option<Pos>, max_rows: usize)
	-> Result<(Vec<RelayItem>, Pos, bool), String> {
	if flow != Flow::Up {
		return Err(t!("light-leaf-no-down"));
	}
	if let Some(db) = &node.db {
		return db.collect(from, max_rows).await;
	}
	let ob = node.outbox.lock().await;
	let mut items = Vec::new();
	let mut rows = 0;
	let mut upto = from.unwrap_or((0, 0));
	let mut more = false;
	for (pos, data) in ob.iter().filter(|(p, _)| Some(**p) > from) {
		if !items.is_empty() && rows + data.len() > max_rows {
			more = true;
			break;
		}
		rows += data.len();
		items.push(RelayItem { term: pos.0, subterm: pos.1, data: data.clone() });
		upto = *pos;
	}
	Ok((items, upto, more))
}

/// Apply a batch that came down: what is at or below the cursor is skipped, so a
/// batch sent twice changes nothing. The cursor moves to the batch's end.
pub async fn apply(node: &Node, batch: &RelayBatch) -> Result<Option<Pos>, String> {
	if let Some(db) = &node.db {
		return db.apply(batch).await;
	}
	let mut cursor = node.cursor.lock().await;
	let at = *cursor;
	let Some(upto) = to_pos(&batch.upto) else { return Ok(at) };
	if Some(upto) <= at {
		return Ok(at);
	}
	let mut applied = node.applied.lock().await;
	for i in batch.items.iter().filter(|i| Some((i.term, i.subterm)) > at) {
		applied.push(i.clone());
	}
	*cursor = Some(upto);
	Ok(Some(upto))
}

/// A round ran: a light node keeps no record of it.
pub async fn note_round(_node: &Node, _az: &str, _flow: Flow, _idle: bool) {}

/// The cluster above holds everything through `upto`: those outbox batches can go.
pub async fn passed(node: &Node, _upper: &str, upto: Pos) {
	if let Some(db) = &node.db {
		let _ = db.passed(upto).await; // again at the next turn when it failed
		return;
	}
	node.outbox.lock().await.retain(|p, _| *p > upto);
}
