# Secrets, detection & response

What this deployment's Secrets actually contain, how to notice a compromise on a
shell-less image, what to do when you have one, and where a managed control plane
takes controls out of your hands.

<!-- toc -->

## Secrets at rest, and what ours contain

**Operator's:** encryption at rest is an API-server flag
(`--encryption-provider-config`), and **Kubernetes does not encrypt Secrets by
default**. A `Secret` is base64-encoded, which is an encoding and not a
protection: without that configuration, everything below sits readable in etcd,
which is why [the etcd
section](hardening-cluster.md#etcd-and-what-our-secrets-contain) is the other half
of this one.

**The useful half is ours: exactly what this deployment's Secrets contain**, so
you can judge the exposure rather than reading "secrets" generically.

| Secret content | Present when | What it gets an attacker |
|---|---|---|
| **The database DSN** | always | **direct read/write access to all patient data**, bypassing the API, its authorization and its audit trail entirely |
| A Basic user's Argon2id hash | `secrets.basicUserPasswordHashes` | an offline cracking target, not a usable password |
| The OIDC HMAC secret | `secrets.authOidcHmacSecret` (HS256 development setups) | the ability to **mint valid tokens** for any user and role |
| The version-signing passphrase (plus the PGP key via `config.files`) | `config.signing.mode: pgp` | the ability to forge version signatures, breaking the integrity guarantee |
| A terminology `client_secret` | `secrets.terminologyOauth2ClientSecrets` | access to that terminology server as this client |
| The AMQP broker URL | `secrets.eventsUrl` | access to the change-event stream, which is PHI-free by design |
| The audit repository URL | `secrets.auditFhirFeedUrl` | the ability to read or forge audit records at the repository |
| S3 credentials | `secrets.multimediaSecretAccessKey` (with `secrets.multimediaAccessKeyId`) | offloaded `DV_MULTIMEDIA` blobs, which **are PHI** |
| The viewer's OIDC client secret | `viewer.existingSecret`, when the viewer is enabled | the ability to impersonate the viewer at your identity provider |

The rendered `ferroehr.toml` itself is **not** in that list, and that is worth
stating because it used to be: while a Basic user's hash had nowhere secure to go,
configuring one moved the whole configuration file into a Secret. Every credential
the server models now has either a `*_file` sibling or a Secret-borne environment
route, so no key takes that branch today; the chart keeps it only so that a
secret key added upstream tomorrow fails safe instead of landing in a ConfigMap.

The first row is the one that matters most, and it has a property worth naming:
**the DSN is a bearer credential.** Any process that can read it can use it, from
anywhere the database is reachable: there is no binding to the workload that was
issued it. That is the same gap named under [service
mesh](hardening-network-policy.md#service-mesh-a-recorded-decision) (workload
identity), reached from the other direction, and the mitigation is the same: a
credential that is short-lived and issued to a workload identity rather than a
long-lived password in an object.

**Enable encryption at rest** with an
[`EncryptionConfiguration`](https://kubernetes.io/docs/tasks/administer-cluster/encrypt-data/)
on every API server. Prefer a KMS provider over `aescbc`/`secretbox` with a local
key: a key sitting in a file on the control-plane node is protected by the same
boundary as the etcd data it encrypts. On a managed cluster this is usually one
setting (envelope encryption with the provider's KMS); check whether it is on,
because it generally is not by default. Existing Secrets are only re-encrypted
when rewritten, so follow the documented `kubectl get secrets --all-namespaces -o
json | kubectl replace -f -` step, or the encryption applies to new writes only.

**Or remove them from etcd entirely.** Every secret this chart carries has a
`*_file` route or an `existingSecret` route, so no code change is needed to source
them from a secret manager:

- **A CSI driver** ([Secrets Store CSI
  Driver](https://secrets-store-csi-driver.sigs.k8s.io/) with the Vault, AWS,
  Azure or GCP provider) mounts the value as a file. Point `extraVolumes` and
  `extraVolumeMounts` at it and set the matching `*_file` configuration key;
  `config.auth.oidc.hmac_secret_file`, `config.signing.key_passphrase_file`,
  `config.multimedia.secret_access_key_file`, a terminology client's
  `client_secret_file`. Nothing reaches a Kubernetes Secret at all.
- **An operator that syncs into a Secret** (External Secrets Operator, Vault
  Agent Injector) still lands in etcd, so it buys rotation rather than removal:
  worth having, but pair it with encryption at rest.
- **The DSN** is mounted too, from `database.existingSecret`, so a
  CSI-provided Secret carries it like any other. What a mount does **not** fix is
  that the DSN remains a *bearer* credential: cloud IAM database authentication is
  the route that removes the standing password (the DSN then carries a short-lived
  token), and `serviceAccount.annotations` exists for the IRSA or Workload
  Identity binding that needs.
- **`config.audit.fhir_feed.url`** is the only credential-bearing key with no
  `*_file` sibling, so it is the one value still passed as an environment
  variable.

**Finding exposed secrets** is already covered in CI rather than left to an
operator: Trivy's `secret` scanner runs over the whole tree and over every
published image, so a credential committed to the repository or baked into a layer
fails a job. The deliberate development credentials were checked against it and are
not flagged, so nothing is exempted to make it pass.

The volume-versus-environment half of this control (mounting secrets as read-only
files rather than passing them as environment variables) is
[covered on the Kubernetes page](kubernetes.md#secrets-and-mounted-config) and is
already the chart's behaviour for every secret whose configuration key has a
`*_file` sibling.

## Runtime detection on a shell-less image

**Operator's tooling** (Falco, Tetragon, or a managed equivalent), but the signals
are unusually high-confidence here, and that is what makes it worth more than a
recommendation.

The runtime image is **distroless and shell-less**. There is no `sh`, no `bash`,
no `curl`, no package manager, and the container runs one process. So the usual
heuristics stop being heuristics:

| Signal | Why it is unambiguous for this image |
|---|---|
| **Any `execve` of anything other than `/usr/local/bin/ferroehr`** | the image contains no other executable to run, so a second process means one arrived from outside |
| **A shell process in the container** | impossible under normal operation; there is no shell in the filesystem |
| **A write outside `/tmp`** | the root filesystem is read-only and `/tmp` is the only declared writable mount |
| **An outbound connection to anything not in the egress table** | the [inventory](hardening-network-policy.md#egress-deny-by-default-and-what-it-breaks) is complete and small |
| **Any attempt to load a kernel module, mount, or change namespaces** | the capability bounding set is empty, so these cannot succeed, but an *attempt* is still a signal |

A starter Falco rule set exploiting exactly that:

```yaml
- macro: ferroehr_container
  condition: container.image.repository endswith "/ferroehr"

- rule: FerroEHR unexpected process
  desc: Any process other than the server binary in a shell-less image
  condition: spawned_process and ferroehr_container and proc.exepath != "/usr/local/bin/ferroehr"
  output: Unexpected process in FerroEHR container (proc=%proc.exepath parent=%proc.pname container=%container.id)
  priority: CRITICAL

- rule: FerroEHR write outside tmp
  desc: The root filesystem is read-only; /tmp is the only writable mount
  condition: open_write and ferroehr_container and not fd.name startswith "/tmp/"
  output: Write outside /tmp in FerroEHR container (file=%fd.name proc=%proc.exepath container=%container.id)
  priority: CRITICAL

- rule: FerroEHR unexpected outbound connection
  desc: Outbound to a destination outside the configured inventory
  condition: >
    outbound and ferroehr_container and
    not fd.sport in (53) and not fd.sport in (5432, 4317, 4318, 443, 5671, 5672, 6514)
  output: Unexpected egress from FerroEHR container (dest=%fd.rip:%fd.rport proc=%proc.exepath)
  priority: WARNING
```

Tune the port list to the destinations you actually enabled; the point of the
third rule is that the list is short enough to be worth writing. The macro matches
the viewer's image too, whose only expected process is its own server binary.

**What each layer sees, and cannot.** The syscall layer sees process execution,
file writes and raw connections (a compromise of the *container*) but it has no
idea which patient's record was read, because to it every request is bytes on an
established socket. The **ATNA audit trail** sees exactly that: which subject
accessed which EHR, through which operation, under which authenticated identity:
but it is emitted *by* the application, so a compromise deep enough to control the
process can stop or falsify it. They are complementary and neither substitutes:
runtime detection is how you learn the process is not itself any more; the audit
trail is how you answer what was accessed while it still was. Forwarding audit
records **off-box** to an external repository (`config.audit.syslog`,
`config.audit.fhir_feed`) is what keeps the second answer available after the
first alarm.

## Replica deviation and the outbound inventory

**Operator's practice, from material the server already publishes:** which makes
it more actionable here than the generic advice.

Replicas of this Deployment are interchangeable: same image, same configuration,
traffic distributed by the Service. So a metric that differs *per pod* is a signal,
and the Prometheus surface is already per-pod (each pod is its own scrape target).
Worth alerting on a divergence between pods rather than on an absolute value:

| Comparison across pods | What a deviation suggests |
|---|---|
| request rate per pod | a load-balancing fault, or one pod being addressed directly, bypassing the Service |
| error ratio (5xx over total) per pod | a pod-local fault: a broken database connection, an exhausted pool, a failing dependency one pod reaches and others do not |
| tail latency per pod | a throttled pod (CPU limit), a noisy neighbour, or a degraded node |
| authentication-failure rate per pod | credential stuffing aimed at one endpoint, or a token-validation path failing on one pod (an unreachable JWKS endpoint) |
| resident memory slope per pod | a leak or an unbounded working set on one replica only |
| database pool acquire-wait per pod | that pod's pool is starved while others are not |

The management surface's `prometheus` endpoint (opened with
`config.management.endpoints.prometheus`) is the source, and
`metrics.serviceMonitor.enabled` is how an operator-managed Prometheus discovers
it. Two things worth knowing: the ATNA audit trail gives a second, independent
view (an access pattern that deviates per pod is visible there at
patient-and-operation granularity) and a pod that fails readiness leaves the
Service, so "one pod has zero traffic" can mean it is unready rather than
unreachable.

**The outbound inventory** the traffic half of this control asks for is the
[egress table](hardening-network-policy.md#egress-deny-by-default-and-what-it-breaks),
derived from the configuration tree. In the chart's default posture, read from node
conntrack on a running pod, the complete set is **two destinations**: TCP 5432 to
the database and UDP 53 to cluster DNS. Nothing else. That is what makes a
deny-by-default egress policy tractable: the base allowance is two rules, and
every addition is a named, configured endpoint rather than an open range. Compare
live traffic against the policy periodically: a connection the policy permits and
nothing makes is a rule to remove.

## Breach containment and rotating credentials

**Scaling to zero is a clinical-safety decision, and it should be made before you
need it.** `kubectl scale deploy/ferroehr --replicas=0` is the Kubernetes-native
containment action, and for this workload it means **clinical access stops
immediately**: no reads, no commits, for everyone. That is the point during a
breach, and it is also an outage of a system clinicians may be depending on at
that moment. Decide in advance who is authorized to make that call.

What scaling to zero **does**:

- stops all new requests, including whatever the attacker is doing through the API;
- leaves the database untouched: it is external, so its data, its contents and
  its own access controls are unaffected;
- preserves the pod's evidence only **partially**: scaling to zero terminates the
  pods, so anything in memory is gone. To preserve a pod for forensics, cordon its
  node and remove the pod from the Service by editing its labels instead; the
  ReplicaSet then creates a replacement while the original keeps running,
  detached from traffic.

What it does **not** do:

- **it does not undo committed data.** openEHR change control is append-only, so a
  malicious commit is a new version, not an overwrite. The prior version is still
  there and still retrievable.
- **it does not stop an attacker who has the DSN.** The database is reachable
  independently of these pods; a leaked DSN is used from anywhere the database
  admits, which is why rotating it (below), not scaling, is the containment
  action for that particular compromise.
- **it does not truncate the audit trail.** Records already written to the local
  store, or already forwarded to an external repository, survive. Records still in
  the outbox at termination are drained during the grace period, so a scale-to-zero
  loses less than an abrupt kill; forwarding to an external repository
  (`config.audit.syslog`, `config.audit.fhir_feed`) is what makes the trail
  survive the pods entirely.

### Rotating each credential

Every secret except two is a mounted file, so rotation is: update the Secret, then
**restart the pods**. The restart is not optional: configuration is read at boot,
and Kubernetes propagating a new Secret into the volume does not make a running
process re-read it:

```shell
kubectl -n ferroehr create secret generic ferroehr-db \
  --from-literal=FERROEHR__DB__URL='postgres://…new…' --dry-run=client -o yaml \
  | kubectl apply -f -
kubectl -n ferroehr rollout restart deploy/ferroehr
```

(A rotation you make through `helm upgrade` needs no explicit restart: any change
under `config`, `config.files` or `secrets` moves the `checksum/config` pod
annotation and rolls the Deployment by itself.)

| Credential | Rotation | Notes |
|---|---|---|
| Database DSN | update the Secret, then `rollout restart` | rotate the **database** password too, or the old one still works; `maxUnavailable: 0` keeps the old pods serving on the old credential until the new ones are ready, so grant both briefly or accept a gap |
| OIDC HMAC secret | update, then restart | invalidates every token signed with the old secret; prefer JWKS or discovery, where the issuer rotates for you and no secret lives here |
| Terminology `client_secret` | update, then restart | rotate at the identity provider in the same window |
| AMQP broker URLs | update, then restart | the credential is inside the URL |
| S3 credentials | update, then restart | or remove them entirely with IRSA or Workload Identity |
| Audit repository URL | update, then restart | still an environment value (no `*_file` sibling) |
| Basic user password hash | update, then restart | rotate the password and re-hash at the OWASP Argon2id floor, which the server checks at boot |
| Viewer OIDC client secret | update, then restart the viewer | rotate at the identity provider in the same window |
| **Version signing key** | **read the next section first** | rotate the signing *subkey*, not the certificate |

### Rotating the version signing key

In the default `digest` mode there is no key: the version signature is a hash of
the canonical form, recomputed at read time. Nothing to rotate, and nothing
breaks.

In `pgp` mode there are two mechanisms, and the first is the one to reach for.

**The ordinary path: rotate the signing subkey.** OpenPGP is built for this. A
certificate is a primary key plus its subkeys ([RFC 9580
§10.1](https://www.rfc-editor.org/rfc/rfc9580.html)), the primary key certifies
while a subkey signs, and rotating means issuing a *new signing subkey* on the
same certificate. The retired subkey stays in the certificate, so every version
it signed keeps verifying, with no configuration change, no second key file,
and no window where history is unreadable. It is also the only path that keeps
the primary key's identity intact; replacing a primary key discards everything
that has ever been said about it.

```console
# add a fresh signing subkey to the existing certificate
gpg --quick-add-key <FINGERPRINT> ed25519 sign 1y
# revoke or expire the previous one, then re-export BOTH halves
gpg --export-secret-keys --armor <FINGERPRINT> > signing.asc
```

Update the mounted Secret and roll the pods. The server signs with the newest
signing-capable subkey and verifies against every subkey the certificate carries,
so the switch is invisible to readers.

**The exception: a whole certificate is replaced.** A compromised primary key,
an organisational change, or migrating from a different signer means a genuinely
new certificate, and then the old one must be retained, because a stored
signature carries no key identifier and a version signature is an immutable
committed fact that cannot be re-issued. Keep the retired **public** key:

```toml
[signing]
mode = "pgp"
key_path = "/etc/ferroehr/signing.asc"                  # the new certificate
retired_key_paths = ["/etc/ferroehr/signing-2025.pub.asc"]   # public, verify-only
```

Under Helm these are `config.signing.key_path` and
`config.signing.retired_key_paths`, with both files mounted through
`config.files`.

> [!NOTE]
> Retired entries are **public** keys, which is what makes the safety property
> structural rather than a promise: no secret key is loaded for them, so a
> retired certificate can verify and can never sign again. Verification does not
> become permissive either: a signature matching neither the active certificate
> nor any retired one still fails, and tampered content still fails.

This is the same mechanism Debian uses for its archive: `debian-archive-keyring`
ships current *and* retired keys so packages signed under an older key stay
verifiable ([Debian archive keys](https://ftp-master.debian.org/keys.html)).

**What neither mechanism covers.** Key *expiry* and *revocation* are a different
problem: a strict verifier arguably should accept a signature made while the key
was valid and reject one made after, and nothing in a detached signature proves
*when* it was made. Solving that needs trusted timestamps (RFC 3161), the
approach [Sigstore](https://docs.sigstore.dev/about/security/) takes with
short-lived keys and a timestamp authority. FerroEHR does not implement it, so
treat an expired signing key as a certificate replacement and use
`config.signing.retired_key_paths`.

If you are mid-rotation and need reads to keep working before either mechanism
is in place, `config.signing.verify_on_read: warn` downgrades a verification
failure from a 5xx to a logged and metered event
(`version_signature_invalid_total{verdict="pgp_invalid"}`). That is a deliberate,
recorded reduction in an integrity guarantee, not a setting to leave on.

## Logging: two streams that are not interchangeable

**Container logging is ours, and already the right shape.** The server writes to
stdout and stderr and never to a file inside the container, which is both the
[Kubernetes logging
architecture](https://kubernetes.io/docs/concepts/cluster-administration/logging/)'s
expectation and a requirement of `readOnlyRootFilesystem: true`: there is nowhere
to write a log file. Set `config.log.format: json` (the chart's default) for a
collector; `pretty` is for a terminal.

**The distinction that matters, because getting it wrong loses the accountability
record:**

| | Application log | ATNA audit trail |
|---|---|---|
| Purpose | diagnostics: what the process is doing | **accountability**: who accessed which patient's record |
| Destination | stdout/stderr → node → your collector | its own store in the database, plus optional forwarding to an external repository |
| Format | JSON lines, ours | DICOM PS3.15 plus FHIR `AuditEvent` (IHE BALP), standardised |
| Retrieval | your log tool | the ITI-81 FHIR `AuditEvent` search endpoint |
| Retention | your collector's policy | `config.audit.store.retention_days` (`0` keeps forever) |
| May it be sampled or dropped? | **yes**, it is diagnostics | **no** |

> [!IMPORTANT]
> **Do not treat the audit trail as "logs".** A collector configured to sample a
> noisy stream, or to drop under volume, is a reasonable policy for diagnostics and
> a compliance failure for the audit trail: it silently discards the record of who
> read which patient's data. The two travel by different paths precisely so that
> one can be lossy: the audit trail does not go through stdout at all. If you also
> ship audit records to your log platform for convenience, that copy is a
> convenience, not the record.

The audit trail's own failure behaviour is configurable, and the default is worth
knowing rather than inheriting: **`config.audit.fail_mode` defaults to `open`**, so
an operation whose audit record cannot be written still proceeds, and the failure
is metered rather than refused. `closed` answers `503` instead, the stronger
compliance posture, and one that turns an audit outage into a clinical outage.
Which is correct depends on whether your regulatory position can tolerate an
unaudited access more or less than a refused one; it is a policy choice either way,
and the shipped default chooses availability.

**Cluster API audit logging is the operator's.** Enable it on the API server with
an audit policy (`None` / `Metadata` / `Request` / `RequestResponse` per rule).
`Metadata` for most resources with `Request`-level detail for Secret and RBAC
changes is a reasonable starting shape. Two things worth alerting on specifically:
**authorization failures** (`Forbidden` responses, a principal probing what it can
reach), and any read of Secrets in this namespace by a principal that is not the
kubelet, since that is what reading the DSN looks like from the API side.

**Kubernetes `Events` are a third source**, and distinct from both: they are the
cluster's account of what happened to your objects, they expire (typically an
hour), and they are where this chart's failures show up first:
`Readiness probe failed: HTTP probe failed with statuscode: 503` when a dependency
is down, `FailedCreate … violates PodSecurity "restricted:latest"` when a pod spec
regresses under enforcement, `Unhealthy` and `Killing` during a rollout. Check
`kubectl get events --sort-by=.lastTimestamp` before the application log when a pod
will not start: the reason is usually there, and it is usually not in the log,
because the container never ran.

## On a managed control plane

On EKS, GKE, AKS or an equivalent, **several controls in this audit stop being
yours**: you cannot set API-server flags, reach etcd, or configure kubelet
authentication. In exchange you inherit the provider's defaults, which may be
stronger or weaker than this sheet assumes, and which you should verify rather
than assume.

| Control | On a managed cluster |
|---|---|
| Host hardening, node OS patching | provider's, though **node images and upgrades are usually still yours to trigger** |
| API-server flags, authorization mode | provider's (RBAC is on by default everywhere mainstream) |
| etcd access + encryption at rest | provider's, but **envelope encryption with your own KMS key is usually opt-in**, and it is the control [our Secrets need](#secrets-at-rest-and-what-ours-contain) |
| Kubelet authentication | provider's |
| Control-plane audit logging | provider's to enable, **often off or short-retention by default**, and usually billed |
| User namespaces (`hostUsers: false`) | depends on the **node image's** runtime version, which is why the pod fails loudly rather than downgrading |
| Pod Security Admission | yours (namespace labels) |
| NetworkPolicy | yours to write; **enforcement depends on the CNI** |
| Everything the chart sets | unchanged: it is a workload |

**Check the CNI before relying on the shipped NetworkPolicy.** This is the item
that varies most and fails most silently: a NetworkPolicy on a cluster whose
network plugin does not enforce it is an object the API accepts, stores and
displays, with no effect and no warning. Provider defaults differ, versions change
them, and some require enabling enforcement at cluster creation, which cannot be
changed afterwards on some platforms. Do not read your provider's documentation
and conclude; **test it**:

```shell
kubectl create namespace ferroehr-probe
kubectl -n ferroehr-probe run probe --image=busybox:1.37 --restart=Never --command -- sleep 600
kubectl -n ferroehr-probe exec probe -- nc -w3 -z <ferroehr-pod-ip> 5432   # must fail
kubectl -n ferroehr-probe exec probe -- nc -w3 -z <ferroehr-pod-ip> 8080   # must succeed
```

If the first command succeeds, the policy is decoration and every claim in this
section that rests on it is void for your cluster. This is also the reason the
project's own deployment probe harness declares NetworkPolicy enforcement as
something it does **not** exercise: a green result on one cluster's CNI would say
nothing about yours.

Provider audit tooling exists: for EKS,
[hardeneks](https://github.com/aws-samples/hardeneks) is the commonly cited one.
**We have not run it**, so it is named as a starting point rather than a
recommendation: treat its output as input to the same ownership question this
section asks, not as a verdict.
