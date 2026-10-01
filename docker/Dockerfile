# syntax=docker/dockerfile:1
#
# The single Dockerfile for the `ferroehr` server image (issue #282 D1/D5).
# ONE runtime stage feeds TWO final targets so a compose-built image and a
# CI-published image can never drift (docs.docker.com/build/building/multi-stage):
#
#   runtime-from-source  the LOCAL/compose path — a persistent BuildKit target
#                         cache compiles the binary in-image.
#   runtime-prebuilt     the CI path — COPY-only packaging of a context-provided
#                         native binary (bin/${TARGETARCH}/ferroehr); no RUN, so a
#                         multi-arch buildx needs no QEMU and finishes in seconds.
#
# Caching (D1; re-grounded 2026-08-21, issue #2530): the whole cargo target
# directory lives in a `--mount=type=cache` that persists across builds on the
# builder host — the official Docker Rust pattern (docs.docker.com/guides/rust).
# Cargo's own freshness logic then decides what recompiles: an app-only edit
# rebuilds only the app crates, a `crates/` regeneration rebuilds the spec
# crates, and dependencies rebuild only on a lockfile change. The previous
# cargo-chef layering is GONE on the tool's own documented ground: chef caches
# EXTERNAL dependencies only, and its README states that local workspace
# crates "may rebuild from scratch due to [cargo's] timestamp-based
# fingerprinting logic, even if they are unchanged" — which made every edit
# recompile the eight generated openehr-* crates, the largest units in the
# tree. The exportable-image-layer rationale chef carried had no consumer:
# the only CI lane building this target (`server-image-from-source`) is a
# deliberate no-cache cold-compile proof.
#
# The RUST_VERSION toolchain is single-sourced from rust-toolchain.toml; CI
# cross-checks the ARG against its `channel` so they never drift.

ARG RUST_VERSION=1.98.1

# ── builder: compile in-image against the persistent target cache ──────────────
# Digest-pinned rust:1.98.1-slim-trixie (Debian 13, glibc 2.41), resolved
# 2026-09-10. The builder and the cc-debian13 runtime below share the Debian 13
# generation so the binary links the exact glibc it runs against. Pinning by
# digest (not the mutable tag) guarantees a reproducible base
# (docs.docker.com/build/building/best-practices — "pinning to a digest
# guarantees the same image version").
FROM rust:${RUST_VERSION}-slim-trixie@sha256:ce84a5edd80c5f91e05c5533b1e53eb1da54028f33734dc06aa6b49fa190462d AS builder
WORKDIR /app
COPY . .
# Build provenance: the OCI-standard REVISION build-arg (the git SHA CI's
# metadata-action already sets for org.opencontainers.image.revision) is the
# ONLY provenance input — no bespoke arg. Exposed as an ENV so build.rs reads it
# and stamps /management/info with the same SHA the image LABEL carries. Absent,
# build.rs degrades to a version-only identity with SHA=unknown BY DESIGN (never
# a broken build; .git is not in the context). An SHA change re-runs only the
# build RUN, whose compile state persists in the target cache mount anyway
# (docs.docker.com/reference/dockerfile — ARG/ENV scoping).
ARG REVISION=""
ENV REVISION=${REVISION}
# The binary is pure-Rust (rustls, no OpenSSL); the slim base needs no extra
# system libraries.
# Build parallelism is capped, and tunable. The generated spec crates carry
# every emitted generation (#1936/#1942), so a single rustc invocation on one of
# them at opt-level 3 holds gigabytes; running several alongside the app crates
# exhausts a small Docker VM and the kernel kills the compiler. That has now
# been observed twice at two different caps — SIGKILL on openehr-its at the
# default parallelism (7.7 GB VM, 2026-08-05), and again at `--jobs 2` once the
# spec-function work landed — so the number is a property of the machine, not a
# constant to keep re-tuning in this file. With the persistent target cache the
# cap bites only on cold or spec-crate-invalidating builds; warm rebuilds
# compile the app crates alone.
#
# `--build-arg CARGO_BUILD_JOBS=1` builds on a memory-constrained host. It costs
# wall-clock and changes no shipped byte: parallelism only decides how many
# translation units compile at once.
ARG CARGO_BUILD_JOBS=2
# cargo-auditable embeds the compressed dependency list in the binary's own
# `.dep-v0` linker section (allocated, so it SURVIVES the strip below —
# verified first-hand on a built binary), letting syft/trivy/grype/osv-scanner
# read the crate graph from the shipped artifact itself
# (https://github.com/rust-secure-code/cargo-auditable).
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo install cargo-auditable --version 0.7.5 --locked
# Both caches are mounts, per the official pattern (docs.docker.com/guides/rust):
# /app/target holds the compile state (fingerprints included, so cargo's
# freshness survives across builds — the thing an image layer can never give
# workspace crates), the registry mount holds downloaded crate sources.
# `sharing=locked` serializes concurrent image builds on the one cache. The
# binary is copied OUT of the mount in the same RUN — mounts are not part of
# the image, so nothing under /app/target survives past this step.
RUN --mount=type=cache,target=/app/target,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo auditable build --release --locked --jobs "${CARGO_BUILD_JOBS}" -p ferroehr-server \
    && cp target/release/ferroehr /usr/local/bin/ferroehr \
    && strip /usr/local/bin/ferroehr

