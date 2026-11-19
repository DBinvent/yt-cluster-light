// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! [mysql] yt-lite-mysql: the lite node beside one MySQL database (owner, 2026-11-19) - crate
//! yt_core::mysql's store on it, linked to the cluster above as yt-lite links.
//!
//!   yt-lite-mysql <url> --uuid <uuid> --upper <host:port,...> --cluster-db <name>
//!                 [--root <SHA256:...>] [--key-dir <dir>] [--migrate <dir>] [--sync <schema.table,...>]
//!   yt-lite-mysql <url> --migrate <dir> --once
//!
//! - `<url>`: `mysql://user:password@host:3306/database` - the database the node serves.
//! - `--migrate <dir>`: sge's schemas and scripts there deployed first, mapped from the
//!   cluster's PostgreSQL, and every table they name with a primary key set up to sync
//!   (yt_core::migrate). Run it again after a schema change. With `--once`, that alone.
//! - `--sync`: tables synced with no capture here: an A row, both ways, no triggers - their rows
//!   come down; writes here go up only for a table `--migrate` set up.
//! - The node's key and certificate in `--key-dir` (`~/.yt-lite/<uuid>` by default); its CA's
//!   fingerprint is said at the start, to approve on the cluster's leader:
//!   `ytctl --skv-approve <uuid> --fingerprint <fp>`.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use yt_core::core::store::Node;
use yt_core::leaf::{self, Status};
use yt_core::mysql::Mysql;

fn args() -> Result<(String, BTreeMap<String, String>), String> {
	let mut a = std::env::args().skip(1);
	let mut url = String::new();
	let mut m = BTreeMap::new();
	while let Some(k) = a.next() {
		match k.strip_prefix("--") {
			Some("once") => { m.insert("once".to_string(), "1".to_string()); }
			Some("help") | Some("version") => { m.insert(k[2..].to_string(), "1".to_string()); }
			// [review 2026-11-19] a lone `--` or an option given twice: said, not taken
			Some("") => return Err("`--` alone: an option's name after it".to_string()),
			Some(k) if m.contains_key(k) => return Err(format!("--{} given twice", k)),
			Some(k) => { m.insert(k.to_string(), a.next().ok_or_else(|| format!("--{} needs a value", k))?); }
			None if url.is_empty() => url = k,
			None => return Err(format!("`{}`: one database URL only", k)),
		}
	}
	Ok((url, m))
}

fn usage() -> &'static str {
	"yt-lite-mysql <mysql://user:password@host:3306/database> --uuid <uuid> --upper <host:port,...> --cluster-db <name>\n\
	 \x20             [--root <SHA256:...>] [--key-dir <dir>] [--migrate <dir>] [--sync <schema.table,...>]\n\
	 yt-lite-mysql <url> --migrate <dir> --once"
}

fn say(s: &str) {
	println!("{}", s);
}

async fn run(url: &str, m: &BTreeMap<String, String>) -> Result<(), String> {
	let need = |k: &str| m.get(k).cloned().ok_or_else(|| format!("--{} is missing\n{}", k, usage()));
	let cluster_db = if m.contains_key("once") { m.get("cluster-db").cloned().unwrap_or_default() } else { need("cluster-db")? };
	// [review 2026-11-19] the link's settings checked before anything is written
	let link = if m.contains_key("once") { None } else {
		let uuid = yt_core_proto::uuid::Uuid::parse_str(&need("uuid")?).map_err(|e| format!("--uuid: {}", e))?;
		let hosts: Vec<String> = need("upper")?.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
		if hosts.is_empty() {
			return Err(format!("--upper names no node\n{}", usage()));
		}
		Some((uuid, hosts))
	};
	let store = Arc::new(Mysql::open(url, &cluster_db).await?);
	if let Some(dir) = m.get("migrate") {
		let r = store.migrate_dir(&*store, &PathBuf::from(dir)).await?;
		for (f, what) in &r.ran {
			say(&format!("{}: {}", f, what));
		}
		for n in &r.notes {
			say(&format!("note: {}", n));
		}
		if !r.synced.is_empty() {
			say(&format!("synced: {}", r.synced.join(", ")));
		}
	}
	if m.contains_key("once") {
		return Ok(());
	}
	for t in m.get("sync").map(|s| s.split(',').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect::<Vec<_>>()).unwrap_or_default() {
		store.sync(&t, "").await?;
	}
	let Some((uuid, hosts)) = link else { return Ok(()) };
	let uuid_s = uuid.to_string();
	let key_dir = m.get("key-dir").map(PathBuf::from)
		.unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".yt-lite").join(&uuid_s));
	let tls = Arc::new(yt_core::tls::Tls::open(&key_dir, &uuid_s, m.get("root").map(|s| s.as_str()))?);
	say(&format!("fingerprint {}", tls.fingerprint()));
	say(&format!("approve on the cluster's leader: ytctl --skv-approve {} --fingerprint {}", uuid_s, tls.fingerprint()));
	let node = Arc::new(Node::with_db(uuid.as_u128(), hosts, store.clone()).with_tls(tls));
	let status = Arc::new(Mutex::new(Status::default()));
	leaf::run(node, status, |s| say(&s)).await;
	Ok(())
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> std::process::ExitCode {
	let (url, m) = match args() {
		Ok(a) => a,
		Err(e) => { eprintln!("yt-lite-mysql: {}\n{}", e, usage()); return std::process::ExitCode::from(2) }
	};
	if m.contains_key("version") {
		println!("yt-lite-mysql {}", env!("CARGO_PKG_VERSION"));
		return std::process::ExitCode::SUCCESS;
	}
	if m.contains_key("help") || url.is_empty() {
		println!("{}", usage());
		return if url.is_empty() && !m.contains_key("help") { std::process::ExitCode::from(2) } else { std::process::ExitCode::SUCCESS };
	}
	match run(&url, &m).await {
		Ok(()) => std::process::ExitCode::SUCCESS,
		Err(e) => { eprintln!("yt-lite-mysql: {}", e); std::process::ExitCode::from(1) }
	}
}
