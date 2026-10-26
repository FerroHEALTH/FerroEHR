- **A BASE 1.3 `TERMINOLOGY_CODE` keeps its `terminology_id` in XML**
  (#4076). ITS-XML 1.0.2 types `TRANSLATION_DETAILS.language` and the other
  slots that hold one as `CODE_PHRASE`, whose `terminology_id` carries a
  `<value>` child; the codec wrote and read bare text instead, so an
  XSD-valid document read with an empty `terminology_id`. It now writes and
  reads the `TERMINOLOGY_ID` shape.
