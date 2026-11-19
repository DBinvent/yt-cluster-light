//! [mysql] The MySQL form: its SQL with no server, and a live run against one when asked for -
//! `YT_MYSQL_URL=mysql://user:pw@host:3306/db cargo test --features mysql --lib mysql -- --ignored`
//! (a database of its own: the run drops and makes again its tables).
use super::*;
use crate::db::Db;
use crate::lite::utc_naive;
use yt_core_proto::base::{var_type::Value as V, ColumnRef, TableRef, VarType};
use yt_core_proto::db::{DbAction, DbInputEntity};
use yt_core_proto::link::{RelayBatch, RelayItem, RelayPos};

fn col(name: &str, attnum: u32, decl: &str, pg: &str, pk: bool) -> Col {
	Col { attnum, name: name.into(), decl: decl.into(), pk, pg: pg.into() }
}

fn row(schema: &str, table: &str, action: DbAction, cols: &[(&str, u32, bool, u32)], old: &[(u32, V)], new: &[(u32, V)]) -> DbInputEntity {
	DbInputEntity { table: Some(TableRef { db: "appdb".into(), schema: schema.into(), table: table.into(),
		columns: cols.iter().map(|(n, a, pk, ty)| ColumnRef { attnum: *a, name: n.to_string(), pk: *pk, atttypid: *ty, ..Default::default() }).collect(),
		..Default::default() }),
		old_data: old.iter().map(|(k, v)| (*k, VarType { value: Some(v.clone()) })).collect(),
		new_data: new.iter().map(|(k, v)| (*k, VarType { value: Some(v.clone()) })).collect(),
		action: action as i32, ..Default::default() }
}

#[test]
fn mysqls_types_give_the_values_and_type_ids_postgresql_has() {
	assert_eq!(kind("tinyint(1)"), (Kind::Bool, 16));
	assert_eq!(kind("bigint auto_increment"), (Kind::Int, 20));
	assert_eq!(kind("int unsigned"), (Kind::Int, 23));
	assert_eq!(kind("datetime(6)"), (Kind::Text, 1114));
	assert_eq!(kind("longtext"), (Kind::Text, 25));
	assert_eq!(kind("longblob"), (Kind::Bytes, 17));
	assert_eq!(kind("double precision"), (Kind::Flt, 701));
	assert_eq!(kind("time without time zone"), (Kind::Text, 1083));
	// [review 2026-11-19] a precision inside the name
	assert_eq!(kind("timestamp(3) with time zone"), (Kind::Text, 1184));
	assert_eq!(kind("character varying(80)"), (Kind::Text, 1043));
	assert_eq!(kind("numeric(12,2)"), (Kind::Text, 1700));
}

#[test]
fn a_timestamp_with_a_zone_is_utc_with_none() {
	assert_eq!(utc_naive("2026-11-13 18:00:00+00").as_deref(), Some("2026-11-13 18:00:00"));
	assert_eq!(utc_naive("2026-11-13 18:00:00.25+02").as_deref(), Some("2026-11-13 16:00:00.25"));
	assert_eq!(utc_naive("2026-11-13 01:30:00-05:30").as_deref(), Some("2026-11-13 07:00:00"));
	assert_eq!(utc_naive("2026-03-01 00:30:00+01").as_deref(), Some("2026-02-28 23:30:00"), "back over a month");
	assert_eq!(utc_naive("2024-12-31T23:00:00Z").as_deref(), Some("2024-12-31 23:00:00"));
	assert_eq!(utc_naive("2026-11-13"), None, "a date: kept as it is");
}

#[test]
fn a_row_comes_down_in_mysqls_sql() {
	let cols = [col("id", 1, "bigint", "bigserial", true), col("name", 2, "varchar(80)", "varchar(80)", false),
		col("at", 3, "datetime(6)", "timestamptz", false)];
	let rc = [("id", 1, true, 20), ("name", 2, false, 1043), ("at", 3, false, 1184)];
	let ins = row("s_app", "accounts", DbAction::Insert, &rc, &[], &[(1, V::Int(5)), (2, V::Universal("a".into())), (3, V::Universal("2026-11-13 18:00:00+02".into()))]);
	let (sql, params, must) = statement_in(Dialect::Mysql, &ins, "s_app.accounts", &cols).unwrap();
	assert_eq!(sql, "INSERT INTO `s_app.accounts` (`id`, `name`, `at`) SELECT ?, ?, ? FROM DUAL WHERE NOT EXISTS \
		(SELECT 1 FROM `s_app.accounts` WHERE `id` = ?)");
	assert_eq!(params.len(), 4);
	assert_eq!(params[2], Val::Text("2026-11-13 16:00:00".into()), "as UTC, with no zone");
	assert!(!must);
	let upd = row("s_app", "accounts", DbAction::Update, &rc, &[(1, V::Int(5))], &[(1, V::Int(5)), (2, V::Universal("b".into()))]);
	assert_eq!(statement_in(Dialect::Mysql, &upd, "s_app.accounts", &cols).unwrap().0, "UPDATE `s_app.accounts` SET `name` = ? WHERE `id` = ?");
	// a keyless table: the first of the equal rows
	let keyless = [col("a", 1, "int", "int", false), col("b", 2, "longtext", "text", false)];
	let del = row("public", "log", DbAction::Delete, &[("a", 1, false, 23), ("b", 2, false, 25)], &[(1, V::Int(1)), (2, V::Universal("x".into()))], &[]);
	assert_eq!(statement_in(Dialect::Mysql, &del, "log", &keyless).unwrap().0, "DELETE FROM `log` WHERE `a` <=> ? AND `b` <=> ? LIMIT 1");
}

