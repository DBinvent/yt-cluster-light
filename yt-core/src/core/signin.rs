// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! A lower az's sign-in to the az above - or a light node's to its cluster: what bytes
//! it signs, and the call.
//!
//! The signatures themselves and the certificates stay on each side - a crypto
//! library of its own there: this file holds only the bytes that are signed, so
//! the two sides cannot disagree about them.
//!
//! Shared: the light node and ytserv compile this same file (no `crate::` paths,
//! no logging, only yt-core-proto, std, tokio and the replaceable files beside it).
use yt_core_proto::base::PeerUuid;
use yt_core_proto::link::{LinkGrant, LinkSignIn, PeerAzLevel};

use super::{calls, offer, store};

/// what the lower CA's proof of possession signs, first
pub const PROOF_DOMAIN: &[u8] = b"yt-cluster-skv-ca-proof/1";
/// what a rotated lower CA's continuity signature signs, first
pub const CONT_DOMAIN: &[u8] = b"yt-cluster-skv-ca-continuity/1";

/// Fields one after another, each preceded by its length (u32, big-endian).
pub fn framed(parts: &[&[u8]]) -> Vec<u8> {
	let mut out = Vec::with_capacity(parts.iter().map(|p| p.len() + 4).sum());
	for p in parts {
		out.extend_from_slice(&(p.len() as u32).to_be_bytes());
		out.extend_from_slice(p);
	}
	out
}

/// What `ca_proof` signs: possession of the lower CA's key, over the upper's
/// challenge for this az (the az's uuid) and the CA's fingerprint. A light node has
/// no az: its own uuid stands there, what the cluster knows it by.
pub fn proof_message(challenge: &[u8], az: &str, fp: &str) -> Vec<u8> {
	framed(&[PROOF_DOMAIN, challenge, az.as_bytes(), fp.as_bytes()])
}

/// What `ca_prev_sig` signs: the new CA certificate's DER, by the previous key.
pub fn continuity_message(new_der: &[u8]) -> Vec<u8> {
	framed(&[CONT_DOMAIN, new_der])
}

/// What `ca_roots_hash` hashes (SHA256): the upper's root certificates as held.
pub fn roots_message(roots: &[Vec<u8>]) -> Vec<u8> {
	let parts: Vec<&[u8]> = roots.iter().map(|r| r.as_slice()).collect();
	framed(&parts)
}

/// One sign-in of this az to the az above (`upper`): the offer of its CA (`offer`), the
/// call through a node there, and the answer taken - a challenge for the next one, or
/// the countersigned CA and the roots. A refusal is an answer too: `Ok` with `granted`
/// false and the reason; `Err` only when no answer came.
///
/// A light node has no az and names none for the cluster above (`store::my_az` and
/// `upper` are empty): its sign-in carries no `target_az`, and the cluster knows it by
/// its node uuid.
pub async fn sign_in(node: &store::Node, upper: &str) -> Result<LinkGrant, String> {
	let target = calls::target(node, upper).await.ok_or_else(|| no_target(upper))?;
	let mine = store::my_az(node).await;
	let mine = mine.as_str();
	let mut req = LinkSignIn {
		peer: store::me(node).await,
		// the az this node presents, which the far side matches its key against - none
		// from a light node
		target_az: if mine.is_empty() { None } else { Some(PeerUuid::from(mine.to_string())) },
		level: PeerAzLevel::Upper as i32,
		base: store::api(node).await,
		..Default::default()
	};
	offer::offer(node, mine, upper, &mut req).await;
	let grant = calls::signin(node, &target, req).await?;
	offer::take(node, mine, upper, &grant).await;
	Ok(grant)
}

/// [i18n] said under its code, in the binary's language - the light node's and ytserv's:
/// the message is in yt-core's locale files, which ytserv reads too (by symlink).
fn no_target(upper: &str) -> String {
	if upper.is_empty() {
		rust_i18n::t!("link-no-target-above").into_owned()
	} else {
		rust_i18n::t!("link-no-target-az", az = upper).into_owned()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_node_with_no_az_names_none_for_the_cluster_above() {
		assert_eq!(no_target(""), "YT-E030 no node of the cluster above to sign in through");
		assert_eq!(no_target("top"), "YT-E031 no node of az top to sign in through");
	}

	#[test]
	fn a_field_is_framed_by_its_length() {
		assert_eq!(framed(&[b"ab", b""]), vec![0, 0, 0, 2, b'a', b'b', 0, 0, 0, 0]);
		assert_ne!(framed(&[b"a", b"bc"]), framed(&[b"ab", b"c"]), "no two splits frame alike");
	}

	#[test]
	fn each_signed_message_starts_with_its_domain() {
		let p = proof_message(b"chal", "az", "SHA256:x");
		assert_eq!(&p[4..4 + PROOF_DOMAIN.len()], PROOF_DOMAIN);
		let c = continuity_message(b"der");
		assert_eq!(&c[4..4 + CONT_DOMAIN.len()], CONT_DOMAIN);
		assert_eq!(roots_message(&[b"r1".to_vec(), b"r2".to_vec()]), framed(&[b"r1", b"r2"]));
	}
}
