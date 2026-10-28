<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Changelog fragments

A pull request with a user-visible effect adds one file here instead of
editing `CHANGELOG.md`, so two pull requests never touch the same lines.
User-visible means the REST surface, AQL semantics, validation, storage and
migrations, configuration, the CLI, the container, compose and Helm
artifacts, and feature flags (`.claude/rules/changelog.md`).

- **Name:** `<issue>-<kebab-slug>.<section>.md`, for example
  `4123-changelog-fragments.changed.md`. `<section>` is a Keep a Changelog
  1.1.0 section in lower case: `added`, `changed`, `deprecated`, `removed`,
  `fixed` or `security`.
- **Content:** the entry exactly as it reads in `CHANGELOG.md`, one or more
  list items that open with `- ` and continue on lines indented by two spaces,
  with no heading and no blank line inside. A fragment carries no SPDX header:
  its text moves into `CHANGELOG.md`, which carries the header.
- **Issue reference:** the cut ends the first paragraph of each entry with
  `(#<issue>)`, the number the file name starts with, unless the entry already
  names that issue, so the release notes link every entry to its issue
  without the fragment spelling it out.

The cut writes each section as a loose list, with a blank line between
entries, as the rest of `CHANGELOG.md` reads; the fragment stays tight.

## Security entries

A `security` fragment names its advisory itself: the `GHSA-xxxx-xxxx-xxxx` id
of the repository advisory drafted for the fix, or the path of the OpenVEX
statement under `security/vex/` that judges it, which must exist. The cut copies
the fragment's text into the release notes, so the advisory reaches them.

When a release carries `security` fragments beside any other section, its
`### Security` section opens with a reason line (CRA Annex I Part II(2) and
`.claude/rules/changelog.md`). That line is a fragment of its own:

- **Name:** `<issue>-<kebab-slug>.security-reason.md`, at most one at a time.
- **Content:** one paragraph that opens with `Shipped with functional changes
  because: ` and the reason, with no list item and no blank line inside. It is
  copied as written, with no issue reference added.

The cut places it first under `### Security`, before the entries, and deletes
it with the other fragments. A reason with no `security` entry beside it is
refused.

## Checks

`scripts/release/changelog.sh --check` validates every fragment, and CI runs it.
`scripts/release/changelog.sh --preview` prints `CHANGELOG.md` with every
fragment folded into `[Unreleased]`, as the next cut would write it, and
`scripts/release/changelog.sh --check-preview` runs
`scripts/checks/changelog-structure.sh` and
`scripts/checks/changelog-security.sh` over that preview; CI runs it too, so a
Security entry without its advisory, or a missing reason, fails the pull
request that adds it.

`scripts/checks/changelog-guard.sh <base> <head>` is the pull-request half: it
asks for a fragment when the change touches a user-visible surface, and it
refuses a change that writes under `[Unreleased]` in `CHANGELOG.md`, with or
without the `no-changelog` label.

The release cut runs `scripts/release/changelog.sh --assemble <version> <date>`,
which moves the fragments into `CHANGELOG.md` under the new version and
deletes them; the release `plan` job refuses a tag that still has a fragment
here. A pre-release cut (`5.6.0-rc1`) copies the fragments into its section
and keeps them, so every release candidate and the stable release of that
version each carry every change since the last stable release; at a
pre-release tag the `plan` job checks that the section carries each fragment
(`changelog.sh --covered <version>`). This README is not a fragment.
