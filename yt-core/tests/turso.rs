// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! The lite node inside an application on Turso (feature `turso`), its link's turns
//! (`leaf::run`) against a node above served in this process: it signs in, what the node
//! above holds comes down into the application's tables, and what the application writes
//! goes up as the PostgreSQL extension would send it - typed, with each column's
//! PostgreSQL type id - and only once.
#![cfg(feature = "turso")]
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use turso::Value;
use yt_core::core::store::Node;
use yt_core::leaf::{self, Status};
use yt_core::turso::Turso;
use yt_core_proto::base::{var_type::Value as V, ColumnRef, TableRef, VarType};
use yt_core_proto::db::{DbAction, DbInputEntity};
use yt_core_proto::link::link_server::{Link, LinkServer};
use yt_core_proto::link::{LinkGrant, LinkSignIn, RelayAck, RelayBatch, RelayFetch, RelayItem, RelayPos};

#[derive(Default, Clone)]
struct Above {
	down: Arc<Mutex<Vec<RelayItem>>>,
	took: Arc<Mutex<BTreeMap<(u64, u32), RelayItem>>>,
	/// every position delivered, in order, a second delivery of one too (review of 2026-11-13)
	delivered: Arc<Mutex<Vec<(u64, u32)>>>,
	at: Arc<Mutex<Option<(u64, u32)>>>,
}

