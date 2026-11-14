// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! Which way a table's rows go across a link, and who may dial whom.
//!
//! Shared: the lite node and ytserv compile this same file (no `crate::` paths,
//! no logging, only yt-core-proto, std, tokio and the replaceable files beside it).
use yt_core_proto::link::{AzRouting, PeerAzLevel as AzLevel};
use yt_core_proto::uuid::Uuid;

/// An az name in its one comparable spelling.
///
/// An az is named freely (`eastc`) or by a cluster's uuid, and a uuid has several
/// spellings - case, hyphens, braces. Rows naming the same az written differently
/// must still compare equal, so a uuid is kept hyphenated and lowercase, and any
/// other name as written (trimmed).
pub fn az_name(s: &str) -> String {
	let t = s.trim();
	match Uuid::parse_str(t) {
		Ok(u) => u.hyphenated().to_string(),
		Err(_) => t.to_string(),
	}
}

/// What an az may do with a table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AzRole {
	/// receives the rows at all
	pub holds: bool,
	/// may originate writes to them
	pub writes: bool,
}

impl AzRole {
	pub const FLAT: AzRole = AzRole { holds: true, writes: true };
}

/// Decide an az's role for one table. Pure, so it is testable without a cluster.
///
/// An unconfigured table, or a node with no az, is `FLAT` - every branch here
/// collapses to "yes" until somebody opts in.
///
/// `cluster` is the uuid of the node's cluster when the node is a member of this
/// one: a rule naming the cluster names every member of it, whatever az each of
/// them is in. It is a second name of a zoned node, never a zone for an unzoned
/// one - that stays `FLAT`.
pub fn role_for(routing: Option<&AzRouting>, az: Option<&str>, cluster: Option<&str>) -> AzRole {
	let r = match routing {
		None => return AzRole::FLAT,
		Some(r) => r,
	};
	if r.az.is_empty() && r.writable_az.is_empty() {
		return AzRole::FLAT;
	}
	// A node with no az of its own cannot be excluded by a rule that names
	// zones: it is not in any of them, so treating it as excluded would silently
	// stop replication the moment one table gained a routing rule.
	let az = match az {
		None => return AzRole::FLAT,
		Some(a) if a.trim().is_empty() => return AzRole::FLAT,
		Some(a) => az_name(a),
	};
	let cluster = cluster.map(az_name).filter(|c| !c.is_empty());
	let named = |list: &[String]| list.iter().map(|z| az_name(z))
		.any(|z| z == az || cluster.as_deref() == Some(z.as_str()));
	let holds = r.az.is_empty() || named(&r.az);
	let writes = holds && (r.writable_az.is_empty() || named(&r.writable_az));
	AzRole { holds, writes }
}

/// The two dial rules of a link between clusters, as a pure decision over three
/// booleans.
///
/// 1. **Downward is leader-terminated** - an upper node may dial a lower cluster
///    only at its leader (or a designated deputy), so the lower cluster's
///    membership can change without the upper cluster reconfiguring.
/// 2. **Sideways is leader-to-leader only** - a member never dials sideways.
///
/// `DEFAULT` is always allowed: it is an ordinary peer of this cluster, and this
/// function must never be able to refuse a connection in a flat cluster.
///
/// [i18n] The reason is said in the binary's language (`link-rule-*`, yt-core's locale
/// files - ytserv reads them too): the tail of the message that refuses the dial, so it
/// carries no code of its own.
pub fn link_rule(relation: AzLevel, i_lead: bool, they_lead: Leads) -> Result<(), String> {
	// `Unknown` is permitted wherever `Yes` is, for the cases that need the far
	// side to be leading. Leadership over there is learned BY pinging, so treating
	// "not yet known" as "not leading" makes the gate refuse the ping that would
	// resolve it and the link is refused forever. It costs at most one ping to a
	// node that turns out to be an ordinary member, after which it is refused.
	let far_ok = matches!(they_lead, Leads::Yes | Leads::Unknown);
	match relation {
		AzLevel::Default => Ok(()),

		// **Never dialled.** A subordinate cluster connects UP to us; we hold no
		// address for it, only a key to verify its sign-in - so there is nothing
		// here to dial even if the rule allowed it. Refusing explicitly rather than
		// relying on the absent address means a stray address in the catalogue
		// cannot quietly turn into a downward mesh.
		//
		// This also makes adding a child cost the parent one key instead of a
		// roster that goes stale whenever the child's membership changes.
		AzLevel::Lower => Err(rust_i18n::t!("link-rule-lower").into_owned()),

		// Only this cluster's leader dials upward, and NOT at the upper cluster's
		// leader. The upper leader is the busiest node in the cluster above; a
		// hierarchy exists to keep load off it, so a boundary that terminated there
		// would spend the saving it was built to make. `Leads::Yes` on the target is
		// therefore a REFUSAL here - the inverse of every other case.
		AzLevel::Upper => match (i_lead, they_lead) {
			(false, _) => Err(rust_i18n::t!("link-rule-upper-member").into_owned()),
			(true, Leads::Yes) => Err(rust_i18n::t!("link-rule-upper-leader").into_owned()),
			(true, _) => Ok(()),
		},

		AzLevel::Neighbor => if i_lead && far_ok { Ok(()) } else {
			Err(rust_i18n::t!("link-rule-neighbor").into_owned())
		},
	}
}

