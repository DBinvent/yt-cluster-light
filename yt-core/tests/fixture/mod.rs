// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! A throwaway PostgreSQL with the yt_cluster extension in a database of it, for the tests that
//! need a real one (pg_live.rs, pg_service.rs, yt_lite_bin.rs). Nothing is installed: the
//! server runs from its own binaries on a data directory of its own, its socket in a directory
//! of its own, and the extension's script runs in database `appdb` with its C functions bound
//! to the library the variable names - what CREATE EXTENSION would do, but for the catalog
//! entry no test needs.
//!
//!   YT_LITE_EXT_SO   the extension's library for this PostgreSQL's major (`make -C ytpgc`
//!                     builds ytpgc/yt_cluster.so); unset: the test is skipped
//!   PG_BIN            the server's binaries (default: `pg_config --bindir`)
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct TmpPg {
	pub dir: PathBuf,
	/// where the server's socket is - and yt-lite's, which the extension finds beside it
	pub sockdir: PathBuf,
	pub port: u16,
	bin: PathBuf,
}

/// The extension's script, as CREATE EXTENSION would run it in schema public, its C functions
/// bound to `so`: the psql guard and the dump registrations (an extension's own) left out.
pub fn script(so: &str) -> String {
	let path = format!("{}/../ytpgc/{}", env!("CARGO_MANIFEST_DIR"), yt_core_proto::EXT_SQL);
	std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
		.lines().filter(|l| !l.starts_with("\\echo") && !l.contains("pg_extension_config_dump"))
		.collect::<Vec<_>>().join("\n")
		.replace("MODULE_PATHNAME", so).replace("@extschema@", "public")
}

impl TmpPg {
	/// A server up, `appdb` with the extension's objects in it; None - the test skipped - when
	/// YT_LITE_EXT_SO is not set or the server's binaries are not here.
	pub async fn start(tag: &str) -> Option<TmpPg> {
		let Ok(so) = std::env::var("YT_LITE_EXT_SO") else {
			eprintln!("skipped: YT_LITE_EXT_SO names no extension library");
			return None;
		};
		assert!(Path::new(&so).is_file(), "YT_LITE_EXT_SO: no {so}");
		let bin = match std::env::var("PG_BIN") {
			Ok(b) => PathBuf::from(b),
			Err(_) => PathBuf::from(String::from_utf8(Command::new("pg_config").arg("--bindir").output().ok()?.stdout).ok()?.trim()),
		};
		if !bin.join("initdb").is_file() {
			eprintln!("skipped: no initdb in {}", bin.display());
			return None;
		}
		let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() % 1_000_000_000;
		let dir = std::env::temp_dir().join(format!("ytl-pg-{tag}-{}-{n}", std::process::id()));
		// a socket's path has room for 107 bytes: its directory under /tmp, short
		let sockdir = PathBuf::from(format!("/tmp/ytl-{}-{n}", std::process::id()));
		std::fs::create_dir_all(&sockdir).unwrap();
		let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
		let data = dir.join("data");
		let ok = Command::new(bin.join("initdb")).args(["-A", "trust", "-U", "postgres", "-E", "UTF8", "--no-locale", "-D"])
			.arg(&data).output().map(|o| o.status.success()).unwrap_or(false);
		assert!(ok, "initdb in {}", data.display());
		std::fs::write(data.join("postgresql.auto.conf"), format!(
			"port = {port}\nlisten_addresses = ''\nunix_socket_directories = '{}'\nshared_preload_libraries = '{so}'\n\
			 max_prepared_transactions = 20\nfsync = off\nyt.node = '00000000-0000-4000-8000-0000000000e2'\n", sockdir.display())).unwrap();
		let st = Command::new(bin.join("pg_ctl")).args(["-w", "-t", "60", "-D"]).arg(&data).arg("-l").arg(dir.join("log"))
			.arg("start").output().unwrap();
		assert!(st.status.success(), "pg_ctl start: {}", std::fs::read_to_string(dir.join("log")).unwrap_or_default());
		let pg = TmpPg { dir, sockdir, port, bin };
		let c = pg.client("postgres").await;
		c.batch_execute("CREATE DATABASE appdb").await.unwrap();
		pg.client("appdb").await.batch_execute(&script(&so)).await.unwrap();
		Some(pg)
	}

	/// The url of database `db`, over the socket.
	pub fn url(&self, db: &str) -> String {
		format!("postgresql://postgres@localhost/{db}?host={}&port={}", self.sockdir.display(), self.port)
	}

	pub async fn client(&self, db: &str) -> tokio_postgres::Client {
		let (c, conn) = tokio_postgres::connect(&format!("host={} port={} user=postgres dbname={db}", self.sockdir.display(), self.port),
			tokio_postgres::NoTls).await.unwrap();
		tokio::spawn(conn);
		c
	}
}

impl Drop for TmpPg {
	fn drop(&mut self) {
		let _ = Command::new(self.bin.join("pg_ctl")).args(["-m", "immediate", "-D"]).arg(self.dir.join("data")).arg("stop").output();
		let _ = std::fs::remove_dir_all(&self.dir);
		let _ = std::fs::remove_dir_all(&self.sockdir);
	}
}
