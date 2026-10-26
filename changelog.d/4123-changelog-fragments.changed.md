- **A pull request records its changelog entry as a fragment under
  `changelog.d/` and no longer edits `CHANGELOG.md`** (#4123). The fragment is
  `changelog.d/<issue>-<kebab-slug>.<section>.md`, in the format
  `changelog.d/README.md` gives, and the release cut assembles the fragments
  into the new version's section with `scripts/release/changelog.sh --assemble`.
  The `changelog-guard` job refuses a malformed fragment and a pull request that
  writes under `[Unreleased]`, so two open pull requests no longer conflict on
  the changelog.
