# Provenance: IHE Basic and Advanced Patient Privacy Consents

- Source: IHE International. BPPC from the IT Infrastructure (ITI)
  Technical Framework as published in HTML at
  <https://profiles.ihe.net/ITI/TF/index.html>; APPC from the Trial
  Implementation supplement linked from that index and from Volume 1
  chapter 43, <https://profiles.ihe.net/ITI/TF/Volume1/ch-43.html>
- Pin: each file by its sha256 below. The HTML Technical Framework is
  republished in place at each revision, so a re-run fails once IHE
  publishes a new one; move the pins then.
- Document:
  - BPPC, ITI TF-1 chapter 19 (`ITI/TF/Volume1/ch-19.html`, the profile:
    use cases, policy identifiers, actors, options, process flow, security
    considerations) and ITI TF-3 section 5.1 (`ITI/TF/Volume3/ch-5.1.html`,
    the Patient Privacy Consent Acknowledgment Document content module).
    Each page heads itself "IHE IT Infrastructure (ITI) Technical Framework,
    Volume Revision 20.0, August 4, 2023 - Final Text"; the index read on
    2026-11-13 names Revision 20.2 of November 11, 2025 as the latest.
  - APPC, "IHE IT Infrastructure Technical Framework Supplement, Advanced
    Patient Privacy Consents (APPC), Rev. 1.4 - Trial Implementation",
    dated July 2, 2021: chapter 43 (the profile) and ITI TF-3 section 5.6
    (the Privacy Consent Document content module, an XACML 2.0 policy set)
  - The IHE Technical Frameworks General Introduction, chapter 9, Copyright
    Licenses (`GeneralIntro/ch-9.html`), the grant this copy rests on
- Retrieved: 2026-11-13
- Upstream licence: read for this copy on 2026-11-13. The General
  Introduction, chapter 9, vendored here, grants "to any other user of these
  documents, an irrevocable, worldwide, perpetual, royalty-free,
  nontransferable, nonexclusive, non-sublicensable license under its
  copyrights in any IHE profiles and Technical Framework documents ... to
  reproduce and distribute" them. The supplement carries "Copyright © 2021:
  IHE International, Inc." and the pages "© 2000 - IHE International".
  `REUSE.toml` declares the tree
  `LicenseRef-IHE-Technical-Framework-Copyright`, whose text quotes the
  grant. XACML 2.0, which APPC section 5.6.1 references, is an OASIS
  standard and is not taken.
- What is taken: the four files above, each whole, without the stylesheets,
  scripts and images the pages link. The APPC chapter 43 page of the HTML
  Technical Framework is not taken: it holds only the link to the
  supplement.
- Layout: the host, then the path, as IHE serves them
- Fetched by: `scripts/vendor/ihe-consents.sh`; never hand-edit, re-run
  the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
| `profiles.ihe.net/ITI/TF/Volume1/ch-19.html` | `a35ecb229ef67302051e92b13f3cd59ee12e67bd9d463b7166f720b6b055455f` |
| `profiles.ihe.net/ITI/TF/Volume3/ch-5.1.html` | `82d220535db84971ed0162a8674e384938d7ae944a82cb217200391933c5b7b8` |
| `profiles.ihe.net/GeneralIntro/ch-9.html` | `2056d5b878f1bee7f3881cada792ec352476613b495afc677b9a467b9d2197cf` |
| `www.ihe.net/uploadedFiles/Documents/ITI/IHE_ITI_Suppl_APPC.pdf` | `823bd5e015ddfb384a19f80ed93e9d1946740393a4be057fd8c9ab6b01936282` |
