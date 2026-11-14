// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! The config table's names and the statements a node and its tools run on it.
use super::*;

fn named(table: &str) -> ClusterConfigNames {
	ClusterConfigNames { table: table.to_string(), ..ClusterConfigNames::default() }
}

#[test]
fn the_default_names_are_the_yt_config_table_and_its_columns() {
	let c = ClusterConfigNames::default();
	assert_eq!(c.table, DEFAULT_TABLE);
	assert_eq!((c.id_name.as_str(), c.column_name.as_str(), c.column_value.as_str(), c.module_column.as_str()),
		("id", "name", "value", "module"));
	assert_eq!(ClusterConfigNames::COLUMN_NAME_SIZE, 80);
	assert_eq!(ClusterConfigNames::COLUMN_MODULE_DEFAULT, "' '");
	// a clone is a copy, not a view
	let mut d = c.clone();
	d.table = "x.y".into();
	assert_eq!(c.table, "yt_config");
	assert!(format!("{:?}", d).contains("x.y"));
}

#[test]
fn the_schema_is_the_part_before_the_first_dot_or_public() {
	assert_eq!(named("yt_config").schema(), DEFAULT_SCHEMA);
	assert_eq!(named(".yt_config").schema(), "public", "an empty schema is the default one");
	assert_eq!(named("ops.yt_config").schema(), "ops");
	assert_eq!(named("a.b.c").schema(), "a");
}

#[test]
fn the_loader_reads_every_module_but_the_default_by_id() {
	let c = named("ops.cfg");
	assert_eq!(c.to_sql_stm(), "select id, name, value, module from ops.cfg where module <> ' ' order by 1");
	assert_eq!(c.to_sql_stm_mod(), "select name, value from ops.cfg where module = $1");
}

#[test]
fn the_extension_script_is_named_by_major_and_minor() {
	assert!(EXT_SQL.starts_with("yt_cluster--") && EXT_SQL.ends_with(".sql"), "{EXT_SQL}");
	let ver = &EXT_SQL["yt_cluster--".len()..EXT_SQL.len() - ".sql".len()];
	assert_eq!(ver.split('.').count(), 2, "major.minor only: {ver}");
	assert!(ver.split('.').all(|p| p.parse::<u32>().is_ok()), "{ver}");
}

/// FNV-1a, 64 bit: the checksum ytpgc/update-sql.py writes into an update script
fn fnv1a64(data: &[u8]) -> String {
	let mut h: u64 = 0xcbf29ce484222325;
	for b in data {
		h = (h ^ *b as u64).wrapping_mul(0x100000001b3);
	}
	format!("{:016x}", h)
}

/// The version is one: VERSION names the script that is there, the control file's
/// template takes it, and the library's own fallback says it.
#[test]
fn the_extension_is_one_version_in_every_place_that_names_it() {
	let dir = format!("{}/../ytpgc", env!("CARGO_MANIFEST_DIR"));
	assert!(std::path::Path::new(&format!("{dir}/{EXT_SQL}")).exists(), "{dir}/{EXT_SQL}");
	assert_eq!(EXT_SQL, format!("{EXT_SQL_PREFIX}{EXT_VERSION}.sql"));
	let template = std::fs::read_to_string(format!("{dir}/yt_cluster.control.in")).unwrap();
	assert!(template.contains("default_version = '@YT_CLUSTER_VERSION@'"), "the control file's version is the Makefile's to fill in");
	let header = std::fs::read_to_string(format!("{dir}/src/yt_cluster.h")).unwrap();
	assert!(header.contains(&format!("#define YT_CLUSTER_VERSION \"{EXT_VERSION}\"")), "yt_cluster.h's fallback is not {EXT_VERSION}");
	// no script of another version left beside it: CREATE EXTENSION would offer both
	for f in std::fs::read_dir(&dir).unwrap() {
		let name = f.unwrap().file_name().to_string_lossy().to_string();
		if name.starts_with(EXT_SQL_PREFIX) && name.ends_with(".sql") {
			assert!(name == EXT_SQL || is_ext_update_sql(&name), "{name}: neither this version's script nor an update to it");
		}
	}
	assert_eq!(ext_lib_prefix(), format!("yt_cluster{}", EXT_VERSION.replace('.', "_")));
	assert!(!ext_lib_prefix().contains('.'));
}

