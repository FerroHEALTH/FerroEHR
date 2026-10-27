# S3 multimedia

Clinical records sometimes carry large binary attachments (scanned documents,
images, waveforms) as `DV_MULTIMEDIA` values. Keeping big blobs inline in the
database bloats storage and slows queries. FerroEHR can offload large multimedia
blobs to any S3-compatible object store, keeping a small content-addressed
reference in the composition, and re-materialize them on demand when a record is
read back.

<!-- toc -->

## How offload works

Offload is a commit-path transformation applied to `DV_MULTIMEDIA` nodes anywhere
in the tree, including a node's nested `thumbnail`, which is itself a multimedia
value:

1. A node **qualifies** only when it is purely inline (it has `data` and no
   `uri`) and its **decoded** byte length is **strictly greater** than the
   configured threshold. A value at or below the threshold stays inline; a value
   that already references external media is stored verbatim, never touched.
2. The raw decoded bytes are written to the object store under a key that is the
   SHA-256 hash of those bytes, in lowercase hex. Because the key is the content
   hash, identical blobs deduplicate automatically and an upload whose key already
   exists is a no-op.
3. The node is rewritten in place: inline `data` is removed and replaced with a
   `uri` of the form `s3://<bucket>/<hash>`, plus an `integrity_check` (the same
   SHA-256 digest), an `integrity_check_algorithm` code phrase (`SHA-256`), and
   `size` set to the decoded byte length.

Uploads happen before anything is persisted, so a failed upload aborts the commit
and a record is never half-stored, so the version count is unchanged.

> [!NOTE]
> What lives where afterwards: the object store holds the blob bytes; the
> composition in PostgreSQL holds a compact, spec-legal `DV_MULTIMEDIA` that
> points at the blob by content hash. Everything stays canonical openEHR JSON:
> the `s3://` reference and the integrity fields are standard Reference Model
> attributes, not FerroEHR inventions. The digest appears twice in two
> encodings, because the model asks for it that way: `integrity_check` is a byte
> array, so canonical JSON renders it base64, while the `uri` spells the same
> digest as lowercase hex.

## Reading blobs back

By default a read returns the stored form, the compact reference. To get the
bytes back inline, ask for expansion on the read with `?expand_multimedia=true`.
It is available on the reads that can return externalized content: composition
and versioned-composition reads, `EHR_STATUS` and its versioned reads, and
directory (folder) reads.

The server fetches each of *its own* externalized blobs (only URIs of the exact
form `s3://<configured-bucket>/<hash>` count as its own, so a foreign `https://`
or other-bucket reference is left alone) **verifies the SHA-256 of the fetched
bytes against the key**, and only then re-inlines the `data`. A hash mismatch is a
hard error, so a corrupted or tampered blob is never quietly served.

An expanded value keeps its `uri` and integrity fields alongside the restored
`data`: it is both inline and external, which is spec-legal and means a
subsequent commit of that same body re-offloads cleanly.

If the object store is unreachable, both halves of the path fail loudly rather
than losing content: a commit that needs to offload is refused `500` and nothing
is written, and an expanded read of an already-offloaded record is refused `500`. A
read **without** `expand_multimedia` still answers `200`; it never touches the
store.

## Turning it back off

`enabled` governs **new offloads**, not access to old ones. Switching it back to
`false`:

- New commits keep large `DV_MULTIMEDIA` **inline**, byte-identical, with no
  dependency on the object store. Nothing else about the request or the record
  changes.
- Records that were **already** offloaded keep their `s3://` reference and stay
  fully readable: `?expand_multimedia=true` still fetches, verifies and re-inlines
  them, as long as an `endpoint` is still configured. Content this server
  externalized does not become unreachable because a switch was flipped.

> [!WARNING]
> **Removing the `endpoint` as well is the decision that matters.** With no store
> reachable at all, an expansion request against an already-offloaded record
> **fails**, and does not quietly answer `200` with the compact reference. The
> bytes are still in your bucket and still reachable with an S3 client; the API
> refuses rather than pretending the request was honoured. So decide what happens
> to the blobs already in the bucket before you remove the endpoint, not before
> you flip `enabled`.

One slim-build caveat: a binary built without the `multimedia` feature has no
object-store code at all, so it serves the stored compact reference and does not
refuse the expansion. Do not read externalized records with a slim binary.

## Enabling it

