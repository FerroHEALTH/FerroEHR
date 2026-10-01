#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# Docs-claim guard: a documented KEY must exist in the thing it documents.
#
# The v3.17.4 book sweep found five substantive defects that had shipped to the
# published site — a fabricated Helm values table, a page asserting endpoints did
# not exist when six did, a `SPEC_VERSION` constant taught for crates that have
# none. None was caught by a gate: `mdbook-lint` checks style, `lychee` checks
# link reachability, and both pass happily on a page whose every technical claim
# is false. The same-PR docs rule depended entirely on an author noticing.
#
# This guard checks the claims that have a machine-readable authority to check
# against, and nothing else. Three of them:
#
#   1. Every `FERROEHR__…` environment form a page shows resolves to a real key
#      in `app/ferroehr/assets/ferroehr.default.toml`.
#   2. Every Helm values path a chart page shows resolves — against
#      `values.yaml`, or for the `config.*` subtree against the TOML schema,
#      because the chart renders `config:` VERBATIM to ferroehr.toml (so
#      `config.<any-toml-path>` is legitimate whether or not values.yaml
#      enumerates it, and the one non-TOML child, `config.files`, resolves in
#      values.yaml instead). The securityContext family and
#      `autoscaling.behavior` are passthroughs of the same kind — the chart
#      renders the whole Kubernetes type, so a documented child resolves on its
#      root and the API reference is the authority for the leaf.
#   3. Every committed generated chart under a `*-assets/` directory is embedded
#      by some page. An unreferenced chart is either a page that lost its figure
#      or a renderer emitting output nobody reads.
#
# What is NOT checked, and why — an unenforced rule is labelled as such
# (`.claude/rules/reliability.md`):
#
#   * **Documented Rust paths.** A grep-level existence check for
#     `crate::a::b` / `openehr_x::y::Z` was measured and rejected: prose names
#     types that exist under a different module path than the sentence implies,
#     and the generated crates' generation modules mean one type has several
#     valid paths. It produced more false positives than findings. Review-only.
#   * **Prose claims of any kind** — "six endpoints exist", "the chart renders
#     one key" — have no machine authority. Review-only, and the reason the
#     same-PR docs rule still matters.
#
# Scope note on check 2: the values vocabulary is only in scope on pages that
# document the chart. `configuration.md` legitimately writes
# `` `service.name` `` for the OpenTelemetry resource attribute, which collides
# with the chart's top-level `service:` key and is not a values path at all.
# Widening the check to every page turns that into a false failure, so the check
# reads the chart pages and the chart pages only.
#
# Usage: scripts/checks/docs-claims.sh [--all | --diff [base] | <file>...]
#   no args  → every tracked book page (same as --all; the CI job's scope).
#              The old default scoped to the diff against origin/main, which
#              examined ZERO pages on uncommitted work and printed OK — a
#              local run before committing was vacuous (#3010).
#   --all    → every tracked book page
#   --diff   → only the book files changed against [base] (default
#              origin/main); the explicit opt-in for a scoped run
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--all | --diff [base] | <file>...]" "--all --diff" "$@"
cd "$(dirname "$0")/../.."

BOOK=website/book/src
TOML=app/ferroehr/assets/ferroehr.default.toml
VALUES=deploy/helm/ferroehr/values.yaml
# The pages that document the Helm chart, and therefore the only pages on which a
# dotted token is read as a chart values path. Glob-derived: a hardcoded list
# went silently stale when the hardening chapter split into sub-pages (#2348),
# leaving five chart pages unchecked while the gate reported OK.
CHART_PAGES=$(printf '%s ' "$BOOK"/installation/kubernetes*.md "$BOOK"/installation/hardening-*.md \
  "$BOOK"/operations.md "$BOOK"/operations-admin-apis.md)

for required in "$TOML" "$VALUES"; do
  [[ -f "$required" ]] || { echo "docs-claims: missing authority file $required" >&2; exit 1; }
done

