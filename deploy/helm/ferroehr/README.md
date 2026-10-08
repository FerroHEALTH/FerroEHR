# ferroehr

Pure-Rust, openEHR-conformant clinical data repository (ITS-REST 1.1.0 + AQL 1.1). A single static binary deployed with a hardened-by-default security posture: runs as a non-root, read-only-rootfs workload whose NetworkPolicy admits its serving port only, and that connects to an EXTERNAL PostgreSQL 18 as an unprivileged app role, with schema preparation on its own credential. A stock install runs the sandbox deployment profile, which must not hold real patient data; set config.deployment_profile for a production holder.

![Version: 11.0.0](https://img.shields.io/badge/Version-11.0.0-informational?style=flat-square) ![Type: application](https://img.shields.io/badge/Type-application-informational?style=flat-square) ![AppVersion: 4.3.6](https://img.shields.io/badge/AppVersion-4.3.6-informational?style=flat-square)

FerroEHR is a pure-Rust openEHR Clinical Data Repository: ITS-REST 1.1.0 at the
API, AQL 1.1 as the query language, PostgreSQL 18-native storage, shipped as a
single static binary. This chart deploys the server.

## Before you install: four things that surprise people

**This chart does not deploy a database.** It expects an **external PostgreSQL
18** (18.6 or newer) and will not start without one. Point it at your own
instance, a managed service, or a separate PostgreSQL chart: `database.url`
or, for production, `database.existingSecret`.

**With no authentication mechanism configured, the server boots and answers
`401` to everything.** `auth.enabled` defaults to `true`, and a deployment with
no `[auth.oidc]` issuer and no Basic user has nothing to authenticate against.
That is fail-closed and deliberate: the alternative is an openEHR repository
serving patient data to anonymous callers. Configure a mechanism, or set
`config.auth.enabled: false` for a throwaway evaluation.

**A stock install runs the `sandbox` profile.** `config.deployment_profile`
defaults to `sandbox`, the binary's default, which must not hold real patient
data and names every missing separation on its boot banner. A deployment
holding patient data sets `production`, which refuses to start until each
separation is made or accepted by name; the comment above the key in
`values.yaml` lists the ones these defaults leave open. The chart ships
`values-production.yaml`, an overlay that sets `production`, makes those
separations and accepts the plaintext pod listener behind a TLS ingress by
name; its comments name each example value to replace
(`helm pull --untar`, then `-f ferroehr/values-production.yaml`).

**A secret set in the wrong place fails the render on purpose.** See
[Secrets](#secrets): this chart refuses to put a credential in a ConfigMap
and says so, never quietly.

## Install

The chart is published as an **OCI artifact**. There is no chart repository
to add; `helm repo add` does not apply to this chart:

```console
helm install ferroehr oci://ghcr.io/ferrohealth/charts/ferroehr \
  --version 11.0.0 \
  --namespace ferroehr --create-namespace \
  --set database.existingSecret=ferroehr-db \
  --set image.tag=4.3.6
```

OCI registries require Helm 3.8 or newer.

### Pin two versions, not one

They are independent SemVer lines and they move independently:

| What | Set with | This release |
|---|---|---|
| the **chart** (templates, defaults, this document) | `--version` | `11.0.0` |
| the **server image** | `image.tag` | `4.3.6` |

`appVersion` is the image the chart defaults to; pinning `image.tag` explicitly
is what keeps an upgrade of one from silently moving the other.

### Verify what you pulled

The chart carries two keyless Sigstore artifacts, and they answer different
questions. A **cosign signature:** who signed this:

```console
cosign verify ghcr.io/ferrohealth/charts/ferroehr:11.0.0 \
  --certificate-identity-regexp '^https://github\.com/FerroHEALTH/FerroEHR/\.github/workflows/publish-chart\.yml@' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

A **SLSA build provenance attestation:** what source it was built from, and how:

```console
gh attestation verify oci://ghcr.io/ferrohealth/charts/ferroehr:11.0.0 \
  -R FerroHEALTH/FerroEHR
gh attestation verify oci://ghcr.io/ferrohealth/ferroehr:4.3.6 \
  -R FerroHEALTH/FerroEHR
```

`helm install --verify` does **not** apply: it reads a PGP `.prov` provenance
file, and this chart ships none, deliberately: keeping a long-lived
private key in CI is a worse posture than not having one. The two commands above
are what replace it.

### Your values file is validated

The chart ships a `values.schema.json`, so `helm install`, `upgrade`, `lint` and
`template` refuse a values file that misspells a key of the chart's own
vocabulary, gets a type wrong, or names a value outside the permitted set,
instead of rendering and ignoring it.

Everything under `config:` stays deliberately open: that vocabulary is the
**server's** (`ferroehr config default` prints it in full), it is validated by
the binary at boot, and duplicating it here would fork it. So a typo under
`config:` is caught when the pod starts, not when the chart renders.

## Secrets

Every value under `secrets:` is carried by a chart-managed Secret, and **how it
reaches the process differs by design**. A secret with a `*_file` sibling in the
server's configuration is **mounted read-only** at
`/etc/ferroehr-secrets/<config.path>`, and only the *path* is passed as an
environment variable, because an environment variable is readable through
`/proc/<pid>/environ` and inherited by every child process. The few keys with no
`*_file` sibling still pass their value as env; that is a gap in the
configuration tree, not a choice this chart made.

**Setting a secret under `config:` refuses to render.** Values under `config:`
become a ConfigMap, which is not a sensitive object: it is readable with
namespace read, collected by backup tooling that skips Secrets, and unencrypted
at rest even where Secret encryption is enabled. The refusal names the
`secrets:` key that carries the value safely.

**A secret with no `secrets:` route at all moves the whole configuration into the
Secret**, and then **no ConfigMap is created**. This is how a Helm-configured
Basic user's Argon2id hash is delivered. If your automation reads
`kubectl get configmap <release>`, read the Secret instead.

The classification is by **name shape**, not a fixed list, so a secret-looking
key added to the server's configuration tomorrow is caught without anyone
remembering to update this chart.

## Hardened by default

The pod runs as uid/gid 65532 with `runAsNonRoot`, an empty capability set,
`allowPrivilegeEscalation: false`, `readOnlyRootFilesystem: true`, and
`seccompProfile: RuntimeDefault`: the Pod Security Standards **Restricted**
profile. `automountServiceAccountToken` is off because the workload never calls
the Kubernetes API, and `enableServiceLinks` is off because the kubelet's
injected `FERROEHR_*` Service variables would otherwise collide with the
server's own configuration namespace and stop it booting.

A NetworkPolicy ships enabled, and it admits inbound traffic to the API (and
management) port only. **It narrows PORTS, not SOURCES, until you set
`networkPolicy.ingressFrom`**: an ingress rule with no `from` admits every
source, including other namespaces
([NetworkPolicies](https://kubernetes.io/docs/concepts/services-networking/network-policies/)),
so the shipped policy reads as default-deny while admitting everything on that
port. Set `ingressFrom` to your ingress controller, or set
`networkPolicy.ingressAllowAll=false` to have the chart refuse to render the
open state at all. And note that **none of it does anything unless your CNI
enforces NetworkPolicy**. Check yours; several do not.

## Requirements

Kubernetes: `>=1.36.0-0`

> [!NOTE]
> Rows under `config.*` carry no description here on purpose. Those keys are the
> **server's**, not the chart's (the chart renders the `config` tree verbatim
> into `ferroehr.toml`), and they are documented once, in the configuration
> reference. Restating them here would fork two copies that drift. The same
> reasoning keeps the server's vocabulary out of `values.schema.json`: it types
> only the shape of the few `config.*` keys the chart branches on or ships a
> default for.
>
> That also means the table is not the boundary of what you can set: **any** key
> in the configuration reference is reachable as `config.<the.toml.path>`,
> whether or not it appears below.

## Values

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| affinity | object | `{}` | Pod affinity/anti-affinity rules. Empty = none. |
| autoscaling.behavior | object | `{}` | Scaling behaviour, passed through verbatim to `spec.behavior`. Empty leaves the documented defaults, which are already asymmetric in the right direction for a clinical API: scale-up is immediate, scale-down waits out a 300-second stabilization window so a traffic trough cannot tear down capacity that is about to be needed. Set this only to make it MORE conservative — e.g. a `scaleDown.policies` entry capping how many pods may go per minute (https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/#configurable-scaling-behavior). |
| autoscaling.enabled | bool | `false` | Horizontal pod autoscaling. When on, the chart omits replicas and the HPA owns the count. |
| autoscaling.maxReplicas | int | `6` | Upper bound the HPA may scale to. |
| autoscaling.minReplicas | int | `2` | Lower bound the HPA may scale to. |
| autoscaling.targetCPUUtilizationPercentage | int | `75` | Target average CPU. 0 removes the metric; removing BOTH metrics is refused, since an HPA with none never scales. |
| autoscaling.targetMemoryUtilizationPercentage | int | `0` | Target average memory. 0 removes the metric. A CDR is usually CPU-bound, so this is off by default. |
| backup.activeDeadlineSeconds | int | `7200` | Hard ceiling on one dump, so a dump blocked on the database ends rather than overlapping the next schedule. |
| backup.audit.existingSecret | string | `""` | REQUIRED when the audit domain has its own DSN: Secret holding the audit BACKUP DSN — a read-only role on the audit schema in the database `database.audit` points at. The clinical backup DSN is not it: that one names the clinical database, which is not where this trail lives. |
| backup.audit.existingSecretKey | string | `"FERROEHR__STORAGE__AUDIT__URL"` | Key within `existingSecret` carrying that DSN. |
| backup.audit.persistentVolumeClaim | string | `""` | REQUIRED when the audit domain has its own DSN: an EXISTING PersistentVolumeClaim for the audit dumps, and a DIFFERENT one from every other domain's. The trail names patients and principals; it is the record of who read what, so give its volume its own audience. |
| backup.audit.schedule | string | `"45 2 * * *"` | Cron schedule for the audit dump (schema audit). Offset from the other three by default so no two dumps read a database in the same minute. Only read when `database.audit.existingSecret` is set; otherwise the trail travels with the clinical dump and no audit job renders. |
| backup.backoffLimit | int | `2` | Retries before a dump is declared failed. |
| backup.clinical.existingSecret | string | `""` | REQUIRED when enabled: Secret holding the clinical BACKUP DSN — a role read-only on the clinical schemas, and not the pool's credential: each domain's runtime role is revoked from the others, so a dump taken through it would be silently partial. |
| backup.clinical.existingSecretKey | string | `"FERROEHR__DB__URL"` | Key within `existingSecret` carrying that DSN. |
| backup.clinical.persistentVolumeClaim | string | `""` | REQUIRED when enabled: the name of an EXISTING PersistentVolumeClaim the clinical dumps are written to, mounted at /backup. The chart creates no claim — its storage class, size, retention and who may read it are yours. |
| backup.clinical.schedule | string | `"15 1 * * *"` | Cron schedule for the clinical dump (schemas clinical and ext, plus audit while the audit domain shares this database). |
| backup.enabled | bool | `false` | Render the per-domain backup CronJobs: clinical, party, linkage, and audit when `database.audit.existingSecret` gives it a database of its own. Each rendered domain then needs its own `persistentVolumeClaim` below, or the render is refused. |
| backup.image.digest | string | `""` | Image digest (`sha256:…`); wins over `tag` entirely when set. |
| backup.image.pullPolicy | string | `"IfNotPresent"` | Pull policy. |
| backup.image.repository | string | `"ghcr.io/ferrohealth/ferroehr-postgres"` | Image carrying `pg_dump`. The project's own PostgreSQL 18 image, so the dump is taken by the same major version the server runs against. |
| backup.image.tag | string | `""` | Image tag. Empty falls back to .Chart.appVersion, as the server's does. |
| backup.linkage.existingSecret | string | `""` | REQUIRED when enabled: Secret holding the linkage BACKUP DSN — a role read-only on the linkage schema, and a DIFFERENT role from the other two. This credential reads the map from a party to its EHR, the additional information that re-identifies a pseudonymised record, so it is the narrowest of the three. |
| backup.linkage.existingSecretKey | string | `"FERROEHR__STORAGE__LINKAGE__URL"` | Key within `existingSecret` carrying that DSN. The name follows its two siblings and matches the server's own `database.linkage.existingSecretKey` for the linkage POOL, but this must be a different Secret holding the backup role's DSN: the pool's credential cannot dump the table. The probe restores this dump and checks the map's temporal key and forced row policy. |
| backup.linkage.persistentVolumeClaim | string | `""` | REQUIRED when enabled: an EXISTING PersistentVolumeClaim for the linkage dumps, and a DIFFERENT one from the other two claims (two domains sharing a claim is refused at render). A volume holding this dump beside either other domain's rebuilds the join the split exists to withhold. |
| backup.linkage.schedule | string | `"15 2 * * *"` | Cron schedule for the linkage dump (schema linkage). Offset from the other two by default so no two dumps read the database in the same minute. |
| backup.nodeSelector | object | `{}` | Node selector for the dump pods. |
| backup.party.existingSecret | string | `""` | REQUIRED when enabled: Secret holding the party BACKUP DSN — a read-only role on the party schema, and a DIFFERENT role from the clinical one. This credential reads every identity the instance holds; treat it accordingly. |
| backup.party.existingSecretKey | string | `"FERROEHR__STORAGE__PARTY__URL"` | Key within `existingSecret` carrying that DSN. |
| backup.party.persistentVolumeClaim | string | `""` | REQUIRED when enabled: an EXISTING PersistentVolumeClaim for the party dumps, and a DIFFERENT one from every other domain's claim (two domains sharing a claim is refused at render). This is the volume that carries identifying data; give it the narrower audience. |
| backup.party.schedule | string | `"45 1 * * *"` | Cron schedule for the party dump (schema party). Offset from the clinical one by default so no two dumps read the database in the same minute. |
| backup.podAnnotations | object | `{}` | Extra annotations on the dump pods. |
| backup.resources | object | `{}` | Resource requests/limits for the dump pods. |
| backup.startingDeadlineSeconds | int | `3600` | Seconds a dump that missed its scheduled time may still start in; past it the run is skipped and the next schedule stands (https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/). |
| backup.timeZone | string | `""` | IANA time zone the schedules below are read in ("Europe/Amsterdam"). Empty leaves the kube-controller-manager's own zone, which is where an unqualified schedule drifts between clusters (KEP-3140, stable v1.27 — https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/). |
| backup.tolerations | list | `[]` | Tolerations for the dump pods. |
| backup.ttlSecondsAfterFinished | int | `86400` | How long a finished dump's pod is kept for its logs. |
| config.admin.enabled | bool | `false` |  |
| config.audit.categories.archetypes | object | `{}` |  |
| config.audit.categories.templates | object | `{}` |  |
| config.audit.enabled | bool | `true` |  |
| config.audit.store.enabled | bool | `true` |  |
| config.audit.store.retention_days | int | `0` |  |
| config.audit.store.retention_years | int | unset |  |
| config.auth.enabled | bool | `true` |  |
| config.authz.abac.enabled | bool | `false` |  |
| config.authz.rbac.admin_role | string | `"ADMIN"` |  |
| config.authz.rbac.enabled | bool | `true` |  |
| config.authz.rbac.user_role | string | `"USER"` |  |
| config.db.acquire_timeout_secs | int | `30` |  |
| config.db.max_connections | int | `10` |  |
| config.db.migrate | string | `"apply"` |  |
| config.db.min_connections | int | `0` |  |
| config.db.statement_timeout_ms | int | `60000` |  |
| config.deployment_profile | string | `"sandbox"` |  |
| config.events.enabled | bool | `false` |  |
| config.events.exchange | string | `"ferroehr.events"` |  |
| config.events.tls | bool | `false` |  |
| config.files | object | `{}` |  |
| config.log.format | string | `"json"` |  |
| config.management.base_path | string | `"/management"` |  |
| config.management.enabled | bool | `true` |  |
| config.management.endpoints.env | string | `"off"` |  |
| config.management.endpoints.flamegraph | string | `"off"` |  |
| config.management.endpoints.info | string | `"off"` |  |
| config.management.endpoints.loggers | string | `"off"` |  |
| config.management.endpoints.metrics | string | `"off"` |  |
| config.management.endpoints.prometheus | string | `"off"` |  |
| config.multimedia.allow_http | bool | `false` |  |
| config.multimedia.bucket | string | `"openehr-multimedia"` |  |
| config.multimedia.enabled | bool | `false` |  |
| config.multimedia.region | string | `"us-east-1"` |  |
| config.server.base_path | string | `"/ferroehr/rest/openehr/v1"` |  |
| config.server.bind | string | `"0.0.0.0:8080"` |  |
| config.server.connection.header_read_timeout_secs | int | `10` |  |
| config.server.connection.http2_keep_alive_interval_secs | int | `30` |  |
| config.server.connection.http2_keep_alive_timeout_secs | int | `10` |  |
| config.server.connection.max_concurrent_streams | int | `256` |  |
| config.server.cors_permissive | bool | `false` |  |
| config.server.limits.body_bytes | int | `16777216` |  |
| config.server.limits.bulk_body_bytes | int | `67108864` |  |
| config.server.rate_limit.address_burst | int | `4096` |  |
| config.server.rate_limit.address_per_second | int | `2048` |  |
| config.server.rate_limit.enabled | bool | `true` |  |
| config.server.rate_limit.principal_burst | int | `2048` |  |
| config.server.rate_limit.principal_per_second | int | `1024` |  |
| config.server.swagger_ui | string | `"private"` |  |
| config.signing.enabled | bool | `true` |  |
| config.signing.mode | string | `"digest"` |  |
| config.signing.verify_on_read | string | `"strict"` |  |
| config.spec_profile | string | `"development"` |  |
| config.terminology.api_enabled | bool | `false` |  |
| database.audit.existingSecret | string | `""` | Reference an existing Secret holding the DSN the audit repository is written through. The audit trail is not a pseudonymisation domain — it is the node's security-surveillance record, strictly outside the EHR content — but it is the one domain a deployment most often wants on its own server, which DSV Art. 4 Abs. 5 asks for. |
| database.audit.existingSecretKey | string | `"FERROEHR__STORAGE__AUDIT__URL"` | Key WITHIN it holding that DSN. Mounted as a file. |
| database.audit.url | string | `""` | Inline audit DSN (DEV/TEST ONLY). Ignored when `audit.existingSecret` is set. |
| database.clinical.existingSecret | string | `""` | Reference an existing Secret holding the CLINICAL-role DSN (`postgres://ferroehr_clinical:...@host:5432/ferroehr`). Empty uses `database.existingSecret` / `database.url`. |
| database.clinical.existingSecretKey | string | `"FERROEHR__STORAGE__CLINICAL__URL"` | Key WITHIN it holding that DSN. Mounted as a file; only its PATH reaches the pod's environment. |
| database.clinical.url | string | `""` | Inline clinical DSN (DEV/TEST ONLY — lands in a chart-managed Secret). Ignored when `clinical.existingSecret` is set. |
| database.existingSecret | string | `""` | Reference an existing Secret holding the app-role DSN (STRONGLY preferred for production — keeps the credential out of chart values and git). The secret's value must be a full `postgres://ferroehr_clinical:...@host:5432/ferroehr` (optionally `?sslmode=verify-full`). |
| database.existingSecretKey | string | `"FERROEHR__DB__URL"` | Key WITHIN existingSecret that holds the DSN. This is a Secret key name, not an environment variable name: the chart mounts that key as a file and passes only its PATH as `FERROEHR__DB__URL_FILE`, so the DSN never enters the pod's environment. The default spelling is kept for compatibility with existing Secrets created for the older env-borne arrangement. |
| database.linkage.existingSecret | string | `""` | Reference an existing Secret holding the LINKAGE-role DSN (`postgres://ferroehr_linkage:...@host:5432/ferroehr`), a third credential distinct from the two above. |
| database.linkage.existingSecretKey | string | `"FERROEHR__STORAGE__LINKAGE__URL"` | Key WITHIN it holding that DSN. Mounted as a file. |
| database.linkage.url | string | `""` | Inline linkage DSN (DEV/TEST ONLY). Ignored when `linkage.existingSecret` is set. |
| database.migrateExistingSecret | string | `""` | Reference an existing Secret holding the DSN that PREPARES the schema (`postgres://ferroehr_migrator:...@host:5432/ferroehr`). Required when the runtime DSNs are domain-scoped roles, including under `config.db.migrate=verify`. |
| database.migrateExistingSecretKey | string | `"FERROEHR__DB__MIGRATE_URL"` | Key WITHIN migrateExistingSecret holding that DSN. Mounted as a file; only its PATH reaches the pod's environment. |
| database.migrateUrl | string | `""` | Inline schema-preparation DSN (DEV/TEST ONLY — lands in a chart-managed Secret). Ignored when migrateExistingSecret is set. |
| database.party.existingSecret | string | `""` | Reference an existing Secret holding the PARTY-role DSN (`postgres://ferroehr_party:...@host:5432/ferroehr`), a different credential from `database.existingSecret`. |
| database.party.existingSecretKey | string | `"FERROEHR__STORAGE__PARTY__URL"` | Key WITHIN it holding that DSN. Mounted as a file. |
| database.party.url | string | `""` | Inline party DSN (DEV/TEST ONLY). Ignored when `party.existingSecret` is set. |
| database.url | string | `""` | Inline DSN (DEV/TEST ONLY — lands in a chart-managed Secret). Leave empty and use existingSecret in production. Ignored when existingSecret is set. |
| extraEnv | list | `[]` | Extra raw env vars (list of {name,value} or {name,valueFrom}). Escape hatch for anything not surfaced above (array-valued keys via comma-separated values, one-off FERROEHR_* overrides). |
| extraEnvFrom | list | `[]` | Extra envFrom sources (configMapRef/secretRef). |
| extraVolumeMounts | list | `[]` | Mounts for extraVolumes, in the server container. |
| extraVolumes | list | `[]` | Extra volumes / volumeMounts (e.g. an external secret store for the PGP key). |
| fullnameOverride | string | `""` | Override the full resource name. |
| hostUsers | bool | `false` | Run the pods in their own USER NAMESPACE, so container UIDs map to unprivileged host UIDs and a container escape lands as nobody rather than as the UID it ran under (KEP-127, stable v1.36 — the reason this chart's kubeVersion floor is 1.36). `false` here is the Kubernetes field spelling and means user namespaces are ON; set it to `true` to share the host's user namespace, which is the API default.  Set it to `true` if your nodes cannot support it. The requirement is a node-level one the chart cannot check: a Linux node whose container runtime implements it (containerd >= 2.0 or CRI-O >= 1.25) with idmap-mount support in the kernel. On a node without it the pod does not start — a loud failure, not a silent downgrade (https://kubernetes.io/docs/tasks/configure-pod-container/user-namespaces/). |
| image.digest | string | `""` | Image digest (`sha256:…`). Set it and the pod runs `repository@digest`, ignoring `tag` entirely: a digest is what the provenance attestation is made over, so deploying by digest is what makes verification bind to the running image. A tag can be moved afterwards; a digest cannot. |
| image.pullPolicy | string | `"IfNotPresent"` | Pull policy. IfNotPresent + an immutable pinned tag/digest in production. |
| image.repository | string | `"ghcr.io/ferrohealth/ferroehr"` | Image repository. Multi-arch distroless (gcr.io/distroless/cc-debian13:nonroot base). |
| image.tag | string | `""` | Image tag. Empty string falls back to .Chart.appVersion. Pin a version in production, never `latest` — and prefer `digest` below, which a tag cannot be substituted for once it is set. |
| imagePullSecrets | list | `[]` | imagePullSecrets for private registries. |
| ingress.annotations | object | `{}` | Ingress annotations (TLS issuer, body size, timeouts — controller-specific). |
| ingress.className | string | `""` | IngressClass name. Empty leaves the cluster default to apply. |
| ingress.enabled | bool | `false` | Create an Ingress. TLS termination belongs here (or at a gateway). |
| ingress.hosts[0].host | string | `"ferroehr.example.com"` |  |
| ingress.hosts[0].paths[0].path | string | `"/ferroehr"` |  |
| ingress.hosts[0].paths[0].pathType | string | `"Prefix"` |  |
| ingress.tls | list | `[]` | TLS blocks, passed through verbatim. |
| metrics.enabled | bool | `false` | Add prometheus.io scrape annotations to the pods. |
| metrics.grafanaDashboard.enabled | bool | `false` | Ship the default "FerroEHR — service overview" Grafana dashboard as a ConfigMap labelled `grafana_dashboard: "1"`, the label the Grafana Helm chart's dashboard sidecar (and therefore kube-prometheus-stack) discovers and auto-imports. The sidecar searches its own release namespace unless its `sidecar.dashboards.searchNamespace` says otherwise. |
| metrics.grafanaDashboard.folder | string | `"FerroEHR"` | Value for the `grafana_folder` annotation. Applied only when the sidecar's `folderAnnotation` feature is configured; harmless otherwise. |
| metrics.serviceMonitor.enabled | bool | `false` | Render a Prometheus Operator ServiceMonitor. Needs the monitoring.coreos.com CRDs installed, or the install fails on an unknown kind. |
| metrics.serviceMonitor.interval | string | `"30s"` | Scrape interval / timeout. |
| metrics.serviceMonitor.labels | object | `{}` | Extra labels, for the `serviceMonitorSelector` your Prometheus matches on. |
| metrics.serviceMonitor.namespace | string | `""` | Namespace for the ServiceMonitor; empty = the release namespace. |
| metrics.serviceMonitor.scrapeTimeout | string | `"10s"` | Per-scrape timeout. Must be shorter than the interval. |
| migrations.job.activeDeadlineSeconds | int | `600` | Hard ceiling on the migration step; a migration blocked behind live traffic must fail the release rather than hang it. |
| migrations.job.backoffLimit | int | `3` | Retries before the Job (and therefore the release) is declared failed. |
| migrations.job.enabled | bool | `false` | Run the migrations as a pre-install/pre-upgrade hook Job under the migrator DSN. Pair it with config.db.migrate=verify and an app-role-only runtime DSN for the least-privilege posture. |
| migrations.job.existingSecret | string | `""` | REQUIRED when enabled: an existing Secret holding the MIGRATOR DSN (`postgres://ferroehr_migrator:...@host:5432/ferroehr`). Deliberately a different credential from `database.*`, which carries the runtime app-role DSN — rendering fails if it is empty. |
| migrations.job.existingSecretKey | string | `"FERROEHR__DB__URL"` | Key WITHIN existingSecret holding the migrator DSN. Mounted as a file; only its PATH reaches the pod's environment. |
| migrations.job.nodeSelector | object | `{}` | Node selector for the migration pod. |
| migrations.job.podAnnotations | object | `{}` | Extra annotations on the migration pod. |
| migrations.job.resources | object | `{}` | Resource requests/limits for the migration pod. |
| migrations.job.tolerations | list | `[]` | Tolerations for the migration pod. |
| migrations.job.ttlSecondsAfterFinished | int | `600` | How long the finished Job's pod is kept for its logs. |
| migrations.runByMigratorRole | bool | `true` | Purely informational marker rendered into NOTES for the operator. |
| nameOverride | string | `""` | Override the chart name portion of resource names. |
| networkPolicy.egress.database | object | `{"port":5432,"to":[]}` | The database, which is NOT optional: the server cannot pass readiness without it. Rendering an egress policy with no database destination is a refusal, not a warning (see the template) — an egress policy that forgets the DSN is a total outage that looks like a database failure. `to` takes raw NetworkPolicyPeer entries: a `podSelector`/ `namespaceSelector` for an in-cluster database, or an `ipBlock` for a managed one. `port` is the DSN's port. |
| networkPolicy.egress.enabled | bool | `false` | Refuse all outbound traffic except DNS, `database` and `rules`. |
| networkPolicy.egress.rules | list | `[]` | Every other destination, as raw NetworkPolicyEgressRule entries — one per integration you have switched on. See the book's table. |
| networkPolicy.enabled | bool | `true` | Install a NetworkPolicy that admits inbound traffic to the API (and management) port and nothing else. Strongly recommended for a PHI workload — but it narrows PORTS unconditionally and SOURCES only when `ingressFrom` is set, so "networkPolicy.enabled=true" alone is not a source restriction. |
| networkPolicy.ingressAllowAll | bool | `true` | Admit every source while `ingressFrom` is empty — which is the posture this chart SHIPS, stated as a value rather than left implicit in an empty list. Set it to `false` to make "no open ingress" a machine-checked fact: with `ingressFrom` still empty the render is then REFUSED instead of quietly producing a policy that admits everything while reading as default-deny. It only decides the empty case — a non-empty `ingressFrom` always narrows. |
| networkPolicy.ingressFrom | list | `[]` | Ingress `from` selectors admitted to the API port. Empty means the rule carries no `from` at all, and a NetworkPolicy ingress rule without `from` admits EVERY source — other namespaces and off-cluster clients included, not just this namespace (https://kubernetes.io/docs/concepts/services-networking/network-policies/). Only the port list is narrowed in that state, so SET this to your ingress-controller namespace/pods for a PHI workload. |
| nodeSelector | object | `{}` | Scheduling. |
| podAnnotations | object | `{}` | Extra annotations on the pod template. Note a change here rolls the Deployment. |
| podDisruptionBudget.enabled | bool | `true` | Protect availability during voluntary disruption (drains, upgrades). |
| podDisruptionBudget.minAvailable | int | `1` | Pods that must stay available. Used only when maxUnavailable is unset. |
| podDisruptionBudget.unhealthyPodEvictionPolicy | string | `"AlwaysAllow"` | Whether a node drain may evict pods that are already unhealthy. `AlwaysAllow` is the documented recommendation; the alternative, `IfHealthyBudget`, is the API default and makes a drain wait for pods to become healthy first — which never completes when they are unhealthy because of the drain itself. |
| podLabels | object | `{}` | Extra pod labels / annotations. |
| podSecurityContext.fsGroup | int | `65532` | Supplemental group owning mounted volumes, so a non-root process can read them. |
| podSecurityContext.fsGroupChangePolicy | string | `"OnRootMismatch"` | Only chown volumes whose ownership differs, avoiding a full relabel on every mount. |
| podSecurityContext.runAsGroup | int | `65532` | GID to run as. |
| podSecurityContext.runAsNonRoot | bool | `true` | Refuse to start as root (pod level). Required by the restricted profile. |
| podSecurityContext.runAsUser | int | `65532` | UID to run as (pod level). 65532 is the distroless nonroot user. |
| podSecurityContext.seccompProfile.type | string | `"RuntimeDefault"` | Seccomp profile. RuntimeDefault is required by the restricted profile. |
| podSecurityContext.supplementalGroupsPolicy | string | `"Strict"` | Whether the groups in the IMAGE's /etc/group are merged into the process's supplemental groups. `Strict` uses only the groups this manifest names, so a group baked into an image cannot silently widen file access (KEP-3619, stable v1.35; https://kubernetes.io/docs/tasks/configure-pod-container/security-context/). |
| preStopSleepSeconds | int | `5` | Lame-duck pause before SIGTERM, in seconds (0 disables). Endpoint removal and SIGTERM happen concurrently on pod deletion, so without a pause a terminating pod can still be sent requests. Uses the native `preStop.sleep` action (this image has no shell, so an exec hook cannot run). Must stay comfortably under terminationGracePeriodSeconds: the sleep runs INSIDE that budget and the server still needs its own drain window after it. |
| probes.liveness.failureThreshold | int | `3` | Liveness probe: consecutive failures before the pod is acted on. |
| probes.liveness.initialDelaySeconds | int | `10` | Liveness probe: delay before the first check. |
| probes.liveness.periodSeconds | int | `15` | Liveness probe: seconds between checks. |
| probes.liveness.timeoutSeconds | int | `3` | Liveness probe: per-check timeout. |
| probes.readiness.failureThreshold | int | `3` | Readiness probe: consecutive failures before the pod is acted on. |
| probes.readiness.initialDelaySeconds | int | `5` | Readiness probe: delay before the first check. |
| probes.readiness.periodSeconds | int | `10` | Readiness probe: seconds between checks. |
| probes.readiness.timeoutSeconds | int | `3` | Readiness probe: per-check timeout. |
| probes.startup.enabled | bool | `true` | Startup probe. Gives a slow first boot (migrations) room before liveness applies. |
| probes.startup.failureThreshold | int | `30` | Startup probe: consecutive failures before the pod is acted on. |
| probes.startup.initialDelaySeconds | int | `5` | Startup probe: delay before the first check. |
| probes.startup.periodSeconds | int | `5` | Startup probe: seconds between checks. |
| probes.startup.timeoutSeconds | int | `3` | Startup probe: per-check timeout. |
| replicaCount | int | `2` | Number of replicas (ignored when autoscaling.enabled is true). |
| resources | object | `{"limits":{"cpu":"2","ephemeral-storage":"1Gi","memory":"1Gi"},"requests":{"cpu":"250m","ephemeral-storage":"128Mi","memory":"256Mi"}}` | Resource requests/limits. Sized for a modest API replica; tune for load. ephemeral-storage is bounded too: the container writes only logs/tmp (the root filesystem is read-only), and an unbounded default would let runaway local writes evict the pod's node neighbours (https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/#setting-requests-and-limits-for-local-ephemeral-storage). |
| secrets.auditFhirFeedUrl | string | `""` | FHIR base URL of the external Audit Record Repository for [audit.fhir_feed] (may carry basic-auth credentials in its userinfo) → FERROEHR__AUDIT__FHIR_FEED__URL env: audit.fhir_feed.url has no `*_file` sibling either. |
| secrets.authOidcHmacSecret | string | `""` | Symmetric HS256 secret for [auth.oidc] (dev/test). MOUNTED as /etc/ferroehr-secrets/auth.oidc.hmac_secret (auth.oidc.hmac_secret_file). |
| secrets.basicUserPasswordHashes | object | `{}` | Argon2id password hashes for [[auth.basic.users]], keyed by username. Each is MOUNTED as /etc/ferroehr-secrets/auth.basic.users.<username>.password_hash and the chart injects the matching `password_hash_file` into the rendered TOML. Declare the user itself — `username`, `roles` — under config.auth.basic.users; a username with no matching entry is a render error. A hash under `config:` is refused: it would reach the ConfigMap. |
| secrets.eventsUrl | string | `""` | AMQP broker URL for [events] (carries credentials). MOUNTED as /etc/ferroehr-secrets/events.url (events.url_file). |
| secrets.multimediaAccessKeyId | string | `""` | S3 access key id for [multimedia] → FERROEHR__MULTIMEDIA__ACCESS_KEY_ID env. Not a secret in the server's own model (it is reported unredacted by /management/env); prefer IRSA/Workload-Identity (leave empty) on cloud. |
| secrets.multimediaSecretAccessKey | string | `""` | S3 secret access key for [multimedia]. MOUNTED as /etc/ferroehr-secrets/multimedia.secret_access_key (multimedia.secret_access_key_file). |
| secrets.signingKeyPassphrase | string | `""` | PGP key passphrase (config.signing.mode=pgp). MOUNTED as /etc/ferroehr-secrets/signing.key_passphrase (signing.key_passphrase_file). |
| secrets.terminologyOauth2ClientSecrets | object | `{}` | OAuth2 client secrets for [terminology.external.oauth2_clients.<name>], keyed by client name. Each is MOUNTED as /etc/ferroehr-secrets/terminology.external.oauth2_clients.<name>.client_secret and the chart injects the matching `client_secret_file` into the rendered TOML (a path is not sensitive). Declare the client itself — token_url, client_id, scopes — under config.terminology.external.oauth2_clients.<name>; a name with no such declaration is a render error. |
| securityContext.allowPrivilegeEscalation | bool | `false` | Block setuid/file-capability escalation. Required by the restricted profile. |
| securityContext.capabilities.drop[0] | string | `"ALL"` |  |
| securityContext.privileged | bool | `false` | Never true. A privileged container is effectively root on the node. |
| securityContext.readOnlyRootFilesystem | bool | `true` | Immutable root filesystem; writable paths are explicit emptyDir mounts. |
| securityContext.runAsNonRoot | bool | `true` | Refuse to start as root (container level). Required by the restricted profile. |
| securityContext.runAsUser | int | `65532` | UID to run as (container level). 65532 is the distroless nonroot user. |
| securityContext.seccompProfile.type | string | `"RuntimeDefault"` | Seccomp profile at container level. RuntimeDefault is required by the restricted profile. |
| service.annotations | object | `{}` | Extra annotations. |
| service.port | int | `8080` | Public API port. |
| service.trafficDistribution | string | `""` | Topology preference for how the Service picks an endpoint (KEP-4444, stable v1.33). Empty leaves the API default: spread across every ready endpoint cluster-wide. `PreferSameZone` (`PreferClose` is its older spelling) keeps traffic inside the client's zone while endpoints there are ready, which cuts cross-zone latency and inter-zone egress billing; `PreferSameNode` is stricter still. Both trade EVENNESS for locality — a zone with one replica and most of the callers will take most of the load — so it is off unless you have measured that you want it (https://kubernetes.io/docs/reference/networking/virtual-ips/#traffic-distribution). |
| service.type | string | `"ClusterIP"` | Service type. ClusterIP + an Ingress/gateway in front is the norm. |
| serviceAccount.annotations | object | `{}` | Extra annotations (e.g. IRSA/Workload-Identity role bindings for S3). |
| serviceAccount.automountServiceAccountToken | bool | `false` | The workload never calls the K8s API, so no token is mounted. |
| serviceAccount.create | bool | `true` | Create a dedicated ServiceAccount. |
| serviceAccount.name | string | `""` | Name to use; generated when empty. |
| strategy.rollingUpdate.maxSurge | int | `1` | Extra pods allowed above `replicaCount` while rolling. |
| strategy.rollingUpdate.maxUnavailable | int | `0` | Pods allowed to be unavailable while rolling. 0 = capacity never drops. |
| strategy.type | string | `"RollingUpdate"` | Rollout strategy. RollingUpdate with maxUnavailable 0 never drops capacity; Recreate takes the service down. |
| terminationGracePeriodSeconds | int | `30` | Termination grace period (audit/outbox drain has a 5s window in-binary). |
| terminology.affinity | object | `{}` | Affinity. |
| terminology.codeSystems.existingConfigMap | string | `""` | Serve code systems from an existing ConfigMap of FHIR JSON instead of the chart's shaped seed. Empty means the chart renders its own. The pod's rollout annotation hashes the NAME you give here, not the ConfigMap's contents — the chart cannot read an object it does not own — so editing that ConfigMap propagates into the volume while every running pod keeps serving what it read at boot. Restart the workload yourself after such an edit (`kubectl rollout restart deployment/<release>-terminology`). |
| terminology.codeSystems.mountPath | string | `"/etc/ferroterm/codesystems"` | Directory the code systems are mounted at, read-only; it is what `FERROTERM_CODESYSTEMS` names. |
| terminology.defaultLanguage | string | `"en"` | Display language FerroTERM prefers when a concept carries several. |
| terminology.enabled | bool | `false` | Deploy FerroTERM alongside the CDR. |
| terminology.extraEnv | list | `[]` | Extra environment for FerroTERM (escape hatch). |
| terminology.failOnError | bool | `false` | What the CDR does when the terminology server cannot answer. `false` (the server's own shipped default) accepts the binding; `true` turns an unreachable server into a 422 refusal. No released openEHR text decides between the two. |
| terminology.image.digest | string | `"sha256:7ec4c9622e88e3028faf7c74a2384fb02e57dc3163c282b21274121a26aa3d5b"` | Image digest (`sha256:…`); wins over `tag`. Pinned by default, so an install runs the exact image this chart was validated against. |
| terminology.image.pullPolicy | string | `"IfNotPresent"` | Pull policy. |
| terminology.image.repository | string | `"ghcr.io/ferrohealth/ferroterm"` | FerroTERM image repository. A separate product on its own release line (https://github.com/FerroHEALTH/FerroTERM), BUSL-1.1 from the same Licensor as FerroEHR. |
| terminology.image.tag | string | `""` | Image tag. Empty falls back to the FerroTERM release this chart is pinned to (0.1.6) — never `.Chart.appVersion`, which is FerroEHR's line. A tag other than that pin is REFUSED while `digest` below is non-empty: the digest wins, so the tag would be inert while the values file claimed otherwise. Clear `digest` to deploy a tag. |
| terminology.index.mountPath | string | `"/data/index"` | Directory the index claim is mounted at, read-only. Rendered as `FERROTERM_INDEX` only when a claim is named. |
| terminology.index.persistentVolumeClaim | string | `""` | Name of an EXISTING PersistentVolumeClaim holding a built index for a release you hold a licence for. Empty means the shaped seed is served alone. The chart provisions no storage and renders no build Job: build the index off-cluster with `ferroterm-build --rf2 <release.zip> --out <dir>` and fill the claim from that output. |
| terminology.logFormat | string | `"json"` | Log format (`json` or `pretty`). |
| terminology.logLevel | string | `"info"` | `RUST_LOG` filter for the terminology container. |
| terminology.networkPolicy.enabled | bool | `true` | Install a NetworkPolicy for FerroTERM. Unlike the CDR's and the viewer's, its ingress is narrowed by DEFAULT and there is no admit-everything state: the caller is known — the CDR's pods — so no value offers one. Egress admits DNS only. |
| terminology.networkPolicy.extraIngressFrom | list | `[]` | Further peers admitted to the terminology port, beside the CDR's pods. Raw NetworkPolicyPeer entries, for a caller you deliberately add (a FerroCKM instance, a migration job); read the licence note above first. No boolean opens this port the way `networkPolicy.ingressAllowAll` opens the CDR's — but a peer you add is admitted exactly as written, and an empty `namespaceSelector: {}` means every namespace. The values schema refuses an empty peer and an empty selector inside one for that reason. |
| terminology.nodeSelector | object | `{}` | Node selector. |
| terminology.podDisruptionBudget | object | `{"enabled":false,"minAvailable":1,"unhealthyPodEvictionPolicy":"AlwaysAllow"}` | Protect FerroTERM during voluntary disruption (drains, upgrades). Off by default: the shipped `replicaCount` is 1, and a budget over a single pod blocks the drain it exists to survive. Turn it on with more than one replica. Same shape and same defaults as the CDR's `podDisruptionBudget`. |
| terminology.podSecurityContext | object | `{"fsGroup":65532,"fsGroupChangePolicy":"OnRootMismatch","runAsGroup":65532,"runAsNonRoot":true,"runAsUser":65532,"seccompProfile":{"type":"RuntimeDefault"},"supplementalGroupsPolicy":"Strict"}` | Pod-level security context. Mirrors the server's; a second workload is where a hardened posture is most easily lost. The user-namespace setting is NOT mirrored here — it is the release-wide `hostUsers` key. |
| terminology.preStopSleepSeconds | int | `5` | Lame-duck pause before SIGTERM, in seconds (0 disables) — the same endpoint-propagation race the server's `preStopSleepSeconds` covers. |
| terminology.replicaCount | int | `1` | Replica count. FerroTERM serves a read-only index, so replicas are a throughput choice; each one loads its own copy, so mind the memory limit. Above 1, turn `podDisruptionBudget` below on: the pods spread across nodes by default, and nothing else keeps a drain from taking all of them. |
| terminology.resources | object | `{"limits":{"memory":"1536Mi"},"requests":{"cpu":"100m","memory":"256Mi"}}` | Resource requests/limits. The 1536Mi limit is the compose overlay's, and it is sized from FerroTERM's own figures: the SNOMED CT Netherlands edition is 889 MB resident, the International edition 702 MB, LOINC 170 MB; the shaped seed is a few megabytes. An index larger than the limit is an OOMKill during startup, so raise it before mounting a bigger edition. |
| terminology.securityContext | object | `{"allowPrivilegeEscalation":false,"capabilities":{"drop":["ALL"]},"privileged":false,"readOnlyRootFilesystem":true,"runAsNonRoot":true,"runAsUser":65532,"seccompProfile":{"type":"RuntimeDefault"}}` | Container-level security context. Mirrors the server's. |
| terminology.service.port | int | `8080` | Service port. The container always listens on 8080. |
| terminology.service.type | string | `"ClusterIP"` | Service type. ClusterIP deliberately: the CDR is the only caller. |
| terminology.strategy | object | `{}` | Deployment update strategy. Empty leaves the API default (`RollingUpdate`), EXCEPT when `index.persistentVolumeClaim` names a claim: a rolling update surges a second pod before the first goes away, and a ReadWriteOnce volume cannot attach to it on another node, so the chart renders `type: Recreate` there. Set `{type: RollingUpdate}` if your index volume is ReadOnlyMany. |
| terminology.terminationGracePeriodSeconds | int | `20` | Termination grace period. FerroTERM holds no write in flight; there is nothing to drain but in-flight lookups. |
| terminology.tolerations | list | `[]` | Tolerations. |
| terminology.topologySpreadConstraints | list | `[]` | Topology spread for FerroTERM's pods. Empty uses the chart's default — one replica per node, `ScheduleAnyway`, scoped to the ReplicaSet being rolled — which is the CDR's policy over this workload's own pods. Set it to replace that wholesale; a merge of two spreading policies is not one. |
| terminology.ui | bool | `false` | Serve FerroTERM's own browsing UI. Off: the workload has no Ingress and the CDR does not use it. |
| terminology.wireCdr | bool | `true` | Point the CDR's `config.terminology.external` at the rendered Service. The chart injects `enabled`, `fail_on_error` and the `default` provider, so the Service address and the workload answering it are one decision. Set it to false to run FerroTERM beside a CDR you wire yourself. While it is true the render refuses every values file that contradicts the injection: `config.terminology.external.providers.default`, an `external.enabled: false`, an `external.fail_on_error` disagreeing with `failOnError` below, and a provider of your own with no `external.routes` entry naming it (an unrouted terminology falls back to `default`, which the injection has taken). |
| tolerations | list | `[]` | Tolerations for tainted nodes. Empty = none. |
| topologySpreadConstraints | list | `[]` | Spread replicas across nodes. Empty does NOT mean "no spreading": it means the chart's own default constraint applies — one soft `maxSkew: 1` over `kubernetes.io/hostname`, so two replicas prefer two nodes and a node failure does not take the whole CDR with it. It is `ScheduleAnyway`, not `DoNotSchedule`, so a single-node or capacity-constrained cluster still schedules rather than leaving a pod Pending forever.  A non-empty list REPLACES that default entirely — give the full constraint, including its own `labelSelector`. Add a `topology.kubernetes.io/zone` constraint here if your cluster spans zones; the chart does not assume one (https://kubernetes.io/docs/concepts/scheduling-eviction/topology-spread-constraints/). |
| usageReport.enabled | bool | `true` | Send the usage report (on by default). false sends nothing. |
| viewer.affinity | object | `{}` | Affinity. |
| viewer.auth.oidc.clientId | string | `""` | OAuth2 client id. |
| viewer.auth.oidc.enabled | bool | `false` | Authenticate viewer users through OIDC. Off means the viewer's own session auth is whatever its defaults provide — acceptable for a private cluster, not for anything reachable by a person who should not see PHI. |
| viewer.auth.oidc.issuer | string | `""` | Issuer URL (must be https for anything but local development). |
| viewer.auth.oidc.publicBaseUrl | string | `""` | The viewer's own externally-reachable base URL, used to build the OIDC redirect. Must match what the identity provider has registered. |
| viewer.enabled | bool | `false` | Deploy the viewer alongside the CDR. |
| viewer.existingSecret | string | `""` | Name of an existing Secret holding the viewer's OIDC client secret. The chart mounts that key as a FILE and passes only its path, so the credential never enters the pod's environment — the same discipline the server's DSN uses. |
| viewer.existingSecretKey | string | `"FERROEHR_VIEWER__AUTH__OIDC__CLIENT_SECRET"` | Key within `existingSecret` carrying the client secret. |
| viewer.extraEnv | list | `[]` | Extra environment for the viewer (escape hatch). |
| viewer.image.digest | string | `""` | Image digest (`sha256:…`); wins over `tag` when set, exactly as the server's `image.digest` does. |
| viewer.image.pullPolicy | string | `"IfNotPresent"` | Pull policy. |
| viewer.image.repository | string | `"ghcr.io/ferrohealth/ferroehr-viewer"` | Viewer image repository. |
| viewer.image.tag | string | `""` | Image tag. Empty falls back to .Chart.appVersion, so the viewer and the server move together by default. |
| viewer.ingress.annotations | object | `{}` | Extra annotations for the viewer Ingress. |
| viewer.ingress.className | string | `""` | IngressClass name. |
| viewer.ingress.enabled | bool | `false` | Publish the viewer through an Ingress. The viewer is a human-facing web UI, so unlike the API this is the normal way to reach it. |
| viewer.ingress.hosts | list | `[]` | Hosts and paths. |
| viewer.ingress.tls | list | `[]` | TLS blocks. |
| viewer.networkPolicy.enabled | bool | `true` | Install a NetworkPolicy for the viewer. Egress admits the CDR Service and DNS, and nothing else — the viewer is a REST client of the CDR by mandate, so that half is enforceable rather than aspirational. Ingress narrows the viewer's PORT unconditionally and its SOURCES only when `ingressFrom` is set. |
| viewer.networkPolicy.ingressAllowAll | bool | `true` | Admit every source while `ingressFrom` is empty — the posture this chart SHIPS for the viewer, stated as a value rather than left implicit in an empty list. Set it to `false` to have the render REFUSED while `ingressFrom` is empty, instead of quietly exposing the viewer to everything. It only decides the empty case — a non-empty `ingressFrom` always narrows. |
| viewer.networkPolicy.ingressFrom | list | `[]` | Ingress `from` selectors admitted to the viewer port. Empty means the rule carries no `from`, which admits EVERY source — other namespaces and off-cluster clients included (https://kubernetes.io/docs/concepts/services-networking/network-policies/). This is the human-facing login surface, so SET this to your ingress-controller namespace/pods. |
| viewer.nodeSelector | object | `{}` | Node selector. |
| viewer.podSecurityContext | object | `{"fsGroup":65532,"fsGroupChangePolicy":"OnRootMismatch","runAsGroup":65532,"runAsNonRoot":true,"runAsUser":65532,"seccompProfile":{"type":"RuntimeDefault"},"supplementalGroupsPolicy":"Strict"}` | Pod-level security context. Mirrors the server's; a second workload is where a hardened posture is most easily lost. The user-namespace setting is NOT mirrored here — it is the release-wide `hostUsers` key, because a posture that differs between two workloads of one release is a posture nobody can state. |
| viewer.preStopSleepSeconds | int | `5` | Lame-duck pause before SIGTERM, in seconds (0 disables) — the same endpoint-propagation race the server's `preStopSleepSeconds` covers. |
| viewer.replicaCount | int | `1` | Replica count. The viewer holds session state in process, so more than one replica needs sticky sessions at the ingress or users get logged out on a reroute; left at 1 deliberately. |
| viewer.resources | object | `{"limits":{"memory":"512Mi"},"requests":{"cpu":"50m","memory":"128Mi"}}` | Resource requests/limits. |
| viewer.securityContext | object | `{"allowPrivilegeEscalation":false,"capabilities":{"drop":["ALL"]},"privileged":false,"readOnlyRootFilesystem":true,"runAsNonRoot":true,"runAsUser":65532,"seccompProfile":{"type":"RuntimeDefault"}}` | Container-level security context. Mirrors the server's. |
| viewer.service.port | int | `3000` | Service port. The container always listens on 3000. |
| viewer.service.type | string | `"ClusterIP"` | Service type for the viewer. |
| viewer.terminationGracePeriodSeconds | int | `20` | Termination grace period for the viewer. Shorter than the server's: it holds no write in flight and nothing to drain but in-flight page renders. |
| viewer.tolerations | list | `[]` | Tolerations. |

## More

Full deployment documentation, including the least-privilege database roles and
the observability wiring, is at
<https://ferroehr.eu/docs/latest/installation/kubernetes.html>.

----------------------------------------------
Autogenerated from chart metadata using [helm-docs v1.14.2](https://github.com/norwoodj/helm-docs/releases/v1.14.2)
