// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! YT Cluster lite: the Rust types of the base, db and link protos, with their helper
//! code, and the gRPC client and server stubs (the server ones behind the `server`
//! feature).
use std::cmp::{self, Ordering};
use std::collections::{BTreeMap, HashMap};
use std::collections::hash_map::DefaultHasher;
use std::fmt::{self, Display, Formatter};
use std::hash::Hash;

// [i18n] this crate's messages, in the language the binary set (locales/, src/i18n.rs)
rust_i18n::i18n!("locales", fallback = "en");
pub mod i18n;

pub mod protogen;
// the extension's script, a node's defaults and the config table's statements
pub mod config;
pub use config::*;
pub use protogen::{base, db, link};
// diff_doc.proto's types: the diff_doc crate's, under this crate's other packages
pub use ::diff_doc::proto as diff_doc;
// the uuid crate these types are built on: the link files name azs and nodes with it
pub use uuid;

use base::*;
use db::*;

#[cfg(test)]
mod lib_test;

/// The config module of a table's row: `A.<db>.<schema>.<table>`
pub const TABLES_MODULE: char = 'A';

pub const PG_IN_PROGRESS: &str = "in progress";
pub const PG_COMMITTED: &str = "committed";
pub const PG_ABORTED: &str = "aborted";

// 0
pub const PG_READ_COMMITTED: &str = "read committed";
// 1
pub const PG_READ_UNCOMMITTED: &str = "read uncommitted";
// 2
pub const PG_REPEATABLE_READ: &str = "repeatable read";
// 3
pub const PG_SERIALIZABLE: &str = "serializable";

/// PgIso
pub const PG_ISO: [&str; 4] = [
	PG_READ_COMMITTED,
	PG_READ_UNCOMMITTED,
	PG_REPEATABLE_READ,
	PG_SERIALIZABLE
];

pub const PG_READ_UNCOMMITTED_ID: i32 = PgIso::ReadUncommitted as i32;
pub const PG_READ_COMMITTED_ID: i32 = PgIso::ReadCommitted as i32;
pub const PG_REPEATABLE_READ_ID: i32 = PgIso::RepeatableRead as i32;
pub const PG_SERIALIZABLE_ID: i32 = PgIso::Serializable as i32;

pub type TXID = u64;

pub const MAX_TERM: Term = u64::MAX - u32::MAX as u64;

/// The election index from the cluster's start, incremented by each election: the
/// cluster-wide counter of time frames. An election follows a term timeout, a leader
/// quitting, or its subterms running out.
pub type Term = u64;

/// The sequential transaction id within a term, incremented at transaction
/// boundaries; a u32 on the wire, see MAX_SUBTERM
pub type SubTerm = u32;

/// the approximate largest subterm: past it, a new term's election is asked for
pub const MAX_SUBTERM: u32 = u16::MAX as u32 * 2;

/// The index of a row change (update, insert, delete) within a subterm; a u32 on
/// the wire, see VERSION_PER_SUBTERM
pub type SubTermVersion = u32;
pub const VERSION_PER_SUBTERM: u32 = u16::MAX as u32 / 2;

// the data change log index within the current term (as in Raft)
pub type TermLogIdx = u32;

/// The column values of a row, by attnum
pub type ColumnData = HashMap<AttNumType, VarType>;
pub type AttNumType = u32;

#[inline]
pub fn lo(input: u128) -> u64 { input as u64 }
#[inline]
pub fn hi(input: u128) -> u64 { (input >> 64) as u64 }

#[inline]
pub fn hilo(input_hi: u64, input_lo: u64) -> u128 {
	let x = (input_hi as u128) << 64;
	x + input_lo as u128
}

impl DbAction {
	pub fn to_str(action: i32) -> String {
		match DbAction::try_from(action) {
			Ok(action) => match action {
				DbAction::Update => "update".to_string(),
				DbAction::Insert => "insert".to_string(),
				_ => "delete".to_string()
			}
			Err(_e) => rust_i18n::t!("types.action_invalid", action = action).into_owned()
		}
	}
}

impl ModifyReply {
	/// Neutral "nothing to report" reply. `skip_complete` stays false - the safe
	/// default: the trigger calls complete() for the transaction.
	pub fn ok0() -> Self {
		ModifyReply { result: 0, dump: None, skip_complete: false }
	}

	/// The node registered no state for this transaction, so the origin trigger can
	/// skip the prepare and commit round trips entirely.
	pub fn ok0_skip() -> Self {
		ModifyReply { result: 0, dump: None, skip_complete: true }
	}

}

impl TableRef {
	pub fn as_key(&self) -> String {
		format!("{}.{}.{}.{}", TABLES_MODULE, self.db, self.schema, self.table)
	}

}

impl EntityKey {
	#[inline]
	pub fn is_valid(&self) -> bool {
		self.term > 0
	}
}

impl Display for EntityKey {
	#[inline]
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}.{}.{}", self.term, self.subterm, self.version)
	}
}

