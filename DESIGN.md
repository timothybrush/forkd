# Design

This document describes how forkd implements fork-on-write for
Firecracker microVMs and the constraints that drove the design.

## Overview

```
                 Parent VM
                 ─────────
   +-------------+   userspace warmed: Python, deps, models loaded
   |  memory.bin |◄── mmap'd into every child Firecracker process
   +-------------+
   |  vmstate    |   vCPU regs, devices, MMIO state, virtio queues
   +-------------+

         │ snapshot (Firecracker /snapshot/create)
         ▼

   Child 1, Child 2, ... Child N
   ──────────────────────────────
   Each child is its own Firecracker process:
     - mmap(memory.bin, MAP_PRIVATE) — kernel CoWs diverged pages
     - PUT /snapshot/load — restores vCPU + device state
     - PATCH /vm {state:Resumed} — runs from the snapshot point
     - Cgroup leaf at /sys/fs/cgroup/forkd/child-N — memory.max
     - Network namespace forkd-child-N — isolated tap + veth
```

The kernel does the hard part (CoW page management). forkd's job is
correctness, isolation, and orchestration.

## Runtime: Firecracker

forkd builds on Firecracker rather than a container runtime or gVisor
because:

- **Snapshot/restore is first-class.** Firecracker's `/snapshot/load`
  with `MEMORY_LOAD_PRIVATE` is the exact primitive we need.
- **KVM-backed.** Each child gets a hardware isolation boundary, not
  a syscall filter or namespace.
- **Small.** ~5 MiB of resident memory per VM process before any
  guest state.
- **Stable API.** Rust ecosystem, well-trodden by AWS Lambda.

forkd uses upstream Firecracker — no vendored fork.

## Component layout

```
crates/
  forkd-vmm          Firecracker wrapper. BootConfig, Vm, Snapshot,
                     ForkOpts, cgroup helpers, network namespace
                     plumbing, raw HTTP/1.1 over Unix socket.
  forkd-cli          `forkd` binary. CLI surface: snapshot, fork,
                     run, exec, eval, ping.
  forkd-controller   `forkd-controller serve`. REST API, persistent
                     registry, audit log, /metrics, bearer-token
                     auth, graceful shutdown.
rootfs-init/
  forkd-init.sh      PID 1 inside the guest. Mounts pseudo-fs, fixes
                     DNS, launches the agent.
  forkd-agent.py     TCP server on :8888 (ping / exec / eval).
sdk/python/          E2B-compatible Python SDK.
```

## Hard problems and how forkd addresses them

### 1. Memory image backing

Putting `memory.bin` on tmpfs invites OOM kill. Slow disk kills
restore latency. forkd writes the image to ext4 by default and relies
on the page cache; on hosts with hugepages provisioned (per
`scripts/setup-host.sh`) the kernel transparently backs hot pages
with 2 MiB pages.

Future: explicit `memfd_create(MFD_HUGETLB)`-backed memory for
high-N fork-out, where the savings on page-table size dominate.

### 2. RNG and TSC

Children boot with the parent's RNG state and the parent's `tsc_offset`.
Both are cryptographically broken if exposed externally.

- **RNG**: Linux 5.20+ exposes `vmgenid`, a virtio-device-driven
  "generation counter" that the guest kernel watches; Firecracker
  bumps the counter on restore and the guest's CRNG re-seeds from
  /dev/hw_random automatically. forkd relies on this — no userspace
  daemon required.
- **TSC**: Firecracker assigns a fresh `tsc_offset` on each restore
  via its `--rdtsc` handling. This is enabled by default.

### 3. MAC / IP collisions

All children inherit the parent snapshot's MAC and guest IP. Without
network isolation they would collide on the host bridge.

forkd places each child in its own pre-provisioned network namespace
(`forkd-child-1` … `forkd-child-N`). Each namespace has:

- An independent tap (same name, same IP — different network stack).
- A `veth` pair into a shared `forkd-br0` bridge for outbound NAT.
- SNAT on egress so the bridge can reverse-route replies.

See `scripts/netns-setup.sh` for the exact iptables rules.

### 4. Block device CoW

Children need a writable rootfs but should share the base image.

Today forkd uses Firecracker's built-in read-write attachment with
overlayfs on the host. Each child gets a fresh upper-dir, lower dir
is the shared rootfs. Writes are per-child; nothing persists
post-exit.

Future: `dm-thin` for production density beyond a few hundred
concurrent children.

### 5. KSM aggressiveness

Default KSM (`kernel same-page merging`) is too lazy — minutes to
reach steady-state sharing. `scripts/setup-host.sh` tunes
`pages_to_scan` and `sleep_millisecs` for forkd's workload. With CoW
mmap, KSM is a backstop for divergent-but-similar pages; it doesn't
need to do the heavy lifting.

### 6. OOM cascades

If the host hits memory pressure and the OOM killer takes the parent
process, every child loses its backing pages.

forkd nudges each child's `oom_score_adj` up by +500 so the kernel
picks a runaway child first. With per-child `memory.max` set via
cgroup v2, runaway children are bounded before they push the host
into global pressure.

### 7. Per-child resource quotas

forkd creates one cgroup v2 leaf per child under
`/sys/fs/cgroup/forkd/child-N/` and writes the Firecracker PID to
`cgroup.procs`. Today only `memory.max` is wired into `ForkOpts`;
cpu.max / io.max / pids.max land before 1.0.

### 8. Scheduling affinity

Children must land on the same host as their parent (otherwise CoW
becomes copy-everything across the wire). v0.1 is single-host only;
multi-host scheduling is a v1.x problem and will require either a
warm parent on each scheduling target or a fast snapshot-replication
path.

## Authentication and audit

The controller daemon optionally requires a bearer token
(`--token-file`) on every request except `/healthz`. The check uses
length-aware constant-time comparison to avoid trivial timing oracles.

Every request is appended to a JSON-Lines audit log
(`/var/log/forkd/audit.log` by default): RFC3339 timestamp, method,
path, status, latency in microseconds, user-agent. Log rotation is
out of process (`logrotate`, `vector`, the journal).

## What forkd does not do

- Replace Modal or E2B as a SaaS.
- Beat function-level snapshot runtimes (single-vCPU, serial I/O) on
  raw spawn time; forkd targets full Linux microVMs with networking
  and multi-vCPU.
- Support arbitrary guest OSes. v0.1 targets Linux x86-64.

## Stability

API versioning: the REST surface is at `/v1`. Breaking changes move
to `/v2` and the previous major is supported for one minor release
after the new one ships. On-disk snapshot format is currently tied
to Firecracker's `Full` snapshot version; we do not promise forward
compatibility across forkd versions until 1.0.
