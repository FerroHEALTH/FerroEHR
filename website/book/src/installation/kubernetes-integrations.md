# Kubernetes: integrations

Turning on the optional integrations through the chart. Installing the chart is on
[Kubernetes & Helm](kubernetes.md).

<!-- toc -->

## Integrations

### Any server setting is reachable

The chart renders its `config` tree **verbatim** into `ferroehr.toml`. So
`config.<the.toml.path>` sets *any* key in the
[configuration reference](configuration.md), including keys this page and
`values.yaml` never mention. There is no allow-list to extend and no chart
release to wait for:

```yaml
# values.yaml — [record.query] plan_cache_capacity, which values.yaml never names
config:
  query:
    plan_cache_capacity: 512
```

```shell
helm upgrade ferroehr oci://ghcr.io/ferrohealth/charts/ferroehr \
  --version 43.0.2 -n ferroehr --reuse-values \
  --set config.record.query.plan_cache_capacity=512
```

Turning something off is the same edit in reverse: remove the key (or set the
integration's `enabled` back to `false`) and upgrade. The tables below are a
curated starting point for the switches most deployments want; they are not
the boundary of what the chart supports.

Two things make this safe to rely on:

- **A typo is a boot refusal, not a silent default.** The server sweeps its
  configuration strictly and rejects an unknown key with a did-you-mean, so a
  misspelled path fails loudly at startup instead of quietly doing nothing.
  Check before you deploy with the schema and boot gates described under
  [Check your values before you deploy them](../upgrades.md#check-your-values-before-you-deploy-them).
- **Credentials do not belong in this tree.** `config` becomes a mounted
  file; secrets have their own routes (`secrets.*`, `existingSecret`, and the
  `*_file` key siblings); see
  [Secrets and mounted config](kubernetes.md#secrets-and-mounted-config).

### The common switches

Every switch below lives in the chart's `config` tree, so its key *is* the
TOML key from the [configuration reference](configuration.md). Most are **off
by default**; the ones that ship on are marked, and enabling any of the others
is an explicit, auditable decision:

| Integration | Chart key | Default | Notes |
|---|---|---|---|
| Specification generation | `config.deployment.spec_profile` | `development` | One coupled choice; `stable` runs the released generations. |
| ADMIN API | `config.api.admin` | off | Physical, irreversible delete. Gate behind admin RBAC. |
| Terminology extension API | `config.api.terminology` | off | 404 when off. |
| Outbox feed | `config.authz.grants.allowed` admitting `read_change_feed` | none | The commit-event feed for the readers a `read_change_feed` grant in the CDR names; FerroBRIDGE delivers it to a broker. See [Change events](../beyond-core/outbox-feed.md). |
| Bearer tokens | `config.auth.issuer` | unset | The issuer tables by name, each with its `issuer` and, for an issuer whose users sign in, a `party` table; each issuer's metadata and key set are read in the background from the start, and the pod is not ready until they are; asymmetric algorithms only. |
| Party binding | `config.authz.party_binding.provider` | unset | Required once authentication admits a credential: the healthcare provider the deployment acts for. Each Basic user maps to its binding key under `config.authz.party_binding.basic_user`. |
| RBAC | `config.authz.rbac.enabled` | **on** | The coarse role gate (active while `config.auth.enabled`). |
| S3 multimedia | `config.record.multimedia.enabled` | off | ⚠ Offloaded blobs are PHI. Private, encrypted, HTTPS bucket; keys via `secrets.multimediaAccessKeyId` and `secrets.multimediaSecretAccessKey`. |
| Terminology server | `config.terminology` | off | The one FHIR terminology server, at `config.terminology.endpoint.url`. To run FerroTERM in the cluster instead, use `terminology.enabled` ([FerroTERM](#ferroterm-a-terminology-server-beside-the-cdr-off-by-default)), which sets these keys for you. |
| ATNA audit trail | `config.audit.enabled` | **on** | On with the local store only; forwarding (`config.audit.syslog`, `config.audit.fhir_feed`) is opt-in per sink. |
| Version signing | `config.record.signing.enabled` | **on** (`config.record.signing.mode: digest`) | `pgp` mode needs a `config.files` key plus `secrets.signingKeyPassphrase`, and fails closed at boot without a usable key. |
| OTLP telemetry | `config.telemetry.otlp.endpoint` | unset | Setting the endpoint is all it takes; without a `config.telemetry.otlp` table nothing is exported. With `networkPolicy.egress.enabled`, add a rule for the collector, since a blocked exporter drops spans without an error. |

Full detail on each is in [Beyond the core](../beyond-core/index.md),
[Security](../security.md), and [Operations](../operations.md).

The [usage report](../usage-report.md) is on by default, and
`usageReport.enabled=false` switches it off for FerroEHR, for FerroTERM and for
FerroBRIDGE. Each of those takes its own `terminology.usageReport.enabled` or
`bridge.usageReport.enabled` when you want it to differ. FerroPIX runs from its
own chart, which cannot read this chart's switch: set
`ferropix.usageReport.enabled=false` as well, or the chart refuses to render.

### FerroEHR Viewer (a second workload, off by default)

`viewer.enabled` renders a second Deployment and Service for the viewer
beside the CDR, from its own image
(`viewer.image.repository`, tagged `appVersion` by default so the two move
together). It is off by default: the viewer is a separate product surface with
its own attack surface, and a CDR is complete without it.

Three properties are worth knowing before you switch it on:

- **It reaches the CDR strictly over the REST API**, which
  `viewer.networkPolicy.enabled` (on by default) *enforces* rather than
  assumes: the viewer's egress admits the CDR Service, DNS and outbound HTTPS
  for an identity provider, and nothing else. It holds no database credential.
- **It is a human-facing web UI**, so `viewer.ingress.enabled` is the normal way
  to reach it, and `viewer.auth.oidc.enabled` with an issuer, client id and
  `viewer.auth.oidc.publicBaseUrl` is how you keep a person who should not see
  PHI out of it. Its client secret comes from `viewer.existingSecret`, mounted
  as a file exactly as the server's DSN is.
- **`viewer.replicaCount` is 1 deliberately.** The viewer holds session state
  in process, so a second replica needs sticky sessions at the ingress or users
  are logged out on a reroute.

The viewer carries the same security context as the server, and the chart's
render gate holds it to the same Restricted profile and the same pod-isolation
settings: a release whose two workloads disagreed about `hostUsers` would be a
posture nobody could state in one sentence. The deployment probe harness then
reads that back off the *running* viewer container, exactly as it does for the
server: its uid, its empty capability bounding set, its read-only root and its
seccomp filter come from the container runtime's own spec, its admission from an
`enforce=restricted` namespace, and its login page from an HTTP request made
inside the cluster through the viewer Service, so the second workload is never
vouched for by the first. What that run does **not** establish is stated in its
own record: the viewer's OIDC path, the screens behind a session, and whether a
CNI enforces the viewer's NetworkPolicy. Its screens are documented in the
[viewer](../viewer/index.md) chapter.

### FerroPIX (the patient identity source, on by default)

The CDR keeps no patient-to-record cross-reference of its own by default: it
asks an IHE patient identity source over PIXm (ITI-83) which pseudonym the
clinical domain holds for a patient
([`[pixm]`](config-integrations.md#pixm)). The chart takes
[FerroPIX](https://github.com/FerroHEALTH/FerroPIX) as the chart dependency
`ferropix` (`oci://ghcr.io/ferrohealth/charts`, pinned in `Chart.lock`), so
its Deployment, Service, NetworkPolicy and configuration are its own chart's,
and every key under `ferropix:` is that chart's value. With
`ferropix.enabled` (the default) the chart writes
`config.record.subject_index = "pixm"` and `config.pixm.endpoint.url` at the
FerroPIX Service, and refuses a values file that sets either too. With the
egress policy on, it adds the rule to FerroPIX's pods.

You supply FerroPIX's PostgreSQL URL and sealing key, its issuer, the
pseudonymisation domain whose consumer is the CDR's client, and the CDR's side
of the same facts:

```yaml
ferropix:
  database: {existingSecret: ferropix-db}
  dataKey: {existingSecret: ferropix-data-key}
  config:
    public_base_url: https://pix.example.org
    auth:
      issuer:
        pix: {issuer: https://idp.example.org/realms/pix, audience: [https://pix.example.org/fhir]}
    trust:
      issuer: [{issuer: pix, clients: [hospital-source]}]
    pseudonym_domain:
      - {namespace: urn:oid:2.999.7.1, name: Clinical domain, consumer_client_id: ferroehr}
config:
  privacy:
    subject_namespaces: [urn:oid:2.999.7.1]
  pixm:
    assigning_authority: urn:oid:2.999.7.1
    endpoint:
      credentials:
        oauth:
          token_endpoint: https://idp.example.org/realms/pix/protocol/openid-connect/token
          client_id: ferroehr
          scope: system/Patient.rs
secrets:
  ehrIndexClientSecret: <the client secret>
```

`secrets.ehrIndexClientSecret` is mounted at
`/etc/ferroehr-secrets/ehr_index.oauth.client_secret`, and the chart injects
the matching `client_secret_file`; the production overlay reads it from a
Secret you own through `extraEnv` instead. Another PIX Manager:
`ferropix.enabled: false` and `config.pixm.endpoint.url`. A CDR run on its
own: `ferropix.enabled: false` and `config.record.subject_index: local`.

The chart pins FerroPIX 0.0.44 by digest (`ferropix.image`) with the
FerroPIX chart built for it (0.5.6). Since FerroPIX 0.0.40 an
`[auth.issuer.<name>]` entry names its `issuer` and its own `audience`, and
FerroPIX reads the caller from the standard claims of an RFC
9068 `at+jwt` access token, and a client credentials token, whose `sub` is
its `client_id`, is a machine's. It refuses `kind` (other than
`federation`), `preset` and the `profile` table, and an audience written on
the auth section itself. Since 0.0.21 a
Patient Identity Source is admitted only with a client credentials token,
and a Basic user holding a `system/` scope refuses the start. In FerroPIX's
configuration, `public_base_url` is a top-level key, the clients that may act
as a Patient Identity Source sit in the `[[trust.issuer]]` table apart from the
`[auth.issuer.<name>]` entry that names the issuer, `config.auth.issuer` is a
map of issuers by name, and `trust.issuer[].issuer` names the issuer by that
name, never by its URL.

The restricted-profile gate holds FerroPIX's pods to the Pod Security
Standards `restricted` profile like every other pod in the release. FerroPIX's
chart sets the two properties this chart adds for its own pods itself:
`enableServiceLinks: false`, and a user namespace per pod unless FerroPIX's
own `hostUsers` value, under `ferropix:`, is `true`.

### FerroTERM (a terminology server beside the CDR, off by default)

[FerroTERM](https://github.com/FerroHEALTH/FerroTERM) is the Ferro family's
FHIR terminology server: R4, R4B and R5 endpoints over a precomputed index, no
JVM, no database, one distroless image. `terminology.enabled` renders it as a
third workload beside the CDR and the viewer, and points the CDR at it, so
archetype value-set bindings resolve at commit and AQL `TERMINOLOGY()` expands
through it. The CDR's own `/terminology/*` extension routes are a separate
switch: they answer only with `config.api.terminology=true`, which is
`false` by default, and return `404` otherwise whether or not FerroTERM is
running. It is the Helm equivalent of the
[compose terminology overlay](compose-overlays.md#the-terminology-overlay-ferroterm).

```shell
helm upgrade --install ferroehr oci://ghcr.io/ferrohealth/charts/ferroehr \
  --version 43.0.2 -n ferroehr --reuse-values \
  --set terminology.enabled=true
```

That renders a Deployment, a ClusterIP Service, its own ServiceAccount, a
ConfigMap holding the code systems, and a NetworkPolicy. It also writes three
two things into the CDR's `ferroehr.toml`: `config.terminology.on_error` from
`terminology.failOnError`, and `config.terminology.endpoint.url` at the
Service address.

Because the chart writes those two, a values file that contradicts either is
refused at render rather than silently overwritten. Two shapes are refused
while `terminology.wireCdr` is on:

- **`config.terminology.endpoint.url`**: two answers to "which server resolves a
  binding" is how resolution ends up pointed somewhere nobody chose.
- **`config.terminology.on_error`** disagreeing with
  `terminology.failOnError`: same reason, for the posture that decides whether
  an unresolvable binding is a `422`.

Set `terminology.wireCdr=false` to keep your own server and run FerroTERM
unwired. The CDR asks one terminology server, so a server of your own and the
in-cluster FerroTERM are an either-or.

**No Ingress is rendered, and no value renders one.** The CDR is the only
caller, which is the posture the [hosted sandbox](https://sandbox.ferroehr.eu)
runs and what the SNOMED CT Affiliate Licence asks of a public deployment:
clause 2.2.4 permits a public system to serve SNOMED-encoded data provided users
cannot extract a substantial portion of the release and no fee is charged, and
clause 2.7 requires measures so the release cannot be downloaded except by
authorised users. A published FHIR terminology endpoint answers `$expand` and
`$lookup` to every client that reaches it, and FerroTERM has neither
authentication nor a rate limit of its own. Its NetworkPolicy is therefore the
one in this chart that narrows sources by default: ingress admits the CDR's pods
on the terminology port and nothing else, egress admits DNS.
`terminology.networkPolicy.extraIngressFrom` adds a peer you decide to admit.
**No boolean opens this port** the way `networkPolicy.ingressAllowAll` opens the
CDR's, and a peer you add is admitted exactly as written, which includes
admitting everything: a `namespaceSelector: {}` selects every namespace in the
cluster (networking/v1: "if present but empty, it selects all namespaces"). The
values schema therefore refuses an empty peer, and an empty `podSelector` or
`namespaceSelector` inside one, so reaching that state takes labels you chose
rather than a blank.

**Out of the box it serves the shaped seed**, which is synthetic content under
the reserved `example.test` domain: two code systems and two value sets, the
same content the conformance lane binds to. The chart carries those files and
mounts them as a ConfigMap, so a fresh install from the registry serves
terminology with no repository checkout.
`terminology.codeSystems.existingConfigMap` swaps in a ConfigMap of your own FHIR
JSON. No licensed terminology ships with FerroEHR or FerroTERM. One caveat with
a ConfigMap of your own: the pod's rollout annotation hashes the chart's own seed
files and the NAME you gave, not the contents of an object the chart does not
own, so editing that ConfigMap updates the volume while the running pods keep
serving what they read at boot. Restart the workload after such an edit
(`kubectl rollout restart deployment/ferroehr-terminology`).

**A real release is an index you build off-cluster.** Run
`ferroterm-build --rf2 <release.zip> --out <dir>` on a machine that holds the
archive, put the output on a PersistentVolume, and name that claim in
`terminology.index.persistentVolumeClaim`. The chart mounts it **read-only** at
`terminology.index.mountPath` and sets `FERROTERM__TERMINOLOGY__INDEX`; with no claim named,
the variable is absent and the seed is served alone. The chart provisions no
storage and renders no build Job, for the same reason it provisions no database:
unpacking a licensed archive is a step whose licence terms are yours, not a step
a chart should start on its own.

Naming a claim also switches this Deployment to `strategy: Recreate`. A rolling
update creates the replacement pod before the outgoing one goes away, and a
`ReadWriteOnce` volume can only be attached by one node, so the new pod would
sit Pending on the attachment while `kubectl` reported a progressing rollout.
Recreate stops the old pods first, which costs a short terminology outage at
every upgrade and finishes. If your index volume is `ReadOnlyMany`, override it
with `terminology.strategy={type: RollingUpdate}`. FerroTERM's
[loading page](https://ferroterm.eu/docs/operate/loading-snomed.html) covers the
other code systems and their build flags. Size the memory limit for the edition
before you mount it: `terminology.resources` ships a 1536Mi limit, and the same
FerroTERM page states the resident size of each edition, which the limit must
cover.

**The fail posture is the CDR's shipped one.** `terminology.failOnError`
defaults to `false`, so a binding the server cannot resolve is accepted; `true`
turns an unreachable terminology server into a `422` refusal. The
[terminology page](../beyond-core/terminology.md#when-the-terminology-server-cannot-answer)
explains both. A code the server resolves as outside its value set is refused
with `422` under either posture.

**FerroTERM's licence.** BUSL-1.1 from the same Licensor as FerroEHR:
non-commercial production use is free, any other production use needs a
commercial licence. Its README says which is which. The licence for whatever
code systems you load is separate and yours to hold; SNOMED CT needs an
Affiliate Licence (free in Member countries, see
[snomed.org/get-snomed](https://www.snomed.org/get-snomed)), and every surface
showing its content carries the notice clause 8.3.1 prescribes plus the release
version and date.

**More than one replica needs two other keys.** `terminology.replicaCount` is a
throughput choice (each replica loads its own copy of the index, so watch the
memory limit), and above 1 the pods spread one per node by default, the same
soft constraint the CDR gets. Turn `terminology.podDisruptionBudget.enabled` on
with it: without a budget a node drain may take every terminology replica at
once, and the CDR's own budget does not cover these pods. It is off by default
because the shipped replica count is 1 and a budget over a single pod blocks the
drain it exists to survive.

FerroTERM carries the same security context as the CDR and the viewer, and the
chart's render gate holds all three to the same Restricted profile. The
deployment probe harness reads this workload back off a running cluster too: its
applied runtime posture, the shaped seed reaching the process from the ConfigMap
volume, a refusal on the terminology port from a pod the policy does not name,
and the CDR resolving a bound code through the Service. Its image is
pinned by digest in `terminology.image.digest` and moves on FerroTERM's own
release line, so it does not follow `appVersion` the way the viewer image does.
Because that digest is set, `terminology.image.tag` alone deploys nothing: the
digest wins, and a tag pointing anywhere else is refused at render rather than
ignored. Clear the digest to deploy by tag.

### FerroBRIDGE (the FHIR bridge, off by default)

[FerroBRIDGE](https://github.com/FerroHEALTH/FerroBRIDGE) maps between FHIR R4
and openEHR with FHIRconnect mappings and reaches FerroEHR over ITS-REST.
FerroEHR itself serves no FHIR API for clinical data, so FerroBRIDGE is the
component that does. `bridge.enabled` renders it as its own workload in the same
release: a Deployment, a ClusterIP Service, its own ServiceAccount, a ConfigMap
holding its configuration file, and a NetworkPolicy.

The chart writes three sections of that file for you:

- **`[cdr.endpoint]`:** `url` is this release's CDR Service with its REST base
  path (`http://<release>:<service.port><config.api.base_path>`), and
  `[cdr.endpoint.credentials.oauth]` names the client of `bridge.cdr.oauth` and
  its secret, a file mounted from your Secret.
- **`[terminology]`:** with `terminology.enabled` on, `endpoint.url` is the
  chart's FerroTERM Service at `/r4` and `wire_version` is `r4`. With it off the
  section is absent, and you may set your own under `bridge.config`.
- **`[mappings]`:** `directory` is where the mapping files below are mounted.

**FerroBRIDGE reaches the CDR as a machine.** It obtains and renews a token
with the OAuth 2.0 client credentials grant (RFC 6749 §4.4) from an issuer
the CDR trusts, and the CDR checks it as it checks every token. A Basic
user is a person and holds no `system/` scope, so the chart has no Basic mode.
Add the issuer to `config.auth.issuer`, name
FerroBRIDGE's client in `bridge.cdr.oauth`, and, when it commits what
`$toopenehr` maps, grant the client in the CDR: an `AGENT` carrying its
`client_id` and performing a `ROLE` that grants `commit_composition`, as
[A system client](../policy-examples.md#a-system-client) shows. The
render is refused when `bridge.cdr.oauth.issuer` is not an issuer of
`config.auth.issuer`. FerroBRIDGE reads the token endpoint from that issuer's
metadata; when the authorization server serves none there, set
`bridge.cdr.oauth.tokenEndpoint`, and `issuer` still names the issuer the CDR
trusts.

```yaml
config:
  auth:
    issuer:
      - issuer: https://idp.example.org/realms/machines
        audience: [ferroehr-machines]
bridge:
  cdr:
    existingSecret: ferroehr-bridge-cdr
    oauth:
      issuer: https://idp.example.org/realms/machines
      clientId: ferrobridge
      scope: system/composition-*.cruds system/template-*.r system/aql-*.rs
      audience: ferroehr-machines
```

**The client secret comes from a Secret you create.** The chart never takes it
as a value. The Secret carries the key `client_secret`, which is mounted as a
file and named by `client_secret_file`, so it is in neither the ConfigMap nor
the environment.

```shell
kubectl -n ferroehr create secret generic ferroehr-bridge-cdr \
  --from-file=client_secret=./bridge-client-secret
```

**The mapping files come from a ConfigMap you create**, named in
`bridge.mappings.existingConfigMap`, with each key listed in
`bridge.mappings.files`. The chart mounts every key on its own: a whole
ConfigMap volume also carries the kubelet's `..data` snapshot directories, and
FerroBRIDGE reads those as a second and third copy of every file and
refuses the start with a duplicate mapping name. FerroBRIDGE fetches each
template a mapping context names from the CDR at boot, so upload the templates
first; a template the CDR does not hold stops the start. Without a mapping set
FerroBRIDGE still starts, and `$tofhir` and `$toopenehr` answer `503`.

**A mapping set needs `[auth]`.** With mappings FerroBRIDGE serves its
FHIRconnect operations, and it refuses to start them until FerroBRIDGE's
`[auth]` keys `issuer` and `audience`, set under `bridge.config`, name the
authorization server whose tokens its callers present, or until its `enabled`
is `false`, which serves them to every caller the NetworkPolicy admits. The keys are FerroBRIDGE's own
([configuration reference](https://ferrobridge.eu/docs/operate/configuration.html)).

```yaml
bridge:
  enabled: true
  cdr:
    existingSecret: ferroehr-bridge-cdr
    oauth:
      issuer: https://idp.example.org/realms/machines
      clientId: ferrobridge
  mappings:
    existingConfigMap: ferroehr-bridge-mappings
    files:
      - vital_signs.context.yml
      - blood_pressure.yml
  config:
    auth:
      issuer:
        callers:
          issuer: https://idp.example.org
          audience: [https://bridge.example.org/fhir]
  networkPolicy:
    ingressFrom:
      - podSelector:
          matchLabels:
            app.kubernetes.io/name: my-fhir-client
```

**It stays inside the cluster.** A caller FerroBRIDGE admits acts on the CDR
under the client above. So the chart
renders no Ingress for it and no value renders one, the Service is always
ClusterIP, and the NetworkPolicy admits only the peers you list in
`bridge.networkPolicy.ingressFrom`. An empty list is refused at render, and the
values schema refuses a peer that selects everything (an empty peer, or an empty
`podSelector` or `namespaceSelector` inside one). There is no key that admits
every source. Egress admits this release's CDR, FerroTERM when it runs, and
DNS; `bridge.networkPolicy.extraEgress` adds a rule for an upstream you name in
`bridge.config`. When you narrow the CDR's own `networkPolicy.ingressFrom`, the
chart adds FerroBRIDGE's pods to it, and FerroTERM's policy admits them too.

**Anything else goes in `bridge.config`**, rendered into the same file with
the keys of FerroBRIDGE's
[configuration reference](https://ferrobridge.eu/docs/operate/configuration.html).
FerroBRIDGE refuses an unknown key at boot. The render refuses a key the chart
writes (`cdr.endpoint`, `server.listen`, `terminology` while
FerroTERM runs, `mappings.directory` while the chart mounts the mappings) and
any secret-shaped key, such as a password, a token or `cdm.url`: set its
`_file` sibling and mount the file with `bridge.extraVolumes` and
`bridge.extraVolumeMounts`. FerroBRIDGE's FHIR facade (`[facade]`) keeps its
identity map in a file it writes, and the root filesystem is read-only, so
turning the facade on also takes a PersistentVolumeClaim of yours mounted with
those two keys and named in `facade.identity_store`, with
`bridge.strategy={type: Recreate}` when the claim is ReadWriteOnce. Back that
volume up with the CDR's database.

Probes use FerroBRIDGE's own endpoints: liveness `/health/liveness`, readiness
`/health/readiness`, which answers `503` while the CDR or FerroTERM is
unreachable. FerroBRIDGE carries the same security context as the other
workloads and the release-wide `hostUsers` setting, and the render gate holds
it to the same Restricted profile. Its image is pinned by digest in
`bridge.image.digest` and moves on FerroBRIDGE's own release line, never with
`appVersion`; a tag other than the pin is refused while the digest is set. The
deployment probe harness boots it on a cluster and reads back its runtime
posture, its admission, the credential being used for the template fetch, a
refusal on its port from a pod outside `ingressFrom`, and its readiness from a
pod inside it. FerroBRIDGE's own deployment documentation is at
<https://ferrobridge.eu/docs/operate/container.html>.

### Per-domain backups (one CronJob per domain, off by default)

`backup.enabled` renders one CronJob per pseudonymisation domain:
`backup.clinical.schedule` dumps the clinical schemas, `backup.party.schedule`
dumps the identities, and each writes to its own existing claim
(`backup.clinical.persistentVolumeClaim`, `backup.party.persistentVolumeClaim`).
The image is `backup.image.repository`, which carries `pg_dump`.

**A third job renders for the audit domain when `database.audit.existingSecret`
gives it a database of its own.** While the trail shares the clinical database
the clinical dump names its schema and nothing more is needed; once it is
somewhere else, that dump cannot reach it and nothing else would take it, so
`backup.audit.schedule`, `backup.audit.existingSecret` and
`backup.audit.persistentVolumeClaim` become required the same way the other
two are, and the clinical dump stops naming the `audit` schema.

The chart refuses to render rather than let the separation collapse quietly: a
missing claim is an error naming the value, and naming the **same** claim for
any two domains is an error too, because one volume holding the clinical record
beside the identities is the state per-domain backups exist to prevent.

Each job needs **its own backup credential**, named by
`backup.clinical.existingSecret`, `backup.party.existingSecret` and, when it
renders,
`backup.audit.existingSecret`, and
the render is refused without them. It cannot be the pool's credential: each
domain's runtime role is revoked from the other domains, so a dump taken through
one would be silently partial. Give each domain a read-only role on that
domain's schema alone. Every job reads its DSN from a mounted file, never an
environment variable.

The chart provisions no storage. Create the claims yourself, and give them
different access control; that part no chart can do for you. The restore
procedure, and the `ferroehr db verify` gate that judges it, are on the
[Operating](../operations.md) page.
