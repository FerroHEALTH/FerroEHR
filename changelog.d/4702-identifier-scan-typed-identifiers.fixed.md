- The identifier scan (`[privacy.identifier_scan]`) no longer refuses a
  `DV_IDENTIFIER` whose `type` declares an item rather than a person, such as
  a laboratory `filler_order_number` that passes the NHS Number check by
  chance. A `DV_IDENTIFIER` in `FEEDER_AUDIT.originating_system_item_ids` or
  `feeder_system_item_ids` is scanned only when its `type` names a person
  identifier. A `type` naming a person (`patient`, `NHS`, `BSN`, `social
  security` and similar) keeps the value scanned in every slot, and free text
  and untyped identifiers in clinical content are scanned as before. The
  default rule set is unchanged; a deployment that knows its jurisdictions
  still narrows `rules`, so a Dutch one with `rules = ["nl-bsn"]` never claims
  an NHS Number.
