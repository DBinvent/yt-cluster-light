// SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
//! What a node and its tools share about the database they run on: the extension's SQL
//! script, the defaults of a node, and the config table's names and statements. (The
//! statements only YT Cluster runs - its roster, a copy's rows - are ytct's.)

/// The extension's version, `<major>.<minor>`: the repo's VERSION file (read by build.rs),
/// in step with YT Cluster's own. It names the SQL script, the control file's
/// `default_version` and the library files.
pub const EXT_VERSION: &str = env!("YT_EXT_MAJOR_MINOR");
/// The extension's SQL script, as ytpgc/Makefile names it: `yt_cluster--<version>.sql`
pub const EXT_SQL: &str = concat!("yt_cluster--", env!("YT_EXT_MAJOR_MINOR"), ".sql");
/// What every script of the extension starts with - this version's, and the update
/// scripts to it from the earlier ones (`yt_cluster--<earlier>--<version>.sql`).
pub const EXT_SQL_PREFIX: &str = "yt_cluster--";

/// Is `file_name` an update script TO this version - `yt_cluster--<earlier>--<version>.sql`,
/// what `ALTER EXTENSION yt_cluster UPDATE` runs in a database that has `<earlier>`?
pub fn is_ext_update_sql(file_name: &str) -> bool {
	file_name.strip_prefix(EXT_SQL_PREFIX).and_then(|r| r.strip_suffix(".sql"))
		.and_then(|r| r.split_once("--"))
		.is_some_and(|(from, to)| to == EXT_VERSION && !from.is_empty() && from != EXT_VERSION && !from.contains("--"))
}

/// What a library file of this version of the extension starts with:
/// `yt_cluster<major>_<minor>` of `yt_cluster<major>_<minor>-pg<N>-<libc>.so`. A build of
/// another version is not this script's library.
pub fn ext_lib_prefix() -> String {
	format!("yt_cluster{}", EXT_VERSION.replace('.', "_"))
}
pub const DEFAULT_SCHEMA:&str = "public";
pub const DEFAULT_TABLE:&str = "yt_config";
pub const DEFAULT_BIND_HOST:&str = "localhost";
pub const DEFAULT_BIND_PORT: u16 = 7421;
pub const DEFAULT_FS_DIR:&str = "/var/yt-cluster";

/// WAL / data directory default: `/var/yt-cluster`, or `%ProgramData%\yt-cluster` on
/// Windows, which has no /var.
#[cfg(windows)]
pub fn default_fs_dir() -> String {
    if let Some(pd) = std::env::var_os("ProgramData") {
        std::path::Path::new(&pd).join("yt-cluster").to_string_lossy().into_owned()
    } else {
        DEFAULT_FS_DIR.to_string()
    }
}

#[cfg(not(windows))]
pub fn default_fs_dir() -> String {
    DEFAULT_FS_DIR.to_string()
}

/// names of config names to build CfgVo from config table
#[derive(Debug, Clone)]
pub struct ClusterConfigNames {
    /// db schema.table, default yt_config
    pub table: String,
    pub id_name: String,
    pub column_name: String,
    pub column_value: String,
    /// the column of the module code: a row's module, for loading only the rows asked for
    pub module_column: String,
    // pub module_code: String,
}

impl ClusterConfigNames {
    pub const COLUMN_NAME_SIZE: usize = 80;
    pub const COLUMN_MODULE_DEFAULT: &'static str = "' '";

    #[inline(always)]
    pub fn schema(&self) -> String {
        match self.table.find("."){
            None => DEFAULT_SCHEMA.to_string(),
            Some(x) => if x == 0 {
                DEFAULT_SCHEMA.to_string()
            } else {
                self.table[0..x].to_string()
            }
        }
    }

