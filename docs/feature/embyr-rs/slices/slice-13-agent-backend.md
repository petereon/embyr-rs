# Slice 13 — embyr Agent (mTLS gRPC Backend)

**Goal**: Customer deploys a Rust agent binary in their VPC; embyr SaaS routes all storage operations through it over mTLS; no DB credential leaves the VPC.

## IN scope
- `embyr-agent` binary: serves `embyr.agent.v1.StorageAgent` gRPC service
- mTLS: agent verifies embyr SaaS client cert; embyr SaaS verifies agent cert against `backend_agent_ca`
- Agent env config: `EMBYR_AGENT_DB_DSN`, `EMBYR_AGENT_LISTEN_ADDR` (`:9191`), `EMBYR_AGENT_CERT`, `EMBYR_AGENT_KEY`, `EMBYR_AGENT_CA`, `EMBYR_AGENT_MAX_CONNS`, `EMBYR_AGENT_LOG_LEVEL`
- Admin API: `backend_mode=agent` with `backend_agent_endpoint` and `backend_agent_ca`
- embyr SaaS `AgentAdapter`: translates StorageAdapter calls → `StorageAgent` gRPC calls
- Health check: embyr SaaS pings agent on project registration; failure → 400
- Cert rotation: PATCH `backend_agent_ca` without downtime

## OUT scope
- Agent binary distribution / packaging (separate concern)
- Agent metrics endpoint

## Learning Hypothesis
Disproves: "Agent mode mTLS certificate management is too operationally complex for a typical DevOps team."
Confirms if: a DevOps engineer can deploy the agent, register the project, and get a green test suite in under 30 minutes using only the documented env vars and a standard K8s Secret for the TLS cert.

## Acceptance Criteria
- Agent starts with all required env vars; logs "listening on :9191" and "connected to Postgres"
- Project registered with `backend_mode=agent`: 201; embyr SaaS system DB contains no DSN
- SDK write routes through agent: agent logs show incoming gRPC call and Postgres query
- mTLS enforced: connection without valid cert → TLS handshake failure, no data transmitted
- Agent exits non-zero if `EMBYR_AGENT_DB_DSN` is missing
- Cert rotation (PATCH `backend_agent_ca`): SDK requests succeed during rolling restart

## Dependencies
S10 (admin API)

## Effort estimate
≤1 day

## Pre-slice SPIKE
Confirm that `tonic` supports mutual TLS with runtime-configurable cert reloading (needed for cert rotation without restart of embyr SaaS).
