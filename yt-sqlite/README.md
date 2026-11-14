# yt-sqlite: a SQLite database as a lite node of a YT cluster

A SQLite loadable extension, `yt`. Load it into SQLite, set up the tables to sync, and link
the database file to a YT cluster. The rows written to those tables go up to the cluster.
What the cluster holds of them comes down. It syncs both ways, as
[yt-lite](../yt-core/README.md#yt-lite) does beside PostgreSQL and the
[Turso form](../yt-core/README.md#inside-an-application-turso) does inside an application.
All three use one store: `yt-core/src/lite.rs`.

## Install

With [sqlpkg](https://github.com/nalgeon/sqlpkg-cli), from this repository's `sqlpkg.json`:

```
sqlpkg install github.com/DBinvent/yt-cluster-light
```

Or take the release asset for your platform, `yt-sqlite-<version>-<platform>.zip`
(linux-x64, linux-arm64, macos-x64, macos-arm64, win-x64). It holds `yt.so`, `yt.dylib` or
`yt.dll`.

It needs SQLite 3.34.1 or later, built thread-safe (the default). On Linux it needs
glibc 2.28 or later. OpenSSL is built in.

## Use

```sql
.load ./yt
CREATE TABLE accounts (aid bigint PRIMARY KEY, abalance int NOT NULL DEFAULT 0);
SELECT yt_setup('accounts');                      -- capture it, sync it both ways
SELECT yt_link('http://node1:7421', 'SHA256:...'); -- the cluster above, and its root CA
SELECT yt_info();
```

From Python:

```python
c = sqlite3.connect("app.db")
c.enable_load_extension(True)
c.load_extension("./yt")
```

| Function | What it does |
|---|---|
| `yt_setup(table)` | Captures the table's writes (three triggers into `yt_outbox`) and syncs it both ways (its A row in `yt_config`). Run it again after a schema change. |
| `yt_sync(table [, value])` | Syncs a table with no capture: only its rows from the cluster come down. The value is the cluster's encoding of which ways, `ytctl --encode-table-cfg`. Leave it empty for both. A value it cannot read is refused (YT-E085). |
| `yt_unsync(table)` | Stops syncing a table: its A row and its triggers are removed. |
| `yt_link()`, `yt_link(upper, upper_root)` | Links the database file to the cluster above, from its settings. The two given are written to the settings first, and a link already running is made again with them. It returns the node's CA fingerprint, for the cluster's operator to approve. Not inside a transaction (YT-E084): a rollback would take the settings and the node's uuid while the link runs. |
| `yt_unlink()` | Stops the link of this database file, once its turn ends: the call waits up to a second. It returns false when the file was not linked. |
| `yt_info()` | Where the link is, as JSON: `linked`, `granted`, `tls`, `caught_up`, `waiting` (rows not yet passed by the cluster), `problem`, the counts of rows not held or not synced, the last batch from above that failed (`apply_error`) and the link's last messages (`said`). |

The functions can be called only from SQL you run yourself, not from a trigger or a view.

### Settings

The settings are rows of `yt_config` in module L, as yt-lite has them:

| Name | What it is |
|---|---|
| `upper` | the nodes of the cluster above, as `http://host:port`, comma separated and tried in turn |
| `upper_root` | the root of the cluster above, `SHA256:` and 43 characters: its CA, as `ytctl --skv-status` prints it there. Only what chains to it is taken. Another form is refused (YT-E086). |
| `cluster_db` | this database's name in the cluster above (default: the file's name without its extension) |
| `key_dir` | where this node keeps its CA (default: `~/.yt-lite/<uuid>`) |
| `uuid` | this node's id: made at the first link and kept here |
| `lang` | the language of its messages: `en`, `ru`, `fr`, `de`, `zh` or `ja` |

### Linking

1. On the cluster, `ytctl --skv-status` prints its CA, `SHA256:...`: that is `upper_root`.
2. `SELECT yt_link('http://node1:7421', 'SHA256:...');` returns this node's fingerprint.
   `yt_info()` shows it with the node's `uuid`.
3. On the cluster: `ytctl --skv-approve <uuid> --fingerprint <fingerprint>`. The link turns
   TLS (`"tls": true`).

The link runs on a thread of its own, with a connection of its own to the file: one link per
file in each process. It keeps running after the connection that started it closes, until
`yt_unlink()` or the process ends. Link again after a restart: `SELECT yt_link();` takes the
settings as they are.

A database in memory, or a temporary one, has no file and cannot link (YT-E082).

### Tables and names

A cluster table `schema.table` is the table named `schema.table` here (quoted, since SQLite
has no schemas); `public`'s tables go by their name alone. Columns are matched by name, and a
table needs a primary key (YT-E080). Declare each column with its PostgreSQL type (`int`,
`bigint`, `text`, `timestamptz`...): the value goes up as that type. A table syncs when it
has an A row, the ways that row says. What comes down for a table with none is not applied,
and its captured rows are not sent; `yt_info()` counts both.

`yt_link` sets the file to WAL (`journal_mode=WAL`), so the application reads while the link
writes. A file that cannot switch right then (another connection is in a transaction) keeps
its mode. The link's applies take the write lock (BEGIN IMMEDIATE): set a `busy_timeout` on
the application's connection too.

### Limits

As in the Turso form:

- A schema change is followed only by `yt_setup` again.
- `INSERT OR REPLACE` of a row that is there goes up as an insert, which the cluster refuses
  on its key: write an upsert (`ON CONFLICT DO UPDATE`).
- The tables that come down have no read-only guard.

## Building

The extension needs perl for the OpenSSL it builds in (NASM too, on Windows); diff_doc comes
from crates.io (see [yt-core-proto](../yt-core-proto/README.md#building)).

```
cargo build --release      # target/release/libyt.so (.dylib, yt.dll)
```

It is a workspace of its own: its rusqlite is built for a loadable extension, where every call
goes through the host's SQLite, and no other crate of the repository can share that build.
rusqlite is held at 0.40: its bindings take a host SQLite from 3.34.1 on, while 0.41 and later
want 3.45.3.

## Tests

```
cargo build
YT_SQLITE_EXT=$PWD/target/debug/libyt.so cargo test -p yt-core --test sqlite_ext   # from the repository's root
```

`yt-core/tests/sqlite_ext.rs` loads the extension into a SQLite of its own, on a database
file, and links it to a node above served in the same process. It checks that a row goes up,
that one comes down and does not go back up, and that `yt_info()` and `yt_unlink()` work.
The store itself is tested with the Turso form's tests (`cargo test --features turso`).
`.github/workflows/sqlite.yml` runs the test on every push. On a tag `v<version>`, it builds
the five platforms and attaches them to the release.
