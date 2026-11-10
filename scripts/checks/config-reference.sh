#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Fail when the configuration reference drifted from the configuration types
# (#5316).
#
# tools/config-reference generates two files from the JSON Schema of
# `ferroehr::config::FerroEhrConfig`: the shipped default ferroehr.toml
# (app/ferroehr/assets/ferroehr.default.toml), every key with its default under
# its description, and the book's reference page
# (website/book/src/installation/config-keys.md). Its `--check` writes nothing
# and names each committed file that differs from what the types generate. No
# specification governs the reference: our own design.
#
# Usage: scripts/checks/config-reference.sh
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"

[[ "$#" -eq 0 ]] || guard_usage ""
cd "$(dirname "$0")/../.."
cargo run -q --locked -p config-reference -- --check
echo "config-reference: the reference matches the configuration types"
