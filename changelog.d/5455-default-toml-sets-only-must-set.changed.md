- The shipped `ferroehr.default.toml` sets only the keys of its "What you
  must set" block: `[deployment] profile`, `[api] system_id` and
  `[privacy] subject_namespaces` are its live lines, and every other key is
  a `#?` line that shows its default. A copy of the file sets 3 keys where
  it set 115 (122 in 5.7.0); this change removes and adds no key. The
  book's "Operational policy" page merges "Which checks read a grant" into
  "What the grants decide".
