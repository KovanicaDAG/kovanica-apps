# Kovanica Node (Linux milestone 1)

Local supervisor for one `kovanica-node` process, with a read-only loopback monitor in the same window. This is not a wallet, not a miner, and not an Android or Windows package.

## What you select

Start stays disabled until Check setup accepts the exact executable, config, and data directory still in the form. Editing those fields, or starting another check, drops an in-flight result. A late success cannot approve a different combination, and a late failure cannot clear a newer acceptance. The three paths are:

- the `kovanica-node` executable
- a config file you already wrote
- the data directory

The app does not search `PATH`, download a binary, build one on start, or generate a config. It does not embed public seeds and it does not rewrite the file you selected.

## Node CLI this was checked against

Reviewed source: `KovanicaDAG/kovanica` `74045cf3e27a70569c32af35aec24ada5dd9ff79`.

The process is started as argv, never a shell string:

```text
<selected-binary> --config <selected-config>
```

There is no `--data-dir` flag on that CLI. `data_dir` is a field in the config. It must be absolute and must be the same directory you selected. Relative `data_dir` is rejected because the node would resolve it against the process working directory.

`KOVANICA_*` environment variables override the file inside the node. If any are set in this app's environment, Start is refused. Their values are not shown. This app does not delete them and does not edit the file.

## What the config must say

| Setting | Rule |
|---|---|
| `chain_id` | 2, 3, 4, 5, or 1337. Chain 1 (mainnet) is blocked. Any other id is blocked and is not relabelled. If the key is omitted the node default is 1337 and the window says so. |
| `run_validator` | Must be `false`. The node default is `true`, so an omitted key is rejected. |
| `[mining] enabled` | Must not be `true`. Omitted means the node default, which is off. |
| `rpc_addr` | `127.0.0.1`, `localhost`, or `::1` only. |
| `rpc_port` | 1–65535. Default 8545 when omitted. |
| `data_dir` | Absolute, and equal to the selected directory. |

P2P `listen_addr` is shown and left as written. This milestone does not rewrite it.

The executable SHA-256 shown after Check setup identifies the file bytes. It does not prove who built them. A debug `kovanica-node` was built from reviewed source `74045cf3e27a70569c32af35aec24ada5dd9ff79` outside this repo and was not committed. Its SHA-256 is `93767a3f0d272877a4cafb62138960990f5dde8d3d77598605012907543fca73`. Package version in that tree is `0.0.31`. The binary has no `--version` flag. A non-validator, non-mining chain-1337 loopback process, built from that reviewed source, reports `committedHeight` 0 before and after restart on the same data directory. `catchingUp: false` at height 0 is not a synced claim. Displayed height is not executed-state recovery.

Nonzero recovery was run on 2026-11-07 on Xvfb `:99` (openbox; the form was filled and the buttons were activated with xdotool, not on a physical display). The supervised binary was a debug `kovanica-node` built outside this repo and outside the active L1 checkout, from `KovanicaDAG/kovanica` `ee5db899e8b5e2f2d922fbe5034fc2dc1e7fab12` (main CI run 42380882775, success). Its SHA-256 is `9b1ece82d180a77f48c5fdcbfc67c63e4c323445f43735491156948d149a3973`. Docker was not installed here, so the producer was that same binary as a native process in the chain-5 compose node1 shape: loopback only (`127.0.0.1:8545`, `127.0.0.1:8000`), empty bootnodes, a fresh data directory, the published `funded_accounts.toml`, mining author 0, and a mode-0600 validator seed generated the way `ops/docker/entrypoint.sh` does. `KOVANICA_TEST_TARGET_BITS` was not set. This app supervised only the observer: chain 5, `run_validator = false`, mining disabled, RPC `127.0.0.1:8548`, bootnode `127.0.0.1:8000`, absolute paths, no `KOVANICA_*` in the child environment, and the selected config was not rewritten.

With the producer paused (`SIGSTOP`), the observer reported `committedHeight` 3 and author-0 reward balance `7500000000` (3 × 2.5 KVNC) via `kovanica_getBalance`. Stop in this window reaped that child with exit code 0; a bystander process was not signaled. Start on the same data directory, producer still paused, reported `committedHeight` 3, `peerCount` 0, and the same reward balance before any peer reconnect. The monitor showed those values with freshness `current` and left the missing health error as `unknown`, not zero. Resuming the producer advanced the supervised node to `committedHeight` 4 and reward balance `10000000000`. The app still does not start a validator or a miner. L1 logged that a commit seal was not stored because the staking committee was empty; PoW commitment and the reward balance still advanced. `/health` execution counters stayed `starting` / 0 across that restart; the recovered balance was read with `kovanica_getBalance`, not from those counters.

