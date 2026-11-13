# Every configuration key

This page shows the shipped `ferroehr.toml` template, [`ferroehr.default.toml`][template]. It
holds every key FerroEHR reads, each under its description with its default.
`tools/config-reference` generates it from the configuration types, so it matches the server you
run.

- The file opens with **What you must set**: the keys a deployment that holds real data sets.
  Every other key is optional, and its note says so.
- Each key's note ends with whether you must set it and why it is a key: infrastructure, trust,
  or neither for a tuning key with a built-in default.
- Only the keys of **What you must set** are live lines, so a copy of the file sets nothing else.
- Every other key is a `#?` line, showing its default or, for a key with no default or a secret,
  a placeholder. Remove the `#?` marker and the space after it, then edit the value, to set it.
- A `# [table.<name>]` header shows a table you name yourself, with its keys commented out.

Every key can also be set as an environment variable: `FERROEHR`, then the TOML path with `__`
between the segments, such as `FERROEHR__DATABASE__MAX_CONNECTIONS`. The guides under
[Configuration reference](configuration.md) explain the sections.

```toml
{{#include ../../../../app/ferroehr/assets/ferroehr.default.toml}}
```

[template]: https://github.com/FerroHEALTH/FerroEHR/blob/main/app/ferroehr/assets/ferroehr.default.toml
