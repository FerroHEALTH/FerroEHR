#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Fail when the configuration reference drifted from the configuration types
# (#5316).
#
# tools/config-reference generates the shipped default ferroehr.toml
# (app/ferroehr/assets/ferroehr.default.toml) from the JSON Schema of
# `ferroehr::config::FerroEhrConfig`: every key with its default under its
# description. The book's reference page includes that file whole. Its
# `--check` writes nothing and fails when the committed file differs from what
# the types generate, and both modes fail on a key the schema leaves without a
# class (`x-ferroehr-class`: infrastructure, trust, or optional with a
# default) or on a class entry that names no key (#5455). No specification
# governs the reference: our own design.
#
# Usage: scripts/checks/config-reference.sh
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"

[[ "$#" -eq 0 ]] || guard_usage ""
cd "$(dirname "$0")/../.."
cargo run -q --locked -p config-reference -- --check
echo "config-reference: the reference matches the configuration types"
