# Contributing to yt-cluster-light (ytcl)

Thank you for helping. ytcl is licensed MIT; a contribution is accepted under the same
terms (see [LICENSE](LICENSE)).

## Issues

Open an issue on GitHub for a bug or a feature request. For a bug, say which part it is
about - the PostgreSQL extension (`ytpgc`), the protos and their types (`yt-core-proto`)
or the light node (`yt-core`) - and give its version (`VERSION`; `select yt_info('T')`
for an installed extension, `yt-light -V` for the service), the PostgreSQL major and the
platform (glibc, EL9, musl), and the
smallest sequence of statements or calls that shows it: what you did, what you expected,
and what you got.

A security problem is not reported in a public issue: write to
**support@dbinvent.com** instead.

## Pull requests

1. Fork the repository and branch from `main`.
2. Keep a change to one subject; describe what it changes and why.
3. Build and test it (below); add tests for what you change.
4. A change to diff_doc - its proto, its C sources, its Rust types - is made in
   [diff-doc-rs](https://github.com/vkrinitsyn/diff-doc-rs), its own public repository:
   the copies here (`yt-core-proto/proto/diff_doc.proto`, `ytpgc/src/proto/diff_apply.*`
   and `bson_doc.*`) are refreshed from it by `ytpgc/Makefile`, never edited here.
5. Generated files are not edited by hand: `yt-core-proto/src/protogen/` (its `build.rs`,
   from the protos), `ytpgc/src/proto/*.pb-c.*` (`make`, by protoc-c) and the extension's
   update scripts `ytpgc/yt_cluster--<earlier>--<version>.sql` (`make update-sql`, from the
   script: run it after every change of `ytpgc/yt_cluster--<version>.sql`).
6. The shared link files in `yt-core/src/core/` (`pos`, `route`, `relay`, `signin`) are
   compiled by another build too, beside its own versions of `store`, `members`, `calls`
   and `offer`: they use only yt-core-proto, std, tokio, rust-i18n and those four files
   (`super::`), no `crate::` paths and no logging, and a change to what they call from the
   four files changes the light node's versions too. A message they say is written
   `rust_i18n::t!("link-...")`, spelled out, with its text in `yt-core/locales/`.
7. A message a user reads is in the crate's locale files (`locales/`), in all six
   languages (en, ru, fr, de, zh, ja), never a string in the code. An error's message
   starts with its code: three digits here, `YT-Ennn` - yt-core-proto YT-E001 to YT-E029,
   yt-core YT-E030 and up - the next free one, never one used before. (Four digits,
   `YT-Ennnn`, are YT Cluster's own.) A SQLSTATE such as `0A000: ` stays at position 0,
   before the code. `src/i18n.rs` in each crate tests all of it.
8. Open the pull request against `main`.

## Build and test

yt-core-proto takes diff_doc's types from crates.io (1.1 on). To try a change of diff-doc-rs
here first, point the workspace at a clone of it, uncommitted:

```
# Cargo.toml, the workspace's
[patch.crates-io]
diff_doc = { path = "../diff-doc-rs" }
```

`protoc` and the protobuf includes are needed (`yt-core-proto/build.rs`), the OpenSSL
headers for yt-light, and for the extension protobuf-c, libcurl and the PostgreSQL server
headers: `apt install protobuf-compiler libprotobuf-dev libssl-dev protobuf-c-compiler
libprotobuf-c-dev libcurl4-openssl-dev postgresql-server-dev-<major>`.

```
cargo test                               # yt-core-proto and yt-core
cargo build --release --bin yt-light     # the service beside one PostgreSQL instance
YT_LIGHT_TEST_DB='postgresql://...' cargo test --test pg_live -- --ignored   # its store on a throwaway database
cd ytpgc && make                         # the extension for this host's PostgreSQL
sudo make install                        # then CREATE EXTENSION yt_cluster in a database
psql -d <db> -f test.sql                 # the diff functions: every line prints t
psql -d <db> -f test_bson.sql            # the BSON functions
bash ytpgc/build-all-ytpgc.sh            # from the repo root: every major in pg-versions.txt, in Docker
```

Before a pull request, check the dependencies as well:

```
cargo audit      # known vulnerabilities in the dependencies
```

## Code of conduct

Taking part in this project means following its
[Code of Conduct](CODE_OF_CONDUCT.md).
