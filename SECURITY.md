# Security Policy

## Supported versions

Security fixes go into the latest release of YT Cluster light: yt-light, the `yt_cluster`
PostgreSQL extension and the crates here, and so into YT Cluster's packages, which carry
them. Upgrade to the latest release to get them.

| Version | Supported |
|---|---|
| 1.21.x (latest) | yes |
| older | no |

## Reporting a vulnerability

Do not open a public issue for a security problem. Report it by email to
**support@dbinvent.com**, with "YT light security" in the subject, and include:

- the version (`yt-light -V`, or `select extversion from pg_extension where extname =
  'yt_cluster'` for the extension) and how it was installed;
- what an attacker can do, and the steps or a proof of concept that show it;
- any fix or mitigation you know of.

We acknowledge the report, keep you informed while we fix it, and credit you in the
release notes unless you ask us not to. Please give us the time to release a fix before
you disclose the problem publicly.

## Dependencies

The crates' dependencies are pinned in `Cargo.lock`. On every push and weekly,
`cargo audit` checks them against the RustSec advisory database and Trivy scans the
repository (`.github/workflows/audit.yml`, `trivy.yml`); a finding accepted is listed in
`.trivyignore` with why.
