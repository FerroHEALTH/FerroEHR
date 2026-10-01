#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# scripts/checks/chart-appversion.sh — the committed chart release facts track
# the workspace version, the compose treatment (#2890).
#
# The committed `appVersion` equals the workspace version in Cargo.toml, bumped
# in the release PR beside the docker-compose.yml image tags — one sweep, one
# philosophy for both committed artifacts. The publish lane still injects
# `--app-version ${TAG#v}` and rewrites the `artifacthub.io/images` tags at
# package time (#2779); since the tree now carries the same value, the
# injection is belt-and-braces for the release leg and remains what makes the
# dispatch-driven `publish-chart.yml` recovery lane correct between releases.
#
# The one accepted cost (adjudicated on #2890, the same window the compose
# defaults have always accepted): between a release PR's merge and the tag's
# Containers leg publishing the X.Y.Z images, a from-tree `helm install`
# references image tags that do not exist yet. The window is ~30-60 minutes,
# exists only during a cut, and buys a tree whose committed versions never lag.
#
# Five properties, each with real content:
#
#   1. `appVersion` equals the workspace version (Cargo.toml is the authority,
#      the same source compose-image-tags.sh reads). A lagging default ships a
#      chart whose images predate the tree's own configuration vocabulary; a
#      diverging one means the release sweep missed a site.
#   2. Every FIRST-PARTY `artifacthub.io/images` tag equals `appVersion`. The
#      two describe one thing — the product version this chart defaults to —
#      and the package-time injection rewrites both from one input, so a tree
#      where they disagree would publish an annotation the package's own
#      appVersion contradicts. First-party means the images built from THIS
#      repository at this version: `ferroehr`, `ferroehr-viewer` and
#      `ferroehr-postgres` today. A third-party image the chart deploys —
#      `ghcr.io/rubentalstra/ferroterm` since #3305 — moves on its own release
#      line, is pinned in the chart's own values and helpers, and is neither
#      equal to appVersion nor rewritten at package time; holding it to this
#      rule would make every FerroEHR release claim a FerroTERM version that
#      does not exist.
#
#      WHICH entry is which is DECLARED, never inferred from the repository
#      name: each `image:` line in the annotation carries a trailing
#      `# party: first` or `# party: third` marker, and an unmarked line fails
#      this check. deploy/helm/release-facts.sh reads the same marker, so the
#      set rewritten at package time and the set checked here are one set by
#      construction. A name pattern would misclassify in both directions — a
#      future `ghcr.io/ferrohealth/ferroehr-<something>` built by somebody
#      else would be rewritten to our version, and a first-party image
#      published under another name would be left a release behind — and
#      neither mistake shows up in any render.
#   3. The third-party FerroTERM entry equals its own two sources: the TAG
#      equals `ferroehr.terminologyPinnedVersion` in _helpers.tpl (which the
#      `terminology.image.tag` default and the chart's tag-versus-digest
#      refusal both read), and the DIGEST equals `terminology.image.digest` in
#      values.yaml (which is what every install actually runs). Without this the
#      annotation is a fourth hand-maintained copy of a version: Artifact Hub
#      would scan whatever `ferroterm:<tag>` had moved to while the chart
#      deployed a pinned digest, and the vulnerability report would describe
#      software nobody is running.
#   4. Chart.yaml carries NONE of the injected annotations. A committed
#      `artifacthub.io/changes`, `containsSecurityUpdates` or `prerelease` key
#      would collide with the injected one and leave the packaged metadata
#      depending on YAML key-collision behaviour.
#   5. The chart README — a GENERATED file, and the one Artifact Hub renders as
#      the package front page — restates the same appVersion. It is generated
#      FROM Chart.yaml, so a disagreement means it was never regenerated.
#
# Usage: scripts/checks/chart-appversion.sh [--self-test]
#   --self-test mutates a COPY of the chart tree six ways — a first-party tag
#   moved off appVersion, an image line with its party marker removed, and each of
#   the FerroTERM tag and digest moved off its pin from either side (the
#   annotation, the helper, the values file) — and asserts the guard rejects each.
#   The detectors are arithmetic over text, and a text detector never shown to fail
#   is a green light nobody has tested (reliability.md §enforcement tiers).
# Callers: the `chart-appversion` job in ci.yml, and the `plan` job of
# release.yml (which re-runs the whole guard tier at the tagged commit; plan
# separately asserts the tag equals the workspace version, so property 1 there
# transitively pins appVersion == ${TAG#v}).

