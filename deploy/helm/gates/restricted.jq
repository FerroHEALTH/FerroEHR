# Pod Security `restricted` compliance, plus the availability and isolation
# properties that live in the same walk, over the array of rendered documents.
#
# Emits one line per violation and nothing when clean. Ported from the Python
# this replaced; every assertion is unchanged.

[ "configMap", "csi", "downwardAPI", "emptyDir", "ephemeral",
  "persistentVolumeClaim", "projected", "secret" ] as $allowed_volumes
| . as $docs
# A CronJob's pod sits one level deeper, under jobTemplate. Reading only
# .spec.template would skip it silently — and a workload this gate cannot see
# is a workload nothing checks, which is the failure §3 of the Helm rules
# describes.
| [ $docs[]
    | select(.kind == "Deployment" or .kind == "StatefulSet"
             or .kind == "DaemonSet" or .kind == "Job" or .kind == "CronJob")
    | { kind: .kind, name: .metadata.name,
        # A workload of the FerroPIX chart dependency is that chart's own; the
        # two properties below that this chart adds to the restricted profile
        # are held to this chart's workloads alone.
        dependency: ((.metadata.labels // {})["helm.sh/chart"] // "" | startswith("ferropix-")),
        replicas: .spec.replicas,
        strategy: (.spec.strategy // {}),
        template: (if .kind == "CronJob"
                   then .spec.jobTemplate.spec.template
                   else .spec.template end) }
    | select(.template != null)
    | { kind: .kind, name: .name, dependency: .dependency,
        replicas: .replicas,
        strategy: .strategy,
        pod: (.template.spec // {}) } ] as $workloads
| ( [ $workloads[]
      | . as $w
      | ($w.pod.securityContext // {}) as $sc
      | (
          ( if ($sc.runAsNonRoot != true)
            then ["\($w.kind)/\($w.name): pod runAsNonRoot must be true"] else [] end )
        + ( if ((($sc.seccompProfile // {}).type) as $t
                | $t != "RuntimeDefault" and $t != "Localhost")
            then ["\($w.kind)/\($w.name): pod seccompProfile.type must be RuntimeDefault or Localhost"] else [] end )
        # The kubelet's per-Service link variables land in the reserved
        # FERROEHR_ namespace and the strict env sweep then refuses to boot, so
        # losing this makes every install crash-loop.
        + ( if ($w.dependency | not) and ($w.pod.enableServiceLinks != false)
            then ["\($w.kind)/\($w.name): enableServiceLinks must be false"] else [] end )
        + ( [ ($w.pod.volumes // [])[]
              | . as $v
              | ($v | keys | map(select(. != "name")))[]
              | select(. as $t | ($allowed_volumes | index($t)) == null)
              | "\($w.kind)/\($w.name): volume \($v.name) type \(.) is outside the restricted set" ] )
        + ( [ (($w.pod.containers // []) + ($w.pod.initContainers // []))[]
              | . as $c
              | ($c.securityContext // {}) as $csc
              | (
                  ( if ($csc.allowPrivilegeEscalation != false)
                    then ["\($w.kind)/\($w.name)/\($c.name): allowPrivilegeEscalation must be false"] else [] end )
                + ( if ($csc.runAsNonRoot != true and $sc.runAsNonRoot != true)
                    then ["\($w.kind)/\($w.name)/\($c.name): runAsNonRoot must be true"] else [] end )
                + ( if ($csc.readOnlyRootFilesystem != true)
                    then ["\($w.kind)/\($w.name)/\($c.name): readOnlyRootFilesystem must be true"] else [] end )
                + ( if ((($csc.capabilities // {}).drop // []) | index("ALL")) == null
                    then ["\($w.kind)/\($w.name)/\($c.name): capabilities.drop must include ALL"] else [] end )
                + ( if ((($csc.capabilities // {}).add // []) | length) > 0
                    then ["\($w.kind)/\($w.name)/\($c.name): capabilities.add must be empty, got \((($csc.capabilities // {}).add))"] else [] end )
                )[] ] )
        )[] ] )
  +
  # Availability, not security: a Deployment asking for more than one replica
  # with nothing telling the scheduler to spread them can put every replica on
  # one node. An ABSENT `replicas` counts as multi-replica — the chart omits the
  # field when autoscaling is on, so reading it as the API default of 1 would
  # exempt precisely the elastic workload that most needs spreading.
  ( [ $workloads[]
      | select(.kind == "Deployment")
      | select(.replicas == null or .replicas > 1)
      | select((.pod.topologySpreadConstraints // []) == [] and (.pod.affinity // {}) == {})
      | "Deployment/\(.name): multi-replica with neither topologySpreadConstraints nor affinity — every replica may land on one node" ] )
  +
  # Rollout feasibility, in the same walk. A rolling update surges a new pod
  # before the old one goes away (Deployment docs, Max Surge: the default is
  # 25%, rounded UP), and a ReadWriteOnce PersistentVolume "can be mounted as
  # read-write by a single node" (PV docs, Access Modes) — so a Deployment
  # mounting a claim rolls forward only if the surge pod happens to land on the
  # node holding the volume, and otherwise hangs Pending while reporting a
  # progressing update. Derived from the render rather than tabulated: any
  # Deployment that grows a claim is covered the day it does.
  ( [ $workloads[]
      | select(.kind == "Deployment")
      | select([ (.pod.volumes // [])[] | select(has("persistentVolumeClaim")) ] | length > 0)
      | select((.strategy.type // "RollingUpdate") != "Recreate")
      | "Deployment/\(.name): mounts a PersistentVolumeClaim under strategy \(.strategy.type // "RollingUpdate") — the surge pod cannot attach a ReadWriteOnce volume held by the outgoing pod on another node, so the rollout hangs; render strategy.type: Recreate (or ReadOnlyMany storage)" ] )
  +
  ( if ($workloads | length) == 0
    then ["no pod-bearing workload found — the gate would pass vacuously"] else [] end )
  +
  ( if ([ $docs[] | select(.kind == "NetworkPolicy") ] | length) == 0
    then ["no NetworkPolicy in the render"] else [] end )
  +
  # Pod-level ISOLATION, recorded per workload rather than asserted per workload.
  # These are not restricted-profile controls, so the property that matters is
  # not "every pod sets X" but that every pod of one release AGREES. A release
  # whose viewer shares the host user namespace while its server does not has a
  # posture nobody can state.
  ( [ $workloads[]
      | select(.dependency | not)
      | { k: "\(.kind)/\(.name)",
          v: [ (if .pod.hostUsers == null then true else .pod.hostUsers end),
               ((.pod.securityContext // {}).supplementalGroupsPolicy) ] } ] as $iso
    | if (($iso | map(.v) | unique | length) > 1)
      then ["pod isolation differs across workloads of one release: "
            + (($iso | sort_by(.k) | map("\(.k) hostUsers=\(.v[0]) supplementalGroupsPolicy=\(.v[1])")) | join(", "))]
      else [] end )
| .[]
