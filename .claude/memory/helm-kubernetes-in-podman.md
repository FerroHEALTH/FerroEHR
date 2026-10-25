---
name: helm-kubernetes-in-podman
description: Helm charts and Kubernetes manifests are tested locally in Podman (owner, 2026-10-25); a missing helm binary is never a reason to skip a chart check
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

The owner ruled on 2026-10-25 that Helm and Kubernetes are tested locally in
Podman, across the FerroHEALTH family. Install `helm` and `kind` with brew
when they are missing, run `helm lint` and `helm template`, then install the
chart on a kind cluster (`KIND_EXPERIMENTAL_PROVIDER=podman kind create
cluster`), wait for readiness and green probes, uninstall, and delete the
cluster. `podman kube play` serves for plain rendered manifests.

**Why:** a release preflight once stopped only because helm was not installed
locally (FerroEHR v5.4.1), and a chart that was never installed is a chart
that was never tested.

**How to apply:** images are still never built locally; a local cluster
pulls only the published, digest-pinned images.
