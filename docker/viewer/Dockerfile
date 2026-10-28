# syntax=docker/dockerfile:1
#
# The single Dockerfile for the `ferroehr-viewer` image (issue #282
# D1/D5) — the third OCI image next to the app + postgres images. The viewer
# is a standalone Leptos SSR binary + WASM/CSS assets built by cargo-leptos; at
# runtime it is a pure ITS-REST client of the CDR — no database.
#
# Same two-target shape as docker/Dockerfile (docs.docker.com/build/building/multi-stage):
#
#   runtime-from-source  compose/local — one builder RUN under a persistent
#                         BuildKit cache mount on /app/target (the official
#                         Rust caching pattern, docs.docker.com/guides/rust/
#                         develop — cache mounts for target + registry), so a
#                         rebuild recompiles only what changed. cargo-chef was
#                         removed here for the same reason docker/Dockerfile
#                         dropped it (#2530): its own README documents that
#                         WORKSPACE member crates cannot survive the COPY
#                         boundary (timestamp fingerprinting), so every source
#                         change recompiled the whole dependency graph anyway.
#                         The mount path is the SAME as the server image's, so
#                         the one workspace's artifacts are shared across both
#                         image builds and `sharing=locked` serializes their
#                         cargo phases — bounding peak memory when compose
#                         builds both images at once.
#   runtime-prebuilt     CI — COPY-only packaging of a context-provided native
#                         binary (bin/${TARGETARCH}/ferroehr-viewer) + the
#                         arch-independent site bundle (site/); no RUN, no QEMU.
#
# Both share ONE runtime-base stage, so the compose-built and CI-published
# images cannot drift (D5 — the F8 divergence where only the CI image baked a
# HEALTHCHECK is closed: the shared base bakes it once).
#
# RUST_VERSION is single-sourced from rust-toolchain.toml. The build TOOLS are
# fetched as PREBUILT release binaries, never `cargo install cargo-leptos`
# (that compiles a vendored OpenSSL C tree — ~10 min per cold build); the same
# prebuilt path the CI workflows use.

ARG RUST_VERSION=1.99.1
ARG CARGO_LEPTOS_VERSION=0.3.11

