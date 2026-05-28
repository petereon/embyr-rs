# Slice 14 — Cloud Secret Backends (AWS + GCP)

**Goal**: embyr fetches DB DSN from AWS Secrets Manager or GCP Secret Manager at request time; password rotation is transparent.

## IN scope
- `backend_mode=aws_secret`: fetch via AWS SDK (`GetSecretValue`); requires `backend_secret_arn`; uses IRSA/instance role — no static AWS credentials in embyr config
- `backend_mode=gcp_secret`: fetch via GCP API (`AccessSecretVersion`); requires `backend_secret_gcp`; uses workload identity — no static GCP service account key required in production
- Secret format: `{"dsn": "postgres://..."}` (JSON object with `dsn` key)
- Credential cache: in-process, TTL=5min, keyed by `(project_id, BLAKE3(api_key))`; cache holds the fetched DSN, not the raw secret
- Error on registration if secret is unreadable: `400 backend_secret_fetch_failed`
- Error on malformed secret format: `400 backend_secret_format_invalid`
- Password rotation: transparent within TTL window (≤5 min gap)

## OUT scope
- Static AWS credentials / GCP service account key file in config (deliberately excluded for security)

## Learning Hypothesis
Disproves: "Cloud IAM credential fetch adds >500ms latency per first request after cache miss."
Confirms if: p99 latency for the first SDK request after a cache miss (cold credential fetch) is < 500ms.

## Acceptance Criteria
- `POST` with `backend_mode=aws_secret` and valid ARN: 201; no DSN in embyr system DB
- `POST` with no IAM access: 400 `backend_secret_fetch_failed`
- `POST` with malformed secret JSON: 400 `backend_secret_format_invalid`
- DB password rotation + 6 min wait: SDK requests succeed with new password
- Cache miss cold fetch p99 < 500ms (measured in integration test with real AWS/GCP)
- `backend_mode=gcp_secret` path symmetric to AWS path

## Dependencies
S10 (admin API)

## Effort estimate
≤1 day

## Pre-slice SPIKE
Verify that `aws-sdk-rust` IRSA token refresh is non-blocking and compatible with Tokio async runtime; same for GCP workload identity credential provider.