Offload is off by default. The keys live under `[multimedia]`; the full table with
every default and its meaning is on
[Integrations](../installation/config-integrations.md#multimedia). The essentials:

| Environment variable | Default | Meaning |
|---|---|---|
| `FERROEHR__MULTIMEDIA__ENABLED` | `false` | master switch |
| `FERROEHR__MULTIMEDIA__THRESHOLD_BYTES` | `262144` (256 KiB) | offload blobs larger than this; smaller stay inline |
| `FERROEHR__MULTIMEDIA__ENDPOINT` | unset | S3 endpoint URL; unset uses default AWS endpoint resolution |
| `FERROEHR__MULTIMEDIA__BUCKET` | `openehr-multimedia` | target bucket |
| `FERROEHR__MULTIMEDIA__REGION` | `us-east-1` | S3 region, required even for non-AWS endpoints |
| `FERROEHR__MULTIMEDIA__ACCESS_KEY_ID` | unset | access key id |
| `FERROEHR__MULTIMEDIA__SECRET_ACCESS_KEY` | unset | secret key (or `…__SECRET_ACCESS_KEY_FILE` for a mounted secret) |
| `FERROEHR__MULTIMEDIA__ALLOW_HTTP` | `false` | permit plain-HTTP endpoints, development only |

With both the access key and the secret unset, the client runs unsigned
(anonymous), the mode a local development SeaweedFS gateway accepts with no
credentials. Set both to make signed requests against a real store.

An enabled integration is refused at **boot** when its `endpoint` is set but
blank, relative, or carrying a scheme other than `http` or `https`. That case is
easy to reach by accident (an unset Compose variable expanding to nothing, an
empty Helm value, a bare `host:port`) so it is refused where an operator can
still act on it, rather than at the first multimedia commit.

> [!WARNING]
> **The bucket must already exist.** The server never creates it, and a missing
> bucket is not distinguishable on the wire: S3 answers a `PUT` into a bucket that
> does not exist with `403 AccessDenied`, not `404 NoSuchBucket`. So every
> multimedia commit fails `500` and the log says `Access Denied`, which reads
> like a credentials problem and is not one. Create the bucket before you enable
> the integration; the SeaweedFS recipe below shows the one-liner.

> [!WARNING]
> Offloaded blobs are PHI. In production the bucket must be private, encrypted,
> and reached over HTTPS (`FERROEHR__MULTIMEDIA__ALLOW_HTTP=false`). Prefer
> instance or workload identity over static keys where your platform supports it.
> See [Operations](../operations.md) for the deployment-side security posture.

> [!NOTE]
> Externalization is also a **cargo feature** (`multimedia`), on in the published
> images and any default build. A slim `--no-default-features` build contains none
> of the object-store code and **refuses to boot** with `multimedia.enabled = true`
> rather than silently storing every blob inline; see
> [From source → Build features](../installation/from-source.md#build-features).

### On Kubernetes

Every key above is reachable as `config.multimedia.*`
([Any server setting is reachable](../installation/kubernetes.md#any-server-setting-is-reachable));
the S3 credentials go through `secrets.*`, which the chart mounts as files:

```yaml
# values.yaml
config:
  multimedia:
    enabled: true
    endpoint: https://s3.example.com
    bucket: openehr-multimedia
    threshold_bytes: 262144
secrets:
  multimediaAccessKeyId: "AKIA…"
  multimediaSecretAccessKey: "…"
```

**Before you enable it:** the bucket must exist (nothing in the chart creates it)
and, if the chart's default-deny egress policy is on, an egress rule must admit
the endpoint. **To turn it off**, set `config.multimedia.enabled: false` and leave
the `endpoint` in place so already-externalized content stays readable (see
[Turning it back off](#turning-it-back-off)).

## Quick setup with SeaweedFS

Any S3-compatible store works: AWS S3, MinIO, SeaweedFS. SeaweedFS is a light
option for development and testing: its S3 gateway needs no credentials.

**Step 1, create the bucket.** The gateway starts with no buckets at all, and
nothing in it creates one. Against an unauthenticated development gateway a bare
`PUT` on the bucket path is enough:

```bash
curl -X PUT http://127.0.0.1:8333/openehr-multimedia
curl -s http://127.0.0.1:8333/            # the bucket now appears in ListAllMyBuckets
```

The [Compose stack](../installation/compose.md) does this for you: the `s3`
profile brings up the gateway plus a `seaweedfs-init` service that performs
exactly that `PUT` once the gateway is healthy, reading the same bucket variable
the server does so the two cannot disagree. This step is only for a gateway you
run yourself.

**Step 2, point the server at the gateway** and allow plain HTTP for local use:

```bash
export FERROEHR__MULTIMEDIA__ENABLED=true
export FERROEHR__MULTIMEDIA__ENDPOINT=http://127.0.0.1:8333
export FERROEHR__MULTIMEDIA__BUCKET=openehr-multimedia
export FERROEHR__MULTIMEDIA__ALLOW_HTTP=true
```

The same exports drive the Compose stack, which passes the whole
`FERROEHR__MULTIMEDIA__*` set through from your shell; only the endpoint changes,
to the in-network hostname `http://seaweedfs:8333`.

**Step 3, check what the server actually took.** `/management/env` reports the
effective configuration, which is the quickest way to catch a variable that never
arrived:

```bash
curl -s -u ferroehr:ferroehr http://localhost:8080/management/env | jq .multimedia
```

The secret access key is masked there. `access_key_id` is **not** masked, and
deliberately so: it is an identifier rather than a credential, and seeing it is
how you confirm which key the server is actually using.

With the switch on and the bucket present, large `DV_MULTIMEDIA` values committed
through the ordinary composition APIs (see
[Using the API](../using-the-api/index.md)) are offloaded automatically. Nothing
about the request or the stored record changes except the size of what lives in
the database.

> [!NOTE]
> Base64 inflates a blob by about a third on the wire, and the whole request body
> is still subject to the request-body limit (`rest.body_limit_bytes` on the
> clinical surface, `server.body_limit_bytes` everywhere); a
> composition that exceeds it is refused `413` before offload is ever considered.
> See [`[server]`](../installation/config-server.md#server).
