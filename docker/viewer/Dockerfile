# syntax=docker/dockerfile:1
#
# The single Dockerfile for the `ferroehr-viewer` image (issue #282
# D1/D5), the third OCI image next to the app and postgres images. The viewer
# is one server binary that draws every page itself and serves the FerroUI
# stylesheet, theme and scripts compiled into it; at runtime it is a pure
# ITS-REST client of the CDR, with no database.
#
# Same two-target shape as docker/Dockerfile (docs.docker.com/build/building/multi-stage):
#
#   runtime-from-source  compose/local: one builder RUN under a persistent
#                         BuildKit cache mount on /app/target (the official
#                         Rust caching pattern, docs.docker.com/guides/rust/
#                         develop), so a rebuild recompiles only what changed.
#                         The mount path is the SAME as the server image's, so
#                         the one workspace's artifacts are shared across both
#                         image builds and `sharing=locked` serializes their
#                         cargo phases, bounding peak memory when compose
#                         builds both images at once.
#   runtime-prebuilt     CI: COPY-only packaging of a context-provided native
#                         binary (bin/${TARGETARCH}/ferroehr-viewer); no RUN,
#                         no QEMU.
#
# Both share ONE runtime-base stage, so the compose-built and CI-published
# images cannot drift (D5: the shared base bakes the HEALTHCHECK once).
#
# RUST_VERSION is single-sourced from rust-toolchain.toml.

ARG RUST_VERSION=1.99.1

# ── builder: toolchain, then cargo under the cache ──────────────────────────────
# Digest-pinned rust:1.99.1-slim-trixie (Debian 13, glibc 2.41), resolved
# 2026-10-21, same base and rationale as docker/Dockerfile
# (docs.docker.com/build/building/best-practices, pin by digest).
FROM rust:${RUST_VERSION}-slim-trixie@sha256:00544ccc9431a2fe93e65185d21381b0a039342f5cefcf84fbddac273c9e573d AS builder
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config ca-certificates \
    && rm -rf /var/lib/apt/lists/*
# cargo-auditable embeds the compressed dependency list in the binary's own
# `.dep-v0` section, so a scan of this image sees the viewer's Rust
# dependencies (#3675; https://github.com/rust-secure-code/cargo-auditable).
# Same pin and install as docker/Dockerfile.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo install cargo-auditable --version 0.7.5 --locked
COPY . .
# The viewer has no provenance build.rs, so it carries no compile-time SHA env;
# its image provenance is the OCI-standard REVISION LABEL on the shared runtime
# stage below (docs.docker.com/build/building/best-practices).
# Serial-compile escape hatch for memory-constrained hosts, same knob as
# docker/Dockerfile (cargo reads CARGO_BUILD_JOBS from the environment,
# doc.rust-lang.org/cargo/reference/environment-variables.html). The default
# stays 2 (adjudicated on #2883): a cold build OOM-killed rustc at 2 on an
# 8-core/24 GB arm64 host that was also running a stack (#2881);
# `CARGO_BUILD_JOBS=1 docker compose … build` completed there, and the compose
# dev overlay forwards the variable.
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}
# The build and the copy-out share ONE RUN because /app/target is a MOUNT: it
# exists only while a RUN that declares it executes, so the binary must be
# copied into the image filesystem before the instruction ends
# (docs.docker.com/guides/rust/develop, the same shape as docker/Dockerfile).
# `--locked` holds the build to Cargo.lock (#2877).
RUN --mount=type=cache,target=/app/target,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo auditable build --release --locked -p ferroehr-viewer --bin ferroehr-viewer \
    && mkdir -p /out \
    && cp /app/target/release/ferroehr-viewer /out/

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
# The listen address is the viewer's own `server.listen`, on every interface
# inside the container.
ENV FERROEHR_VIEWER__SERVER__LISTEN=0.0.0.0:3000
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
      org.opencontainers.image.description="Viewer for openEHR ITS-REST CDRs — pure-Rust server-rendered web app (template manager, visual AQL query builder, EHR browser)" \
      org.opencontainers.image.url="https://ferroehr.eu/" \
      org.opencontainers.image.documentation="https://ferroehr.eu/docs/latest/" \
      org.opencontainers.image.source="https://github.com/FerroHEALTH/FerroEHR" \
      org.opencontainers.image.vendor="Cadasto B.V." \
      org.opencontainers.image.authors="Cadasto B.V. <https://www.cadasto.com/contact/>" \
      org.opencontainers.image.licenses="BUSL-1.1" \
      org.opencontainers.image.base.name="gcr.io/distroless/cc-debian13:nonroot" \
      org.opencontainers.image.base.digest="sha256:3ee00291cf386ec52a552e6a858d555debd74776eb4bfa53fa3df2cd6677515e" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      eu.ferroehr.image.manufacturer="Cadasto B.V., Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands, info@cadasto.com" \
      eu.ferroehr.image.accompanying-documents="Information sheet and instructions for use: the assets ferroehr-v<version>-information-sheet.md and ferroehr-v<version>-instructions-for-use.md of https://github.com/FerroHEALTH/FerroEHR/releases/tag/v<version>, where <version> is this image's org.opencontainers.image.version"

# ── runtime-from-source: compose/local (COPY the in-image build) ───────────────
FROM runtime-base AS runtime-from-source
COPY --from=builder /out/ferroehr-viewer /usr/local/bin/ferroehr-viewer

# ── runtime-prebuilt: CI (COPY the context-provided native binary) ─────────────
# The server binary is built natively per architecture on the CI runners
# (rust:*-slim-trixie, Debian 13 glibc 2.41, matching this base) and lands in
# the context as bin/${TARGETARCH}/ferroehr-viewer. No RUN executes, so a
# multi-arch buildx needs no QEMU.
FROM runtime-base AS runtime-prebuilt
# amd64 | arm64, supplied by buildx per platform.
ARG TARGETARCH
COPY --chmod=0755 bin/${TARGETARCH}/ferroehr-viewer /usr/local/bin/ferroehr-viewer