# ── runtime-base: the SHARED final base (digest-pinned distroless, non-root) ───
# gcr.io/distroless/cc-debian13:nonroot (glibc + libgcc_s + CA certs; non-root
# 65532, Helm-aligned), digest-pinned, resolved 2026-09-01. `cc` (not the
# smaller base-nossl) is REQUIRED: the Rust binary dynamically links
# libgcc_s.so.1 for stack unwinding, which the base-nossl image does not ship
# (verified 2026-07-24: base-nossl fails at load with "libgcc_s.so.1: cannot
# open shared object file"); the ~11 MB base-nossl would save is unusable.
# Both final targets inherit this stage's EXPOSE/HEALTHCHECK/ENTRYPOINT/LABELs,
# so they are byte-for-byte identical except the one binary-provenance layer
# (D5 — zero drift).
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:54df941ed0d06a1bd95ef5e0ce391fd8d9f94b64782dc9a60062727849ee3f97 AS runtime-base
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
EXPOSE 8080
# Shell-less image: the healthcheck is the binary's own subcommand, which probes
# /rest/status and exits 0 (2xx) / 1 otherwise.
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD ["/usr/local/bin/ferroehr", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/ferroehr"]
# OCI image labels. CI (docker/metadata-action) overrides source/revision/
# version/created at build time; these are sensible static defaults.
ARG VERSION="0.1.0"
ARG REVISION=""
# The OCI predefined annotation keys (image-spec annotations.md). Split by who
# can know the value: everything build-INDEPENDENT is declared here so a direct
# `docker build` produces a correctly-labelled image, and the four
# build-DEPENDENT ones (created, version, revision, ref.name) come from the
# publishing workflow, which is the only place they exist.
#
# base.name/base.digest name the IMMEDIATE parent — the `FROM` of this runtime
# stage, not an intermediate build stage. They are kept in step with that pin by
# scripts/checks/image-labels.sh.
LABEL org.opencontainers.image.title="FerroEHR" \
      org.opencontainers.image.description="Pure-Rust, openEHR-conformant clinical data repository (ITS-REST 1.1.0 + AQL 1.1)" \
      org.opencontainers.image.url="https://ferroehr.eu/" \
      org.opencontainers.image.documentation="https://ferroehr.eu/docs/latest/" \
      org.opencontainers.image.source="https://github.com/FerroHEALTH/FerroEHR" \
      org.opencontainers.image.vendor="FerroEHR" \
      org.opencontainers.image.authors="Ruben Talstra <https://github.com/rubentalstra>" \
      org.opencontainers.image.licenses="BUSL-1.1" \
      org.opencontainers.image.base.name="gcr.io/distroless/cc-debian13:nonroot" \
      org.opencontainers.image.base.digest="sha256:54df941ed0d06a1bd95ef5e0ce391fd8d9f94b64782dc9a60062727849ee3f97" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      eu.ferroehr.image.carries-sandbox-posture="true"

# ── The hosted-sandbox posture travels in the image (#2974) ────────────────────
# The compose file, the Caddyfile and the two sandbox configuration files the
# Hetzner box runs (deploy/hosted/): the box holds no checkout, so its deploy
# script extracts these from the image it just pulled and installs them with a
# parse check and .prev rollback. A posture change therefore arrives with the
# release that carries it, with that artifact's provenance.
COPY deploy/hosted/docker-compose.yml deploy/hosted/Caddyfile \
     deploy/hosted/ferroehr.sandbox.toml deploy/hosted/ferroehr-viewer.sandbox.toml \
     /opt/sandbox-posture/
# The licence-free shaped terminology seed (docker/terminology/seed/) travels the
# same way: the sandbox and the terminology overlay mount this directory into
# FerroTERM straight from this image (a Compose `type: image` volume of the
# whole image, FERROTERM_CODESYSTEMS naming this directory in it), so a box that
# holds no checkout still serves the code systems the seed binds to.
COPY docker/terminology/seed/ /opt/sandbox-posture/terminology/

# ── runtime-from-source: compose/local (COPY the in-image build) ───────────────
FROM runtime-base AS runtime-from-source
COPY --from=builder /usr/local/bin/ferroehr /usr/local/bin/ferroehr

# ── runtime-prebuilt: CI (COPY the context-provided native binary; no compile) ─
# The `ferroehr` binaries are built natively per architecture on the CI runners
# (inside rust:*-slim-trixie, so they target Debian 13 glibc 2.41 — exactly what
# this distroless base ships) and land in the build context as
# bin/amd64/ferroehr and bin/arm64/ferroehr. Because no RUN executes, a multi-arch
# buildx of this target needs no QEMU.
FROM runtime-base AS runtime-prebuilt
# amd64 | arm64 — supplied by buildx per platform.
ARG TARGETARCH
COPY --chmod=0755 bin/${TARGETARCH}/ferroehr /usr/local/bin/ferroehr
