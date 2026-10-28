- The three audit indexes on `recorded_at` ascend, so their pages fill instead
  of splitting half full on every insert; the newest-first ITI-81 search scans
  them backward.
