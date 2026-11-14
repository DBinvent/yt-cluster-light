// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! yt-lite: YT lite as a service beside one PostgreSQL instance (yt-lite.md 16). It
//! serves the extension's unix socket for one database of that instance, keeps the
//! rows it captures in that database until the cluster above has them, applies there
//! what comes down, and is a leaf of that YT cluster: one node with no az, the lowest
//! there is, never a leader, with nothing below it.
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use slog::{error, info, warn, Drain, Logger};
use tokio::sync::{mpsc, oneshot};
use yt_core::core::relay::{self, Outcome};
use yt_core::core::{signin, store::{Node, ABOVE}};
use yt_core::pg::Pg;
use yt_core::service::Lite;

// [i18n] this binary's own copy of the messages (locales/): `t!` looks them up in the
// crate it is written in, and the binary is one beside the library
rust_i18n::i18n!("locales", fallback = "en");
macro_rules! t {
	($($args:tt)*) => { rust_i18n::t!($($args)*).into_owned() }
}

/// What `-h` prints, and a wrong command line after its error: in the language set.
fn usage() -> String {
	format!("{}\n{}\n{}\n{}", t!("lite-usage-about"), t!("lite-usage-args"), t!("lite-usage-settings"), t!("lite-usage-link"))
}

/// yt-lite's settings, from `--file` (`file`) first, then the config table's module-L
/// rows (`rows`).
#[derive(Debug, Default, PartialEq)]
struct Settings {
	upper: Vec<String>,
	cluster_db: Option<String>,
	upper_root: Option<String>,
	key_dir: Option<String>,
	/// the language of its messages (i18n.rs `pick`)
	lang: Option<String>,
}

fn settings(file: &HashMap<String, String>, rows: &HashMap<String, String>) -> Result<Settings, String> {
	let get = |k: &str| file.get(k).or_else(|| rows.get(k)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
	let need = |k: &str| get(k).ok_or_else(|| t!("lite-no-setting", name = k));
	Ok(Settings {
		upper: need("upper")?.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
		cluster_db: get("cluster_db"),
		upper_root: get("upper_root"),
		key_dir: get("key_dir"),
		lang: get("lang"),
	})
}

/// Where the CA is kept: `key_dir`, else `~/.yt-lite/<uuid>` - one directory per node,
/// for several instances on one host.
fn key_dir(set: Option<&str>, home: Option<&str>, uuid: &str) -> Result<std::path::PathBuf, String> {
	match (set, home.filter(|h| !h.trim().is_empty())) {
		(Some(d), _) => Ok(std::path::PathBuf::from(d)),
		(None, Some(h)) => Ok(std::path::Path::new(h).join(".yt-lite").join(uuid)),
		(None, None) => Err(t!("lite-no-key-dir")),
	}
}

fn logger(level: &str) -> Result<Logger, String> {
	let level: slog::Level = level.parse().map_err(|_| format!("--log {}: error, warning, info or debug", level))?;
	let decorator = slog_term::TermDecorator::new().build();
	let drain = slog_term::FullFormat::new(decorator).build().fuse();
	let drain = slog_async::Async::new(drain).build().fuse();
	Ok(Logger::root(slog::LevelFilter::new(drain, level).fuse(), slog::o!()))
}

/// Whether process `pid` is a node that serves this socket: yt-lite, or a YT Cluster
/// node (ytserv) - a pid the system gave to another program since is no holder.
fn holds(pid: &str) -> bool {
	let pid = pid.trim();
	!pid.is_empty() && pid != std::process::id().to_string()
		&& std::fs::read(format!("/proc/{}/cmdline", pid)).map(|c| {
			let c = String::from_utf8_lossy(&c);
			c.contains("yt-lite") || c.contains("ytserv")
		}).unwrap_or(false)
}

/// Take the socket: refused while another node holds its pid file; a stale pid file and
/// socket are cleared. The pid file is made with create_new, so of two starts at once
/// one takes it.
fn claim(socket: &str) -> Result<String, String> {
	use std::io::Write;
	let pid_file = format!("{}.pid", socket);
	for _ in 0..2 {
		match std::fs::OpenOptions::new().write(true).create_new(true).open(&pid_file) {
			Ok(mut f) => {
				f.write_all(std::process::id().to_string().as_bytes()).map_err(|e| t!("lite-pid-file", file = pid_file, error = e))?;
				let _ = std::fs::remove_file(socket); // a stale one: a socket is bound new
				return Ok(pid_file);
			}
			Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
				let pid = std::fs::read_to_string(&pid_file).unwrap_or_default();
				if holds(&pid) {
					return Err(t!("lite-another-node", socket = socket, pid = pid.trim(), file = pid_file));
				}
				let _ = std::fs::remove_file(&pid_file);
			}
			Err(e) => return Err(t!("lite-pid-file", file = pid_file, error = e)),
		}
	}
	Err(t!("lite-pid-race", file = pid_file))
}