/// Whether the far end of a link is leading its own cluster. Three states, because
/// "not yet known" and "known not to be" must be treated differently - see
/// `link_rule`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leads {
	Yes,
	No,
	/// never successfully pinged, so leadership is not established either way
	Unknown,
}

/// Which way rows go across a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
	/// from the cluster above to one below
	Down,
	/// from a cluster below to the one above
	Up,
}

/// Does a row of this table, committed in this cluster, cross to `to_az` going
/// `flow`? Only a ROUTED table crosses - an unrouted one is this cluster's own, the
/// config table first of all - and only to an az that holds it, in a direction its
/// `no_down` / `no_up` allows.
pub fn crosses(routing: Option<&AzRouting>, to_az: &str, flow: Flow) -> bool {
	let Some(r) = routing else { return false };
	let to = az_name(to_az);
	let held = r.az.is_empty() || r.az.iter().any(|z| az_name(z) == to);
	held && match flow {
		Flow::Down => !r.no_down,
		Flow::Up => !r.no_up,
	}
}

/// May a relayed row of this table, written in `from_az`, be applied here? The
/// table must be held here and writable there - checked on arrival too, so a
/// source cannot put rows into a table it may not write.
pub fn accepts(routing: Option<&AzRouting>, my_az: &str, from_az: &str) -> bool {
	let Some(r) = routing else { return false };
	role_for(Some(r), Some(my_az), None).holds && role_for(Some(r), Some(from_az), None).writes
}

/// Does a row of this table cross between a node and a lite node linked to it, going
/// `flow`?
///
/// A lite node has no az of its own: it is taken for a node of the az of the node it
/// is linked to - `az`, in the cluster `cluster` - and cannot say otherwise. So the
/// answer is that node's own role (`role_for`), across a link:
///
/// - only a ROUTED table crosses, as across any link - an unrouted one is the
///   cluster's own, the config table first of all;
/// - down, a table that node holds, unless `no_down`;
/// - up - a row written on the lite node - a table that node may write, unless
///   `no_up`.
///
/// In a cluster with no azs (`az` none) a routing rule excludes nobody, here either:
/// every routed table crosses, both ways, but for its `no_down` / `no_up`.
pub fn crosses_lite(routing: Option<&AzRouting>, az: Option<&str>, cluster: Option<&str>, flow: Flow) -> bool {
	let Some(r) = routing else { return false };
	let role = role_for(Some(r), az, cluster);
	match flow {
		Flow::Down => role.holds && !r.no_down,
		Flow::Up => role.writes && !r.no_up,
	}
}

/// May a row a lite node wrote be applied here, on a member of the cluster it is
/// linked to? The row counts as written in `from_az`, the az of the node the lite
/// node delivered to: the table must go up at all (`crosses_lite`, there), and be
/// held here - `my_az`, in the same cluster.
pub fn accepts_lite(routing: Option<&AzRouting>, my_az: Option<&str>, from_az: Option<&str>, cluster: Option<&str>) -> bool {
	crosses_lite(routing, from_az, cluster, Flow::Up) && role_for(routing, my_az, cluster).holds
}

#[cfg(test)]
mod tests {
	use super::*;

