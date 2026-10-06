# Getting help

Four destinations, and they are not interchangeable. Picking the right one is
the difference between an answer and a thread nobody is paged for.

## I have a question

**[GitHub Discussions](https://github.com/FerroHEALTH/FerroEHR/discussions)** —
how to configure something, whether an approach fits, what an openEHR concept
means in this implementation, why a design is the way it is.

Read first, because the answer is often already written and is more precise
than a reply:

- **[The documentation site](https://ferroehr.eu/):** installation, the
  configuration reference, the API walkthroughs, AQL, templates, security and
  multi-tenancy, operations. Start at
  [Getting started](https://ferroehr.eu/docs/latest/getting-started.html).
- **[The API reference](https://sandbox.ferroehr.eu/ferroehr/rest/swagger-ui):**
  the live sandbox's Swagger UI. The server generates the document from its own
  handlers, so it describes the surface that actually exists. Sign in with the
  public demo credentials `ferroehr` / `ferroehr`.
- **[`docs/architecture.md`](docs/architecture.md):** the design, in one file.

There is no commercial support offering and no service-level agreement, and no
feature is held back for a paid tier. Answers come when the maintainer is at a
keyboard
([MAINTAINERS.md](MAINTAINERS.md) is honest about how many keyboards that is).

A commercial licence or any other business question goes to Cadasto B.V., the
Licensor: write to [info@cadasto.com](mailto:info@cadasto.com) or use
<https://www.cadasto.com/contact/>.

## I found a defect

**[Open an issue](https://github.com/FerroHEALTH/FerroEHR/issues/new/choose)**
— something is wrong, missing, or contradicts the openEHR specifications.

The reports that get fixed fastest carry:

- the version (`ferroehr --version`, or the image tag), and how it is deployed;
- the request and the response, verbatim (method, path, headers, bodies),
  or the AQL and the result set;
- what the openEHR specification says should have happened, with the
  file and section from `docs/specs/openehr/` if you have it. A citation turns
  a disagreement into a defect.

**A specification-conformance report is the most valuable kind here.** The implementation is never presumed correct because it
was written to the specification; the specification text is the authority and
the implementation is the usual culprit.

## I found a vulnerability

**Do not open a public issue.** Follow [SECURITY.md](SECURITY.md): report
privately through
[GitHub private vulnerability reporting](https://github.com/FerroHEALTH/FerroEHR/security/advisories/new).

That document also carries what you can expect in return (an acknowledgement
window, an assessment window, coordinated disclosure, safe harbour for
good-faith research, and credit by default), plus what to do if the
acknowledgement does not arrive.

**A vulnerability in Kubernetes itself, or in a database, broker or terminology
server you deployed alongside FerroEHR, goes to that project**, not here.
SECURITY.md § *Reporting a vulnerability in Kubernetes itself* has the routing.

## I want to make a complaint, or report harm

Cadasto B.V. is FerroEHR's manufacturer under Regulation (EU) 2025/327 and
Regulation (EU) 2024/2847. Write to [info@cadasto.com](mailto:info@cadasto.com)
with "FerroEHR complaint" in the subject, or open an issue when the complaint
can be public. Anything that harmed a person, or could have, goes to the same
address with "FerroEHR incident" in the subject. Every complaint enters the
[register of complaints](docs/registers/complaints.tsv), whatever the
channel. The book's
[Complaints, incidents and vulnerabilities](https://ferroehr.eu/docs/latest/compliance/post-market.html)
page says what happens next. Never include patient data.

## I want to change something

[CONTRIBUTING.md](CONTRIBUTING.md) is the practical guide: setup, the gates
every pull request must pass, and the hard rules. [GOVERNANCE.md](GOVERNANCE.md)
is how the decision gets made and how someone becomes a maintainer.

## What you are entitled to

Nothing, and that is worth saying plainly. FerroEHR is source-available
software under the Business Source License 1.1, provided as-is, by
volunteers, with no warranty. Read the [LICENSE](LICENSE), which says exactly
that in the language that binds.
Everything above describes what the project *intends* to do, and the intent is
sincere; none of it is a contractual commitment, and only the security-report
windows in SECURITY.md are stated as promises at all.

Only the newest release receives fixes ([SECURITY.md § Supported
versions](SECURITY.md#supported-versions)). The manufacturer's duties under
Regulation (EU) 2025/327 and Regulation (EU) 2024/2847 are set by those
Regulations, not by this file. If your deployment needs a stronger
guarantee than a single-maintainer project can give, the honest options are to
fork and maintain, or to fund the capacity that would change the answer.