impl DbInputEntity {
	#[inline]
	pub fn new() -> Self {
		DbInputEntity {
			tx: 0,
			table: None,
			old_data: Default::default(),
			new_data: Default::default(),
			action: 0,
			origin: None,
			iso: 0,
			user_role: "".to_string(),
			jwt: "".to_string()
		}
	}

	#[inline]
	pub fn pk(&self) -> ColumnData {
		let mut pv = HashMap::new();
		if let Some(table) =  &self.table {
			for c in &table.columns {
				if c.pk {
					match self.old_data.get(&c.attnum) {
						None => if let Some(v) = self.new_data.get(&c.attnum) {
							pv.insert(c.attnum, v.clone());
						}
						Some(v) => {
							pv.insert(c.attnum, v.clone());
						}
					}
				}
			}
		}
		pv
	}

	/// filter only PK column values into new_data (on Delete copied from old_data)
	#[inline]
	pub fn as_pk_new_val(&self) -> Self {
		let mut pv = HashMap::new();
		let mut columns = Vec::new();
		if let Some(table) =  &self.table {
			for c in &table.columns {
				if c.pk {
					columns.push(c.clone());
					match self.new_data.get(&c.attnum) {
						None => if let Some(v) = self.old_data.get(&c.attnum) {
							pv.insert(c.attnum, v.clone());
						}
						Some(v) => {
							pv.insert(c.attnum, v.clone());
						}
					}
				}
			}
		}

		DbInputEntity {
			tx: self.tx,
			table: self.table.as_ref().map(|t| TableRef {
				oid: t.oid,
				db: t.db.clone(),
				schema: t.schema.clone(),
				table: t.table.clone(),
				columns,
			}),
			old_data: Default::default(),
			new_data: pv,
			action: self.action,
			origin: self.origin,
			iso: self.iso,
			user_role: self.user_role.clone(),
			jwt: self.jwt.clone(),
		}
	}

	#[inline]
	pub fn columns(&self) -> BTreeMap<AttNumType, ColumnRef> {
		let mut c = BTreeMap::new();
		if let Some(table) =  &self.table {
			for cc in &table.columns {
				c.insert(cc.attnum, cc.clone());
		}}
		c
	}


	#[inline]
	pub fn as_table_oid(&self) -> Option<u32> {
		self.table.as_ref().map(|e| e.oid)
	}

	#[inline]
	pub fn as_db(&self) -> String {
		self.table.as_ref().map(|e| e.db.clone()).unwrap_or("".to_string())
	}

}

impl VarType {
	#[allow(clippy::should_implement_trait)] // an inherent method callers across ytserv use; the std trait takes another signature
	pub fn hash(&self, hash: &mut DefaultHasher) {
		if let Some(value) = &self.value {
			match value {
				// VarType::Nil => {}
				var_type::Value::UInt(v) => v.hash(hash),
				var_type::Value::Int(v) => v.hash(hash),
				// VarType::BigInt(v) => v.hash(hash),
				// VarType::BigUint(v) => v.hash(hash),
				var_type::Value::Flt(v) => v.to_bits().hash(hash),
				var_type::Value::Bool(v) => v.hash(hash),
				var_type::Value::Universal(v) => v.hash(hash),
				var_type::Value::Diff(v) => v.hash(hash),
			}
		}
	}

	pub fn to_str(&self) -> String {
		match &self.value {
			None => "".to_string(),
			Some(value) => match value {
				var_type::Value::UInt(v) => v.to_string(),
				var_type::Value::Int(v) => v.to_string(),
				var_type::Value::Flt(v) => v.to_string(),
				var_type::Value::Bool(v) => v.to_string(),
				var_type::Value::Universal(v) => v.clone(),
				var_type::Value::Diff(v) => v.to_string(),
			}
		}
	}
}

impl CfgVo {
	#[inline]
	pub fn new(name: &str, value: String, module: char) -> Self {
		CfgVo {
			name: name.to_string(),
			value,
			module: module.to_string(), // (module as u8) as char //ConfigModule::from(module)
		}
	}
	/// in this config module: its char, or what turns into one
	pub fn is(&self, m: impl Into<char>) -> bool {
		let m: char = m.into();
		let m = m.to_string();
		self.module == m
	}

	#[inline]
	pub fn as_cfg_key(&self) -> String {
		format!("{}.{}", self.module, self.name)
	}
}

impl DataVersion {
	#[inline(always)]
	pub fn size(&self) -> usize {
		self.subterm.len() + self.except.len()
	}

	/// The first element is the mark - complete without a break through it, 0 when
	/// nothing at the start of the term is - and every hole after it is missed,
	/// after a mark of 0 or 1 too: a node that rejoined mid-term, holding nothing
	/// below its first subterm, reports the start of the term missing.
	#[inline(always)]
	pub fn missed(&self) -> u32 {
		self.missed_vec().len() as u32
	}