    #[inline(always)]
    pub fn to_sql_create_stm(&self) -> String {
        // value is text, as in the extension's script (ytpgc/yt_cluster--<version>.sql): a row may hold a PEM certificate
        // or key. Keyed by the setting, (module, name); the id stays a plain column, for
        // the order a loader reads in
        format!("create table {} ({} serial, {} varchar({}) not null, {} char not null default {}, {} text not null, primary key ({}, {}));",
                self.table, self.id_name,
                self.column_name, ClusterConfigNames::COLUMN_NAME_SIZE,
                self.module_column, ClusterConfigNames::COLUMN_MODULE_DEFAULT,
                self.column_value,
                self.module_column, self.column_name)
    }

    /// Re-key an existing config table by `(module, name)`: run at every install, the way
    /// the extension's later tables reach an installed one - the extension has no upgrade
    /// script. Idempotent: a table already keyed so is left alone.
    ///
    /// One transaction, on this node alone (`session_replication_role` replica:
    /// the sync trigger stays out of it - every node runs it for itself). Of two
    /// rows of one setting the newest (highest id) stays, which is the one the
    /// loader already took; a row without a name, which no setting can be, goes.
    pub fn to_sql_natural_key_stm(&self) -> String {
        let mut key = [self.module_column.as_str(), self.column_name.as_str()];
        key.sort(); // compared with array_agg(attname ORDER BY attname)
        format!(r#"DO $$
DECLARE
 pk name;
 gone bigint;
BEGIN
 IF EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = '{t}'::regclass AND i.indisprimary
     AND (SELECT array_agg(a.attname::text ORDER BY a.attname::text) FROM pg_attribute a
          WHERE a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey)) = ARRAY['{k0}', '{k1}']) THEN
  RETURN;
 END IF;
 PERFORM set_config('session_replication_role', 'replica', true);
 PERFORM set_config('lock_timeout', '30s', true);
 DELETE FROM {t} a USING {t} b
  WHERE a.{m} = b.{m} AND a.{n} = b.{n} AND a.{id} < b.{id};
 GET DIAGNOSTICS gone = ROW_COUNT;
 IF gone > 0 THEN RAISE NOTICE '{t}: % older duplicate row(s) of a setting removed', gone; END IF;
 DELETE FROM {t} WHERE {n} IS NULL;
 GET DIAGNOSTICS gone = ROW_COUNT;
 IF gone > 0 THEN RAISE NOTICE '{t}: % row(s) without a name removed', gone; END IF;
 ALTER TABLE {t} ALTER COLUMN {n} SET NOT NULL;
 SELECT conname INTO pk FROM pg_constraint WHERE conrelid = '{t}'::regclass AND contype = 'p';
 IF pk IS NOT NULL THEN EXECUTE format('ALTER TABLE {t} DROP CONSTRAINT %I', pk); END IF;
 ALTER TABLE {t} ADD PRIMARY KEY ({m}, {n});
 RAISE NOTICE '{t}: keyed by ({m}, {n})';
END $$;"#,
            t = self.table, m = self.module_column, n = self.column_name, id = self.id_name,
            k0 = key[0], k1 = key[1])
    }

    #[inline(always)]
    pub fn to_sql_stm(&self) -> String {
        // by id: of two rows with one (module, name) a loader's map keeps the last, so the
        // highest id wins - not whichever the heap returned last. `order by 1`: a loader
        // may put this last in a UNION, where the order is the whole union's and the
        // column is named by its first branch
        format!("select {}, {}, {}, {} from {} where {} <> {} order by 1",
                self.id_name, self.column_name, self.column_value,
                self.module_column, self.table,
                self.module_column, ClusterConfigNames::COLUMN_MODULE_DEFAULT)
    }

    #[inline(always)]
    pub fn to_sql_stm_mod(&self) -> String {
        format!("select {}, {} from {} where {} = $1",
                self.column_name, self.column_value,
                self.table,
                self.module_column)
    }
}

impl Default for ClusterConfigNames {
    #[inline(always)]
    fn default() -> Self {
        ClusterConfigNames {
            table: "yt_config".to_string(),
            id_name: "id".to_string(),
            column_name: "name".to_string(),
            column_value: "value".to_string(),
            module_column: "module".to_string(),
        }
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod tests;
