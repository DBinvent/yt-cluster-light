// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! yt-light's service (crate::service) under the extension, on a throwaway PostgreSQL
//! (tests/fixture, YT_LIGHT_EXT_SO; skipped without it): yt_setup syncs a table and captures
//! it, a transaction's rows reach the outbox through the extension's calls and go up to a node
//! above served in this process, yt_info() answers, and a table without an A row keeps its
//! writes here.
mod fixture;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use yt_core::core::{relay, relay::Outcome, store, store::ABOVE};
use yt_core::pg::Pg;
use yt_core::service::Light;
use yt_core_proto::link::link_server::{Link, LinkServer};
use yt_core_proto::link::{LinkGrant, LinkSignIn, RelayAck, RelayBatch, RelayFetch, RelayItem, RelayPos};

#[derive(Default, Clone)]
struct Above {
	took: Arc<Mutex<BTreeMap<(u64, u32), RelayItem>>>,
	at: Arc<Mutex<Option<(u64, u32)>>>,
}

#[tonic::async_trait]
impl Link for Above {
	async fn signin(&self, _r: tonic::Request<LinkSignIn>) -> Result<tonic::Response<LinkGrant>, tonic::Status> {
		Ok(tonic::Response::new(LinkGrant { granted: true, az: "top".into(), ..Default::default() }))
	}
	async fn fetch(&self, r: tonic::Request<RelayFetch>) -> Result<tonic::Response<RelayBatch>, tonic::Status> {
		let from = r.into_inner().from;
		Ok(tonic::Response::new(RelayBatch { upto: from, ..Default::default() }))
	}
	async fn deliver(&self, r: tonic::Request<RelayBatch>) -> Result<tonic::Response<RelayAck>, tonic::Status> {
		let b = r.into_inner();
		if let Some(u) = b.upto {
			for i in b.items { self.took.lock().unwrap().insert((i.term, i.subterm), i); }
			*self.at.lock().unwrap() = Some((u.term, u.subterm));
		}
		let at = *self.at.lock().unwrap();
		Ok(tonic::Response::new(RelayAck { at: at.map(|p| RelayPos { term: p.0, subterm: p.1 }), ..Default::default() }))
	}
}

