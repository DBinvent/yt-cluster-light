# SBOMs

The software bill of materials of each project of this repository, and of sge, CycloneDX 1.5
in JSON: every library a binary links, with its version, purl and license - a project without
a license stops the run that makes them. Made again whenever their dependencies change, by
[`sbom.yml`](.github/workflows/sbom.yml) with [`tools/sbom.py`](tools/sbom.py) (from cargo
metadata) and [`tools/sbom-ytpgc.py`](tools/sbom-ytpgc.py) (the extension's linked libraries),
and kept on the `sbom` branch.

Every binary of the debs - these and the rest of the yt-cluster deb - is listed with its
libraries and their licenses, by deb and then by binary, with its SBOM to download, at
[dbinvent.com/licenses](https://dbinvent.com/licenses). Each deb carries the same in
`/usr/share/doc/<package>/`: the SBOMs, `lib-licenses.txt`, `THIRD-PARTY-LICENSES.txt` (each
library's license, copyright and notice texts), and for the CPython library the yt-cluster deb
ships, `LICENSE-Python.txt`.

## In the yt-cluster deb

| Project | In the deb | SBOM |
|---|---|---|
| [yt-lite](yt-core/README.md) | `/usr/local/bin/yt-cluster/yt-lite` | [sbom-yt-lite.cdx.json](https://raw.githubusercontent.com/DBinvent/yt-cluster-lite/sbom/sbom-yt-lite.cdx.json) |
| [the extension](ytpgc/README.md) (`yt_cluster`) | `/usr/local/include/yt-cluster/`: `yt_cluster*.so`, `libyt_cluster*.a`, `yt_cluster.control` and the SQL. The SBOM is one build, PostgreSQL 16 for the build machine's glibc: the deb's other builds (PostgreSQL 14 to 19; glibc 2.34, 2.35, 2.39; musl) link the same libraries, each of its own build | [sbom-ytpgc.cdx.json](https://raw.githubusercontent.com/DBinvent/yt-cluster-lite/sbom/sbom-ytpgc.cdx.json) |

### A feature away

| Project | What it is | SBOM |
|---|---|---|
| yt-core with `turso` | the lite node inside an application, on Turso | [sbom-yt-core-turso.cdx.json](https://raw.githubusercontent.com/DBinvent/yt-cluster-lite/sbom/sbom-yt-core-turso.cdx.json) |

## The sge deb

| Project | In the deb | SBOM |
|---|---|---|
| [sge](https://github.com/DBinvent/sge) | `/usr/bin/sge` | [sbom-sge.cdx.json](https://raw.githubusercontent.com/DBinvent/sge/sbom/sbom-sge.cdx.json) |

## Optional, in no deb

| Project | What it is | SBOM |
|---|---|---|
| [yt-core-proto](yt-core-proto/README.md) | the protocol crate | [sbom-yt-core-proto.cdx.json](https://raw.githubusercontent.com/DBinvent/yt-cluster-lite/sbom/sbom-yt-core-proto.cdx.json) |
| [yt-sqlite](yt-sqlite/README.md) | the lite node as a SQLite loadable extension, a release asset - for every target its releases build | [sbom-yt-sqlite.cdx.json](https://raw.githubusercontent.com/DBinvent/yt-cluster-lite/sbom/sbom-yt-sqlite.cdx.json) |
| yt-turso-load | the Turso example and the end-to-end tests' load | [sbom-yt-turso-load.cdx.json](https://raw.githubusercontent.com/DBinvent/yt-cluster-lite/sbom/sbom-yt-turso-load.cdx.json) |
