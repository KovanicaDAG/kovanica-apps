# node-desktop IPC contract (milestone 1)

Commands are `#[tauri::command]`. JavaScript calls them with `invoke` from `@tauri-apps/api/core`. Errors are plain strings safe to show. They do not include environment values.

There is no wallet, stake, mining, or key command.

## Commands

| Command | Arguments | Result |
|---|---|---|
| `inspect_setup` | `binary`, `configPath`, `dataDir` | `CheckedSetup` or an error. Does not start a process or write the config. |
| `node_start` | same three paths | Re-checks the setup, spawns `<binary> --config <config>`, returns a snapshot. |
| `node_stop` | none | SIGTERM, grace, SIGKILL, only for the owned child. Returns a snapshot after the child is reaped. On signal or wait failure the error string is returned, state stays `stopping`, and the pid is kept. |
| `node_status` | none | Last snapshot. Does not call RPC. |
| `monitor_refresh` | none | One non-overlapping loopback read when the owned process is running and the poll gate allows it. |
| `set_monitor_paused` | `paused` | Hides the window from starting a new read. |

## CheckedSetup

`binary`, `binarySha256`, `configPath`, `dataDir`, `chainId`, `chainLabel`, `chainIdFromFile`, `rpcAddr`, `rpcPort`, `rpcHttp`, `listenAddr`, `miningEnabled` (always false when accepted), `runValidator` (always false when accepted).

`binarySha256` is the SHA-256 of the selected file bytes. It is not a publisher signature.

## Snapshot

- `process.state`: `stopped`, `starting`, `running`, `stopping`, `exited`
- `process.pid`, `process.exitCode`, `process.lastError`, `process.logs`
- `monitor.rpc`: `unknown`, `reachable`, `unreachable`
- `monitor.health` and `monitor.sync` fields: `{ kind: "value" \| "unknown" \| "unavailable", value }`
- `monitor.stale`, `monitor.lastSuccessAgeMs`, `monitor.detail`
- `setup`, `reviewedNodeSource`, `stopGraceSecs`, `exitStopsOwnedChild`

Sync field names in the snapshot are camelCase JSON of the Rust view (`committedHeight`, `localTipRound`, `catchingUp`, `peerCount`, `networkTag`, `peerIds`). They are parsed from the node's camelCase `kovanica_syncStatus` result, not from the SDK's snake_case types.

## Exit

`WindowEvent::CloseRequested` and `RunEvent::ExitRequested` call the same `stop_owned` path as `node_stop`.

`Ok` means the child was reaped (or there was no owned child). The window may close.
`Err` means the child was not confirmed exited. The pid stays owned, the state stays `stopping`, `CloseRequested` calls `prevent_close`, and `ExitRequested` calls `prevent_exit`. The app does not report that cleanup succeeded.

On this WebKitGTK build, destroying the window can raise `BadDrawable` and the process can `exit` before those events run. An `atexit` hook then signals the owned process group only, after comparing `/proc/<pid>/stat` start time. It does not clear a failure into `exited` and it does not pick a pid by name.
