# CubeSandbox bench methodology

The `CubeSandbox*` row in [`chart-spawn-100.png`](./chart-spawn-100.png)
combines two figures rather than measuring CubeSandbox locally:

1. Tencent's published P95 cold-spawn at 50 concurrent: **~90 ms**.
2. The cold `import numpy` cost a freshly-spawned sandbox pays
   before our benchmark workload completes: **~300 ms**.

Sum: **~390 ms** for the same end-to-end task forkd / Docker / gVisor /
Firecracker are measured on (`bench/bench-spawn-100.sh`).

## Why we didn't measure it directly

CubeSandbox's one-click installer expects a clean host. On a host
already running a server-management stack (in our case, 1Panel with
its own MySQL, Redis, Grafana, openresty), CubeSandbox conflicts on
five default ports and ultimately fails at a `cubelet`-internal
storage step (`mke2fs` on an XFS-loop-mounted `/data/cubelet`).

We could resolve the port conflicts via the installer's `.env`
overrides but the storage failure is harder to work around without
disrupting the existing services on the host.

## Reproducing on a clean host

If you have access to a fresh Linux machine, install CubeSandbox per
its README and run `bench/compare-all.py --backend cubesandbox` to
generate a real measurement. The chart generator picks up the result
via `BENCH_RESULTS=/path/to/results.json`.
