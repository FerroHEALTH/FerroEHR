# Usage report

Every FerroEHR instance sends a small JSON report to FerroPULSE, a collector
that Cadasto B.V. runs. It goes out at each process start and once a day. It
says that the instance is alive, which version it runs, under which licence
grant type, and how fast it answers, in coarse aggregates. The report is on by
default. Every boot logs one INFO line that says whether it is on and how to
switch it off.

This page lists every field the report carries, says when it is sent and how to
print it before it is sent, gives the switch, and carries the privacy notice for
the data the collector stores. No openEHR specification governs any of it; the
report is FerroEHR's own design.

<!-- toc -->

## When a report is sent

There are two kinds of report, and both are one HTTPS `POST` to
`https://report.ferropulse.eu/v1/report` with a JSON body of at most 64 KiB.

- **`start`:** at each process start, once the server has connected to its
  database. An instance sends at most one start report per ten minutes,
  however many replicas start: the time of the last one is kept in the
  database. A pod in a crash loop sends one start report per ten minutes, not
  one per restart.
- **`daily`:** the first attempt 10 minutes after boot, then roughly every
  24 hours (23.5 to 24.5 hours after the previous one, spread at random). One
  replica sends it for the whole instance. Every replica adds its counts to a
  window kept in the database, and the replica that claims the report sends
  the totals and empties the window. A replica can claim it only when no daily
  report went out in the last 23 hours, so a restart does not add one.

The boot line, with the report on, reads:

```text
usage report ON: instance id, version, licence type, deployment, uptime, host size and coarse performance aggregates, at each start and once a day; no patient data. Off: FERROEHR__USAGE_REPORT__ENABLED=false. Details: https://ferroehr.eu/docs/latest/usage-report.html
```

It carries the configured endpoint as the structured field `endpoint`. With the
report off, the line reads:

```text
usage report OFF (usage_report.enabled = false): nothing is sent. Details: https://ferroehr.eu/docs/latest/usage-report.html
```

## What a report contains

Every report carries these fields:

