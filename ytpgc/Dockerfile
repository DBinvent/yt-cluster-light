# SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
# ─── Build context: the repo root (yt-cluster-light/) ────────────────────────
# Generic parametric Dockerfile - override via --build-arg (defaults: PG 16, Ubuntu 22.04)
#
# Build:
#   docker build --build-arg PG_MAJOR=16 -f ytpgc/Dockerfile -t yt-cluster-pg16 .
#   docker build --build-arg PG_MAJOR=17 --build-arg BASE_IMAGE=ubuntu:24.04 -f ytpgc/Dockerfile -t yt-cluster-pg17 .
# Extract artifacts to ./dist/:
#   docker run --rm -v "$(pwd)/dist:/dist" yt-cluster-pg16 sh -c "cp /output/* /dist/"
# ─────────────────────────────────────────────────────────────────────────────

ARG BASE_IMAGE=ubuntu:22.04
ARG PG_MAJOR=16
FROM ${BASE_IMAGE}
ARG PG_MAJOR

ENV DEBIAN_FRONTEND=noninteractive

# ── System tools + PGDG apt repository ───────────────────────────────────────
RUN apt-get update && apt-get install -y --no-install-recommends \
        curl ca-certificates gnupg lsb-release \
    && curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc \
       | gpg --dearmor -o /usr/share/keyrings/postgresql.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/postgresql.gpg] \
https://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" \
       > /etc/apt/sources.list.d/pgdg.list \
    && rm -rf /var/lib/apt/lists/*

# ── Build dependencies ────────────────────────────────────────────────────────
# postgresql-server-dev-N  PGXS build infra, PG headers, pg_config
# protobuf-compiler        protoc - generates .pb-c.{c,h} via --c_out plugin
# protobuf-c-compiler      protoc-gen-c plugin required by protoc --c_out
# libprotobuf-c-dev        <protobuf-c/protobuf-c.h>; links as -lprotobuf-c
# libcurl4-openssl-dev     <curl/curl.h>; links as -lcurl (HTTP/2 gRPC transport)
# build-essential          gcc, make, ar, binutils, sed
#
# A major PGDG has not released yet - its betas and release candidates - is in a
# component of its own, named by the major, not in `main`. It is added only when `main`
# has no server-dev package for this major: a released one is always built from `main`.
RUN apt-get update \
    && if ! apt-cache show "postgresql-server-dev-${PG_MAJOR}" >/dev/null 2>&1; then \
           echo "PostgreSQL ${PG_MAJOR} is not in PGDG's main yet: built from its pre-release component, ${PG_MAJOR}" \
           && sed -i "s/-pgdg main\$/-pgdg main ${PG_MAJOR}/" /etc/apt/sources.list.d/pgdg.list \
           && apt-get update; \
       fi \
    && apt-get install -y --no-install-recommends \
        build-essential \
        postgresql-server-dev-${PG_MAJOR} \
        protobuf-compiler \
        protobuf-c-compiler \
        libprotobuf-c-dev \
        libcurl4-openssl-dev \
    && rm -rf /var/lib/apt/lists/*

# ── The build as a user of its own ────────────────────────────────────────────
# root installs the packages above; the sources, the compile and the artifacts are an
# unprivileged user's (Trivy DS-0002: no image of these builds as root)
RUN useradd --create-home --uid 10001 builder \
    && mkdir -p /build /output && chown builder:builder /build /output
USER builder
WORKDIR /build

# ── Copy project sources (context = repo root) ────────────────────────────────
# VERSION                        version string → YT_CLUSTER_VERSION in Makefile
# yt-core-proto/proto/           base.proto, db.proto and the copy of diff-doc-rs's
#                                diff_doc.proto → generated .pb-c.{c,h}
# ytpgc/                       extension C source, Makefile, SQL, control.in
COPY --chown=builder:builder VERSION                       VERSION
COPY --chown=builder:builder yt-core-proto/proto           yt-core-proto/proto
COPY --chown=builder:builder ytpgc                         ytpgc

WORKDIR /build/ytpgc

# ── Build ─────────────────────────────────────────────────────────────────────
# make   → yt_cluster.so (PGXS), versioned yt_cluster*-pgN-glibcX_Y.so, yt_cluster.control
# ar     → libyt_cluster.a static archive of all object files
# final  → /output/ collects all distributable artifacts
RUN make clean && make \
 && mkdir -p /output \
 && cp yt_cluster*.so libyt_cluster*.a /output/ \
 && echo "=== Artifacts for PostgreSQL ${PG_MAJOR} ===" \
 && ls -lah /output/

CMD ["ls", "-lah", "/output/"]