set -euo pipefail

if [[ "${1:-}" == "--self-test" ]]; then
  cd "$(dirname "$0")/../.."
  self_root="$(mktemp -d)"
  trap 'rm -rf "$self_root"' EXIT
  mkdir -p "$self_root/deploy/helm" "$self_root/scripts"
  cp -R deploy/helm/ferroehr "$self_root/deploy/helm/ferroehr"
  cp -R scripts/checks scripts/lib "$self_root/scripts/"
  cp Cargo.toml "$self_root/Cargo.toml"
  self_chart="$self_root/deploy/helm/ferroehr/Chart.yaml"
  self_helpers="$self_root/deploy/helm/ferroehr/templates/_helpers.tpl"
  self_values="$self_root/deploy/helm/ferroehr/values.yaml"
  self_fail=0
  # <label>|<file>|<sed program>
  mutations=(
    "a first-party tag moved off appVersion|${self_chart}|s|^      image: ghcr.io/ferrohealth/ferroehr:.*|      image: ghcr.io/ferrohealth/ferroehr:0.0.1  # party: first|"
    "an image line with no party marker|${self_chart}|s|^\\(      image: ghcr.io/ferrohealth/ferroehr:[^ ]*\\)  # party: first|\\1|"
    "the FerroTERM tag moved off the helper pin|${self_chart}|s|ferroterm:[0-9][^@]*@|ferroterm:9.9.9@|"
    "the FerroTERM digest moved off the values pin|${self_chart}|s|@sha256:[0-9a-f]\\{8\\}|@sha256:0000c0de|"
    "the helper pin moved without the annotation|${self_helpers}|s|^[0-9]*\\.[0-9]*\\.[0-9]*$|9.9.9|"
    "the values digest moved without the annotation|${self_values}|s|^    digest: sha256:[0-9a-f]\\{8\\}|    digest: sha256:0000c0de|"
  )
  for mutation in "${mutations[@]}"; do
    label="${mutation%%|*}"
    rest="${mutation#*|}"
    file="${rest%%|*}"
    program="${rest#*|}"
    cp "$file" "${file}.orig"
    sed "$program" "${file}.orig" > "$file"
    if cmp -s "${file}.orig" "$file"; then
      echo "::error::--self-test could not apply the mutation '${label}' — its anchor no longer matches, so this case proves nothing." >&2
      self_fail=1
    elif (cd "$self_root" && bash scripts/checks/chart-appversion.sh >/dev/null 2>&1); then
      echo "::error::--self-test: the guard ACCEPTED ${label}." >&2
      self_fail=1
    else
      echo "  self-test: rejected ${label}"
    fi
    mv "${file}.orig" "$file"
  done
  # And the unmutated copy must still pass, or every rejection above is just a
  # broken fixture rejecting itself.
  if ! (cd "$self_root" && bash scripts/checks/chart-appversion.sh >/dev/null 2>&1); then
    echo "::error::--self-test: the guard REJECTED the unmutated tree." >&2
    self_fail=1
  fi
  [[ "$self_fail" -eq 0 ]] || exit 1
  echo "chart-appversion --self-test: every mutation rejected, the unmutated tree accepted — OK."
  exit 0
fi

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

CHART=deploy/helm/ferroehr/Chart.yaml
MANIFEST=Cargo.toml
HELPERS=deploy/helm/ferroehr/templates/_helpers.tpl
VALUES=deploy/helm/ferroehr/values.yaml

for required in "$CHART" "$MANIFEST" "$HELPERS" "$VALUES"; do
  [[ -f "$required" ]] || { echo "::error::missing $required" >&2; exit 1; }
done

failures=0
report() {
  echo "::error::$1" >&2
  failures=$((failures + 1))
}

app=$(sed -nE 's/^appVersion: *"?([^"]+)"?$/\1/p' "$CHART")
if [[ -z "$app" ]]; then
  report "could not read appVersion from $CHART."
  exit 1
fi

workspace=$(sed -nE 's/^version = "(.*)"$/\1/p' "$MANIFEST" | head -1)
if [[ -z "$workspace" ]]; then
  report "could not read the workspace version from $MANIFEST."
  exit 1
fi

# 1. appVersion equals the workspace version (the compose-image-tags contract).
if [[ "$app" != "$workspace" ]]; then
  report "$CHART appVersion is $app but the workspace version is $workspace — the release cut bumps appVersion beside the compose image tags (one version sweep, .claude/rules/changelog.md). Refresh appVersion + the artifacthub.io/images tags + the generated README: helm-docs --chart-search-root deploy/helm/ferroehr --template-files README.md.gotmpl"
