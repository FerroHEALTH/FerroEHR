# Selector immutability + disjointness, over the array of rendered documents.
#
# Emits one line per violation and nothing when clean, so the caller decides the
# exit status from the output. Ported from the Python this replaced; the
# assertions are unchanged.

["app.kubernetes.io/instance", "app.kubernetes.io/name"] as $expect
| . as $docs
| [ $docs[]
    | select(.kind == "Deployment")
    # The FerroPIX chart dependency's selector is that chart's own contract.
    | select((.metadata.labels // {})["helm.sh/chart"] // "" | startswith("ferropix-") | not)
    | { name:   .metadata.name,
        labels: (.spec.template.metadata.labels // {}),
        keys:   ((.spec.selector.matchLabels // {}) | keys) } ] as $deps
# Every pod a release puts in the namespace, whatever workload declares it.
| [ $docs[]
    | select(.kind == "Deployment" or .kind == "StatefulSet" or .kind == "DaemonSet"
             or .kind == "ReplicaSet" or .kind == "Job" or .kind == "CronJob")
    | { kind:   .kind,
        name:   .metadata.name,
        labels: ( if .kind == "CronJob"
                  then (.spec.jobTemplate.spec.template.metadata.labels // {})
                  else (.spec.template.metadata.labels // {}) end ) } ] as $pods
| (
    [ $deps[]
      | select(.keys != $expect)
      | "\(.name): selector.matchLabels is \(.keys), expected \($expect) — this field is IMMUTABLE; changing it breaks helm upgrade on every existing release" ]
  )
  +
  (
    # A Service/PDB selector is a SUBSET match, so a selector naming only the
    # labels two workloads share silently selects both. Key sets alone cannot
    # catch this: after the CDR/viewer fix both Deployments have the SAME
    # selector keys and differ only in the name's value.
    #
    # EVERY workload carrying a pod template is a candidate, not just the
    # Deployments: a Job's pod is selected for as long as it exists, and the
    # migration hook exists during exactly the install and upgrade a Service
    # and a PodDisruptionBudget are being read (#3197). A CronJob's template
    # sits one level deeper, under jobTemplate.
    [ $docs[]
      | select(.kind == "Service" or .kind == "PodDisruptionBudget")
      | . as $d
      | ( ($d.spec.selector // {})
          | if (type == "object" and has("matchLabels")) then .matchLabels else . end ) as $sel
      | select(($sel | type) == "object" and ($sel | length) > 0)
      | ( [ $pods[]
            | . as $pod
            | select([ $sel | to_entries[] | ($pod.labels[.key] == .value) ] | all)
            | "\($pod.kind)/\($pod.name)" ] ) as $hit
      | select(($hit | length) > 1)
      | "\($d.kind)/\($d.metadata.name): selector \($sel | tostring) matches \($hit) — a subset match selects EVERY one of them; give each workload its own app.kubernetes.io/name rather than a shared name plus a component" ]
  )
| .[]
