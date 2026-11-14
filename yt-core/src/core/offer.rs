// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! The light node's version of a replaceable file: what its sign-in offers, and
//! what it takes from the answer. (ytserv has its own `offer.rs`: the secret store,
//! rotation, continuity, the meter and splits.)
//!
//! yt-light offers its one CA (crate::tls): the certificate as held, its key as the one
//! live, what it holds of the upper's roots, and the key's proof over the challenge the
//! last answer handed (`signin::proof_message`); from the answer it keeps the next
//! challenge, and takes the countersigned CA and the roots. A light node with no CA - in
//! an application, for now - offers nothing and keeps only the challenge.
use yt_core_proto::link::{LinkGrant, LinkSignIn};

use super::store::Node;

#[cfg_attr(not(feature = "tls"), allow(unused_variables))]
pub async fn offer(node: &Node, _mine: &str, _upper: &str, req: &mut LinkSignIn) {
	#[cfg(feature = "tls")]
	if let Some(tls) = &node.tls {
		// a light node has no az: its own uuid binds the proof - what the far end knows it
		// by, the peer of the sign-in
		let id = yt_core_proto::base::PeerUuid::from(node.uuid).to_string();
		let challenge = node.challenge.lock().await.clone();
		let o = tls.offer(&id, &challenge);
		req.ca_cert = o.cert;
		req.ca_live = o.live;
		req.ca_roots_hash = o.roots_hash;
		req.ca_proof = o.proof;
	}
}

pub async fn take(node: &Node, _mine: &str, _upper: &str, grant: &LinkGrant) {
	if !grant.challenge.is_empty() {
		*node.challenge.lock().await = grant.challenge.clone();
	}
	// the az the node above takes this one to be in - its own: kept to be shown
	// (yt_info), set by nothing here
	*node.az.lock().await = if grant.granted { Some(grant.az.clone()) } else { None };
	#[cfg(feature = "tls")]
	if let Some(tls) = &node.tls {
		tls.take(grant);
	}
}
