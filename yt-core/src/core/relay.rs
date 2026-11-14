// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! The feed across a link, driven by the lower side: `down` fetches what crossed
//! to this az from the az above and applies it; `up` delivers what this az
//! committed that crosses up, from where the az above says it is.
//!
//! A lite node drives it the same way and has no az: `store::my_az` answers with
//! none and `upper` is empty (`store::ABOVE`) - the node it calls takes it for a node
//! of its own az, and reads no az from it.
//!
//! Every receiver skips what is at or below its own cursor, so a batch may be sent
//! again at no cost but the transfer: a batch applied twice changes nothing.
//!
//! Shared: the lite node and ytserv compile this same file. It reaches its node
//! only through the replaceable files beside it - `store` (the node, its rows and
//! its cursor), `members` (who else applies what this node applies), `calls` (the
//! node above) - and logs nothing: each turn returns an `Outcome` for the caller.
use std::time::Duration;
use yt_core_proto::link::{RelayBatch, RelayFetch};

use super::pos::{to_pos, wire};
use super::route::{az_name, Flow};
use super::{calls, members, store};

/// rows per batch
pub const BATCH_ROWS: usize = 500;
/// subterms one collect examines at most, crossing or not
pub const MAX_EXAMINE: usize = 5000;
/// batches one turn moves per link
pub const MAX_ROUNDS: usize = 20;
/// a member's position is trusted this long without hearing from it again
pub const MEMBER_FRESH: Duration = Duration::from_secs(60);
/// a relay call carries applies on the far side: generous
pub const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// What one turn of `down` or `up` did, for the caller to log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
	/// no node of the az above to relay through
	NoTarget,
	/// nothing new to carry
	Idle,
	/// carried what there was
	Moved,
	/// the az above did not say where it is (up): the target is forgotten
	AskFailed(String),
	/// a fetch failed (down): the target is forgotten
	FetchFailed(String),
	/// a fetched batch was not applied here (down): again next turn
	ApplyFailed(String),
	/// this node cannot collect what the az above needs (up)
	CollectFailed(String),
	/// the az above refused a batch (up)
	Refused(String),
	/// a delivery failed (up): the target is forgotten
	DeliverFailed(String),
	/// the az above took a batch but did not move (up): a member behind, again next turn
	Stalled,
}

/// Down, for one az above: fetch what crossed to this az, apply it here, hand it to
/// the members - from the slowest position among this node and its members.
pub async fn down(node: &store::Node, upper: &str) -> Outcome {
	let Some(target) = calls::target(node, upper).await else { return Outcome::NoTarget };
	let src = az_name(upper);
	let me = store::my_az(node).await;
	let mut from = members::start(node, &src).await;
	let mut moved = false;
	for _ in 0..MAX_ROUNDS {
		let req = RelayFetch { base: store::api(node).await, az: me.clone(), from: from.map(wire),
			max_items: BATCH_ROWS as u32 };
		let batch = match calls::fetch(node, &target, req).await {
			Ok(b) => b,
			Err(e) => {
				calls::forget(node, upper).await;
				return Outcome::FetchFailed(e);
			}
		};
		let upto = to_pos(&batch.upto);
		if upto <= from {
			store::note_round(node, upper, Flow::Down, true).await;
			return if moved { Outcome::Moved } else { Outcome::Idle }; // nothing new over there
		}
		if let Err(e) = store::apply(node, &batch).await {
			return Outcome::ApplyFailed(e);
		}
		members::after_apply(node, &batch).await;
		store::note_round(node, upper, Flow::Down, batch.items.is_empty() && !batch.more).await;
		moved = true;
		from = upto;
		if !batch.more {
			break;
		}
	}
	Outcome::Moved
}

/// Up, for one az above: deliver this az's crossing rows to the node it links to
/// there, from where the az above is - for a cluster, its slowest member.
pub async fn up(node: &store::Node, upper: &str) -> Outcome {
	let Some(target) = calls::target(node, upper).await else { return Outcome::NoTarget };
	let me = store::my_az(node).await;
	let probe = RelayBatch { base: store::api(node).await, source_az: me.clone(), ..Default::default() };
	let mut from = match calls::deliver(node, &target, probe).await {
		Ok(ack) => to_pos(&ack.at),
		Err(e) => {
			calls::forget(node, upper).await;
			return Outcome::AskFailed(e);
		}
	};
	if let Some(p) = from {
		store::passed(node, upper, p).await;
	}
	let mut moved = false;
	for _ in 0..MAX_ROUNDS {
		let (items, upto, more) = match store::collect(node, upper, Flow::Up, from, BATCH_ROWS).await {
			Ok(r) => r,
			// this node cannot serve what the az above still needs; a later leader
			// with the history can
			Err(e) => return Outcome::CollectFailed(e),
		};
		if Some(upto) <= from {
			store::note_round(node, upper, Flow::Up, true).await;
			return if moved { Outcome::Moved } else { Outcome::Idle };
		}
		let carried = !items.is_empty() || more;
		// `lite` is not a sender's to set: the node a lite node delivers to names it
		let batch = RelayBatch { base: store::api(node).await, source_az: me.clone(), items,
			upto: Some(wire(upto)), more, ..Default::default() };
		let at = match calls::deliver(node, &target, batch).await {
			Ok(ack) if ack.error.is_empty() => to_pos(&ack.at),
			Ok(ack) => return Outcome::Refused(ack.error),
			Err(e) => {
				calls::forget(node, upper).await;
				return Outcome::DeliverFailed(e);
			}
		};
		if at <= from {
			store::note_round(node, upper, Flow::Up, false).await;
			return Outcome::Stalled; // no progress there: a member behind, retried next turn
		}
		store::note_round(node, upper, Flow::Up, !carried).await;
		if let Some(p) = at {
			store::passed(node, upper, p).await;
		}
		moved = true;
		from = at;
		if !more {
			break;
		}
	}
	Outcome::Moved
}
