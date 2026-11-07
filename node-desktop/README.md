# Kovanica Node (Linux milestone 1)

Local supervisor for one `kovanica-node` process, with a read-only loopback monitor in the same window. This is not a wallet, not a miner, and not an Android or Windows package.

## What you select

Start stays disabled until Check setup accepts three absolute paths:

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

The executable SHA-256 shown after Check setup identifies the file bytes. It does not prove who built them. A debug `kovanica-node` was built from reviewed source `74045cf3e27a70569c32af35aec24ada5dd9ff79` outside this repo and was not committed. Its SHA-256 is `93767a3f0d272877a4cafb62138960990f5dde8d3d77598605012907543fca73`. Package version in that tree is `0.0.31`. The binary has no `--version` flag. A non-validator, non-mining chain-1337 loopback process reports `committedHeight` 0 before and after restart on the same data directory. Nonzero committed-state recovery was **not tested**: this app will not start a validator or a miner, and that node does not certify a leader without a seal quorum. `catchingUp: false` at height 0 is not a synced claim.

## Process control

- Start and Stop are explicit.
- A second Start is refused while the owned child is starting, running, or stopping.
- Stop sends `SIGTERM` to the process group created for that child (`setsid` in the child), waits 10 seconds, then `SIGKILL`, then waits 2 seconds for the child to be reaped.
- `Exited` is recorded only after `wait` confirms it. If the signal fails or the child is still alive, the state stays `stopping`, the pid is kept, and Stop can be retried. Nothing is killed by name.
- Closing the window uses that same stop. If it is confirmed, the window closes. If it is not, the window stays open.
- If the toolkit calls `exit` during teardown before that stop runs, an exit hook uses the same 10 second SIGTERM grace and then SIGKILL, only for the owned process group. It rechecks `/proc` start time before each signal, so a reused pid is not signaled. A zombie is already exited and is not SIGKILLed. A failed signal does not pick another pid. A bystander is not a target.
- On this GTK build an untrapped X error is logged and then `_exit`, which skips that hook. `BadWindow` and `BadDrawable` are ignored only after teardown has started (a GDK delete or destroy event, or a window-state event whose new state is withdrawn or iconified). Focus-only changes do not arm that. Any other X error, and those two before teardown, are passed to the toolkit handler.
- Logs keep the last 200 lines. Lines that look like secrets are replaced with `[redacted]`. The config file is not copied into the log.

## Monitor

While the owned process is running and the window is visible, the app polls loopback:

- `GET {rpc}/health`
- `POST {rpc}/rpc` method `kovanica_syncStatus`

Shapes follow the reviewed L1 handlers (`status`, `peer_count`, camelCase `committedHeight`, `peerId`, `reportedTipRound`). The TypeScript SDK is not used. Missing or wrong-typed fields stay `unknown`. A failed read is `unavailable` or unreachable, not `0` and not synced. An explicit zero is shown as zero. `catchingUp: false` only means the node did not report an active catch-up.

Polling does not overlap, times out after 3 seconds, and backs off up to 30 seconds after failures. A hidden window pauses new reads. A response that finishes after Stop or a new Start is dropped.

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

