# crates/embyr-agent/Dockerfile
#
# Wraps the pre-built, statically-linked x86_64-unknown-linux-musl
# embyr-agent binary (produced by the `agent` CI job's musl build step) for
# operators who deploy via Kubernetes (docs/product/journeys/agent-deployment.yaml
# step 2: `kubectl apply`) rather than a bare-binary VPC install.
#
# No compilation happens here — the release binary is already built by CI
# before this step runs. ADR-001 requires embyr-agent to remain a
# structurally separate artifact from embyr-server: this image contains
# only the agent binary, nothing from crates/embyr-server.
#
# FROM scratch (not debian:bookworm-slim, unlike the root Dockerfile) is
# possible because the binary is fully statically linked and the agent's
# dependency tree is rustls/ring-only (zero openssl-sys, confirmed in
# ADR-074) — no glibc, no shared libs, no package manager needed at runtime.
# embyr-agent makes no outbound public-CA-verified HTTPS calls (its mTLS
# material is customer-supplied via EMBYR_AGENT_CERT/KEY/CA), so no CA
# bundle is copied in either.

FROM scratch AS runtime

COPY target/x86_64-unknown-linux-musl/release/embyr-agent /embyr-agent

# Numeric non-root UID — scratch has no /etc/passwd, so a named user isn't
# possible; 65532 is the conventional distroless "nonroot" UID.
USER 65532:65532

EXPOSE 9191

ENTRYPOINT ["/embyr-agent"]
