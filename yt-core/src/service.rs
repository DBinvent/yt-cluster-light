// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! yt-light's unix socket: what the PostgreSQL extension calls (db.proto's LocalGrpc),
//! at `<unix_socket_directories>/.s.YT.<port>` - one service per instance.
//!
//! A row (`modify`) waits in memory under its transaction; the transaction's prepare
//! (the extension's PRE_COMMIT) puts its rows in the outbox at their position, its
//! commit lets them go up once the database says it committed, its rollback drops them
//! (`crate::pg`). There is no party to ask: the local commit is this node's commit, and
//! the rows go up by the relay. A transaction that never prepares here (auto_complete
//! off, no yt_complete) is taken as its database decided it (`sweep`).
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use slog::{debug, error, info, warn, Logger};
use tokio::sync::{oneshot, Mutex};
use tonic::{Request, Response, Status};
use yt_core_proto::base::{ClusterCommitType, ClusterComplete};
use yt_core_proto::db::local_grpc_server::{LocalGrpc, LocalGrpcServer};
use yt_core_proto::db::{CfgVo, CompleteReply, DbInputEntity, InfoRequest, InfoResponse, LlmReply, LlmRequest, MergeRequest, ModifyReply};

use crate::core::store::Node;
use crate::pg::{full_version, one_database, Pg};

#[derive(Clone)]
pub struct Light {
	pub node: Arc<Node>,
	pub pg: Arc<Pg>,
	/// the rows of the transactions still open, by transaction id, and since when
	pending: Arc<Mutex<HashMap<u64, (Instant, Vec<DbInputEntity>)>>>,
	pub started: Instant,
	pub log: Logger,
}

impl Light {
	pub fn new(node: Arc<Node>, pg: Arc<Pg>, log: Logger) -> Self {
		Light { node, pg, pending: Arc::new(Mutex::new(HashMap::new())), started: Instant::now(), log }
	}

	/// Serve the socket `path` until `stop` fires; an error that ends the server is sent
	/// on `ended`.
	pub async fn run(self, path: &str, stop: oneshot::Receiver<()>, ended: tokio::sync::mpsc::UnboundedSender<String>) -> Result<(), String> {
		let uds = tokio::net::UnixListener::bind(path).map_err(|e| format!("{}: {}", path, e))?;
		// connect() on a unix socket needs WRITE permission on it: the postgres backend is
		// another user. PostgreSQL's own socket is 0777 by default too.
		{
			use std::os::unix::fs::PermissionsExt;
			if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o777)) {
				warn!(self.log, "{}", t!("light-chmod", path = path, error = e));
			}
		}
		let incoming = tokio_stream::wrappers::UnixListenerStream::new(uds);
		let log = self.log.clone();
		tokio::spawn(async move {
			if let Err(e) = tonic::transport::Server::builder()
				.add_service(LocalGrpcServer::new(self))
				.serve_with_incoming_shutdown(incoming, async { let _ = stop.await; })
				.await {
				error!(log, "{}", t!("light-socket-stopped", error = e));
				let _ = ended.send(e.to_string());
			}
		});
		Ok(())
	}

	/// The open transactions older than `age`: the database decided them without a word
	/// here - committed ones are prepared and sent as any other, aborted ones and those
	/// the database no longer knows dropped. How many went each way.
	pub async fn sweep(&self, age: Duration) -> Result<(usize, usize), String> {
		let old: Vec<u64> = self.pending.lock().await.iter().filter(|(_, (t, _))| t.elapsed() > age).map(|(x, _)| *x).collect();
		let (mut sent, mut dropped) = (0, 0);
		for xid in old {
			let st = self.pg.xact_status(xid).await?;
			if st.as_deref() == Some("in progress") {
				continue;
			}
			let Some((at, rows)) = self.pending.lock().await.remove(&xid) else { continue };
			if st.as_deref() == Some("committed") {
				// a prepare that failed puts the rows back, for the next sweep: the transaction
				// is committed and nothing else would send it (review of 2026-11-13)
				if let Err(e) = self.pg.prepare(xid, rows.clone()).await {
					self.pending.lock().await.insert(xid, (at, rows));
					return Err(e);
				}
				self.pg.commit(xid).await?;
				sent += 1;
			} else {
				dropped += 1;
			}
		}
		Ok((sent, dropped))
	}

	fn rows(&self, module: char, rows: &[(&str, String)]) -> Response<InfoResponse> {
		Response::new(InfoResponse { data: rows.iter().map(|(n, v)| CfgVo::new(n, v.clone(), module)).collect() })
	}
}

#[tonic::async_trait]
impl LocalGrpc for Light {
	async fn modify(&self, request: Request<DbInputEntity>) -> Result<Response<ModifyReply>, Status> {
		let mut row = request.into_inner();
		let Some(t) = row.table.as_mut() else { return Err(Status::invalid_argument(t!("light-row-no-table"))) };
		// one database: a write from any other one is refused, never skipped
		if t.db != self.pg.db {
			let why = one_database(&self.pg.db);
			warn!(self.log, "{}", t!("light-modify-refused", why = why, table = format!("{}.{}.{}", t.db, t.schema, t.table)));
			return Err(Status::failed_precondition(why));
		}
		// a table that does not sync up - no A row, or no_up in it - keeps its write here: not
		// sent, counted (crate::sync)
		let key = format!("{}.{}", t.schema, t.table);
		match self.pg.syncs().await {
			Ok(s) if s.get(&key).is_some_and(|s| s.up) => {}
			Ok(_) => {
				self.pg.not_synced.fetch_add(1, Ordering::Relaxed);
				return Ok(Response::new(ModifyReply::ok0()));
			}
			Err(e) => return Err(Status::unavailable(e)),
		}
		// the cluster above knows this database by its own name for it
		t.db = self.pg.cluster_db.clone();
		debug!(self.log, "modify tx={} action={} table={}", row.tx, row.action, t.as_key());
		self.pending.lock().await.entry(row.tx).or_insert_with(|| (Instant::now(), Vec::new())).1.push(row);
		Ok(Response::new(ModifyReply::ok0()))
	}

