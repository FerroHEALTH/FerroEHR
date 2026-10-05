# #3580 adjudication: the usage report's default under ePrivacy Art. 5(3) and Tw art. 11.7a

Read 2026-10-05, first-hand, from the checkout on branch `chore/v434-law-and-ci`. No legal advice: this reports what the vendored text says, who it binds, and what the software would have to be able to do. The word "compliant" is not used. EDPB Guidelines 2/2023 are NOT vendored; the issue body relies on their §18 and §33. Nothing below rests on them; where they would add weight it is marked.

Texts read:
- `docs/law/eu/eprivacy/text.html` (consolidation 02002L0058-20091219): Art. 1, 2, 3, 5 (`id="art_5"`, line 394).
- `docs/law/eu/eprivacy/oj.html` (2002/58/EC, OJ L 201): recitals 7, 12, 17, 24, 25.
- `docs/law/eu/eprivacy/amending-directive-2009-136.html` (2009/136/EC): Art. 2 point (5), Art. 4(1), recitals 65, 66 (`id="rct_66"`, line 1116).
- `docs/law/nl/telecommunicatiewet/text.html` (BWBR0009950, 2026-08-15): art. 1.1 (line 781), 11.1 (line 12643), 11.2, 11.7a (line 13497).
- `docs/law/eu/gdpr/text.html`: Art. 4(1), 4(11), 6(1), 7, 94, 95. `docs/law/eu/gdpr/oj.html`: recitals 14, 26, 30, 32 (line 566), 42, 43 (line 744), 47, 173.
- `docs/plans/usage-report/research-law.md` (whole).

---

## Verdict in one paragraph