| Field | Sent in | What it is |
|---|---|---|
| `schema` | both | Always `1`, the version of the report format. |
| `event` | both | `start` or `daily`. |
| `first_start` | `start` | `true` when this start created the instance id, which marks a new installation; `false` on every later start. |
| `product` | both | Always `ferroehr`. |
| `instance_id` | both | A random UUID (version 4), created at the first start and stored in the database. It is not derived from the host, the licence or anything else about the deployment. Every replica shares it, and a restore of the database keeps it. |
| `version` | both | The server version, for example `4.3.4`. |
| `git_sha` | both | The commit the binary was built from, 7 to 40 hex digits. A build that recorded no commit, such as one from a source tarball, sends `0000000`. |
| `spec_profile` | both | The [`spec_profile`](installation/configuration.md#spec_profile) setting: `development` or `stable`. |
| `licence` | both | The licence grant type: `commercial` when an installed licence permits commercial use, `non-commercial` otherwise. Never the licence id. |
| `deployment` | both | How the instance was deployed: `helm`, `compose`, `binary` or `unknown`. It comes from `[usage_report] deployment`, which the Helm chart sets to `helm`. |
| `uptime_s` | both | Seconds since this process started. |
| `db_ok` | both | Whether the database answered the health check. |
| `migrations_ok` | both | Whether the database carries this build's migrations. |
| `postgres_major` | both | The PostgreSQL major version, for example `18`. |
| `cpu_bucket` | both | The CPUs available to the process, as a range: `1-2`, `3-4`, `5-8`, `9-16` or `17+`. |
| `memory_bucket` | both | The memory available to the process in GiB, the smaller of the host's memory and the container limit, as a range: `<4`, `4-8`, `8-16`, `16-32` or `32+`. Off Linux the server cannot read it and sends `<4`. |
| `metrics` | `daily` | The performance aggregates below. |

A start report, formatted here for reading (the server sends it on one line):

```json
{
  "schema": 1,
  "event": "start",
  "first_start": true,
  "product": "ferroehr",
  "instance_id": "5f0c6b2e-3d4a-4c8b-9e1f-2a3b4c5d6e7f",
  "version": "4.3.4",
  "git_sha": "1a2b3c4d",
  "spec_profile": "stable",
  "licence": "non-commercial",
  "deployment": "helm",
  "uptime_s": 0,
  "db_ok": true,
  "migrations_ok": true,
  "postgres_major": 18,
  "cpu_bucket": "3-4",
  "memory_bucket": "8-16"
}
```

### The daily metrics

The daily report adds one object that covers the window since the previous
daily report:

```json
"metrics": {
  "window_24h": {
    "requests_bucket": "1k-10k",
    "routes": {
      "composition": {
        "p50_ms": 12.5, "p95_ms": 80.1, "p99_ms": 240.0,
        "errors_5xx": 0,
        "histogram": [10, 40, 120, 300, 90, 30, 8, 2, 0, 0, 0]
      }
    },
    "aql": {
      "executions": 1200, "slow": 3, "p95_ms": 310.0,
      "histogram": [0, 5, 40, 200, 500, 300, 120, 30, 4, 1, 0]
    }
  }
}
```

- **`requests_bucket`:** the number of requests the REST API served in the
  window, as a range: `0`, `1-100`, `100-1k`, `1k-10k`, `10k-100k` or
  `100k+`. Each range includes its lower bound and excludes its upper one, so
  `1-100` means 1 to 99.
- **`routes`:** one entry per group of endpoints that served at least one
  request. A request is placed in a group by the route template it matched,
  never by the path the client sent. The groups are `ehr` (EHR and
  `EHR_STATUS`), `composition` (compositions and their version history),
  `contribution`, `query` (query execution), `definition` (templates and
  stored queries), `directory`, `admin`, and `other` for everything else,
  such as demographics, messaging and terminology.
- **`p50_ms`, `p95_ms`, `p99_ms`:** the median, 95th and 99th percentile
  latency of the group in milliseconds, estimated from its histogram.
- **`errors_5xx`:** the responses with a 5xx status.
- **`histogram`:** 11 request counts, one per latency bucket. The bucket upper
  bounds in milliseconds are 5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000,
  and no bound for the last. Counts add up across instances, so a fleet
  percentile comes from summed histograms.
- **`aql`:** AQL execution over the window. `executions` counts the queries
  that reached execution, `slow` the executions that took longer than
  [`slow_aql_ms`](installation/config-server.md#usage_report) (its default
  is in the configuration reference), and `p95_ms` and `histogram` are as above.

## What a report never contains

A report carries no patient data, no EHR id or any other id taken from a
request, no request path, no AQL text or query parameter, no error message, no
audit-log content, no user name, no client IP address, no licence id, no host
name and no customer or staff name. The report types in the server have no
field that could hold any of these, and the collector refuses a report with a
member it does not know.

The collector does record the IP address the report arrives from. See
[What the collector stores](#what-the-collector-stores).

## See the report before it is sent

`ferroehr usage-report --print` prints the exact JSON body the instance would
send now, and sends nothing. Add `--event daily` for the daily report; the
default is `--event start`. The command loads the same configuration as the
server and connects to the database to read the instance id and the shared
window, to which the running replicas add their counts every five minutes. It
writes nothing. Run it where the server runs:

```shell
# Kubernetes (the image has no shell, so call the binary by its path)
kubectl -n ferroehr exec deploy/ferroehr -- /usr/local/bin/ferroehr usage-report --print

# Docker Compose
docker compose exec ferroehr /usr/local/bin/ferroehr usage-report --print --event daily
```

The JSON goes to stdout on one line. Notes go to stderr: when the database
holds no instance id yet, the id shown is a sample, and the first start creates
the real one; with the report off, the command says so. It works with the
report off, and then shows the instance id stored at an earlier start. That id
is what you give Cadasto to have stored reports erased (see
[Your right to object](#your-right-to-object)).

## Switch it off

Any one of these stops the report:

```toml
[usage_report]
enabled = false
```

```shell
FERROEHR__USAGE_REPORT__ENABLED=false
```

```yaml
# Helm values
usageReport:
  enabled: false
```

Under the Helm chart the switch is `usageReport.enabled` (on the command line,
`--set usageReport.enabled=false`). The chart writes it into the rendered
configuration together with `deployment = "helm"`, and it refuses to render
when `config.usage_report.enabled` or `config.usage_report.deployment` is also
set.

With the report off, the server sends nothing to FerroPULSE and writes nothing
to its database for the report, and the boot line says `usage report OFF`.
Nothing else in the server changes. Switching it off stops new reports only.
Reports already stored at the collector stay until the retention period ends,
unless you ask for them to be erased.

## Network

- **Destination:** outbound HTTPS to `report.ferropulse.eu`, port 443. The
  `endpoint` key moves it; the server refuses an endpoint that is plain HTTP
  (except to a loopback host) or that carries credentials.
- **One direction only:** the server reads the status code of the answer and
  nothing else. It does not follow redirects, and nothing the collector answers
  changes the server's configuration or behaviour.
- **Failures stay local:** a send gives up after 5 seconds without a
  connection or 10 seconds in total. A failure is logged once at DEBUG. Boot,
  the health and readiness probes, and request handling never wait on the
  report, so a slow or unreachable collector affects none of them. A daily
  attempt that failed before it claimed the window tries again an hour later.
  A claimed window whose send fails is dropped, not resent.
- **NetworkPolicy egress:** with the chart's
  `networkPolicy.egress.enabled` on, the pod needs a rule in
  `networkPolicy.egress.rules` that admits `report.ferropulse.eu` on port 443.
  A NetworkPolicy cannot match a DNS name, so the rule is an `ipBlock`; see
  [Egress](installation/hardening-network-policy.md#egress-deny-by-default-and-what-it-breaks).
  Without the rule the report fails quietly and nothing else is affected. If
  you do not want the report, switch it off: a blocked report is still
  attempted at every start and every day.

## Privacy notice

This section gives the information that Articles 13 and 14 of the
[GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) list, for the personal
data a report and its stored record can contain. A report names no person.
It can still relate to one: the collector stores the address each report
comes from, and an instance run by one person, such as a practitioner on their
own internet connection, can be linked to that person through the address or
through the instance id. Where that is so, the report is personal data, and
this notice applies to it.

This page is also part of the user information Cadasto supplies with FerroEHR
under Annex II of the
[Cyber Resilience Act](https://eur-lex.europa.eu/eli/reg/2024/2847/oj): it
names the outbound connection, what it carries, and how to switch it off.

### Controller

Cadasto B.V., the Netherlands. Write to
[info@cadasto.com](mailto:info@cadasto.com) or use
<https://www.cadasto.com/contact/>. Cadasto has not appointed a data
protection officer.

### Purposes and lawful basis

Cadasto processes the reports for three purposes:

- to know which FerroEHR versions run in the field, so it can tell how many
  installations a security release concerns and whether they upgrade;
- to know how many installations run under each licence grant type;
- to see how the product performs in real use, so it can find and fix slow
  paths.

The lawful basis is Art. 6(1)(f) GDPR, legitimate interest. The interest is
Cadasto's, as the maker of FerroEHR: keeping the product it publishes secure
and maintained, which needs the versions in use and the reach of each security
release, and improving it, which needs the licence mix and field performance.
The data come from the FerroEHR instance itself. You are not obliged to send
them: the report can be switched off, and switching it off changes nothing else
in FerroEHR.

### What the collector stores

- Every report as it arrives, with the fields listed above.
- With each report: the time of receipt, the UTC day, the source IP address
  (IPv4 or IPv6), and the country derived from that address.
- Per instance id: when it was last seen, its current version, and whether it
  is alive.
- Per day: latency totals summed across the whole fleet, without instance ids.

The country comes from an IP-to-country database bundled with the collector,
so no address leaves the collector host for the lookup. IP geolocation by
[DB-IP](https://db-ip.com), licensed
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). To limit request
rates, the collector also keeps a short counter per source address in memory,
which is lost when it restarts.

### Recipients and location

Hetzner Online GmbH (Gunzenhausen, Germany) hosts the collector in its cloud
and processes the reports as Cadasto's processor, under a data processing
agreement Cadasto has signed (Art. 28 GDPR). The collector runs in Hetzner's
data centre in Falkenstein, Germany. The FerroPULSE dashboard that shows the
reports sits behind a GitHub login with no public or anonymous access; only
members of the FerroHEALTH organisation can sign in.

### Retention

A daily job deletes reports and start reports 13 months after they were
received, an instance record 13 months after the instance was last seen, and
the fleet-wide daily totals 13 months after their day. Backups of the
collector's database are kept for 14 days, so a deleted report can remain in a
backup for at most 14 days more.

### Your right to object

> [!IMPORTANT]
> **You have the right to object** at any time, on grounds relating to your
> particular situation, to the processing of reports that are personal data
> concerning you (Art. 21(1) GDPR).
>
> **To stop future reports**, switch the report off as described in
> [Switch it off](#switch-it-off). No request to anyone is needed.
>
> **To have the reports already stored erased**, write to
> [info@cadasto.com](mailto:info@cadasto.com) with the instance id that
> `ferroehr usage-report --print` shows. Cadasto then deletes every report,
> start report and instance record stored for that instance id, source
> addresses included (Art. 17(1)(c) GDPR). Switch the report off first, or the
> next report starts a new record.
>
> The fleet-wide daily totals hold no instance ids, so an erased instance's
> share in days already summed stays in those totals.

### Your other rights

At the same address you can ask for access to the reports stored for your
instance id (Art. 15), for their rectification (Art. 16) or erasure (Art. 17),
or for restriction of their processing (Art. 18). You have the right to lodge
a complaint with a supervisory authority, in particular in the Member State of
your habitual residence, place of work or place of the alleged infringement
(Art. 77).

### Operators in Switzerland

Under the Swiss Federal Act on Data Protection
([DSG](https://www.fedlex.admin.ch/eli/cc/2022/491/de) Art. 19 Abs. 4), these
are the states your report's data are disclosed to: **Germany**, where the
Hetzner data centre in Falkenstein runs the collector, and **the Netherlands**,
where Cadasto B.V. is established. Both are listed in Annex 1 of the
Swiss Data Protection Ordinance
([DSV](https://www.fedlex.admin.ch/eli/cc/2022/568/de)) as states whose
legislation provides adequate protection (DSG Art. 16 Abs. 1).
