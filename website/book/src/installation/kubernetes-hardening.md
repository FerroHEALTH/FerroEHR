# Cluster hardening

The [Kubernetes chapter](kubernetes.md) documents the chart. This section is the
other half: an audit of this deployment against the [OWASP Kubernetes Security
Cheat
Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Kubernetes_Security_Cheat_Sheet.html),
split by **who can actually apply each control**.

That split is the point. A workload chart controls its pod security context, its
resource bounds, its NetworkPolicy, its ServiceAccount and how it consumes
secrets. It cannot patch a node, set an API-server flag, configure etcd,
authenticate a kubelet, or install an admission controller. For those, the honest
deliverable is not a setting but a statement of **what you owe and what happens if
you do not**, because a deployment that inherits an unchecked assumption is not
secured by the chart's own hardening.

> [!IMPORTANT]
> Several controls here are ones **no application-level hardening can compensate
> for**. If the kubelet accepts anonymous requests, or anything untrusted can read
> etcd, then every control this project ships (non-root, read-only rootfs, dropped
> capabilities, RBAC, the access evaluator, the audit trail) is bypassable, because the attacker
> is beneath the layer they operate in. Those are marked where they appear.

Claims about what the chart renders are checked against the chart's own render
gates. Claims about what a cluster then *applies* come from the deployment probe
harness, which reads each answer from the layer that actually decides it (the
container runtime's own spec for the security context, the API server for
admission, the EndpointSlice for readiness) and which declares in its own record
what it did **not** exercise, so silence is never read as coverage.

## The five pages

| Page | Covers |
|---|---|
| [The cluster: hosts, control plane, access](hardening-cluster.md) | node and OS hardening, the supported version window, rolling upgrades, advisories, etcd, the port surface, cluster API access, cluster RBAC, kubelet authentication |
| [Images: build, provenance, scanning](hardening-supply-chain.md) | the distroless image and what it costs you during an incident, keyless signing and the identity to trust, copyable admission policies, scanning after release, the full supply-chain map |
| [The workload: security context & admission](hardening-workload.md) | the applied security context, user namespaces, AppArmor, Restricted-profile compliance versus enforcement, sandboxing, kernel modules |
| [Namespaces, network & policy](hardening-network-policy.md) | namespace scoping, one instance per organisation, the service-mesh decision, which admission engine, resource bounds in four layers, deny-by-default egress |
| [Secrets, detection & response](hardening-detection-response.md) | Secrets at rest and exactly what ours contain, runtime detection on a shell-less image, per-replica alerting, breach containment and credential rotation, the two log streams, managed control planes |

## The ownership map

