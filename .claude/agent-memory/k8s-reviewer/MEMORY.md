# Memory index

- [Deployment schema-name surfaces](deployment-schema-name-surfaces.md) — a DB schema rename must be swept through four independent copies; pg_dump silently omits a stale `--schema`
- [Chart review read-only procedure](chart-review-read-only-procedure.md) — what validate.sh/helm cover locally, and the chart-version-guard + chart-boot gaps a reviewer must check by hand
- [Helm hook deletion semantics](helm-hook-deletion-semantics.md) — hook-succeeded runs after ALL hooks; on failure earlier hooks are deleted (v4.2.3 source)
