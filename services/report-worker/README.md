# awase report worker

Cloudflare Workers + R2 based private intake endpoint for awase tray bug reports.

## Implemented endpoint

- `POST https://report.awase.cc/v1/reports`
- Accepts the schema documented in `payload-schema.md` with `schema_version: 3`.
- Rejects request bodies over 512 KiB.
- Applies a per-IP daily rate limit of 20 reports/day using KV. The IP address is hashed before it is used in the KV key and is not stored in the R2 report object.
- Writes reports only with `env.REPORT_BUCKET.put(...)`. The Worker does not call R2 `get`, `list`, or `delete`.
- Stores objects under server-generated keys such as `reports/2026/08/<report_id>.json`.

## Local commands

```sh
pnpm install
pnpm test
pnpm typecheck
pnpm dev
```

Do not use `npm install`, `npm add`, or yarn in this repository.

## Manual Cloudflare setup

This repository change does not perform any real Cloudflare operation. Before deployment, do these steps manually:

1. Log in:

   ```sh
   wrangler login
   ```

2. Create a private R2 bucket:

   ```sh
   wrangler r2 bucket create awase-report-bucket
   ```

3. Create a KV namespace for rate limiting:

   ```sh
   wrangler kv namespace create RATE_LIMIT_KV
   ```

4. Edit `wrangler.toml` and replace:

   - `account_id`
   - `bucket_name`
   - `kv_namespaces[0].id`

5. Enable `report.awase.cc` as a Workers custom domain in the Cloudflare dashboard or with `wrangler`. The `routes` entry in `wrangler.toml` documents the intended hostname, but custom domain activation is still a separate Cloudflare configuration step.

6. Configure R2 lifecycle deletion. A 90-day retention period is a reasonable starting point for these bug reports:

   ```sh
   pnpm wrangler r2 bucket lifecycle add \
     awase-report-bucket \
     delete-old-reports \
     reports/ \
     --expire-days 90
   ```

   This follows the Wrangler R2 lifecycle command form documented by Cloudflare: `r2 bucket lifecycle add [BUCKET] [NAME] [PREFIX] --expire-days <days>`.

7. Deploy:

   ```sh
   pnpm deploy
   ```

## Deploying schema_version 4 (ADR-222)

ADR-222 raises `schema_version` to 4 (gzip + base64 logs, `log_excerpt_gz` / `app_log_excerpt_gz`) and the body limit to 2MiB. The Worker accepts both 3 and 4, so deploying it first is safe for existing clients. **Deploy the Worker before shipping a client that sends version 4**: an old Worker silently drops unknown fields, but it rejects `schema_version: 4` with `400 unsupported_schema_version`, so the client saves the report locally instead of losing the logs. The reverse order only fails loudly; it never loses data silently.

### 0. Check the plan (Workers Free has a 10 ms CPU limit per request)

The plan could not be confirmed from the CLI/API (the maintainer OAuth token has no subscription scope). ADR-095 chose Cloudflare for its free tier (fail-closed on overage) and recorded that R2 was enabled without a credit card, so **assume Workers Free** until the dashboard says otherwise:

- Dashboard → Workers & Pages → Plans (shows "Free" or "Paid").
- Free: 10 ms CPU per request (I/O waits do not count). Paid: 30 s default.

The CPU-bound part of an intake request is: decoding the body, `JSON.parse`, validation, and `JSON.stringify(stored)` for R2. Measured in CI (GitHub runner, Node/V8, same-run comparison in `test/index.test.ts`, which prints a `[cpu]` line) for a 1.75MiB body:

| | first run | warm |
| --- | --- | --- |
| before the optimization (full-field regex + pretty-printed JSON) | 14.2 ms | 13.3 ms |
| now (head/tail check + compact JSON) | 11.9 ms | 11.4 ms |

> **Production measurement (2026-10-04, Worker version `8cbcf745`, `wrangler tail`)** replaces the CI estimate below: `cpuTime` was 1 ms for a 1KiB report, **8 ms for a 399KiB report**, and **31 ms for a 1.8MiB report** (all `outcome: ok`, HTTP 201). That is about 16 ms per MiB on workerd (plus ~1.5 ms fixed), roughly 2.5x the Node/CI figure, so the realistic range below is too optimistic: a 0.5MB report costs about 9 ms and a 1MB report about 17 ms. That a 31 ms request still returned `ok` is consistent with either Workers Paid, or the Free plan's allowance for an isolate that only occasionally runs over (the docs say it is terminated once it hits the limit consistently); a single run cannot tell which. Check the plan in the dashboard.

