#!/usr/bin/env bash
# SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
# Build yt_cluster extension for one or all PostgreSQL major versions.
# Run from the repo root:
#   bash ytpgc/build-all-ytpgc.sh           # all versions from pg-versions.txt
#   bash ytpgc/build-all-ytpgc.sh 16        # single version
#   bash ytpgc/build-all-ytpgc.sh 15 17     # specific subset
#   MUSL=1 bash ytpgc/build-all-ytpgc.sh 16 # also build musl variant
#   EL9=1 bash ytpgc/build-all-ytpgc.sh 16  # also build the glibc 2.34 variant
#                                           # (RHEL/Alma/Rocky 9, Amazon Linux 2023)
#   FORCE=1 bash ytpgc/build-all-ytpgc.sh   # rebuild even when artifacts exist
#
# Versioned artifacts (.so, .a) land flat in ytpgc/dist/. A version whose
# artifacts are already in dist/ is skipped - set FORCE=1 (or delete the
# files) to rebuild it.
#
# The extension's version is the major.minor of ../VERSION: it names the artifacts
# (yt_cluster<major>_<minor>-pg<N>-<libc>.so) and the control file's default_version.
# The control file is written here, on this host, from its template - a package takes
# it from ytpgc/ and no local `make` has to have run - and an artifact of ANOTHER
# version left in dist/ is removed: the packages take every yt_cluster*.so there, and
# two versions of the library for one PostgreSQL are one too many.
set -euo pipefail
# The images build on the daemon DOCKER_HOST names, this host's own when it is unset
# (ssh://<host> for another box's); the artifacts come back here either way.

# musl variant is opt-in: set MUSL=1 to enable
BUILD_MUSL="${MUSL:-0}"
# glibc 2.34 variant (ytpgc/Dockerfile.el9, PGDG yum) is opt-in: set EL9=1
BUILD_EL9="${EL9:-0}"
FORCE="${FORCE:-0}"

if ! docker info > /dev/null 2>&1; then
    echo "ERROR: cannot connect to Docker. Add your user to the docker group and re-login:" >&2
    echo "  sudo usermod -aG docker \$USER" >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VERSIONS_FILE="$SCRIPT_DIR/pg-versions.txt"
DIST_BASE="$REPO_ROOT/ytpgc/dist"

