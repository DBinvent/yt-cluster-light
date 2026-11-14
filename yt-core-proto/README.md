# Embedding a big cluster node capabilities with bi-directional sync of selected (portion) table right into your app.

`yt-core-proto` is the wire format of YT Cluster light: the protos that a YT node, its
PostgreSQL extension and the zone above it exchange, with their Rust types. YT Cluster's
server takes these same types, so each message has one definition.

## What is in it

- **`base`** (`proto/base.proto`): a node's identity and the header of every call
  (`PeerUuid`, `BaseApi`), a transaction's completion (`ClusterComplete`), and the
  references to tables, columns, keys and values.
- **`db`** (`proto/db.proto`): the `LocalGrpc` service that the extension calls on the node
  beside the database (`modify`, `complete`, `info`, `merge`, `llm`), and its messages,
  `DbInputEntity` among them: one row change.
- **`link`** (`proto/link.proto`): the `Link` service that a lower zone calls on the zone
  above. `signin` offers its CA, `fetch` takes what crossed down, and `deliver` sends what
  goes up. Its messages are here too.
- **`diff_doc`**: diff-doc-rs's types, from the `diff_doc` crate's `proto` feature,
  re-exported here. `proto/diff_doc.proto` is a copy for the extension's C build.
- **The gRPC stubs:** clients always, and the servers (`LocalGrpc`, `Link`) behind the
  `server` feature (on by default). A node that only calls can turn it off.
- **`config`:** the extension's version (`EXT_VERSION`), its SQL script's name (`EXT_SQL`)
  and what its update scripts and library files are called, all from the repository's
  `VERSION` at build time; a node's defaults; and the names and statements of its config
  table (`ClusterConfigNames`).
- **`i18n` and `locales/`:** the crate's messages in six languages (en, ru, fr, de, zh,
  ja), said in the language the binary set with `rust_i18n::set_locale`. An error's message
  starts with its code: `YT-E001` to `YT-E029` here. Three digits are YT light's; YT
  Cluster's own codes have four.

## Building

The crate needs `protoc`. It takes `diff_doc` from crates.io, 1.1 or later: the release that
carries its `proto` feature. `build.rs` compiles `proto/base.proto`, `db.proto` and
`link.proto`, and takes the `diff_doc` package's types from that crate.

```
cargo build
cargo test
```

## License

MIT, as the rest of the repository.