fi

# 2. Every image entry is CLASSIFIED, and the first-party ones agree with
# appVersion. The marker is the one release-facts.sh reads, so the set checked
# here and the set rewritten at package time are the same set by construction.
IMAGE_LINE='^[[:space:]]+image: [^[:space:]]+'
first_party=0
while IFS= read -r line; do
  case "$line" in
    *"# party: first")
      first_party=$((first_party + 1))
      ref="${line#*image: }"
      ref="${ref%%[[:space:]]*}"
      [[ "$ref" == *":${app}" ]] || report "$CHART lists first-party image '${ref}', whose tag is not appVersion (${app}) — the release cut bumps appVersion beside the artifacthub.io/images tags (one version sweep, .claude/rules/changelog.md), and the publish lane rewrites exactly these entries."
      ;;
    *"# party: third") ;;
    *)
      ref="${line#*image: }"
      report "$CHART lists image '${ref% *}' with no '# party: first' / '# party: third' marker. An image is first party by DECLARATION, not by how its repository is spelled: the marker decides whether deploy/helm/release-facts.sh rewrites its tag at every release and whether this guard holds it to appVersion, and an unmarked entry would silently get neither."
      ;;
  esac
done < <(grep -E "$IMAGE_LINE" "$CHART" || true)
if [[ "$first_party" -eq 0 ]]; then
  report "$CHART declares no first-party artifacthub.io/images entry — the publish lane rewrites those tags at package time and would rewrite nothing."
fi

# 3. The third-party FerroTERM entry equals the two places the chart pins it.
# Both are read out of the chart itself rather than restated here: a guard
# carrying its own copy of a version is the fourth copy it exists to prevent.
terminology_pin=$(sed -nE '/define "ferroehr\.terminologyPinnedVersion"/{n;s/^[[:space:]]*([0-9][0-9A-Za-z.+-]*)[[:space:]]*$/\1/p;}' "$HELPERS")
terminology_digest=$(sed -nE '/^  image:$/,/^  [a-z]/{s/^    digest: (sha256:[0-9a-f]{64})$/\1/p;}' "$VALUES" | tail -1)
if [[ -z "$terminology_pin" ]]; then
  report "could not read the FerroTERM pin from $HELPERS — 'ferroehr.terminologyPinnedVersion' must be a define whose next line is the bare version, because this guard and the chart's own tag default both read it there."
elif [[ -z "$terminology_digest" ]]; then
  report "could not read terminology.image.digest from $VALUES — the chart pins FerroTERM by digest, and this guard holds the published annotation to that pin."
else
  want="ghcr.io/rubentalstra/ferroterm:${terminology_pin}@${terminology_digest}"
  have=$(grep -oE 'image: ghcr\.io/rubentalstra/ferroterm[^[:space:]]*' "$CHART" | head -1)
  have="${have#image: }"
  if [[ "$have" != "$want" ]]; then
    report "$CHART lists FerroTERM as '${have:-<absent>}' but the chart deploys '${want}' (tag from ferroehr.terminologyPinnedVersion in $HELPERS, digest from terminology.image.digest in $VALUES). Artifact Hub scans the reference in the annotation, so a stale one reports vulnerabilities for an image nobody runs."
  fi
fi

# 4. None of the injected annotations is committed.
for injected in artifacthub.io/changes artifacthub.io/containsSecurityUpdates artifacthub.io/prerelease; do
  if grep -q "^  ${injected}:" "$CHART"; then
    report "$CHART commits ${injected}, which build-chart.yml injects at package time — two declarations of one key make the packaged metadata depend on YAML key-collision behaviour."
  fi
done

# 5. The generated README agrees with it.
README=deploy/helm/ferroehr/README.md
if [[ -f "$README" ]]; then
  if ! grep -qF -- "--set image.tag=${app}" "$README"; then
    report "$README teaches an image.tag other than the committed appVersion ($app) — it is generated from Chart.yaml, so regenerate it: helm-docs --chart-search-root deploy/helm/ferroehr --template-files README.md.gotmpl"
  fi
else
  report "no $README — the chart publishes a generated README and this check has nothing to read."
fi

if [[ "$failures" -gt 0 ]]; then
  exit 1
fi
echo "chart-appversion: appVersion $app equals the workspace version, the first-party artifacthub.io/images tags and the generated README agree with it, and no injected annotation is committed — OK."
