// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! A SQLite connection as the store's (yt_core::lite::Conn): its statements run on it as
//! they come, under a lock of its own - the store holds one connection at a time anyway.
use std::sync::Mutex;

use rusqlite::types::{Value, ValueRef};
use rusqlite::{params_from_iter, Connection};
use yt_core::lite::{Conn, Val};

pub struct Sqlite(Mutex<Connection>);

impl Sqlite {
	pub fn new(c: Connection) -> Sqlite {
		Sqlite(Mutex::new(c))
	}
}

fn to(v: Val) -> Value {
	match v {
		Val::Null => Value::Null,
		Val::Integer(i) => Value::Integer(i),
		Val::Real(f) => Value::Real(f),
		Val::Text(s) => Value::Text(s),
		Val::Blob(b) => Value::Blob(b),
	}
}

fn from(v: ValueRef<'_>) -> Val {
	match v {
		ValueRef::Null => Val::Null,
		ValueRef::Integer(i) => Val::Integer(i),
		ValueRef::Real(f) => Val::Real(f),
		ValueRef::Text(t) => Val::Text(String::from_utf8_lossy(t).into_owned()),
		ValueRef::Blob(b) => Val::Blob(b.to_vec()),
	}
}

impl Sqlite {
	fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
		let c = self.0.lock().unwrap_or_else(|e| e.into_inner());
		f(&c).map_err(|e| e.to_string())
	}
}

#[tonic::async_trait]
impl Conn for Sqlite {
	async fn exec(&self, sql: &str, params: Vec<Val>) -> Result<u64, String> {
		self.with(|c| c.execute(sql, params_from_iter(params.into_iter().map(to))).map(|n| n as u64))
	}

	async fn batch(&self, sql: &str) -> Result<(), String> {
		self.with(|c| c.execute_batch(sql))
	}

	async fn rows(&self, sql: &str, params: Vec<Val>) -> Result<Vec<Vec<Val>>, String> {
		self.with(|c| {
			let mut st = c.prepare(sql)?;
			let n = st.column_count();
			let mut r = st.query(params_from_iter(params.into_iter().map(to)))?;
			let mut out = Vec::new();
			while let Some(row) = r.next()? {
				let mut v = Vec::with_capacity(n);
				for i in 0..n {
					v.push(from(row.get_ref(i)?));
				}
				out.push(v);
			}
			Ok(out)
		})
	}
}
