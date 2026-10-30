- The partitioned tables `node`, `version` and `vo_attestation` of the
  clinical and party stores never had planner statistics of their own,
  since autovacuum does not analyze a partitioned table, so the planner
  estimated joins over them without any (PostgreSQL 18 §24.1.3). The
  server now runs `ANALYZE ONLY` on them, and on `ehr` while autovacuum
  has never analyzed it, after applying the migrations, after
  `load_ehrs`, at boot and every hour. The role it runs as needs the
  `MAINTAIN` privilege on those tables; without it the server logs the
  tables it could not analyze. The population `CONTAINS` of an archetype
  now enters through `idx_node_hot_arch_subsume` (3 703 buffers to
  1 745 on the storage bench's seed).
