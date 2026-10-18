---
name: ferroui
description: "Every console draws in FerroUI, pinned exactly; one uniform design, shared components, all four consoles move in lockstep with every FerroUI release, screenshots read and reviewed; owner 2026-10-18"
metadata:
  type: project
---

Every FerroHEALTH console (FerroBRIDGE, FerroFED, FerroEHR, FerroTERM) draws
in FerroUI, the shared component library (`ferrohealth-ui` on crates.io,
FerroHEALTH/FerroUI), pinned exactly (`=0.1.x`) from crates.io, never a
submodule or a path dependency (owner, 2026-10-18).

- **One uniform design.** Each component takes the best of the four
  products' versions on its merits and is restyled to one type scale,
  spacing, radius, hover and focus treatment, so the consoles read as one
  family. Each product keeps only its own brand palette.
- **Shared, never single use.** A component this console needs and FerroUI
  lacks is filed on FerroUI (label `component`) and built there, never as a
  local one-off.
- **All four move in lockstep.** Every FerroUI release is rolled out to all
  four consoles at once: a pin-bump pull request in each repository, merged
  together, so no console runs an older FerroUI than the others. The
  rollout is coordinated from the FerroBRIDGE session; only the pin, its API
  changes and the visual fixes it brings change here, and anything else
  found is filed on this tracker.
- **Look at the screens, not only the code.** Every UI change captures every
  screen at 1440 and 390 px, light and dark, plus hover, focus and open
  states; each image is read, every defect (overflow, bad wrapping,
  misalignment, dead space, contrast) is fixed and recaptured, and the
  orchestrator reviews the screenshots before a merge or tag.
- **Theme switch:** FerroEHR's moon and sun icon button in the top bar, the
  choice kept in the server cookie written as `data-theme`.
