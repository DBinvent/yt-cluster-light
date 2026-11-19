# Embedding a big cluster node capabilities with bi-directional sync of selected (portion) table right into your app.

[![audit](https://github.com/DBinvent/yt-cluster-lite/actions/workflows/audit.yml/badge.svg)](https://github.com/DBinvent/yt-cluster-lite/actions/workflows/audit.yml)
[![trivy](https://github.com/DBinvent/yt-cluster-lite/actions/workflows/trivy.yml/badge.svg)](https://github.com/DBinvent/yt-cluster-lite/actions/workflows/trivy.yml)
[![coverage](https://raw.githubusercontent.com/DBinvent/yt-cluster-lite/badges/coverage.svg?sanitize=true)](https://github.com/DBinvent/yt-cluster-lite/actions/workflows/coverage.yml)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![license: sbom](https://img.shields.io/badge/license-sbom-blue.svg)](SBOM.md)

YT Cluster lite is the open part of [YT Cluster](https://www.dbinvent.com/cluster/), under
the MIT license. It contains:

- the protocol that the levels of a YT Cluster hierarchy use to exchange rows;
- the PostgreSQL extension that sits inside the database;
- YT lite: a YT node for one database that links to a YT Cluster nodes, and syncs the
  tables you choose in both directions.

## Modules

| Module | What it is | Its README covers |
|---|---|---|
| [`yt-core-proto/`](yt-core-proto/README.md) | The protocol: the `base`, `db` and `link` protos, their Rust types, the gRPC stubs, and the names and statements of a node's config table. | the protos one by one, the `server` feature, building |
| [`ytpgc/`](ytpgc/README.md) | `yt_cluster`, the PostgreSQL extension, in C. It captures the rows each transaction writes and hands them to the YT node beside the database over a unix socket. Its builds for each supported PostgreSQL version are here too. | the version and updating a database to it, building, tests, behaviour notes |
| [`yt-core/`](yt-core/README.md) | The link code, the lite node, and `yt-lite`, the lite node as a service beside one PostgreSQL instance; with the `turso` feature, the lite node inside an application on Turso. | yt-lite's settings, linking it to a cluster, which tables it takes, `yt_info()`, limits, the Turso form, the link code, tests |
| [`yt-sqlite/`](yt-sqlite/README.md) | The lite node as a SQLite loadable extension, `yt`: a SQLite database file linked to a cluster, its tables synced both ways. Installed with sqlpkg (`sqlpkg.json`) or from a release asset. | its functions, settings, linking, limits, building, tests |
| `yt-turso-load/` | An application on Turso with the lite node inside it, and a load like pgbench's: the example of the Turso form, and the leaf the end-to-end tests run. | its header (`src/main.rs`) |

`VERSION`, at the root, is YT lite's version, in step with YT Cluster's. The extension is
built from it: [ytpgc/README.md](ytpgc/README.md#version) says what it names, and how a
database that has an earlier version moves to it.

## Is it true?

[ai_confirm.md](ai_confirm.md) is the comparison table of [dbinvent.com](https://www.dbinvent.com/),
cross-referenced: YT Cluster beside six other PostgreSQL clustering options, each statement with
the page that states it and the sentence quoted from that page. Review it, then give the whole
file to the AI you prefer. It is written from the site's table, so a change to it goes there.

## License

MIT: see [LICENSE](LICENSE). [COPYRIGHT](COPYRIGHT) lists the copies of diff-doc-rs files
that this repository keeps. [CONTRIBUTING.md](CONTRIBUTING.md) explains how to send a
change, and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) covers conduct. [SECURITY.md](SECURITY.md)
says how to report a vulnerability.
