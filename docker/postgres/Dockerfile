# syntax=docker/dockerfile:1
#
# Preconfigured PostgreSQL 18 for ferroehr (mirrors the official
# `ehrbase/ehrbase-v2-postgres` two-image model, built fresh for this stack).
#
# The stock postgres:18.6 image, plus init scripts that create the app login
# role, its owned database, the `ehr` + `ext` schemas it owns, and the required
# extensions installed by the bootstrap superuser — so the app role never needs
# superuser. Schema CONTENT is owned by the app's sqlx migrators at boot; this
# image bakes NO migration state (see README.md).
#
# Digest-pinned (resolved 2026-08-31, the upstream same-version respin): a
# mutable tag can be re-pointed, a digest cannot
# (docs.docker.com/build/building/best-practices — "pinning to a digest
# guarantees the same image version"). 18.6 is the latest PG 18 patch
# (docs/VERSIONS.md; 18.5 was never released upstream); bump the tag AND the
# digest together — the full pin-site checklist is
# .claude/rules/image-security.md.
FROM postgres:18.6@sha256:5a5a84b19854a9ffaa54082c166ff4ec27473a361e496e5ea167f298f2da9722

# Debian security updates, applied at OUR build time (issue #2408). The
# upstream image rebuilds on its own cadence, so a fix already published in
# trixie-security can sit unpulled for weeks in the pinned base — the
# 2026-08-13 build of postgres:18.6 shipped util-linux packages whose
# CVE-2026-53615 fix trixie-security already carried. Plain `upgrade`, never
# `dist-upgrade`: packages are only patched, none added or removed.
RUN set -eux; \
    apt-get update; \
    apt-get -y upgrade; \
    rm -rf /var/lib/apt/lists/*

# Init scripts run once, on an empty data directory, as the bootstrap superuser.
COPY initdb/ /docker-entrypoint-initdb.d/

# See docker/Dockerfile for the split: build-independent keys here, the four
# build-dependent ones from the publishing workflow. base.* names the `FROM`
# above, kept in step by scripts/checks/image-labels.sh.
#
# `licenses` is an SPDX EXPRESSION and this image genuinely is one: our init
# scripts are BUSL-1.1, and every layer under them is upstream `postgres`, which
# is PostgreSQL-licensed. Claiming only BUSL-1.1 would understate what the image
# contains.
LABEL org.opencontainers.image.title="FerroEHR PostgreSQL" \
      org.opencontainers.image.description="PostgreSQL 18 preconfigured (role/db/schemas/extensions) for ferroehr" \
      org.opencontainers.image.url="https://ferroehr.eu/" \
      org.opencontainers.image.documentation="https://ferroehr.eu/docs/latest/" \
      org.opencontainers.image.source="https://github.com/FerroHEALTH/FerroEHR" \
      org.opencontainers.image.vendor="FerroHEALTH" \
      org.opencontainers.image.authors="FerroHEALTH <https://github.com/FerroHEALTH>" \
      org.opencontainers.image.licenses="BUSL-1.1 AND PostgreSQL" \
      org.opencontainers.image.base.name="postgres:18.6" \
      org.opencontainers.image.base.digest="sha256:5a5a84b19854a9ffaa54082c166ff4ec27473a361e496e5ea167f298f2da9722"
