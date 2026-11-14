// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! The shared relay loops (core/relay.rs) on a light node, against a node above
//! served in this process over the public Link service: down applies what crossed,
//! once; up delivers the outbox from where the node above is. The light node names no
//! az in any of it: the node above knows it by its uuid, and says which az it shares.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use yt_core::core::{relay, relay::Outcome, store, store::ABOVE, signin};
use yt_core_proto::base::{TableRef, VarType, var_type::Value};
use yt_core_proto::db::DbInputEntity;
use yt_core_proto::link::link_server::{Link, LinkServer};
use yt_core_proto::link::{LinkGrant, LinkSignIn, RelayAck, RelayBatch, RelayFetch, RelayItem, RelayPos};

/// The node above, in az `top`: what its az holds for the light node, what it took
/// from it, and every az a light node's call named - none, when the light node is right.
#[derive(Default, Clone)]
struct Above {
	down: Arc<Mutex<Vec<RelayItem>>>,
	took: Arc<Mutex<BTreeMap<(u64, u32), RelayItem>>>,
	at: Arc<Mutex<Option<(u64, u32)>>>,
	fetches: Arc<Mutex<u32>>,
	/// what the light node's calls carried where a cluster's carry an az
	named: Arc<Mutex<Vec<String>>>,
}

fn row(id: i64) -> DbInputEntity {
	let mut e = DbInputEntity { table: Some(TableRef { db: "appdb".into(), schema: "public".into(), table: "t".into(), ..Default::default() }), ..Default::default() };
	e.new_data.insert(1, VarType { value: Some(Value::Int(id)) });
	e
}

#[tonic::async_trait]
impl Link for Above {
	async fn signin(&self, r: tonic::Request<LinkSignIn>) -> Result<tonic::Response<LinkGrant>, tonic::Status> {
		let r = r.into_inner();
		// a light node presents no az: the node above knows it by its uuid (7 here), and
		// tells it the az it is taken to be in - its own
		if let Some(a) = r.target_az {
			self.named.lock().unwrap().push(format!("signin target_az {}", a));
		}
		let known = r.peer.is_some_and(|p| p.as_uuid() == 7);
		Ok(tonic::Response::new(LinkGrant {
			granted: r.level == yt_core_proto::link::PeerAzLevel::Upper as i32 && known && r.target_az.is_none(),
			challenge: b"next".to_vec(), az: "top".into(), ..Default::default() }))
	}
	async fn fetch(&self, r: tonic::Request<RelayFetch>) -> Result<tonic::Response<RelayBatch>, tonic::Status> {
		*self.fetches.lock().unwrap() += 1;
		let r = r.into_inner();
		if !r.az.is_empty() {
			self.named.lock().unwrap().push(format!("fetch az {}", r.az));
		}
		let from = r.from.map(|p| (p.term, p.subterm));
		let all = self.down.lock().unwrap().clone();
		let items: Vec<RelayItem> = all.into_iter().filter(|i| Some((i.term, i.subterm)) > from).collect();
		let upto = items.last().map(|i| (i.term, i.subterm)).or(from).unwrap_or((0, 0));
		Ok(tonic::Response::new(RelayBatch { source_az: "top".into(), items,
			upto: Some(RelayPos { term: upto.0, subterm: upto.1 }), more: false, ..Default::default() }))
	}
	async fn deliver(&self, r: tonic::Request<RelayBatch>) -> Result<tonic::Response<RelayAck>, tonic::Status> {
		let b = r.into_inner();
		if !b.source_az.is_empty() || !b.light.is_empty() {
			self.named.lock().unwrap().push(format!("deliver source_az [{}] light [{}]", b.source_az, b.light));
		}
		if let Some(u) = b.upto {
			for i in b.items { self.took.lock().unwrap().insert((i.term, i.subterm), i); }
			*self.at.lock().unwrap() = Some((u.term, u.subterm));
		}
		let at = *self.at.lock().unwrap();
		Ok(tonic::Response::new(RelayAck { at: at.map(|p| RelayPos { term: p.0, subterm: p.1 }), ..Default::default() }))
	}
}

async fn serve(above: Above) -> String {
	let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = l.local_addr().unwrap();
	tokio::spawn(tonic::transport::Server::builder().add_service(LinkServer::new(above))
		.serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(l)));
	format!("http://{}", addr)
}