/// An earlier version has its way here: for each one in previous-versions.txt an update
/// script, made from the script as it is now (its checksum), with a block for every
/// object the script makes - and nothing in it that drops or alters.
#[test]
fn every_earlier_version_has_an_update_script_made_from_the_script_as_it_is() {
	let dir = format!("{}/../ytpgc", env!("CARGO_MANIFEST_DIR"));
	let script = std::fs::read(format!("{dir}/{EXT_SQL}")).unwrap();
	let sum = fnv1a64(&script);
	let text = String::from_utf8(script).unwrap();
	let earlier: Vec<String> = std::fs::read_to_string(format!("{dir}/previous-versions.txt")).unwrap().lines()
		.map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
	assert!(earlier.contains(&"0.5".to_string()), "0.5 is what databases have: {:?}", earlier);
	assert!(!earlier.iter().any(|e| e == EXT_VERSION), "{EXT_VERSION} is this version, not an earlier one");
	// what the script makes, by the words that start a statement
	let made = |word: &str| text.lines().filter(|l| l.to_uppercase().starts_with(word)).count();
	let (functions, views, tables, triggers) = (made("CREATE FUNCTION") + made("CREATE OR REPLACE FUNCTION"),
		made("CREATE VIEW"), made("CREATE TABLE IF NOT EXISTS"), made("CREATE EVENT TRIGGER"));
	assert!(functions > 20 && views == 3 && tables == 4 && triggers == 4, "{functions} {views} {tables} {triggers}");
	for e in &earlier {
		let name = format!("{EXT_SQL_PREFIX}{e}--{EXT_VERSION}.sql");
		assert!(is_ext_update_sql(&name), "{name}");
		let update = std::fs::read_to_string(format!("{dir}/{name}")).unwrap_or_else(|x| panic!("{name}: {x} - make update-sql"));
		assert!(update.contains(&format!("(fnv1a64 {sum})")),
			"{name} was made from another {EXT_SQL} than the one here - `make update-sql` in ytpgc");
		let count = |what: &str| update.matches(what).count();
		assert_eq!(count("CREATE OR REPLACE FUNCTION"), functions, "{name}: a block a function");
		assert_eq!(count("CREATE OR REPLACE VIEW"), views, "{name}: a block a view");
		assert_eq!(count("CREATE TABLE IF NOT EXISTS"), tables, "{name}: a block a table");
		assert_eq!(count("CREATE EVENT TRIGGER"), triggers, "{name}: a block an event trigger");
		assert_eq!(count("WHERE evtname = "), triggers, "{name}: an event trigger is made only when it is not there");
		let blocks = count("DO $yt_update$ BEGIN");
		assert_eq!(count(&format!("    WHEN OTHERS THEN\n        RAISE WARNING 'yt_cluster update to {EXT_VERSION}: % is kept as it is: %'")),
			blocks, "{name}: every block says what it kept");
		// a statement's `;` is on a line of its own: one ending in a `--` comment would take it along
		// (an event trigger's block ends in `\n;\nEND IF\n;\nEXCEPTION\n`, which holds this too)
		assert_eq!(count("\n;\nEXCEPTION\n"), blocks, "{name}: each statement closed on its own line");
		assert_eq!(count("\n;\nEND IF\n;\nEXCEPTION\n"), triggers, "{name}: an event trigger's too");
		// a C function binds a symbol of the library PostgreSQL has loaded: one it does not have
		// stops the update, the database left at its version, instead of being kept for good
		let in_c = text.lines().filter(|l| { let u = l.trim().to_uppercase(); u == "LANGUAGE C" || u.starts_with("LANGUAGE C ") }).count();
		assert!(in_c >= 18 && in_c < functions, "{in_c} of {functions} functions are C");
		assert_eq!(count("    WHEN undefined_function OR undefined_file THEN\n        RAISE EXCEPTION 'yt_cluster update to "), in_c,
			"{name}: every C function's block stops on a library that is not this version's");
		// a table that is there is not altered: the block after its own says one in an older shape
		assert_eq!(count("has the shape an older version gave it"), tables, "{name}: a shape check a table");
		assert!(update.contains("IF lacks IS NOT NULL OR keyed <> 'module, name' THEN"), "{name}: the config table is keyed by the setting");
		assert!(update.contains("unnest(ARRAY['id', 'name', 'module', 'value']) c"), "{name}: the config table's columns");
		for l in update.lines().filter(|l| !l.starts_with("--")) {
			let u = l.trim_start().to_uppercase();
			assert!(!u.starts_with("DROP ") && !u.starts_with("ALTER ") && !u.starts_with("TRUNCATE ") && !u.starts_with("DELETE "),
				"{name}: an update drops and alters nothing: {l}");
		}
		assert!(update.contains(&format!("UPDATE TO '{EXT_VERSION}'")), "{name}: the psql guard names the version");
	}
}

