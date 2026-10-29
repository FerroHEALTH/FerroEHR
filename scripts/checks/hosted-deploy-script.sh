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
  local file="$1"
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
  ' "$file"
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

# The compose file reads posture paths in place, out of the ferroehr image it
# mounts (FerroTERM's seed, the FerroPIX seed script), so each of those must be
# baked too: a COPY of the same-named file from deploy/hosted/, or a COPY whose
# destination is that path.
compose_paths_baked() {
  local compose="$1" dockerfile="$2" f base rc=0
  while IFS= read -r f; do
    base="${f##*/}"
    if ! grep -qE "(deploy/hosted/${base}([[:space:]]|\$)|/opt/sandbox-posture/${base}/?([[:space:]]|\$))" "$dockerfile"; then
      echo "::error::$compose reads ${f} out of the ferroehr image, but $dockerfile bakes nothing there" >&2
      rc=1
    fi
  done < <(grep -oE '/opt/sandbox-posture/[A-Za-z0-9._-]+' "$compose" | sort -u)
  return "$rc"
}
# The verb contract (#4372): the verbs the script's `case` admits, the list its
# `verbs` verb answers, and every verb a caller names must agree, because the
# script on the box does not update itself and its callers run from main.
# $1 the extracted script, $2.. the caller files.
verb_contract() {
  local script="$1" admitted answered called v rc=0
  shift
  admitted="$(sed -nE 's/^  ([a-z|]+)\) ;;$/\1/p' "$script" | sed -n 1p | tr '|' '\n' | sort -u | paste -sd' ' -)"
  answered="$(sed -nE 's/^  echo "verbs ([a-z ]+)"$/\1/p' "$script" | tr ' ' '\n' | sort -u | paste -sd' ' -)"
  if [[ -z "$admitted" || "$admitted" != "$answered" ]]; then
    echo "::error::the deploy script admits '${admitted}' and its verbs verb answers '${answered}'; the two lists must be the same" >&2
    return 1
  fi
  # shellcheck disable=SC2016 # the literal $HOST is the caller's text being matched
  called="$(grep -hoE '(remote-command|verb): [a-z]+$|SSH_ORIGINAL_COMMAND=[a-z]+|"deploy@\$HOST" [a-z]+' "$@" \
    | sed -E 's/.*[:= ]//' | sort -u)"
  for v in $called; do
    if [[ " $admitted " != *" $v "* ]]; then
      echo "::error::a caller of the deploy script names the verb '$v', which the script does not offer ($admitted)" >&2
      rc=1
    fi
  done
  return "$rc"
}

readonly VERB_CALLERS=(
  .github/actions/hosted-deploy/action.yml
  .github/workflows/sandbox-reseed.yml
  .github/workflows/hosted-deploy.yml
  .github/workflows/release.yml
  deploy/hosted/sandbox-ferropix-rollout.sh
)

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
  printf '    entrypoint: ["sh", "/image/opt/sandbox-posture/unbaked.sh"]\n' > "$scratch/compose.yml"
  if compose_paths_baked "$scratch/compose.yml" docker/Dockerfile >/dev/null 2>&1; then
    echo "::error::the self-test's unbaked compose posture path was not caught" >&2
    exit 1
  fi
  compose_paths_baked deploy/hosted/docker-compose.yml docker/Dockerfile >/dev/null
  {
    echo '  deploy|wipe) ;;'
    echo '  echo "verbs deploy wipe"'
  } > "$scratch/deploy.sh"
  printf '        remote-command: restart\n' > "$scratch/caller.yml"
  if verb_contract "$scratch/deploy.sh" "$scratch/caller.yml" >/dev/null 2>&1; then
    echo "::error::the self-test's caller of a verb the script lacks was not caught" >&2
    exit 1
  fi
  printf '  echo "verbs deploy"\n' > "$scratch/answer"
  { echo '  deploy|wipe) ;;'; cat "$scratch/answer"; } > "$scratch/deploy.sh"
  if verb_contract "$scratch/deploy.sh" /dev/null >/dev/null 2>&1; then
    echo "::error::the self-test's verbs answer that disagrees with the case was not caught" >&2
    exit 1
  fi
  extract "$CLOUD_INIT" > "$scratch/committed.sh"
  verb_contract "$scratch/committed.sh" "${VERB_CALLERS[@]}" >/dev/null
  echo "hosted-deploy-script: self-test OK (a seeded unquoted expansion, an unbaked compose path, a verb the script lacks and a verbs answer that disagrees are caught, the committed files pass)."
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

compose_paths_baked deploy/hosted/docker-compose.yml docker/Dockerfile || missing=1

script_tmp="$(mktemp)"
extract "$CLOUD_INIT" > "$script_tmp"
verb_contract "$script_tmp" "${VERB_CALLERS[@]}" || missing=1
rm -f "$script_tmp"

# An explicit exit: bash before 4.1 does not stop on a failing [[ ]] under set -e.
if [[ "$missing" -ne 0 ]]; then
  exit 1
fi
echo "hosted-deploy-script: the $SCRIPT_PATH embedded in $CLOUD_INIT is shellcheck-clean, every posture path it reads is baked, and every verb a caller names is one it offers — OK."
