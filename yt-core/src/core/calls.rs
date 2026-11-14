// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! The light node's version of a replaceable file: the calls to the cluster above, over
//! the public Link service. (ytserv has its own `calls.rs`: its peer pool, and a
//! target that is not leading there.)
//!
//! One channel to the target, kept between calls and made again after a failure, a
//! new target, or once TLS is held (review of 2026-11-13: a TCP and TLS handshake a
//! second otherwise).
use tonic::transport::Channel;
use yt_core_proto::link::link_client::LinkClient;
use yt_core_proto::link::{LinkGrant, LinkSignIn, RelayAck, RelayBatch, RelayFetch};

use super::relay::CALL_TIMEOUT;
use super::store::Node;

/// A node of the cluster above: its address.
pub type Target = String;

/// The host to call: the configured ones in turn, the next one after a failure.
pub async fn target(node: &Node, _az: &str) -> Option<Target> {
	if node.hosts.is_empty() {
		return None;
	}
	let i = *node.next_host.lock().await % node.hosts.len();
	Some(node.hosts[i].clone())
}

/// The target failed: the next host next time.
pub async fn forget(node: &Node, _az: &str) {
	let mut i = node.next_host.lock().await;
	*i = i.wrapping_add(1);
}

/// Which turn of the hosts the calls are at: it changes whenever a target is forgotten.
/// The cluster above admits a light node on the node it signed in through, so a new target
/// is signed in to before anything else is asked of it.
pub async fn turn(node: &Node) -> usize {
	*node.next_host.lock().await
}

/// how long a host may take to answer the dial: an unreachable one is left soon
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// yt-light, once the cluster above countersigned its CA: every call is TLS (crate::tls) -
/// `Some`, with which change of the chain it holds this is. A channel is kept for one of
/// them: after the cluster handed over another countersigned CA or other roots, the next
/// call connects again and presents the new chain (review of 2026-11-13).
#[cfg_attr(not(feature = "tls"), allow(unused_variables))]
fn tls_held(node: &Node) -> Option<u64> {
	#[cfg(feature = "tls")]
	if let Some(t) = &node.tls {
		return t.ready().then(|| t.generation());
	}
	None
}

/// A channel to `t`: TLS when `tls` - every reconnect too - else plaintext, the sign-in
/// being the one call the cluster above takes so.
#[cfg_attr(not(feature = "tls"), allow(unused_variables))]
async fn connect(node: &Node, t: &Target, tls: bool) -> Result<Channel, String> {
	#[cfg(feature = "tls")]
	if let Some(tls) = node.tls.as_ref().filter(|_| tls) {
		let addr = t.trim().trim_start_matches("http://").trim_start_matches("https://").trim_end_matches('/').to_string();
		let ep = tonic::transport::Endpoint::from_shared(format!("http://{}", addr)).map_err(|e| format!("{}: {}", t, e))?
			.connect_timeout(CONNECT_TIMEOUT);
		let tls = tls.clone();
		return ep.connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
			let (tls, addr) = (tls.clone(), addr.clone());
			async move { tls.dial(&addr, CONNECT_TIMEOUT).await.map(hyper_util::rt::TokioIo::new) }
		})).await.map_err(|e| t!("light-call-tls", host = t, error = e));
	}
	tonic::transport::Endpoint::new(t.clone()).map_err(|e| format!("{}: {}", t, e))?
		.connect_timeout(CONNECT_TIMEOUT).connect().await.map_err(|e| format!("{}: {}", t, e))
}

/// The client of `t`, on the kept channel when it is to the same target the same way.
async fn client(node: &Node, t: &Target) -> Result<LinkClient<Channel>, String> {
	let tls = tls_held(node);
	if let Some((kt, ktls, ch)) = node.channel.lock().await.as_ref()
		&& kt == t && *ktls == tls {
			return Ok(LinkClient::new(ch.clone()));
		}
	let ch = connect(node, t, tls.is_some()).await?;
	*node.channel.lock().await = Some((t.clone(), tls, ch.clone()));
	Ok(LinkClient::new(ch))
}

/// A call failed: its channel is not kept.
async fn failed(node: &Node, t: &Target, e: tonic::Status) -> String {
	*node.channel.lock().await = None;
	no_link(t, &e).unwrap_or_else(|| e.message().to_string())
}

