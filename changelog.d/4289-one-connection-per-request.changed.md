- A REST request checks a connection out of each database pool once and keeps
  it until it answers, where it took one for each statement: two to four
  checkouts for a read of an EHR, an EHR_STATUS or a COMPOSITION, each waiting
  in the pool's queue under load and each paying the pool's release round
  trip. At 48 concurrent clients and a pool of 10, throughput rose by about
  18 % and the median latency fell by about 13 %.
