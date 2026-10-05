# claude-bridge on the TimeTracker VM

The HRMS Knowledge builder (KT document, summary PDF, presentation, chat index) calls Claude through
**claude-bridge** — a small Node service that wraps the `claude` CLI so callers never hold model
credentials. Since 5 Oct 2026 our own copy runs on the TimeTracker VM; before that, generation used
Tapan's bridge on his VM.

- **Public URL:** `https://time-tracker.rapidinnovation.dev/bridge` (nginx `location /bridge/`, this repo)
- **Callers set:** `CLAUDE_BRIDGE_URL` = that URL and `CLAUDE_BRIDGE_TOKEN` = the bridge's token.
  The builder runs inside the HRMS worker (`perf_tracker/screening_worker`) on Tapan's VM, so his
  worker's env is where those two values go.
- **Source:** `ops/claude-bridge/server.mjs` in `TapanTalukdar004/RUH-HRMS` (v2.4.0 deployed).

## What lives on the VM (not in git)

| Path | What |
|---|---|
| `/home/dell/claude-bridge/server.mjs` | copy of the bridge (Node built-ins only, runs on the host's Node 20) |
| `/home/dell/claude-bridge/bridge.env` | config + secrets, mode `600` — see below |
| `/home/dell/claude-bridge/workdir/` | CLI scratch dir. **Must exist**: the bridge doesn't create it, and a missing one fails every call with a misleading `spawn /usr/bin/claude ENOENT` |
| `/etc/systemd/system/claude-bridge.service` | the service, with the caps below |
| `/usr/bin/claude` | Claude Code CLI, npm `stable` channel (2.1.285 at setup). A native binary, so the package's `node >=22` engine warning doesn't matter on Node 20 |

`bridge.env` keys: `BRIDGE_PORT=8787`, `BRIDGE_TOKEN` (random, 64 hex), `BRIDGE_DEFAULT_MODEL=claude-sonnet-5`,
`BRIDGE_ALLOWED_MODELS=claude-opus-4-8,claude-sonnet-5,claude-haiku-4-5`, `CLAUDE_BIN=/usr/bin/claude`,
`BRIDGE_WORKDIR=/home/dell/claude-bridge/workdir`, `CLAUDE_CODE_OAUTH_TOKEN` (from `claude setup-token`).

## Why it's capped

This VM is an `e2-medium` (2 vCPUs, 4 GB) that also serves the production TimeTracker API. Every request
spawns a full `claude` process, and the bridge allows up to 3 at once (`MAX_CONCURRENT`, hard-coded). So
the unit makes the **bridge, never the API**, the loser under pressure:

```ini
MemoryHigh=550M        # throttle point
MemoryMax=700M         # one limit for the bridge AND every claude it spawns; over it, only they die
OOMScoreAdjust=1000    # if the whole VM runs short, the kernel kills these first
CPUQuota=60%           # at most 0.6 of a core
Nice=10                # the API wins any CPU contention
```

In practice concurrency is one: the HRMS worker builds one Knowledge job at a time, and each job makes a
single outline call. Each call is slower here than on a laptop (~13 s for a trivial chat turn) because of
the CPU cap — fine for KT builds, too slow for interactive chat until the VM is resized.

Port 8787 is closed by the GCP firewall; `/bridge/` through nginx is the only way in, and every request
still needs the bridge token.

## Operating it

```bash
ssh timetracker-vm
systemctl status claude-bridge
journalctl -u claude-bridge -f                       # one line per call: model, size, duration
curl -s localhost:8787/healthz                       # no auth needed
curl -s https://time-tracker.rapidinnovation.dev/bridge/healthz
```

- **Renew the Claude login** (if calls fail with `OAuth session expired`): run `claude setup-token` on any
  machine logged into the seat, put the new value after `CLAUDE_CODE_OAUTH_TOKEN=` in `bridge.env`, then
  `sudo systemctl restart claude-bridge`. The seat pays for every call — anyone with this token can use it.
- **Rotate `BRIDGE_TOKEN`** without downtime: set `BRIDGE_TOKEN_OLD` to the current value and
  `BRIDGE_TOKEN` to a new one, restart, switch the callers, then remove `BRIDGE_TOKEN_OLD` and restart again.
- **Update the bridge:** copy the new `server.mjs` over, restart, and confirm `/healthz` shows the new version.

**Don't remove `extra_hosts` from the nginx service while `location /bridge/` is still in `nginx.conf`.**
nginx resolves `host.docker.internal` at startup, so without the host entry nginx refuses to start and the
whole site goes down. Removing the route first (or both together) is safe.