#[tonic::async_trait]
impl Link for Above {
	async fn signin(&self, _r: tonic::Request<LinkSignIn>) -> Result<tonic::Response<LinkGrant>, tonic::Status> {
		Ok(tonic::Response::new(LinkGrant { granted: true, az: "top".into(), challenge: b"c".to_vec(), ..Default::default() }))
	}
	async fn fetch(&self, r: tonic::Request<RelayFetch>) -> Result<tonic::Response<RelayBatch>, tonic::Status> {
		let from = r.into_inner().from.map(|p| (p.term, p.subterm));
		let items: Vec<RelayItem> = self.down.lock().unwrap().iter().filter(|i| Some((i.term, i.subterm)) > from).cloned().collect();
		let upto = items.last().map(|i| (i.term, i.subterm)).or(from).unwrap_or((0, 0));
		Ok(tonic::Response::new(RelayBatch { source_az: "top".into(), items, upto: Some(RelayPos { term: upto.0, subterm: upto.1 }),
			..Default::default() }))
	}
	async fn deliver(&self, r: tonic::Request<RelayBatch>) -> Result<tonic::Response<RelayAck>, tonic::Status> {
		let b = r.into_inner();
		if let Some(u) = b.upto {
			for i in b.items {
				self.delivered.lock().unwrap().push((i.term, i.subterm));
				self.took.lock().unwrap().insert((i.term, i.subterm), i);
			}
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

fn history(hid: i64) -> DbInputEntity {
	let cols = vec![
		ColumnRef { attnum: 1, name: "hid".into(), pk: true, atttypid: 20, ..Default::default() },
		ColumnRef { attnum: 2, name: "aid".into(), atttypid: 23, ..Default::default() },
		ColumnRef { attnum: 3, name: "delta".into(), atttypid: 23, ..Default::default() },
		ColumnRef { attnum: 4, name: "mtime".into(), atttypid: 1184, ..Default::default() }];
	let v = |x: V| VarType { value: Some(x) };
	DbInputEntity { table: Some(TableRef { db: "appdb".into(), schema: "s_t".into(), table: "history".into(), columns: cols, ..Default::default() }),
		new_data: [(1, v(V::Int(hid))), (2, v(V::Int(1))), (3, v(V::Int(1))), (4, v(V::Universal("2026-11-13 17:00:00+00".into())))].into_iter().collect(),
		action: DbAction::Insert as i32, ..Default::default() }
}

fn account(aid: i64, balance: i64) -> DbInputEntity {
	let cols = vec![
		ColumnRef { attnum: 1, name: "aid".into(), pk: true, atttypid: 23, ..Default::default() },
		ColumnRef { attnum: 2, name: "abalance".into(), atttypid: 23, ..Default::default() },
		ColumnRef { attnum: 3, name: "filler".into(), atttypid: 25, ..Default::default() }];
	let v = |x: V| VarType { value: Some(x) };
	DbInputEntity { table: Some(TableRef { db: "appdb".into(), schema: "s_u".into(), table: "accounts".into(), columns: cols, ..Default::default() }),
		new_data: [(1, v(V::Int(aid))), (2, v(V::Int(balance))), (3, v(V::Universal(format!("row {aid}"))))].into_iter().collect(),
		action: DbAction::Insert as i32, ..Default::default() }
}

async fn wait(what: &str, mut f: impl FnMut() -> bool) {
	for _ in 0..200 {
		if f() { return }
		tokio::time::sleep(Duration::from_millis(100)).await;
	}
	panic!("not in 20 s: {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_application_on_turso_is_a_leaf_of_the_cluster() {
	let path = std::env::temp_dir().join(format!("yt-turso-link-{}.db", std::process::id()));
	let _ = std::fs::remove_file(&path);
	let db = turso::Builder::new_local(path.to_str().unwrap()).build().await.unwrap();
	let app = db.connect().unwrap();
	app.busy_timeout(Duration::from_secs(10)).unwrap();
	app.execute_batch("CREATE TABLE \"s_u.accounts\" (aid int PRIMARY KEY, abalance int NOT NULL DEFAULT 0, filler text NOT NULL);
		CREATE TABLE \"s_t.history\" (hid bigint PRIMARY KEY, aid int NOT NULL, delta int NOT NULL, mtime timestamptz NOT NULL);").await.unwrap();
	let store = Arc::new(Turso::open(&db, "appdb").await.unwrap());
	store.setup("s_t.history").await.unwrap();
	// the table the cluster's rows come down to syncs: an A row
	store.sync("s_u.accounts", "").await.unwrap();

	let above = Above::default();
	// and a row of the application's own captured table, written above: it comes down, and
	// never goes back up (the loop guard)
	*above.down.lock().unwrap() = vec![RelayItem { term: 1, subterm: 1, data: vec![account(1, 10), account(2, 20), history(100)] }];
	let host = serve(above.clone()).await;
	let node = Arc::new(Node::with_db(7, vec![host], store.clone()));
	let status = Arc::new(Mutex::new(Status::default()));
	let said = Arc::new(Mutex::new(Vec::<String>::new()));
	let s2 = said.clone();
	let linker = tokio::spawn(leaf::run(node.clone(), status.clone(), move |s| s2.lock().unwrap().push(s)));

	wait("signed in and caught up", || { let s = status.lock().unwrap(); s.granted && s.caught_up }).await;
	assert_eq!(status.lock().unwrap().az.as_deref(), Some("top"));
	let mut r = app.query("SELECT aid, abalance, filler FROM \"s_u.accounts\" ORDER BY aid", ()).await.unwrap();
	let mut got = Vec::new();
	while let Some(row) = r.next().await.unwrap() {
		got.push((row.get_value(0).unwrap(), row.get_value(1).unwrap(), row.get_value(2).unwrap()));
	}
	assert_eq!(got, vec![(Value::Integer(1), Value::Integer(10), Value::Text("row 1".into())),
		(Value::Integer(2), Value::Integer(20), Value::Text("row 2".into()))], "what the node above holds came down");

	// the application writes: it goes up, typed
	app.execute("INSERT INTO \"s_t.history\" VALUES (1, 2, -5, '2026-11-13 18:00:00+00')", ()).await.unwrap();
	let took = above.took.clone();
	wait("the application's row taken above", || !took.lock().unwrap().is_empty()).await;
	wait("nothing waits to go up", || status.lock().unwrap().waiting == 0).await;
	let row = took.lock().unwrap().values().next().unwrap().data[0].clone();
	let t = row.table.as_ref().unwrap();
	assert_eq!((t.db.as_str(), t.schema.as_str(), t.table.as_str()), ("appdb", "s_t", "history"));
	assert_eq!(t.columns.iter().map(|c| c.atttypid).collect::<Vec<_>>(), vec![20, 23, 23, 1184]);
	assert_eq!(row.new_data[&3].value, Some(V::Int(-5)));
	assert_eq!(row.new_data[&4].value, Some(V::Universal("2026-11-13 18:00:00+00".into())));

	// more comes down later; nothing came down twice, nothing went up twice
	above.down.lock().unwrap().push(RelayItem { term: 1, subterm: 2, data: vec![account(3, 30)] });
	wait("the next row came down", || store.not_held.load(std::sync::atomic::Ordering::Relaxed) == 0 && {
		let s = status.lock().unwrap(); s.caught_up && s.turns > 0 }).await;
	for _ in 0..50 {
		let mut r = app.query("SELECT count(*) FROM \"s_u.accounts\"", ()).await.unwrap();
		if r.next().await.unwrap().unwrap().get_value(0).unwrap() == Value::Integer(3) { break }
		tokio::time::sleep(Duration::from_millis(100)).await;
	}
	let mut r = app.query("SELECT count(*) FROM \"s_u.accounts\"", ()).await.unwrap();
	assert_eq!(r.next().await.unwrap().unwrap().get_value(0).unwrap(), Value::Integer(3));
	assert_eq!(above.took.lock().unwrap().len(), 1, "one row went up");
	assert_eq!(above.delivered.lock().unwrap().len(), 1, "once: {:?}", above.delivered.lock().unwrap());
	let row = above.took.lock().unwrap().values().next().unwrap().data[0].clone();
	assert_eq!(row.new_data[&1].value, Some(V::Int(1)), "the application's own row, not the one that came down (hid 100)");
	let mut r = app.query("SELECT count(*) FROM \"s_t.history\" WHERE hid = 100", ()).await.unwrap();
	assert_eq!(r.next().await.unwrap().unwrap().get_value(0).unwrap(), Value::Integer(1), "it came down");
	assert!(status.lock().unwrap().problem.is_none(), "{:?} - said: {:?}", status.lock().unwrap().problem, said.lock().unwrap());
	linker.abort();
	drop(app);
	let _ = std::fs::remove_file(&path);
}
