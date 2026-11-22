- A COMPOSITION create, update and delete and a CONTRIBUTION commit no longer
  read the policy revision before they are decided: the write's first
  statement returns the policy and definition revisions, and a write decided
  on a policy copy behind them is refused before anything is written and run
  once more behind the up-front confirm. `access_policy_reruns_total` counts
  those runs too. Each of those commits sends one statement fewer.