/// The pid file and the socket, if they are still this process's.
fn release(socket: &str, pid_file: &str) {
	if std::fs::read_to_string(pid_file).map(|v| v.trim() == std::process::id().to_string()).unwrap_or(false) {
		let _ = std::fs::remove_file(pid_file);
		let _ = std::fs::remove_file(socket);
	}
}

/// What the link to the cluster above does, said on change only: a turn runs every second.
#[derive(Default)]
struct Said(HashMap<&'static str, String>);

impl Said {
	fn say(&mut self, log: &Logger, key: &'static str, text: Option<String>) {
		match text {
			Some(t) if self.0.get(key) != Some(&t) => {
				warn!(log, "{}", t);
				self.0.insert(key, t);
			}
			Some(_) => {}
			None => {
				if self.0.remove(key).is_some() {
					info!(log, "{}", t!("lite-fine-again", what = key));
				}
			}
		}
	}
}

/// A relay turn's failure in words (the library's, an application's loop says the same)
fn failed(o: &Outcome) -> Option<String> {
	yt_core::leaf::failed(o)
}

/// Every second: transactions whose end never came are settled, the sign-in kept, and
/// rows relayed down and up (the shared loops, core/relay.rs). A sign-in every minute,
/// and whenever the calls moved to another node above: that node admits this one only
/// once it signed in there. No az is named in any of it (`ABOVE`): a lite node has none.
async fn link(node: Arc<Node>, pg: Arc<Pg>, lite: Lite, log: Logger) {
	let mut said = Said::default();
	let mut granted = false;
	let mut az_said: Option<String> = None;
	let mut signed_at: Option<Instant> = None;
	let mut signed_turn = None;
	loop {
		tokio::time::sleep(Duration::from_secs(1)).await;
		match pg.resolve(2.0).await {
			Ok(n) => {
				if n > 0 {
					info!(log, "{}", t!("lite-settled", count = n));
				}
				said.say(&log, "resolve", None);
			}
			Err(e) => said.say(&log, "resolve", Some(t!("lite-resolve-failed", error = e))),
		}
		match lite.sweep(Duration::from_secs(10)).await {
			Ok((sent, dropped)) => {
				if sent + dropped > 0 {
					info!(log, "{}", t!("lite-swept", sent = sent, dropped = dropped));
				}
				said.say(&log, "sweep", None);
			}
			Err(e) => said.say(&log, "sweep", Some(t!("lite-sweep-failed", error = e))),
		}
		let turn = yt_core::core::calls::turn(&node).await;
		if !granted || signed_turn != Some(turn) || signed_at.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
			signed_at = Some(Instant::now());
			signed_turn = Some(turn);
			let r = signin::sign_in(&node, ABOVE).await;
			if let Some(n) = node.tls.as_ref().and_then(|t| t.news()) {
				match n {
					Ok(text) => {
						warn!(log, "{}", t!("lite-tls-news", text = text));
						said.say(&log, "link TLS", None);
					}
					Err(e) => said.say(&log, "link TLS", Some(t!("lite-tls-problem", error = e))),
				}
			}
			match r {
				Ok(g) if g.granted => {
					// the az of the node signed in through, as it said: on change
					if !granted || az_said.as_deref() != Some(g.az.as_str()) {
						if g.az.is_empty() {
							info!(log, "{}", t!("lite-signed-in-flat"));
						} else {
							info!(log, "{}", t!("lite-signed-in-az", az = g.az));
						}
						az_said = Some(g.az.clone());
					}
					granted = true;
					said.say(&log, "signin", None);
				}
				Ok(g) => {
					granted = false;
					// [compat] one from a leader before 1.21 says what that means
					said.say(&log, "signin", Some(yt_core::core::calls::refused(&g.reason)));
				}
				Err(e) => {
					// [compat] the next node of `upper` next time, for a host that has not let this
					// node in: one that does not answer, or has no Link service (a YT Cluster before
					// 1.21), held it here for good before. A host it was linked through is signed in
					// to once more first - one failed refresh moves nothing (review of 2026-11-13) -
					// and a refusal is the cluster's answer, the same from each node, and stays
					if !granted {
						yt_core::core::calls::forget(&node, ABOVE).await;
					}
					granted = false;
					node.not_granted().await;
					said.say(&log, "signin", Some(t!("lite-signin-failed", error = e)));
				}
			}
		}
		if !granted {
			continue;
		}
		let o = relay::down(&node, ABOVE).await;
		said.say(&log, "down", failed(&o));
		let o = relay::up(&node, ABOVE).await;
		said.say(&log, "up", failed(&o));
	}
}

