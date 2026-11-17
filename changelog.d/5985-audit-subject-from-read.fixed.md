- The access record of a read now names the EHR subject that the request's
  own read found in the store. Before, after a subject changed, a read record
  could keep naming the old subject the audit memo still held. The read no
  longer looks the subject up a second time, and concurrent lookups of one
  EHR share a single read.