	async fn complete(&self, request: Request<ClusterComplete>) -> Result<Response<CompleteReply>, Status> {
		let c = request.into_inner();
		// the rows taken out first: the lock is not held over the database's answer
		let rows = self.pending.lock().await.remove(&c.txid).map(|(_, r)| r);
		let done = match ClusterCommitType::try_from(c.commit) {
			Ok(ClusterCommitType::Prepare) => match rows {
				Some(rows) => self.pg.prepare(c.txid, rows).await.map(|_| ()),
				None => Ok(()),
			},
			Ok(ClusterCommitType::Commit) => {
				// a commit with no prepare before it (yt_complete inside the transaction):
				// prepared now, and sent once the database committed it
				if let Some(rows) = rows {
					if let Err(e) = self.pg.prepare(c.txid, rows).await {
						return Err(Status::unavailable(e));
					}
				}
				self.pg.commit(c.txid).await.map(|p| {
					if let Some(p) = p {
						debug!(self.log, "tx={} committed at {}.{}", c.txid, p.0, p.1);
					}
				})
			}
			Ok(ClusterCommitType::Rollback) => self.pg.rollback(c.txid).await,
			Err(_) => Err(t!("light-unknown-completion", value = c.commit)),
		};
		match done {
			Ok(()) => Ok(Response::new(CompleteReply { result: 0 })),
			Err(e) => {
				let e = t!("light-complete-failed", tx = c.txid, error = e);
				error!(self.log, "{}", e);
				Err(Status::unavailable(e))
			}
		}
	}

	async fn info(&self, request: Request<InfoRequest>) -> Result<Response<InfoResponse>, Status> {
		let raw = request.into_inner().input;
		let raw = raw.trim();
		let cfg = format!("{}.{}", self.pg.schema, self.pg.config_table);
		match raw {
			// yt_setup takes its config table from its database's `U` line; with no `R`
			// line, yt_setup in any other database stops at once (16.5: refuse early)
			"R" | "r" => Ok(self.rows('R', &[])),
			// the databases this service replicates: one
			"U" | "u" => Ok(self.rows('U', &[(self.pg.db.as_str(), format!("participant cfg={}", cfg))])),
			"U+" | "u+" => Ok(self.rows('U', &[("announce", "ok".to_string())])),
			// what a DDL trigger tells the node: nothing to do here - run yt_setup again
			// after a schema change
			_ if raw.starts_with("W-:") || raw.starts_with("S-:") => Ok(self.rows('I', &[])),
			// yt_info(): what this node is
			"" | "I" | "i" => {
				let uuid = yt_core_proto::uuid::Uuid::from_u128(self.node.uuid).to_string();
				let mut rows = vec![
					("version", env!("CARGO_PKG_VERSION").to_string()),
					("node", uuid),
					("database", self.pg.db.clone()),
					("cluster_database", self.pg.cluster_db.clone()),
					("hosts_above", self.node.hosts.join(",")),
					// a light node has no az: this is the az of the node above it signed in
					// through, as that node said - shown, never set here
					("az", match self.node.told_az().await {
						Some(a) if a.is_empty() => t!("light-info-az-none"),
						Some(a) => t!("light-info-az-shared", az = a),
						None => t!("light-info-az-untold"),
					}),
					("outbox", self.node.waiting().await.to_string()),
					("open_transactions", self.pending.lock().await.len().to_string()),
					("unknown_transactions", self.pg.unknown.load(Ordering::Relaxed).to_string()),
					("rows_missed_here", self.pg.missed.load(Ordering::Relaxed).to_string()),
					("rows_not_held_here", self.pg.not_held.load(Ordering::Relaxed).to_string()),
					("rows_not_synced_here", self.pg.not_synced.load(Ordering::Relaxed).to_string()),
					("apply_error", self.pg.apply_error.lock().map(|e| e.clone()).unwrap_or_default()),
					("uptime_sec", self.started.elapsed().as_secs().to_string()),
				];
				if let Some(t) = &self.node.tls {
					rows.push(("ca_fingerprint", t.fingerprint().to_string()));
					rows.push(("link_tls", t.status()));
				}
				rows.push(("light", t!("light-info-light")));
				Ok(self.rows('I', &rows))
			}
			_ => {
				let what = raw.split(':').next().unwrap_or(raw);
				info!(self.log, "{}", t!("light-info-refused", what = raw));
				Err(Status::failed_precondition(full_version(&format!("yt_info('{}')", what))))
			}
		}
	}

	async fn merge(&self, _request: Request<MergeRequest>) -> Result<Response<InfoResponse>, Status> {
		Err(Status::failed_precondition(full_version("yt_merge")))
	}

	async fn llm(&self, _request: Request<LlmRequest>) -> Result<Response<LlmReply>, Status> {
		Err(Status::failed_precondition(full_version("yt_llm_query")))
	}
}
