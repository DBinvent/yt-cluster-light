# Embedding a big cluster node capabilities with bi-directional sync of selected (portion) table right into your app.

`yt-core` is a YT node for one database. It links to a YT Cluster as a leaf of it: one
node, the lowest there is, with nothing below it. It comes in three forms:

- **yt-lite** (the `service` feature, on by default) runs as a service beside one
  PostgreSQL instance and serves one of its databases through the `yt_cluster` extension.
  The database is its store.
- **The library inside an application**, on the application's own database: Turso, with
  the `turso` feature (see [Inside an application](#inside-an-application-turso)). Without a
  database the node keeps its rows in memory.
- **A SQLite loadable extension**, [yt-sqlite](../yt-sqlite/README.md): a SQLite database
  file is the node, on the same store as the Turso form (`src/lite.rs`, feature `lite`).

## No az

A YT Cluster places its nodes in azs (zones), and relates clusters to each other by them:
which cluster is above, which leader signs in there, which neighbours it dials. A lite
node is none of that. It is never a leader, it has no cluster below it, and it dials no
neighbour, so it has no az and no setting for one.

The cluster node it signs in through takes it for a node of that node's own az. The lite
node is told which az that is (`yt_info()` shows it) and cannot change it. That az decides
which tables it takes: see [Tables](#tables).

## yt-lite

yt-lite serves the extension's unix socket, `.s.YT.<port>`, for one database. A pid file
beside the socket keeps a second node off it.

- **Commits.** It keeps each transaction's rows until the transaction commits. It then
  writes them to `yt_outbox`, at the next position, and keeps them there until the cluster
  above has them. A transaction whose end never arrived is settled as the database decided
  it (`pg_xact_status`).
- **What comes down** is applied in one transaction with its position. The session sets
  `yt.node = 'local'`, so the rows are not captured again, and each value is cast to the
  local column's type.
- **Another database:** a write from one is refused with SQLSTATE `0A000`, naming the full
  version.

```
yt-lite [--db] <url> [--config <schema.table>] [--file <file>] [--uuid <uuid>] [-v | -q]

  <url>       the database it serves, over the instance's unix socket:
              postgresql://postgres@localhost/appdb?host=/var/run/postgresql
  --config    its config table (default: the extension's yt_config)
  --file      key=value lines: the settings below, ahead of the config table's
  --uuid      this node's uuid (default: the module-L row `uuid`, made at the first start)
```

### Settings

Each setting is a `name=value` line of `--file`, or a row of the config table in module L.
yt-lite takes marg, the command-line library, at 0.4.7 or later: 0.4.4 read `--file` from a
file named `file` instead of the path given.

| Name | What it is |
|---|---|
| `upper` | the nodes of the cluster above to dial, as `http://host:port`, comma separated and tried in turn |
| `upper_root` | the root of the cluster above, `SHA256:...`: its CA, as `ytctl --skv-status` prints it there. yt-lite takes only what chains to it. |
| `cluster_db` | this database's name in the cluster above (default: its own name) |
| `key_dir` | where this node keeps its CA (default: `~/.yt-lite/<uuid>`) |
| `lang` | the language of its messages: `en`, `ru`, `fr`, `de`, `zh` or `ja` (default: the database's `lc_messages`, else the system locale's, else English) |

```sql
INSERT INTO yt_config (name, module, value) VALUES ('upper', 'L', 'http://node1:7421');
```

### Linking it to a cluster

The cluster needs its secret store and TLS between its nodes. It needs no entry for the
lite node: no az, no roster row. Its operator approves the lite node's CA, and that is
all.

1. On the cluster, `ytctl --skv-status` prints its CA, `SHA256:...`. That goes in
   `upper_root`.
2. Set `upper` and `upper_root`, then start yt-lite.
3. At its first start, yt-lite makes its CA in `key_dir` and logs the CA's fingerprint
   with the line to run. The cluster refuses its sign-in until that CA is approved.
4. On the cluster's leader, run `ytctl --skv-approve <uuid> --fingerprint <fp>`. Compare the
   fingerprint with the one yt-lite logged.
5. The cluster countersigns the CA. yt-lite keeps the countersigned CA and the cluster's
   roots beside the key, so a restart needs no new approval. From then on every call to
   the cluster is TLS 1.3.

### Tables

A lite node takes the tables of the az it shares: the az of the cluster node it is linked
to.

- **On the cluster**, a table goes to lite nodes only when it has an az rule (it is
  *routed*). A table with no rule is the cluster's own and never leaves it. Of the routed
  tables, a lite node is sent those its node's az holds, and its own writes are taken for
  those that az may write. `no_down` and `no_up` keep a table off the link in that
  direction. In a cluster with no azs, every routed table goes both ways.
- **On the lite node**, a table syncs as on a cluster node: when its config table holds
  an A row for it (`schema.table`). `select yt_setup('<schema.table>')` writes one and puts
  the trigger that captures its writes; an A row alone (`INSERT INTO yt_config (name,
  module, value) VALUES ('<schema.table>', 'A', '')`) syncs a table whose rows only come
  down. The row's value is the cluster's encoding (`ytctl --encode-table-cfg`): `no_up`
  keeps the table's writes here, `no_down` keeps what comes down off it; empty is both
  ways. A table with no A row syncs neither way: what comes down for it is not applied and
  its writes are not sent, each counted (`rows_not_synced_here`). A change of the A rows is
  read within two seconds.
- A row that comes down for a table this database does not have is not this node's: it is
  skipped and counted (`rows_not_held_here`).
- A table created here later starts from that moment. Copy its rows first.

### yt_info()

`select yt_info()` in the served database returns these fields:

| Field | Value |
|---|---|
| `version` | this node's version |
| `node` | its uuid |
| `database` | the database it serves |
| `cluster_database` | that database's name in the cluster above |
| `hosts_above` | the nodes it dials |
| `az` | the az the node above takes this one to be in: its own. Shown only; a lite node has no az to set. |
| `outbox` | transactions in `yt_outbox` that the cluster above does not hold yet |
| `open_transactions` | transactions that have not prepared yet |
| `unknown_transactions` | transactions whose outcome the database could not report |
| `rows_missed_here` | rows from above that found nothing to change here |
| `rows_not_held_here` | rows from above for a table this database does not have: skipped |
| `rows_not_synced_here` | rows of a table that does not sync that way (no A row, or `no_up` / `no_down` in it): not applied, not sent |
| `apply_error` | the last apply that failed, if any |
| `uptime_sec` | seconds since it started |
| `ca_fingerprint` | its CA |
| `link_tls` | whether the cluster has countersigned the CA, and whether calls are TLS |

### Messages

yt-lite says everything in one language (the `lang` setting). An error's message starts
with its code, `YT-Ennn`, the same in every language, after a SQLSTATE such as `0A000:`
when it has one. `https://docs.dbinvent.com/errors/<code>` explains a code. YT lite's
codes have three digits; YT Cluster's own have four.

### Versions

yt-lite links to YT Cluster 1.21 or later: the Link service it calls came with 1.21. In a
cluster that is being upgraded from 1.20:

- A node still at 1.20 has no Link service. yt-lite says so (`YT-E077`, the node named)
  and signs in through the next node of `upper`.
- A leader still at 1.20 does not admit a lite node. yt-lite says so (`YT-E078`) and signs
  in again every minute; it is admitted once the leader runs 1.21.
- Rows cross as they do in a cluster of one version, and the extension of either version
  works under either node (`../ytpgc/README.md`, "Version").

### Limits

- One database per instance. Several databases, and the cluster itself, need the full
  version.
- It is a leaf: nothing goes further down from it.
- One hop. A lite node gets what its cluster commits. Rows that the cluster itself
  received over a link (from a cluster above it, or from another lite node) are not passed
  on.
- A schema change is not followed by itself: deploy it with the `migrate` setting (below), or
  run `yt_setup` again after one.
- A local write to a table that comes down from the cluster is not refused here.

## Migrate, then sync

The `migrate` setting (a line of `--file`, or a row of the config table, module L) names a
directory of sge's files, applied at the start as sge applies them: `S<version>__name.yaml`
schemas through schema_guard_tokio, `V<version>__name.sql` scripts once, `R__name.sql` again
when changed - each kept in `yt_migrations` with its MD5. Then each table the schemas name with
a primary key is set up to sync (`yt_setup('schema.table')`); one with none is said, not synced.
Run it again after a schema change. The Turso and MySQL forms do the same (`migrate_dir`; the
SQLite extension's `yt_migrate(dir)`, feature `lite-migrate`), and
there the cluster's PostgreSQL schema is mapped for the node's database first - its custom SQL
left out - with each column's PostgreSQL type kept in `yt_columns`, so a value goes up with the
type the cluster has.

## Beside MySQL: yt-lite-mysql

With the `mysql` feature, the lite node's store is a MySQL database (sqlx, pure Rust), and
`yt-lite-mysql` its service:

```sh
yt-lite-mysql mysql://user:pw@host:3306/app --migrate ./schemas --once       # the schema only
yt-lite-mysql mysql://user:pw@host:3306/app --uuid <uuid> --upper http://node1:7421 \
    --cluster-db appdb --root SHA256:... --migrate ./schemas
```

- **Capture** as on Turso: three triggers a table write the row as JSON to `yt_outbox`. The
  node's own applies set `@yt_apply` on their session and the triggers write nothing then, so
  what came down does not go back up while the application's writes go on.
- **Types.** A `timestamptz` is a `datetime(6)` in UTC here: it goes up with `+00` and comes
  down as UTC. A boolean is a `tinyint(1)`.
- **Names** as on Turso: a table of schema `s` is the MySQL table `s.t` (a name with a dot,
  quoted) in the URL's database.
- MySQL 8 signs in with TLS here (no RSA): a server makes its certificate by itself.

## Inside an application: Turso

With the `turso` feature (and `default-features = false`), an application on Turso is a
lite node itself: the node runs on the application's tokio runtime and keeps its rows in
the application's database. `../yt-turso-load` is a whole example: a load like pgbench's on
Turso, which the end-to-end tests run against a cluster.

```rust
let db = turso::Builder::new_local("app.db").build().await?;
let store = Arc::new(yt_core::turso::Turso::open(&db, "appdb").await?); // the cluster's name for this database
store.setup("s_app.accounts").await?;                                // capture a table's writes, and sync it
store.sync("common.prices", "").await?;                              // sync a table whose rows only come down
let tls = Arc::new(yt_core::tls::Tls::open(&key_dir, &uuid, Some(upper_root))?);
let node = Arc::new(Node::with_db(uuid_u128, hosts, store).with_tls(tls));
let status = Arc::new(Mutex::new(yt_core::leaf::Status::default()));
tokio::spawn(yt_core::leaf::run(node, status.clone(), |said| println!("{said}")));
```

- **Linking** is yt-lite's: the CA's fingerprint (`tls.fingerprint()`) is approved on the
  cluster's leader with `ytctl --skv-approve <uuid> --fingerprint <fp>`, and from then on the
  calls are TLS. `leaf::Status` says where the link is: granted, TLS, caught up, waiting.
- **Names.** A cluster table `schema.table` is the Turso table named `schema.table` (quoted:
  SQLite has no schemas); a table of `public` is named by its table alone. Columns match by
  name, and a table needs a primary key.
- **Types.** Declare each column with its PostgreSQL type (`int`, `bigint`, `text`,
  `timestamptz`...). The node sends a value as the PostgreSQL extension does - an integer
  type as an integer, a float, a boolean, anything else as text - with the PostgreSQL type
  id, so the cluster binds it as one of its own. A timestamp is text: write it as PostgreSQL
  does (`2026-11-13 18:00:00+00`).
- **Which tables sync:** as yt-lite's, by an A row in the application database's own
  `yt_config` (`schema.table`, the cluster's name). `setup(table)` writes one with the
  triggers; `sync(table, value)` writes one alone, for a table whose rows only come down;
  `unsync(table)` takes both away. No A row: nothing of the table moves, counted
  (`not_synced`).
- **Capture.** `setup(table)` puts three triggers on the table that write each row to
  `yt_outbox`; they commit with it. Run it again after a schema change. Write an upsert
  (`ON CONFLICT DO UPDATE`), not `INSERT OR REPLACE`: SQLite's triggers see a replaced row as
  an insert only.
- **Down.** What comes down is applied on the node's own connection, with its cursor, in one
  transaction that takes the write lock first; the application's writes wait for it (set a
  `busy_timeout` on the application's connection too). A table with no key is applied by
  every column of the old row, one of equal rows; a float that is not finite is kept as
  PostgreSQL's word for it (`NaN`, `Infinity`), a `bytea` as a BLOB; a value that does not
  read as its column's type refuses the batch (`YT-E081`), as PostgreSQL would.
- **A table dropped** with rows not yet sent: they go up while the node still knows its
  columns; after a restart their positions are passed over and counted (`not_sent`).
- **Turso** is taken at `0.9.0-pre.5`, a pre-release: the newest published.

## The link

`src/core` holds the link to the cluster above, in two kinds of files.

- **Shared files:** `pos`, `route`, `relay` and `signin`. YT Cluster's server compiles
  these same files. They are written for a link between two azs; a lite node has none,
  and answers with none wherever they ask.
- **The lite node's own versions** of `store`, `members`, `calls` and `offer`. The shared
  files reach the node only through these. The server keeps its own files under these
  names.

`src/tls.rs` holds the CA and the TLS (feature `tls`). `src/db.rs` is what a store is;
`src/pg.rs` and `src/service.rs` are the PostgreSQL store and the unix socket service, and
`src/lite.rs` the store in a database of the SQLite dialect, which `src/turso.rs` runs on Turso
and [yt-sqlite](../yt-sqlite/README.md), the SQLite loadable extension, on SQLite.
`src/leaf.rs` runs the link's turns for an application.
`locales/` holds the messages, one file per language, and `src/i18n.rs` checks them.

## Building

The crate needs `protoc` (see [yt-core-proto](../yt-core-proto/README.md#building)).
yt-lite also needs the OpenSSL headers (`libssl-dev`).

```
cargo build --release      # yt-lite at target/release/yt-lite
cargo build --no-default-features    # the library alone, without yt-lite
```

## Tests

```
cargo test
YT_LITE_EXT_SO=../ytpgc/yt_cluster1_21-pg16-glibc2_39.so cargo test
```

The tests that need PostgreSQL (`tests/pg_live.rs`, `pg_service.rs`, `yt_lite_bin.rs`) start
a throwaway server of their own (`tests/fixture`), from `pg_config --bindir` (or `PG_BIN`), with
the extension's library `YT_LITE_EXT_SO` - built by `make -C ../ytpgc` for that PostgreSQL. Without
it they are skipped. `pg_live` also runs against a database of yours, `YT_LITE_TEST_DB`.
The coverage run (`.github/workflows/coverage.yml`) runs them all; its target is 80% of the lines,
the generated gRPC code left out.

`tests/link.rs` runs the link against a cluster node served in the same process, and
`tests/turso.rs` (`cargo test --features turso`) an application on Turso the same way.
`tests/sqlite_ext.rs` loads yt-sqlite, the SQLite extension (`YT_SQLITE_EXT`, its built
`libyt.so`; skipped without it), into a SQLite of its own and links it the same way.
`tests/pg_live.rs` runs yt-lite's store against a real PostgreSQL, `pg_service.rs` the extension
writing through yt-lite's service, and `yt_lite_bin.rs` the yt-lite binary itself.
