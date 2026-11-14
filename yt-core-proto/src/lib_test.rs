// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
use super::*;
use crate::diff_doc::{mismatches::Kind, Mismatches};
use std::hash::Hasher;
use std::collections::HashMap;

fn peer(n: u128) -> Option<PeerUuid> { PeerUuid::some(n) }

#[test]
fn a_uuid_splits_into_hi_and_lo_and_back() {
	let u = uuid::Uuid::new_v4();
	let a = u.as_u128();
	assert_eq!(a, hilo(hi(a), lo(a)));
	let p = PeerUuid::from(a);
	assert_eq!(p.as_uuid(), a);
	assert_eq!(p.to_string(), u.to_string());
	assert_eq!(PeerUuid::from(p.as_bytes()), p);
	assert_eq!(PeerUuid::from(u.to_string()), p);
	let s = u.to_string();
	assert_eq!(p.as_uuid8(), format!("{}..{}", &s[0..2], &s[30..]));
}

#[test]
fn a_peer_uuid_that_is_no_uuid_is_its_bytes_folded() {
	assert_eq!(PeerUuid::from(String::new()).as_uuid(), 0);
	let a = PeerUuid::from("node-a".to_string());
	assert_ne!(a.as_uuid(), 0);
	assert_eq!(a, PeerUuid::from("node-a".to_string()));
	assert_ne!(a, PeerUuid::from("node-b".to_string()));
}

#[test]
fn peer_uuids_order_by_their_u128() {
	let (a, b) = (PeerUuid::from(1u128), PeerUuid::from(u64::MAX as u128 + 1));
	assert!(a < b);
	assert_eq!(a.cmp(&a), Ordering::Equal);
	let mut v = vec![b, a];
	v.sort();
	assert_eq!(v, vec![a, b]);
}

#[test]
fn a_data_version_lists_what_it_misses() {
	let dv = |subterm: Vec<u32>, last: u32| DataVersion { term: 1, subterm, last, except: vec![] };
	assert_eq!(dv(vec![2, 4], 4).missed_vec(), vec![3]);
	assert_eq!(dv(vec![2, 5], 5).missed_vec(), vec![3, 4]);
	assert_eq!(dv(vec![2, 5], 7).missed_vec(), vec![3, 4, 6, 7]);
	assert_eq!(dv(vec![0, 3], 3).missed_vec(), vec![1, 2]);
	assert_eq!(dv(vec![], 2).missed_vec(), vec![1, 2]);
	assert_eq!(dv(vec![2, 5], 7).missed(), 4);
	assert_eq!(dv(vec![2, 5], 7).size(), 2);
}

#[test]
fn data_versions_compare_by_term_then_last_subterm() {
	let dv = |term: u64, subterm: Vec<u32>| DataVersion { term, subterm, last: 0, except: vec![] };
	assert!(dv(1, vec![9]) < dv(2, vec![1]));
	assert!(dv(2, vec![1, 3]) > dv(2, vec![2]));
	assert_eq!(dv(2, vec![]).cmp(&dv(2, vec![])), Ordering::Equal);
	assert!(dv(2, vec![1]) > dv(2, vec![]));
	assert_eq!(DataVersion::cmpv(&[], &[]), Ordering::Equal);
	assert_eq!(DataVersion::cmpv(&[], &[dv(1, vec![1])]), Ordering::Less);
	assert_eq!(DataVersion::cmpv(&[dv(1, vec![1])], &[]), Ordering::Greater);
	assert_eq!(DataVersion::cmpv(&[dv(9, vec![1]), dv(1, vec![1])], &[dv(2, vec![1])]), Ordering::Less);
}

#[test]
fn an_isolation_level_reads_and_prints_as_postgresql_names_it() {
	for (i, name) in PG_ISO.iter().enumerate() {
		let iso = PgIso::from_str(name.to_string());
		assert_eq!(iso as i32, i as i32);
		assert_eq!(iso.to_string(), *name);
	}
	assert_eq!(PgIso::from_str("no such".into()), PgIso::ReadCommitted);
}