# Use CLI args when provided, otherwise fall back to pg-versions.txt
if [ $# -gt 0 ]; then
    VERSIONS=("$@")
else
    if [ ! -f "$VERSIONS_FILE" ]; then
        echo "ERROR: $VERSIONS_FILE not found" >&2
        exit 1
    fi
    mapfile -t VERSIONS < <(grep -v '^\s*#' "$VERSIONS_FILE" | grep -v '^\s*$')
fi

mkdir -p "$DIST_BASE"

# ── the extension's version, its control file, and no other version's artifacts ──
FULL_VERSION="$(tr -d ' \n' < "$REPO_ROOT/VERSION")"
[[ "$FULL_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
    || { echo "ERROR: $REPO_ROOT/VERSION is no <major>.<minor>.<patch>: [$FULL_VERSION]" >&2; exit 1; }
EXT_VERSION="$(cut -d. -f1,2 <<< "$FULL_VERSION")"
EXT_SHORT="${EXT_VERSION//./_}"
[ -f "$SCRIPT_DIR/yt_cluster--$EXT_VERSION.sql" ] \
    || { echo "ERROR: no $SCRIPT_DIR/yt_cluster--$EXT_VERSION.sql - the script of the version VERSION names" >&2; exit 1; }
# an update script from every earlier version, or a database that has one stays at it: the
# packages take every yt_cluster--*.sql here, and would take none without a word
while IFS= read -r earlier; do
    earlier="${earlier%%#*}"; earlier="${earlier//[[:space:]]/}"
    [ -n "$earlier" ] || continue
    [ -f "$SCRIPT_DIR/yt_cluster--$earlier--$EXT_VERSION.sql" ] \
        || { echo "ERROR: no $SCRIPT_DIR/yt_cluster--$earlier--$EXT_VERSION.sql (previous-versions.txt lists $earlier) - make update-sql" >&2; exit 1; }
done < "$SCRIPT_DIR/previous-versions.txt"
sed -e "s/@YT_CLUSTER_VERSION@/$EXT_VERSION/g" "$SCRIPT_DIR/yt_cluster.control.in" > "$SCRIPT_DIR/yt_cluster.control"
for f in "$DIST_BASE"/yt_cluster[0-9]*-pg*.so "$DIST_BASE"/libyt_cluster[0-9]*-pg*.a; do
    [ -e "$f" ] || continue
    case "$(basename "$f")" in
        "yt_cluster${EXT_SHORT}-pg"*|"libyt_cluster${EXT_SHORT}-pg"*) ;;
        *) echo "  another version's artifact, removed: $(basename "$f")"; rm -f "$f" ;;
    esac
done

echo "Build context: $REPO_ROOT"
echo "Extension:     $EXT_VERSION (yt_cluster${EXT_SHORT}-pg<N>-<libc>.so, yt_cluster.control written)"
echo "Versions:      ${VERSIONS[*]}"
echo "Artifacts:     $DIST_BASE/"
echo "musl variant:  $([ "$BUILD_MUSL" = "1" ] && echo "enabled (MUSL=1)" || echo "disabled (set MUSL=1 to enable)")"
echo "EL9 variant:   $([ "$BUILD_EL9" = "1" ] && echo "enabled (EL9=1): glibc 2.34" || echo "disabled (set EL9=1 to enable)")"
echo ""

FAILED=""

for PG_MAJOR in "${VERSIONS[@]}"; do

    IMAGE="yt-cluster-pg${PG_MAJOR}"

    if [ "${PG_MAJOR}" -ge 17 ]; then
        BASE_IMAGE="ubuntu:24.04"
        GLIBC_TAG="2_39"
    else
        # jammy (22.04) is the oldest Ubuntu with an active PGDG apt repo;
        # focal (20.04) was dropped from PGDG in 2025.
        BASE_IMAGE="ubuntu:22.04"
        GLIBC_TAG="2_35"
    fi

    # Alpine version that ships postgresql${PG_MAJOR}-dev in its standard repo:
    #   3.21 → PG 15-17  |  3.20 → PG 14-16
    if [ "${PG_MAJOR}" -ge 15 ]; then
        ALPINE_TAG="3.21"
    else
        ALPINE_TAG="3.20"
    fi

    echo "──────────────────────────────────────────"
    echo "  PostgreSQL ${PG_MAJOR}  →  ${BASE_IMAGE}  →  image: ${IMAGE}"
    [ "$BUILD_MUSL" = "1" ] && echo "  musl: alpine:${ALPINE_TAG}"
    echo "──────────────────────────────────────────"

    if [ "$FORCE" != "1" ] \
            && [ -f "$DIST_BASE/yt_cluster${EXT_SHORT}-pg${PG_MAJOR}-glibc${GLIBC_TAG}.so" ] \
            && [ -f "$DIST_BASE/libyt_cluster${EXT_SHORT}-pg${PG_MAJOR}-glibc${GLIBC_TAG}.a" ]; then
        echo "  SKIP: artifacts already in dist (FORCE=1 to rebuild):"
        ls -lah "$DIST_BASE/"yt_cluster*-pg${PG_MAJOR}-glibc${GLIBC_TAG}.so "$DIST_BASE/"libyt_cluster*-pg${PG_MAJOR}-glibc${GLIBC_TAG}.a
    elif docker build \
            -f "$SCRIPT_DIR/Dockerfile" \
            --build-arg PG_MAJOR="${PG_MAJOR}" \
            --build-arg BASE_IMAGE="${BASE_IMAGE}" \
            -t "$IMAGE" \
            "$REPO_ROOT"; then

        # Extract with docker cp, NOT a -v bind mount: a bind mount resolves on
        # the DAEMON host, so with a remote DOCKER_HOST (e.g. ssh://<host>) the
        # artifacts are silently written on that box and never arrive here.
        # docker cp streams back over the same transport and works either way.
        cid=$(docker create "$IMAGE")
        docker cp "$cid:/output/." "$DIST_BASE/"
        docker rm "$cid" > /dev/null
        chmod a+rw "$DIST_BASE"/yt_cluster*.so "$DIST_BASE"/libyt_cluster*.a 2>/dev/null || true
        chmod a+rwx "$DIST_BASE"

        echo "  Artifacts:"
        ls -lah "$DIST_BASE/"yt_cluster*-pg${PG_MAJOR}-* "$DIST_BASE/"libyt_cluster*-pg${PG_MAJOR}-* 2>/dev/null || true
    else
        echo "  BUILD FAILED for PostgreSQL ${PG_MAJOR}" >&2
        FAILED="$FAILED pg${PG_MAJOR}"
    fi

    # ── EL9 variant (glibc 2.34: RHEL 9 family, Amazon Linux 2023) ──────────
    if [ "$BUILD_EL9" = "1" ]; then
        EL9_IMAGE="${IMAGE}-el9"
        echo "  Building EL9 variant → image: ${EL9_IMAGE}"
        if [ "$FORCE" != "1" ] \
                && [ -f "$DIST_BASE/yt_cluster${EXT_SHORT}-pg${PG_MAJOR}-glibc2_34.so" ] \
                && [ -f "$DIST_BASE/libyt_cluster${EXT_SHORT}-pg${PG_MAJOR}-glibc2_34.a" ]; then
            echo "  SKIP: EL9 artifacts already in dist (FORCE=1 to rebuild):"
            ls -lah "$DIST_BASE/"yt_cluster*-pg${PG_MAJOR}-glibc2_34.so "$DIST_BASE/"libyt_cluster*-pg${PG_MAJOR}-glibc2_34.a
        elif docker build \
                -f "$SCRIPT_DIR/Dockerfile.el9" \
                --build-arg PG_MAJOR="${PG_MAJOR}" \
                -t "$EL9_IMAGE" \
                "$REPO_ROOT"; then

            # docker cp, not -v: see the glibc variant above (remote daemon)
            cid=$(docker create "$EL9_IMAGE")
            docker cp "$cid:/output/." "$DIST_BASE/"
            docker rm "$cid" > /dev/null
            chmod a+rw "$DIST_BASE"/yt_cluster*.so "$DIST_BASE"/libyt_cluster*.a 2>/dev/null || true

            echo "  EL9 Artifacts:"
            ls -lah "$DIST_BASE/"yt_cluster*-pg${PG_MAJOR}-glibc2_34* "$DIST_BASE/"libyt_cluster*-pg${PG_MAJOR}-glibc2_34* 2>/dev/null || true
        else
            echo "  EL9 BUILD FAILED for PostgreSQL ${PG_MAJOR}" >&2
            FAILED="$FAILED pg${PG_MAJOR}-el9"
        fi
    fi

    # ── musl variant (Alpine/musl PG deployments, .so only, no .a) ───────────
    if [ "$BUILD_MUSL" = "1" ]; then
        MUSL_IMAGE="${IMAGE}-musl"
        echo "  Building musl variant → image: ${MUSL_IMAGE}"
        if [ "$FORCE" != "1" ] \
                && compgen -G "$DIST_BASE/yt_cluster${EXT_SHORT}-pg${PG_MAJOR}-musl*.so" > /dev/null; then
            echo "  SKIP: musl artifact already in dist (FORCE=1 to rebuild):"
            ls -lah "$DIST_BASE/"yt_cluster*-pg${PG_MAJOR}-musl*.so
        elif docker build \
                -f "$SCRIPT_DIR/Dockerfile.musl" \
                --build-arg PG_MAJOR="${PG_MAJOR}" \
                --build-arg ALPINE_TAG="${ALPINE_TAG}" \
                -t "$MUSL_IMAGE" \
                "$REPO_ROOT"; then

            # docker cp, not -v: see the glibc variant above (remote daemon)
            cid=$(docker create "$MUSL_IMAGE")
            docker cp "$cid:/output/." "$DIST_BASE/"
            docker rm "$cid" > /dev/null
            chmod a+rw "$DIST_BASE"/yt_cluster*.so 2>/dev/null || true

            echo "  musl Artifacts:"
            ls -lah "$DIST_BASE/"yt_cluster*-pg${PG_MAJOR}-musl* 2>/dev/null || true
        else
            echo "  musl BUILD FAILED for PostgreSQL ${PG_MAJOR} (non-fatal)" >&2
            FAILED="$FAILED pg${PG_MAJOR}-musl"
        fi
    fi

    echo ""
done

echo "══════════════════════════════════════════"
if [ -n "$FAILED" ]; then
    echo "  FAILED versions:$FAILED"
    exit 1
else
    echo "  Built: ${VERSIONS[*]}"
    echo "  Artifacts in $DIST_BASE/:"; ls -lah "$DIST_BASE/"
fi
