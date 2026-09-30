# Vendored openEHR spec docs: ITS-REST

- Source: https://github.com/openEHR/specifications-ITS-REST
- Ref: Release-1.1.0 (released 19-Jul-2026; matches the vendored OAS identity)
- Commit: `24058992d5fa96e8dfbd855d9c133f328387fc09`
- License: Apache-2.0 — the upstream `LICENSE` is vendored verbatim alongside
  this file, from the same pinned commit. Root reference copies:
  `LICENSE-CC-BY-SA-3.0` / `LICENSE-APACHE-2.0`.
- Vendored by: `scripts/vendor/spec-docs.sh` (text formats only: adoc md txt csv json yaml yml robot xml opt g4)
- No UML class-diagram SVGs: the vendored chapters of this component
  reference none.
- Plus the 4 per-document figure(s) under
  `docs/<doc_name>/diagrams/` and `docs/<doc_name>/images/` that the vendored
  chapters reference as `image::{diagrams_uri}/<name>` /
  `image::{images_uri}/<name>`, taken byte-for-byte from the same pinned
  commit. Referenced files only — the upstream figure directories are not
  mirrored wholesale.
- Unreferenced figures, UML `.xmi`/`.mdzip`, XSDs and other binaries
  excluded — fetch from the repo at the pinned commit.

Do not hand-edit files under this directory; re-run the script instead.
