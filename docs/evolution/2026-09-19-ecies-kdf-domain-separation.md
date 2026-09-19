# ecies-kdf-domain-separation (finding #33)

FINALIZED 2026-09-19

## Business Context

`crates/embyr-core/src/auth/ecies.rs` encrypts every backend DSN / TOTP secret
/ OIDC client secret at rest. The AES key was derived via `HKDF-SHA256(salt =
"embyr-ecies-v1-enc", ikm = raw_DH_output, info = "")` — the HKDF `info`
parameter was empty. Standard ECIES practice binds the ephemeral and
recipient public keys into `info` for domain separation against
key-substitution attacks. Not currently exploitable given this system's
single-fixed-recipient design (the recipient keypair is deterministically
re-derived from the project's API key, never a multi-recipient or
attacker-influenced key set), but it's a correctness gap against the standard
construction and cheap to close.

## KDF Change

`derive_aes_key` now takes the ephemeral and recipient public keys and binds
both into HKDF's `info`:

```rust
fn derive_aes_key(shared_secret: &[u8], eph_pub: &[u8; 32], recipient_pub: &[u8; 32]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(b"embyr-ecies-v1-enc"), shared_secret);
    let mut info = [0u8; 64];
    info[..32].copy_from_slice(eph_pub);
    info[32..].copy_from_slice(recipient_pub);
    let mut okm = [0u8; 32];
    hk.expand(&info, &mut okm).expect("32 bytes valid");
    okm
}
```

`encrypt()` passes its own generated `eph_pub` + the caller-supplied
`recipient_pubkey`. `decrypt()` re-derives the recipient's own public key
from the API key (`PublicKey::from(&static_secret)`) and passes it alongside
the ephemeral public key recovered from the ciphertext header. The
`derive_static_secret`/static-secret-from-API-key design (lines 12-17) is
unchanged — this finding was about the KDF `info` binding only, not the
recipient-key derivation itself.

## Compatibility / Re-Encryption Impact — Read Before Deploying

**This changes the derived AES key for every ciphertext ever produced by
this function.** A ciphertext encrypted under the old (empty-`info`) KDF
cannot be decrypted by the new code, and vice versa.

**In this repo's own test/dev context: no compatibility break.** Every
`ecies::encrypt`/`ecies::decrypt` call site found (grep across `crates/` and
`tests/`) is a same-test-run round-trip — encrypt then immediately decrypt
within the same test or the same request, never a hardcoded ciphertext
fixture pinned to the old KDF. No test needed modification beyond adding new
coverage; all pre-existing round-trip tests pass unchanged because they
generate fresh ciphertext under the new KDF and decrypt it under the same new
KDF.

**In a real deployed environment with already-provisioned customer data:
this IS a breaking change.** Any `backend_pg_dsn_enc`, `totp_secret_enc`,
`client_secret_enc`, or `hosted_identity_signing_keys.private_key_enc` row
written by the old code becomes undecryptable after this deploy — decrypt
will fail with an authentication-tag mismatch (fails closed, not a silent
wrong-plaintext). **This repo has no real production database** (audit scope
is code correctness, not a live deployment), so no re-encryption migration
was run or needed as part of this change. Before this code ships to any
environment with real provisioned rows, those rows must be re-encrypted
(decrypt under the old KDF, re-encrypt under the new KDF) as part of the
rollout — there is currently no automated tool for this; see the new
"customer API-key compromise" section added to `docs/operations/runbook.md`
for the same manual decrypt/re-encrypt procedure (the mechanics are
identical whether triggered by a compromise or by this KDF change).

## Key Files

- `crates/embyr-core/src/auth/ecies.rs` — `derive_aes_key`, `encrypt`, `decrypt`
- `docs/operations/runbook.md` — new "Incident: customer API-key compromise" section

## Follow-Up

- No automated re-encryption tool exists for rotating ECIES ciphertext under
  a new KDF/key. Out of scope for this Low-severity fix; noted in the runbook.
- Finding #40 (unrelated Rust refactor, same batch) — see commit body.
