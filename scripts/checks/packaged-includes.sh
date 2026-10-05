#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# A packaged source file may only embed a file the PACKAGE carries.
#
# `include_str!`/`include_bytes!` resolve against the source file's directory,
# so a path reaching out of the crate (`../../corpus/…`,
# `../../../../../openehr-its/tests/vendor/…`) compiles perfectly in this
# workspace and fails with "couldn't read file" for anyone who unpacks the
# published `.crate` and runs `cargo test` — the source is there, the fixture
# never was. `#[cfg(test)]` does not save it: the file still ships, and a
# consumer's test run still expands the macro.
#
# The packaged set comes from `cargo package --list`, which is the same
# computation the publish lane performs, rather than from reading the `include`
# list by eye. Every literal is resolved against the including file and must
# land inside that list; a non-literal include is reported rather than assumed
# safe, because this gate cannot evaluate it.
#
# Usage: scripts/checks/packaged-includes.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"

cd "$(dirname "$0")/../.."
ROOT="$(pwd -P)"

fail=0
note() { echo "packaged-includes: $*" >&2; fail=1; }

# The resolved absolute path of an include literal, or empty when the directory
# it names does not exist (a broken include, reported by the caller).
resolve() {
  local dir="$1" literal="$2" target_dir
  target_dir="$(cd "$dir" && cd "$(dirname "$literal")" 2>/dev/null && pwd -P)" || return 0
  printf '%s/%s' "$target_dir" "$(basename "$literal")"
}

crates=0
embeds=0
for manifest in crates/*/Cargo.toml; do
  crate_dir="$(dirname "$manifest")"
  crate_name="$(sed -nE 's/^name *= *"([^"]+)".*/\1/p' "$manifest" | head -1)"
  [[ -n "$crate_name" ]] || { note "$manifest declares no package name"; continue; }
  crates=$((crates + 1))

  packaged="$(cargo package --list -p "$crate_name" --allow-dirty 2>/dev/null)" || {
    note "cargo package --list failed for $crate_name — the packaged set is unknown, so nothing here is verified"
    continue
  }

  while IFS= read -r packaged_file; do
    case "$packaged_file" in *.rs) ;; *) continue ;; esac
    source_file="$crate_dir/$packaged_file"
    [[ -f "$source_file" ]] || continue

    # Newlines become spaces so a macro call split across lines is still one
    # match; a path literal never contains a newline.
    while IFS= read -r match; do
      [[ -n "$match" ]] || continue
      literal="$(printf '%s' "$match" | sed -E 's/.*"([^"]*)".*/\1/')"
      embeds=$((embeds + 1))
      target="$(resolve "$(dirname "$source_file")" "$literal")"
      if [[ -z "$target" ]]; then
        note "$source_file embeds \"$literal\", which resolves to no existing directory"
        continue
      fi
      case "$target" in
        "$ROOT/$crate_dir/"*)
          relative="${target#"$ROOT/$crate_dir/"}"
          grep -qxF -- "$relative" <<<"$packaged" || {
            note "$source_file embeds \"$literal\" ($relative), which is INSIDE $crate_dir but not in its package"
            note "  a consumer running \`cargo test\` in the unpacked .crate gets \"couldn't read file\";"
            note "  move the code that embeds it into the crate's tests/ tree (never packaged), or"
            note "  add the file to the crate's \`include\` list if it is a runtime asset"
          }
          ;;
        *)
          note "$source_file embeds \"$literal\", which is OUTSIDE $crate_dir and therefore outside the published package"
          note "  a consumer running \`cargo test\` in the unpacked .crate gets \"couldn't read file\";"
          note "  move the test to the crate's tests/ tree (unpackaged) or embed a fixture the package carries"
          ;;
      esac
    done < <(tr '\n' ' ' <"$source_file" |
      grep -oE 'include_(str|bytes)![[:space:]]*\([[:space:]]*"[^"]+"' || true)

    # An include whose argument is not a plain literal (a `concat!`, a macro, a
    # constant) cannot be resolved here, and silence would read as coverage.
    if tr '\n' ' ' <"$source_file" |
      grep -qE 'include_(str|bytes)![[:space:]]*\([[:space:]]*[^"[:space:])]'; then
      note "$source_file embeds a computed path this gate cannot resolve — make it a literal, or exempt it here with the reason"
    fi
  done <<<"$packaged"
done

[[ "$fail" -eq 0 ]] || {
  echo >&2
  echo "A published crate is a self-contained artifact: everything its packaged" >&2
  echo "sources embed must travel with it." >&2
  exit 1
}

echo "ok: ${embeds} embedded file(s) across ${crates} packaged crates, each carried by its own package"
