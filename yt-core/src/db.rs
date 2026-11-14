// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! A light node's store: the database its rows live in - what it committed and has not seen
//! passed by the cluster above, and what came down with the cursor of how far. yt-light's is
//! the PostgreSQL database it serves (crate::pg); an application's is its own (crate::turso,
//! feature `turso`). A node with none keeps both in memory (core/store.rs).
use yt_core_proto::link::{RelayBatch, RelayItem};

use crate::core::pos::Pos;

#[tonic::async_trait]
pub trait Db: Send + Sync {
	/// Local batches not yet passed by the cluster above.
	async fn waiting(&self) -> Result<i64, String>;
	/// How far this node applied what came down.
	async fn cursor(&self) -> Result<Option<Pos>, String>;
	/// What goes up after `from`: whole batches, at most `max_rows` rows of them - one more
	/// when the first alone has more; the position of the last one taken (`from` with
	/// none); and whether some are left.
	async fn collect(&self, from: Option<Pos>, max_rows: usize) -> Result<(Vec<RelayItem>, Pos, bool), String>;
	/// A batch that came down, applied with the cursor in one transaction: what is at or
	/// below the cursor is skipped, so a batch sent twice changes nothing. A row of a table
	/// this database does not hold is not this node's: skipped and counted.
	async fn apply(&self, batch: &RelayBatch) -> Result<Option<Pos>, String>;
	/// The cluster above holds everything through `upto`.
	async fn passed(&self, upto: Pos) -> Result<(), String>;
}
