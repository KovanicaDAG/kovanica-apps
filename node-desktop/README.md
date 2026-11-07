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

The executable SHA-256 shown after Check setup identifies the file bytes. It does not prove who built them or that they match the reviewed source revision. This change did not build or hash a `kovanica-node` binary. Committed-height recovery against a real chain database was **not tested**.

## Process control

- Start and Stop are explicit.
- A second Start is refused while the owned child is starting, running, or stopping.
- Stop sends `SIGTERM` to the process group created for that child (`setsid` in the child), waits 10 seconds, then `SIGKILL`.
- The app never signals by executable name and never adopts another pid.
- Closing the window uses the same stop path. The node is not left running.
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

That needs `pkg-config` and the GTK/WebKit development packages. This sandbox did not have them, so the Linux package was not produced here.

`console-desktop` still path-depends on `../../../crates` and is not fixed here.

