- Two query failures still answer `500`, and their bodies now carry curated
  text only. A stored query whose record holds no query text reads "the
  stored query has no query text" without echoing its name, and a
  terminology server failure while a `TERMINOLOGY('expand', …)` operand is
  resolved reads "the terminology service failed while the query was being
  planned" instead of the upstream message.