	#[inline(always)]
	pub fn missed_vec(&self) -> Vec<u32> {
		let mut p = 0;
		let mut r = Vec::new();
		for (i, x) in self.subterm.iter().enumerate() {
			if i > 0 && *x > p + 1 {
				let mut t = Vec::with_capacity((*x - p) as usize);
				for i in p+1..*x {
					t.push(i);
				}
				r.append(&mut t);
			}
			p = *x;
		}
		if self.last > p {
			let mut t = Vec::with_capacity((self.last - p) as usize);
			for i in p + 1..self.last+1 {
				t.push(i);
			}
			r.append(&mut t);
		}
		r
	}

	#[inline(always)]
	pub fn cmpv(this: &[Self], other: &[Self], ) -> cmp::Ordering {
		if this.is_empty() {
			if other.is_empty() {
				cmp::Ordering::Equal
			} else {
				cmp::Ordering::Less
			}
		} else {
			if other.is_empty() {
				cmp::Ordering::Greater
			} else {
				this[this.len() - 1].cmp(&other[other.len() - 1])
			}
		}
	}

}

impl PeerUuid {
	#[inline]
	pub fn as_uuid(&self) -> u128 {
		hilo(self.hi, self.lo)
	}

	#[inline(always)]
	pub fn as_uuid8(&self) -> String {
		let s = uuid::Uuid::from_u128(self.as_uuid()).to_string();
		format!("{}..{}", &s[0..2], &s[30..])
	}

	#[inline(always)]
	pub fn as_bytes(&self) -> [u8; 16] {
		self.as_uuid().to_be_bytes()
	}

	#[inline(always)]
	pub fn some(uuid: u128) -> Option<Self> {
		Some(PeerUuid {
			hi: hi(uuid),
			lo: lo(uuid)
		})
	}

}

impl  PgIso {
	#[inline(always)]
	#[allow(clippy::should_implement_trait)] // an inherent method callers across ytserv use; the std trait takes another signature
	pub fn from_str(value: String) -> Self {
    	match value.as_str() {
			PG_REPEATABLE_READ => PgIso::RepeatableRead,
			PG_READ_UNCOMMITTED => PgIso::ReadUncommitted,
			PG_SERIALIZABLE => PgIso::Serializable,
			_ => PgIso::ReadCommitted
		}
	}
}

impl Display for PgIso {
	#[inline]
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		write!(f, "{}", match &self {
			PgIso::RepeatableRead => PG_REPEATABLE_READ,
			PgIso::ReadUncommitted => PG_READ_UNCOMMITTED,
			PgIso::Serializable => PG_SERIALIZABLE,
			PgIso::ReadCommitted => PG_READ_COMMITTED
		})
	}
}

impl From<String> for PeerUuid {
	#[inline(always)]
    fn from(value: String) -> Self {
		let uuid = match uuid::Uuid::parse_str(value.as_str()) {
			Ok(u) => u.as_u128(),
			Err(_) => {
				let mut res = 0u128;
				if !value.is_empty() {
					for i in 0..std::cmp::max(16, value.len()) {
						let c = value.as_bytes()[i % value.len()];
						res = (res << 8) + c as u128;
					}
				}
				res
			}
		};
		PeerUuid {
			hi: hi(uuid),
			lo: lo(uuid)
		}
    }
}

impl From<u128> for PeerUuid {
	#[inline(always)]
    fn from(uuid: u128) -> Self {
		PeerUuid {
			hi: hi(uuid),
			lo: lo(uuid)
		}
    }
}

impl From<[u8; 16]> for PeerUuid {
	#[inline(always)]
    fn from(uuid: [u8; 16]) -> Self {
		let uuid = u128::from_be_bytes(uuid);
		PeerUuid {
			hi: hi(uuid),
			lo: lo(uuid)
		}
    }
}

impl Display for PeerUuid {
	#[inline]
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
	    write!(f, "{}", uuid::Uuid::from_u128(self.as_uuid()))
	}
}

impl PartialOrd<Self> for PeerUuid {
	#[inline]
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for PeerUuid {
	#[inline]
	fn cmp(&self, other: &Self) -> Ordering {
		let this = self.as_uuid();
		let other = other.as_uuid();
		if this > other {
			Ordering::Greater
		} else if this < other {
			Ordering::Less
		} else {
			Ordering::Equal
		}
	}
}

impl Ord for DataVersion {
	#[inline]
	fn cmp(&self, other: &Self) -> Ordering {
		if self.term < other.term {
			Ordering::Less
		}
		else if self.term > other.term {
			Ordering::Greater
		} else {
			if self.subterm.is_empty() && other.subterm.is_empty() {
				Ordering::Equal
			} else if !self.subterm.is_empty() && other.subterm.is_empty() {
				Ordering::Greater
			} else if self.subterm.is_empty() && !other.subterm.is_empty() {
				Ordering::Less
			} else {
				let x = self.subterm[self.subterm.len() - 1] as i64
					- other.subterm[other.subterm.len() - 1] as i64;
				if x < 0 {
					Ordering::Less
				} else if x > 0 {
					Ordering::Greater
				} else {
					Ordering::Equal
				}
			}
		}
	}
}

impl PartialOrd<Self> for DataVersion {
	#[inline]
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}
