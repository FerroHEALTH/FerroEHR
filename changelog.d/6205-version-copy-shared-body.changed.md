- A COMPOSITION read served from the replica's version copy no longer copies
  the stored body: the copy and the response share one text, so a copy hit of
  the 128 kB IPS composition allocates about 125 KiB less and holds about
  125 KiB less at its peak, and a fill about 140 KiB less. The copy's 64 MiB
  bound now weighs each entry by what it holds in memory, its key and the
  cache's own bookkeeping, counted without allocating. The bytes served are
  unchanged.
