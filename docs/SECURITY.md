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
| Daemon bind | `127.0.0.1:8889` (loopback only) | Override at your own risk; pair with `--tls-cert` + `--token-file` |
| TLS | off (loopback HTTP) | `--tls-cert /etc/forkd/tls/cert.pem --tls-key ...` (rustls 0.23, modern cipher suites only) |
| Authentication | none | `--token-file /etc/forkd/token` |
| Per-child memory cap | none | `memory_limit_mib` per sandbox |
| Per-child netns | shared (same host bridge) | `per_child_netns: true` + `scripts/netns-setup.sh N` |
| Firecracker seccomp | enabled by Firecracker default | n/a — already on |
| Guest agent reachability | inside netns | each child's agent is reachable only from its own netns |
| Audit log | `/var/log/forkd/audit.log`, JSON lines | tail with vector / fluentbit; rotate with logrotate |

## TLS

Pass `--tls-cert <cert.pem> --tls-key <key.pem>` to `forkd-controller
serve` (or set `FORKD_TLS_CERT` / `FORKD_TLS_KEY`). The daemon uses
rustls 0.23 with the aws-lc-rs crypto provider; TLS 1.2 and TLS 1.3
are accepted, legacy cipher suites are not negotiable. Both PEM
files must be readable by the daemon's user and SHOULD have mode 0600.

Operationally:

- Use a real CA (Let's Encrypt or your internal PKI). Self-signed
  certs work but require clients to bypass cert validation.
- Rotate by writing new files and `systemctl restart forkd-controller`.
- Bearer-token auth is **not** automatically enabled by TLS — supply
  `--token-file` as well for any non-loopback deployment.

## What forkd does not do (yet)

- **Multi-node scheduling.** One daemon = one host. No HA, no failover.
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