# ── builder: toolchain + prebuilt tools, then cargo-leptos under the cache ─────
# Digest-pinned rust:1.99.1-slim-trixie (Debian 13, glibc 2.41), resolved
# 2026-10-21 — same base + rationale as docker/Dockerfile
# (docs.docker.com/build/building/best-practices — pin by digest).
FROM rust:${RUST_VERSION}-slim-trixie@sha256:00544ccc9431a2fe93e65185d21381b0a039342f5cefcf84fbddac273c9e573d AS builder
WORKDIR /app
ARG CARGO_LEPTOS_VERSION
# amd64 | arm64 — provided by buildx/compose.
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown
# cargo-auditable embeds the compressed dependency list in the binary's own
# `.dep-v0` section, so a scan of this image sees the viewer's Rust
# dependencies (#3675; https://github.com/rust-secure-code/cargo-auditable).
# Same pin and install as docker/Dockerfile. It runs before the tool fetch
# below, whose layer is re-run whenever Cargo.lock changes.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo install cargo-auditable --version 0.7.5 --locked
# Prebuilt tool binaries fetched behind a cache mount (F13): the downloaded
# artifacts persist across builds keyed by version, so a rebuild that did not
# bump a tool version re-uses them instead of re-downloading
# (docs.docker.com/build/cache/optimize — RUN --mount=type=cache). cargo-leptos
# resolves wasm-bindgen from PATH before its own downloader.
# The wasm-bindgen CLI must be the exact version of the wasm-bindgen library
# the viewer links, so its version is read from the bind-mounted Cargo.lock
# rather than pinned here (#4222); a lock with no single wasm-bindgen fails
# the build. scripts/checks/viewer-tool-pins.sh holds every other pin to it.
# `sh` on Debian is dash, which has no `pipefail`, so the tool downloads below
# need bash: each is `curl … | tar … > file`, and without pipefail a failed
# curl leaves tar's exit status to decide. Each download lands in a `.part`
# file and moves into the cache only on success, so a failed fetch never
# leaves a truncated binary for the next build's cache check to `install`.
# wasm-bindgen ships its Linux CLI as a static musl build only on x86_64, so
# its download takes the musl triple on both architectures.
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
RUN --mount=type=cache,target=/opt/tool-cache,sharing=locked \
    --mount=type=bind,source=Cargo.lock,target=/tmp/Cargo.lock \
    set -eu; \
    WASM_BINDGEN_VERSION="$(sed -n '/^name = "wasm-bindgen"$/{n;s/^version = "\(.*\)"$/\1/p;}' /tmp/Cargo.lock)"; \
    if [[ ! "${WASM_BINDGEN_VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then \
      echo "Cargo.lock resolves no single wasm-bindgen version: '${WASM_BINDGEN_VERSION}'" >&2; exit 1; \
    fi; \
    case "${TARGETARCH}" in \
      amd64) arch=x86_64 ;; \
      arm64) arch=aarch64 ;; \
      *) echo "unsupported TARGETARCH ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    triple="${arch}-unknown-linux-gnu"; wasm_triple="${arch}-unknown-linux-musl"; \
    cache="/opt/tool-cache/${triple}"; mkdir -p "$cache"; \
    if [ ! -f "$cache/cargo-leptos-${CARGO_LEPTOS_VERSION}" ]; then \
      curl -sSfL "https://github.com/leptos-rs/cargo-leptos/releases/download/v${CARGO_LEPTOS_VERSION}/cargo-leptos-${triple}.tar.gz" \
        | tar -xz -O "cargo-leptos-${triple}/cargo-leptos" > "$cache/cargo-leptos.part" \
        && mv "$cache/cargo-leptos.part" "$cache/cargo-leptos-${CARGO_LEPTOS_VERSION}"; \
    fi; \
    if [ ! -f "$cache/wasm-bindgen-${WASM_BINDGEN_VERSION}" ]; then \
      curl -sSfL "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${WASM_BINDGEN_VERSION}/wasm-bindgen-${WASM_BINDGEN_VERSION}-${wasm_triple}.tar.gz" \
        | tar -xz -O "wasm-bindgen-${WASM_BINDGEN_VERSION}-${wasm_triple}/wasm-bindgen" > "$cache/wasm-bindgen.part" \
        && mv "$cache/wasm-bindgen.part" "$cache/wasm-bindgen-${WASM_BINDGEN_VERSION}"; \
    fi; \
    install -m0755 "$cache/cargo-leptos-${CARGO_LEPTOS_VERSION}" /usr/local/bin/cargo-leptos; \
    install -m0755 "$cache/wasm-bindgen-${WASM_BINDGEN_VERSION}" /usr/local/bin/wasm-bindgen
COPY . .
# The viewer has no provenance build.rs, so it carries no compile-time SHA env;
# its image provenance is the OCI-standard REVISION LABEL on the shared runtime
# stage below (docs.docker.com/build/building/best-practices).
# cargo-leptos builds the server binary with the command this variable names
# instead of `cargo` (cargo-leptos 0.3.11, `bin-cargo-command`, read from the
# environment in src/config/dotenvs.rs), so the server leg runs
# `cargo auditable build`; the wasm leg stays plain cargo and ships no binary.
ENV LEPTOS_BIN_CARGO_COMMAND="cargo auditable"
# Serial-compile escape hatch for memory-constrained hosts, same knob as
# docker/Dockerfile (cargo reads CARGO_BUILD_JOBS from the environment —
# doc.rust-lang.org/cargo/reference/environment-variables.html). The default
# stays 2 (adjudicated on #2883): every CI lane builds at 2, and jobs=1 would
# roughly double every build to spare the rare constrained host. A cold build
# OOM-killed rustc at 2 on an 8-core/24 GB arm64 host that was also running a
# stack (#2881); `CARGO_BUILD_JOBS=1 docker compose … build` completed there —
# the compose dev overlay forwards the variable.
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}
# `cargo leptos` reads its configuration from the viewer crate's own manifest
# directory, so the build runs from there — as a WORKDIR rather than a `cd`
# inside RUN, which would silently not apply to any later instruction.
WORKDIR /app/app/ferroehr-viewer
# The build and the copy-out share ONE RUN because /app/target is a MOUNT: it
# exists only while a RUN that declares it executes, so artifacts must be
# copied into the image filesystem before the instruction ends
# (docs.docker.com/guides/rust/develop — the same shape as docker/Dockerfile).
# `sharing=locked` serializes every cargo build over the one workspace cache —
# including the server image's, when compose builds both images at once.
#
# The build runs through scripts/cargo-leptos.sh, which freezes Cargo.lock —
# `cargo leptos` re-resolves it otherwise, and an image build silently
# re-resolving the workspace is a build nobody can reproduce (#2877).
#
# hash.txt is the content-hash manifest cargo-leptos writes beside the server
# binary; it travels WITH the site bundle it describes, and the runtime stages
# put it back beside the binary, which is the only place leptos reads it from.
RUN --mount=type=cache,target=/app/target,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    bash /app/scripts/cargo-leptos.sh build --release \
    && mkdir -p /out \
    && cp /app/target/release/ferroehr-viewer /out/ \
    && cp -r /app/target/site /out/site \
    && cp /app/target/release/hash.txt /out/site/hash.txt

# ── runtime-base: the SHARED final base (digest-pinned distroless, non-root) ───
# gcr.io/distroless/cc-debian13:nonroot, digest-pinned, resolved 2026-10-22 —
# same base + `cc`-not-base-nossl rationale (libgcc_s.so.1) as docker/Dockerfile.
# The HEALTHCHECK the previous split baked ONLY into the CI image (F8) is baked
# here once, so both targets carry it identically.
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:3ee00291cf386ec52a552e6a858d555debd74776eb4bfa53fa3df2cd6677515e AS runtime-base
# The `:nonroot` tag already runs as uid 65532, but a scanner reading the
# Dockerfile cannot see a base image.s user — and neither can a reader. Stating
# it makes the posture a property of this file rather than of a tag someone
# could change (OWASP Docker Security Cheat Sheet rule 2).
#
# NUMERIC, not `nonroot:nonroot`: the kubelet cannot resolve a username against
# an image it does not read, so `runAsNonRoot: true` with a named USER refuses
# the pod with "cannot verify user is non-root" (hadolint DL3066). 65532 is what
# the distroless `:nonroot` tag sets, so this states the base uid rather than
# choosing one.
USER 65532:65532
# LEPTOS_HASH_FILES is the RUNTIME half of the crate manifest's `hash-files`:
# leptos_config reads it from the environment at startup (there is no
# compile-time fallback the way LEPTOS_OUTPUT_NAME has one), and it is what
# makes the renderer name the content-hashed /pkg files this image ships
# instead of the unhashed names it would otherwise 404 on. It must stay in
# step with the manifest key and with hash.txt below — all three or none.
# The listen address is the viewer's own `server.listen`, on every interface
# inside the container.
ENV LEPTOS_SITE_ROOT=/site \
    LEPTOS_OUTPUT_NAME=ferroehr-viewer \
    LEPTOS_HASH_FILES=true \
    FERROEHR_VIEWER__SERVER__LISTEN=0.0.0.0:3000
EXPOSE 3000
# Shell-less image: the healthcheck is the binary's own subcommand, which asks
# its own /health/readiness and exits 0 on 200, 1 otherwise.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD ["/usr/local/bin/ferroehr-viewer", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/ferroehr-viewer"]
# OCI image labels. CI (docker/metadata-action) overrides source/revision/
# version/created at build time; these are sensible static defaults.
ARG VERSION="0.1.0"
ARG REVISION=""
# See docker/Dockerfile for the split: build-independent keys here, the four
# build-dependent ones from the publishing workflow. base.* names this runtime
# stage's `FROM`, kept in step by scripts/checks/image-labels.sh.
LABEL org.opencontainers.image.title="FerroEHR Viewer" \
      org.opencontainers.image.description="Viewer for openEHR ITS-REST CDRs — pure-Rust Leptos web app (template manager, visual AQL query builder, EHR browser)" \
      org.opencontainers.image.url="https://ferroehr.eu/" \
      org.opencontainers.image.documentation="https://ferroehr.eu/docs/latest/" \
      org.opencontainers.image.source="https://github.com/FerroHEALTH/FerroEHR" \
      org.opencontainers.image.vendor="Cadasto B.V." \
      org.opencontainers.image.authors="Cadasto B.V. <https://www.cadasto.com/contact/>" \
      org.opencontainers.image.licenses="BUSL-1.1" \
      org.opencontainers.image.base.name="gcr.io/distroless/cc-debian13:nonroot" \
      org.opencontainers.image.base.digest="sha256:83b9b50586253821cf63ffcec87d3d94265a093ad30da232039af962dd8f4edf" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      eu.ferroehr.image.manufacturer="Cadasto B.V., Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands, info@cadasto.com" \
      eu.ferroehr.image.accompanying-documents="Information sheet and instructions for use: the assets ferroehr-v<version>-information-sheet.md and ferroehr-v<version>-instructions-for-use.md of https://github.com/FerroHEALTH/FerroEHR/releases/tag/v<version>, where <version> is this image's org.opencontainers.image.version"

# ── runtime-from-source: compose/local (COPY the in-image build) ───────────────
FROM runtime-base AS runtime-from-source
COPY --from=builder /out/ferroehr-viewer /usr/local/bin/ferroehr-viewer
COPY --from=builder /out/site /site
# The manifest travels inside the site bundle it describes; leptos resolves it
# as current_exe().parent()/hash.txt, so it is placed beside the binary too.
COPY --from=builder /out/site/hash.txt /usr/local/bin/hash.txt

# ── runtime-prebuilt: CI (COPY the context-provided native binary + site) ──────
# The server binary is built natively per architecture on the CI runners
# (rust:*-slim-trixie → Debian 13 glibc 2.41, matching this base) and lands in
# the context as bin/${TARGETARCH}/ferroehr-viewer; the site bundle (WASM + JS
# glue + CSS) is architecture-independent, built once, and lands as site/. No
# RUN executes, so a multi-arch buildx needs no QEMU.
FROM runtime-base AS runtime-prebuilt
# amd64 | arm64 — supplied by buildx per platform.
ARG TARGETARCH
COPY --chmod=0755 bin/${TARGETARCH}/ferroehr-viewer /usr/local/bin/ferroehr-viewer
COPY site /site
# Same placement as the from-source stage: the manifest rides the
# (arch-independent) site artifact and is copied beside the binary, which is
# where leptos looks for it.
COPY site/hash.txt /usr/local/bin/hash.txt
