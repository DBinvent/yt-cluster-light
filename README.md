# Embedding a big cluster node capabilities with bi-directional sync of selected (portion) table right into your app.

[![audit](https://github.com/DBinvent/yt-cluster-light/actions/workflows/audit.yml/badge.svg)](https://github.com/DBinvent/yt-cluster-light/actions/workflows/audit.yml)
[![trivy](https://github.com/DBinvent/yt-cluster-light/actions/workflows/trivy.yml/badge.svg)](https://github.com/DBinvent/yt-cluster-light/actions/workflows/trivy.yml)
[![coverage](https://raw.githubusercontent.com/DBinvent/yt-cluster-light/badges/coverage.svg?sanitize=true)](https://github.com/DBinvent/yt-cluster-light/actions/workflows/coverage.yml)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

YT Cluster light is the open part of [YT Cluster](https://www.dbinvent.com/cluster/), under
the MIT license. It contains:

- the protocol that the levels of a YT Cluster hierarchy use to exchange rows;
- the PostgreSQL extension that sits inside the database;
- YT light: a YT node for one database that links to a YT Cluster nodes, and syncs the
  tables you choose in both directions.

## Modules

| Module | What it is | Its README covers |
|---|---|---|
| [`yt-core-proto/`](yt-core-proto/README.md) | The protocol: the `base`, `db` and `link` protos, their Rust types, the gRPC stubs, and the names and statements of a node's config table. | the protos one by one, the `server` feature, building |
| [`ytpgc/`](ytpgc/README.md) | `yt_cluster`, the PostgreSQL extension, in C. It captures the rows each transaction writes and hands them to the YT node beside the database over a unix socket. Its builds for each supported PostgreSQL version are here too. | the version and updating a database to it, building, tests, behaviour notes |
| [`yt-core/`](yt-core/README.md) | The link code, the light node, and `yt-light`, the light node as a service beside one PostgreSQL instance; with the `turso` feature, the light node inside an application on Turso. | yt-light's settings, linking it to a cluster, which tables it takes, `yt_info()`, limits, the Turso form, the link code, tests |
| [`yt-sqlite/`](yt-sqlite/README.md) | The light node as a SQLite loadable extension, `yt`: a SQLite database file linked to a cluster, its tables synced both ways. Installed with sqlpkg (`sqlpkg.json`) or from a release asset. | its functions, settings, linking, limits, building, tests |
| `yt-turso-load/` | An application on Turso with the light node inside it, and a load like pgbench's: the example of the Turso form, and the leaf the end-to-end tests run. | its header (`src/main.rs`) |

`VERSION`, at the root, is YT light's version, in step with YT Cluster's. The extension is
built from it: [ytpgc/README.md](ytpgc/README.md#version) says what it names, and how a
database that has an earlier version moves to it.

## License

MIT: see [LICENSE](LICENSE). [COPYRIGHT](COPYRIGHT) lists the copies of diff-doc-rs files
that this repository keeps. [CONTRIBUTING.md](CONTRIBUTING.md) explains how to send a
change, and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) covers conduct. [SECURITY.md](SECURITY.md)
says how to report a vulnerability.
