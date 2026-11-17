- The multimedia blob store now runs on Apache OpenDAL, and its S3 settings
  have moved to their own table. **Upgrade:** move `url`, `bucket`, `region`,
  `access_key_id` and `secret_access_key` (or `secret_access_key_file`) from
  `[record.multimedia]` to `[record.multimedia.s3]`, and rename their
  environment variables from `FERROEHR__RECORD__MULTIMEDIA__<KEY>` to
  `FERROEHR__RECORD__MULTIMEDIA__S3__<KEY>`. A configuration that still sets
  them under `[record.multimedia]` is refused at boot. The Helm chart moves to
  39.0.0 and renders the new names. New keys under `[record.multimedia]`:
  `backend` (`"s3"`, the one backend for now), `max_concurrent_requests`
  (default 64), `request_timeout` (default `"30s"`), `part_size` (default
  `"8MiB"`) and `part_concurrency` (default 4). A commit now uploads its blobs
  before it takes a database connection, so no transaction waits on the blob
  store. Blobs larger than `part_size` upload in parallel parts. Every upload
  and read is checked against the blob's SHA-256 key and declared size as the
  bytes stream, so the bytes are never read a second time, and a dump load
  refuses an archived blob whose bytes do not match its name. A failed request
  to the store is retried at most twice. The server reads S3 credentials from
  the configuration only, never from `AWS_*` variables, profile files or
  instance metadata. `object_store`, and with it `chrono` (RUSTSEC-2026-0543)
  and `humantime` (RUSTSEC-2026-0542), is gone from the build.
