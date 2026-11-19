// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! [mysql] The lite node on MySQL (owner, 2026-11-19): crate::lite's store in MySQL's SQL, on a
//! connection of this node's own (sqlx, pure Rust). What it captures, how rows come down, which
//! tables sync: crate::lite, as on Turso - with MySQL's differences there (crate::lite::Dialect):
//!
//! - **Capture.** Three triggers a table, each one statement, write the row as `JSON_OBJECT` to
//!   `yt_outbox`. This node's own applies set `@yt_apply` on their session, and the triggers
//!   write nothing then: nothing that came down goes back up, and the application's writes in
//!   between are captured as ever.
//! - **Types.** A column is MySQL's, mapped from the cluster's PostgreSQL by a migration
//!   (crate::migrate), which keeps each column's PostgreSQL type in `yt_columns`: a value goes
//!   up with that type's id. A `timestamptz` is a `datetime(6)` here, in UTC: it goes up with
//!   `+00`, and comes down as UTC.
//! - **The database** is the URL's (`mysql://user:password@host:3306/database`): the one
//!   `DATABASE()` names. Its tables are the cluster's, `schema.table` for a schema other than
//!   `public` (a name with a dot, quoted).
use sqlx::{Column, Connection, Row, TypeInfo};
use tokio::sync::Mutex;

pub use crate::lite::{cluster_name, kind, local_name, statement_in, Col, Dialect, Kind, Lite, Val};
use crate::lite::Conn;

/// The store on MySQL.
pub type Mysql = Lite<MysqlConn>;

/// A connection of this node's own to the MySQL database.
pub struct MysqlConn {
	conn: Mutex<sqlx::MySqlConnection>,
}

fn bind<'q>(mut q: sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments>, params: Vec<Val>)
	-> sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments> {
	for p in params {
		q = match p {
			Val::Null => q.bind(Option::<String>::None),
			Val::Integer(i) => q.bind(i),
			Val::Real(f) => q.bind(f),
			Val::Text(s) => q.bind(s),
			Val::Blob(b) => q.bind(b),
		};
	}
	q
}

/// a value of a row, by its column's MySQL type
fn val(r: &sqlx::mysql::MySqlRow, i: usize) -> Val {
	let ty = r.column(i).type_info().name().to_uppercase();
	let text = || r.try_get::<Option<String>, _>(i).ok().flatten();
	match ty.as_str() {
		t if t.contains("INT") && !t.contains("UNSIGNED") => match r.try_get::<Option<i64>, _>(i) {
			Ok(Some(v)) => Val::Integer(v),
			Ok(None) => Val::Null,
			Err(_) => text().map(Val::Text).unwrap_or(Val::Null),
		},
		t if t.contains("UNSIGNED") => match r.try_get::<Option<u64>, _>(i) {
			Ok(Some(v)) => Val::Integer(v as i64),
			Ok(None) => Val::Null,
			Err(_) => Val::Null,
		},
		"DOUBLE" | "FLOAT" => match r.try_get::<Option<f64>, _>(i) {
			Ok(Some(v)) => Val::Real(v),
			_ => Val::Null,
		},
		t if t.contains("BLOB") || t.contains("BINARY") => match r.try_get::<Option<Vec<u8>>, _>(i) {
			Ok(Some(v)) => Val::Blob(v),
			_ => Val::Null,
		},
		_ => match text() {
			Some(s) => Val::Text(s),
			None => match r.try_get::<Option<Vec<u8>>, _>(i) {
				Ok(Some(b)) => Val::Text(String::from_utf8_lossy(&b).to_string()),
				_ => Val::Null,
			},
		},
	}
}

fn sql_of(sql: &str) -> sqlx::AssertSqlSafe<String> {
	sqlx::AssertSqlSafe(sql.to_string())
}

#[tonic::async_trait]
impl Conn for MysqlConn {
	async fn exec(&self, sql: &str, params: Vec<Val>) -> Result<u64, String> {
		let mut c = self.conn.lock().await;
		if params.is_empty() {
			// a statement of MySQL's that takes no prepare (START TRANSACTION, COMMIT): as text
			return sqlx::raw_sql(sql_of(sql)).execute(&mut *c).await.map(|r| r.rows_affected()).map_err(|x| x.to_string());
		}
		bind(sqlx::query(sql_of(sql)), params).execute(&mut *c).await.map(|r| r.rows_affected()).map_err(|x| x.to_string())
	}

	async fn batch(&self, sql: &str) -> Result<(), String> {
		let mut c = self.conn.lock().await;
		sqlx::raw_sql(sql_of(sql)).execute(&mut *c).await.map(|_| ()).map_err(|x| x.to_string())
	}

	async fn rows(&self, sql: &str, params: Vec<Val>) -> Result<Vec<Vec<Val>>, String> {
		let mut c = self.conn.lock().await;
		let rows = bind(sqlx::query(sql_of(sql)), params).fetch_all(&mut *c).await.map_err(|x| x.to_string())?;
		Ok(rows.iter().map(|r| (0..r.len()).map(|i| val(r, i)).collect()).collect())
	}

	fn dialect(&self) -> Dialect {
		Dialect::Mysql
	}
}

impl MysqlConn {
	/// A connection to `url`'s database
	pub async fn connect(url: &str) -> Result<MysqlConn, String> {
		let c = sqlx::MySqlConnection::connect(url).await
			.map_err(|x| crate::lite::e("connect")(format!("{}: {}", url.rsplit('@').next().unwrap_or(""), x)))?;
		Ok(MysqlConn { conn: Mutex::new(c) })
	}

	/// The connection itself, for a migration (schema_guard_tokio::mysql)
	pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, sqlx::MySqlConnection> {
		self.conn.lock().await
	}
}

impl Lite<MysqlConn> {
	/// This node's store in the MySQL database of `url`, and its tables. `cluster_db`: this
	/// database's name in the cluster above.
	pub async fn open(url: &str, cluster_db: &str) -> Result<Mysql, String> {
		Lite::new(MysqlConn::connect(url).await?, cluster_db).await
	}
}

/// [migrate] the MySQL form's database for a migration: its version, deployed by schema_guard_tokio
#[tonic::async_trait]
impl crate::migrate::Target for Lite<MysqlConn> {
	async fn source(&self) -> Result<schema_guard_tokio::mapping::Source, String> {
		let r = self.conn().await.rows("SELECT VERSION()", vec![]).await?;
		let v = r.first().and_then(|r| crate::lite::text(&r[0])).unwrap_or_default();
		let mut it = v.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty());
		schema_guard_tokio::mapping::Source::parse(&format!("mysql.{}.{}", it.next().unwrap_or("8"), it.next().unwrap_or("0")))
	}
	async fn deploy(&self, y: yaml_rust2::Yaml, file: &str) -> Result<Vec<String>, String> {
		let c = self.conn().await;
		let mut m = c.lock().await;
		schema_guard_tokio::mysql::migrate(y, &mut m, false, file, &Default::default()).await.map(|p| p.statements)
	}
}

#[cfg(test)]
#[path = "mysql_test.rs"]
mod tests;
