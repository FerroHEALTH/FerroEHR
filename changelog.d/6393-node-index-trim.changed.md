- A composition commit writes fewer node index entries:
  `idx_node_hot_ehr_type` leaves out the data-structure classes and
  `idx_node_hot_value_search` the rows with no search text, and
  `idx_node_hot_context_start` and `idx_vo_head_ehr_template`, which no read
  used, are gone. AQL answers are unchanged.
