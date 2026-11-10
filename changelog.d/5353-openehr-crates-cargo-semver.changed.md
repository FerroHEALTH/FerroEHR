- The nine `openehr-*` crates follow The Cargo Book's SemVer rules, judged
  by `cargo-semver-checks` 0.51.0 against their newest release on crates.io
  on every pull request, in the merge queue and in the release plan. Their
  next publish moves the line from `0.0.x` to `0.1.0`. From then on a
  compatible change ships as a patch and only a breaking change moves the
  minor, so a caret requirement such as `openehr-rm = "0.1"` takes every
  compatible release. A public dependency moved to another compatibility
  line counts as a breaking change.