	fn routing(az: &[&str], write: &[&str], no_up: bool, no_down: bool) -> AzRouting {
		AzRouting {
			az: az.iter().map(|s| s.to_string()).collect(),
			writable_az: write.iter().map(|s| s.to_string()).collect(),
			no_up,
			no_down,
		}
	}

	/// A write-by-top table goes down to every az that holds it, and nowhere it is
	/// not held; an unrouted table - the config table - never crosses.
	#[test]
	fn a_routed_table_crosses_down_to_an_az_that_holds_it() {
		let down = routing(&["top", "eastc", "westc"], &["top"], false, false);
		assert!(crosses(Some(&down), "eastc", Flow::Down));
		assert!(crosses(Some(&down), "westc", Flow::Down));
		assert!(!crosses(Some(&down), "northc", Flow::Down), "not held there");
		assert!(!crosses(None, "eastc", Flow::Down), "unrouted: this cluster's own");
		let local = routing(&[], &[], true, true);
		assert!(!crosses(Some(&local), "eastc", Flow::Down), "no_down");
		assert!(!crosses(Some(&local), "top", Flow::Up), "no_up");
		let up = routing(&["top", "eastc"], &["eastc"], false, true);
		assert!(crosses(Some(&up), "top", Flow::Up));
		assert!(!crosses(Some(&up), "westc", Flow::Down), "no_down, and not held");
	}

	/// The receiver applies only rows of a table it holds and the source az may
	/// write: a lower cluster cannot push rows into a write-by-top table.
	#[test]
	fn a_relayed_row_is_accepted_only_from_an_az_that_may_write_it() {
		let down = routing(&["top", "eastc", "westc"], &["top"], false, false);
		assert!(accepts(Some(&down), "eastc", "top"));
		assert!(!accepts(Some(&down), "top", "eastc"), "east may not write it");
		let up = routing(&["top", "eastc"], &["eastc"], false, true);
		assert!(accepts(Some(&up), "top", "eastc"));
		assert!(!accepts(Some(&up), "westc", "eastc"), "west does not hold it");
		assert!(!accepts(None, "eastc", "top"), "unrouted");
	}

	/// A lite node has no az: what crosses to it and from it is decided by the az of the
	/// node it is linked to - that node's role - and only for a routed table.
	#[test]
	fn a_lite_node_takes_what_the_az_of_its_node_holds() {
		let c = Some("4b2d1c3e-0000-4000-8000-00000000000a");
		let both = routing(&["top", "eastc"], &[], false, false);
		assert!(crosses_lite(Some(&both), Some("top"), c, Flow::Down));
		assert!(crosses_lite(Some(&both), Some("top"), c, Flow::Up));
		assert!(!crosses_lite(Some(&both), Some("westc"), c, Flow::Down), "its node's az does not hold it");
		assert!(!crosses_lite(Some(&both), Some("westc"), c, Flow::Up));
		assert!(!crosses_lite(None, Some("top"), c, Flow::Down), "unrouted: the cluster's own");
		assert!(!crosses_lite(None, Some("top"), c, Flow::Up));
		// held and read-only in its node's az: down, never up
		let read = routing(&["top", "eastc"], &["top"], false, false);
		assert!(crosses_lite(Some(&read), Some("eastc"), c, Flow::Down));
		assert!(!crosses_lite(Some(&read), Some("eastc"), c, Flow::Up), "its node's az may not write it");
		assert!(crosses_lite(Some(&read), Some("top"), c, Flow::Up));
		// the link's own flags
		let no_down = routing(&["top"], &[], false, true);
		assert!(!crosses_lite(Some(&no_down), Some("top"), c, Flow::Down));
		assert!(crosses_lite(Some(&no_down), Some("top"), c, Flow::Up));
		let no_up = routing(&["top"], &[], true, false);
		assert!(crosses_lite(Some(&no_up), Some("top"), c, Flow::Down));
		assert!(!crosses_lite(Some(&no_up), Some("top"), c, Flow::Up));
		// a rule naming the cluster names its node, whatever zone that node is in
		let by_cluster = routing(&["4B2D1C3E-0000-4000-8000-00000000000A"], &[], false, false);
		assert!(crosses_lite(Some(&by_cluster), Some("east"), c, Flow::Down));
		assert!(!crosses_lite(Some(&by_cluster), Some("east"), None, Flow::Down));
	}

