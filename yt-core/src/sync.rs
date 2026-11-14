// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! Which of a light node's tables sync, and which way: as on a cluster node, a table syncs
//! when its config table holds an A row for it (`schema.table`) - `yt_setup` writes one with
//! the table's trigger - and its rows go up unless the row's value says `no_up`, down unless
//! `no_down`. A table with no A row syncs neither way: what comes down for it is not applied
//! and what is written to it is not sent, each counted. The value is the cluster's encoding
//! (`ytctl --encode-table-cfg`); empty, or a value this node cannot read, is both ways.
use std::collections::HashMap;

use base64::Engine;
use prost::Message;
use yt_core_proto::link::TableLink;

/// The ways a table syncs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sync {
	/// its local writes go up
	pub up: bool,
	/// what comes down for it is applied
	pub down: bool,
}

/// The ways of an A row's value.
pub fn of(value: &str) -> Sync {
	let both = Sync { up: true, down: true };
	let v = value.trim();
	if v.is_empty() {
		return both;
	}
	let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(v) else { return both };
	match TableLink::decode(bytes.as_slice()) {
		Ok(TableLink { az_routing: Some(r) }) => Sync { up: !r.no_up, down: !r.no_down },
		_ => both,
	}
}

/// The value is one this node reads: empty (both ways), or the cluster's encoding. A value
/// given by hand (yt-sqlite's `yt_sync`) is checked with it; one from the cluster is taken as
/// `of` takes it.
pub fn readable(value: &str) -> bool {
	let v = value.trim();
	v.is_empty() || base64::engine::general_purpose::STANDARD.decode(v).is_ok_and(|b| TableLink::decode(b.as_slice()).is_ok())
}

/// The A rows of a config table, `(name, value)`, as the tables they sync: by `schema.table`.
pub fn tables<'a>(rows: impl Iterator<Item = (&'a str, &'a str)>) -> HashMap<String, Sync> {
	rows.map(|(n, v)| (n.to_string(), of(v))).collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use yt_core_proto::link::AzRouting;

	fn encoded(no_up: bool, no_down: bool) -> String {
		let t = TableLink { az_routing: Some(AzRouting { az: vec!["top".into()], no_up, no_down, ..Default::default() }) };
		base64::engine::general_purpose::STANDARD.encode(t.encode_to_vec())
	}

	#[test]
	fn an_a_row_syncs_its_table_the_ways_its_value_says() {
		assert_eq!(of(""), Sync { up: true, down: true }, "yt_setup's row: both ways");
		assert_eq!(of(&encoded(false, false)), Sync { up: true, down: true });
		assert_eq!(of(&encoded(true, false)), Sync { up: false, down: true });
		assert_eq!(of(&encoded(false, true)), Sync { up: true, down: false });
		assert_eq!(of("not base64 at all!"), Sync { up: true, down: true }, "a value it cannot read: both ways");
		// the cluster's own encoding: the az rule `az=top` alone, as ytctl --encode-table-cfg writes it
		// (field 20, AzRouting { az: ["top"] }), and other fields of the cluster's config beside it
		let mut cluster = vec![0x08, 0x01]; // field 1, a varint: a field this node does not read
		cluster.extend(TableLink { az_routing: Some(AzRouting { az: vec!["top".into()], no_down: true, ..Default::default() }) }.encode_to_vec());
		assert_eq!(of(&base64::engine::general_purpose::STANDARD.encode(cluster)), Sync { up: true, down: false });
		let t = tables([("s.a", ""), ("s.b", encoded(true, true).as_str())].into_iter());
		assert_eq!(t["s.b"], Sync { up: false, down: false });
		assert!(!t.contains_key("s.c"), "no A row: not synced");
	}

	#[test]
	fn a_value_given_by_hand_is_read_or_refused() {
		assert!(readable(""));
		assert!(readable("  "));
		assert!(readable(&encoded(true, false)));
		assert!(!readable("not base64 at all!"));
		// base64, but no TableLink: a field of a wire type that does not end
		assert!(!readable(&base64::engine::general_purpose::STANDARD.encode([0xa2, 0x01, 0x05])));
	}
}
