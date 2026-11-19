// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! [migrate] A directory of sge's schemas deployed to the lite node's own database, then synced
//! (owner, 2026-11-19: "yt-lite supports sge 2.4, cross db migrate and cluster sync").
//!
//! - **The files,** as sge names and orders them: `S<version>__name.yaml` (or `.yml`) a YAML
//!   schema, `V<version>__name.sql` a script run once, `R__name.sql` one run again when it
//!   changed - by version, schemas before scripts of the same version, the repeatable ones
//!   last. Each is kept in `yt_migrations` (its name, its MD5): a schema is applied again when
//!   it changed, a script that changed after it ran is refused, as sge refuses it.
//! - **Cross database.** A schema is the cluster's, PostgreSQL's (its `source`, none being
//!   PostgreSQL's). For this node's database it goes through schema_guard_tokio's mapping
//!   first - for SQLite (Turso), for MySQL - and each column's PostgreSQL type is kept in
//!   `yt_columns`, so its values go up with the type the cluster has (crate::lite). Its custom
//!   SQL is left out there, each one said.
//! - **Names.** A table of schema `s` is the local table `s.t` here (crate::lite's names), a
//!   table of `public` its name alone.
//! - **Sync.** Each table the schemas name (a template not) with a primary key is set up as
//!   `setup` sets one up: its capture triggers and its A row - both ways. One with no key is
//!   said, not synced. A schema change is followed: run the migration again after one.
//! - **Scripts** run on this node's connection, in the database's own SQL.
use std::path::{Path, PathBuf};

#[cfg(feature = "lite")]
use schema_guard_tokio::mapping::Mapping;
use schema_guard_tokio::mapping::Source;
use yaml_rust2::Yaml;

#[cfg(feature = "lite")]
use crate::lite::{Conn, Lite, Val};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
	S,
	V,
	R,
}

/// a file of the directory
#[derive(Debug, Clone)]
pub struct File {
	pub kind: Kind,
	pub version: Vec<u64>,
	pub name: String,
	pub path: PathBuf,
	pub checksum: String,
}

/// What a migration did
#[derive(Debug, Default, Clone)]
pub struct Report {
	/// each file run, with what it did
	pub ran: Vec<(String, String)>,
	/// the tables set up to sync
	pub synced: Vec<String>,
	/// what the mapping left out or changed, a table left unsynced
	pub notes: Vec<String>,
}

fn version_of(text: &str) -> Result<Vec<u64>, String> {
	if text.is_empty() {
		return Ok(vec![0]);
	}
	text.split(|c| c == '.' || c == '_').map(|p| p.parse::<u64>().map_err(|_| format!("`{}` is no version", text))).collect()
}

/// a file's MD5, base64, as sge writes it
pub fn checksum(bytes: &[u8]) -> String {
	use base64::Engine;
	let digest = md5_digest(bytes);
	base64::engine::general_purpose::STANDARD.encode(digest)
}

/// MD5 (RFC 1321): sge's checksum, with no crate for it here
fn md5_digest(input: &[u8]) -> [u8; 16] {
	let s: [u32; 64] = [7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
		4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21];
	let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32).collect();
	let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
	let mut msg = input.to_vec();
	let bits = (input.len() as u64).wrapping_mul(8);
	msg.push(0x80);
	while msg.len() % 64 != 56 {
		msg.push(0);
	}
	msg.extend_from_slice(&bits.to_le_bytes());
	for chunk in msg.chunks(64) {
		let m: Vec<u32> = (0..16).map(|i| u32::from_le_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]])).collect();
		let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
		for i in 0..64 {
			let (f, g) = match i / 16 {
				0 => ((b & c) | (!b & d), i),
				1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
				2 => (b ^ c ^ d, (3 * i + 5) % 16),
				_ => (c ^ (b | !d), (7 * i) % 16),
			};
			let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
			a = d;
			d = c;
			c = b;
			b = b.wrapping_add(f.rotate_left(s[i]));
		}
		a0 = a0.wrapping_add(a);
		b0 = b0.wrapping_add(b);
		c0 = c0.wrapping_add(c);
		d0 = d0.wrapping_add(d);
	}
	let mut out = [0u8; 16];
	for (i, v) in [a0, b0, c0, d0].iter().enumerate() {
		out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
	}
	out
}