collect() {
  if [[ "$#" -eq 0 || "${1:-}" = "--all" ]]; then
    git ls-files "$BOOK/**/*.md" "$BOOK/*.md"
  elif [[ "${1:-}" = "--diff" ]]; then
    # The explicit scoped mode. Uncommitted work is invisible to a
    # commit-range diff, so the worktree diff is included too.
    local base="${2:-origin/main}"
    {
      git diff --name-only "$base"...HEAD -- "$BOOK/*.md" "$BOOK/**/*.md" 2>/dev/null || true
      git diff --name-only -- "$BOOK/*.md" "$BOOK/**/*.md" 2>/dev/null || true
    } | sort -u
  else
    printf '%s\n' "$@"
  fi
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# ── the authorities ──────────────────────────────────────────────────────────

# Every TOML path, plus every ancestor table (a page may document `[audit]` as
# `config.audit`). A `#?` line documents a real optional/secret/derived key and
# counts; a plain `#` comment does not.
toml_paths() {
  awk '
    { line = $0
      sub(/^[[:space:]]*/, "", line)
      if (line ~ /^#\?[[:space:]]*/) sub(/^#\?[[:space:]]*/, "", line)
      else if (line ~ /^#/) next
      if (line ~ /^\[\[?[A-Za-z0-9_.]+\]\]?/) { sec = line; gsub(/^\[\[?|\]\]?.*$/, "", sec); next }
      if (line ~ /^[A-Za-z0-9_]+[[:space:]]*=/) {
        k = line; sub(/[[:space:]]*=.*$/, "", k)
        path = (sec == "" ? k : sec "." k)
        print path
        n = split(path, seg, ".")
        acc = seg[1]
        for (i = 2; i < n; i++) { print acc; acc = acc "." seg[i] }
        if (n > 1) print acc
      }
    }' "$1" | sort -u
}

# values.yaml flattened to dotted paths by indentation. Our own file, two-space
# indented; list items and comments are skipped.
values_paths() {
  awk '
    /^[[:space:]]*#/ { next }
    /^[[:space:]]*$/ { next }
    /^[[:space:]]*-/ { next }
    { line = $0
      sub(/[[:space:]]+#.*$/, "", line)
      match(line, /^[[:space:]]*/); ind = RLENGTH
      key = line; sub(/^[[:space:]]*/, "", key)
      if (key !~ /^[A-Za-z_][A-Za-z0-9_]*:/) next
      sub(/:.*$/, "", key)
      lvl = int(ind / 2)
      stack[lvl] = key
      path = stack[0]
      for (i = 1; i <= lvl; i++) path = path "." stack[i]
      print path
    }' "$1" | sort -u
}

toml_paths "$TOML" > "$work/toml"
values_paths "$VALUES" > "$work/values"
# Env forms: the TOML path upper-cased with `__` between segments.
sed 's/\./__/g' "$work/toml" | tr '[:lower:]' '[:upper:]' | sed 's/^/FERROEHR__/' | sort -u > "$work/env"
grep -vE '\.' "$work/values" > "$work/values_top"

failures=0
report() {
  printf '%s: %s\n' "$1" "$2" >&2
  failures=$((failures + 1))
}

files=$(collect "$@")

# Membership of the file list, for the sections that check ONE named page.
#
# It needs a helper because the obvious spelling does not work: `$files` is
# NEWLINE-separated, so `case " $files " in *" $page "*)` asks for a match
# delimited by spaces and can never find one. Three sections were written that
# way and were silently inert — the quoted-wire-evidence check on comparison.md
# and the Rust-version check on from-source.md never ran once, proven by
# mutating both pages and watching the guard report OK (#2779). Space-normalise
# the list once and test against that.
files_spaced=" ${files//$'\n'/ } "
in_files() {
  case "$files_spaced" in *" $1 "*) return 0 ;; *) return 1 ;; esac
}

# ── 1. configuration keys ────────────────────────────────────────────────────
for f in $files; do
  [[ -f "$f" ]] || continue
  # A form ending in `__` is prose naming a prefix ("FERROEHR__AUDIT__…"), not a
  # key. Anything else must resolve.
  while read -r form; do
    [[ -n "$form" ]] || continue
    case "$form" in *__) continue ;; *) ;; esac
    grep -qxF "$form" "$work/env" \
      || report "$f" "documents \`$form\`, which is not a key in $TOML"
  done < <(grep -ohE 'FERROEHR__[A-Z0-9_]+' "$f" | sort -u)
done