Art. 5(3) is worded to cover any storing of information in, or gaining of access to information stored in, the terminal equipment of a subscriber or user, personal data or not; the report both stores (the instance id in the operator's database) and reads (instance id, version, migration state) information held on the operator's equipment and sends it to Cadasto. The vendored text does not define "terminal equipment" (ePrivacy imports its definitions from acts that are not vendored; the Tw does not define "randapparatuur"), so whether a server is terminal equipment is a point the corpus leaves open; nothing in it excludes a server. Neither exemption of Art. 5(3) fits on its wording: the report is not for "the sole purpose of carrying out the transmission of a communication", and it is not "strictly necessary" for any service the operator "explicitly requested", which the feature's own design shows (the CDR works the same with the report off or the collector down). The Dutch-only third limb of Tw art. 11.7a lid 3(b) (quality or effectiveness of a delivered information society service, minor privacy impact) has no counterpart in the Directive's wording, rests on an "information society service" definition the corpus does not carry, and reaches Dutch law only. Consent in the sense of GDPR Art. 4(11) needs "a statement or … a clear affirmative action", given before the access ("has given"); default-on with a boot log line is inactivity, which GDPR recital 32 says is not consent, and the first `start` report goes out before anyone can read that line. **Recommendation: switch the default to off (opt-in) for every licence type, before the release that ships #3578.** An explicit operator action (config key, env var, chart value, or an answered setup prompt whose default is "no") can carry the consent; a commercial order form can carry it only through a separate, optional tick box that is not a condition of the licence.

---

## Q1. Does Art. 5(3) ePrivacy (and Tw art. 11.7a) apply at all?

### 1.1 ePrivacy Art. 5(3), first sentence

- **(a) Testable statement:** storing information in, or gaining access to information already stored in, the terminal equipment of a subscriber or user is allowed only after that subscriber or user has consented, having first received clear and comprehensive information including the purposes. The condition covers "information", with no limitation to personal data.
- **(b) Quote:** "Member States shall ensure that the storing of information, or the gaining of access to information already stored, in the terminal equipment of a subscriber or user is only allowed on condition that the subscriber or user concerned has given his or her consent, having been provided with clear and comprehensive information, in accordance with Directive 95/46/EC, inter alia, about the purposes of the processing."
- **(c) Citation:** `docs/law/eu/eprivacy/text.html Art. 5(3)`, first sentence. The same words are the substitution made by `docs/law/eu/eprivacy/amending-directive-2009-136.html Art. 2` point (5) ("Article 5(3) shall be replaced by the following: ‘3. Member States shall ensure that the storing …’"). Transposition deadline: same file, `Art. 4(1)`: "Member States shall adopt and publish by 25 May 2011 the laws, regulations and administrative provisions necessary to comply with this Directive."
- **(d) Binds:** Member States ("Member States shall ensure"); the duty reaches persons through national law (in NL: Tw art. 11.7a, §1.3 below). The provision does not name who stores or gains access; recital 66 of 2009/136/EC frames it as third parties: "Third parties may wish to store information on the equipment of a user, or gain access to information already stored, for a number of purposes, ranging from the legitimate (such as certain types of cookies) to those involving unwarranted intrusion into the private sphere (such as spyware or viruses)." `docs/law/eu/eprivacy/amending-directive-2009-136.html Recital 66`.
- **(e) Closed or open:** the consent condition is closed; the information content is open ("inter alia, about the purposes").
- **(f) Scope tension, flagged:** Art. 3 reads: "This Directive shall apply to the processing of personal data in connection with the provision of publicly available electronic communications services in public communications networks in the Community, including public communications networks supporting data collection and identification devices." (`text.html Art. 3`). Art. 5(3) itself speaks of "information", not personal data, and names no provider of an electronic communications service. The vendored text does not reconcile Art. 3 with Art. 5(3). Recital 66 (third parties, cookies, spyware) and recital 65 (spyware "delivered and installed in software distributed on other external data storage media") read Art. 5(3) as reaching parties who are not network or service providers. The Dutch text (§1.3) is not limited to personal data or to providers on its face.

### 1.2 The three elements, applied

**"storing of information, or the gaining of access to information already stored"**
- *Storing:* the software generates a random instance id and stores it in the operator's database (facts of #3578). That is information placed on the operator's equipment for the purpose of Cadasto's report. On the wording, it is "storing of information".
- *Gaining access:* the software reads the instance id, the version, the migration status, the PostgreSQL major version and the CPU/memory class from the instance, and sends them to Cadasto. Cadasto receives information that was stored on the operator's equipment. On the wording, it is "gaining of access to information already stored".
- *Silence:* the text does not define "stored". Whether values that exist only in memory (uptime, route-group latency aggregates) are "information already stored" is not settled. It does not matter for the outcome: the instance id, the version and the migration state are stored on disk.
- *Silence:* the text does not say whether access performed by software the subscriber installed and runs itself is access "in the terminal equipment of a subscriber" by a third party. Recital 65 of 2009/136/EC covers software "delivered and installed in software distributed on other external data storage media", and Tw art. 11.7a lid 2 (below) covers access brought about "op een andere wijze dan door middel van een elektronisch communicatienetwerk". Both point toward installation by the operator not taking the act outside the rule. EDPB 2/2023 §33 (not vendored), as the issue quotes it, says the same of software that "will then proactively call an Application Programming Interface ('API') endpoint over the network"; that would add weight here.

**"terminal equipment"**
- *Silence:* Art. 2 defines no "terminal equipment". It imports definitions: "Save as otherwise provided, the definitions in Directive 95/46/EC and in Directive 2002/21/EC … (Framework Directive) shall apply." (`text.html Art. 2`, first paragraph). Directive 2002/21/EC is not vendored, and no vendored file defines the term. The Tw does not define "randapparatuur" either: art. 1.1 and art. 11.1 carry no such entry, and the word occurs four times in the whole act.
- What the vendored recitals say: "Terminal equipment of users of electronic communications networks and any information stored on such equipment are part of the private sphere of the users requiring protection under the European Convention for the Protection of Human Rights and Fundamental Freedoms." (`docs/law/eu/eprivacy/oj.html Recital 24`). Recital 66 speaks of "the equipment of a user". Neither limits the term to end-user devices such as phones or browsers, and neither mentions servers.
- **Result:** the corpus neither includes nor excludes a server. A design that relies on "a server is not terminal equipment" rests on a definition the corpus does not carry, against the issue's reading of EDPB 2/2023 §33. This is the single largest open point. EDPB 2/2023, and the definition in the act Art. 2 imports, would settle it in either direction; neither is vendored.

**"subscriber or user"**
- "User": "‘user’ means any natural person using a publicly available electronic communications service, for private or business purposes, without necessarily having subscribed to this service;" (`text.html Art. 2(a)`). A natural person only.
- "Subscriber": not defined in the ePrivacy text (imported from Directive 2002/21/EC, not vendored). Recital 12: "Subscribers to a publicly available electronic communications service may be natural or legal persons." (`oj.html Recital 12`). Recital 17: "consent of a user or subscriber, regardless of whether the latter is a natural or a legal person". Art. 1(2): the provisions "provide for protection of the legitimate interests of subscribers who are legal persons."
- The Tw defines it: "abonnee: natuurlijke persoon of rechtspersoon die partij is bij een overeenkomst met een aanbieder van openbare elektronische communicatiediensten voor de levering van dergelijke diensten;" (`docs/law/nl/telecommunicatiewet/text.html Art. 1.1`).
- **Reading:** the hospital or care organisation that runs FerroEHR on its own server is the subscriber (a legal person party to a contract for public electronic communications services, i.e. its internet connection). Its staff who administer the server are "users" (natural persons using a public ECS for business purposes). A sole practitioner is both.
- *Ambiguity:* where the server is a rented virtual machine at a hosting provider, the text does not say whether the operator is the subscriber of the connectivity the provider bundles. The text does not settle whose "terminal equipment" a hosted VM is.

### 1.3 Tw art. 11.7a lid 1 and lid 2 (authentic Dutch)

- **(a) Testable statement:** storing information in, or obtaining access to information in, a user's terminal equipment via an electronic communications network is allowed only if the user has received clear and complete information (at least about the purposes) and has consented. The same applies where the storing or access over the network is brought about by other means than the network (for example by installed software).
- **(b) Quote, lid 1:** "Onverminderd de Algemene verordening gegevensbescherming is het via een elektronisch communicatienetwerk opslaan van of toegang verkrijgen tot informatie in de randapparatuur van een gebruiker, alleen toegestaan op voorwaarde dat de betrokken gebruiker: a. is voorzien van duidelijke en volledige informatie overeenkomstig de Algemene verordening gegevensbescherming, in ieder geval over de doeleinden waarvoor deze informatie wordt gebruikt, en b. daarvoor toestemming heeft verleend."
- **Quote, lid 2:** "De in het eerste lid, onder a en b, genoemde vereisten zijn ook van toepassing in het geval op een andere wijze dan door middel van een elektronisch communicatienetwerk wordt bewerkstelligd dat via een elektronisch communicatienetwerk informatie wordt opgeslagen of toegang wordt verleend tot op het randapparaat opgeslagen informatie."
- **(c) Citation:** `docs/law/nl/telecommunicatiewet/text.html Art. 11.7a lid 1, lid 2`.
- **(d) Binds:** the provision is impersonal ("is het … alleen toegestaan"); it binds whoever stores or obtains access. It is not limited to providers of public networks or services (contrast art. 11.2, which names "de aanbieder van een openbaar elektronisch communicatienetwerk en de aanbieder van een openbare elektronische communicatiedienst"). It covers "informatie", not only persoonsgegevens, and any "elektronisch communicatienetwerk", not only a public one.
- **(e) Closed:** two cumulative conditions (a and b).
- **(f) Discrepancy with the Directive, flagged:** lid 1 protects only "de randapparatuur van een gebruiker" and asks consent of "de betrokken gebruiker". "gebruiker" is defined in art. 11.1 onder a: "een natuurlijke persoon die gebruik maakt van een openbare elektronische communicatiedienst voor particuliere of zakelijke doeleinden zonder noodzakelijkerwijze op die dienst te zijn geabonneerd". The Directive says "subscriber or user", and its recitals cover legal-person subscribers. Lid 3 onder b and art. 11.1 onder g ("toestemming van een gebruiker of abonnee: … met dien verstande dat de toestemming mede betrekking kan hebben op gegevens van abonnees die geen natuurlijke personen zijn") do mention the abonnee. On its literal wording, then, lid 1 might be read as not reaching a server owned by a legal person; the vendored text does not resolve whether it is read that narrowly or in line with the Directive's "subscriber or user". A product default cannot rest on that gap: the Directive's wording covers subscribers, the administering staff are "gebruikers", and the operators are not only Dutch.
- *Silence:* the vendored provisions read do not state the territorial reach of art. 11.7a (whether it follows the location of the equipment or the establishment of the party gaining access). Not settled here.
- *Pending amendments:* the publisher marks art. 1.1 (line 793) and art. 11.1 (line 12655) "Wijziging(en) zonder datum inwerkingtreding aanwezig". Art. 11.7a carries no such marker. The pending text is not vendored; the definitions quoted are those in force at 2026-08-15.

### Answer to Q1

On the wording, both texts reach the report: it stores information (the instance id) on the operator's equipment and gains access to stored information (instance id, version, migration state) that is then sent to Cadasto, and neither text is limited to personal data. The "subscriber or user" is the operating organisation (subscriber, Tw "abonnee") and its administering staff ("user", Tw "gebruiker", natural persons); for a sole practitioner, the practitioner. The corpus is **silent** on whether a server is "terminal equipment" / "randapparatuur"; nothing vendored excludes it, and the issue's reading of EDPB 2/2023 §33 includes it. Treating Art. 5(3) as applicable is the reading the vendored text supports; treating it as inapplicable requires a definition the corpus does not carry.

---

## Q2. Does an exemption fit?

### 2.1 ePrivacy Art. 5(3), second sentence

- **Quote:** "This shall not prevent any technical storage or access for the sole purpose of carrying out the transmission of a communication over an electronic communications network, or as strictly necessary in order for the provider of an information society service explicitly requested by the subscriber or user to provide the service." `docs/law/eu/eprivacy/text.html Art. 5(3)`, second sentence.
- **Closed list:** two exemptions, no "such as", no "in particular". No exemption for a legal obligation of the party gaining access, for security purposes, for statistics or for non-personal information.
- **Recital 66 (2009/136/EC), the stated limit on the exemptions:** "Exceptions to the obligation to provide information and offer the right to refuse should be limited to those situations where the technical storage or access is strictly necessary for the legitimate purpose of enabling the use of a specific service explicitly requested by the subscriber or user." `docs/law/eu/eprivacy/amending-directive-2009-136.html Recital 66`.

**Exemption 1: "for the sole purpose of carrying out the transmission of a communication"**
- Does not fit. The instance id, version and metrics are not read or stored to carry out the transmission of some communication; they are the content Cadasto wants to receive. Their purpose is Cadasto's knowledge of the installed base and field performance (#3577: "today we cannot see which releases are in use, whether instances upgrade after a security release, or where the CDR is slow in the field"). "Sole purpose" excludes a dual purpose.

**Exemption 2: "strictly necessary in order for the provider of an information society service explicitly requested by the subscriber or user to provide the service"**
Three cumulative elements, each failing or unsupported:
1. *An information society service:* not defined in the ePrivacy text. GDPR Art. 4(25) and Tw art. 1.1 point to "Richtlijn (EU) 2015/1535", Art. 1(1)(b), which is not vendored. **The corpus is silent** on whether licensing self-hosted software that runs on the operator's own server is an information society service Cadasto "provides".
2. *Explicitly requested by the subscriber or user:* the operator requested the FerroEHR software (the CDR). It did not request a reporting service. The report serves Cadasto's purposes, not a service rendered to the operator.
3. *Strictly necessary to provide that service:* the feature's own design shows the opposite. #3577 rules that "a slow or unreachable collector never affects boot, readiness or request handling", and that one switch stops all report traffic with no loss of CDR function. Recital 66's test is "enabling the use of a specific service explicitly requested"; the CDR is fully usable without the report.
- **CRA Art. 14 does not change this:** Art. 5(3) has no legal-obligation exemption, and `research-law.md` Q6 records from `docs/law/eu/cra/text.html` that the CRA neither requires nor prohibits telemetry (Art. 14(8) lets the manufacturer inform "where appropriate all users"). A vulnerability-handling purpose is a reason for the report, not an Art. 5(3) exemption.
- **Narrow case, flagged as unsettled:** if a commercial customer explicitly contracts a service such as "notify me when my running version is affected by a vulnerability", version reporting might be argued "strictly necessary" for that requested service. The text does not decide this, it would cover the version field only (not performance aggregates, not uptime), and it would cover only customers who requested that service. It is no basis for a default.

### 2.2 Tw art. 11.7a lid 3 (with the Dutch-only limb)

- **Quote:** "Het bepaalde in het eerste lid is niet van toepassing indien het de opslag of toegang betreft: a. met als uitsluitend doel de communicatie over een elektronisch communicatienetwerk uit te voeren, b. die strikt noodzakelijk is om de door de abonnee of gebruiker gevraagde dienst van de informatiemaatschappij te leveren of – mits dit geen of geringe gevolgen heeft voor de persoonlijke levenssfeer van de betrokken abonnee of gebruiker – om informatie te verkrijgen over de kwaliteit of effectiviteit van een geleverde dienst van de informatiemaatschappij."
- **Citation:** `docs/law/nl/telecommunicatiewet/text.html Art. 11.7a lid 3`.
- **Onder a** and the first limb of **onder b** mirror the Directive's two exemptions and fail for the same reasons as §2.1.
- **Second limb of onder b (the Dutch analytics exemption):** "om informatie te verkrijgen over de kwaliteit of effectiviteit van een geleverde dienst van de informatiemaatschappij", on condition of "geen of geringe gevolgen … voor de persoonlijke levenssfeer". Applied:
  - *Quality or effectiveness:* route-group latency aggregates, uptime and database status are information about the quality of the software in the field. That element plausibly fits.
  - *A delivered information society service ("geleverde dienst van de informatiemaatschappij"):* same gap as §2.1 point 1. The Tw defines only "aanbieder van diensten van de informatiemaatschappij" by reference to Directive (EU) 2015/1535 (`text.html Art. 1.1`), which is not vendored. **Silent** on whether self-hosted licensed software is such a service.
  - *No or minor privacy consequences:* for a legal-person operator the payload carries no natural-person data. But the collector stores the source IP and country with every report, keyed to a persistent instance id; for a sole practitioner that IP is "an online identifier" of a natural person (GDPR Art. 4(1); recital 30: "internet protocol addresses … may be used to create profiles of the natural persons and identify them", `docs/law/eu/gdpr/oj.html Recital 30`). The text gives no threshold for "geringe gevolgen".
  - *Not in the Directive:* the Directive's Art. 5(3) second sentence lists two exemptions and has no quality/effectiveness limb. The corpus carries nothing on how a national exemption beyond the Directive's wording stands. **Silent.**
  - *Reach:* this limb is Dutch law. It does nothing for an operator whose position is governed by another Member State's transposition (none vendored) or by Swiss law (no Swiss counterpart vendored).
- **Lid 4** (presumption of personal-data processing for collecting data about the use of several information society services in order to treat the user differently) does not describe the report. **Lid 5** concerns information society services provided by public-law legal persons; Cadasto B.V. is not one. **Lid 6** allows an AMvB with further rules; none is vendored, and whether one exists is outside the corpus.

### Answer to Q2

Neither exemption in the Directive fits on its wording. The Dutch quality/effectiveness limb is the only text that could cover part of the report, and it depends on an undefined term ("dienst van de informatiemaatschappij"), has no counterpart in the Directive's text, binds only under Dutch law, and is strained by the collector's storage of source IPs against a persistent instance id. It cannot carry a product-wide default.

---

## Q3. What consent means, and which mechanisms can carry it

### 3.1 The definition chain

- ePrivacy: "‘consent’ by a user or subscriber corresponds to the data subject's consent in Directive 95/46/EC;" `docs/law/eu/eprivacy/text.html Art. 2(f)`.
- GDPR Art. 94(2): "References to the repealed Directive shall be construed as references to this Regulation." `docs/law/eu/gdpr/text.html Art. 94(2)`.
- GDPR Art. 4(11): "‘consent’ of the data subject means any freely given, specific, informed and unambiguous indication of the data subject's wishes by which he or she, by a statement or by a clear affirmative action, signifies agreement to the processing of personal data relating to him or her;" `docs/law/eu/gdpr/text.html Art. 4(11)`.
- Tw: "toestemming van een gebruiker of abonnee: toestemming van een betrokkene als bedoeld in artikel 4, onderdeel 11, van de Algemene verordening gegevensbescherming, met dien verstande dat de toestemming mede betrekking kan hebben op gegevens van abonnees die geen natuurlijke personen zijn;" `docs/law/nl/telecommunicatiewet/text.html Art. 11.1 onder g`.
- ePrivacy recital 17: "For the purposes of this Directive, consent of a user or subscriber, regardless of whether the latter is a natural or a legal person, should have the same meaning as the data subject's consent as defined and further specified in Directive 95/46/EC. Consent may be given by any appropriate method enabling a freely given specific and informed indication of the user's wishes, including by ticking a box when visiting an Internet website." `docs/law/eu/eprivacy/oj.html Recital 17`.
- Recital 66 (2009/136/EC): "Where it is technically possible and effective, in accordance with the relevant provisions of Directive 95/46/EC, the user's consent to processing may be expressed by using the appropriate settings of a browser or other application."
- GDPR recital 32: "Consent should be given by a clear affirmative act establishing a freely given, specific, informed and unambiguous indication of the data subject's agreement … This could include ticking a box when visiting an internet website, choosing technical settings for information society services or another statement or conduct which clearly indicates in this context the data subject's acceptance of the proposed processing of his or her personal data. Silence, pre-ticked boxes or inactivity should not therefore constitute consent. … If the data subject's consent is to be given following a request by electronic means, the request must be clear, concise and not unnecessarily disruptive to the use of the service for which it is provided." `docs/law/eu/gdpr/oj.html Recital 32`.

### 3.2 Conditions (GDPR Art. 7, applied through Art. 2(f))

- Art. 7(1): "Where processing is based on consent, the controller shall be able to demonstrate that the data subject has consented to processing of his or her personal data."
- Art. 7(2): "If the data subject's consent is given in the context of a written declaration which also concerns other matters, the request for consent shall be presented in a manner which is clearly distinguishable from the other matters, in an intelligible and easily accessible form, using clear and plain language. Any part of such a declaration which constitutes an infringement of this Regulation shall not be binding."
- Art. 7(3): "The data subject shall have the right to withdraw his or her consent at any time. … Prior to giving consent, the data subject shall be informed thereof. It shall be as easy to withdraw as to give consent."
- Art. 7(4): "When assessing whether consent is freely given, utmost account shall be taken of whether, inter alia, the performance of a contract, including the provision of a service, is conditional on consent to the processing of personal data that is not necessary for the performance of that contract."
- Recital 43: "Consent is presumed not to be freely given … if the performance of a contract, including the provision of a service, is dependent on the consent despite such consent not being necessary for such performance." Recital 42: "Consent should not be regarded as freely given if the data subject has no genuine or free choice or is unable to refuse or withdraw consent without detriment."
- Citations: `docs/law/eu/gdpr/text.html Art. 7(1)-(4)`; `docs/law/eu/gdpr/oj.html Recitals 42, 43`.
- Binds: the party relying on consent (here Cadasto, which gains access and receives the report). Art. 7's "controller" wording is applied to Art. 5(3) consent through the Art. 2(f) reference; the text does not spell out how the GDPR controller role maps onto the Art. 5(3) actor. Flagged.
- Timing: Art. 5(3) "has given his or her consent, having been provided with clear and comprehensive information"; Tw lid 1 "is voorzien van … informatie" and "toestemming heeft verleend". Both are in the perfect tense: information and consent come before the storing or access.

### 3.3 Each mechanism against these conditions

| Mechanism | Statement or clear affirmative action? | Informed before the act? | Freely given / separable? | Reading |
|---|---|---|---|---|
| **Default on + INFO boot line + book page** | No. Leaving a default untouched is "inactivity"; GDPR recital 32: "Silence, pre-ticked boxes or inactivity should not therefore constitute consent." A vendor default is the functional equivalent of a pre-ticked box. | No for the first report: the `start` report is sent at boot, the same moment the line is logged. The instance id is stored before any line can be read. | n/a | **Cannot carry consent.** It informs; it does not obtain agreement. |
| **Installer's explicit configuration choice** (`[usage_report] enabled = true`, `FERROEHR__USAGE_REPORT__ENABLED=true`, a chart value set to true by the operator, where the shipped default is false) | Yes: "choosing technical settings" (GDPR rct 32); "appropriate settings of a browser or other application" (2009/136 rct 66). | Yes, if the book page and `ferroehr usage-report --print` are available before the key is set. | Yes, if nothing else depends on it. | **Can carry consent.** |
| **First-run / setup prompt** (e.g. a setup or init step that asks y/N, default N; or a required key with no default that refuses boot until the operator writes true or false) | Yes, an answer "yes" is a statement; a forced explicit choice is an affirmative act either way. A prompt whose default is "yes" is a pre-ticked box (rct 32). | Yes, if the prompt states purposes and points to the payload. Rct 32: the request "must be clear, concise and not unnecessarily disruptive". | Yes. | **Can carry consent**, with the default answer "no". Note the repo practice of checking new boot refusals against the shipped chart values; a required key needs a chart value. |
| **Licence-agreement clause** | A signed or accepted clause is a "statement". | Yes, if the order form carries the information. | Only if the clause is "clearly distinguishable from the other matters" (Art. 7(2)) and the licence is not conditional on it (Art. 7(4), rct 43). A clause that must be accepted to obtain the licence is presumed not freely given. | **Can carry consent only as a separate, optional tick box**, never as a licence term. There is no such vehicle for the non-commercial grant, which is not accepted by an affirmative act in an order flow. |
| **Commercial grant file that carries an opt-in flag** (the customer ticked the separate box; installing the grant turns the report on) | The tick is the statement; installing the grant is the operator's act. | Yes, via the order form. | As for the licence clause. | **Possible**, provided the config switch can still turn it off at any time (Art. 7(3): "as easy to withdraw as to give"). The text does not address this pattern specifically. |

Further silences:
- **Who may consent for an organisation.** Recital 17 and Tw art. 11.1 onder g accept consent of a legal-person subscriber. The text does not say which natural person may give it for the organisation (an administrator acting under instruction, a DPO, a manager). **Silent.** The book page should say that switching the report on is the organisation's decision.
- **Demonstrability (Art. 7(1)).** With an opt-in default, receipt of a report shows the instance was switched on; the text does not prescribe a form of proof. If the software records when and how the switch was turned on and the payload carries that fact, Cadasto can show it. That is a design reading, not a textual requirement.
- **ePrivacy recital 25** (2002) lets "Access to specific website content … be made conditional on the well-informed acceptance of a cookie". It predates the 2009 consent wording and GDPR Art. 7(4); the corpus carries nothing reconciling them. It concerns website content and does not support making a software licence conditional on telemetry.

### Answer to Q3

Consent is the GDPR Art. 4(11) notion, reached through ePrivacy Art. 2(f) and GDPR Art. 94(2), and in the Tw through art. 11.1 onder g, available to legal-person subscribers. It needs a statement or clear affirmative action, after information including the purposes, before the storing or access, revocable as easily as given, and not a condition of an unrelated contract. A deliberate operator configuration choice or an answered setup prompt (default "no") can carry it; a licence clause only as a separate optional tick box. **Default-on with a boot log line cannot carry it**: that is inactivity, and the first report precedes the information.

---

## Q4. Recommendation for FerroEHR's default

### 4.1 The options

| Option | Holds on the vendored text? | Residual risk |
|---|---|---|
| **Keep on by default** | Only if (i) a server is not "terminal equipment" (corpus silent; issue's EDPB 2/2023 §33 reading against), or (ii) the Dutch quality/effectiveness limb applies (Dutch law only, ISS undefined, no Directive counterpart, IP storage strains "geringe gevolgen"), or (iii) Tw lid 1 is read as excluding legal-person servers (the Directive's "subscriber" wording covers them; staff are "gebruikers"). | High: the default would rest on three readings the corpus does not support, for every EU and Swiss operator. |
| **On only for commercial licences, with a contract clause** | Only if the clause is a separate, optional opt-in (Art. 7(2), 7(4), rct 43). A mandatory clause is presumed not freely given. Non-commercial installs need another mechanism anyway. | Medium as a mandatory clause, low as an optional tick box. If optional, it is an opt-in in contract form, compatible with the next row. |
| **First-start prompt** | Yes, with default answer "no" and the information shown before the answer. | Low. Headless and Kubernetes installs have no interactive first start; the chart needs a value, and the prompt cannot be the only route. |
| **Default off; explicit opt-in by config / env / chart value** | Yes. The operator's setting is "choosing technical settings" (GDPR rct 32) / "appropriate settings of … other application" (2009/136 rct 66), given after the book page and `--print` have informed it. | Low; see 4.2. |

**Recommendation: switch the default to off for every licence type, with explicit opt-in.** Offer three equivalent opt-in routes: the config key, the env var and a chart value (shipped default `false`). Optionally add a setup prompt whose default answer is "no", and a separate, optional opt-in tick box on the commercial order form, delivered as a flag in the commercial grant file. The config switch must always be able to turn the report off again. Make the change before the release that ships #3578; #3580's body already says "If the text requires consent, the default changes before the release that ships the report."

What this means for the software (testable):
1. With no explicit opt-in, the software stores no instance id and sends no report, `start` included. The instance id is generated only on opt-in, because generating and storing it is itself "storing of information" for Cadasto's purpose.
2. The shipped `ferroehr.toml` default, the env default and every shipped chart values file (including `deploy/helm/ci/*-values.yaml`) say off. An upgrade never turns the report on.
3. The INFO boot line says the report is off and how to switch it on, pointing at the book page; when on, it says so and how to switch it off. The line is information, not consent.
4. `ferroehr usage-report --print` works with the report off, so the operator can see the exact payload before deciding (Art. 5(3) "clear and comprehensive information"; Tw lid 1 onder a).
5. Turning it off is a single setting, with nothing harder than turning it on (GDPR Art. 7(3)).

### 4.2 Residual risk with the recommended default

- **Authority to consent:** the text is silent on who may give consent for a legal-person subscriber. The installer's act is treated as the organisation's.
- **Personal data at the collector:** the source IP and country are stored with each report. For sole practitioners (and wherever an IP identifies a natural person: GDPR Art. 4(1), recital 30) that is personal data, with the GDPR Art. 13 notice, retention and objection/erasure duties `research-law.md` lists. Recital 26 is now vendored ("all the means reasonably likely to be used, such as singling out"), and recital 14 confirms the Regulation "does not cover the processing of personal data which concerns legal persons"; a hospital's instance id and metrics alone are outside it, the IP is the open point. Opt-in does not remove these duties. Whether the Art. 5(3) consent also serves as the GDPR basis for the IP, or Cadasto keeps Art. 6(1)(f), is a separate choice the book page must state; recital 47 now gives the "reasonable expectations" factor.
- **Germany and Switzerland:** no German transposition and no Swiss counterpart of Art. 5(3) is in the corpus. The recommendation rests on the Directive and the Tw; what those other texts require is outside the corpus.
- **Hosted VMs:** whose terminal equipment a rented VM is, and who the subscriber is, is not settled; opt-in by the operator covers both readings.
- **The parent's acceptance criteria conflict with this ruling** and need rewording: #3577 "A fresh install sends the documented v1 payload within minutes of boot" and the title "on by default". Under the recommendation: a fresh install sends nothing until switched on, then the `start` report within minutes and the `daily` report thereafter.

### 4.3 What the book page must say

Grounded in Art. 5(3) ("clear and comprehensive information … inter alia, about the purposes"), Tw art. 11.7a lid 1 onder a ("in ieder geval over de doeleinden waarvoor deze informatie wordt gebruikt"), GDPR Art. 7(3) and, for the IP, GDPR Art. 13:
1. The report is **off by default**. Nothing is sent and no instance id is created until the operator switches it on. Switching it on is the organisation's decision to send the listed data to Cadasto B.V.
2. Each way to switch it on and off (config key, env var, chart value, setup prompt if built, commercial order-form opt-in if built) and that turning it off is one setting.
3. What the software stores on the instance (the random instance id, in the operator's database) and what it reads and sends: the complete v1 payload schema field by field, the `start` and `daily` schedule, and `ferroehr usage-report --print` to see the exact payload.
4. The purposes, each named (installed versions, upgrade uptake after security releases, field performance; CRA Art. 14 vulnerability handling if that is a stated purpose).
5. The recipient: Cadasto B.V., the collector at report.ferropulse.eu, its host (Hetzner, EU), and that the collector stores the **source IP address and the country** derived from it with each report.
6. Retention, the erasure route by instance id, and how to object or withdraw.
7. For a sole practitioner and anyone whose IP identifies them: the GDPR Art. 13 items (controller identity and contact, legal basis, recipients, retention, rights, complaint right); for Swiss operators the destination country (DSG Art. 19 Abs. 4, per `research-law.md` Q9).
8. Never "anonymous", never "compliant", never that the software "meets" Art. 5(3). Wording: what the software does, the provision it serves (Art. 5(3) ePrivacy, Tw art. 11.7a), and what the operator decides.

---

## Synthesis A: provisions that reach the CDR software (as manufactured by Cadasto)

| Provision | What the software must be able to do | Binds / scope |
|---|---|---|
| ePrivacy Art. 5(3) first sentence (`docs/law/eu/eprivacy/text.html`) | Store nothing and send nothing for the report before an explicit opt-in; make the information available before opt-in (`--print`, book page) | Member States, then via national law the party storing/gaining access (Cadasto through its software); 2009/136/EC Art. 4(1), transposition by 25 May 2011 |
| ePrivacy Art. 5(3) second sentence | No exemption available: the report is not transmission-only and not strictly necessary for a requested service; the software must not rely on one | as above; closed list |
| Tw art. 11.7a lid 1, 2 (`docs/law/nl/telecommunicatiewet/text.html`) | Same as Art. 5(3); lid 2 covers access brought about by installed software | Impersonal: whoever stores or obtains access "via een elektronisch communicatienetwerk" |
| Tw art. 11.7a lid 3 onder b, second limb | Not relied on (undefined ISS, Dutch only, no Directive counterpart) | Dutch law only |
| ePrivacy Art. 2(f) → GDPR Art. 4(11), 7(1)-(4) (via Art. 94(2)); Tw art. 11.1 onder g | Opt-in by an affirmative act; off as easy as on; never a condition of the licence; optionally record when and how opt-in happened | The party relying on consent (Cadasto); consent of legal-person subscribers accepted (ePrivacy rct 17) |

## Synthesis B: provisions that fall on the organisation or a third party only (a page must never claim the software meets them)

| Provision | Who it binds |
|---|---|
| ePrivacy Art. 5(3) as such | Member States ("Member States shall ensure"); persons only through national law |
| The act of giving consent (Art. 5(3) "the subscriber or user concerned has given"; Tw lid 1 onder b) | The operating organisation (subscriber/abonnee) or its staff (user/gebruiker); who may consent for it is not settled |
| GDPR Art. 7(1) demonstrability, Art. 13 notice, retention and erasure for the stored IP | Cadasto B.V. as the collector operator, not the CDR software |
| GDPR Art. 95, recital 173 (relation GDPR / ePrivacy) | Providers of publicly available ECS in public networks; not the CDR and not the operator |
| Tw art. 11.2 | Providers of public electronic communications networks and services only |
| Tw art. 11.7a lid 5 | Information society services provided by public-law legal persons |

## Silences, listed

1. "Terminal equipment" / "randapparatuur": undefined in the vendored text (ePrivacy Art. 2 imports from Directive 2002/21/EC, not vendored; the Tw has no definition). Whether a server is covered is not settled by the corpus. EDPB 2/2023 (§33 per the issue) would add weight; not vendored.
2. "Subscriber": undefined in ePrivacy (imported, not vendored); defined in Tw art. 1.1 as "abonnee". Whose equipment a hosted VM is: not settled.
3. "Information society service": defined only by reference to Directive (EU) 2015/1535 (GDPR Art. 4(25), Tw art. 1.1), not vendored.
4. Relation between Art. 3 (personal data, public ECS) and Art. 5(3) ("information", any actor): not reconciled in the text.
5. Tw art. 11.7a lid 1 "gebruiker" (natural person only) versus the Directive's "subscriber or user": not reconciled in the text.
6. Validity or reach of the Dutch quality/effectiveness limb against the Directive's closed list: not addressed in the corpus.
7. Territorial reach of Tw art. 11.7a: not stated in the provisions read.
8. Who may give consent for a legal-person subscriber: not stated.
9. German and Swiss counterparts of Art. 5(3): not vendored.
10. Any AMvB under Tw art. 11.7a lid 6: not vendored.

---

## En-route findings

1. `.claude/agent-memory/compliance-researcher/corpus-coverage-gaps.md:10-11`: the memory says the GDPR has no recitals and that Directive 2002/58/EC and the Telecommunicatiewet are not vendored; all three are now in the tree (`docs/law/eu/gdpr/oj.html` with `id="rct_1"`..`rct_173`, `docs/law/eu/eprivacy/`, `docs/law/nl/telecommunicatiewet/`). Not edited here (read-only task); the memory needs updating.
2. `docs/law/nl/telecommunicatiewet/PROVENANCE.md` (whole record): it does not mention that the publisher marks pending amendments without a commencement date ("Wijziging(en) zonder datum inwerkingtreding aanwezig", `text.html:726` for the act, `:793` for art. 1.1, `:12655` for art. 11.1, the two definition articles 11.7a depends on). A reader of the definitions is not warned that changed wording is enacted but not in force.
3. Definitions that the ePrivacy text imports by reference are absent from the corpus: `docs/law/eu/eprivacy/text.html Art. 2` first paragraph points to Directive 2002/21/EC (terminal equipment, subscriber), and GDPR Art. 4(25) / Tw art. 1.1 point to Directive (EU) 2015/1535 Art. 1(1)(b) (information society service). Without them, Q1 and Q2 of this adjudication end in "the text is silent" on their central terms. Candidate for a vendoring issue, together with the EDPB Guidelines 2/2023 the issue body relies on.
4. `docs/plans/usage-report/research-law.md:443` recommends "Not storing the IP at the collector (or proxy/LB logs)"; #3577's ruling (owner decision 2026-10-05) stores the source IP and country. The plan file and the ruling disagree; the book page and the GDPR notice follow the ruling, so the research's Must-1 is stale.
5. `docs/plans/usage-report/research-law.md:7, 47, 443` still describe a licence id that names the licensee as part of the payload; #3577 rules the licence is reported as grant type only. The banner at line 3 covers only the newly vendored texts, not this change.
6. `docs/plans/usage-report/research-law.md:16, 36-38, 57, 111, 457, 467`: statements that GDPR recitals 14, 26, 30, 47 and ePrivacy/Tw are not in the corpus are now false; recitals 14 and 26 answer open point 1 in part (legal-person data outside the Regulation; the "means reasonably likely" test). The line-3 banner names the change, but the Q1 answer and open points 1 and 11 are not updated.
7. #3577 body (GitHub issue, title and first acceptance criterion): "on by default" and "A fresh install sends the documented v1 payload within minutes of boot" conflict with this adjudication; they need rewording if the owner adopts the opt-in default.

## Suggested memory updates (not written; the task was read-only)

- corpus-coverage-gaps: GDPR recitals now in `eu/gdpr/oj.html` (articles still from `text.html`); ePrivacy in `eu/eprivacy/` (articles `text.html`, 2002 recitals `oj.html`, 2009 recitals incl. 66 in `amending-directive-2009-136.html`); Tw in `nl/telecommunicatiewet/text.html`. Still absent: Directive 2002/21/EC and 2015/1535 definitions, EDPB 2/2023, DE/CH counterparts of Art. 5(3).
- corpus-navigation: Tw ids `Hoofdstuk1_Artikel1.1`, `Hoofdstuk11_Paragraaf11.1_Artikel11.1`, `..._Artikel11.7a`; Tw art. 1.1 definitions are unlettered entries split by ";"; "randapparatuur" is undefined in the Tw. The 2002/58 OJ HTML has no `rct_` ids (old EUR-Lex layout: recitals are `<p>(N) …</p>`).