/// The files of a directory, in sge's order
pub fn files(dir: &Path) -> Result<Vec<File>, String> {
	let mut names: Vec<String> = std::fs::read_dir(dir).map_err(|x| format!("{}: {}", dir.display(), x))?
		.filter_map(|e| e.ok()).filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
	names.sort();
	let mut out = Vec::new();
	for name in names {
		let low = name.to_lowercase();
		let kind = if name.starts_with('S') && (low.ends_with(".yaml") || low.ends_with(".yml")) { Kind::S }
			else if name.starts_with('V') && low.ends_with(".sql") { Kind::V }
			else if name.starts_with("R__") && low.ends_with(".sql") { Kind::R }
			else { continue };
		let Some(i) = name[1..].find("__") else { continue };
		let version = version_of(&name[1..1 + i]).map_err(|x| format!("{}: {}", name, x))?;
		let path = dir.join(&name);
		let bytes = std::fs::read(&path).map_err(|x| format!("{}: {}", path.display(), x))?;
		out.push(File { kind, version, name, path, checksum: checksum(&bytes) });
	}
	out.sort_by(|a, b| (a.kind == Kind::R).cmp(&(b.kind == Kind::R)).then(a.version.cmp(&b.version))
		.then(a.kind.cmp(&b.kind)).then(a.name.cmp(&b.name)));
	Ok(out)
}

/// The name of a cluster table on a lite node of the SQLite dialect or MySQL: `schema.table`,
/// `public`'s by its name alone (crate::lite's)
pub fn local_name(schema: &str, table: &str) -> String {
	if schema == "public" { table.to_string() } else { format!("{}.{}", schema, table) }
}

/// A schema's tables of a schema other than `public` as this node names them (`s.t`), each
/// schema `public`: crate::lite's names, in one database
pub fn local_names(y: &Yaml) -> Yaml {
	let mut out = y.clone();
	if let Yaml::Hash(h) = &mut out {
		if let Some(Yaml::Array(dbs)) = h.get_mut(&Yaml::String("database".into())) {
			for s in dbs.iter_mut() {
				let Yaml::Hash(sh) = s else { continue };
				let name = sh.get(&Yaml::String("schemaName".into())).and_then(|v| v.as_str()).unwrap_or("public").to_string();
				if name == "public" {
					continue;
				}
				sh.insert(Yaml::String("schemaName".into()), Yaml::String("public".into()));
				if let Some(Yaml::Array(tables)) = sh.get_mut(&Yaml::String("tables".into())) {
					for t in tables.iter_mut() {
						let Yaml::Hash(outer) = t else { continue };
						let Some(Yaml::Hash(th)) = outer.get_mut(&Yaml::String("table".into())) else { continue };
						if let Some(tn) = th.get(&Yaml::String("tableName".into())).and_then(|v| v.as_str()).map(String::from) {
							th.insert(Yaml::String("tableName".into()), Yaml::String(local_name(&name, &tn)));
						}
					}
				}
			}
		}
	}
	out
}

/// Each table of a schema (not a template): (its local name, its columns' (name, PostgreSQL type),
/// whether it has a primary key)
pub fn tables(y: &Yaml, file: &str) -> Result<Vec<(String, Vec<(String, String)>, bool)>, String> {
	let parsed = schema_guard_tokio::parse_yaml_schema(y.clone(), file)?;
	let mut out = Vec::new();
	for s in &parsed.list {
		for t in s.tables.list.iter().filter(|t| t.is_template != Some(true)) {
			let local = local_name(&s.schema_name, &t.table_name);
			let cols = t.columns.list.iter().map(|c| (c.name.clone(), c.column_type.trim().to_lowercase())).collect();
			let pk = t.columns.list.iter().any(|c| c.constraint.as_ref().and_then(|k| k.primary_key).unwrap_or(false));
			out.push((local, cols, pk));
		}
	}
	Ok(out)
}

/// A store a migration deploys to: its database's source and its deploy of a YAML schema
#[cfg(feature = "lite")]
#[tonic::async_trait]
pub trait Target: Send + Sync {
	/// this database's vendor and version, as a schema's `source` names one
	async fn source(&self) -> Result<Source, String>;
	/// a YAML schema (already this database's) deployed: its statements
	async fn deploy(&self, y: Yaml, file: &str) -> Result<Vec<String>, String>;
}

