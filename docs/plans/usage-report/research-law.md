# Product telemetry ("alive" report to a Cadasto-operated collector): what the vendored corpus says

> Superseded in part (2026-10-05): the GDPR recitals (`docs/law/eu/gdpr/oj.html`), the ePrivacy Directive (`docs/law/eu/eprivacy/`) and the Telecommunicatiewet (`docs/law/nl/telecommunicatiewet/`) are now vendored (#3580, #3581). Where this report says they are absent, the #3580 adjudication on the issue is the current reading.

Read 2026-10-05, first-hand, from `docs/law/` at the pins in `docs/law/README.md`. No legal advice: this reports what the text says, who it binds and what software would have to be able to do. The word "compliant" is not used.

Proposal under review: each self-hosted FerroEHR instance periodically sends, by default (opt-out), a random instance id, the product version, the licence grant in force (embedded non-commercial grant, or an installed commercial grant whose licence id names the licensee organisation), uptime, and aggregated performance data (latency histograms per endpoint, slow-query statistics, PostgreSQL version, CPU/memory class, coarse data-volume buckets) to a collector Cadasto B.V. (NL) runs on Hetzner (EU), shown in Grafana. No patient data, query literals or identifiers. A boot log line says telemetry is on and how to switch it off.

---

## 0. Corpus coverage for this question (read this first)

| Item | In the corpus? | Consequence |
|---|---|---|
| GDPR articles | Yes: `docs/law/eu/gdpr/text.html`, consolidation CELEX `02016R0679-20160504` | Cited below |
| **GDPR recitals 14, 26, 30 (and every other recital)** | **No.** The vendored consolidation has no recitals: 0 `id="rct_…"` anchors in `docs/law/eu/gdpr/text.html`, against 130 in the CRA, 115 in EHDS and 144 in NIS2 | **For recitals 14 (legal persons), 26 (identifiability, "means reasonably likely", anonymous data) and 30 (IP addresses and online identifiers), the corpus has no text.** The answer below rests on the articles alone, mainly Art. 1, 2 and 4(1). The recital wording has to be read from the OJ text (32016R0679), which is not vendored |
| Directive 2002/58/EC (ePrivacy), Art. 5(3) | **No.** It appears only as a cross-reference: GDPR Art. 21(5) and Art. 95, CRA recital 72, EHDS Art. 1(3), NIS2 Art. 2(12) and recitals 14, 92, 106, 108, Art. 46 | The text is silent, so this needs separate research (see Q10) |
| Telecommunicatiewet art. 11.7a | **No.** No file under `docs/law/nl/` and no hit for "Telecommunicatiewet" or "11.7a" anywhere in the tree | The text is silent, so this needs separate research (see Q10) |
| NEN 7510 / 7512 / 7513 | Provenance records only (`docs/law/nl/nen-75xx/PROVENANCE.md`), no text. NEN sells the standards under copyright | The corpus cannot say what NEN 7510-2 asks of a vendor's outbound connection |
| NIS2 national transpositions (NL, DE) | **No.** Only the Directive is vendored | Art. 21 binds Member States, so the concrete entity duties are in the national acts, which are not in the corpus |
| German Landeskrankenhausgesetze and Land data-protection acts | **No** (README §Germany: federal law only) | Silent |
| Swiss cantonal data-protection law (public hospitals) | **No** (only federal DSG/DSV/EPDG/EPDV) | Silent |
| CRA | OJ text `32024R2847`, unconsolidated | Does not carry the EHDS Art. 104 amendments (see En-route finding 3) |

---

## Q1. Is the installation report personal data at all?

### GDPR Art. 4(1): the definition
- **(a) Obligation/test:** information is personal data only if it relates to an identified or identifiable **natural** person. Identifiability can come from "an identifier such as … an identification number, location data, an online identifier" or from "factors specific to the … economic … identity of that natural person".
- **(b) Quote:** "‘personal data’ means any information relating to an identified or identifiable natural person (‘data subject’); an identifiable natural person is one who can be identified, directly or indirectly, in particular by reference to an identifier such as a name, an identification number, location data, an online identifier or to one or more factors specific to the physical, physiological, genetic, mental, economic, cultural or social identity of that natural person;"
- **(c) Citation:** `docs/law/eu/gdpr/text.html Art. 4(1)`; scope `Art. 1(1)`: "This Regulation lays down rules relating to the protection of natural persons with regard to the processing of personal data…"; `Art. 1(2)`: "…fundamental rights and freedoms of natural persons…"; `Art. 2(1)`: "This Regulation applies to the processing of personal data wholly or partly by automated means…".
- **(d) Binds:** this is a definition. Whoever processes personal data is bound, which here means Cadasto as the collector operator.
- **(e) Closed or open:** the identifiers are an open list ("in particular", "such as").
- **(f) Silence and ambiguity:**
  - **Legal persons.** No article says in terms that data about a legal person fall outside the Regulation. That statement is recital 14, which is not vendored. The articles give only the natural-person wording of Art. 1(1)/(2) and 4(1).
  - **IP addresses.** No article names IP addresses. "Online identifier" appears only as an example of an identifier through which a *natural person* can be identified. Recital 30 is not vendored.
  - **The identifiability test.** The "means reasonably likely to be used" test is recital 26, which is not vendored. The articles contain no test for when someone becomes "identifiable".
  - **EDPB guidance.** The only text in the corpus that addresses IP addresses is non-binding: EDPB Guidelines 01/2025, Example 7 ("Risk reduction as a factor in the balancing of interests…"), printed p. 41, `docs/law/eu/edpb-guidelines-01-2025-pseudonymisation/guidelines.pdf`. It treats "identifying information (IP addresses, access tokens, login credentials)" as data to be removed or transformed. The context there is the traffic of an online service's users, who are natural persons. Its PROVENANCE says guidelines "bind nobody".

### Applied to the proposed fields (reading of Art. 4(1); no article settles it)
| Field | Hospital or other legal-person operator | Sole practitioner (natural person, e.g. a GP in a one-person practice) |
|---|---|---|
| Random instance id | Identifies an installation, not a natural person. Whether it is personal data depends on whether it can be linked to a natural person. The text gives no test (recital 26 is not vendored) | Linked to the practitioner as soon as anything in the report, or held by the collector, identifies the practitioner. It is then an "identification number"/"online identifier" relating to a natural person |
| Source IP at the collector | The organisation's egress address. No article addresses it | Can be the practitioner's own connection. The text is silent on the threshold |
| Licence grant type (non-commercial / commercial) | Information about the organisation | Information about the practitioner's licensing situation, which is an economic factor (Art. 4(1) "economic … identity") |
| **Licence id naming the licensee** | Names a legal person, so not personal data under the Art. 4(1) wording (the recital-14 confirmation is not vendored). If the licence record also names a contact person, that contact is personal data | **Names a natural person directly ("identified").** Every field joined to it becomes "information relating to an identified natural person": version, uptime, latency, data-volume bucket (practice size, an economic factor) |
| Version, uptime, PG version, CPU/memory class, latency histograms, slow-query statistics, data-volume buckets | Information about a system. Not personal data unless linkable to a natural person | Personal data of the practitioner once linked through the licence id or the IP |
| Anything carrying a patient-level identifier (an `ehr_id` in a concrete URL path, a subject id in a query literal) | **Personal data of a patient**: an "identification number" relating to a natural person, Art. 4(1). In an EHR it is also "data concerning health" under Art. 4(15) (see Q2) | Same |

### DSG (CH), for contrast: here the text is explicit
- **Quote (authentic):** Art. 2 Abs. 1: "Dieses Gesetz gilt für die Bearbeitung von Personendaten natürlicher Personen durch: a. private Personen; b. Bundesorgane." Art. 5 lit. a: "Personendaten: alle Angaben, die sich auf eine bestimmte oder bestimmbare natürliche Person beziehen;"
- **English (Fedlex, non-binding):** "This Act applies to the processing of personal data of natural persons by: a. private persons; b. federal bodies." / "personal data means any information relating to an identified or identifiable natural person;"
- **Citation:** `docs/law/ch/fadp/text-de.html Art. 2 Abs. 1`, `Art. 5 lit. a`.
- **Result:** data about a legal person (a Swiss hospital AG or foundation) are outside the DSG on its own wording. A Swiss sole practitioner's licence id is a Personendatum.

**Answer to Q1:** for a hospital (legal person) whose report carries no natural-person identifier, the articles give no basis to call the report personal data. The corpus cannot confirm this through recitals 14/26/30, because they are not vendored. For a sole practitioner, a licence id that names the practitioner makes the whole report personal data of an identified natural person. The source IP and a stable instance id can do the same without the licence id; the corpus contains no binding text on that threshold.

---

## Q2. Roles: does anything risk making Cadasto a processor, or a recipient of health data?

### GDPR Art. 4(7), 4(8), 4(9), 4(10)
- **Quotes:**
  - Art. 4(7): "‘controller’ means the natural or legal person … which, alone or jointly with others, determines the purposes and means of the processing of personal data;"
  - Art. 4(8): "‘processor’ means a natural or legal person … which processes personal data on behalf of the controller;"
  - Art. 4(9): "‘recipient’ means a natural or legal person … to which the personal data are disclosed, whether a third party or not."
  - Art. 4(10): "‘third party’ means a natural or legal person … other than the data subject, controller, processor and persons who, under the direct authority of the controller or processor, are authorised to process personal data;"
- **Citation:** `docs/law/eu/gdpr/text.html Art. 4(7)–(10)`.
- **Reading:**
  - Cadasto fixes the purpose (product telemetry) and the means (the client code and the collector). For any personal data in the reports, that is the controller position of Art. 4(7).
  - Cadasto does not process telemetry "on behalf of" the hospital, so the Art. 4(8)/Art. 28 processor relationship does not arise from telemetry.
  - If personal data of the hospital's own processing ever reached Cadasto (a patient id in an endpoint path, a literal in a slow query, a staff user name), Cadasto would be a **recipient** (4(9)) and a **third party** (4(10)) of data the hospital disclosed "by transmission" (Art. 4(2)).
- **Art. 28(10):** "…if a processor infringes this Regulation by determining the purposes and means of processing, the processor shall be considered to be a controller in respect of that processing." This is relevant if Cadasto also holds a support or hosting contract as a processor. Using data obtained under that contract for telemetry purposes would make Cadasto a controller of that processing. `docs/law/eu/gdpr/text.html Art. 28(10)`.
- **Art. 29:** "The processor and any person acting under the authority of the controller or of the processor, who has access to personal data, shall not process those data except on instructions from the controller…" `Art. 29`.

### Health data: GDPR Art. 4(15), Art. 9(1)–(3)
- **Quotes:**
  - Art. 4(15): "‘data concerning health’ means personal data related to the physical or mental health of a natural person, including the provision of health care services, which reveal information about his or her health status;"
  - Art. 9(1): "Processing of … data concerning health … shall be prohibited."
  - Art. 9(3) ties ground (h) to professional secrecy: "…when those data are processed by or under the responsibility of a professional subject to the obligation of professional secrecy…"
- **Citation:** `docs/law/eu/gdpr/text.html Art. 4(15)`, `Art. 9(1)`, `Art. 9(2)(a)–(j)`, `Art. 9(3)`.
- **Binds:** every controller. Here that is the hospital (disclosing) and Cadasto (receiving).
- **Closed or open:** the Art. 9(2) grounds are a closed list. Read on their face, none of (a)–(j) describes a software vendor's product-improvement purpose:
  - (h) is "medical diagnosis, the provision of health or social care…";
  - (i) is public health "on the basis of Union or Member State law";
  - (j) is research or statistics "based on Union or Member State law".
- **Consequence:** patient-level data of any kind in the telemetry stream (including pseudonymous `ehr_id`s, which are "identification numbers" under Art. 4(1)) would put the hospital in the position of disclosing health data to a third party, and Cadasto in the position of receiving it, with no Art. 9(2) ground visible in the text. The text gives no exception for small quantities or for accidental inclusion.

### Joint controllership (Art. 26)
- **Quote:** "Where two or more controllers jointly determine the purposes and means of processing, they shall be joint controllers." `docs/law/eu/gdpr/text.html Art. 26(1)`.
- **Silence:** the text does not say whether an operator that leaves a vendor's default enabled thereby "jointly determines" the purposes and means of that processing. Open point.
- **Where it matters:** Art. 6(1), second subparagraph: "Point (f) of the first subparagraph shall not apply to processing carried out by public authorities in the performance of their tasks." If a public hospital were a (joint) controller of a transmission that contains personal data, it could not use point (f) for its own part. Keeping the report free of personal data takes the hospital out of this question.

### Cadasto's own processor (Hetzner)
- **Art. 28(1):** "…the controller shall use only processors providing sufficient guarantees…"
- **Art. 28(3):** "Processing by a processor shall be governed by a contract or other legal act…"
- **Binds:** Cadasto, as controller of whatever personal data the collector holds. The collector is in the EU, so the corpus raises no Chapter V transfer question for EU instances. For CH, see Q9.

**Answer to Q2:** as designed (no patient data, no literals, no identifiers), nothing in the text makes Cadasto the hospital's processor. Cadasto is the controller of any personal data in the telemetry and owes the controller duties below. The one path to "recipient of health data" is leakage of a patient-level value: a concrete URL path, a query literal or text, an error message, a template or composition id. If that happens, the text gives the hospital no visible Art. 9(2) ground for the disclosure.

---

## Q3. Lawful basis, right to object, transparency

### Art. 6(1)(f): legitimate interest
- **(a) Test:** three conditions. (1) A legitimate interest of the controller (Cadasto). (2) Processing "necessary" for it. (3) Not overridden by the data subject's interests or rights.
- **(b) Quote:** "processing is necessary for the purposes of the legitimate interests pursued by the controller or by a third party, except where such interests are overridden by the interests or fundamental rights and freedoms of the data subject which require protection of personal data, in particular where the data subject is a child." Second subparagraph: "Point (f) of the first subparagraph shall not apply to processing carried out by public authorities in the performance of their tasks."
- **(c) Citation:** `docs/law/eu/gdpr/text.html Art. 6(1)(f)` and second subparagraph.
- **(d) Binds:** the controller (Cadasto).
- **(e)/(f) Silence:** the articles contain no list of balancing factors. Recital 47 ("reasonable expectations", "relationship with the controller") is not vendored. The only factor list in the articles is Art. 6(4), and it concerns compatibility of a *further* purpose:
  - Art. 6(4)(a): "any link between the purposes…"
  - Art. 6(4)(b): "the context … in particular regarding the relationship between data subjects and the controller"
  - Art. 6(4)(c): "the nature of the personal data"
  - Art. 6(4)(d): "the possible consequences…"
  - Art. 6(4)(e): "the existence of appropriate safeguards, which may include encryption or pseudonymisation."

  It becomes relevant if Cadasto later uses telemetry for a new purpose, for example checking whether a non-commercial grant is being used commercially.

### Consent is not available with a default-on design
- **Quote:** Art. 4(11): "‘consent’ … means any freely given, specific, informed and unambiguous indication of the data subject's wishes by which he or she, by a statement or by a clear affirmative action, signifies agreement…"
- **Citation:** `docs/law/eu/gdpr/text.html Art. 4(11)`; Art. 7(3): "It shall be as easy to withdraw as to give consent."
- **Reading:** silence or inaction (leaving the default on) is not "a statement or … a clear affirmative action". A default-on design therefore cannot rest on Art. 6(1)(a) for any personal data it carries. Point (f) is the only basis the proposal can use.

### Art. 21: right to object; Art. 17(1)(c)
- **Quotes:**
  - Art. 21(1): "The data subject shall have the right to object, on grounds relating to his or her particular situation, at any time to processing of personal data concerning him or her which is based on point (e) or (f) of Article 6(1)… The controller shall no longer process the personal data unless the controller demonstrates compelling legitimate grounds for the processing which override the interests, rights and freedoms of the data subject or for the establishment, exercise or defence of legal claims."
  - Art. 21(4): "At the latest at the time of the first communication with the data subject, the right referred to in paragraphs 1 and 2 shall be explicitly brought to the attention of the data subject and shall be presented clearly and separately from any other information."
  - Art. 21(5): "In the context of the use of information society services, and notwithstanding Directive 2002/58/EC, the data subject may exercise his or her right to object by automated means using technical specifications."
  - Art. 17(1)(c): erasure where "the data subject objects to the processing pursuant to Article 21(1) and there are no overriding legitimate grounds for the processing".
- **Citation:** `docs/law/eu/gdpr/text.html Art. 21(1), 21(4), 21(5)`, `Art. 17(1)(c)`.
- **Binds:** the controller (Cadasto).
- **Does an opt-out switch satisfy it?** The text does not settle this. It does show four things:
  1. A switch that stops sending, with no reasons asked, goes further than Art. 21(1) requires (Art. 21(1) lets the controller argue "compelling legitimate grounds"; the switch does not).
  2. Art. 21(1)'s consequence is that the controller "shall no longer process the personal data", and Art. 17(1)(c) adds erasure. Both reach data **already held** at the collector, and a client-side switch does not touch those. A route to stop processing and erase the history for an instance is the part a switch alone does not cover.
  3. Art. 21(4) requires the right to be presented "explicitly", "clearly and separately from any other information", "at the latest at the time of the first communication with the data subject". The text is silent on whether a server log line is a "communication with the data subject".
  4. Art. 21(5) addresses objection by automated means only "in the context of the use of information society services". Whether an on-premises CDR's configuration flag falls under that is not settled by the text.
- **The right belongs to the data subject**, not to the operating organisation. For a hospital with no natural person in the report there is no data subject who could object. The switch then serves the operator's choice (NIS2/contract), not Art. 21. For a sole practitioner, the person holding the switch is the data subject.

### Art. 12–14: transparency
- **Quotes:**
  - Art. 12(1): "The controller shall take appropriate measures to provide any information referred to in Articles 13 and 14 … in a concise, transparent, intelligible and easily accessible form, using clear and plain language… The information shall be provided in writing, or by other means, including, where appropriate, by electronic means."
  - Art. 13(1): "…the controller shall, at the time when personal data are obtained, provide the data subject with all of the following information: (a) the identity and the contact details of the controller…; (b) the contact details of the data protection officer, where applicable; (c) the purposes … as well as the legal basis…; (d) where the processing is based on point (f) of Article 6(1), the legitimate interests pursued…; (e) the recipients or categories of recipients…; (f) [third-country transfer]."
  - Art. 13(2) adds: "(a) the period for which the personal data will be stored, or if that is not possible, the criteria used to determine that period; (b) the existence of the right to … object…; (d) the right to lodge a complaint with a supervisory authority…"
  - Art. 14(1)(d) adds "the categories of personal data concerned" where the data were not obtained from the data subject. Art. 14(3)(a) sets timing: "within a reasonable period after obtaining the personal data, but at the latest within one month".
- **Citation:** `docs/law/eu/gdpr/text.html Art. 12(1)`, `Art. 13(1)–(2)`, `Art. 14(1)–(3)`.
- **Binds:** the controller (Cadasto). The duty runs to **the data subject**.
- **(e) Closed or open:** Art. 13(1)/(2) and 14(1)/(2) are closed lists of required items ("all of the following").
- **Is a boot log line plus documentation enough?** The text names no medium beyond "in writing, or by other means, including, where appropriate, by electronic means". A single log line cannot carry the full Art. 13 list. Read on its face, the text allows a line that points to a notice which does carry the list. The text is silent on layered notices; the guidance on them is not vendored. Art. 13 is "at the time when personal data are obtained": the information has to exist and be reachable no later than the first report. The text does not say whether a record arriving from the data subject's own server counts as "collected from the data subject" (Art. 13) or "not obtained from the data subject" (Art. 14).
- **Who must be informed:** under GDPR, only a data subject: the sole practitioner, or any natural person whose data are in a report. **GDPR imposes no duty to inform an operator organisation or its sysadmins as such.** Sysadmins become data subjects only if their data are in the report, and the proposal includes none. The duty to inform the *user organisation* comes from the CRA (Annex II, Q6) and from contract, not from GDPR.

### DSG (CH): justification model and information
- **No general lawful-basis requirement for private persons.** A processing is unlawful only if it violates personality and is not justified:
  - Art. 30 Abs. 1: "Wer Personendaten bearbeitet, darf die Persönlichkeit der betroffenen Personen nicht widerrechtlich verletzen."
  - Art. 31 Abs. 1: "Eine Persönlichkeitsverletzung ist widerrechtlich, wenn sie nicht durch Einwilligung der betroffenen Person, durch ein überwiegendes privates oder öffentliches Interesse oder durch Gesetz gerechtfertigt ist."
  - **Art. 30 Abs. 2 lit. b**: a violation "liegt insbesondere vor, wenn: … b. Personendaten entgegen der ausdrücklichen Willenserklärung der betroffenen Person bearbeitet werden;"
  - English (non-binding): "personal data are processed contrary to the express wishes of the data subject".
- **Information:** Art. 19 Abs. 1: "Der Verantwortliche informiert die betroffene Person angemessen über die Beschaffung von Personendaten…" Abs. 2 minimum: "a. die Identität und die Kontaktdaten des Verantwortlichen; b. den Bearbeitungszweck; c. gegebenenfalls die Empfängerinnen und Empfänger…"
- **Citation:** `docs/law/ch/fadp/text-de.html Art. 19 Abs. 1–5`, `Art. 30 Abs. 1–2`, `Art. 31 Abs. 1`.
- **Binds:** the Verantwortlicher (Cadasto) and anyone who processes.
- **Reading:** once a Swiss data subject has said no (by switch or by request), continuing to process their data, including the data already held, is a listed personality violation.

---

## Q4. GDPR Art. 25(2): data protection by default

- **(a) Obligation:** by default, only personal data **necessary for each specific purpose** are processed. This covers amount collected, extent of processing, storage period and accessibility. By default the data are not made accessible to an indefinite number of persons without the individual's intervention.
- **(b) Quote:** "The controller shall implement appropriate technical and organisational measures for ensuring that, by default, only personal data which are necessary for each specific purpose of the processing are processed. That obligation applies to the amount of personal data collected, the extent of their processing, the period of their storage and their accessibility. In particular, such measures shall ensure that by default personal data are not made accessible without the individual's intervention to an indefinite number of natural persons."
- **(c) Citation:** `docs/law/eu/gdpr/text.html Art. 25(2)`; Art. 25(1) for design: "…appropriate technical and organisational measures, such as pseudonymisation, which are designed to implement data-protection principles, such as data minimisation…"
- **(d) Binds:** the **controller**. For the telemetry processing, that is Cadasto. The hospital's Art. 25 duties cover its own processing of its own data subjects, which the report (as proposed) does not touch.
- **(e) Closed or open:** Art. 25(1)'s measures are a menu ("such as pseudonymisation"). Art. 25(2) is an outcome duty with four named dimensions.
- **(f) If the report is personal data** (sole practitioner, or a linkable IP/instance id):
  - The text measures the default against "each specific purpose", not against "off". It does not say in terms that a processing operation must be off by default. It says that what runs by default must be limited to what each purpose needs.
  - A default-on report is therefore not excluded by the wording. Each default-on field has to be necessary for a stated purpose. A field that serves a separate, optional purpose (for example the licensee-naming licence id, if licence auditing is not a declared necessary purpose) sits badly with "by default".
  - The last sentence bears directly on the Grafana dashboard: no public or anonymous access.
  - Whether the purpose itself justifies processing at all is decided under Art. 6(1)(f) (Q3), not by Art. 25(2).
- **If the report is not personal data:** Art. 25 does not apply (Art. 2(1): the Regulation applies to "the processing of personal data"). Minimisation of non-personal data then comes from the CRA (Annex I Part I (2)(g), "data, personal or other", Q6).
- **DSG equivalent:**
  - Art. 7 Abs. 3: "Der Verantwortliche ist verpflichtet, mittels geeigneter Voreinstellungen sicherzustellen, dass die Bearbeitung der Personendaten auf das für den Verwendungszweck nötige Mindestmass beschränkt ist, soweit die betroffene Person nicht etwas anderes bestimmt."
  - English (non-binding): "…by means of suitable default settings that the processing of personal data is limited to the minimum required for the purpose intended, unless the data subject specifies otherwise."
  - Citation: `docs/law/ch/fadp/text-de.html Art. 7 Abs. 3`.
  - Binds: the Verantwortlicher (Cadasto).
  - Same structure as GDPR: the default is measured against the purpose, and the data subject can deviate.

---

## Q5. Minimisation and storage limitation at the collector

- **GDPR Art. 5(1)(c):** "adequate, relevant and limited to what is necessary in relation to the purposes for which they are processed (‘data minimisation’);"
- **Art. 5(1)(e):** "kept in a form which permits identification of data subjects for no longer than is necessary for the purposes for which the personal data are processed; personal data may be stored for longer periods insofar as the personal data will be processed solely for … statistical purposes in accordance with Article 89(1) subject to implementation of the appropriate technical and organisational measures…"
- **Art. 5(1)(b):** "collected for specified, explicit and legitimate purposes and not further processed in a manner that is incompatible with those purposes…"
- **Art. 5(2):** "The controller shall be responsible for, and be able to demonstrate compliance with, paragraph 1 (‘accountability’)."
- **Citation:** `docs/law/eu/gdpr/text.html Art. 5(1)(b), (c), (e), 5(2)`.
- **Binds:** the controller (Cadasto, for the collector).
- **Testable statements for the collector:**
  1. **Source IP.** The collector receives it at the network layer whether or not it wants it. Recording it (in application storage, reverse-proxy access logs, Hetzner load-balancer logs, Grafana) is "processing" under Art. 4(2): "collection, recording, … storage". If no declared purpose needs it, Art. 5(1)(c) points to not recording it. If abuse defence needs it, Art. 5(1)(e) points to a short, stated retention.
  2. **Retention.** A defined period or criteria must exist anyway, because Art. 13(2)(a)/14(2)(a) require telling the data subject "the period for which the personal data will be stored, or if that is not possible, the criteria used to determine that period".
  3. **Purposes stated up front** (Art. 5(1)(b)). A later use (licence enforcement, sales outreach to non-commercial users) is a new purpose and goes through the Art. 6(4) compatibility test.
  4. **Records of processing.** Art. 30(5) exempts organisations under 250 employees "unless the processing … is likely to result in a risk…, the processing is not occasional, or the processing includes special categories". Periodic telemetry is "not occasional" on its face, so if it carries personal data, the Art. 30(1) record is not exempted. `docs/law/eu/gdpr/text.html Art. 30(5)`.
  5. **Security of the collector:** Art. 32(1). "(a) the pseudonymisation and encryption of personal data; (b) the ability to ensure the ongoing confidentiality, integrity, availability and resilience…" These are listed "inter alia as appropriate", a menu, not a checklist.
- **DSG:**
  - Art. 6 Abs. 2: "…verhältnismässig sein."
  - Abs. 3: "Personendaten dürfen nur zu einem bestimmten und für die betroffene Person erkennbaren Zweck beschafft werden…"
  - Abs. 4: "Sie werden vernichtet oder anonymisiert, sobald sie zum Zweck der Bearbeitung nicht mehr erforderlich sind."
  - Citation: `docs/law/ch/fadp/text-de.html Art. 6 Abs. 2–4`. English (non-binding) Abs. 4: "They shall be destroyed or anonymised as soon as they are no longer required for the purpose of processing."
- **Silence:** the corpus fixes no retention number for telemetry. The Begz/Besluit bewaartermijn logging period concerns NEN 7513 access logging of patient records, not vendor telemetry, and does not transfer.

---

## Q6. CRA (Regulation (EU) 2024/2847)

### Scope and dates (read first)
- **Art. 2(1):** "This Regulation applies to products with digital elements made available on the market, the intended purpose or reasonably foreseeable use of which includes a direct or indirect logical or physical data connection to a device or network."
- **Art. 2(2)(a):** "does not apply to products with digital elements to which the following Union legal acts apply: (a) Regulation (EU) 2017/745". This is the MDR exclusion; it applies only if FerroEHR is an MDR device, which is a separate open question.
- **Art. 3(22):** "‘making available on the market’ means the supply … in the course of a commercial activity, whether in return for payment or free of charge". Recital 15 lists among the signs of commercial activity "requiring as a condition for use the processing of personal data for reasons other than exclusively for improving the security, compatibility or interoperability of the software". An opt-out design is not a "condition for use". The commercial grants already make the supply commercial.
- **Art. 71(2):** "This Regulation shall apply from 11 December 2027. However, Article 14 shall apply from 11 September 2026…" Article 14 reporting is therefore already in application on the date of this report.
- **Art. 69(2):** products placed on the market before 11 December 2027 are subject to the Regulation "only if, from that date, those products are subject to a substantial modification". Art. 69(3): "the obligations laid down in Article 14 shall apply to all products … placed on the market before 11 December 2027".
- **EHDS Art. 104** inserts CRA Art. 32(5a): "Manufacturers of products with digital elements that are classified as EHR systems under Regulation (EU) 2025/327 … shall demonstrate conformity with the essential requirements set out in Annex I to this Regulation using the relevant conformity assessment procedure provided for in Chapter III of Regulation (EU) 2025/327." `docs/law/eu/ehds/text.html Art. 104(3)`. This is not in the vendored CRA text.
- **Addressee:** the **manufacturer**. Art. 3(13): "a natural or legal person who develops or manufactures products with digital elements … and markets them under its name or trademark, whether for payment, monetisation or free of charge". That is Cadasto. These duties reach the **software**.

### Annex I Part I: essential requirements that touch telemetry
Lead-in, Annex I Part I (2): "On the basis of the cybersecurity risk assessment referred to in Article 13(2) and where applicable, products with digital elements shall:"

| Point | Verbatim | Bearing on telemetry |
|---|---|---|
| (2)(b) | "be made available on the market with a secure by default configuration, unless otherwise agreed between manufacturer and business user in relation to a tailor-made product with digital elements, including the possibility to reset the product to its original state;" | "Secure by default" is **not defined** in Art. 3 (the defined-term list has no such entry). The text is silent on whether a default-on outbound connection to the manufacturer fits it |
| (2)(c) | "…automatic security updates that are installed within an appropriate timeframe enabled as a default setting, with a clear and easy-to-use opt-out mechanism…" | The Regulation's own pattern for a default-on function with opt-out. It is used **for a security function** |
| (2)(e) | "protect the confidentiality of stored, transmitted or otherwise processed data, personal or other, such as by encrypting relevant data at rest or in transit by state of the art mechanisms…" | The report in transit (TLS). Menu: "such as" |
| (2)(f) | "protect the integrity of stored, transmitted or otherwise processed data, personal or other, commands, programs and configuration against any manipulation or modification not authorised by the user…" | The telemetry channel must not become a path to change configuration. A collector response must not alter the instance |
| **(2)(g)** | "process only data, personal or other, that are adequate, relevant and limited to what is necessary in relation to the intended purpose of the product with digital elements (data minimisation);" | **Covers non-personal data too.** Measured against the **intended purpose of the product**. Whether vendor telemetry is "necessary in relation to the intended purpose" of a CDR is the central CRA ambiguity, see below |
| (2)(h) | "protect the availability of essential and basic functions, also after an incident…" | A collector outage or a slow, hostile collector must never degrade the CDR |
| (2)(i) | "minimise the negative impact by the products themselves or connected devices on the availability of services provided by other devices or networks;" | Bounded report size and frequency, backoff |
| **(2)(j)** | "be designed, developed and produced to limit attack surfaces, including external interfaces;" | An outbound client that parses collector responses, resolves DNS and traverses proxies is an external interface |
| (2)(l) | "provide security related information by recording and monitoring relevant internal activity, including the access to or modification of data, services or functions, with an opt-out mechanism for the user;" | The second instance of the default-on-with-opt-out pattern. It concerns **local** security monitoring, not reporting to the manufacturer |

- **Citation:** `docs/law/eu/cra/text.html` Annex I Part I (1), (2)(b)–(l).
- **Closed or open:** the (2) list is closed as a list of requirements, but each applies "where applicable" and "on the basis of the cybersecurity risk assessment". Several points contain menus ("such as").
- **Ambiguity on (2)(g) and "intended purpose":**
  - Art. 3: "‘intended purpose’ means the use for which a product with digital elements is intended by the manufacturer, including the specific context and conditions of use, as specified in the information supplied by the manufacturer in the instructions for use, promotional or sales materials and statements, as well as in the technical documentation;"
  - So the manufacturer's own documentation shapes the measure. The text does not say whether processing that serves the manufacturer rather than the user (product analytics) can be part of a product's "intended purpose". Open point.
- **Recital 64 (non-operative):** "Manufacturers should make their products with digital elements available on the market with a secure by default configuration…" It does not define the term.

### Art. 13: the risk assessment has to address the telemetry function
- **Art. 13(2):** "…manufacturers shall undertake an assessment of the cybersecurity risks associated with a product with digital elements and take the outcome of that assessment into account during the planning, design, development, production, delivery and maintenance phases…"
- **Art. 13(3):** "The cybersecurity risk assessment shall indicate whether and, if so in what manner, the security requirements set out in Part I, point (2), of Annex I are applicable to the relevant product … and how those requirements are implemented…"
- **Citation:** `docs/law/eu/cra/text.html Art. 13(2), 13(3)`; Art. 13(4) (replaced by EHDS Art. 104(1)) puts the assessment in the technical documentation.
- **Testable:** the technical documentation states how (2)(b), (e), (f), (g), (h), (i) and (j) are met by the telemetry client.

### Annex II: information and instructions to the user
- **Annex II lead-in:** "At minimum, the product with digital elements shall be accompanied by:"
- **Items that bear on telemetry:**
  - **4:** "the intended purpose of the product with digital elements, including the security environment provided by the manufacturer, as well as the product’s essential functionalities and information about the security properties;"
  - **5:** "any known or foreseeable circumstance … which may lead to significant cybersecurity risks;"
  - **8(a):** "the necessary measures during initial commissioning and throughout the lifetime of the product … to ensure its secure use;"
  - **8(b):** "how changes to the product with digital elements can affect the security of data;"
- **The only Annex II item that names a default setting and how to turn it off is 8(e):** "how the default setting enabling the automatic installation of security updates … can be turned off;". **Annex II is silent on telemetry and on data the product sends to the manufacturer.** Item 8(e) is the textual model for a "how to switch it off" instruction, but it covers updates only.
- **Art. 13(18):** the information must be "clear, understandable, intelligible and legible" and "allow for the secure installation, operation and use", "in a language which can be easily understood by users". It must be kept available for at least 10 years or the support period.
- **Citation:** `docs/law/eu/cra/text.html` Annex II points 4, 5, 8(a), 8(b), 8(e); Art. 13(18).
- **Silence:** the CRA has no definition of "user" (the Art. 3 defined-term list has none). Whether "the user" means the operating organisation or its administrators is not settled.
- **Reading:** an outbound connection to a manufacturer endpoint is a security property and a commissioning matter (egress rules). Annex II 4 and 8(a) are the natural place to document it, but the text does not name telemetry.

### Collection by the manufacturer; vulnerability handling
- **No CRA provision governs a manufacturer collecting usage or telemetry data from installed products.** A search of the OJ text for telemetry, usage-data and collection wording found nothing on point. **The text is silent: it neither requires nor prohibits telemetry.**
- **"Remote data processing" (Art. 3(2)):** "data processing at a distance for which the software is designed and developed by the manufacturer, or under the responsibility of the manufacturer, and the absence of which would prevent the product with digital elements from performing one of its functions". Art. 3(1) brings such processing inside the "product". **Reading:** an opt-out collector whose absence prevents no CDR function does not match the "absence of which would prevent" limb, so the collector would sit outside the product while the client code stays inside it. The text does not address this case directly. Open point.
- **Duties telemetry could support but that do not require it:**
  - Annex I Part II (2): "address and remediate vulnerabilities without delay, including by providing security updates…"
  - Part II (7): "provide for mechanisms to securely distribute updates…"
  - Part II (8): "…disseminated without delay … accompanied by advisory messages providing users with the relevant information…"
  - Art. 14(8): "…the manufacturer shall inform the impacted users of the product with digital elements, and where appropriate all users, of that vulnerability or incident…"
  - Art. 13(19): "Where technically feasible … manufacturers shall display a notification to users informing them that their product … has reached the end of its support period."
  - Version telemetry would let Cadasto identify "impacted users". The text does not require it; "where appropriate all users" is the text's own fallback. The Art. 13(19) notification is shown to users locally and does not need a report to the manufacturer.
- **Possible side effect of collecting data:**
  - Art. 14(1): "A manufacturer shall notify any actively exploited vulnerability … that it becomes aware of…"
  - Collected data could be a source of such awareness. The text is silent on whether the data a manufacturer holds count toward "becomes aware". Open point.

**Answer to Q6:**
- The CRA neither requires nor prohibits telemetry.
- A default-on outbound connection is not named as a breach of "secure by default". That term is undefined, and the text's two default-on-with-opt-out examples are security functions.
- The CRA pulls the telemetry client into the manufacturer's risk assessment (Art. 13(3)) and into (2)(e), (f), (g), (h), (i), (j).
- (2)(g) applies to non-personal data as well.
- Annex II has no telemetry item. Documenting the connection and the switch fits Annex II 4/8(a) by analogy with 8(e), but the text does not require it in terms.
- Dates: Annex I applies from 11 December 2027 (Art. 71(2), Art. 69(2)). Art. 14 has applied since 11 September 2026.

---

## Q7. EHDS (Regulation (EU) 2025/327)

- **Annex II** (essential requirements for the harmonised software components):
  - 1.2: "…the EHR system can be supplied and installed, taking into account the instructions and information provided by the manufacturer, without adversely affecting its characteristics and performance during its intended use."
  - 2.5: "…shall not include features that prohibit, restrict or place an undue burden on authorised access, personal electronic health data sharing or use of personal electronic health data for permitted purposes."
  - 3.2: the logging component "shall provide sufficient logging mechanisms that record at least the following information on every access event…: (a) identification of the healthcare provider…; (b) identification of the specific natural person or persons having accessed…; (c) the categories of data accessed; (d) the time and date of access; (e) the origin or origins of data."
  - 3.3: "…tools or mechanisms to review and analyse the log data, or it shall support the connection and use of external software for the same purposes."
- **Citation:** `docs/law/eu/ehds/text.html` Annex II 1.2, 2.5, 3.2, 3.3.
- **Art. 30(1)(b):** manufacturers shall "ensure that the harmonised software components of their EHR systems are not adversely affected by other software components of the same EHR system;" `docs/law/eu/ehds/text.html Art. 30(1)(b)`.
- **Art. 2(2)(o):** the "European logging software component" "provides logging information related to access by health professionals or other individuals to priority categories of personal electronic health data". This is access logging of health data, a different thing from product telemetry.
- **Art. 25(2):** "This Chapter shall not apply to general purpose software used in a healthcare environment."
- **Serious incidents (Art. 44(7) with Art. 2(2)(r)):**
  - Manufacturers "shall report any serious incident … not later than three days after the manufacturer becomes aware of the serious incident".
  - "‘serious incident’ means any malfunction or deterioration in the characteristics or performance of an EHR system … that directly or indirectly leads, might have led or might lead to … (i) the death of a natural person or serious harm to a natural person’s health; …"
  - Citation: `docs/law/eu/ehds/text.html Art. 44(7)`, `Art. 2(2)(r)`.
- **Addressee:** the manufacturer of the EHR system (Cadasto). This reaches the software.
- **Dates (Art. 105):** "This Regulation shall apply from 26 March 2027." Articles 25, 26, 27, 47, 48 and 49 apply from 26 March 2029 or 2031 depending on the priority data categories. Chapter III applies to EHR systems put into service under Art. 26(2) from 26 March 2031.
- **Silence:** **EHDS contains no provision on a manufacturer receiving data from installed EHR systems, and nothing on product telemetry.**
- **What the text does constrain:**
  1. Telemetry must not adversely affect the logging and interoperability components (Art. 30(1)(b), Annex II 1.2).
  2. The report must never carry Annex II 3.2 log content (who accessed whose record).
  3. Collecting performance data can, in principle, bring a "deterioration in … performance" to the manufacturer's notice. The serious-incident duty turns on harm or potential harm to health, and the text does not say whether latency telemetry alone can amount to awareness of a serious incident. Open point.

---

## Q8. NIS2: what the hospital will demand of a vendor's outbound connection

- **Art. 21(1):** "Member States shall ensure that essential and important entities take appropriate and proportionate technical, operational and organisational measures to manage the risks posed to the security of network and information systems which those entities use…"
- **Art. 21(2)**, "…shall include at least the following":
  - "(d) supply chain security, including security-related aspects concerning the relationships between each entity and its direct suppliers or service providers;"
  - "(e) security in network and information systems acquisition, development and maintenance, including vulnerability handling and disclosure;"
  - "(i) human resources security, access control policies and asset management;"
- **Art. 21(3):** "…entities take into account the vulnerabilities specific to each direct supplier and service provider and the overall quality of products and cybersecurity practices of their suppliers and service providers, including their secure development procedures."
- **Recital 85 (non-operative):** "Essential and important entities should in particular be encouraged to incorporate cybersecurity risk-management measures into contractual arrangements with their direct suppliers and service providers."
- **Citation:** `docs/law/eu/nis2/text.html Art. 21(1)–(3)`, recital 85.
- **Scope:** Art. 2(1) covers entities "of a type referred to in Annex I or II which qualify as medium-sized enterprises … or exceed the ceilings". Annex I sector 5, Health, lists "Healthcare providers as defined in Article 3, point (g), of Directive 2011/24/EU". Art. 3(1)(a): essential if above the medium-sized ceilings; otherwise important (Art. 3(2)). A sole GP practice is below the Art. 2(1) size threshold unless Art. 2(2) applies.
- **(d) Binds:** **Member States**, and through national law **the hospital**. Not Cadasto and not the software. The national transposition acts are not vendored.
- **(e) Closed or open:** "at least the following" names a minimum set of *areas*. Within each, the measures are the entity's "appropriate and proportionate" choice. It is not a product checklist.
- **(f) Silence:** **NIS2 says nothing specific about vendor telemetry or outbound connections.** What a hospital will actually ask of the vendor (egress documentation, an off switch, disclosure of the endpoint and payload) is a matter of its own Art. 21(2)(d)/(3) assessment and contract. The Directive does not prescribe it. The text also does not say whether Cadasto, as telemetry collector, is a "service provider" or only a "supplier". On the Art. 6(39) wording, a "managed service provider" provides "installation, management, operation or maintenance … via assistance or active administration", which a receive-only collector does not match. That is a reading, not settled text.

---

## Q9. National law

### Netherlands
- **UAVG art. 4 lid 1:** "Deze wet en de daarop berustende bepalingen zijn van toepassing op de verwerking van persoonsgegevens in het kader van activiteiten van een vestiging van een verwerkingsverantwoordelijke of een verwerker in Nederland." `docs/law/nl/uavg/text.html Art. 4 lid 1`.
  - Binds: Cadasto (NL establishment) for any personal data in the telemetry.
  - **The UAVG is silent on telemetry, on legitimate interest, and on data about legal persons.** It adds nothing to Q1–Q5.
- **BW 7:457 lid 1:** "…draagt de hulpverlener zorg, dat aan anderen dan de patiënt geen inlichtingen over de patiënt dan wel inzage in of afschrift van de gegevens uit het dossier worden verstrekt dan met toestemming van de patiënt."
  - Lid 2 excludes from "anderen" only "degenen die rechtstreeks betrokken zijn bij de uitvoering van de behandelingsovereenkomst en degene die optreedt als vervanger van de hulpverlener".
  - Citation: `docs/law/nl/bw7-geneeskundige-behandelingsovereenkomst/text.html Art. 7:457 lid 1–2`.
  - Binds: the **hulpverlener**, a natural or legal person under 7:446 lid 1. Not Cadasto, and not the software as such.
  - **Reading:** a report containing no "inlichtingen over de patiënt" and no "gegevens uit het dossier" is not a provision of such information. A vendor is not within lid 2. **The text is silent on aggregates** (a coarse count of records). It speaks of "de patiënt", the individual.
- **Wabvpz art. 15j lid 1** ("Bij algemene maatregel van bestuur kunnen regels worden gesteld over de functionele, technische en organisatorische maatregelen voor het beheer, de beveiliging en het gebruik van een zorginformatiesysteem…") **→ Begz art. 3 lid 2:** "Een zorgaanbieder draagt overeenkomstig het bepaalde in NEN 7510 en NEN 7512, zorg voor een veilig en zorgvuldig gebruik van het zorginformatiesysteem…"
  - Citation: `docs/law/nl/wabvpz/text.html Art. 15j lid 1`, `docs/law/nl/begz/text.html Art. 3 lid 2`.
  - Binds: the **zorgaanbieder**.
  - **Silence:** what NEN 7510/7512 require of a supplier's outbound connection is in the standards, which are not vendored (`docs/law/nl/nen-7510/PROVENANCE.md`, `nen-7512/PROVENANCE.md`). The corpus cannot answer it.
- **NEN 7513:** not vendored (record only). It concerns logging of actions on patient records, not vendor telemetry.

### Germany
- **BDSG § 1 Abs. 1 Satz 2 and Abs. 4:** the BDSG applies to non-public bodies' automated processing where "der Verantwortliche oder Auftragsverarbeiter personenbezogene Daten im Inland verarbeitet" or the processing is "im Rahmen der Tätigkeiten einer inländischen Niederlassung" (Abs. 4 Nr. 1–2).
- **§ 1 Abs. 2 Satz 3:** "Die Verpflichtung zur Wahrung gesetzlicher Geheimhaltungspflichten oder von Berufs- oder besonderen Amtsgeheimnissen, die nicht auf gesetzlichen Vorschriften beruhen, bleibt unberührt."
- **§ 36:** limits the Art. 21(1) objection right only "gegenüber einer öffentlichen Stelle". It does not apply to Cadasto.
- **Citation:** `docs/law/de/bdsg/BJNR209710017.xml § 1 Abs. 1, 2, 4`; `§ 36`.
- **The BDSG is silent on telemetry.** § 26 (employee data) would matter only if staff data were in the report, and none are proposed.
- **StGB § 203:**
  - Abs. 1: "Wer unbefugt ein fremdes Geheimnis, namentlich ein zum persönlichen Lebensbereich gehörendes Geheimnis oder ein Betriebs- oder Geschäftsgeheimnis, offenbart, das ihm als 1. Arzt, Zahnarzt, … anvertraut worden oder sonst bekanntgeworden ist, wird mit Freiheitsstrafe bis zu einem Jahr oder mit Geldstrafe bestraft."
  - Abs. 3 Satz 2: "Die in den Absätzen 1 und 2 Genannten dürfen fremde Geheimnisse gegenüber sonstigen Personen offenbaren, die an ihrer beruflichen oder dienstlichen Tätigkeit mitwirken, soweit dies für die Inanspruchnahme der Tätigkeit der sonstigen mitwirkenden Personen erforderlich ist…"
  - Abs. 4 Satz 1: "…wer unbefugt ein fremdes Geheimnis offenbart, das ihm bei der Ausübung oder bei Gelegenheit seiner Tätigkeit als mitwirkende Person … bekannt geworden ist."
  - Abs. 4 Satz 2 Nr. 1: punishes the professional who "nicht dafür Sorge getragen hat, dass eine sonstige mitwirkende Person … zur Geheimhaltung verpflichtet wurde".
  - Citation: `docs/law/de/stgb/BJNR001270871.xml § 203 Abs. 1, 3, 4`.
  - **Binds:** natural persons in the listed professions (Abs. 1 Nr. 1: "Arzt, Zahnarzt, … Angehörigen eines anderen Heilberufs") and "mitwirkende Personen" (Abs. 4). It does not bind the hospital as an entity, and it does not bind software.
  - **Reading:**
    - The offence needs an "Offenbaren" of a "fremdes Geheimnis" that came to the professional "als Arzt". A report carrying no patient-related secret discloses none. The safe harbour of Abs. 3 Satz 2 is not needed for it.
    - If a secret ever leaked into telemetry, Abs. 3 Satz 2 would not plainly cover it. It permits disclosure only "soweit dies für die Inanspruchnahme der Tätigkeit der sonstigen mitwirkenden Personen erforderlich ist", and product telemetry is not a contributing service the physician uses.
  - **Silence:**
    - The text does not define "Geheimnis" and says nothing about aggregates (a data-volume class).
    - It does not say whether a vendor receiving operational metrics is a "mitwirkende Person".
    - Protected secrets include "Betriebs- oder Geschäftsgeheimnis[se]" of others. The text does not say whether a hospital's own operational figures fall within "fremd" from an employed physician's standpoint.
  - Land law (Landeskrankenhausgesetze) is not vendored.

### Switzerland
- **Natural persons only:** DSG Art. 2 Abs. 1, Art. 5 lit. a (Q1). A Swiss legal-person operator's report is outside the DSG unless it identifies a natural person.
- **Territorial scope:** Art. 3 Abs. 1: "Dieses Gesetz gilt für Sachverhalte, die sich in der Schweiz auswirken, auch wenn sie im Ausland veranlasst werden." English (non-binding): "…circumstances that have an effect in Switzerland, even if they were initiated abroad." `docs/law/ch/fadp/text-de.html Art. 3 Abs. 1`. Cadasto (NL) is reached for a Swiss sole practitioner's data.
- **Export to NL and to Hetzner sites:**
  - DSG Art. 16 Abs. 1: "Personendaten dürfen ins Ausland bekanntgegeben werden, wenn der Bundesrat festgestellt hat, dass die Gesetzgebung des betreffenden Staates … einen angemessenen Schutz gewährleistet."
  - DSV Art. 8 Abs. 1: "Die Staaten … mit einem angemessenen Datenschutz werden in Anhang 1 aufgeführt."
  - **DSV Anhang 1 lists "1 Deutschland*", "13 Finnland*", "34 Niederlande*".**
  - Citation: `docs/law/ch/fadp/text-de.html Art. 16 Abs. 1`; `docs/law/ch/dpo/text-de.html Art. 8 Abs. 1` and Anhang 1.
  - Binds: whoever "bekanntgibt" (discloses abroad).
  - The text does not settle who discloses when the software sends by default and the operator does not switch it off: the operator or Cadasto. Open point.
- **Information:** Art. 19 Abs. 4: "Werden die Personendaten ins Ausland bekanntgegeben, so teilt er der betroffenen Person auch den Staat … mit…" The Netherlands (and the Hetzner country) must be named in the notice for Swiss data subjects.
- **Defaults:** Art. 7 Abs. 3 (Q4).
- **Consent:** only where required. Art. 6 Abs. 7 lit. a requires explicit consent for "die Bearbeitung von besonders schützenswerten Personendaten". Health data are besonders schützenswert under Art. 5 lit. c Ziff. 2. This is a further reason the report must carry no patient data.
- **No Swiss consent rule specific to telemetry or to default settings beyond Art. 7 Abs. 3 is in the corpus.** EPDG/EPDV bind EPD communities and are not engaged by vendor telemetry. Cantonal law for public hospitals is not vendored.

---

## Q10. ePrivacy Directive Art. 5(3) and Telecommunicatiewet art. 11.7a

**Confirmed: neither is in the corpus.**
- No `docs/law/eu/eprivacy/` directory exists, and no file vendors Directive 2002/58/EC. The Directive appears only in cross-references:
  - GDPR Art. 21(5) and Art. 95 (`docs/law/eu/gdpr/text.html`);
  - CRA recital 72;
  - EHDS Art. 1(3);
  - NIS2 Art. 2(12), Art. 46 and recitals 14, 92, 106, 108.
- GDPR Art. 95 reads: "This Regulation shall not impose additional obligations on natural or legal persons in relation to processing in connection with the provision of publicly available electronic communications services in public communication networks in the Union in relation to matters for which they are subject to specific obligations with the same objective set out in Directive 2002/58/EC."
- No Dutch Telecommunicatiewet is vendored. No file under `docs/law/nl/` and no text anywhere in `docs/law/` contains "Telecommunicatiewet" or "11.7a".
- **The corpus is silent on whether reading or storing information on the operator's terminal equipment (here, a server the operator controls) needs consent or a strict-necessity exemption.** Nothing here may be read as an answer to that question.

---

## Synthesis A: provisions that reach the CDR software (the telemetry client inside FerroEHR, as manufactured by Cadasto)

| Provision | What the software must be able to do | Addressee and scope cite |
|---|---|---|
| CRA Annex I Part I (2)(b) | Ship a configuration the risk assessment can defend as "secure by default", and support reset to the original state | Manufacturer, Art. 13(1); from 11 Dec 2027 (Art. 71(2), 69(2)) |
| CRA Annex I Part I (2)(e), (f) | Encrypt the report in transit. Never let the channel modify configuration, commands or programs | Manufacturer, Art. 13(1) |
| CRA Annex I Part I (2)(g) | Send only data, personal **or other**, limited to what is necessary for the product's intended purpose | Manufacturer, Art. 13(1) |
| CRA Annex I Part I (2)(h), (i), (j) | Telemetry failure must never impair the CDR. Bounded traffic. Minimal attack surface on the outbound client | Manufacturer, Art. 13(1) |
| CRA Art. 13(2)–(3) | The telemetry function is covered in the documented risk assessment | Manufacturer |
| CRA Annex II 4, 5, 8(a), 8(b) (8(e) by analogy only) | User information names the outbound connection, its payload and the switch | Manufacturer, Art. 13(18) |
| CRA Art. 14 (since 11 Sep 2026) | No duty to build telemetry. Data the manufacturer holds may bear on "becomes aware" | Manufacturer, Art. 71(2), 69(3) |
| EHDS Art. 30(1)(b), Annex II 1.2 | Telemetry must not adversely affect the logging and interoperability components or the system's performance | EHR-system manufacturer; dates per Art. 105 |
| EHDS Annex II 3.2 | (Negative) access-log content never leaves through telemetry | EHR-system manufacturer |
| GDPR Art. 25(2) | Where any field is personal data, defaults limited per purpose. Optional fields off by default | **Cadasto as controller of the telemetry**, which the software implements (Art. 4(7)) |
| GDPR Art. 21(1)/(4), 17(1)(c) | A working off switch, plus a route (keyed by instance id) to stop processing and erase stored reports | Cadasto as controller |
| GDPR Art. 12(1), 13/14 | A notice reachable before the first report, with the complete Art. 13/14 item list | Cadasto as controller |
| DSG Art. 7 Abs. 3, Art. 30 Abs. 2 lit. b | Default limited to the minimum. Honour an expressed objection, including for stored data | Cadasto as Verantwortlicher |

## Synthesis B: provisions that fall on the organisation or a third party only (a page must never claim the software meets them)

| Provision | Who it binds |
|---|---|
| NIS2 Art. 21(1)–(3) (supply-chain assessment of the vendor) | Member States, then the hospital as an essential or important entity under national law (not vendored) |
| BW 7:457 | The hulpverlener |
| Begz art. 3 lid 2 (NEN 7510/7512) | The zorgaanbieder |
| StGB § 203 | The physician or other listed professional, and any mitwirkende Person (natural persons) |
| BDSG § 36 | Public bodies only |
| GDPR Art. 6(1) second subparagraph (no point (f) for public authorities) | A public hospital, if it were a (joint) controller of the transmission |
| GDPR Art. 28(1)/(3) contract with Hetzner | Cadasto as an organisation (collector operator), not the CDR software |
| GDPR Art. 30(1)/(5) records of processing; Art. 32 security of the collector; Art. 5(1)(e) retention at the collector | Cadasto as an organisation |
| DSG Art. 16 / DSV Anhang 1 export | The discloser (operator or Cadasto; open point) |

---

## Design constraints that follow (what the telemetry must and must not do to stay inside these texts)

**Must not:**
1. **Carry any patient-level value**: no concrete request paths (only route templates), no `ehr_id`/subject ids, no AQL text or literals (only normalised shapes, or none), no error messages that echo input, no Annex II 3.2 log content. Otherwise GDPR Art. 4(1)/4(15)/9(1) and DSG Art. 6 Abs. 7 are engaged with no visible ground, and BW 7:457 / StGB § 203 come into play.
2. **Carry staff identities**: no user names, no IPs of clients that call the CDR.
3. **Be used to change instance configuration or behaviour**: CRA Annex I (2)(f), (j).
4. **Affect CDR availability or the EHDS harmonised components when the collector is slow or unreachable**: CRA (2)(h), (i); EHDS Art. 30(1)(b), Annex II 1.2.
5. **Be put on a Grafana dashboard without access control**: GDPR Art. 25(2), last sentence.
6. **Be described as "anonymous" on the strength of recital 26**, which the corpus does not carry.

**Must:**
1. Bring the whole of "is it personal data" down to two switches: the licensee-naming licence id and the stored source IP. Not storing the IP at the collector (or proxy/LB logs) and reducing the licence id to grant *type* by default keeps a legal-person operator's report outside Art. 4(1). A sole practitioner's report is personal data once the licence id names them (Q1). Under GDPR Art. 25(2), sending the licensee-naming id belongs off by default unless it is necessary for a stated purpose.
2. State the purposes before the first report: GDPR Art. 5(1)(b), 13(1)(c)/(d); DSG Art. 6 Abs. 3, Art. 19. If licence-grant auditing is a purpose, state it now. A later addition is a further purpose under Art. 6(4).
3. Publish an Art. 13/14 notice that the boot line points to: controller identity and contact, purposes, Art. 6(1)(f) and the interest, recipients (Hetzner), retention, the right to object (presented "clearly and separately", Art. 21(4)), the complaint right, and for CH the destination states (DSG Art. 19 Abs. 4). Send the first report no earlier than the point at which that notice is reachable.
4. Provide an objection and erasure route for data already collected (by instance id), not only the client switch: GDPR Art. 21(1), 17(1)(c); DSG Art. 30 Abs. 2 lit. b.
5. Fix a retention period at the collector and say what it is: GDPR Art. 5(1)(e), 13(2)(a); DSG Art. 6 Abs. 4.
6. Encrypt in transit; bound the size and frequency of reports; keep the client minimal: CRA Annex I (2)(e), (i), (j).
7. Document the connection (endpoint, payload schema, frequency, switch) in the CRA user information (Annex II 4, 8(a)) and in the technical documentation's risk assessment (Art. 13(3)). Hospitals' NIS2 Art. 21(2)(d)/(3) assessments will look for it.
8. Update the book pages that today say the opposite, in the same PR (repository rule, not law):
   - `website/book/src/threat-model.md:221` ("Outbound terminology, object-store and broker calls go only to operator-configured endpoints").
   - `website/book/src/compliance/shared-responsibility.md:27-28` ("it operates nothing on your behalf and holds none of your data").
   - `website/book/src/compliance/index.md:194` ("The FerroEHR project operates nothing and is not your processor").
   - `website/book/src/security/records-of-processing.md:33`: Cadasto becomes a recipient to list where telemetry carries personal data.

## Open points the text does not settle
1. Whether data about a legal person and an organisation's IP are outside "personal data". Recitals 14, 26 and 30 are not vendored. The articles alone give the natural-person wording only.
2. Whether a random, stable instance id is an "online identifier" of a natural person for a small practice. The corpus has no identifiability test.
3. Whether the operator that leaves the default on is a (joint) controller of the transmission (GDPR Art. 26(1)). If it is a public authority, Art. 6(1) second subparagraph follows.
4. Whether a server log line is a "communication with the data subject" (Art. 21(4)), and whether machine-sent data are "collected from the data subject" (Art. 13) or not (Art. 14).
5. Whether a configuration switch is an Art. 21(5) automated objection (that paragraph is limited to "information society services").
6. CRA: the meaning of "secure by default configuration" (undefined), whether vendor telemetry can be part of the product's "intended purpose" for Annex I (2)(g), and whether the collector is "remote data processing" (Art. 3(2)).
7. CRA Art. 14(1) and EHDS Art. 44(7): whether data held through telemetry count toward the manufacturer "becom[ing] aware" of an actively exploited vulnerability or a serious incident.
8. Whether Cadasto is a "direct supplier or service provider" in NIS2 terms for the telemetry relationship. The national transposition acts are not vendored.
9. Whether coarse data-volume buckets for a very small instance can reveal "inlichtingen over de patiënt" (BW 7:457) or a "Geheimnis" (StGB § 203). Both texts are silent on aggregates.
10. Who "discloses abroad" under DSG Art. 16 when the software sends by default: the Swiss operator or Cadasto.
11. ePrivacy Art. 5(3) and Tw art. 11.7a: not in the corpus.
12. What NEN 7510-2 / 7512 require of a supplier's outbound connection: the standards are not vendored.

## En-route findings (out of scope, for the tracker)
1. `docs/law/eu/gdpr/PROVENANCE.md:28-34` (## Version): the record does not say that the vendored consolidation CELEX `02016R0679-20160504` carries **no recitals**. `docs/law/eu/gdpr/text.html` has 0 `id="rct_…"` anchors, against 130/115/144 in CRA/EHDS/NIS2. Any recital citation therefore resolves to no vendored bytes.
2. `website/book/src/audit.md:259` cites "GDPR Art. 15 with Recital 63". Recital 63 is not in the vendored GDPR text, which `.claude/rules/law-corpus.md` §Citation form forbids ("Never cite a consolidation the tree does not carry").
3. `docs/law/eu/cra/PROVENANCE.md:31-34` says an initial consolidation "folds in no amendment". EHDS Art. 104 (`docs/law/eu/ehds/text.html:9499`, `id="art_104"`) amends CRA Art. 13(4) and 31(3) and inserts Art. 32(5a), which applies to EHR systems. The vendored CRA OJ text carries none of these (no "32(5a)"/"2025/327" in `docs/law/eu/cra/text.html`). A reader of CRA Art. 13(4) for an EHR system is not warned.
