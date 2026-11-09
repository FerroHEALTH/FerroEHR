- **Breaking: a `patients` token that names an actor is refused.** With
  `ferrohealth-smart` 0.15, a `patients` issuer reads a token whose top-level
  `act` (RFC 8693 §4.1) names someone other than the patient only when its
  `[[auth.issuer]]` table sets `act_represents_patient = true`; without it the
  token is refused `401`, where v5.6 admitted it as the patient's own access.
  With it, FerroEHR refuses the token `403`
  (`refused-patient-representative`), because it binds no party to someone
  acting for a patient, and the access log records the refusal.
