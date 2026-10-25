---
name: ferrohealth-umbrella
description: "FerroHEALTH is the family name of the five products; family-level documentation belongs in the FerroHEALTH book, not in one product's book; the umbrella repository is paused (owner, 2026-10-25)"
metadata:
  type: project
---

**FerroHEALTH** is the family name; FerroEHR is one of its five products,
beside FerroBRIDGE, FerroFED, FerroPIX and FerroTERM ([[family-architecture]]).
Cadasto B.V. is the manufacturer of every product. The family site is
<https://ferrohealth.eu> (repo FerroHEALTH/FerroHEALTH, local
`../FerroHEALTH`); each product documents itself on its own domain.

The FerroHEALTH umbrella repository is paused (owner, 2026-10-25): do no
work there until the owner says otherwise. The pages it already carries stay
where they are, and FerroEHR keeps linking them.

**Why:** owner 2026-10-08: "most of our documentation needs to go to
FerroHEALTH because not everything is only for FerroEHR… it's about the
bundled products" and "otherwise we are duplicating so much of the same
things". Tracked as FerroHEALTH#37 (the family mdBook), with FerroEHR#3701
(v4.3.6) as FerroEHR's part.

**How to apply:** before writing a page in FerroEHR's book, ask whether it is
about FerroEHR alone. The first move (#3701, before v4.3.6) put the
manufacturer, the shared security policy, the post-market procedure, the
CRA's manufacturer side, the EHDS intended purpose and shared responsibility,
licensing and the family architecture at `https://ferrohealth.eu/docs/`
(`manufacturer.html`, `security.html`, `post-market.html`, `cra.html`,
`ehds/intended-purpose.html`, `ehds/shared-responsibility.html`,
`licensing.html`, `architecture.html`); FerroEHR keeps short pages at the old
URLs with only its own facts. The information sheet, the instructions for use
and the generated EHDS readiness and technical-documentation pages stay here
(the release lane attaches or checks them per release). Product-specific
pages (install, config, API, its own CRA risk assessment and Annex II user
information, hazard log, claims review, control matrix) stay here. Never copy
family material back in; while the umbrella is paused, a new family-level
page is raised with the owner rather than written. The page table is
`.claude/rules/docs-website.md`. See [[ehds-cra-manufacturer-posture]].
