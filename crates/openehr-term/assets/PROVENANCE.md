# Vendored openEHR terminology assets

- Source: https://github.com/openEHR/specifications-TERM
- Ref: master (TERM 3.1.0)
- Commit: `78edd7f59600c40da00fa4e8a1282d563b8942cc`
- License: CC-BY-SA 3.0 Unported (the upstream repo's `LICENSE`; root
  reference copy `LICENSE-CC-BY-SA-3.0`) — redistributed verbatim with
  attribution.
- Upstream paths:
  - `computable/XML/{en,es,ja,pt,zh}/openehr_terminology.xml` → `assets/<lang>/openehr_terminology.xml`
  - `computable/XML/openehr_external_terminologies.xml` → `assets/openehr_external_terminologies.xml`
  - `computable/XML/PropertyUnitData.xml` → `assets/PropertyUnitData.xml`
  - `computable/XML/schema/*.xsd` → `assets/schema/`

The computable form is the definitive expression of the openEHR Support
Terminology (`docs/specs/openehr/TERM/docs/SupportTerminology/master02-overview.adoc`),
so every asset here must stay **byte-identical** to the upstream file at the
pinned commit — never "clean up", reformat, or re-indent them; known upstream
defects (e.g. SPECPR-51) are handled in access logic with a citation, never by
editing the asset.

Vendored by `scripts/vendor/spec-docs.sh`, from the same checkout it vendors
the spec text + computable XML at `docs/specs/openehr/TERM/` from (see that
`PROVENANCE.md`); the script also rewrites the commit line above. The
`asset_identity` test in `tests/it/` byte-compares this directory against that
copy. The XSDs are copied here although the spec-text vendoring excludes
them.