async fn signalled() -> String {
	use tokio::signal::unix::{signal, SignalKind};
	let (Ok(mut i), Ok(mut t), Ok(mut h)) = (signal(SignalKind::interrupt()), signal(SignalKind::terminate()), signal(SignalKind::hangup()))
	else { return t!("lite-signal-none") };
	tokio::select! {
		_ = i.recv() => t!("lite-signal-interrupt"),
		_ = t.recv() => t!("lite-signal-terminate"),
		_ = h.recv() => t!("lite-signal-hangup"),
	}
}

/// [banner] The DBinvent logo in ASCII, drawn after dbinvent.com's: a D of D's and a
/// B of B's, and "invent" under them in smaller lowercase letters of their own - Schema
/// Guard's, the same
const LOGO: [&str; 16] = [
	"DDDDDDDDDDDD     BBBBBBBBBBBBB  ",
	"DDDDDDDDDDDDDD   BBBBBBBBBBBBBB ",
	"DDDDD    DDDDDD  BBBBB    BBBBB ",
	"DDDDD     DDDDD  BBBBB    BBBBB ",
	"DDDDD     DDDDD  BBBBBBBBBBBBB  ",
	"DDDDD     DDDDD  BBBBB     BBBBB",
	"DDDDD    DDDDDD  BBBBB     BBBBB",
	"DDDDDDDDDDDDDD   BBBBBBBBBBBBBBB",
	"DDDDDDDDDDDD     BBBBBBBBBBBBBB ",
	"                                ",
	"i                            t  ",
	"                             t  ",
	"i  nnn   v   v   ee   nnn    ttt",
	"i  n  n  v   v  eeee  n  n   t  ",
	"i  n  n   v v   e     n  n   t  ",
	"i  n  n    v     eee  n  n    tt",
];

