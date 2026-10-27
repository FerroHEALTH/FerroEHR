- **The Helm chart's `usageReport.enabled` reaches the other products it
  runs**: FerroTERM (`FERROTERM__USAGE_REPORT__ENABLED`) and a FerroBRIDGE
  release from 0.0.7 on (`[usage_report]`) follow it, each overridable with
  `terminology.usageReport.enabled` or `bridge.usageReport.enabled`. The
  FerroPIX dependency chart reads only `ferropix.usageReport.enabled`, so the
  chart now refuses to render `usageReport.enabled=false` while that stays
  true: a release you switched off sent FerroPIX's report regardless. Upgrading
  a release that sets `usageReport.enabled=false` with FerroPIX on takes
  `--set ferropix.usageReport.enabled=false` as well. Chart 20.0.0.
