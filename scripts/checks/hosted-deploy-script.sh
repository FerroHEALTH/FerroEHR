#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The hosted box's deploy script is real shell, and it is linted like real shell
# (#2974).
#
# WHY IT NEEDS A GUARD AT ALL. `deploy.sh` lives inside `cloud-init.yaml` as a
# `write_files` content block, which is where shellcheck never looks: the file
# is YAML, and no Rust test, no rustfmt run and no actionlint pass reads a
# string inside it. It is also the ONE command the CI deploy key may run, so a
# defect there is a box that stops taking deploys — the failure mode with the
# most expensive recovery on this host, because the fix has to be typed over
# SSH.
#
# So the block is extracted to a real file and linted. The extraction is also
# the check: a `write_files` entry that stops writing `deploy.sh`, or a block
# that stops being a bash script, fails here.
#
# It also checks the OTHER half of the contract: the posture files the script
# extracts from the image must actually be baked into it, so docker/Dockerfile
# must COPY every path the script reads out of /opt/sandbox-posture.
#
# What it deliberately does NOT check: that a real deploy works. Only a deploy
# proves that, and `deploy/hosted/README.md` says which properties only a
# deploy can show.
#
# Usage:
#   scripts/checks/hosted-deploy-script.sh
#   scripts/checks/hosted-deploy-script.sh --self-test
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--self-test]" "--self-test" "$@"

cd "$(dirname "$0")/../.."

readonly CLOUD_INIT=deploy/hosted/cloud-init.yaml
readonly SCRIPT_PATH=/opt/ferroehr-sandbox/deploy.sh

# The `content: |` block of the `write_files` entry whose `path:` is
# $SCRIPT_PATH, dedented by the block's own indentation. cloud-init's schema
# fixes the shape (https://cloudinit.readthedocs.io/en/latest/reference/modules.html
# — Write Files), so awk over it beats a YAML dependency this repository does
# not otherwise need in shell.
extract() {
  awk -v want="  - path: $SCRIPT_PATH" '
    $0 == want { entry = 1; next }
    entry && /^[[:space:]]*content: \|/ { body = 1; next }
    body {
      if ($0 == "") { print ""; next }
      if (indent == 0) {
        match($0, /^[[:space:]]*/)
        indent = RLENGTH
      }
      if (match($0, /^[[:space:]]*/) && RLENGTH < indent) { exit }
      print substr($0, indent + 1)
      next
    }
    entry && /^  - / { entry = 0 }
  ' "$1"
}

# One extracted script: non-empty, a bash shebang, syntactically valid, and
# clean under shellcheck at the default severity floor.
check() {
  local cloud_init="$1"
  local script
  script="$(mktemp)"
  # shellcheck disable=SC2064 # the path is expanded now, on purpose
  trap "rm -f '$script'" RETURN

  extract "$cloud_init" > "$script"
  if [[ ! -s "$script" ]]; then
    echo "::error::$cloud_init writes no non-empty $SCRIPT_PATH content block" >&2
    return 1
  fi
  if [[ "$(head -n 1 "$script")" != "#!/usr/bin/env bash" ]]; then
    echo "::error::the extracted $SCRIPT_PATH does not open with a bash shebang" >&2
    head -n 1 "$script" >&2
    return 1
  fi
  bash -n "$script" || {
    echo "::error::the extracted $SCRIPT_PATH is not valid bash" >&2
    return 1
  }
  shellcheck --shell=bash "$script" || {
    echo "::error::shellcheck refuses the $SCRIPT_PATH embedded in $cloud_init" >&2
    return 1
  }
  return 0
}

if ! command -v shellcheck >/dev/null 2>&1; then
  echo "::error::shellcheck is not installed, so this guard would pass vacuously" >&2
  echo "install it (https://github.com/koalaman/shellcheck#installing) and re-run." >&2
  exit 1
fi

if [[ "${1:-}" == "--self-test" ]]; then
  # A seeded shell defect must be caught, and the committed file must pass.
  scratch="$(mktemp -d)"
  # shellcheck disable=SC2064 # the path is expanded now, on purpose
  trap "rm -rf '$scratch'" EXIT
  seeded="$scratch/cloud-init.yaml"
  {
    echo '#cloud-config'
    echo 'write_files:'
    echo "  - path: $SCRIPT_PATH"
    echo '    content: |'
    echo '      #!/usr/bin/env bash'
    echo '      set -euo pipefail'
    # shellcheck disable=SC2016  # the literal unquoted $1 IS the seeded defect
    echo '      cat $1'
  } > "$seeded"
  if check "$seeded" >/dev/null 2>&1; then
    echo "::error::the self-test's unquoted \$1 was not caught" >&2
    exit 1
  fi
  check "$CLOUD_INIT" >/dev/null
  echo "hosted-deploy-script: self-test OK (a seeded unquoted expansion is caught, the committed script passes)."
  exit 0
fi

[[ -f "$CLOUD_INIT" ]] || { echo "::error::missing $CLOUD_INIT" >&2; exit 1; }
check "$CLOUD_INIT"

# Every /opt/sandbox-posture file the embedded script copies out of the image
# must be a path docker/Dockerfile actually bakes.
script_tmp="$(mktemp)"
extract "$CLOUD_INIT" > "$script_tmp"
missing=0
while IFS= read -r f; do
  base="${f##*/}"
  if ! grep -q "deploy/hosted/${base}" docker/Dockerfile; then
    echo "::error::the deploy script extracts ${f} but docker/Dockerfile does not COPY deploy/hosted/${base} into /opt/sandbox-posture" >&2
    missing=1
  fi
done < <(grep -oE '/opt/sandbox-posture/[A-Za-z0-9._-]+' "$script_tmp" | sort -u)
rm -f "$script_tmp"
[[ "$missing" -eq 0 ]]
echo "hosted-deploy-script: the $SCRIPT_PATH embedded in $CLOUD_INIT is shellcheck-clean and every posture path it reads is baked — OK."