#[tokio::test]
async fn a_light_node_relays_down_once_and_up_from_where_the_node_above_is() {
	let above = Above::default();
	*above.down.lock().unwrap() = vec![
		RelayItem { term: 1, subterm: 1, data: vec![row(1)] },
		RelayItem { term: 1, subterm: 2, data: vec![row(2)] },
	];
	let host = serve(above.clone()).await;
	let node = store::Node::new(7, vec![host]);

	// down: both rows, then nothing new - and a batch sent again changes nothing
	assert_eq!(relay::down(&node, ABOVE).await, Outcome::Moved);
	assert_eq!(node.received().await.len(), 2);
	assert_eq!(store::cursor(&node, ABOVE).await, Some((1, 2)));
	assert_eq!(relay::down(&node, ABOVE).await, Outcome::Idle);
	let again = RelayBatch { source_az: "top".into(), items: above.down.lock().unwrap().clone(),
		upto: Some(RelayPos { term: 1, subterm: 2 }), ..Default::default() };
	assert_eq!(store::apply(&node, &again).await, Ok(Some((1, 2))));
	assert_eq!(node.received().await.len(), 2, "at or below the cursor: skipped");

	// up: the outbox, from where the node above is
	node.committed((1, 1), vec![row(10)]).await;
	node.committed((1, 2), vec![row(11), row(12)]).await;
	assert_eq!(relay::up(&node, ABOVE).await, Outcome::Moved);
	assert_eq!(above.took.lock().unwrap().len(), 2);
	assert_eq!(*above.at.lock().unwrap(), Some((1, 2)));
	assert_eq!(node.waiting().await, 0, "what the node above holds leaves the outbox");
	assert_eq!(relay::up(&node, ABOVE).await, Outcome::Idle, "nothing after where the node above is");
	store::passed(&node, ABOVE, (1, 2)).await;
	node.committed((1, 3), vec![row(13)]).await;
	assert_eq!(relay::up(&node, ABOVE).await, Outcome::Moved);
	assert_eq!(above.took.lock().unwrap().keys().copied().collect::<Vec<_>>(), vec![(1, 1), (1, 2), (1, 3)]);
	assert_eq!(node.waiting().await, 0);

	// the sign-in: granted, and the next challenge kept
	let g = signin::sign_in(&node, ABOVE).await.unwrap();
	assert!(g.granted);
}

/// A light node has no az: its sign-in, its fetch and its batches name none, and the az
/// it is in is what the node above says - shown, with nothing here to set it.
#[tokio::test]
async fn a_light_node_names_no_az_and_is_told_the_one_it_shares() {
	let above = Above::default();
	*above.down.lock().unwrap() = vec![RelayItem { term: 1, subterm: 1, data: vec![row(1)] }];
	let host = serve(above.clone()).await;
	let node = store::Node::new(7, vec![host]);
	assert_eq!(store::my_az(&node).await, "", "no az of its own");
	assert_eq!(node.told_az().await, None, "not told before a sign-in");

	let g = signin::sign_in(&node, ABOVE).await.unwrap();
	assert!(g.granted, "{}", g.reason);
	assert_eq!(node.told_az().await.as_deref(), Some("top"), "the az of the node above, as it said");
	assert_eq!(relay::down(&node, ABOVE).await, Outcome::Moved);
	node.committed((1, 1), vec![row(10)]).await;
	assert_eq!(relay::up(&node, ABOVE).await, Outcome::Moved);
	assert_eq!(*above.named.lock().unwrap(), Vec::<String>::new(), "no call of a light node names an az");

	// the cursor is one, whatever az a batch says its rows were committed in
	let other = RelayBatch { source_az: "elsewhere".into(), items: vec![RelayItem { term: 1, subterm: 1, data: vec![row(1)] }],
		upto: Some(RelayPos { term: 1, subterm: 1 }), ..Default::default() };
	assert_eq!(store::apply(&node, &other).await, Ok(Some((1, 1))));
	assert_eq!(node.received().await.len(), 1, "the same position again, under another az's name: skipped");
	assert_eq!(store::cursor(&node, "elsewhere").await, store::cursor(&node, ABOVE).await);

	// a node that is not known above (another uuid) is refused: told no az
	let stranger = store::Node::new(8, node.hosts.clone());
	let g = signin::sign_in(&stranger, ABOVE).await.unwrap();
	assert!(!g.granted);
	assert_eq!(stranger.told_az().await, None, "a refusal tells no az");
	// and the az told before does not outlive its grant: no answer, nothing to show
	assert_eq!(node.told_az().await.as_deref(), Some("top"));
	node.not_granted().await;
	assert_eq!(node.told_az().await, None);
}

#[tokio::test]
async fn a_node_above_that_does_not_answer_is_said_and_the_next_host_tried() {
	let node = store::Node::new(7, vec!["http://127.0.0.1:9".into()]);
	assert!(matches!(relay::down(&node, ABOVE).await, Outcome::FetchFailed(_)));
	assert!(matches!(relay::up(&node, ABOVE).await, Outcome::AskFailed(_)));
	assert_eq!(relay::down(&store::Node::new(7, vec![]), ABOVE).await, Outcome::NoTarget);
	assert_eq!(signin::sign_in(&store::Node::new(7, vec![]), ABOVE).await.unwrap_err(),
		"YT-E030 no node of the cluster above to sign in through");
}

/// [compat] a node of a YT Cluster before 1.21 serves no Link service: the sign-in says so
/// (YT-E077, the host named, where tonic's answer has no text), and the next host of
/// `upper` signs the light node in - as yt-light's loop turns to it after a failed sign-in
#[tokio::test]
async fn a_node_with_no_link_service_is_said_and_the_next_host_signs_in() {
	let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let old = format!("http://{}", l.local_addr().unwrap());
	tokio::spawn(tonic::transport::Server::builder().add_routes(tonic::service::Routes::default())
		.serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(l)));
	let host = serve(Above::default()).await;
	let node = store::Node::new(7, vec![old.clone(), host]);
	let e = signin::sign_in(&node, ABOVE).await.unwrap_err();
	assert!(e.starts_with(&format!("YT-E077 {} has no Link service", old)), "{e}");
	yt_core::core::calls::forget(&node, ABOVE).await;
	let g = signin::sign_in(&node, ABOVE).await.unwrap();
	assert!(g.granted, "{}", g.reason);
	assert_eq!(node.told_az().await.as_deref(), Some("top"));
}
