# forkd HTTP API (v1)

The forkd controller exposes a JSON/HTTP API on `127.0.0.1:8889` by
default. All routes except `/healthz` require a bearer token when the
daemon is started with `--token-file`.

```http
Authorization: Bearer <contents-of-token-file>
```

API versioning: every breaking change moves to a new `/vN` prefix. The
controller will support the previous major in parallel for one minor
release after the new one ships.

---

## Status and discovery

### GET /healthz

Liveness probe. Always returns 200. Bypasses authentication so that
load balancers can probe the daemon without a credential.

```json
{ "ok": true }
```

### GET /version

```json
{ "version": "0.1.0", "api": "v1" }
```

### GET /metrics

Prometheus text exposition format. Stable metric names:

- `forkd_snapshots_total` (gauge) — registered snapshots
- `forkd_sandboxes_active` (gauge) — currently-alive child VMs
- `forkd_build_info{version="X.Y.Z"}` — always 1, label carries the build version

---

## Snapshots

### POST /v1/snapshots

Build a snapshot of a freshly booted parent VM and register it under
`<tag>`. Blocks for `boot_wait_secs` while userspace warms up inside
the guest.

Request:

```json
{
  "tag": "py",
  "kernel": "/var/lib/forkd/kernels/vmlinux-6.1",
  "rootfs": "/var/lib/forkd/rootfs/python.ext4",
  "rw": true,
  "tap": "forkd-tap0",
  "boot_wait_secs": 10
}
```

Response (201 Created):

```json
{
  "tag": "py",
  "dir": "/var/lib/forkd/snapshots/py",
  "created_at_unix": 1717000000
}
```

Errors:

- `400 Bad Request` — invalid `tag`, missing `kernel`/`rootfs`, snapshot already exists
- `500 Internal Server Error` — Firecracker boot/snapshot failure (see logs)

### GET /v1/snapshots

List registered snapshots: `[SnapshotInfo, ...]`.

### DELETE /v1/snapshots/:tag

Remove the registry entry and delete the on-disk snapshot files.
Returns `204 No Content`. `404` if no such tag is registered and no
on-disk files exist.

---

## Sandboxes

### POST /v1/sandboxes

Fork N children from a registered snapshot tag.

Request:

```json
{
  "snapshot_tag": "py",
  "n": 10,
  "per_child_netns": true,
  "memory_limit_mib": 256
}
```

- `n` — 1 ≤ n ≤ 1000
- `per_child_netns` — when true, each child is placed in
  `forkd-child-<i>`; the host must have provisioned those namespaces
  via `scripts/netns-setup.sh N` first.
- `memory_limit_mib` — sets `memory.max` on a per-child cgroup v2
  leaf. Requires cgroup v2 unified hierarchy and write access to
  `/sys/fs/cgroup/forkd/`.

Response (201 Created): `[SandboxInfo, ...]`.

### GET /v1/sandboxes

List active sandboxes.

### GET /v1/sandboxes/:id

One sandbox's metadata.

### DELETE /v1/sandboxes/:id

Terminate. Kills the Firecracker process and removes the cgroup leaf.
Returns `204 No Content`.

### POST /v1/sandboxes/:id/ping

Round-trip to the guest agent inside the VM.

```json
{ "pong": true, "numpy_version": "1.26.4", "pid": 1 }
```

### POST /v1/sandboxes/:id/exec

Spawn a subprocess in the sandbox.

Request:

```json
{ "args": ["python3", "-c", "print(2+2)"], "timeout_secs": 30 }
```

Response:

```json
{ "stdout": "4\n", "stderr": "", "exit_code": 0 }
```

### POST /v1/sandboxes/:id/eval

Evaluate a Python expression against the already-warmed interpreter
running as PID 1.

Request: `{ "code": "numpy.zeros(5).sum()" }`

Response: `{ "result": "0.0", "error": null, "exit_code": 0 }`

---

## SandboxInfo

```json
{
  "id": "sb-67a1b3-0000",
  "snapshot_tag": "py",
  "netns": "forkd-child-1",
  "guest_addr": "10.42.0.2:8888",
  "created_at_unix": 1717000123,
  "pid": 314159,
  "memory_limit_mib": 256
}
```

## SnapshotInfo

```json
{
  "tag": "py",
  "dir": "/var/lib/forkd/snapshots/py",
  "created_at_unix": 1717000000
}
```

## ErrorBody

Every 4xx and 5xx response carries:

```json
{ "error": "human-readable message" }
```
