- The storage-bench lane measures the baseline and the candidate on one
  runner in eight interleaved rounds each and gives two verdicts. The plan
  verdict fails an operation whose median pages per call rose more than 10 %
  and one page above the baseline's highest round, whatever its time. The
  time verdict divides each round by its machine factor, so a slow spell of
  the runner no longer reads as the code, and fails an operation whose median
  p50 or p99 rose more than 15 % with every candidate round slower than every
  baseline round, or a candidate build that ran more than 15 % slower across
  the board in every round. A latency is no longer compared with a record
  measured on another runner.
