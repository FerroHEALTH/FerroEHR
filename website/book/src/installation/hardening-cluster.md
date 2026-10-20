# The cluster: hosts, control plane, access

The controls on this page are almost all the **operator's**, and several are ones
no application-level hardening can compensate for. Read them as what you owe the
CDR running on top, not as background.

<!-- toc -->

## Host hardening and the version window

**Operator's.** Keep the node OS patched, hardened and firewalled; a workload chart
cannot reach any of it. The cheat sheet's list applies unchanged.

The part worth stating precisely is the **version window**. Upstream Kubernetes
maintains release branches for the **three most recent minor releases**, each
receiving roughly a year of patch support
([kubernetes.io/releases](https://kubernetes.io/releases/)). A cluster below that
window receives **no security backports at all**: a published CVE in the API
server or kubelet simply stays open on it.

The chart's `kubeVersion: ">=1.36.0-0"` is a **compatibility** floor, not a
statement about that window. It sits at the window's newest release because 1.36
is where the newest field the chart renders (`hostUsers`, user namespaces)
became stable, which is what lets every field it renders apply unconditionally
instead of being gated into silence on the clusters that most needed it. The cost
is real and worth naming: an operator one minor behind cannot install this chart.
See [Beyond Restricted: the user
namespace](hardening-workload.md#beyond-restricted-the-user-namespace) for what
that buys.

A version gate is a silent absence; a floor is a loud refusal. For a workload
holding PHI the loud one is correct, so the chart declares the floor and refuses
below it rather than installing with a safety property quietly inapplicable.

If you run outside the supported window, you have accepted that the platform
beneath this CDR is unpatched, and no setting in `values.yaml` changes that.

## Upgrades roll, they do not replace

**Chart's.** The cheat sheet asks that a new version arrive by rolling update
rather than by mutating a running container. The chart sets the strategy
explicitly rather than inheriting the API server's percentages:

```yaml
strategy:
  type: RollingUpdate
  rollingUpdate:
    maxSurge: 1
    maxUnavailable: 0        # served capacity never drops
```

`maxUnavailable: 0` means a replacement pod must pass its readiness probe before
an old one is removed. At the default two replicas this is identical to what
Kubernetes computed from `25%` (it rounds down to zero); above two replicas the
percentage default would have taken a pod out of service, and this does not. The
trade-off is real and fails in the safe direction: on a cluster with no spare
scheduling capacity a `maxUnavailable: 0` rollout **stalls visibly** instead of
proceeding at reduced capacity.

Observed on a live cluster rather than argued: a probe pod drove a long run of
sequential requests at the Service through a full `helm upgrade`, with
`Killing`/`Started` events for both replicas inside the window, and recorded no
failed request, while the ReplicaSet history showed the old revision scaled to
zero as the new one scaled up (a roll, not a recreate) and the Deployment reported
`Available=True (MinimumReplicasAvailable)` throughout.

## Security advisories

**Operator's, with a decision recorded on our side.**

You must follow
[`kubernetes-announce`](https://groups.google.com/g/kubernetes-announce) and the
[official advisory
feed](https://kubernetes.io/docs/reference/issues-security/official-cve-feed/).
Kubernetes CVEs are announced there, and nothing in this project will tell you
about them.

**This project does not track Kubernetes platform advisories, deliberately.** The
decision, so it is not mistaken for an oversight: we run no cluster and cannot act
on a node or control-plane CVE, and a watcher that opened issues we could only
close as "the operator's" would be noise that trains people to ignore it. What we
*do* track is what we ship: dependency advisories on every change, our own
container images on a schedule, and the openEHR specifications through release
watchers.

A vulnerability in Kubernetes itself is reported to
[Kubernetes](https://kubernetes.io/docs/reference/issues-security/security/), not
to this project. A vulnerability in FerroEHR, including in the chart, comes to
us, through the security policy published with the source.

## The dashboard we do not ship

**Operator's.** This chart installs no Kubernetes Dashboard, and nothing in it
depends on one. If you install one, the cheat sheet's conditions apply: never
expose it publicly, give it a limited-privilege ServiceAccount, and put an
authenticating reverse proxy in front of it if it must be reachable at all.

This section stays because **the same reasoning applies to two surfaces that
_are_ ours**, and an operator hardening
"the dashboard" should find them here:

- **`/management/*`:** the ops-introspection surface (`info`, `prometheus`,
  `metrics`, `env`, `loggers`, `flamegraph`). It is a privileged read onto the
  deployment: `env` renders the effective configuration and `flamegraph` profiles
  the live process. The chart ships the master switch on and **every endpoint
  `off`**, so nothing is exposed until you name an endpoint and a level. Set
  `config.management.port` to move the whole surface onto its own listener so it
  is never reachable on the clinical API port.
- **The viewer:** a separate image with its own Deployment, which this
  chart can render but leaves off (`viewer.enabled`). It consumes the CDR
  strictly over the REST API and holds no database credential, but it is a
  privileged UI and belongs behind the same authenticating edge you would put in
  front of a dashboard, which is what `viewer.auth.oidc.enabled` and
  `viewer.ingress.enabled` are for.

## etcd and what our secrets contain

**Operator's**, and this is one of the controls that cannot be compensated for.

The cheat sheet's requirements stand: mutual TLS between the API server and etcd,
etcd reachable from nothing else, and separate instances or ACLs to bound what a
component can read.

What makes it concrete for this deployment: **anything that can read etcd can read
every Secret in the cluster**, and this release's Secrets are not incidental. They
hold the **database DSN** (the credential that reaches patient data) plus the
version-signing passphrase, any Basic user's Argon2id hash,
and any terminology-server client secret. So "etcd is a cluster concern" is true
and insufficient: for this workload etcd is the confidentiality boundary of the
credentials that reach PHI. The full inventory is [what our Secrets
contain](hardening-detection-response.md#secrets-at-rest-and-what-ours-contain).

Two mitigations you can apply without touching etcd's network posture:

1. **Encryption at rest for Secrets:** not on by default in Kubernetes. See
   Kubernetes'
   [encryption-at-rest
   configuration](https://kubernetes.io/docs/tasks/administer-cluster/encrypt-data/),
   which is a cluster-level setting an operator applies once.
2. **An external secret manager**, which removes the credential from etcd
   entirely: every secret this chart carries has a `*_file` route or an
   `existingSecret` route, so the value can arrive from a CSI driver or an
   operator-synced Secret rather than from chart values.

## Ports, theirs and ours

**Operator's, for the cluster's ports.** Block untrusted access to the
control-plane ports `6443` (API server), `2379-2380` (etcd), `10250-10257`
(kubelet and controller/scheduler), and the worker ports `10248-10250`. An
exposed `10250` is the kubelet case below.

**Ours, for the workload's ports**, and in the default posture the surface is one
port:

| Port | Serves | Who should reach it |
|---|---|---|
| `8080` (`service.port`) | the openEHR REST API, the always-on `/health` family, and `/management/*` when `config.management.port` is unset | your ingress controller or gateway, not the internet directly |
| `config.management.port` (unset by default) | `/management/*` on its own listener when set | operators and your Prometheus, never clinical clients |
| `3000` (`viewer.service.port`) | the viewer, only when `viewer.enabled` | your ingress controller, in front of an authenticating edge |

Read off the running pod rather than the template, from the listening sockets in
the container's own network namespace: the default posture binds one port and
nothing else:

```text
LISTENING TCP ports in the container netns: [8080]
```

The shipped NetworkPolicy narrows inbound traffic to that port list. Whether the
narrowing is **enforced** is a property of your CNI, not of the object: on a
cluster whose network plugin does not implement NetworkPolicy the object is
accepted, stored and displayed with no effect and no warning. Verify it the way
[the managed-cluster
section](hardening-detection-response.md#on-a-managed-control-plane) shows:
attempt a connection the policy should refuse, from a pod in another namespace,
and require it to fail.

Two limits stated in full under [Namespaces, network & policy
§Ingress](hardening-network-policy.md#ingress-ports-are-narrowed-sources-are-yours)
and repeated because they matter here: with `networkPolicy.ingressFrom` empty the
rule carries no `from` and therefore admits **every** source, including other
namespaces (only the port list is narrowed in that state; set
`networkPolicy.ingressAllowAll: false` to have the chart refuse to render that
state at all, and the same pair exists for the viewer under
`viewer.networkPolicy`); and a NetworkPolicy is only as real as the CNI that
implements it.

## Cluster API access

**Operator's.** Control access to the Kubernetes API: authenticate, then
authorize, and deny by default.

- **Recommended routes:** OIDC, a managed-IAM integration, or user
  impersonation, with **MFA** on the identities that can reach the cluster API.
- **Not suitable for production:** static token files, long-lived X.509 client
  certificates, and service-account tokens used as human credentials. They cannot
  be revoked individually, they do not expire usefully, and they carry no second
  factor.

Worth drawing explicitly, because it saves a translation: **this server implements
the same shape.** Authentication then authorization, deny by default, OIDC
preferred over long-lived credentials, and a missing credential distinguished from
a refused one (`401` with a challenge versus `403`). The
[Security chapter](../security.md) is the detail. An operator who understands why
a static token file is a poor cluster credential already understands why
`config.auth.basic` is a machine and bootstrap mechanism here and a
`config.auth.issuer` is the production one for people.

## Cluster RBAC, and why this chart needs none

**Operator's, for the cluster:** run the API server with
`--authorization-mode=Node,RBAC` and enable the `NodeRestriction` admission
plugin, so a compromised kubelet cannot edit objects belonging to other nodes.

**Ours, and it is an absence on purpose.** The chart creates a ServiceAccount and
**no Role, RoleBinding, ClusterRole or ClusterRoleBinding at all**, with
`serviceAccount.automountServiceAccountToken: false`. That is not an omission to
be tidied up later: the workload never calls the Kubernetes API, so it needs no
permissions, and it is not given a token with which to try. The same holds for the
viewer's own ServiceAccount when that workload is enabled. Checkable on a
live release:

```shell
helm get manifest ferroehr | grep -cE '^kind: (Role|RoleBinding|ClusterRole|ClusterRoleBinding)'
kubectl -n ferroehr get role,rolebinding
```

The first prints zero; the second reports no resources. And because the token is
not mounted, no service-account token file exists under the pod's volumes on the
node.

**If you are reviewing this chart and reaching for a Role to add: don't.** The
correct fix for a future feature that genuinely needs the Kubernetes API is a
Role enumerating exactly the verbs and resources it needs, plus turning the token
mount back on for that ServiceAccount alone, not a broad grant added
speculatively.

## Kubelet access

**Operator's**, and the second control no application hardening can compensate
for.

Run every kubelet with `--anonymous-auth=false` and
`--authorization-mode=Webhook` so its HTTPS endpoint is not open. Left open, that
endpoint permits **arbitrary command execution in any container on the node**.

For this deployment, spelled out: an attacker reaching an unauthenticated kubelet
gets a process-level foothold in a running CDR: the ability to read the database
DSN out of the mounted secret files, to read patient data straight from memory,
and to do so **beneath** the layer where authentication, RBAC, ABAC and the ATNA
audit trail operate, so none of them see it and none of them can stop it.
Non-root, a read-only root filesystem, an empty capability set and a private user
namespace raise the cost of what happens next; they do not prevent the entry.

This is a cluster-configuration control, and it is worth confirming rather than
assuming; an exposed `10250` is a routine finding in real clusters.
