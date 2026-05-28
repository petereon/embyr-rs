# embyr-rs — Claude Code Project Instructions

## Development Paradigm

This project follows the **functional-where-practical** Rust paradigm. Use `@nw-software-crafter` for implementation. Preference: pure transformations, explicit `Result<T, E>` error types, minimal shared mutable state.

## Mutation Testing Strategy

`per-feature` — mutation testing runs after each feature's DELIVER wave.

## Architecture Overview

embyr-rs is a Rust reimplementation of the Google Firestore gRPC protocol server.
It is a **protocol translation layer** only — not a database.

**5-crate Cargo workspace:**
- `embyr-proto` — generated Firestore + agent gRPC stubs (no domain logic)
- `embyr-core` — pure domain types and port traits (NO IO: no tokio, sqlx, tonic, axum)
- `embyr-server` — composition root, wires adapters, opens 3 TCP listeners
- `embyr-admin` — admin HTTP server on :9090
- `embyr-agent` — customer-VPC agent binary (separate deployment)

**Critical constraint:** `embyr-core` must never import IO crates.
Enforced by `deny.toml` + CI.

**3 TCP listeners in embyr-server:**
- :8080 gRPC (tonic)
- :8081 REST/gRPC-Web/BrowserChannel (axum + tonic-web)
- :9090 Admin HTTP (axum, separate TcpListener)

**Key design decisions:**
- Auth: Argon2id (memory=65536KiB, iter=3, par=4) + dual-hash rotation window
- Credential encryption: ECIES (x25519-dalek + hkdf + aes-gcm); private key re-derived from API key
- Cache keys: BLAKE3(api_key)
- Real-time: Postgres LISTEN/NOTIFY, channel name `dc_<BLAKE3_16hex(project_id)>` (19 chars)
- OCC via `version` column (BIGINT) on documents table
- Resume tokens: BLAKE3(timestamp + project_id), 24h retention window
- Rate limiting: per-project per-instance token bucket (no distributed coordination)
- Soft-delete: projects → deleted_at, sweeper after 168h