So a body near the 2MiB limit is **around or above the Free limit even after the optimization**: decoding, parsing and re-serializing 1.75MiB of JSON costs about 11 ms on a CI runner by itself. The cost is roughly linear in the body size. A realistic report (ten minutes of typing, journal + awase.log gzipped) is about 0.1-0.5MB (CI-scale estimate: 1-3 ms; production measurement above: about 2-9 ms). These are rough guides (CI runners vary run to run, and workerd is not Node); the real number is step 4.

If the plan is Free, expect only unusually large reports to be at risk, and a failed intake is not silent: the client gets a 5xx and saves the report under `%TEMP%`.

### 1. Check the checks passed

The PR's CI jobs (`report-worker`: typecheck + vitest, plus the Rust jobs) must be green. Do not deploy from a red branch.

### 2. Note the version to roll back to

```sh
cd services/report-worker
pnpm install --frozen-lockfile
pnpm exec wrangler deployments list
```

Write down the current deployment/version id.

### 3. Deploy the Worker

```sh
pnpm deploy
```

### 4. Smoke test and measure CPU

In a second terminal, watch the Worker (look at `cpuTime` and `outcome`; `exceededCpu` means the Free limit was hit):

```sh
pnpm exec wrangler tail awase-report-worker --format json
```

Then, from the repository root (7 requests; the per-IP limit is 20/day and cases 1-4 count against it, so do not run it repeatedly):

```sh
python3 scripts/report_worker_smoke.py
```

Expected: cases 1-4 return 201 (v3 legacy, v4 small, v4 realistic ~400KiB, v4 stress ~1.8MiB), case 5 returns 400 `legacy_log_fields_not_allowed_in_schema_4`, case 6 returns 400 `log_excerpt_gz_invalid`, case 7 returns 413. Read `cpuTime` for cases 3 (realistic) and 4 (stress) separately: the realistic one should be far under 10 ms.

### 5. Decide from the CPU result

| `wrangler tail` for case 4 (stress) | Meaning | Action |
| --- | --- | --- |
| `outcome: ok`, `cpuTime` < 10 ms | Within the Free limit | Done |
| `cpuTime` 10 ms or more but `outcome: ok` | Over the Free limit on this run (the limit is enforced per request, so it can fail intermittently) | Treat as a failure: option A or B below |
| `outcome: exceededCpu` / HTTP 5xx on case 4 | Large reports fail; the client saves them locally | Option A or B below (only needed if case 3 also fails, or you want large reports to work) |

- **A. Upgrade to Workers Paid** (about $5/month): no code change.
- **B. Cap the body lower for the Free plan**: set `MAX_BODY_BYTES` (here and in `crates/awase-windows/src/bug_report.rs`) to a size whose measured `cpuTime` is under 10 ms (for example 1MiB, about 6 ms on the CI scale above), and ship the client change. Ten minutes of typing compresses to roughly 0.1-0.5MB, so 1MiB still holds it.
- **C. Cheaper validation**: validate only the gzip header, length and a bounded prefix/suffix instead of scanning the whole field (weaker; only if A and B are rejected).

### 6. Delete the smoke-test reports

The script prints the `report_id` of each stored report. The object key is `reports/<year>/<month>/<report_id>.json` (the year/month come from the ULID timestamp, i.e. today's UTC date):

```sh
pnpm exec wrangler r2 object delete awase-report-bucket/reports/YYYY/MM/REPORT_ID.json --remote
```

They also expire by the 90-day lifecycle rule if you skip this.

### 7. Roll back if needed

```sh
pnpm exec wrangler rollback <version-id>
```

After a rollback to the pre-ADR-222 Worker, clients that send version 4 get `400 unsupported_schema_version` and save the report under `%TEMP%\awase_bug_report_failed_*.json`; old (version 3) clients keep working.

### 8. Ship the client

Merge the PR and release only after steps 3-5 are done. Old reports (version 3, plain-text logs) stay readable: `scripts/fetch_latest_bug_report.py` and the `bug-report-fetch` skill handle both formats.

## Notes

- The Worker intentionally does not implement Turnstile or browser-based bot checks.
- The report object contains the client payload and server-generated metadata only: `report_id` and `received_at`.
- R2 bucket read/list/delete access should be granted only to separate maintainer credentials, not to this Worker binding.
