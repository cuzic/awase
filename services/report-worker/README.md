# awase report worker

Cloudflare Workers + R2 based private intake endpoint for awase tray bug reports.

## Implemented endpoint

- `POST https://report.awase.cc/v1/reports`
- Accepts the schema documented in `payload-schema.md` with `schema_version: 1`.
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

## Notes

- The Worker intentionally does not implement Turnstile or browser-based bot checks.
- The report object contains the client payload and server-generated metadata only: `report_id` and `received_at`.
- R2 bucket read/list/delete access should be granted only to separate maintainer credentials, not to this Worker binding.
