// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! The light node's version of a replaceable file: the members that apply what
//! this node applies. A light node is a leaf, one node: it has none. (ytserv has its
//! own `members.rs`: the slowest fresh member, and the fan-out to them.)
use yt_core_proto::link::RelayBatch;

use super::pos::Pos;
use super::store::{self, Node};

/// Where a fetch starts: this node's own cursor.
pub async fn start(node: &Node, src: &str) -> Option<Pos> {
	store::cursor(node, src).await
}

/// After an apply: nobody else to hand the batch to.
pub async fn after_apply(_node: &Node, _batch: &RelayBatch) {}