| Cheat-sheet control | Owner | Where |
|---|---|---|
| Host hardening, OS patching, node firewall | operator | [cluster](hardening-cluster.md#host-hardening-and-the-version-window) |
| Supported Kubernetes version window | operator (chart states the floor) | [cluster](hardening-cluster.md#host-hardening-and-the-version-window) |
| Rolling upgrades rather than mutating containers | **chart** | [cluster](hardening-cluster.md#upgrades-roll-they-do-not-replace) |
| Kubernetes security advisories | operator | [cluster](hardening-cluster.md#security-advisories) |
| Kubernetes Dashboard | operator (we ship none) | [cluster](hardening-cluster.md#the-dashboard-we-do-not-ship) |
| etcd access + encryption at rest | operator | [cluster](hardening-cluster.md#etcd-and-what-our-secrets-contain) |
| Control-plane and kubelet ports | operator | [cluster](hardening-cluster.md#ports-theirs-and-ours) |
| The workload's own port surface | **chart** | [cluster](hardening-cluster.md#ports-theirs-and-ours) |
| Cluster API access control, MFA | operator | [cluster](hardening-cluster.md#cluster-api-access) |
| Cluster RBAC (`Node,RBAC`, `NodeRestriction`) | operator | [cluster](hardening-cluster.md#cluster-rbac-and-why-this-chart-needs-none) |
| The workload's own RBAC | **chart**, deliberately none | [cluster](hardening-cluster.md#cluster-rbac-and-why-this-chart-needs-none) |
| Kubelet authentication/authorization | operator | [cluster](hardening-cluster.md#kubelet-access) |
| Minimal, current, authorized images | **chart/CI** | [images](hardening-supply-chain.md#the-build-phase-and-what-distroless-costs) |
| Image provenance at admission | operator (we publish the attestations) | [images](hardening-supply-chain.md#image-provenance-at-admission) |
| Continuous scanning of published images | **CI** | [images](hardening-supply-chain.md#continuous-scanning-of-published-images) |
| Supply chain | **CI**, with two gaps | [images](hardening-supply-chain.md#the-supply-chain-map) |
| Pod/container security context | **chart** | [workload](hardening-workload.md#the-security-context-and-what-keeps-it-true) |
| Pod Security Admission enforcement | operator (one `kubectl label`) | [workload](hardening-workload.md#pod-security-admission-complying-versus-being-refused) |
| Container sandboxing | neither; a recorded decision | [workload](hardening-workload.md#sandboxing-is-not-a-substitute-for-instance-separation) |
| Kernel-module loading | **satisfied by the chart** | [workload](hardening-workload.md#kernel-modules-already-impossible) |
| Namespace isolation | **chart** (namespace-scoped by construction) | [network](hardening-network-policy.md#namespaces-and-one-instance-per-organisation) |
| Service mesh | neither; a recorded decision | [network](hardening-network-policy.md#service-mesh-a-recorded-decision) |
| Centralized policy management | operator, for admission only | [network](hardening-network-policy.md#centralized-policy-and-which-engine) |
| Container resource bounds | **chart** | [network](hardening-network-policy.md#resource-bounds-four-layers) |
| Namespace `ResourceQuota`/`LimitRange` | operator | [network](hardening-network-policy.md#resource-bounds-four-layers) |
| Egress restriction | **chart** (mechanism) + operator (destinations) | [network](hardening-network-policy.md#egress-deny-by-default-and-what-it-breaks) |
| Secrets encrypted at rest | operator | [secrets](hardening-detection-response.md#secrets-at-rest-and-what-ours-contain) |
| Runtime/syscall detection | operator (unusually cheap here) | [secrets](hardening-detection-response.md#runtime-detection-on-a-shell-less-image) |
| Replica behavioural deviation | operator (from metrics we publish) | [secrets](hardening-detection-response.md#replica-deviation-and-the-outbound-inventory) |
| Breach containment + credential rotation | operator (procedure is ours) | [secrets](hardening-detection-response.md#breach-containment-and-rotating-credentials) |
| Cluster API audit logging | operator | [secrets](hardening-detection-response.md#logging-two-streams-that-are-not-interchangeable) |
| Container logging | **chart/app** | [secrets](hardening-detection-response.md#logging-two-streams-that-are-not-interchangeable) |
| Managed control plane | provider | [secrets](hardening-detection-response.md#on-a-managed-control-plane) |

## Final thoughts: the three practices, checked

The cheat sheet closes with three practices rather than controls. Checking them
against what this project actually ships, rather than claiming them:

**"Embed security into the container lifecycle as early as possible":
evidenced.** Security here is CI, not a review checklist: the chart's render gate
parses every rendered object and asserts the Restricted profile per container,
the golden renders pin the exact bytes, the secret-leak gate refuses a credential
that would reach a ConfigMap, the image scanners run at build and again on a
schedule against the published tags, `cargo deny` runs on every change, and
zizmor and CodeQL read the workflows themselves. Each of those refuses a merge
rather than filing a comment, and each of the chart-side ones has been watched to
fail deliberately; a gate nobody has seen fail is a gate nobody knows works.

**"Use Kubernetes-native controls to reduce operational risk": evidenced.** The
chart's controls are the platform's own: a NetworkPolicy rather than an in-app
firewall, a security context and the Restricted profile rather than a hardening
sidecar, resource limits rather than in-process throttling alone, probes rather
than an external watchdog, a user namespace rather than a trusted UID, and
[no service mesh](hardening-network-policy.md#service-mesh-a-recorded-decision)
because what one would add is either already provided or not needed at this
shape. The one place we did *not* take a native control is Pod Security
Admission, and that is because [labelling a namespace is not a chart's
call](hardening-workload.md#pod-security-admission-complying-versus-being-refused),
stated as the operator's rather than quietly skipped.

**"Leverage the context Kubernetes provides to prioritize remediation": partly,
and here is the honest version.** The generic form of this practice is to rank
findings by whether the affected code is reachable in your deployment. This
project's answer is the **OpenVEX documents**: when a scanner reports advisories
in a privilege-dropping helper that opens no socket and parses no untrusted
input, the response is a machine-readable `not_affected` statement with a
controlled-vocabulary justification and an `impact_statement` a reader can
check, not a silenced ignore list, and not a rebuild that fixes nothing.

That mechanism has a cost, and stating it is what makes the claim honest: **a VEX
statement is a claim about today's binary, and it must be re-checked on every
base-image bump.** When upstream rebuilds that helper, statements about it become
obsolete, and a stale `not_affected` is worse than no VEX at all: it is an
argument that has quietly stopped being true while still suppressing its finding.
The scheduled scan is what surfaces a finding whose statement no longer matches,
and the re-check itself is a human obligation, not an automated one.