#[cfg(feature = "lite")]
impl<C: Conn> Lite<C> {
	/// [migrate] The schemas and scripts of `dir` deployed to this database, then each table
	/// they name set up to sync (crate::migrate)
	pub async fn migrate_dir<T: Target + ?Sized>(&self, target: &T, dir: &Path) -> Result<Report, String> {
		let mut report = Report::default();
		let to = target.source().await?;
		{
			let c = self.conn().await;
			let ddl = match c.dialect() {
				crate::lite::Dialect::Sqlite => "CREATE TABLE IF NOT EXISTS yt_migrations (name TEXT PRIMARY KEY, checksum TEXT NOT NULL)",
				crate::lite::Dialect::Mysql => "CREATE TABLE IF NOT EXISTS yt_migrations (name VARCHAR(255) PRIMARY KEY, checksum VARCHAR(64) NOT NULL)",
			};
			c.batch(ddl).await.map_err(crate::lite::e("yt_migrations"))?;
		}
		let ran: std::collections::HashMap<String, String> = {
			let c = self.conn().await;
			c.rows("SELECT name, checksum FROM yt_migrations", vec![]).await.map_err(crate::lite::e("yt_migrations"))?
				.iter().map(|r| (crate::lite::text(&r[0]).unwrap_or_default(), crate::lite::text(&r[1]).unwrap_or_default())).collect()
		};
		let mut to_sync = Vec::new();
		for f in files(dir)? {
			let before = ran.get(&f.name);
			if before == Some(&f.checksum) {
				continue;
			}
			if f.kind == Kind::V && before.is_some() {
				return Err(format!("{}: changed after it ran - a versioned script runs once; write a new version", f.name));
			}
			let what = match f.kind {
				Kind::S => {
					let text = std::fs::read_to_string(&f.path).map_err(|x| format!("{}: {}", f.name, x))?;
					let y = schema_guard_tokio::load_schema_from_src(text).map_err(|x| format!("{}: {}", f.name, x))?;
					let from = Source::of(&y)?;
					let named = local_names(&y);
					let mapped = if from.vendor == to.vendor { named.clone() } else {
						let m = Mapping::builtin(&from.vendor, &to.vendor)
							.ok_or_else(|| format!("{}: no mapping from {} to {}", f.name, from.vendor, to.vendor))?
							.map_err(|x| format!("{}: {}", f.name, x))?;
						let out = m.apply(&named, &to).map_err(|r| format!("{}: refused for {}:\n  {}", f.name, to.name(), r.join("\n  ")))?;
						report.notes.extend(out.notes.iter().map(|n| format!("{}: {}", f.name, n)));
						out.schema
					};
					let st = target.deploy(mapped, &f.name).await.map_err(|x| format!("{}: {}", f.name, x))?;
					// the cluster's types of each column, where the mapping changed them
					let c = self.conn().await;
					let d = c.dialect();
					for (local, cols, pk) in tables(&named, &f.name)? {
						if from.vendor != to.vendor {
							for (col, pg) in &cols {
								c.exec(&format!("INSERT INTO yt_columns (tbl, col, pg) VALUES (?, ?, ?){}", d_upsert(d)),
									vec![Val::Text(local.clone()), Val::Text(col.clone()), Val::Text(pg.clone())]).await.map_err(crate::lite::e("yt_columns"))?;
							}
						}
						if pk { to_sync.push(local) } else { report.notes.push(format!("{}: {}: no primary key - not synced", f.name, local)) }
					}
					format!("{} statement(s)", st.len())
				}
				Kind::V | Kind::R => {
					let sql = std::fs::read_to_string(&f.path).map_err(|x| format!("{}: {}", f.name, x))?;
					self.conn().await.batch(&sql).await.map_err(|x| format!("{}: {}", f.name, x))?;
					"ran".to_string()
				}
			};
			{
				let c = self.conn().await;
				let d = c.dialect();
				c.exec(&format!("INSERT INTO yt_migrations (name, checksum) VALUES (?, ?){}", match d {
					crate::lite::Dialect::Sqlite => " ON CONFLICT (name) DO UPDATE SET checksum = excluded.checksum",
					crate::lite::Dialect::Mysql => " ON DUPLICATE KEY UPDATE checksum = VALUES(checksum)",
				}), vec![Val::Text(f.name.clone()), Val::Text(f.checksum.clone())]).await.map_err(crate::lite::e("yt_migrations"))?;
			}
			report.ran.push((f.name.clone(), what));
		}
		to_sync.dedup();
		for t in to_sync {
			self.setup(&t).await?;
			report.synced.push(t);
		}
		Ok(report)
	}
}

#[cfg(feature = "lite")]
fn d_upsert(d: crate::lite::Dialect) -> &'static str {
	match d {
		crate::lite::Dialect::Sqlite => " ON CONFLICT (tbl, col) DO UPDATE SET pg = excluded.pg",
		crate::lite::Dialect::Mysql => " ON DUPLICATE KEY UPDATE pg = VALUES(pg)",
	}
}