async fn up_until(node: &store::Node, above: &Above, n: usize) {
	for _ in 0..100 {
		let _ = relay::up(node, ABOVE).await;
		if above.took.lock().unwrap().values().map(|i| i.data.len()).sum::<usize>() >= n { return }
		tokio::time::sleep(Duration::from_millis(100)).await;
	}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_extension_writes_through_the_service_and_rows_go_up() {
	let Some(tmp) = fixture::TmpPg::start("svc").await else { return };
	let pg = Arc::new(Pg::connect(&tmp.url("appdb"), None, None).await.unwrap());
	let above = Above::default();
	let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let host = format!("http://{}", l.local_addr().unwrap());
	tokio::spawn(tonic::transport::Server::builder().add_service(LinkServer::new(above.clone()))
		.serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(l)));
	let node = Arc::new(store::Node::with_pg(7, vec![host], pg.clone()));
	let log = slog::Logger::root(slog::Discard, slog::o!());
	let socket = pg.socket_path().await.unwrap();
	let (stop, stopped) = tokio::sync::oneshot::channel();
	let (ended_send, _ended) = tokio::sync::mpsc::unbounded_channel();
	Light::new(node.clone(), pg.clone(), log).run(&socket, stopped, ended_send).await.unwrap();

	let c = tmp.client("appdb").await;
	// yt_setup: the trigger and the A row - the table syncs both ways
	c.batch_execute("CREATE TABLE public.t (id bigint PRIMARY KEY, v text); SELECT yt_setup('public.t');").await.unwrap();
	let a: i64 = c.query_one("SELECT count(*) FROM public.yt_config WHERE module = 'A' AND name = 'public.t'", &[]).await.unwrap().get(0);
	assert_eq!(a, 1, "yt_setup wrote the A row");
	// a transaction of two rows, and one rolled back
	c.batch_execute("BEGIN; INSERT INTO public.t VALUES (1, 'a'); UPDATE public.t SET v = 'b' WHERE id = 1; COMMIT;").await.unwrap();
	c.batch_execute("BEGIN; INSERT INTO public.t VALUES (2, 'gone'); ROLLBACK;").await.unwrap();
	up_until(&node, &above, 2).await;
	let rows: Vec<String> = above.took.lock().unwrap().values().flat_map(|i| i.data.iter())
		.map(|r| format!("{} {}", r.table.as_ref().unwrap().table, r.action)).collect();
	assert_eq!(rows.len(), 2, "the committed transaction's two rows went up, the rolled back one never: {rows:?}");
	assert_eq!(node.waiting().await, 0, "what the node above holds left the outbox");

	// yt_info(): what this node is, through the service
	let info: String = c.query_one("SELECT yt_info()::text", &[]).await.unwrap().get(0);
	for f in ["\"node\"", "rows_not_synced_here", "rows_not_held_here", "outbox", "uptime_sec"] {
		assert!(info.contains(f), "yt_info() names {f}: {info}");
	}
	let u: String = c.query_one("SELECT yt_info('U')::text", &[]).await.unwrap().get(0);
	assert!(u.contains("participant cfg=public.yt_config"), "{u}");

	// its A row gone: the table's writes stay here, counted
	c.batch_execute("DELETE FROM public.yt_config WHERE module = 'A' AND name = 'public.t'").await.unwrap();
	tokio::time::sleep(Duration::from_secs(3)).await; // the A rows are read again after 2 s
	c.batch_execute("BEGIN; INSERT INTO public.t VALUES (3, 'kept'); COMMIT;").await.unwrap();
	tokio::time::sleep(Duration::from_millis(500)).await;
	let _ = relay::up(&node, ABOVE).await;
	assert_eq!(above.took.lock().unwrap().values().map(|i| i.data.len()).sum::<usize>(), 2, "nothing more went up");
	assert!(pg.not_synced.load(std::sync::atomic::Ordering::Relaxed) >= 1);
	let _ = stop.send(());
	assert_eq!(relay::up(&node, ABOVE).await, Outcome::Idle);
}

/// a transaction that reached no node - it changed nothing, or only a table with no trigger,
/// in a session that never connected to the service - completes as nothing: yt_complete is 0,
/// as on a light node, for a commit and a rollback (2026-11-14: a cluster node raised
/// "yt_complete: not connected" for it, the first transaction of a session only)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_transaction_that_reached_no_node_completes_as_nothing() {
	let Some(tmp) = fixture::TmpPg::start("nothing").await else { return };
	let c = tmp.client("appdb").await;
	c.batch_execute("CREATE TABLE plain (id int PRIMARY KEY)").await.unwrap();
	for commit in [true, false] {
		c.batch_execute("BEGIN").await.unwrap();
		let n: i32 = c.query_one("SELECT yt_complete($1)", &[&commit]).await.unwrap().get(0);
		assert_eq!(n, 0, "changed nothing, commit {commit}");
		c.batch_execute("COMMIT").await.unwrap();
	}
	c.batch_execute("BEGIN; INSERT INTO plain VALUES (1); DELETE FROM plain WHERE id = 99;").await.unwrap();
	let n: i32 = c.query_one("SELECT yt_complete(true)", &[]).await.unwrap().get(0);
	assert_eq!(n, 0, "a table with no trigger");
	c.batch_execute("COMMIT").await.unwrap();
	let rows: i64 = c.query_one("SELECT count(*) FROM plain", &[]).await.unwrap().get(0);
	assert_eq!(rows, 1, "its write committed");
	// out of a transaction too
	let n: i32 = c.query_one("SELECT yt_complete(true)", &[]).await.unwrap().get(0);
	assert_eq!(n, 0);
}