#[test]
fn an_update_script_is_one_to_this_version_from_another() {
	assert!(is_ext_update_sql(&format!("yt_cluster--0.5--{EXT_VERSION}.sql")));
	assert!(!is_ext_update_sql(EXT_SQL), "this version's own script");
	assert!(!is_ext_update_sql(&format!("yt_cluster--{EXT_VERSION}--{EXT_VERSION}.sql")));
	assert!(!is_ext_update_sql("yt_cluster--0.5--0.6.sql"), "to another version");
	assert!(!is_ext_update_sql(&format!("other--0.5--{EXT_VERSION}.sql")));
	assert!(!is_ext_update_sql(&format!("yt_cluster--0.5--{EXT_VERSION}.txt")));
	assert!(!is_ext_update_sql(&format!("yt_cluster----{EXT_VERSION}.sql")));
}

#[test]
fn the_data_dir_and_the_bind_defaults_hold() {
	#[cfg(not(windows))]
	assert_eq!(default_fs_dir(), DEFAULT_FS_DIR);
	assert!(default_fs_dir().ends_with("yt-cluster"));
	assert_eq!((DEFAULT_BIND_HOST, DEFAULT_BIND_PORT), ("localhost", 7421));
}

/// keyed by (module, name), the migration's test agreeing with
/// the order Postgres hands the key's names back in
#[test]
fn the_config_table_is_keyed_by_the_setting() {
    let c = ClusterConfigNames { table: "public.yt_config".to_string(), ..ClusterConfigNames::default() };
    let create = c.to_sql_create_stm();
    assert!(create.contains("primary key (module, name)"), "{create}");
    assert!(create.contains("id serial,") && !create.contains("serial primary key"), "{create}");
    assert!(create.contains("name varchar(80) not null"), "{create}");
    let m = c.to_sql_natural_key_stm();
    assert!(m.contains("ARRAY['module', 'name']"), "{m}");
    assert!(m.contains("ADD PRIMARY KEY (module, name)"), "{m}");
    assert!(m.contains("a.id < b.id"), "the newest of a setting stays: {m}");
    assert!(m.contains("session_replication_role"), "on this node alone: {m}");
    // other column names: the sorted pair for the test, the key in (module, name) order
    let o = ClusterConfigNames { table: "s.cfg".into(), id_name: "rid".into(), column_name: "akey".into(),
        column_value: "val".into(), module_column: "zmod".into() };
    let m = o.to_sql_natural_key_stm();
    assert!(m.contains("ARRAY['akey', 'zmod']") && m.contains("ADD PRIMARY KEY (zmod, akey)"), "{m}");
}
