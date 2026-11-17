# Network Topology — DNS, Load Balancer, Firewall

Closes Medium finding #53 from
[`docs/product/production-readiness-audit-2026-09-08.md`](../product/production-readiness-audit-2026-09-08.md).

## Who this is for

An operator running `embyr-server` behind a real DNS name and load balancer (multi-host, or a
single host that still wants a stable public hostname and a firewall boundary in front of it).
[ADR-001 (Process Topology)](../product/architecture/adr-001-process-topology.md) states the
*requirements* — two externally-visible ports, one admin port that "must be unreachable from the
public network," and sticky routing for BrowserChannel — but stops at architectural intent. This
document translates that into concrete DNS records, a firewall rule example, and LB
session-affinity configuration.

If you're running a single host, [`single-host-deployment.md`](./single-host-deployment.md)
already gives you the loopback-bind + nginx pattern that satisfies the admin-port requirement
without any of the LB machinery below — read that first and only come here once you're adding a
load balancer or a second host.

## 1. DNS record structure

ADR-001's three listeners map to two public surfaces and one that must never be public:

| Listener | Port | Suggested DNS name | Public? |
|---|---|---|---|
| gRPC | 8080 | `grpc.yourdomain.com` | Yes |
| REST / gRPC-Web / BrowserChannel | 8081 | `api.yourdomain.com` (or `rest.yourdomain.com`) | Yes |
| Admin | 9090 | `admin.internal.yourdomain.com` | **No** — VPN/bastion/private-network only |

This mirrors the two-hostname split `single-host-deployment.md`'s nginx example already uses
(`grpc.embyr.example.com` / `rest.embyr.example.com`) — extend that pattern to a load-balanced
DNS name instead of a single host's IP:

- `grpc.yourdomain.com` → `A`/`ALIAS` record → your load balancer's public IP/hostname → LB
  listener on 443 → backend target group on `:8080`
- `api.yourdomain.com` → same LB, different backend target group on `:8081`
- `admin.internal.yourdomain.com` → **not** a public DNS record at all. Put it in a private
  hosted zone resolvable only inside your VPN/VPC (e.g. Route 53 private hosted zone, or an
  internal-only zone on your DNS provider), or skip DNS entirely and reach `:9090` by private IP
  over a bastion/SSH tunnel/VPN, exactly as `single-host-deployment.md` §4 already recommends
  (`curl http://127.0.0.1:9090/healthz` from the host itself, or via SSH port-forward).

## 2. Firewall / security-group rule: keeping :9090 unreachable from the public internet

The concrete requirement from ADR-001 is a firewall rule, not just a routing decision — even if
no LB target group points at `:9090`, the port must not be open to `0.0.0.0/0` at the network
layer. Example as an AWS security-group rule set (the same shape applies to a GCP firewall rule
or an on-prem iptables/nftables rule):

| Direction | Port | Source | Purpose |
|---|---|---|---|
| Inbound | 8080 | `0.0.0.0/0` | Public gRPC |
| Inbound | 8081 | `0.0.0.0/0` | Public REST/gRPC-Web/BrowserChannel |
| Inbound | 9090 | Your VPN/bastion security group or CIDR only (e.g. `10.0.99.0/24`) | Admin API — **never** `0.0.0.0/0` |
| Inbound | 9090 | *(no rule for `0.0.0.0/0`)* | Explicitly absent — the default-deny is the point |

```bash
# Example: AWS CLI, restricting :9090 to a VPN CIDR only
aws ec2 authorize-security-group-ingress \
  --group-id sg-embyr-server \
  --protocol tcp --port 9090 \
  --cidr 10.0.99.0/24   # your VPN/bastion subnet, not 0.0.0.0/0
```

Two independent layers already close this gap in the single-host case
(`single-host-deployment.md` §2's `-p 127.0.0.1:9090:9090` Docker bind) — the security-group rule
above is the equivalent control when `embyr-server` runs behind a real LB/multiple hosts, where
loopback binding alone no longer applies because the bind address is the container network, not
the public interface. Apply both where relevant: bind `:9090` to a private interface/loopback
*and* firewall it, rather than relying on either alone.

## 3. Sticky routing / session affinity for BrowserChannel

BrowserChannel (the web SDK's fallback realtime transport, `/google.firestore.v1.Firestore/Listen/channel`
and `.../Write/channel` on `:8081`) keeps its sessions **in the memory of the replica that handled
the handshake**. Native gRPC (`:8080`) and every other route are stateless and need no affinity.

**Routing on the `SID` query parameter cannot work.** An earlier version of this section (and
ADR-001) recommended nginx `hash $arg_SID consistent`. The handshake request carries no `SID`; the
replica that serves it mints a random `SID` and returns it in the body. A hash of that `SID` picks a
replica unrelated to the one holding the session, so with N replicas roughly (N-1)/N of follow-up
requests land on a replica that has no such session.

What a miss looks like: the replica answers `400` with body `Unknown SID` (also for an expired or
swept session). That is the reply the closure/firebase-js-sdk channel treats as "session gone", so the
SDK recreates the stream itself instead of hanging. Misrouting is therefore *degraded*, not *broken*:
the client re-handshakes and re-listens, but every miss costs a re-sync of the query.

`embyr-server` deliberately does **not** set its own affinity cookie: cross-origin XHR carries cookies
only with `withCredentials`, which would also require `Access-Control-Allow-Credentials: true` and an
exact-origin response on the whole `:8081` surface (today `allow_credentials(false)`), and
`SameSite=None` cookies are blocked in third-party contexts by several browsers. Not worth the CORS
change for a fallback transport.

Options, cheapest first:

1. **Path-route the two BrowserChannel paths to one dedicated replica** (or a one-replica target
   group). No code change; a known ceiling and single point of failure for web realtime. Correct
   default.
2. **LB-inserted cookie stickiness on those two paths** (HAProxy `cookie ... insert`, Envoy stateful
   session, nginx `sticky`; the LB sets the cookie on the handshake response, the app does nothing).
   Works only where the browser sends cookies on those requests, i.e. the web app and the API are
   served from the same origin behind the same LB hostname. Cross-origin setups will silently get no
   stickiness. Test with the real web SDK before relying on it.
3. **Accept degraded reconnect** with N > 1 and no affinity: the SDK recovers via `Unknown SID`, but
   most follow-ups miss, so streams thrash. Only tolerable for a handful of web clients.
4. **Not built:** make the `SID` routable (prefix it with a per-replica tag and hash on the prefix).
   This is the proper fix if web realtime at scale matters.

An L4 client-IP hash is not recommended: it breaks behind NAT/shared egress and when the client IP
changes mid-session.

## Cross-references

- [ADR-001 (Process Topology)](../product/architecture/adr-001-process-topology.md) — the
  three-listener requirement, the "admin must be unreachable from the public network" line, and
  the BrowserChannel sticky-routing trade-off this document translates into concrete steps.
- [`single-host-deployment.md`](./single-host-deployment.md) — the one-instance case (no LB, no
  sticky-routing decision needed), whose nginx config this document's §3 extends.
- [`resource-sizing.md`](./resource-sizing.md) — CPU/memory guidance for the instances this
  document's LB/DNS/firewall setup sits in front of (finding #52).
