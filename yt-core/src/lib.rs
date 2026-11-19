// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! YT Cluster lite: a YT node for one database - beside one PostgreSQL instance
//! (yt-lite), or inside an application. It is a leaf of a YT cluster, one node with no az
//! of its own: it signs in to the cluster above and relays rows down and up over the link
//! (the `core` module).
// [i18n] this crate's messages (locales/, src/i18n.rs). `t!` is the short form its own files
// use; the shared link files spell `rust_i18n::t!` out, as they are compiled into ytserv too
rust_i18n::i18n!("locales", fallback = "en");
#[allow(unused_macros)]
macro_rules! t {
	($($args:tt)*) => { rust_i18n::t!($($args)*).into_owned() }
}
pub mod i18n;

pub mod core;
// a lite node's store: the database its rows live in (PostgreSQL for yt-lite, Turso in an
// application), or none - in memory
pub mod db;
// which of its tables sync, and which way: their A rows, as on a cluster node
#[cfg(any(feature = "service", feature = "lite"))]
pub mod sync;
// the link's turns for a lite node: sign in, down, up (yt-lite's own loop is src/main.rs)
pub mod leaf;

// yt-lite, the service beside one PostgreSQL instance (src/main.rs)
#[cfg(feature = "service")]
pub mod pg;
#[cfg(feature = "service")]
pub mod service;
// the node's CA and its TLS to the cluster above
#[cfg(feature = "tls")]
pub mod tls;
// a lite node's store in a database of the SQLite dialect: Turso's, SQLite's
#[cfg(feature = "lite")]
pub mod lite;
// the lite node inside an application, on Turso
#[cfg(feature = "turso")]
pub mod turso;
// [mysql] the lite node on MySQL
#[cfg(feature = "mysql")]
pub mod mysql;
// [migrate] sge's schemas deployed to the node's own database, then synced
#[cfg(any(feature = "lite-migrate", feature = "mysql", feature = "service"))]
pub mod migrate;
