# node-desktop IPC contract (milestone 1)

Commands are `#[tauri::command]`. JavaScript calls them with `invoke` from `@tauri-apps/api/core`. Errors are plain strings safe to show. They do not include environment values.

There is no wallet, stake, mining, or key command.

## Commands

| Command | Arguments | Result |
|---|---|---|
| `inspect_setup` | `binary`, `configPath`, `dataDir` | `CheckedSetup` or an error. Does not start a process or write the config. |
| `node_start` | same three paths | Re-checks the setup, spawns `<binary> --config <config>`, returns a snapshot. |
| `node_stop` | none | SIGTERM, 10s, SIGKILL, only for the owned child. Returns a snapshot. |
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

`RunEvent::ExitRequested` calls the same stop function as `node_stop`.