/// [migrate] yt-lite's: the schemas and scripts of `dir` deployed to the PostgreSQL database it
/// serves (schema_guard_tokio, in one transaction a file), then each table they name with a
/// primary key set up to sync with the extension's `yt_setup('schema.table')`; `yt_migrations` in
/// `schema`, the config table's. Its `migrate` setting runs it at the start.
#[cfg(feature = "service")]
pub async fn migrate_pg(url: &str, schema: &str, dir: &Path) -> Result<Report, String> {
	use std::str::FromStr;
	let cfg = tokio_postgres::Config::from_str(url).map_err(|x| x.to_string())?;
	let (mut client, conn) = cfg.connect(tokio_postgres::NoTls).await.map_err(|x| x.to_string())?;
	tokio::spawn(async move { let _ = conn.await; });
	let hist = format!("\"{}\".yt_migrations", schema.replace('"', "\"\""));
	client.batch_execute(&format!("CREATE TABLE IF NOT EXISTS {} (name text PRIMARY KEY, checksum text NOT NULL)", hist))
		.await.map_err(|x| format!("yt_migrations: {}", x))?;
	let ran: std::collections::HashMap<String, String> = client.query(&format!("SELECT name, checksum FROM {}", hist), &[]).await
		.map_err(|x| x.to_string())?.iter().map(|r| (r.get(0), r.get(1))).collect();
	let mut report = Report::default();
	let mut to_sync: Vec<String> = Vec::new();
	for f in files(dir)? {
		let before = ran.get(&f.name);
		if before == Some(&f.checksum) {
			continue;
		}
		if f.kind == Kind::V && before.is_some() {
			return Err(format!("{}: changed after it ran - a versioned script runs once; write a new version", f.name));
		}
		let tx = client.transaction().await.map_err(|x| x.to_string())?;
		let what = match f.kind {
			Kind::S => {
				let text = std::fs::read_to_string(&f.path).map_err(|x| format!("{}: {}", f.name, x))?;
				let y = schema_guard_tokio::load_schema_from_src(text).map_err(|x| format!("{}: {}", f.name, x))?;
				let from = Source::of(&y)?;
				if from.vendor != "postgres" {
					return Err(format!("{}: a schema of {} - yt-lite's database is PostgreSQL; map it first", f.name, from.vendor));
				}
				let mut tx = tx;
				let n = schema_guard_tokio::migrate(y.clone(), &mut tx, None, &f.name, &Default::default()).await
					.map_err(|x| format!("{}: {}", f.name, x))?;
				for (s_name, t_name, pk) in pg_tables(&y, &f.name)? {
					if pk { to_sync.push(format!("{}.{}", s_name, t_name)) } else { report.notes.push(format!("{}: {}.{}: no primary key - not synced", f.name, s_name, t_name)) }
				}
				tx.execute(&format!("INSERT INTO {} (name, checksum) VALUES ($1, $2) ON CONFLICT (name) DO UPDATE SET checksum = excluded.checksum", hist),
					&[&f.name, &f.checksum]).await.map_err(|x| x.to_string())?;
				tx.commit().await.map_err(|x| x.to_string())?;
				format!("{} table(s) changed", n)
			}
			Kind::V | Kind::R => {
				let sql = std::fs::read_to_string(&f.path).map_err(|x| format!("{}: {}", f.name, x))?;
				tx.batch_execute(&sql).await.map_err(|x| format!("{}: {}", f.name, x))?;
				tx.execute(&format!("INSERT INTO {} (name, checksum) VALUES ($1, $2) ON CONFLICT (name) DO UPDATE SET checksum = excluded.checksum", hist),
					&[&f.name, &f.checksum]).await.map_err(|x| x.to_string())?;
				tx.commit().await.map_err(|x| x.to_string())?;
				"ran".to_string()
			}
		};
		report.ran.push((f.name.clone(), what));
	}
	to_sync.dedup();
	for t in to_sync {
		client.query("SELECT yt_setup($1::varchar)", &[&t]).await.map_err(|x| format!("yt_setup('{}'): {}", t, x))?;
		report.synced.push(t);
	}
	Ok(report)
}

/// a schema's tables as PostgreSQL has them: (schema, table, has a primary key)
#[cfg(feature = "service")]
fn pg_tables(y: &Yaml, file: &str) -> Result<Vec<(String, String, bool)>, String> {
	let parsed = schema_guard_tokio::parse_yaml_schema(y.clone(), file)?;
	Ok(parsed.list.iter().flat_map(|s| s.tables.list.iter().filter(|t| t.is_template != Some(true)).map(move |t|
		(s.schema_name.clone(), t.table_name.clone(),
		 t.columns.list.iter().any(|c| c.constraint.as_ref().and_then(|k| k.primary_key).unwrap_or(false))))).collect())
}
