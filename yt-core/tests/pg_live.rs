// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! yt-light's store on a real PostgreSQL (crate::pg): down applies a batch into a table
//! with its cursor, once; up delivers the outbox from where the node above is, and what
//! the node above holds leaves the outbox. Run against a database of a throwaway instance -
//! it creates and drops table `yt_light_live` - given, or one the test starts itself (fixture):
//!
//!   YT_LIGHT_TEST_DB='postgresql://user@localhost/db?host=/run/postgresql' cargo test --test pg_live
//!   YT_LIGHT_EXT_SO=ytpgc/yt_cluster.so cargo test --test pg_live
//!
//! With neither it is skipped.
mod fixture;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use yt_core::core::{relay, relay::Outcome, store, store::ABOVE};
use yt_core::pg::Pg;
use yt_core_proto::base::{ColumnRef, TableRef, VarType, var_type::Value};
use yt_core_proto::db::{DbAction, DbInputEntity};
use yt_core_proto::link::link_server::{Link, LinkServer};
use yt_core_proto::link::{LinkGrant, LinkSignIn, RelayAck, RelayBatch, RelayFetch, RelayItem, RelayPos};

#[derive(Default, Clone)]
struct Above {
	down: Arc<Mutex<Vec<RelayItem>>>,
	took: Arc<Mutex<BTreeMap<(u64, u32), RelayItem>>>,
	at: Arc<Mutex<Option<(u64, u32)>>>,
}

#[tonic::async_trait]
impl Link for Above {
	async fn signin(&self, _r: tonic::Request<LinkSignIn>) -> Result<tonic::Response<LinkGrant>, tonic::Status> {
		Ok(tonic::Response::new(LinkGrant { granted: true, ..Default::default() }))
	}
	async fn fetch(&self, r: tonic::Request<RelayFetch>) -> Result<tonic::Response<RelayBatch>, tonic::Status> {
		let from = r.into_inner().from.map(|p| (p.term, p.subterm));
		let items: Vec<RelayItem> = self.down.lock().unwrap().iter().filter(|i| Some((i.term, i.subterm)) > from).cloned().collect();
		let upto = items.last().map(|i| (i.term, i.subterm)).or(from).unwrap_or((0, 0));
		Ok(tonic::Response::new(RelayBatch { source_az: "top".into(), items, upto: Some(RelayPos { term: upto.0, subterm: upto.1 }), ..Default::default() }))
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

fn row(db: &str, action: DbAction, id: i64, name: Option<&str>) -> DbInputEntity {
	let mut e = DbInputEntity {
		table: Some(TableRef { db: db.into(), schema: "public".into(), table: "yt_light_live".into(), columns: vec![
			ColumnRef { attnum: 1, name: "id".into(), pk: true, ..Default::default() },
			ColumnRef { attnum: 2, name: "name".into(), ..Default::default() }], ..Default::default() }),
		action: action as i32,
		..Default::default()
	};
	let data = if action == DbAction::Delete { &mut e.old_data } else { &mut e.new_data };
	data.insert(1, VarType { value: Some(Value::Int(id)) });
	if let Some(n) = name {
		e.new_data.insert(2, VarType { value: Some(Value::Universal(n.into())) });
	}
	if action == DbAction::Update {
		e.old_data.insert(1, VarType { value: Some(Value::Int(id)) });
	}
	e
}

#[tokio::test]
async fn yt_light_applies_what_comes_down_and_sends_its_outbox_up() {
	// the database given, else a throwaway PostgreSQL (tests/fixture); neither: skipped
	let tmp;
	let url = match std::env::var("YT_LIGHT_TEST_DB") {
		Ok(u) => u,
		Err(_) => {
			tmp = match fixture::TmpPg::start("live").await { Some(t) => t, None => return };
			tmp.url("appdb")
		}
	};
	let pg = Arc::new(Pg::connect(&url, None, None).await.unwrap());
	let db = pg.db.clone();
	// the test's table, and an empty outbox and cursors
	let (c, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
	tokio::spawn(conn);
	c.batch_execute(&format!("DROP TABLE IF EXISTS public.yt_light_live; CREATE TABLE public.yt_light_live (id bigint PRIMARY KEY, name text); \
		DELETE FROM {s}.yt_outbox; DELETE FROM {s}.yt_position WHERE name LIKE 'cursor%' OR name LIKE 'passed%'", s = pg.schema)).await.unwrap();
	// both tables sync (crate::sync): an A row each, as yt_setup writes one - the second
	// names a table this database does not have
	c.batch_execute(&format!("DELETE FROM {s}.{t} WHERE module = 'A' AND name IN ('public.yt_light_live', 'public.yt_light_not_here'); \
		INSERT INTO {s}.{t} (name, module, value) VALUES ('public.yt_light_live', 'A', ''), ('public.yt_light_not_here', 'A', '')",
		s = pg.schema, t = pg.config_table)).await.unwrap();

	let above = Above::default();
	*above.down.lock().unwrap() = vec![
		RelayItem { term: 7, subterm: 1, data: vec![row(&db, DbAction::Insert, 1, Some("a")), row(&db, DbAction::Insert, 2, Some("b"))] },
		RelayItem { term: 7, subterm: 2, data: vec![row(&db, DbAction::Update, 2, Some("bb")), row(&db, DbAction::Delete, 1, None)] },
	];
	let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let host = format!("http://{}", l.local_addr().unwrap());
	tokio::spawn(tonic::transport::Server::builder().add_service(LinkServer::new(above.clone()))
		.serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(l)));
	let node = store::Node::with_pg(7, vec![host], pg.clone());

	// down: applied into the table, the cursor moved; again: nothing changes
	assert_eq!(relay::down(&node, ABOVE).await, Outcome::Moved);
	let rows: Vec<(i64, String)> = c.query("SELECT id, name FROM public.yt_light_live ORDER BY id", &[]).await.unwrap()
		.iter().map(|r| (r.get(0), r.get(1))).collect();
	assert_eq!(rows, vec![(2, "bb".to_string())]);
	assert_eq!(store::cursor(&node, ABOVE).await, Some((7, 2)));
	assert_eq!(relay::down(&node, ABOVE).await, Outcome::Idle);
	// a light node has no az: the node above sends what its own az holds, in every database
	// and table. A row of another database, or of a table this database does not have, is
	// not this node's: left out and counted, the rows beside it applied, the cursor moved
	let mut elsewhere = row(&db, DbAction::Insert, 9, Some("x"));
	elsewhere.table.as_mut().unwrap().table = "yt_light_not_here".into();
	above.down.lock().unwrap().push(RelayItem { term: 7, subterm: 3, data: vec![
		row("other", DbAction::Insert, 9, Some("x")), elsewhere, row(&db, DbAction::Insert, 3, Some("c"))] });
	assert_eq!(relay::down(&node, ABOVE).await, Outcome::Moved);
	assert_eq!(store::cursor(&node, ABOVE).await, Some((7, 3)));
	assert_eq!(pg.not_held.load(std::sync::atomic::Ordering::Relaxed), 2, "one of another database, one of a table not here");
	let rows: Vec<(i64, String)> = c.query("SELECT id, name FROM public.yt_light_live ORDER BY id", &[]).await.unwrap()
		.iter().map(|r| (r.get(0), r.get(1))).collect();
	assert_eq!(rows, vec![(2, "bb".to_string()), (3, "c".to_string())], "the row of the table held here is applied");
	assert_eq!(pg.apply_error.lock().unwrap().as_str(), "", "a row not held is no error");
	// one cursor, kept under no az's name
	let names: Vec<String> = c.query(&format!("SELECT name FROM {}.yt_position WHERE name LIKE 'cursor%'", pg.schema), &[]).await.unwrap()
		.iter().map(|r| r.get(0)).collect();
	assert_eq!(names, vec!["cursor".to_string()]);

	// real transactions of this database: one committed, one rolled back, one still open
	let xid = |sql: &'static str| { let c = &c; async move {
		let tx = c.simple_query(sql).await.unwrap();
		tx.iter().find_map(|m| match m { tokio_postgres::SimpleQueryMessage::Row(r) => r.get(0).map(|v| v.parse::<u64>().unwrap()), _ => None }).unwrap()
	} };
	let committed = xid("BEGIN; SELECT pg_current_xact_id()::text; COMMIT").await;
	let aborted = xid("BEGIN; SELECT pg_current_xact_id()::text; ROLLBACK").await;
	let other_committed = xid("BEGIN; SELECT pg_current_xact_id()::text; COMMIT").await;