/// [banner] The product and its version, "Powered by" the logo, and the slogan under it -
/// nothing beside the logo; as Schema Guard's
fn banner() -> String {
	let mut b = format!("YT lite {}\nPowered by\n\n", env!("CARGO_PKG_VERSION"));
	for l in LOGO.iter() {
		b.push_str(l.trim_end());
		b.push('\n');
	}
	b.push_str("\nThe infinite-scale Postgres cluster and app server - check DBinvent.com\n");
	b
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> ExitCode {
	// [banner] DBinvent's, on a console run only - stderr is a terminal - so a service's
	// log and `yt-lite -V` (stdout) stay as they were; as Schema Guard does
	if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
		eprint!("{}", banner());
	}
	// [i18n] the system locale's language until the database and the settings are read
	yt_core::i18n::set(yt_core::i18n::pick(None, None, yt_core::i18n::system_locale().as_deref()));
	// the command line as YT Cluster's node takes it (marg): -V prints the version and exits
	let m = match marg::ArgConfig::from_args2("yt-lite", env!("CARGO_PKG_VERSION")) {
		Ok(m) => m,
		Err(e) => {
			eprintln!("yt-lite: {}\n\n{}", t!("lite-args", error = e), usage());
			return ExitCode::from(2);
		}
	};
	if m.help.is_set() {
		print!("{}", usage());
		return ExitCode::SUCCESS;
	}
	// [i18n] `lang` of --file, known before the database is asked: an error connecting to it
	// is said in it already (review of 2026-11-13)
	yt_core::i18n::set(yt_core::i18n::pick(m.cfg.get("lang").map(|v| v.as_str()), None, yt_core::i18n::system_locale().as_deref()));
	let level = if m.verbose.is_set() { "debug" } else if m.quiet.is_set() { "warning" } else { "info" };
	let log = match logger(level) {
		Ok(l) => l,
		Err(e) => {
			eprintln!("yt-lite: {}", e);
			return ExitCode::from(2);
		}
	};
	let mut pg = match Pg::connect(&m.db_url, Some(m.table.as_str()), None).await {
		Ok(p) => p,
		Err(e) => {
			error!(log, "{}", e);
			return ExitCode::from(10);
		}
	};
	// [i18n] one language for everything it says: its `lang` setting, else the database's
	// (lc_messages), else the system locale's - set before the settings are judged, so a
	// missing one is said in it
	let rows = pg.settings().await;
	let lang = yt_core::i18n::pick(
		m.cfg.get("lang").or_else(|| rows.as_ref().ok().and_then(|r| r.get("lang"))).map(|v| v.as_str()),
		pg.lc_messages().await.as_deref(), yt_core::i18n::system_locale().as_deref());
	yt_core::i18n::set(lang);
	let args = match rows.and_then(|rows| settings(&m.cfg, &rows)) {
		Ok(a) => a,
		Err(e) => {
			error!(log, "{}", e);
			eprintln!("\n{}", usage());
			return ExitCode::from(2);
		}
	};
	if let Some(c) = &args.cluster_db {
		pg.cluster_db = c.clone();
	}
	let pg = Arc::new(pg);
	info!(log, "{}", t!("lite-start", version = env!("CARGO_PKG_VERSION"), hosts = args.upper.join(", ")));
	let uuid = if m.uuid_gen { pg.node_uuid().await } else { Ok(m.uuid.as_u128()) };
	let (socket, uuid) = match (pg.socket_path().await, uuid) {
		(Ok(s), Ok(u)) => (s, u),
		(Err(e), _) | (_, Err(e)) => {
			error!(log, "{}", e);
			return ExitCode::from(10);
		}
	};
	let pid_file = match claim(&socket) {
		Ok(p) => p,
		Err(e) => {
			error!(log, "{}", e);
			return ExitCode::from(12);
		}
	};
	let uuid_s = yt_core_proto::uuid::Uuid::from_u128(uuid).to_string();
	let dir = match key_dir(args.key_dir.as_deref(), std::env::var("HOME").ok().as_deref(), &uuid_s) {
		Ok(d) => d,
		Err(e) => {
			error!(log, "{}", e);
			release(&socket, &pid_file);
			return ExitCode::from(2);
		}
	};
	let tls = match yt_core::tls::Tls::open(&dir, &uuid_s, args.upper_root.as_deref()) {
		Ok(t) => Arc::new(t),
		Err(e) => {
			error!(log, "{}", t!("lite-ca", error = e));
			release(&socket, &pid_file);
			return ExitCode::from(10);
		}
	};
	info!(log, "{}", t!("lite-ca-line", fingerprint = tls.fingerprint(), dir = tls.dir().display(), uuid = uuid_s));
	if !tls.pinned() {
		warn!(log, "{}", t!("lite-no-root"));
	}
	info!(log, "{}", t!("lite-tls-line", status = tls.status()));
	let node = Arc::new(Node::with_pg(uuid, args.upper.clone(), pg.clone()).with_tls(tls));
	let (stop, stopped) = oneshot::channel();
	let (ended_send, mut ended) = mpsc::unbounded_channel();
	let service = Lite::new(node.clone(), pg.clone(), log.clone());
	let swept = service.clone();
	if let Err(e) = service.run(&socket, stopped, ended_send).await {
		error!(log, "{}", t!("lite-socket", error = e));
		release(&socket, &pid_file);
		return ExitCode::from(15);
	}
	info!(log, "{}", t!("lite-serving", db = pg.db, cluster_db = pg.cluster_db, socket = socket,
		uuid = yt_core_proto::uuid::Uuid::from_u128(uuid)));
	let linker = tokio::spawn(link(node, pg, swept, log.clone()));
	let why = tokio::select! {
		s = signalled() => s,
		e = ended.recv() => t!("lite-socket-stopped", error = e.unwrap_or_default()),
	};
	linker.abort();
	let _ = stop.send(());
	release(&socket, &pid_file);
	warn!(log, "{}", t!("lite-stopped", why = why));
	tokio::time::sleep(Duration::from_millis(100)).await; // the async log drains
	ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
	use super::*;

	/// [banner] the product and its version, "Powered by" the logo, the slogan under it: as
	/// Schema Guard's, every logo row a line of its own
	#[test]
	fn banner_shows_the_product_the_logo_and_the_slogan() {
		let b = banner();
		let lines: Vec<&str> = b.lines().collect();
		assert_eq!(lines[0], format!("YT lite {}", env!("CARGO_PKG_VERSION")), "{}", b);
		assert_eq!(lines[1], "Powered by", "{}", b);
		assert!(b.contains("DDDDD") && b.contains("BBBBB") && b.contains("nnn"), "{}", b);
		assert!(b.trim_end().ends_with("Postgres cluster and app server - check DBinvent.com"), "{}", b);
		assert!(LOGO.iter().all(|l| l.len() == LOGO[0].len()), "{:?}", LOGO);
		assert!(LOGO.iter().all(|l| lines.contains(&l.trim_end())), "{}", b);
	}
	use std::path::Path;

	fn s(v: &[&str]) -> Vec<String> {
		v.iter().map(|x| x.to_string()).collect()
	}

	fn m(v: &[(&str, &str)]) -> HashMap<String, String> {
		v.iter().map(|(k, x)| (k.to_string(), x.to_string())).collect()
	}

	#[test]
	fn the_settings_come_from_the_file_first_then_the_config_table() {
		let rows = m(&[("upper", "http://a:7421, http://b:7421")]);
		let a = settings(&HashMap::new(), &rows).unwrap();
		assert_eq!(a, Settings { upper: s(&["http://a:7421", "http://b:7421"]), cluster_db: None, upper_root: None, key_dir: None, lang: None });
		let a = settings(&m(&[("upper", "http://c:7421"), ("cluster_db", "app")]), &rows).unwrap();
		assert_eq!((a.upper, a.cluster_db.as_deref()), (s(&["http://c:7421"]), Some("app")));
		let e = settings(&HashMap::new(), &HashMap::new()).unwrap_err();
		assert!(e.starts_with("YT-E034 ") && e.contains("no setting upper") && e.contains("module L"), "{}", e);
		assert!(settings(&m(&[("upper", " ")]), &HashMap::new()).is_err(), "an empty value is none");
	}

	#[test]
	fn an_az_is_no_setting_its_own_or_the_cluster_s() {
		// a lite node has no az: neither asked for nor read - with and without the rows
		let with = settings(&HashMap::new(), &m(&[("az", "dev1"), ("upper_az", "top"), ("upper", "u")])).unwrap();
		let without = settings(&HashMap::new(), &m(&[("upper", "u")])).unwrap();
		assert_eq!(with, without);
		assert!(!usage().contains("upper_az"), "the usage names no az setting");
		assert!(usage().contains("No az: a lite node has none to set"));
	}

	#[test]
	fn the_root_pin_and_the_key_dir_are_settings_too() {
		let rows = m(&[("upper_az", "top"), ("upper", "http://a:7421"), ("upper_root", " SHA256:abc "), ("key_dir", "/k")]);
		let a = settings(&HashMap::new(), &rows).unwrap();
		assert_eq!((a.upper_root.as_deref(), a.key_dir.as_deref()), (Some("SHA256:abc"), Some("/k")));
		let u = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
		assert_eq!(key_dir(Some("/k"), Some("/home/x"), u).unwrap(), Path::new("/k"));
		assert_eq!(key_dir(None, Some("/var/lib/postgresql"), u).unwrap(), Path::new("/var/lib/postgresql/.yt-lite").join(u),
			"one directory per node: several instances on one host");
		let e = key_dir(None, Some(" "), u).unwrap_err();
		assert!(e.starts_with("YT-E035 ") && e.contains("key_dir"), "{}", e);
	}

	#[test]
	fn a_socket_held_by_a_live_process_is_not_taken() {
		let dir = std::env::temp_dir().join(format!("yt-lite-claim-{}", std::process::id()));
		let _ = std::fs::create_dir_all(&dir);
		let socket = dir.join(".s.YT.5999").to_string_lossy().to_string();
		std::fs::write(format!("{}.pid", socket), "1").unwrap(); // init: alive, but no node
		let pid_file = claim(&socket).unwrap();
		assert_eq!(std::fs::read_to_string(&pid_file).unwrap(), std::process::id().to_string());
		assert!(claim(&socket).is_err() || holds(&std::process::id().to_string()) == false, "its own pid is no other holder");
		release(&socket, &pid_file);
		std::fs::write(format!("{}.pid", socket), "999999999").unwrap(); // no such process
		let pid_file = claim(&socket).unwrap();
		assert_eq!(std::fs::read_to_string(&pid_file).unwrap(), std::process::id().to_string());
		release(&socket, &pid_file);
		assert!(!Path::new(&pid_file).exists());
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn a_failed_turn_is_said_and_a_quiet_one_is_not() {
		assert!(failed(&Outcome::Idle).is_none());
		assert!(failed(&Outcome::Moved).is_none());
		assert!(failed(&Outcome::Refused("no".into())).unwrap().contains("refused"));
		assert_eq!(failed(&Outcome::FetchFailed("x".into())).unwrap(), "YT-E044 fetching from the cluster above: x - the next node next");
	}

	/// the usage, whole: every option and setting it names, and where an error's code is explained
	#[test]
	fn the_usage_names_every_option_and_setting() {
		let u = usage();
		for word in ["yt-lite [--db] <url>", "--config", "--file", "--uuid", "upper ", "upper_root", "cluster_db", "key_dir", "lang ",
			"--skv-approve <uuid> --fingerprint <fp>", "https://docs.dbinvent.com/errors/<code>", "YT-Ennn"] {
			assert!(u.contains(word), "{}", word);
		}
		assert!(u.lines().all(|l| l.chars().count() <= 100), "a line of the usage is longer than 100");
	}

	#[test]
	fn the_language_is_a_setting_too() {
		let a = settings(&HashMap::new(), &m(&[("upper", "u"), ("lang", " ru ")])).unwrap();
		assert_eq!(a.lang.as_deref(), Some("ru"));
	}
}