## Process control

- Start and Stop are explicit.
- A second Start is refused while the owned child is starting, running, or stopping.
- Stop sends `SIGTERM` to the process group created for that child (`setsid` in the child), waits 10 seconds, then `SIGKILL`, then waits 2 seconds for the child to be reaped.
- `Exited` is recorded only after `wait` confirms it. If the signal fails or the child is still alive, the state stays `stopping`, the pid is kept, and Stop can be retried. Nothing is killed by name.
- Closing the window uses that same stop. If it is confirmed, the window closes. If it is not, the window stays open.
- If the toolkit calls `exit` during teardown before that stop runs, an exit hook uses the same 10 second SIGTERM grace and then SIGKILL, only for the owned process group. It rechecks `/proc` start time before each signal, so a reused pid is not signaled. A zombie is already exited and is not SIGKILLed. A failed signal does not pick another pid. A bystander is not a target.
- On this GTK build an untrapped X error is logged and then `_exit`, which skips that hook. `BadWindow` and `BadDrawable` are ignored only during confirmed window teardown (close approved, or `GDK_DESTROY` on the monitored toplevel) and only when the error's resource id is that toplevel window's XID. Minimize, restore, a stale window-state event, an unrelated resource id, and a cancelled close do not arm teardown or suppress errors. Any other X error is passed to the previous handler. If that handler returns, the owned child is left running. `_exit` does not return, so a separate parent-death reaper (same 10 second SIGTERM grace, then SIGKILL, same `/proc` start-time check) stops the owned child if the GUI process disappears. The reaper adopts that child only when it is still a group-leader child of this process, or already reparented to init after this process has exited, and only for the published pid and start time. A pidfd is held across the signal so a reused pid is not a target. If the reaper process cannot be started, Start stops the new child instead of leaving it running. Normal Stop and a later Start disarm the previous reaper without signaling. The reaper does not run the stop merely because an X error was forwarded.
- Logs keep the last 200 lines. Lines that look like secrets are replaced with `[redacted]`. The config file is not copied into the log.

## Monitor

While the owned process is running and the window is visible, the app polls loopback:

- `GET {rpc}/health`
- `POST {rpc}/rpc` method `kovanica_syncStatus`

Shapes follow the reviewed L1 handlers (`status`, `peer_count`, camelCase `committedHeight`, `peerId`, `reportedTipRound`). The TypeScript SDK is not used. Missing or wrong-typed fields stay `unknown`. A failed read is `unavailable` or unreachable, not `0` and not synced. An explicit zero is shown as zero. `catchingUp: false` only means the node did not report an active catch-up.

Polling does not overlap, times out after 3 seconds, and backs off up to 30 seconds after failures. A hidden window pauses new reads. A response that finishes after Stop or a new Start is dropped. The window says the monitor is not observed until a read succeeds, unreachable when a read fails with no prior success, and stale when a later failure keeps the last values. A missing timestamp is not shown as current. Missing numbers stay unknown or unavailable, not zero. Displayed height is not treated as executed-state recovery. Each snapshot carries `process.statusSeq` and `monitorSeq`; a slower response does not replace a newer process or monitor observation.

There is no remote admin endpoint and no transaction submission.

## Build

From `node-desktop/`, with no sibling `crates/` tree:

```text
npm install
npm test
npm run lint
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --manifest-path src-tauri/Cargo.toml --lib --no-default-features
cargo clippy --manifest-path src-tauri/Cargo.toml --lib --no-default-features -- -D warnings
```

The default Cargo feature is `gui`, which links Tauri and therefore GTK/WebKit. The commands above test the supervisor, policy, and monitor without that feature. The window binary is:

```text
cargo build --manifest-path src-tauri/Cargo.toml --features gui
```

That needs `pkg-config` and the GTK/WebKit development packages (`libgtk-3-dev`, `libwebkit2gtk-4.1-dev`, and their dependencies). On this Debian 12 check those were GTK 3.24.38, glib 2.74.6, and WebKitGTK 4.1 2.50.6. `cargo build --manifest-path src-tauri/Cargo.toml --locked --features gui --bins` produced the debug binary. A `.deb` package was not built. The window was exercised on Xvfb `:99`, not on a physical display.

`console-desktop` still path-depends on `../../../crates` and is not fixed here.

