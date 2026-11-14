// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! The yt-lite binary, run as an operator runs it, on a throwaway PostgreSQL (tests/fixture,
//! YT_LITE_EXT_SO; skipped without it) and linked to a node above served in this process: it
//! starts, serves the extension's socket, a row written in its database goes up, one from above
//! comes down, yt_info() answers through it, and SIGTERM stops it cleanly. And its command line:
//! -h, -V, a bad argument.
mod fixture;

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use yt_core_proto::base::{ColumnRef, TableRef, VarType, var_type::Value};
use yt_core_proto::db::{DbAction, DbInputEntity};
use yt_core_proto::link::link_server::{Link, LinkServer};
use yt_core_proto::link::{LinkGrant, LinkSignIn, RelayAck, RelayBatch, RelayFetch, RelayItem, RelayPos};

const BIN: &str = env!("CARGO_BIN_EXE_yt-lite");

#[derive(Default, Clone)]
struct Above {
	down: Arc<Mutex<Vec<RelayItem>>>,
	took: Arc<Mutex<BTreeMap<(u64, u32), RelayItem>>>,
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
		Ok(tonic::Response::new(RelayBatch { items, upto: Some(RelayPos { term: upto.0, subterm: upto.1 }), ..Default::default() }))
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

fn down_row(id: i64, v: &str) -> DbInputEntity {
	let cols = vec![ColumnRef { attnum: 1, name: "id".into(), pk: true, atttypid: 20, ..Default::default() },
		ColumnRef { attnum: 2, name: "v".into(), atttypid: 25, ..Default::default() }];
	let x = |v: Value| VarType { value: Some(v) };
	DbInputEntity { table: Some(TableRef { db: "appdb".into(), schema: "public".into(), table: "t".into(), columns: cols, ..Default::default() }),
		new_data: [(1, x(Value::Int(id))), (2, x(Value::Universal(v.into())))].into_iter().collect(),
		action: DbAction::Insert as i32, ..Default::default() }
}

#[test]
fn its_command_line_answers_help_version_and_a_bad_argument() {
	let h = Command::new(BIN).arg("-h").output().unwrap();
	assert!(h.status.success());
	assert!(String::from_utf8_lossy(&h.stdout).contains("yt-lite"), "{}", String::from_utf8_lossy(&h.stdout));
	let v = Command::new(BIN).arg("-V").output().unwrap();
	assert!(String::from_utf8_lossy(&v.stdout).contains(env!("CARGO_PKG_VERSION")) || String::from_utf8_lossy(&v.stderr).contains(env!("CARGO_PKG_VERSION")));
	let bad = Command::new(BIN).arg("--no-such-flag").output().unwrap();
	assert!(!bad.status.success(), "a bad argument is refused");
	// no database to serve: it says so and stops
	let none = Command::new(BIN).args(["--db", "postgresql://nobody@localhost/none?host=/nonexistent"]).output().unwrap();
	assert!(!none.status.success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn yt_lite_runs_serves_its_database_and_links_it() {
	let Some(tmp) = fixture::TmpPg::start("bin").await else { return };
	let above = Above::default();
	let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let host = format!("http://{}", l.local_addr().unwrap());
	tokio::spawn(tonic::transport::Server::builder().add_service(LinkServer::new(above.clone()))
		.serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(l)));
	// its settings: rows of its config table, module L (`--file`: the test below)
	tmp.client("appdb").await.batch_execute(&format!("INSERT INTO public.yt_config (name, module, value) VALUES
		('upper', 'L', '{host}'), ('key_dir', 'L', '{}'), ('lang', 'L', 'en')", tmp.dir.join("key").display())).await.unwrap();
	let mut child = Command::new(BIN)
		.args(["--db", &tmp.url("appdb"), "--uuid", "00000000-0000-4000-8000-0000000000e3"])
		.stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
	let socket = tmp.sockdir.join(format!(".s.YT.{}", tmp.port));
	for _ in 0..100 {
		if socket.exists() { break }
		tokio::time::sleep(Duration::from_millis(100)).await;
	}
	if !socket.exists() {
		let _ = child.kill();
		let out = child.wait_with_output().unwrap();
		panic!("yt-lite serves no {}: {}", socket.display(), String::from_utf8_lossy(&out.stderr));
	}

	let c = tmp.client("appdb").await;
	c.batch_execute("CREATE TABLE public.t (id bigint PRIMARY KEY, v text); SELECT yt_setup('public.t');").await.unwrap();
	let info: String = c.query_one("SELECT yt_info()::text", &[]).await.unwrap().get(0);
	assert!(info.contains(env!("CARGO_PKG_VERSION")), "{info}");
	// up: its link loop takes the committed row to the node above
	c.batch_execute("BEGIN; INSERT INTO public.t VALUES (1, 'up'); COMMIT;").await.unwrap();
	// down: what the node above holds comes into the table
	above.down.lock().unwrap().push(RelayItem { term: 3, subterm: 1, data: vec![down_row(2, "down")] });
	let mut ok = (false, false);
	for _ in 0..150 {
		ok.0 = above.took.lock().unwrap().values().any(|i| !i.data.is_empty());
		ok.1 = c.query_opt("SELECT 1 FROM public.t WHERE id = 2 AND v = 'down'", &[]).await.unwrap().is_some();
		if ok == (true, true) { break }
		tokio::time::sleep(Duration::from_millis(200)).await;
	}
	assert_eq!(ok, (true, true), "a row went up, one came down");
	// a clean stop on SIGTERM
	let st = Command::new("kill").args(["-TERM", &child.id().to_string()]).status().unwrap();
	assert!(st.success());
	let mut stopped = None;
	for _ in 0..100 {
		if let Some(s) = child.try_wait().unwrap() { stopped = Some(s); break }
		tokio::time::sleep(Duration::from_millis(100)).await;
	}
	let st = stopped.unwrap_or_else(|| { let _ = child.kill(); panic!("yt-lite did not stop on SIGTERM") });
	assert!(st.success(), "{st:?}");
	assert!(!socket.exists(), "its socket is gone");
}

// its settings from `--file <path>` (marg 0.4.7; 0.4.4 read a file named "file"): it starts
// with the node above named there, and a row goes up to it
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn yt_lite_takes_its_settings_from_file() {
	let Some(tmp) = fixture::TmpPg::start("binf").await else { return };
	let above = Above::default();
	let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let host = format!("http://{}", l.local_addr().unwrap());
	tokio::spawn(tonic::transport::Server::builder().add_service(LinkServer::new(above.clone()))
		.serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(l)));
	let settings = tmp.dir.join("yt-lite.conf");
	std::fs::write(&settings, format!("upper={host}\nkey_dir={}\nlang=en\n", tmp.dir.join("key").display())).unwrap();
	let mut child = Command::new(BIN)
		.args(["--db", &tmp.url("appdb"), "--uuid", "00000000-0000-4000-8000-0000000000e4", "--file", settings.to_str().unwrap()])
		.stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
	let socket = tmp.sockdir.join(format!(".s.YT.{}", tmp.port));
	for _ in 0..100 {
		if socket.exists() { break }
		tokio::time::sleep(Duration::from_millis(100)).await;
	}
	if !socket.exists() {
		let _ = child.kill();
		let out = child.wait_with_output().unwrap();
		panic!("yt-lite serves no {}: {}", socket.display(), String::from_utf8_lossy(&out.stderr));
	}
	let c = tmp.client("appdb").await;
	c.batch_execute("CREATE TABLE public.t (id bigint PRIMARY KEY, v text); SELECT yt_setup('public.t');").await.unwrap();
	c.batch_execute("BEGIN; INSERT INTO public.t VALUES (1, 'up'); COMMIT;").await.unwrap();
	let mut up = false;
	for _ in 0..150 {
		up = above.took.lock().unwrap().values().any(|i| !i.data.is_empty());
		if up { break }
		tokio::time::sleep(Duration::from_millis(200)).await;
	}
	let _ = Command::new("kill").args(["-TERM", &child.id().to_string()]).status();
	for _ in 0..100 {
		if child.try_wait().unwrap().is_some() { break }
		tokio::time::sleep(Duration::from_millis(100)).await;
	}
	if child.try_wait().unwrap().is_none() { let _ = child.kill(); }
	assert!(up, "a row went up to the node named in --file");
}