/// A node that has no Link service - a YT Cluster before 1.21 - answers Unimplemented, with
/// no text: said as what it is, the host named
fn no_link(t: &Target, e: &tonic::Status) -> Option<String> {
	(e.code() == tonic::Code::Unimplemented).then(|| t!("light-no-link-service", host = t))
}

/// [compat] What a YT Cluster leader before 1.21 answers a sign-in that names no az - a
/// light node's, which it does not know: its words, as that release says them
const BEFORE_LIGHT: &str = "sign-in carried a level but no target_az";

/// A refusal of the cluster above as yt-light says it: one from a leader before 1.21 with
/// what that means, any other as it is
pub fn refused(reason: &str) -> String {
	if reason.contains(BEFORE_LIGHT) {
		t!("light-leader-before-link", reason = reason)
	} else {
		t!("light-signin-refused", reason = reason)
	}
}

fn timed<T>(m: T) -> tonic::Request<T> {
	let mut r = tonic::Request::new(m);
	r.set_timeout(CALL_TIMEOUT);
	r
}

pub async fn fetch(node: &Node, t: &Target, req: RelayFetch) -> Result<RelayBatch, String> {
	match client(node, t).await?.fetch(timed(req)).await {
		Ok(r) => Ok(r.into_inner()),
		Err(e) => Err(failed(node, t, e).await),
	}
}

pub async fn deliver(node: &Node, t: &Target, batch: RelayBatch) -> Result<RelayAck, String> {
	match client(node, t).await?.deliver(timed(batch)).await {
		Ok(r) => Ok(r.into_inner()),
		Err(e) => Err(failed(node, t, e).await),
	}
}

/// A sign-in: on TLS once it is held; plaintext when TLS does not get through - the one
/// call the cluster above takes either way, and the one that brings what TLS to it needs (its
/// roots after a rotation there, a CA countersigned again). Only then: an answer the az
/// above gave over TLS stands - it took the challenge, so the same sign-in again would be
/// refused (review of 2026-11-13).
pub async fn signin(node: &Node, t: &Target, req: LinkSignIn) -> Result<LinkGrant, String> {
	let tls = tls_held(node).is_some();
	let first = match client(node, t).await {
		Ok(mut c) => match c.signin(timed(req.clone())).await {
			Ok(r) => return Ok(r.into_inner()),
			Err(e) => {
				let through = !matches!(e.code(), tonic::Code::Unavailable | tonic::Code::Unknown);
				let m = failed(node, t, e).await;
				if through || !tls {
					return Err(m);
				}
				m
			}
		},
		Err(e) if tls => e,
		Err(e) => return Err(e),
	};
	// The answer over plaintext is unauthenticated (review of 2026-11-13). What it hands over
	// for TLS is taken only as it chains to the root pinned in `upper_root` (tls.rs `take`);
	// its `granted` and `az` are taken as they are: the az is shown, no more, and the fetch
	// and deliver after it are TLS again, which a node without the cluster's chain cannot answer.
	let ch = connect(node, t, false).await.map_err(|e| t!("light-call-then-plaintext", first = first, error = e))?;
	LinkClient::new(ch).signin(timed(req)).await.map(|r| r.into_inner())
		.map_err(|e| t!("light-call-then-plaintext", first = first, error = no_link(t, &e).unwrap_or_else(|| e.message().to_string())))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// [compat] a node of a YT Cluster before 1.21 has no Link service: it answers
	/// Unimplemented with no text, which is said as what it is, the host named
	#[test]
	fn a_node_with_no_link_service_is_said_as_one() {
		crate::i18n::set("en");
		let host = "http://u2:7421".to_string();
		let m = no_link(&host, &tonic::Status::unimplemented("")).unwrap();
		assert!(m.starts_with("YT-E077 http://u2:7421 has no Link service"), "{m}");
		assert!(no_link(&host, &tonic::Status::unavailable("refused")).is_none(), "another failure is said as it is");
	}

	/// [compat] a leader before 1.21 refuses a sign-in with no az in its own words: said with
	/// what that means; any other refusal as it is
	#[test]
	fn a_refusal_of_a_leader_before_light_nodes_says_so() {
		crate::i18n::set("en");
		let old = "sign-in carried a level but no target_az, so there is no cluster to match a key against";
		let m = refused(old);
		assert!(m.starts_with("YT-E078 ") && m.contains(old) && m.contains("1.21"), "{m}");
		let m = refused("light node x awaits approval");
		assert_eq!(m, "YT-E051 the cluster above refused the sign-in: light node x awaits approval");
	}
}
