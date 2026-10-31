<!-- Describe the change itself. No AI/tool attribution anywhere in this PR. -->

## What this changes

Closes #<!-- tracker issue number — the merge into main auto-closes it -->

## Checks

- [ ] `cargo fmt --all --check` · `cargo clippy --workspace --all-targets` · `cargo nextest run --workspace`
- [ ] No test weakened, skipped, or edited to route around a bug
- [ ] User-visible change → a fragment, `changelog.d/<issue>-<kebab-slug>.<section>.md` (`changelog.d/README.md`), never an edit of `CHANGELOG.md` (else the `no-changelog` label)
- [ ] REST/config/CLI/deployment change → matching `website/book/src` page updated
- [ ] Implemented `docs/plans/*.md` plan file deleted (unless another open issue still consumes it)
- [ ] New issues opened from this work are linked as GitHub relationships (sub-issue / blocked-by via `scripts/gh/rel.sh`), not prose (`.claude/rules/issue-relationships.md`)

## Privacy boundary

<!-- Required when this change touches the demographic or linkage migrations,
`service::demographic`, `service::linkage`, the identifier scanner, the
access-event model or an outbox payload builder; the privacy-boundary-guard job
checks it then. Rules: CONTRIBUTING.md § Personal data. -->

- [ ] The data flow is described above: what personal data this change reads, writes or exports, and where it goes
- [ ] The roles are named: which database roles and which caller roles reach that data
- [ ] No new grant spans the clinical and demographic domains
- [ ] Synthetic data only in tests, fixtures, seeds and examples
- [ ] An access event is emitted wherever this change added a read of personal data

<!--
HARD RULE: this PR description, its title, and all commits must contain NO
AI/Claude attribution (no "Co-authored-by: Claude", no "Generated with Claude
Code", no robot emoji or 🤖). Describe only the change.
-->
