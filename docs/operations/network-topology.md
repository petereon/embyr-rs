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

ADR-001 states this plainly: "the load balancer must route requests with the same `SID` to the
same embyr instance." This is a real, non-optional requirement once you run more than one
`embyr-server` instance behind a shared LB — `single-host-deployment.md` doesn't need it (only
one instance exists), which is why it wasn't covered there.

**The affinity key is the BrowserChannel `SID`, carried as a URL query parameter, not a cookie.**
This matters because the common cloud-LB stickiness feature (AWS ALB "application-based cookie
stickiness," GCP HTTP(S) LB session affinity by cookie) expects the *application* to set a
cookie the LB then pins on — `embyr-server` does not set one for this purpose. Two honest options,
in order of how well they match the actual requirement:

1. **Run a Layer-7 proxy capable of hashing on the query parameter directly** (nginx or HAProxy),
   with your cloud LB in front of it purely as a TCP/TLS-terminating passthrough. Extend
   `single-host-deployment.md`'s existing `embyr_rest` upstream block with nginx's consistent-hash
   load-balancing method, keyed on the `SID` query argument:

   ```nginx
   upstream embyr_rest {
       hash $arg_SID consistent;
       server 10.0.1.10:8081;
       server 10.0.1.11:8081;
       keepalive 32;
   }
   ```

   `$arg_SID` is nginx's built-in reference to the `SID` query-string parameter; `hash ... consistent`
   (`ngx_http_upstream_module`) means requests carrying the same `SID` are routed to the same
   upstream server as long as that server stays healthy, without requiring a cookie. This is the
   most direct translation of ADR-001's own stated requirement.

2. **A cloud Network Load Balancer (L4) with client-IP-based routing** is a fallback, not a
   recommendation — it approximates affinity by source IP, which breaks for any client population
   behind NAT/a shared corporate egress IP (multiple distinct `SID`s from the same IP would
   incorrectly need to land on the same instance, or worse, one client's IP changing mid-session
   breaks affinity entirely). Only use this if you cannot run an L7 proxy for some reason, and
   understand the failure mode before you do.

There is no third option where a stock cloud L7 LB's built-in stickiness feature satisfies this
requirement out of the box — its stickiness key (a cookie) and embyr's actual affinity key (the
`SID` query parameter) don't match. This is a genuine architectural gap for any operator scaling
past one instance with browser clients, not something this document can paper over; ADR-001's own
"Trade-offs and costs" section already names it as a cost of the single-process design, not a
solved problem.

## Cross-references

- [ADR-001 (Process Topology)](../product/architecture/adr-001-process-topology.md) — the
  three-listener requirement, the "admin must be unreachable from the public network" line, and
  the BrowserChannel sticky-routing trade-off this document translates into concrete steps.
- [`single-host-deployment.md`](./single-host-deployment.md) — the one-instance case (no LB, no
  sticky-routing decision needed), whose nginx config this document's §3 extends.
- [`resource-sizing.md`](./resource-sizing.md) — CPU/memory guidance for the instances this
  document's LB/DNS/firewall setup sits in front of (finding #52).