/// live: a migration on MySQL, a row the application writes going up with the cluster's types,
/// what comes down applied once and not captured back
#[tokio::test]
#[ignore = "needs a MySQL server: YT_MYSQL_URL=mysql://user:pw@host/db"]
async fn on_mysql_a_migration_syncs_up_and_down() {
	let url = std::env::var("YT_MYSQL_URL").expect("YT_MYSQL_URL");
	let reset = MysqlConn::connect(&url).await.unwrap();
	for t in ["`s_app.accounts`", "yt_outbox", "yt_position", "yt_config", "yt_columns", "yt_migrations"] {
		reset.batch(&format!("DROP TABLE IF EXISTS {}", t)).await.unwrap();
	}
	drop(reset);
	let dir = std::env::temp_dir().join(format!("yt-mysql-{}", std::process::id()));
	std::fs::create_dir_all(&dir).unwrap();
	std::fs::write(dir.join("S1__accounts.yaml"), "database:
  - schemaName: s_app
    tables:
      - table:
          tableName: accounts
          columns:
            - column:
                name: id
                type: bigserial
                constraint:
                  primaryKey: true
            - column:
                name: name
                type: varchar(80)
            - column:
                name: active
                type: boolean
                defaultValue: \"true\"
            - column:
                name: at
                type: timestamptz
                defaultValue: now()
").unwrap();
	let s = Mysql::open(&url, "appdb").await.unwrap();
	let r = s.migrate_dir(&s, &dir).await.unwrap();
	assert_eq!(r.synced, ["s_app.accounts"], "{:?}", r);

	// up: a row of the application's, with the cluster's types
	let app = MysqlConn::connect(&url).await.unwrap();
	app.batch("INSERT INTO `s_app.accounts` (id, name, active, at) VALUES (1, 'a', 1, '2026-11-19 10:00:00')").await.unwrap();
	let (items, upto, _) = s.collect(None, 10).await.unwrap();
	assert_eq!(items.len(), 1, "{:?}", items);
	let e = &items[0].data[0];
	let t = e.table.as_ref().unwrap();
	let attn = |n: &str| t.columns.iter().find(|c| c.name == n).unwrap().clone();
	assert_eq!((attn("id").atttypid, attn("active").atttypid, attn("at").atttypid), (20, 16, 1184));
	assert_eq!(e.new_data[&attn("at").attnum].value, Some(V::Universal("2026-11-19 10:00:00.000000+00".into())), "UTC, said");
	assert_eq!(e.new_data[&attn("active").attnum].value, Some(V::Bool(true)));
	s.passed(upto).await.unwrap();

	// down: applied once, not captured back
	let rc = [("id", 1, true, 20), ("name", 2, false, 1043), ("active", 3, false, 16), ("at", 4, false, 1184)];
	let batch = RelayBatch { items: vec![RelayItem { term: 3, subterm: 1, data: vec![
		row("s_app", "accounts", DbAction::Insert, &rc, &[], &[(1, V::Int(2)), (2, V::Universal("down".into())), (3, V::Bool(false)),
			(4, V::Universal("2026-11-19 12:00:00+02".into()))]),
		row("s_app", "accounts", DbAction::Update, &rc, &[(1, V::Int(1))], &[(1, V::Int(1)), (2, V::Universal("a2".into()))]),
	]}], upto: Some(RelayPos { term: 3, subterm: 1 }), ..Default::default() };
	assert_eq!(s.apply(&batch).await.unwrap(), Some((3, 1)));
	assert_eq!(s.apply(&batch).await.unwrap(), Some((3, 1)), "sent again: nothing");
	let got = app.rows("SELECT CAST(name AS CHAR), CAST(at AS CHAR) FROM `s_app.accounts` WHERE id = 2", vec![]).await.unwrap();
	assert_eq!(got[0][0], Val::Text("down".into()));
	assert!(matches!(&got[0][1], Val::Text(t) if t.starts_with("2026-11-19 10:00:00")), "{:?}", got);
	assert_eq!(s.waiting().await.unwrap(), 0, "the apply's rows are not captured");
	let (more, _, _) = s.collect(Some(upto), 10).await.unwrap();
	assert!(more.is_empty(), "{:?}", more);
	// and the application's next write still is
	app.batch("UPDATE `s_app.accounts` SET name = 'a3' WHERE id = 1").await.unwrap();
	assert_eq!(s.waiting().await.unwrap(), 1);
	let _ = std::fs::remove_dir_all(&dir);
}
