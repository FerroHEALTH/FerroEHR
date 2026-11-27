#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Installs packages on a GitHub-hosted runner from the Ubuntu archive alone
# (#6722). A plain `apt-get update` fails when any third-party source of the
# runner image is broken: packages.microsoft.com answered 403 on the ubuntu
# 26.04 image and failed jobs that never use it. The refresh reads the Ubuntu
# sources only and keeps the other lists as they are; a package that cannot be
# installed still fails the install, loudly.
#
# Usage: scripts/lib/apt-install.sh <package>...
set -euo pipefail

if [[ $# -eq 0 ]]; then
  echo "apt-install: name at least one package" >&2
  exit 2
fi

sudo=()
if [[ "$(id -u)" -ne 0 ]]; then
  sudo=(sudo)
fi

# The Ubuntu sources: the deb822 file of Ubuntu 24.04 and later, or the
# one-line list of older images.
sources=""
for candidate in /etc/apt/sources.list.d/ubuntu.sources /etc/apt/sources.list; do
  if [[ -s "$candidate" ]] && grep -qE '^(deb |Types:)' "$candidate"; then
    sources="$candidate"
    break
  fi
done
if [[ -z "$sources" ]]; then
  echo "::error::apt-install: this runner carries no Ubuntu apt source list" >&2
  exit 1
fi

${sudo[@]+"${sudo[@]}"} apt-get update \
  -o Dir::Etc::sourcelist="$sources" \
  -o Dir::Etc::sourceparts=- \
  -o APT::Get::List-Cleanup=0
${sudo[@]+"${sudo[@]}"} apt-get install -y --no-install-recommends "$@"
