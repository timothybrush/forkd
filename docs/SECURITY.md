# Security policy

forkd is alpha software. The threat model and current guarantees are
documented below so operators can decide what workload they are
willing to point at it.

## Threat model

forkd assumes:

1. **Host kernel and Firecracker are part of the TCB.** A compromised
   host can do anything to its sandboxes. forkd does not attempt to
   protect against a hostile administrator.

2. **Sandboxes are mutually untrusted.** Each child runs in its own
   KVM-backed microVM with a separate netns and cgroup. Escaping
   requires a KVM or Firecracker vulnerability (the same boundary
   AWS Lambda relies on).

3. **The daemon's REST surface is partially trusted.** When
   `--token-file` is set, possessing the token grants full control
   over snapshots and sandboxes on that host. Treat the token like a
   root credential.

## Default posture

| Concern | Default | How to harden |
|---|---|---|
| Daemon bind | `127.0.0.1:8889` (loopback only) | Override at your own risk; require TLS-terminating reverse proxy |
| Authentication | none | `--token-file /etc/forkd/token` |
| Per-child memory cap | none | `memory_limit_mib` per sandbox |
| Per-child netns | shared (same host bridge) | `per_child_netns: true` + `scripts/netns-setup.sh N` |
| Firecracker seccomp | enabled by Firecracker default | n/a — already on |
| Guest agent reachability | inside netns | each child's agent is reachable only from its own netns |
| Audit log | `/var/log/forkd/audit.log`, JSON lines | tail with vector / fluentbit; rotate with logrotate |

## What forkd does not do (yet)

- **Multi-node scheduling.** One daemon = one host. No HA, no failover.
- **TLS termination.** Put a reverse proxy (nginx, traefik) in front
  for non-loopback access.
- **Default-deny egress.** Children share the host's MASQUERADE rule;
  outbound to the internet works by default. For an allow-list policy,
  add per-netns iptables rules after `scripts/netns-setup.sh`.
- **Quotas beyond memory.** cpu.max, io.max, pids.max are not yet
  wired into ForkOpts.
- **Third-party security audit.** Not started. Will be required
  before forkd claims a "production" status badge.

## Reporting a vulnerability

Email `security@deeplethe.com`. Please do not open a public issue for
security reports. We aim to acknowledge within 72 hours and ship a fix
or mitigation within 14 days for confirmed issues.

## Supported versions

Pre-1.0 releases receive fixes only on the latest minor. The CHANGELOG
records which API versions are affected by each advisory.
