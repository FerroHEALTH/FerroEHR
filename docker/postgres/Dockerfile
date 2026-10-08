# syntax=docker/dockerfile:1
#
# Preconfigured PostgreSQL 18 for ferroehr (mirrors the official
# `ehrbase/ehrbase-v2-postgres` two-image model, built fresh for this stack).
#
# The stock postgres:18.6 image with Debian's security updates applied, the
# build-time GnuPG toolchain purged, upstream's Go `gosu` replaced by a
# `setpriv` wrapper, plus init scripts that create the app login role, its
# owned database, the `ehr` + `ext` schemas it owns, and the required
# extensions installed by the bootstrap superuser — so the app role never needs
# superuser. Schema CONTENT is owned by the app's sqlx migrators at boot; this
# image bakes NO migration state (see README.md).
#
# Digest-pinned (resolved 2026-10-08, the upstream same-version respin of
# 2026-10-06): a
# mutable tag can be re-pointed, a digest cannot
# (docs.docker.com/build/building/best-practices — "pinning to a digest
# guarantees the same image version"). 18.6 is the latest PG 18 patch
# (docs/VERSIONS.md; 18.5 was never released upstream); bump the tag AND the
# digest together — the full pin-site checklist is
# .claude/rules/image-security.md.
FROM postgres:18.6@sha256:74935e72241653ca55e0414067e6d8763aceb8a810eb51b452253ec3dcfc4336

# Debian security updates, applied at OUR build time (issue #2408). The
# upstream image rebuilds on its own cadence, so a fix already published in
# trixie-security can sit unpulled for weeks in the pinned base — the
# 2026-08-13 build of postgres:18.6 shipped util-linux packages whose
# CVE-2026-53615 fix trixie-security already carried. Plain `upgrade`, never
# `dist-upgrade`: the upgrade only patches, it adds and removes nothing.
#
# Then one removal: GnuPG and what only it pulls in (gnupg, gnupg-l10n, gpg,
# gpg-agent, gpgconf, gpgsm, dirmngr, pinentry-curses, libassuan9, libgnutls30t64,
# libksba8, libnpth0t64, libp11-kit0, libtasn1-6). Upstream installs GnuPG to
# import the PGDG signing key while building its image; nothing in the running
# server, its entrypoint or postgresql-common calls it, and apt verifies with
# sqv, which stays.
RUN set -eux; \
    apt-get update; \
    apt-get -y upgrade; \
    apt-get purge -y --auto-remove \
        gnupg gnupg-l10n gpg gpg-agent gpgconf gpgsm dirmngr pinentry-curses; \
    for pkg in gnupg gpg dirmngr libgnutls30t64 libp11-kit0; do \
        status="$(dpkg-query -W -f '${db:Status-Status}' "$pkg" 2>/dev/null || :)"; \
        if [ "$status" = installed ]; then \
            echo "$pkg survived the purge" >&2; exit 1; \
        fi; \
    done; \
    rm -rf /var/lib/apt/lists/*; \
    rm /usr/local/bin/gosu

# gosu, the upstream entrypoint's privilege drop, as a POSIX-sh wrapper over
# util-linux `setpriv` (the wrapper's header says how it resolves the user). The
# RUN above deletes upstream's Go binary in its own layer, so no scanner reads
# it from a lower one; docker-entrypoint.sh calls the wrapper unmodified.
COPY --chmod=0755 gosu /usr/local/bin/gosu

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
      org.opencontainers.image.vendor="Cadasto B.V." \
      org.opencontainers.image.authors="Cadasto B.V. <https://www.cadasto.com/contact/>" \
      org.opencontainers.image.licenses="BUSL-1.1 AND PostgreSQL" \
      org.opencontainers.image.base.name="postgres:18.6" \
      org.opencontainers.image.base.digest="sha256:74935e72241653ca55e0414067e6d8763aceb8a810eb51b452253ec3dcfc4336" \
      eu.ferroehr.image.manufacturer="Cadasto B.V., Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands, info@cadasto.com" \
      eu.ferroehr.image.accompanying-documents="Information sheet and instructions for use: the assets ferroehr-v<version>-information-sheet.md and ferroehr-v<version>-instructions-for-use.md of https://github.com/FerroHEALTH/FerroEHR/releases/tag/v<version>, where <version> is this image's org.opencontainers.image.version"