# ── 1b. documented defaults vs the shipped default TOML ─────────────────────
# A `| Key | Type | Default |` row on a configuration page states a default the
# TOML asset also states; the two went out of sync on the published site
# (`[query] timeout_ms` documented as `0` against a shipped `30000` — #2348).
# Only rows whose Default cell is a single backticked literal AND whose key has
# an uncommented assignment in the TOML are compared — optional/`#?` keys and
# prose cells ("unset", "—") carry no machine-checkable claim and are skipped,
# so every report from this section is a real disagreement.
awk '
  /^\[/ { line=$0; gsub(/[][]/, "", line); sect=line; next }
  /^[a-z0-9_]+[ \t]*=/ {
    key=$0; sub(/[ \t]*=.*/, "", key)
    val=$0; sub(/^[^=]*=[ \t]*/, "", val)
    if (val ~ /^"/) { sub(/^"/, "", val); sub(/".*$/, "", val) }
    else { sub(/[ \t]*#.*$/, "", val); sub(/[ \t]+$/, "", val) }
    print (sect == "" ? key : sect "." key) "\t" val
  }
' "$TOML" > "$work/toml_defaults"
for f in $files; do
  case "$f" in
    "$BOOK"/installation/configuration.md|"$BOOK"/installation/config-*.md) ;;
    *) continue ;;
  esac
  [[ -f "$f" ]] || continue
  awk '
    /^#/ { intab=0
           if (match($0, /\[[a-z0-9_.]+\]/)) sect = substr($0, RSTART+1, RLENGTH-2)
           next }
    /^\| Key \| Type \| Default/ { intab=1; next }
    intab && /^\|[-| ]+$/ { next }
    intab && /^\|/ {
      split($0, c, /\|/)
      key=c[2]; def=c[4]
      gsub(/[` ]/, "", key)
      sub(/^[ ]+/, "", def); sub(/[ ]+$/, "", def)
      if (key != "" && def ~ /^`[^`]*`$/) { gsub(/`/, "", def); print sect "." key "\t" def }
      next }
    !/^\|/ { intab=0 }
  ' "$f" > "$work/page_defaults"
  while IFS=$(printf '\t') read -r path documented; do
    [[ -n "$path" ]] || continue
    actual=$(awk -F'\t' -v p="$path" '$1==p{print $2; exit}' "$work/toml_defaults")
    [[ -n "$actual" ]] || continue
    doc_n=$documented; act_n=$actual
    # A page may spell a string default with its TOML quotes (`"1.3"`); the
    # TOML side is stored unquoted, so strip a quote wrapper before comparing.
    doc_n=${doc_n#\"}; doc_n=${doc_n%\"}
    case "$doc_n" in \[*) doc_n=$(printf '%s' "$doc_n" | tr -d ' '); act_n=$(printf '%s' "$act_n" | tr -d ' ') ;; *) ;; esac
    [[ "$doc_n" = "$act_n" ]] \
      || report "$f" "documents \`$path\` default as \`$documented\`; $TOML says \`$actual\`"
  done < "$work/page_defaults"
done

# ── 2. Helm values paths (chart pages only) ──────────────────────────────────
for f in $files; do
  case " $CHART_PAGES " in *" $f "*) ;; *) continue ;; esac
  [[ -f "$f" ]] || continue
  # shellcheck disable=SC2016 # the ERE below matches literal markdown backticks
  while read -r path; do
    [[ -n "$path" ]] || continue
    first=${path%%.*}
    # Only a token rooted at a real top-level values key is read as a values
    # path; anything else is ordinary prose that happens to contain a dot.
    grep -qxF "$first" "$work/values_top" || continue
    if grep -qxF "$path" "$work/values"; then continue; fi
    # `config.*` is rendered verbatim into ferroehr.toml, so the TOML schema is
    # its authority.
    case "$path" in
      config.*) grep -qxF "${path#config.}" "$work/toml" && continue ;;
      *) ;;
    esac
    # The other verbatim-passthrough subtrees. `config.*` is not the only one:
    # values.schema.json declares these "rendered verbatim", so the whole
    # Kubernetes type is legitimately settable under them and values.yaml
    # enumerates only the members the chart has an opinion about. A documented
    # child here resolves because its ROOT does — the Kubernetes API is the
    # authority for the leaf, and no file in this repo can stand in for it.
    case "$path" in
      securityContext.*|podSecurityContext.*|autoscaling.behavior.*|\
      viewer.securityContext.*|viewer.podSecurityContext.*) continue ;;
      *) ;;
    esac
    report "$f" "documents Helm value \`$path\`, which the chart does not define"
  done < <(grep -ohE '`[a-z][A-Za-z0-9]*(\.[A-Za-z0-9_]+)+(=[^`]*)?`' "$f" | tr -d '`' | sed -E 's/=.*$//' | sort -u)
  # `--set` is unambiguous wherever it appears, so it is checked without the
  # top-level-key filter above.
  while read -r path; do
    [[ -n "$path" ]] || continue
    grep -qxF "$path" "$work/values" && continue
    case "$path" in
      config.*) grep -qxF "${path#config.}" "$work/toml" && continue ;;
      *) ;;
    esac
    # The other verbatim-passthrough subtrees. `config.*` is not the only one:
    # values.schema.json declares these "rendered verbatim", so the whole
    # Kubernetes type is legitimately settable under them and values.yaml
    # enumerates only the members the chart has an opinion about. A documented
    # child here resolves because its ROOT does — the Kubernetes API is the
    # authority for the leaf, and no file in this repo can stand in for it.
    case "$path" in
      securityContext.*|podSecurityContext.*|autoscaling.behavior.*|\
      viewer.securityContext.*|viewer.podSecurityContext.*) continue ;;
      *) ;;
    esac
    report "$f" "\`--set $path=\` names a value the chart does not define"
  done < <(grep -ohE '\-\-set +[a-z][A-Za-z0-9_.]*=' "$f" | sed -E 's/--set +//; s/=$//' | sort -u)
done

# ── 3. chart/app version literals (chart pages only) ────────────────────────
# Both went stale on the published site (`--version 5.0.1` against a 6.x chart,
# `image.tag=3.17.3` against a 3.17.5 appVersion — #2348), so a literal that
# disagrees with its authority fails here.
#
# TWO authorities, not one (#2779). The CHART version is Chart.yaml's own
# `version` — the chart's independent SemVer line, still committed and still
# hand-bumped. The APPLICATION version is the WORKSPACE version in Cargo.toml,
# no longer Chart.yaml's `appVersion`: the publish lane now injects the released
# version into the packaged chart (`helm package --app-version`), so the
# committed appVersion is a between-releases default that may legitimately lag,
# while a book page teaches the tag an operator can actually pull from the
# newest release — which is the workspace version.
CHART_YAML=deploy/helm/ferroehr/Chart.yaml
chart_version=$(awk '$1=="version:"{print $2; exit}' "$CHART_YAML")
app_version=$(sed -nE 's/^version = "(.*)"$/\1/p' Cargo.toml | head -1)
[[ -n "$chart_version" && -n "$app_version" ]] \
  || { echo "docs-claims: could not read the chart version ($CHART_YAML) or the workspace version (Cargo.toml)" >&2; exit 1; }
for f in $files; do
  case " $CHART_PAGES " in *" $f "*) ;; *) continue ;; esac
  [[ -f "$f" ]] || continue
  while read -r v; do
    [[ -n "$v" ]] || continue
    [[ "$v" = "$chart_version" ]] \
      || report "$f" "pins chart \`--version $v\`; Chart.yaml says $chart_version"
  done < <(grep -ohE '\-\-version +[0-9]+\.[0-9]+\.[0-9]+' "$f" | awk '{print $2}' | sort -u)
  # `image.tag=<ver>` is in the pattern deliberately (#2779). The motivating
  # defect this section records is "`image.tag=3.17.3` against a 3.17.5
  # appVersion", and the pattern did not match that spelling at all — only a
  # fully-spelled `…/ferroehr:<ver>` reference. Mutating the kubernetes page's
  # `--set image.tag=` pin was reported as OK.
  while read -r v; do
    [[ -n "$v" ]] || continue
    [[ "$v" = "$app_version" ]] \
      || report "$f" "pins image tag \`$v\`; the workspace version is $app_version"
  done < <(grep -ohE '(ferroehr(-viewer)?:|image\.tag=)[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?' "$f" \
    | sed -E 's/^.*[:=]//' | sort -u)
done
# The root README is not a book page, so the loop above never reads it, and its
# `helm install … --version` pin sat two chart versions stale until #3305
# corrected it by hand (#3327). The same two literal checks, over that one file.
f=README.md
if [[ -f "$f" ]]; then
  while read -r v; do
    [[ -n "$v" ]] || continue
    [[ "$v" = "$chart_version" ]] \
      || report "$f" "pins chart \`--version $v\`; Chart.yaml says $chart_version"
  done < <(grep -ohE '\-\-version +[0-9]+\.[0-9]+\.[0-9]+' "$f" | awk '{print $2}' | sort -u)
  while read -r v; do
    [[ -n "$v" ]] || continue
    [[ "$v" = "$app_version" ]] \
      || report "$f" "pins image tag \`$v\`; the workspace version is $app_version"
  done < <(grep -ohE 'image\.tag=[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?' "$f" | sed -E 's/^.*=//' | sort -u)
fi
# Every page (and the landing): a fully-spelled ghcr image reference must carry
# the current appVersion, and never a `v` prefix — the publish lane tags
# `{{version}}` without one, so `ghcr.io/…:v3.17.5` does not resolve at all.
for f in $files website/landing/index.html; do
  [[ -f "$f" ]] || continue
  if grep -qE 'ghcr\.io/(rubentalstra|ferrohealth)/[A-Za-z0-9._-]+:v[0-9]' "$f"; then
    report "$f" "references a v-prefixed ghcr image tag; published tags carry no v prefix"
  fi
  case " $CHART_PAGES " in *" $f "*) continue ;; *) ;; esac
  while read -r v; do
    [[ -n "$v" ]] || continue
    [[ "$v" = "$app_version" ]] \
      || report "$f" "pins ghcr image tag \`$v\`; the workspace version is $app_version"
  done < <(grep -ohE 'ghcr\.io/(rubentalstra|ferrohealth)/(ferroehr|ferroehr-viewer|ferroehr-postgres):[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?' "$f" | sed 's/.*://' | sort -u)
done
# The from-source page: a Rust version literal must be the toolchain channel
# (with or without its patch) or the EXACT declared MSRV ("Rust 1.96.1"
# shipped against a 1.97.1 toolchain — #2348; the MSRV arm then accepted the
# same phantom patch because "1.96.*" matched it — #2805).
RUST_PAGE="$BOOK/installation/from-source.md"
if in_files "$RUST_PAGE"; then
  channel=$(awk -F'"' '/^channel/{print $2; exit}' rust-toolchain.toml)
  msrv=$(awk -F'"' '/^rust-version/{print $2; exit}' Cargo.toml)
  while read -r v; do
    case "$v" in "$channel"|"${channel%.*}"|"$msrv") continue ;; *) ;; esac
    report "$RUST_PAGE" "names Rust \`$v\`, which is neither the toolchain channel ($channel) nor the MSRV ($msrv)"
  done < <(grep -ohE '1\.[0-9]{2}(\.[0-9]+)?' "$RUST_PAGE" | sort -u)
fi

# ── 3b. quoted wire evidence on the comparison page ─────────────────────────
# Five fabricated response-body quotes shipped on the published comparison page
# (#2348) — none occurred anywhere in the committed run records. A backticked
# double-quoted string on that page presents itself as captured wire evidence,
# so it must occur verbatim in one of the committed results records.
COMPARISON_PAGE="$BOOK/comparison.md"
if in_files "$COMPARISON_PAGE"; then
  # shellcheck disable=SC2016 # the ERE below matches literal markdown backticks
  while read -r quote; do
    [[ -n "$quote" ]] || continue
    inner=${quote#\`\"}; inner=${inner%\"\`}
    grep -qF "$inner" docs/conformance/ehrbase/results.json docs/conformance/ferroehr/results.json 2>/dev/null \
      || report "$COMPARISON_PAGE" "quotes \`\"$inner\"\` as wire evidence, but no committed results record contains it"
  done < <(grep -ohE '`"[^"`]{4,}"`' "$COMPARISON_PAGE" | sort -u)
fi

# ── 3c. banned phrases with a recorded refutation ────────────────────────────
# "static binary" shipped on four pages, the landing and the README while the
# runtime image is distroless/cc precisely because the binary links glibc
# dynamically (#2348). The claim is false by construction for this build.
# A NEGATED use ("is not a static binary: …") is the truthful correction and
# stays legal; the match runs over line-joined text so a negation split across
# a line wrap is still recognised.
while read -r f; do
  [[ -f "$f" ]] || continue
  tr '\n' ' ' < "$f" \
    | sed -E 's/(not|never) a static(ally linked)? binary//Ig' \
    | grep -qiE 'static(ally linked)? binary' \
    && report "$f" "claims a static binary — the binary is dynamically linked (distroless/cc); say 'self-contained'"
done < <(grep -rilE 'static(ally linked)? binary' --include='*.md' "$BOOK" website/landing README.md 2>/dev/null; \
         grep -lFi 'static binary' website/landing/index.html 2>/dev/null)

# ── 3d. the release-verification page's copy-paste literals ─────────────────
# `verifying-releases.md` is a procedure an operator pastes, so a version
# literal in it is a claim about the CURRENT release rather than prose. The
# v4.0.5 cut updated the substitution note and left both
# `gh attestation verify …/ferroehr:4.0.4` examples behind; the docs freeze then
# published them, and the fix could only ever land in a later version's site
# (#2779). The bare image-tag half was already covered by the every-page ghcr
# rule in section 3 — and it DID report on the release PR, which was merged
# anyway — so what is added here is the half with no authority at all: the
# release-ASSET filename shape, plus two independent nets under the
# substitute-this-tag note.
#
# Historical prose on this page is deliberately NOT checked. "From v4.0.1 on,
# the shipped binary embeds…" and "Images and the chart reach L3 from the first
# publish after v4.0.1" are correct BECAUSE they name an older release, so a
# blanket rule over every `vX.Y.Z` literal would report three true sentences.
# The currency claim lives in the asset names, the substitution note and the
# commands — and nowhere else on the page.
VERIFY_PAGE="$BOOK/verifying-releases.md"
if in_files "$VERIFY_PAGE"; then
  # A release asset is named after the git TAG, so its version carries the `v`.
  while read -r v; do
    [[ -n "$v" ]] || continue
    [[ "$v" = "$app_version" ]] \
      || report "$VERIFY_PAGE" "names the release asset \`ferroehr-v${v}…\`; the workspace version is $app_version"
  done < <(grep -ohE 'ferroehr-v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?' "$VERIFY_PAGE" | sed 's/^ferroehr-v//' | sort -u)
  # The substitution note. Its sentence wraps, so the page is line-joined first.
  # shellcheck disable=SC2016 # the ERE and sed below match literal markdown backticks
  while read -r v; do
    [[ -n "$v" ]] || continue
    [[ "$v" = "$app_version" ]] \
      || report "$VERIFY_PAGE" "tells the reader to substitute the tag \`v$v\`; the workspace version is $app_version"
  done < <(tr '\n' ' ' < "$VERIFY_PAGE" \
    | grep -ohE 'for example[[:space:]]+`v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?`' \
    | sed -E 's/.*`v([0-9A-Za-z.-]+)`.*/\1/' | sort -u)
  # The wording-independent net, because the rule above is anchored on a phrase
  # and a reword would silently disarm it: the released tag has to appear on the
  # page SOMEWHERE. A cut that bumps Cargo.toml and forgets this page fails here
  # whatever the sentence around the literal says.
  grep -qF "v${app_version}" "$VERIFY_PAGE" \
    || report "$VERIFY_PAGE" "never names the released tag \`v$app_version\` — its substitution examples still teach an older release"
fi

# ── 4. generated charts nothing embeds ───────────────────────────────────────
# Whole-corpus by nature: a page deleting its figure is exactly the case to
# catch, and that page may not be in a narrow diff.
#
# The reference may sit in an mdBook `{{#include}}` source rather than in a page
# — `comparison.md` pulls its figures in from `website/book/generated/` — so that
# tree is searched too. Searching only the pages reported a chart as orphaned when
# it was in fact published, which is the failure mode that gets a guard ignored.
#
# And `website/book/generated/` is a BUILD ARTIFACT: it is not committed and does
# not exist in a fresh checkout, so on its own it never matches and a figure
# published only through an include is reported as orphaned regardless — measured
# on `perf-stress-compare.svg`, which `scripts/render/comparison.sh` both renders
# AND embeds, while this guard called it unpublished. The durable reference for
# generated markdown is therefore the GENERATOR, so the render scripts are the
# third haystack.
while read -r svg; do
  [[ -n "$svg" ]] || continue
  base=${svg##*/}
  grep -rqF "$base" --include='*.md' "$BOOK" website/book/generated \
    || grep -rqF "$base" scripts/render \
    || report "$svg" "is committed but neither a page, a generated include, nor a render script embeds it"
done < <(git ls-files "$BOOK/*-assets/*.svg")

if [[ "$failures" -gt 0 ]]; then
  echo "docs-claims: $failures unresolvable claim(s) — see above." >&2
  echo "  A documented key must exist in its source. Fix the page, or fix the" >&2
  echo "  source if the page is describing what the software should do." >&2
  exit 1
fi
# The examined-page count is part of the verdict: "OK over 0 pages" is how
# the old diff-scoped default passed vacuously on uncommitted work (#3010).
page_count=$(printf '%s\n' "$files" | grep -c . || true)
echo "docs-claims: OK ($page_count pages examined)."