	// up: a prepared and committed transaction goes up at its position, a rolled-back one never
	let p = pg.prepare(committed, vec![row(&db, DbAction::Insert, 10, Some("up"))]).await.unwrap();
	assert_eq!(pg.commit(committed).await.unwrap(), Some(p));
	pg.prepare(aborted, vec![row(&db, DbAction::Insert, 11, Some("no"))]).await.unwrap();
	assert_eq!(pg.commit(aborted).await.unwrap(), None, "the extension said commit, the database aborted it");
	assert_eq!(pg.commit(other_committed).await.unwrap(), None, "nothing prepared: no position");
	assert_eq!(relay::up(&node, ABOVE).await, Outcome::Moved);
	assert_eq!(above.took.lock().unwrap().keys().copied().collect::<Vec<_>>(), vec![p]);
	assert_eq!(node.waiting().await, 0, "what the node above holds leaves the outbox");
	assert_eq!(relay::up(&node, ABOVE).await, Outcome::Idle);
	// the node above comes back from a backup, before that row: it is sent again
	*above.at.lock().unwrap() = Some((p.0, p.1 - 1));
	above.took.lock().unwrap().clear();
	assert_eq!(relay::up(&node, ABOVE).await, Outcome::Moved);
	assert_eq!(above.took.lock().unwrap().keys().copied().collect::<Vec<_>>(), vec![p]);
	c.batch_execute("DROP TABLE public.yt_light_live").await.unwrap();
}
