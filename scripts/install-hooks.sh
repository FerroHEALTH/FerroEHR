#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/install-hooks.sh
#
# Installs the repo's tracked git hooks by pointing git at .githooks/.
# Run once after cloning the fork:  bash scripts/install-hooks.sh
#
# We use core.hooksPath (not .git/hooks) so the hooks are version-controlled
# and shared across everyone who clones the repo.

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

chmod +x .githooks/* 2>/dev/null || true
git config core.hooksPath .githooks

echo "core.hooksPath set to .githooks"
echo "Installed hooks:"
ls -1 .githooks
