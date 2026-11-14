// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! A position in a source cluster's history.
//!
//! Shared: the lite node and ytserv compile this same file (the link's rules: no
//! `crate::` paths, no logging, only yt-core-proto, std, tokio and the replaceable
//! files beside it).
use yt_core_proto::Term;
use yt_core_proto::link::RelayPos;

/// A position in a source cluster's history: (term, subterm), everything up to and
/// including it examined.
pub type Pos = (Term, u32);

pub fn to_pos(p: &Option<RelayPos>) -> Option<Pos> {
	p.as_ref().map(|p| (p.term, p.subterm))
}

pub fn wire(p: Pos) -> RelayPos {
	RelayPos { term: p.0, subterm: p.1 }
}

/// `term.subterm`, as a cursor row holds it.
pub fn parse_pos(s: &str) -> Option<Pos> {
	let (t, s) = s.trim().split_once('.')?;
	Some((t.parse().ok()?, s.parse().ok()?))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_position_round_trips_and_orders_by_term_first() {
		assert_eq!(parse_pos("3.1729"), Some((3, 1729)));
		assert_eq!(parse_pos(" 12.0 "), Some((12, 0)));
		assert_eq!(parse_pos("x"), None);
		assert!(Some((2u64, 900u32)) < Some((3, 1)));
		assert!(None < Some((1u64, 0u32)), "no cursor: from the start");
	}

	#[test]
	fn a_position_goes_on_the_wire_and_back() {
		assert_eq!(to_pos(&Some(wire((7, 9)))), Some((7, 9)));
		assert_eq!(to_pos(&None), None);
	}
}
