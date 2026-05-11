# forkd

A microVM sandbox runtime that forks children from a warmed parent
snapshot, so each child inherits the parent's address space
copy-on-write instead of cold-booting its own kernel.

[![CI](https://img.shields.io/github/actions/workflow/status/deeplethe/forkd/ci.yml?branch=main&label=ci)](https://github.com/deeplethe/forkd/actions)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](./LICENSE)
[![status](https://img.shields.io/badge/status-alpha-orange)](https://github.com/deeplethe/forkd/releases)

forkd is built on Firecracker. The parent VM boots once, imports
your runtime (Python + your dependencies, a JIT-warmed JVM, an
already-loaded ML model) and is paused to disk. Each child is a
separate Firecracker process that `mmap`s the parent's memory image
with `MAP_PRIVATE`; the kernel implements copy-on-write at the page
level, so children share the parent's resident memory until they
diverge.

The result is two properties at once: per-child KVM isolation, and a
spawn cost that's closer to `fork(2)` than to a cold-boot VM.

---

## Properties

- **Hardware isolation.** Each child is its own Firecracker microVM
  backed by KVM. Escape requires a hypervisor or kernel vulnerability,
  not a `runc` regression.
- **Warmed runtimes inherit for free.** Imports, JIT compilation, model
  weights, prefetched caches — anything the parent did is already
  resident in the child.
- **Multi-tenant by construction.** Per-child network namespace, per-
  child cgroup v2 memory limit, independent `/dev/urandom` re-seeded
  by `vmgenid` (Linux 5.20+).
- **Operable.** Daemon process owning state, REST API on Unix or TCP,
  Prometheus `/metrics`, append-only JSON audit log, systemd unit.
- **Open source.** Apache 2.0, no vendor SDK.

---

## Benchmarks

Same Linux host (Ubuntu 24.04, Linux 6.14, 20 vCPU, 30 GiB, KVM).
Workload: spawn 100 sandboxes that each run `import numpy;
numpy.zeros(5).tolist()`.

![Spawn time at N=100](./bench/chart-spawn-100.png)

![Host memory per sandbox](./bench/chart-memory-per.png)

| Backend | Wall-clock at N=100 | Memory delta per sandbox |
|---|---:|---:|
| forkd | 101 ms | 0.12 MiB |
| CubeSandbox¹ | 390 ms | 5 MiB |
| Firecracker cold-boot | 759 ms | 84 MiB |
| gVisor (runsc) | 288.6 s | — |
| Docker (runc) | 335.3 s | 4 MiB |

¹ CubeSandbox didn't install on the bench host due to port conflicts
with an existing 1Panel stack; the number above combines Tencent's
published P95 spawn (~90 ms at 50 concurrent) with the cold
`import numpy` cost (~300 ms). See [`bench/CUBESANDBOX.md`](./bench/CUBESANDBOX.md).

Reproduce: `bench/bench-spawn-100.sh` then `bench/generate_charts.py`.

For one sandbox doing the same numpy expression two ways:

| Call | Time | What it does |
|---|---:|---|
| `sandbox.eval("numpy.zeros(5).tolist()")` | 1 ms | Reuses the warmed Python in PID 1 |
| `sandbox.commands.run("python3 -c '...'")` | 96 ms | Cold subprocess re-imports numpy |

---

## How it works

```
                 Parent VM
                 ─────────
   Boots once, /forkd-init.sh imports your runtime, pauses.
   Snapshot writes vmstate + memory.bin to disk.
                       │
                       │  POST /v1/sandboxes  (n=100)
                       ▼
   ┌──────────────────────────────────────────────────┐
   │ netns forkd-child-1     ...    netns forkd-child-100
   │ ┌──────────┐                   ┌──────────┐
   │ │ FC proc  │                   │ FC proc  │
   │ │ mmap     │  ━━━━━━━━━━━━━━━━ │ mmap     │  ← same memory.bin
   │ │ MAP_     │                   │ MAP_     │     file; kernel CoWs
   │ │ PRIVATE  │                   │ PRIVATE  │     diverged pages
   │ └──────────┘                   └──────────┘
   │ cgroup forkd/child-1           cgroup forkd/child-100
   │ memory.max = 256 MiB           memory.max = 256 MiB
   └──────────────────────────────────────────────────┘
        │ veth                           │ veth
        └────────── host bridge forkd-br0 (NAT) ───┘
                                  │
                                  ▼  MASQUERADE
                            uplink → internet
```

See [`DESIGN.md`](./DESIGN.md) for the full design and the open
problems the architecture leaves on the table.

---

## Quick start

Requires: x86_64 Linux with KVM, Ubuntu 22.04 or newer.

```bash
# 1. Host setup: KVM, Firecracker, Rust, KSM, hugepages, tap device.
sudo bash scripts/setup-host.sh
sudo bash scripts/host-tap.sh
cargo build --release
sudo install -m 0755 target/release/{forkd,forkd-controller} /usr/local/bin/

# 2. Build a warmed rootfs from a Docker image.
sudo bash scripts/build-rootfs.sh python:3.12-slim python-rootfs.ext4 1536 python3-numpy

# 3. Fetch a kernel.
curl -O https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.10/x86_64/vmlinux-6.1.141

# 4. Run a one-shot sandbox.
sudo -E forkd run --image python:3.12-slim --kernel ./vmlinux-6.1.141 \
    -- python3 -c "import numpy; print(numpy.zeros(5).sum())"
# 0.0
```

### Multi-child fork-out

```bash
# Provision N per-child network namespaces (one-time per N).
sudo bash scripts/netns-setup.sh 100

# Create a tagged parent snapshot.
sudo forkd snapshot --tag pyagent \
    --kernel ./vmlinux-6.1.141 \
    --rootfs ./python-rootfs.ext4 \
    --tap forkd-tap0

# Fork 100 children sharing the parent's memory.
sudo -E forkd fork --tag pyagent -n 100 --per-child-netns --memory-limit-mib 256

# Talk to one of them.
sudo forkd eval --child forkd-child-42 -- "numpy.zeros(100).sum()"
```

### Python SDK

```python
from forkd import Sandbox   # drop-in for `from e2b import Sandbox`

with Sandbox() as sb:
    print(sb.commands.run("uname -a").stdout)
    print(sb.eval("numpy.zeros(5).tolist()"))    # reuses warmed PID 1
```

---

## Operating in daemon mode

The controller daemon owns the registry of snapshots and live
sandboxes, exposes the REST API, and writes structured audit logs.
Recommended for any deployment beyond local development.

```bash
sudo install -m 0644 packaging/systemd/forkd-controller.service /etc/systemd/system/
sudo mkdir -p /etc/forkd
sudo bash -c 'head -c 32 /dev/urandom | base64 > /etc/forkd/token'
sudo chmod 600 /etc/forkd/token
sudo systemctl enable --now forkd-controller
```

Then drive it over HTTP:

```bash
TOKEN=$(sudo cat /etc/forkd/token)
curl -H "Authorization: Bearer $TOKEN" -X POST http://127.0.0.1:8889/v1/sandboxes \
     -H 'Content-Type: application/json' \
     -d '{"snapshot_tag":"pyagent","n":5,"per_child_netns":true,"memory_limit_mib":256}'
# [{"id":"sb-67a1b3-0000","pid":...,...}, ...]

curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8889/metrics
# forkd_sandboxes_active 5
```

Full API reference: [`docs/API.md`](./docs/API.md).
Operator runbook: [`docs/RUNBOOK.md`](./docs/RUNBOOK.md).
Security posture: [`docs/SECURITY.md`](./docs/SECURITY.md).

---

## Repository layout

```
crates/
  forkd-vmm/            Firecracker wrapper: BootConfig, Vm, Snapshot, cgroup
  forkd-cli/            `forkd` binary (snapshot, fork, run, exec, eval)
  forkd-controller/     `forkd-controller` daemon: REST, registry, audit
rootfs-init/
  forkd-init.sh         PID 1 inside the guest; mounts pseudo-fs, launches agent
  forkd-agent.py        TCP server on :8888 inside the guest (ping/exec/eval)
sdk/python/             E2B-compatible Python SDK
scripts/                Host-side helpers (KVM, Firecracker, netns, rootfs)
packaging/systemd/      systemd unit for the controller
bench/                  Benchmark harness, chart generators, results
docs/                   API.md, SECURITY.md, RUNBOOK.md
```

---

## Status

Alpha. The fork-on-write primitive, controller daemon, REST API,
auth, audit logging, cgroup memory limits, Prometheus metrics, and
Python SDK are in place and exercised by 25 unit + integration tests
in CI. On-disk formats and API shapes may still change before 1.0.

Production-readiness items not yet in this release:

- Multi-node scheduling (one daemon = one host).
- TLS termination — front the daemon with a reverse proxy for
  non-loopback access.
- Default-deny egress on per-child netns (today: shared MASQUERADE
  rule; users wanting an allow-list policy add their own `iptables`
  rules per netns).
- cpu.max, io.max, pids.max quotas beyond the existing
  `memory.max`.
- Third-party security audit.

The roadmap and tracked work live in [GitHub issues](https://github.com/deeplethe/forkd/issues).

---

## Contributing

Pull requests welcome. Before opening one, please:

1. Open an issue describing what you want to change. APIs are still
   moving; we'd rather align early than ask you to rewrite the patch.
2. `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --all` locally.
3. Sign-off your commits (`git commit -s`).

---

## License

Apache 2.0. See [LICENSE](./LICENSE) and [NOTICE](./NOTICE).