#[test]
fn an_action_prints_as_its_name_and_a_bad_one_says_so() {
	assert_eq!(DbAction::to_str(DbAction::Insert as i32), "insert");
	assert_eq!(DbAction::to_str(DbAction::Update as i32), "update");
	assert_eq!(DbAction::to_str(DbAction::Delete as i32), "delete");
	assert!(DbAction::to_str(99).contains("99"));
}

#[test]
fn keys_and_config_rows_print_as_the_cluster_names_them() {
	let t = TableRef { oid: 7, db: "appdb".into(), schema: "public".into(), table: "t".into(), columns: vec![] };
	assert_eq!(t.as_key(), "A.appdb.public.t");
	let k = EntityKey { term: 3, subterm: 4, version: 5 };
	assert!(k.is_valid());
	assert!(!EntityKey::default().is_valid());
	assert_eq!(k.to_string(), "3.4.5");
	let c = CfgVo::new("port", "7421".into(), 'S');
	assert!(c.is('S'));
	assert!(!c.is('C'));
	assert_eq!(c.as_cfg_key(), "S.port");
	assert_eq!(ModifyReply::ok0(), ModifyReply { result: 0, dump: None, skip_complete: false });
	assert!(ModifyReply::ok0_skip().skip_complete);
}

fn row() -> DbInputEntity {
	let col = |attnum: u32, name: &str, pk: bool| ColumnRef { attnum, name: name.into(), pk, ..Default::default() };
	let v = |s: &str| VarType { value: Some(var_type::Value::Universal(s.into())) };
	let mut e = DbInputEntity::new();
	e.tx = 42;
	e.table = Some(TableRef { oid: 7, db: "appdb".into(), schema: "public".into(), table: "t".into(),
		columns: vec![col(1, "id", true), col(2, "name", false)] });
	e.old_data = HashMap::from([(1, v("old-id")), (2, v("old"))]);
	e.new_data = HashMap::from([(2, v("new"))]);
	e.action = DbAction::Update as i32;
	e.origin = peer(9);
	e
}

#[test]
fn a_rows_key_comes_from_its_primary_key_columns() {
	let e = row();
	let pk = e.pk();
	assert_eq!(pk.len(), 1);
	assert_eq!(pk[&1].to_str(), "old-id");
	let k = e.as_pk_new_val();
	assert_eq!(k.new_data.len(), 1);
	assert_eq!(k.new_data[&1].to_str(), "old-id");
	assert!(k.old_data.is_empty());
	assert_eq!(k.table.as_ref().map(|t| t.columns.len()), Some(1));
	assert_eq!((k.tx, k.action, k.origin), (42, DbAction::Update as i32, peer(9)));
	assert_eq!(e.columns().keys().copied().collect::<Vec<_>>(), vec![1, 2]);
	assert_eq!(e.as_table_oid(), Some(7));
	assert_eq!(e.as_db(), "appdb");
	assert_eq!(DbInputEntity::new().as_db(), "");
}

#[test]
fn a_value_prints_as_its_text() {
	let v = |x| VarType { value: Some(x) };
	assert_eq!(v(var_type::Value::Int(-3)).to_str(), "-3");
	assert_eq!(v(var_type::Value::UInt(3)).to_str(), "3");
	assert_eq!(v(var_type::Value::Bool(true)).to_str(), "true");
	assert_eq!(v(var_type::Value::Flt(1.5)).to_str(), "1.5");
	assert_eq!(VarType { value: None }.to_str(), "");
	let patch = Mismatches { kind: Some(Kind::Patch("@@ -1 +1 @@".into())) };
	assert_eq!(v(var_type::Value::Diff(patch.clone())).to_str(), "@@ -1 +1 @@");
	assert_eq!(patch.to_string(), "@@ -1 +1 @@");
	let (mut a, mut b) = (DefaultHasher::new(), DefaultHasher::new());
	v(var_type::Value::Int(1)).hash(&mut a);
	v(var_type::Value::Int(1)).hash(&mut b);
	assert_eq!(a.finish(), b.finish());
}
