- The server's resident memory no longer climbs under sustained load until the
  container's memory limit kills it. The `ferroehr` binary now allocates through
  mimalloc instead of glibc `malloc`, whose per-thread arenas kept freed memory
  and never gave it back. On a load test of 10 000 mixed jobs at 48 workers the
  peak resident set fell from 2.3 to 3.2 GB (growing run after run) to about
  600 MB, and throughput rose by about a third.
