- Representatives in the patient portal (#5446). A parent, a guardian or a
  proxy signs in through a `patients` issuer, as the RFC 8693 `act` of the
  patient's token or under their own login naming the patient's EHR, and
  reads the patient's record while their `PERSON` holds a live
  representation of the patient: a `PARTY_RELATIONSHIP` carrying
  `ferroehr-operations::represents`, in any archetype. Who represents whom
  and until when, including the age and legal-capacity rules of national
  law, is the representations and `time_validity` the deployment records;
  FerroEHR builds in no country's rules. The patient authorises a proxy with
  `POST /demographic/party_relationship` and revokes it with a `DELETE`,
  effective on the next request, and every access is recorded bound to the
  representative, naming the subject.
