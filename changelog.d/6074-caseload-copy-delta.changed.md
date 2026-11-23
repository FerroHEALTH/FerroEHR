- A care relationship commit no longer drops the care team's cached caseload on every replica.
  The replica applies the changed patients' relationships and record subjects, which it reads
  with the facts of the parties the commit stamped, to the caseloads it holds. The next query
  narrowed to the team's caseload therefore no longer reads the whole team again, which took
  43.8 ms for a team of 21,000 patients under an ADT feed.
