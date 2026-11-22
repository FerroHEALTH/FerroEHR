- Two directory creates sent at once to one EHR answered `500` for the one
  that lost; it now answers `409`, as a second create always did. A
  directory update or delete that raced an `EHR_STATUS` update making the EHR
  not modifiable could still commit; it now answers `409`.
