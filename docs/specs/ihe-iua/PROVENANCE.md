# Provenance: the IHE IUA Technical Framework Supplement

- Source: <https://github.com/IHE/ITI.IUA>, rendered at
  <https://profiles.ihe.net/ITI/IUA/index.html>
- Pin: tag `2.5`, which resolves to commit `a8e20c92928b7560ee07a410922187757248c8b9`, each file
  checked against the sha256 below, the same bytes as
  FerroHEALTH/FerroFED vendors in `docs/specs/ihe-iua/`
- Document: Internet User Authorization (IUA), Revision 2.5, Trial
  Implementation, dated June 18, 2026: Volume 1 §34 (the profile) and Volume 2
  §3.71 Get Access Token [ITI-71], §3.72 Incorporate Access Token [ITI-72],
  §3.102 Introspect Token [ITI-102] and §3.103 Get Authorization Server
  Metadata [ITI-103], with the JWT and introspection extension claims
  (`subject_name`, `person_id` and the others) the party binding reads
- Retrieved: 2026-11-05
- Upstream licence: read again for this copy on 2026-11-05. The
  repository's `LICENSE` is the Creative Commons Attribution 4.0
  International Public License, vendored beside this file. The IHE
  Technical Frameworks General Introduction, chapter 9
  (<https://profiles.ihe.net/GeneralIntro/ch-9.html>), also grants "to any
  other user of these documents, an irrevocable, worldwide, perpetual,
  royalty-free, nontransferable, nonexclusive, non-sublicensable license
  under its copyrights in any IHE profiles and Technical Framework
  documents ... to reproduce and distribute ... such IHE Technical
  Documents". The attribution is this file: the supplement is IHE
  International's. `REUSE.toml` declares the tree `CC-BY-4.0`.
- What is taken: the supplement's whole text, the licence and the eight
  figures the text links. The slide deck the actor diagrams are drawn in,
  the two older actor diagrams the text does not link, and the build script,
  stylesheet and README are not taken. The status is the document's own: a
  Trial Implementation supplement may be amended before it enters the
  Technical Framework.
- Layout: the upstream paths, unchanged
- Fetched by: `scripts/vendor/ihe-iua.sh`; never hand-edit, re-run the
  script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
| `IHE_ITI_Suppl_IUA.md` | `af63d1ddfb5b908bc88d7c01490d56a036a7666145680171a71c8c95d2e83f95` |
| `LICENSE` | `9e5f1b3c610b9c2da5c313bf81d577a7d1acec686bdb0384edefa6df0f90cd94` |
| `media/1.2_IUA-actor-diagram.png` | `35b61c3f14961d1cbf155dfe8f8a769195e5f1fe8fd062e2687dbe9ecfa8184c` |
| `media/IHE-logo.jpeg` | `cf435f56b459a56790905268c416d24f26d100cdf81c0ac4e427e315c1e5b2c6` |
| `media/authorization-code-grant.png` | `4ba1719104defb2b93a3ac605fc2b6abb51d8618fe60e127aa53ff8bf518ca3a` |
| `media/basic-flow.png` | `c38ef7ce0f84aadcebe33ebd78246785e912fa32d2ab076f13eb16fa86f1e5ad` |
| `media/client-credential-grant.png` | `961a1b4c0ca1e88e547be1b0b465b094d48e4a4c591b0376245fc69967facc56` |
| `media/get-authorization-server-metadata.png` | `2198221403eb9200be567e716fd02407c72da0a2256c32039a8b3b4a55e7d5b2` |
| `media/incorporate-token.png` | `e533a7e1d1a6d91bad16a6daefdce85cc4e6ec350585bc44f21e11f7c4bde438` |
| `media/introspect.png` | `c1b7ae8e2a233a1a73264d57ddc604bb5f020af7c957d897d16206797c784cc3` |
