#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Codespace create-time image pull (#2709): fetch the published quickstart
# images once, so the first `postStartCommand` boot is fast. Explicit `-f`
# keeps this on the standalone file even if COMPOSE_FILE is ever unset.
# (The dev overlay stopped auto-merging in #2868; the pin stays as
# belt-and-braces so the tester sandbox can never build from source.)
set -Eeuo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

docker compose -f docker-compose.yml --profile viewer pull
