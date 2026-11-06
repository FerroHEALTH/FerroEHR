- The family crates move together: `ferrohealth-smart` 0.9.1,
  `ferrohealth-authz` 0.3.0, `ferrohealth-session` 0.3.1, `ihe-atna` and
  `ehds-logging` 0.4.0, `ihe-iti` 0.0.76, `ferrohealth-config` 0.2.10,
  `ferrohealth-server` 0.5.1 and `ferrohealth-ui` 0.6.9. `ihe-atna` and
  `ehds-logging` now escape every control and bidirectional formatting
  character of a value they write as its code point. With `ferrohealth-ui`
  0.6.9 the viewer's session watch no longer throws when the party
  relationship form, which holds a control named `target`, is submitted.
- The terminology client moves to `ferrobridge-term` 0.1.143 (FerroBRIDGE
  v0.0.22), which takes the `ferrohealth-smart` 0.9 token source.