	/// In a cluster with no azs a routing rule excludes nobody - a lite node neither:
	/// every routed table crosses, an unrouted one still does not.
	#[test]
	fn a_lite_node_of_a_cluster_with_no_azs_takes_every_routed_table() {
		let any = routing(&["whatever"], &["whatever"], false, false);
		for az in [None, Some(""), Some("  ")] {
			assert!(crosses_lite(Some(&any), az, None, Flow::Down));
			assert!(crosses_lite(Some(&any), az, None, Flow::Up));
			assert!(!crosses_lite(None, az, None, Flow::Down));
		}
		assert!(!crosses_lite(Some(&routing(&["x"], &[], false, true)), None, None, Flow::Down), "no_down still holds");
	}

	/// A lite node's row on a member: written in the az of the node it delivered to,
	/// applied where the table is held.
	#[test]
	fn a_lite_node_s_row_is_applied_where_the_table_is_held() {
		let c = Some("4b2d1c3e-0000-4000-8000-00000000000a");
		let r = routing(&["east", "west"], &["east"], false, false);
		assert!(accepts_lite(Some(&r), Some("west"), Some("east"), c), "written in east, held in west");
		assert!(!accepts_lite(Some(&r), Some("east"), Some("west"), c), "west may not write it");
		assert!(!accepts_lite(Some(&r), Some("north"), Some("east"), c), "not held in north");
		assert!(!accepts_lite(None, Some("east"), Some("east"), c), "unrouted");
		assert!(!accepts_lite(Some(&routing(&["east"], &[], true, false)), Some("east"), Some("east"), c), "no_up");
		assert!(accepts_lite(Some(&routing(&["x"], &[], false, false)), None, None, None), "no azs: every routed table");
		// named by the cluster: every member of it holds the table, whatever its zone
		let by_cluster = routing(&["4b2d1c3e-0000-4000-8000-00000000000a"], &[], false, false);
		assert!(accepts_lite(Some(&by_cluster), Some("west"), Some("east"), c));
	}

	#[test]
	fn an_az_named_by_a_uuid_has_one_spelling() {
		let u = "4B2D1C3E-0000-4000-8000-00000000000A";
		assert_eq!(az_name(u), az_name(&format!("{{{}}}", u.to_lowercase())));
		assert_eq!(az_name(" eastc "), "eastc");
	}

	#[test]
	fn an_unrouted_table_or_an_unzoned_node_is_flat() {
		assert_eq!(role_for(None, Some("eastc"), None), AzRole::FLAT);
		let r = routing(&["eastc"], &["eastc"], false, false);
		assert_eq!(role_for(Some(&r), None, None), AzRole::FLAT);
		assert_eq!(role_for(Some(&r), Some("westc"), None), AzRole { holds: false, writes: false });
		assert_eq!(role_for(Some(&r), Some("westc"), Some("eastc")), AzRole::FLAT, "named by its cluster");
	}

	#[test]
	fn only_a_lower_clusters_leader_dials_up_and_never_at_the_upper_leader() {
		assert!(link_rule(AzLevel::Default, false, Leads::No).is_ok());
		assert!(link_rule(AzLevel::Lower, true, Leads::Yes).is_err());
		assert!(link_rule(AzLevel::Upper, false, Leads::No).is_err());
		assert!(link_rule(AzLevel::Upper, true, Leads::Yes).is_err());
		assert!(link_rule(AzLevel::Upper, true, Leads::No).is_ok());
		assert!(link_rule(AzLevel::Neighbor, true, Leads::Unknown).is_ok());
		assert!(link_rule(AzLevel::Neighbor, false, Leads::Yes).is_err());
		// the reason, as the refusal's tail: said, and under no code of its own
		let why = link_rule(AzLevel::Lower, true, Leads::Yes).unwrap_err();
		assert!(why.starts_with("a LOWER cluster is never dialled"), "{}", why);
		assert!(link_rule(AzLevel::Upper, true, Leads::Yes).unwrap_err().starts_with("rule 1: an upward link must avoid"));
		assert!(link_rule(AzLevel::Upper, false, Leads::No).unwrap_err().starts_with("rule 1: only this cluster's leader"));
		assert!(link_rule(AzLevel::Neighbor, false, Leads::Yes).unwrap_err().starts_with("rule 2:"));
	}
}
