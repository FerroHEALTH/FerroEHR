{{/*
Expand the name of the chart.
*/}}
{{- define "ferroehr.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Fully qualified app name (max 63 chars for DNS labels).
*/}}
{{- define "ferroehr.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Chart name and version, for the helm.sh/chart label.
*/}}
{{- define "ferroehr.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels.
*/}}
{{- define "ferroehr.labels" -}}
helm.sh/chart: {{ include "ferroehr.chart" . }}
{{ include "ferroehr.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: ferroehr
{{- end }}

{{/*
Selector labels.
*/}}
{{- define "ferroehr.selectorLabels" -}}
app.kubernetes.io/name: {{ include "ferroehr.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
ServiceAccount name.
*/}}
{{- define "ferroehr.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "ferroehr.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
The container port the ops-introspection surface (prometheus / metrics / info /
env / loggers) is reachable on. When management.port is set the surface runs on
its own listener; otherwise it shares the main API port. The health probes are
never on this port-selection path: they are always served on the main HTTP port.
*/}}
{{- define "ferroehr.managementPort" -}}
{{- if .Values.config.management.port }}
{{- .Values.config.management.port }}
{{- else }}
{{- 8080 }}
{{- end }}
{{- end }}

{{/*
The container image reference.

A digest wins over a tag, and the separator differs: `repository@sha256:…` versus
`repository:tag`. A digest is what a build-provenance attestation is made over, so
deploying by digest is what makes `gh attestation verify` bind to the image that
is actually running — a tag can be moved after it is verified.

`sha256:` is accepted with or without the prefix, because both spellings are in
circulation and the one-character difference between them is an unhelpful way to
fail a deployment.
*/}}
{{- define "ferroehr.image" -}}
{{- if .Values.image.digest }}
{{- $digest := .Values.image.digest }}
{{- if not (hasPrefix "sha256:" $digest) }}{{- $digest = printf "sha256:%s" $digest }}{{- end }}
{{- printf "%s@%s" .Values.image.repository $digest }}
{{- else }}
{{- printf "%s:%s" .Values.image.repository (.Values.image.tag | default .Chart.AppVersion) }}
{{- end }}
{{- end }}

{{/*
The chart-managed Secret name (env-borne secrets: DB DSN, passphrases, keys).
*/}}
{{- define "ferroehr.secretName" -}}
{{- printf "%s-env" (include "ferroehr.fullname" .) }}
{{- end }}

{{/*
The chart-managed config-files Secret name (mounted TOML/PEM at /etc/ferroehr).
*/}}
{{- define "ferroehr.configSecretName" -}}
{{- printf "%s-config" (include "ferroehr.fullname" .) }}
{{- end }}

{{/*
Whether any storage domain carries an INLINE DSN the chart must put in its own
Secret. One helper rather than a branch per domain, so a new domain is a values
block and a name in the list.
*/}}
{{- define "ferroehr.hasInlineDomainDsn" -}}
{{- range $domain := list "clinical" "party" "linkage" "audit" -}}
{{- $dsn := index $.Values.database $domain -}}
{{- if and (not $dsn.existingSecret) $dsn.url -}}
true
{{- end -}}
{{- end -}}
{{- end }}

{{/*
Whether any storage domain names an operator-supplied Secret, which the chart
projects as a file without creating a Secret of its own.
*/}}
{{- define "ferroehr.hasDomainSecret" -}}
{{- range $domain := list "clinical" "party" "linkage" "audit" -}}
{{- $dsn := index $.Values.database $domain -}}
{{- if $dsn.existingSecret -}}
true
{{- end -}}
{{- end -}}
{{- end }}

{{/*
Whether the chart-managed env Secret has any content (so we only create/mount it
when there is at least one secret value to carry).
*/}}
{{- define "ferroehr.hasChartSecret" -}}
{{- $inlineDb := and (not .Values.database.existingSecret) .Values.database.url }}
{{- $inlineDomain := include "ferroehr.hasInlineDomainDsn" . }}
{{- $inlineMigrate := and (not .Values.database.migrateExistingSecret) .Values.database.migrateUrl }}
{{- if or $inlineDb (eq $inlineDomain "true") $inlineMigrate .Values.secrets.basicUserPasswordHashes .Values.secrets.authOidcHmacSecret .Values.secrets.signingKeyPassphrase .Values.secrets.eventsUrl .Values.secrets.fhirOutboundUrl .Values.secrets.auditFhirFeedUrl .Values.secrets.multimediaAccessKeyId .Values.secrets.multimediaSecretAccessKey .Values.secrets.terminologyOauth2ClientSecrets -}}
true
{{- end -}}
{{- end }}

{{/*
Whether any secret is carried as a FILE rather than an env value. A secret whose
config key has a `*_file` sibling is mounted from a read-only volume, which the
OWASP Kubernetes Security Cheat Sheet prefers over an environment variable
(https://cheatsheetseries.owasp.org/cheatsheets/Kubernetes_Security_Cheat_Sheet.html).
*/}}
{{- define "ferroehr.hasFileSecrets" -}}
{{- if or (eq (include "ferroehr.hasChartFileSecrets" .) "true") .Values.database.existingSecret (eq (include "ferroehr.hasDomainSecret" .) "true") .Values.database.migrateExistingSecret -}}
true
{{- end -}}
{{- end }}

{{/*
Whether the CHART-MANAGED Secret carries any file-borne key. Distinct from
`hasFileSecrets`: an operator-supplied `database.existingSecret` puts a file in the
mount without the chart's own Secret existing at all, and projecting a source that
names a Secret nothing created makes the pod fail to mount.
*/}}
{{- define "ferroehr.hasChartFileSecrets" -}}
{{- $inlineDb := and (not .Values.database.existingSecret) .Values.database.url -}}
{{- $inlineDomain := include "ferroehr.hasInlineDomainDsn" . -}}
{{- $inlineMigrate := and (not .Values.database.migrateExistingSecret) .Values.database.migrateUrl -}}
{{- if or $inlineDb (eq $inlineDomain "true") $inlineMigrate .Values.secrets.authOidcHmacSecret .Values.secrets.signingKeyPassphrase .Values.secrets.multimediaSecretAccessKey .Values.secrets.terminologyOauth2ClientSecrets .Values.secrets.basicUserPasswordHashes .Values.secrets.eventsUrl .Values.secrets.fhirOutboundUrl -}}
true
{{- end -}}
{{- end }}

{{/*
The directory the file-borne secrets are mounted at. Deliberately NOT under
/etc/ferroehr: that path is the config projection, and authz.abac.cedar.policy_dir
commonly points at it, so a secrets subdirectory there would be walked as policy.
*/}}
{{- define "ferroehr.secretMountPath" -}}
/etc/ferroehr-secrets
{{- end }}

{{/*
Secret-shaped leaf key names, as one regex. A ConfigMap is not a sensitive
object — it is readable with namespace read, collected by backup tooling that
excludes Secrets, and not covered by Secret encryption at rest — so a credential
must never reach one (Kubernetes ConfigMap docs: "A ConfigMap is not designed to
hold large chunks of data … If you want to store data that is confidential, use
a Secret", https://kubernetes.io/docs/concepts/configuration/configmap/).

Deny-by-default is the point: matching on the NAME rather than on a fixed list of
today's keys is what makes a secret key added to the server's config tree
tomorrow fail the render instead of leaking silently. The `_file`/`_path`/`_dir`
suffixes are the one exemption class, and they are provably safe: those keys hold
a PATH, never a credential.
*/}}
{{- define "ferroehr.secretKeyPattern" -}}
(password|passphrase|secret|credential|private_key|api_key|(^|_)token$)
{{- end }}

{{/*
Recursive scan of a config subtree for secret-bearing keys, emitting one
`class\tpath\tremedy` line per finding. Call with (dict "node" <tree> "path" "").

Two classes, because there are two right answers. `routed` = the chart already
carries this secret through a `secrets:` key, so a value under `config:` is an
operator mistake and the render fails naming the key that belongs there.
`unrouted` = no `secrets:` route exists, so the whole rendered ferroehr.toml moves
into the chart Secret and no ConfigMap is created — the safe direction, taken
automatically, for a key nobody has routed yet.

NOTE: the `unrouted` class currently has NO members — every secret the server
models now has either a `*_file` sibling or a Secret-borne env route, including a
Basic user's hash since `password_hash_file` landed. The branch is kept precisely
because that can change: a secret key added upstream with no file sibling must
fail safe by default rather than land in a ConfigMap, and deleting the branch is
what would make the next one leak.

The four SecretUrl leaves are matched by PATH because their name (`url`) carries
no shape a classifier can see — a URL's userinfo component is the credential.
*/}}
{{- define "ferroehr.secretScan" -}}
{{- $urlPaths := list "db.url" "storage.clinical.url" "storage.party.url" "storage.linkage.url" "storage.audit.url" "events.url" "fhir.outbound.url" "audit.fhir_feed.url" -}}
{{- $node := .node -}}
{{- $path := .path -}}
{{- $kind := kindOf $node -}}
{{- if eq $kind "map" -}}
{{- range $key, $value := $node -}}
{{- $child := ternary $key (printf "%s.%s" $path $key) (eq $path "") -}}
{{- if or (has $child $urlPaths) (and (regexMatch (include "ferroehr.secretKeyPattern" $) $key) (not (regexMatch "(_file|_path|_dir)$" $key))) -}}
{{- printf "%s\t%s\t%s\n" (include "ferroehr.secretClass" $key) $child (include "ferroehr.secretRemedy" $key) -}}
{{- else -}}
{{- include "ferroehr.secretScan" (dict "node" $value "path" $child) -}}
{{- end -}}
{{- end -}}
{{- else if eq $kind "slice" -}}
{{- range $index, $value := $node -}}
{{- include "ferroehr.secretScan" (dict "node" $value "path" (printf "%s[%d]" $path $index)) -}}
{{- end -}}
{{- end -}}
{{- end }}

{{/*
Whether the chart has a dedicated `secrets:` route for a secret-bearing config
key, keyed on its leaf name. Unknown ⇒ `unrouted`, which is what makes a secret
key added upstream tomorrow move to the Secret instead of leaking.
*/}}
{{- define "ferroehr.secretClass" -}}
{{- if has . (list "url" "hmac_secret" "key_passphrase" "secret_access_key" "client_secret" "password_hash") -}}
routed
{{- else -}}
unrouted
{{- end -}}
{{- end }}

{{/*
The `secrets:` key that carries a routed secret, keyed on its leaf name.
*/}}
{{- define "ferroehr.secretRemedy" -}}
{{- $routes := dict
  "url" "route it through the matching `secrets:` key — `eventsUrl` or `fhirOutboundUrl` (mounted as files via events.url_file / fhir.outbound.url_file), `auditFhirFeedUrl` (env; that key still has no `*_file` sibling), or `database.existingSecret` for the shared DSN (mounted via db.url_file) and `database.<domain>.existingSecret` for one storage domain's own (mounted via storage.<domain>.url_file)"
  "hmac_secret" "set `secrets.authOidcHmacSecret` instead"
  "key_passphrase" "set `secrets.signingKeyPassphrase` instead"
  "secret_access_key" "set `secrets.multimediaSecretAccessKey` instead"
  "client_secret" "set `secrets.terminologyOauth2ClientSecrets.<client name>` instead"
  "password_hash" "set `secrets.basicUserPasswordHashes.<username>` instead, and leave only `username`/`roles` under `config:` (the chart mounts the hash and injects password_hash_file)"
-}}
{{- get $routes . | default "no `secrets:` route exists for this key" -}}
{{- end }}

{{/*
"true" when the rendered ferroehr.toml carries a secret the chart cannot route
elsewhere, in which case it is delivered by the Secret and NO ConfigMap exists.
*/}}
{{- define "ferroehr.configInSecret" -}}
{{- $findings := include "ferroehr.secretScan" (dict "node" (omit .Values.config "files") "path" "") | trim -}}
{{- range $finding := splitList "\n" $findings -}}
{{- if hasPrefix "unrouted\t" $finding -}}
true
{{- end -}}
{{- end -}}
{{- end }}

{{/*
The rendered ferroehr.toml body: `.Values.config` minus the separately-mounted
`files`, with the file-borne secret PATHS the chart owns injected, and a routed
secret VALUE refused outright.
*/}}
{{- define "ferroehr.configToml" -}}
{{- $config := omit .Values.config "files" -}}
{{/* Multi-tenancy was withdrawn: the server rejects unknown configuration keys,
     so a values file still carrying config.tenancy renders a ferroehr.toml the
     pod refuses at boot and crash-loops on. Refuse it here, where the operator
     reads the reason (CHANGELOG.md, ### Removed: multi-tenancy is achieved by
     running separate instances). */}}
{{- if hasKey $config "tenancy" -}}
{{- fail "config.tenancy is set, and multi-tenancy no longer exists: it is achieved by running separate instances — one instance, one database, one set of domain roles per organisation (CHANGELOG.md, ### Removed). The server refuses unknown configuration keys, so rendering this key would crash-loop the pod at boot. Drop config.tenancy." -}}
{{- end -}}
{{- $findings := include "ferroehr.secretScan" (dict "node" $config "path" "") | trim -}}
{{- $lines := list -}}
{{- range $finding := splitList "\n" $findings -}}
{{- $parts := splitn "\t" 3 $finding -}}
{{- if eq $parts._0 "routed" -}}
{{- $lines = append $lines (printf "  - config.%s: %s" $parts._1 $parts._2) -}}
{{- end -}}
{{- end -}}
{{- if $lines -}}
{{- fail (printf "refusing to render a secret into the ConfigMap (a ConfigMap is not a sensitive object — it is readable with namespace read, collected by tooling that skips Secrets, and not covered by Secret encryption at rest):\n%s" (join "\n" $lines)) -}}
{{- end -}}
{{- $rendered := deepCopy $config -}}
{{- range $user, $hash := .Values.secrets.basicUserPasswordHashes -}}
{{- $users := dig "auth" "basic" "users" (list) $rendered -}}
{{- $matched := false -}}
{{- range $entry := $users -}}
{{- if eq (get $entry "username") $user -}}
{{- $_ := set $entry "password_hash_file" (printf "%s/auth.basic.users.%s.password_hash" (include "ferroehr.secretMountPath" $) $user) -}}
{{- $matched = true -}}
{{- end -}}
{{- end -}}
{{- if not $matched -}}
{{- fail (printf "secrets.basicUserPasswordHashes.%s has no matching entry at config.auth.basic.users[] with username: %s (declare the username and roles there; only the hash belongs in secrets:)" $user $user) -}}
{{- end -}}
{{- end -}}
{{- range $name, $value := .Values.secrets.terminologyOauth2ClientSecrets -}}
{{- $clients := dig "terminology" "external" "oauth2_clients" (dict) $rendered -}}
{{- if not (hasKey $clients $name) -}}
{{- fail (printf "secrets.terminologyOauth2ClientSecrets.%s has no client declared at config.terminology.external.oauth2_clients.%s (declare its token_url/client_id there; only the secret belongs here)" $name $name) -}}
{{- end -}}
{{- $_ := set (get $clients $name) "client_secret_file" (printf "%s/terminology.external.oauth2_clients.%s.client_secret" (include "ferroehr.secretMountPath" $) $name) -}}
{{- end -}}
{{- if and $.Values.terminology.enabled $.Values.terminology.wireCdr -}}
{{- /* The in-cluster FerroTERM is wired the same way the mounted secret paths
       above are: by injection, so the Service address and the workload that
       answers it are one decision. An operator who declared the `default`
       provider by hand meant a different server, and silently replacing it
       would point binding resolution somewhere they did not choose.

       Every key this block WRITES is refused when the operator wrote it too.
       An injection that overwrites is indistinguishable from one that agrees:
       the render succeeds either way and the value an operator reads back in
       their own values file is not the value the server runs on. The chart's
       own `config.terminology.external` defaults therefore declare neither
       `enabled` nor `fail_on_error` — a defaulted key cannot be told apart
       from a typed one, and the server's own defaults for both are the same
       `false`, so nothing is lost by leaving them to it. */ -}}
{{- if not (hasKey $rendered "terminology") -}}{{- $_ := set $rendered "terminology" (dict) -}}{{- end -}}
{{- $terminology := get $rendered "terminology" -}}
{{- if not (hasKey $terminology "external") -}}{{- $_ := set $terminology "external" (dict) -}}{{- end -}}
{{- $external := get $terminology "external" -}}
{{- $providers := dig "providers" (dict) $external -}}
{{- if hasKey $providers "default" -}}
{{- fail "terminology.enabled=true already declares config.terminology.external.providers.default at the in-cluster FerroTERM Service, so declaring it in values too is two answers to one question: drop config.terminology.external.providers.default, or set terminology.wireCdr=false to keep your own provider and run FerroTERM unwired. A second, differently-named provider under config.terminology.external.providers is unaffected." -}}
{{- end -}}
{{- if and (hasKey $external "enabled") (not (get $external "enabled")) -}}
{{- fail "config.terminology.external.enabled=false is set while terminology.enabled=true and terminology.wireCdr=true, which inject enabled=true — the rendered ferroehr.toml would say the opposite of your values file, and the CDR would resolve bindings against FerroTERM anyway. Drop config.terminology.external.enabled, or set terminology.wireCdr=false to keep your own [terminology.external] and run FerroTERM unwired." -}}
{{- end -}}
{{- if and (hasKey $external "fail_on_error") (ne (get $external "fail_on_error") $.Values.terminology.failOnError) -}}
{{- fail (printf "config.terminology.external.fail_on_error=%v contradicts terminology.failOnError=%v, and the wiring injects the second — the rendered ferroehr.toml would carry a fail posture your values file denies. Set terminology.failOnError=%v and drop config.terminology.external.fail_on_error, or set terminology.wireCdr=false to keep your own [terminology.external]." (get $external "fail_on_error") $.Values.terminology.failOnError (get $external "fail_on_error")) -}}
{{- end -}}
{{- /* A provider nothing routes to is a provider the router never reaches:
       an unmatched terminology falls back to the provider named `default`,
       which the injection above has just claimed for the in-cluster
       FerroTERM. So an operator's own provider declared beside the injection
       without a `routes` entry naming it is dead configuration that renders,
       boots and resolves nothing — the silent-demotion shape, refused rather
       than shipped. */ -}}
{{- $extraProviders := keys (omit $providers "default") -}}
{{- if and $extraProviders (not (dig "routes" (dict) $external)) -}}
{{- fail (printf "config.terminology.external.providers declares %v while config.terminology.external.routes is empty, and terminology.wireCdr=true has taken the `default` provider for the in-cluster FerroTERM: an unrouted terminology falls back to `default`, so nothing would ever reach %v. Add a config.terminology.external.routes entry (terminology id or system URI -> provider name) for each, or set terminology.wireCdr=false to keep `default` for yourself." (join ", " $extraProviders) (join ", " $extraProviders)) -}}
{{- end -}}
{{- $_ := set $providers "default" (dict "type" "fhir" "url" (printf "http://%s:%v/r4b" (include "ferroehr.terminologyFullname" $) $.Values.terminology.service.port)) -}}
{{- $_ := set $external "providers" $providers -}}
{{- $_ := set $external "enabled" true -}}
{{- $_ := set $external "fail_on_error" $.Values.terminology.failOnError -}}
{{- end -}}
{{- /* The usage report: the switch is the top-level usageReport.enabled value,
       and the deployment kind the report states is always `helm` under this
       chart. Both keys are injected, so an operator who also wrote either under
       config.usage_report is refused, for the reason the terminology injection
       above gives: an injection that overwrites cannot be told apart from one
       that agrees. The endpoint and slow_aql_ms pass through config untouched. */ -}}
{{- if not (hasKey $rendered "usage_report") -}}{{- $_ := set $rendered "usage_report" (dict) -}}{{- end -}}
{{- $usageReport := get $rendered "usage_report" -}}
{{- if or (hasKey $usageReport "enabled") (hasKey $usageReport "deployment") -}}
{{- fail "config.usage_report.enabled or config.usage_report.deployment is set, and the chart injects both: the usage report switch is usageReport.enabled, and the deployment it states is always helm under this chart. Drop the config.usage_report key and set usageReport.enabled instead." -}}
{{- end -}}
{{- $_ := set $usageReport "enabled" $.Values.usageReport.enabled -}}
{{- $_ := set $usageReport "deployment" "helm" -}}
{{- toToml $rendered -}}
{{- end }}

{{/*
Whether the config-files Secret exists: any `config.files` entry, or the whole
rendered TOML having moved there because it carries an unroutable secret.
*/}}
{{- define "ferroehr.hasConfigSecret" -}}
{{- if or .Values.config.files (eq (include "ferroehr.configInSecret" .) "true") -}}
true
{{- end -}}
{{- end }}

{{/*
The pod-level user-namespace field, rendered only when it asks for something
other than the API default.

`hostUsers: false` puts the pod in its own user namespace: container UID 0 maps
to an unprivileged host UID, so capabilities held inside the container are not
capabilities on the node (KEP-127 — keps/sig-node/127-user-namespaces — alpha
v1.25, beta v1.35, stable v1.36, which is this chart's kubeVersion floor, so it
renders ungated).

`hostUsers: true` IS the API default, so it is omitted rather than stated
("Don't specify default values unnecessarily",
https://kubernetes.io/docs/concepts/configuration/overview/). The difference
matters here beyond tidiness: an omitted field lets a future cluster-wide
default apply, while an explicit `true` would pin the pod to the weaker posture
forever.

The `if` lives at each CALL SITE rather than in here, because a helper that
renders nothing still leaves the `nindent` that invoked it — an indented blank
line in every manifest.
*/}}
{{- define "ferroehr.hostUsers" -}}
hostUsers: false
{{- end }}

{{/*
The default topology spread: prefer one replica per node.

Rendered only when the operator supplied no constraints of their own, and
replaced wholesale when they did — a merge of two spreading policies is not a
spreading policy, and the operator's `labelSelector` is the part that would
silently disagree with ours.

`ScheduleAnyway` rather than `DoNotSchedule`: this is an availability
preference, not a placement requirement, and a hard constraint would leave a
replica Pending forever on a single-node cluster instead of running it
(https://kubernetes.io/docs/concepts/scheduling-eviction/topology-spread-constraints/).

`matchLabelKeys: [pod-template-hash]` scopes the skew calculation to the
ReplicaSet being rolled. Without it a rolling update counts the OUTGOING pods
too, so the scheduler sees a skew that the rollout is in the middle of
resolving and places the new pods to correct it — the "respect topology spread
after rolling upgrade" problem KEP-3243 exists for. That KEP is beta and
enabled by default from v1.27 with stable targeted at v1.36; if a cluster has
the gate off the field is ignored and the constraint still spreads, just with
the older counting. A graceful degradation, so it is not version-gated.
*/}}
{{- define "ferroehr.topologySpreadConstraints" -}}
{{- if .Values.topologySpreadConstraints -}}
{{- toYaml .Values.topologySpreadConstraints -}}
{{- else -}}
- maxSkew: 1
  topologyKey: kubernetes.io/hostname
  whenUnsatisfiable: ScheduleAnyway
  matchLabelKeys:
    - pod-template-hash
  labelSelector:
    matchLabels:
      {{- include "ferroehr.selectorLabels" . | nindent 6 }}
{{- end }}
{{- end }}

{{/*
The viewer's resource name: the release fullname plus a suffix, so the
viewer's objects never collide with the server's and are obvious in `kubectl
get`.
*/}}
{{- define "ferroehr.viewerFullname" -}}
{{- printf "%s-viewer" (include "ferroehr.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Viewer labels.

The viewer carries its own `app.kubernetes.io/name` rather than the server's
name plus a `component`. That is the documented convention — the recommended-
labels example gives a WordPress chart's MySQL workload `name: mysql`,
`component: database`, `part-of: wordpress`
(https://kubernetes.io/docs/concepts/overview/working-with-objects/common-labels/)
— and here it is load-bearing, not cosmetic.

A Service selector is a SUBSET match. While the viewer's pods carried the
server's `name` and `instance` plus an extra `component`, they matched the
server's own Service and PodDisruptionBudget, whose selectors are exactly that
pair. Because the viewer's container port is also named `http`, the Service's
`targetPort: http` resolved to 3000 on those pods, so a share of openEHR API
requests were answered by the viewer — and the PDB counted three pods
where it guards two, inflating `disruptionsAllowed` enough for a drain to evict
both server replicas at once. Adding `component` to the SERVER's selectors
instead would have fixed the same overlap, but a Service applied before its
Deployment selects zero pods until the new ReplicaSet is Ready: a brief total
outage on a clinical API at every upgrade.
*/}}
{{- define "ferroehr.viewerLabels" -}}
helm.sh/chart: {{ include "ferroehr.chart" . }}
{{ include "ferroehr.viewerSelectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: ferroehr
app.kubernetes.io/component: viewer
{{- end }}

{{- define "ferroehr.viewerSelectorLabels" -}}
app.kubernetes.io/name: {{ printf "%s-viewer" (include "ferroehr.name" .) | trunc 63 | trimSuffix "-" }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
The migration hook pod's labels, and its own `app.kubernetes.io/name`.

Same reason the viewer carries one (see above): a Service selector is a SUBSET
match, so a pod labelled with the server's selector pair is selected by the
server's Service and PodDisruptionBudget for as long as it exists — and this
pod exists during every install and upgrade, which is exactly when a drain or
a rollout is reading those objects. The migration pod exposes no port named
`http`, so the Service has no endpoint to route to it today; the PDB match is
real, and the port is a property of this pod rather than a guarantee about the
next one. Giving the hook its own name takes both out of scope rather than
relying on that.
*/}}
{{- define "ferroehr.migrationLabels" -}}
helm.sh/chart: {{ include "ferroehr.chart" . }}
{{ include "ferroehr.migrationSelectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: ferroehr
app.kubernetes.io/component: migration
{{- end }}

{{- define "ferroehr.migrationSelectorLabels" -}}
app.kubernetes.io/name: {{ printf "%s-migration" (include "ferroehr.name" .) | trunc 63 | trimSuffix "-" }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
The viewer image reference — digest wins over tag, as for the server.
*/}}
{{- define "ferroehr.viewerImage" -}}
{{- if .Values.viewer.image.digest }}
{{- $digest := .Values.viewer.image.digest }}
{{- if not (hasPrefix "sha256:" $digest) }}{{- $digest = printf "sha256:%s" $digest }}{{- end }}
{{- printf "%s@%s" .Values.viewer.image.repository $digest }}
{{- else }}
{{- printf "%s:%s" .Values.viewer.image.repository (.Values.viewer.image.tag | default .Chart.AppVersion) }}
{{- end }}
{{- end }}

{{/*
A backup CronJob's resource name: the release fullname, `-backup-`, and the
domain it dumps.

Truncated to 52, not 63: the CronJob controller appends 11 characters to build
each Job's name and a Job name may not exceed 63
(https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/).

Call with (dict "root" $ "domain" "clinical").
*/}}
{{- define "ferroehr.backupFullname" -}}
{{- printf "%s-backup-%s" (include "ferroehr.fullname" .root) .domain | trunc 52 | trimSuffix "-" }}
{{- end }}

{{/*
Labels for a backup CronJob and its pods.

Each domain carries its OWN `app.kubernetes.io/name` rather than the server's
name plus a component, for the reason the viewer's labels carry: a Service or
PodDisruptionBudget selector is a SUBSET match, so a dump pod labelled with the
server's name would be selected by the server's Service while it runs
(https://kubernetes.io/docs/concepts/overview/working-with-objects/common-labels/).
*/}}
{{- define "ferroehr.backupLabels" -}}
helm.sh/chart: {{ include "ferroehr.chart" .root }}
{{ include "ferroehr.backupSelectorLabels" . }}
{{- if .root.Chart.AppVersion }}
app.kubernetes.io/version: {{ .root.Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .root.Release.Service }}
app.kubernetes.io/part-of: ferroehr
app.kubernetes.io/component: backup
{{- end }}

{{- define "ferroehr.backupSelectorLabels" -}}
app.kubernetes.io/name: {{ printf "%s-backup-%s" (include "ferroehr.name" .root) .domain | trunc 63 | trimSuffix "-" }}
app.kubernetes.io/instance: {{ .root.Release.Name }}
{{- end }}

{{/*
The backup image reference — digest wins over tag, as for the server.
*/}}
{{- define "ferroehr.backupImage" -}}
{{- if .Values.backup.image.digest }}
{{- $digest := .Values.backup.image.digest }}
{{- if not (hasPrefix "sha256:" $digest) }}{{- $digest = printf "sha256:%s" $digest }}{{- end }}
{{- printf "%s@%s" .Values.backup.image.repository $digest }}
{{- else }}
{{- printf "%s:%s" .Values.backup.image.repository (.Values.backup.image.tag | default .Chart.AppVersion) }}
{{- end }}
{{- end }}

{{/*
The fixed path a dump is written to. A path, not a value: the claim behind it is
the operator's choice, the location inside the container is not.
*/}}
{{- define "ferroehr.backupMountPath" -}}
/backup
{{- end }}

{{/*
The Secret a backup domain's DSN comes from, and the key inside it.

Looked up by domain name under `backup`, so a new domain is a values block and
a table entry in backup-cronjob.yaml rather than another branch here. Whichever
Secret it is, the key is mounted as `db.url` and only that PATH reaches the
container, so the command is identical for every domain and only the credential
differs.

Call with (dict "root" $ "domain" "clinical").

The backup DSN is the domain's OWN backup Secret, never the pool's credential.
Each domain's runtime role is revoked from the other domains, so a dump taken
through it would be silently partial. The dump needs a read-only role on its own
domain, which is a different credential from the one the server serves with.
*/ -}}
{{- define "ferroehr.backupDsnSecret" -}}
{{ (index .root.Values.backup .domain).existingSecret }}
{{- end }}

{{- define "ferroehr.backupDsnSecretKey" -}}
{{ (index .root.Values.backup .domain).existingSecretKey }}
{{- end }}

{{/*
The terminology workload's resource name: the release fullname plus a suffix,
so FerroTERM's objects never collide with the CDR's.
*/}}
{{- define "ferroehr.terminologyFullname" -}}
{{- printf "%s-terminology" (include "ferroehr.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Terminology labels.

FerroTERM carries its OWN `app.kubernetes.io/name` rather than the CDR's name
plus a `component`, for the reason the viewer's labels carry (see above): a
Service or PodDisruptionBudget selector is a SUBSET match, so a pod labelled
with the server's selector pair is selected by the server's Service — and this
container also listens on 8080, which is what the CDR's `targetPort: http`
resolves to
(https://kubernetes.io/docs/concepts/overview/working-with-objects/common-labels/).
*/}}
{{- define "ferroehr.terminologyLabels" -}}
helm.sh/chart: {{ include "ferroehr.chart" . }}
{{ include "ferroehr.terminologySelectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: ferroehr
app.kubernetes.io/component: terminology
{{- end }}

{{- define "ferroehr.terminologySelectorLabels" -}}
app.kubernetes.io/name: {{ printf "%s-terminology" (include "ferroehr.name" .) | trunc 63 | trimSuffix "-" }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
The FerroTERM release this chart is pinned to, and the ONE place that version
is written.

A separate product on its own release line, so it does NOT fall back to
`.Chart.AppVersion` the way the viewer and the backup image do: those are built
from this repository at the same version, and FerroTERM is not. Three sites
read this one string — the `terminology.image.tag` default below, the
tag-versus-digest refusal beside it, and scripts/checks/chart-appversion.sh,
which holds the `artifacthub.io/images` entry in Chart.yaml to it. Bumping
FerroTERM is this line plus `terminology.image.digest` in values.yaml; the
annotation is then checked against both rather than maintained a third time.
*/}}
{{- define "ferroehr.terminologyPinnedVersion" -}}
0.1.6
{{- end }}

{{/*
The terminology image reference — digest wins over tag, as for the server.

Unlike the server's, this chart ships a DEFAULT digest, so the tag is inert out
of the box: a `--set terminology.image.tag=…` alone changes the values file and
not the deployed image. That is refused rather than rendered, because the two
keys then disagree about which FerroTERM is running and only the digest is
telling the truth.
*/}}
{{- define "ferroehr.terminologyImage" -}}
{{- $pinned := include "ferroehr.terminologyPinnedVersion" . }}
{{- if and .Values.terminology.image.digest .Values.terminology.image.tag (ne .Values.terminology.image.tag $pinned) }}
{{- fail (printf "terminology.image.tag=%s is set while terminology.image.digest is non-empty, and a digest wins over a tag here: the render would deploy the digest and ignore the tag entirely. Set terminology.image.digest=\"\" to deploy tag %s, or set terminology.image.digest to that tag's own digest. The chart pins FerroTERM %s by digest by default, which is the pairing this chart was validated against." .Values.terminology.image.tag .Values.terminology.image.tag $pinned) }}
{{- end }}
{{- if .Values.terminology.image.digest }}
{{- $digest := .Values.terminology.image.digest }}
{{- if not (hasPrefix "sha256:" $digest) }}{{- $digest = printf "sha256:%s" $digest }}{{- end }}
{{- printf "%s@%s" .Values.terminology.image.repository $digest }}
{{- else }}
{{- printf "%s:%s" .Values.terminology.image.repository (.Values.terminology.image.tag | default $pinned) }}
{{- end }}
{{- end }}

{{/*
FerroTERM's topology spread: the CDR's default, over FerroTERM's own pods.

Identical reasoning to `ferroehr.topologySpreadConstraints` above, including the
`ScheduleAnyway` preference and the `matchLabelKeys: [pod-template-hash]` scope
— what differs is the `labelSelector`, which must name THIS workload's pods.
Rendered unconditionally, like the CDR's: a replica count is an operator's
runtime decision, and a spread policy that appears only above some threshold is
a policy nobody can state.
*/}}
{{- define "ferroehr.terminologyTopologySpreadConstraints" -}}
{{- if .Values.terminology.topologySpreadConstraints -}}
{{- toYaml .Values.terminology.topologySpreadConstraints -}}
{{- else -}}
- maxSkew: 1
  topologyKey: kubernetes.io/hostname
  whenUnsatisfiable: ScheduleAnyway
  matchLabelKeys:
    - pod-template-hash
  labelSelector:
    matchLabels:
      {{- include "ferroehr.terminologySelectorLabels" . | nindent 6 }}
{{- end }}
{{- end }}

{{/*
The ConfigMap holding the shaped code systems FerroTERM serves out of the box:
the chart's own, unless the operator named one of theirs.
*/}}
{{- define "ferroehr.terminologyCodeSystemsConfigMap" -}}
{{- if .Values.terminology.codeSystems.existingConfigMap }}
{{- .Values.terminology.codeSystems.existingConfigMap }}
{{- else }}
{{- printf "%s-codesystems" (include "ferroehr.terminologyFullname" .) }}
{{- end }}
{{- end }}
