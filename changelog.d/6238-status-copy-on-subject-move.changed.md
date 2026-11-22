- An `EHR_STATUS` update drops the server's copy of the EHR's access settings
  only when it moves the EHR's subject, so an update that changes the name,
  `other_details` or `is_queryable` sends 4 statements where it sent 5.
