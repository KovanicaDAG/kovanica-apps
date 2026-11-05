# node-desktop IPC contract

Authoritative bridge contract between the Tauri webview (`src/`) and the Rust
shell (`src-tauri/src/`). Both sides implement **exactly** this. If reality
cannot match it, change this file first, then the code.

All commands are `#[tauri::command]` and return `Result<T, String>`; the error
string is human-readable and safe to display (never contains key material).
Invoked from TS via `invoke` from `@tauri-apps/api/core`.

## Constants

```
EXPECTED_NETWORK_TAG = "4-0eacd5699210b145"
CHAIN_ID             = 4
MIN_STAKE_ATOMS      = 50_000_000_000_000   // 50_000 KUNA, 9 decimals
MIN_STAKE_KUNA       = "50000"
RPC_URL              = "http://127.0.0.1:8545/rpc"
LOG_LINES            = 200
```

## DTOs (serde `Serialize`, snake_case field names)

```rust
struct StatusDto {
  state: String,            // "stopped" | "starting" | "running" | "stopping" | "error"
  pid: Option<u32>,
  network_tag: Option<String>,
  expected_tag: String,     // EXPECTED_NETWORK_TAG
  chain_id: u32,            // 4
  committed_height: Option<u64>,
  local_round: Option<u64>,
  peer_count: Option<u32>,
  catching_up: Option<bool>,
  rpc_url: String,          // RPC_URL
  data_dir: String,
  config_path: String,
  last_error: Option<String>,
}

struct ValidatorDto {
  run_validator: bool,
  has_key: bool,
  key_address: Option<String>,   // kvnc...dag
  stake_atoms: Option<String>,   // decimal string, exact atoms
  stake_kuna: Option<String>,    // human formatted
  min_stake_atoms: String,       // "50000000000000"
  min_stake_kuna: String,        // "50000"
  eligible: bool,                // stake_atoms >= MIN_STAKE_ATOMS
  can_toggle: bool,              // state==running && eligible && catching_up==false
  stake_available: bool,         // state==running && !catching_up
  message: Option<String>,       // why not eligible / why staking unavailable
}

struct KeyDto {
  exists: bool,
  path: Option<String>,
  address: Option<String>,
}

struct ActionResult {
  ok: bool,
  message: Option<String>,
  tx_id: Option<String>,
}

struct SettingsDto {
  keep_running: bool,
}
```

## Commands

| command | args | returns | notes |
|---|---|---|---|
| `node_status` | — | `StatusDto` | cheap; polled every 1500 ms |
| `node_start` | — | `StatusDto` | runs tag+config gate; refuses on mismatch |
| `node_stop` | — | `StatusDto` | kills process group, waits for RPC port free |
| `node_logs` | — | `Vec<String>` | last 200 lines, oldest→newest |
| `validator_state` | — | `ValidatorDto` | reads self-stake over RPC |
| `set_run_validator` | `enabled: bool` | `ValidatorDto` | **refuses** `true` unless eligible; writes config + restarts; `false` never unbonds |
| `stake` | `amount_kuna: String` | `ActionResult` | submits the chain's existing stake tx; does not set `run_validator`; chain guard + RPC fee, see *Sign-path refusals* |
| `unbond` | — | `ActionResult` | existing unbond tx; honors 100_000-block period; chain guard + RPC fee, see *Sign-path refusals* |
| `key_status` | — | `KeyDto` | |
| `key_create` | `passphrase: String` | `KeyDto` | keystore v2; never logged |
| `key_import` | `secret: String, passphrase: String` | `KeyDto` | secret zeroized |
| `settings_get` | — | `SettingsDto` | |
| `set_keep_running` | `enabled: bool` | `SettingsDto` | |

## Sign-path refusals (`stake`, `unbond`)

Before a key signs anything, both commands run the chain guard, in this order:

1. node `state == "running"` (the sidecar is up);
2. `kovanica_syncStatus` answers and reports `networkTag`;
3. the reported chain id is `CHAIN_ID` (numeric `chainId`/`chain_id` when
   present, else the `<chain_id>-` prefix of `networkTag`);
4. `networkTag == EXPECTED_NETWORK_TAG`.

Then `kovanica_getNonce`, then `kovanica_estimateFee` (atoms per byte, must be
`>= 1`). The fee is `rate × bincode size` of the transaction (same rule as
`packages/signer` `feeForRate` and the light FFI). There is **no** hardcoded
or fallback fee.

Any failure returns `Ok(ActionResult { ok: false, message: Some(..), tx_id: None })`:
nothing is signed and `kovanica_sendRawTransaction` is never called. Messages
(UI may display them verbatim):

| condition | `message` |
|---|---|
| node not running | `Node is not running. Start the node before signing.` |
| `kovanica_syncStatus` failed | `Cannot verify chain: kovanica_syncStatus failed (<rpc error>). Refusing to sign.` |
| no `networkTag` | `Cannot verify chain: node did not report a network tag. Refusing to sign.` |
| chain id not parseable | `Cannot verify chain: node did not report a chain id. Refusing to sign.` |
| chain id ≠ 4 | `Chain id mismatch: node reports <id>, expected 4. Refusing to sign.` |
| tag ≠ expected | `Network tag mismatch: node reports '<tag>', expected '4-0eacd5699210b145'. Refusing to sign.` |
| `kovanica_estimateFee` failed | `Fee estimate unavailable: kovanica_estimateFee failed (<rpc error>). Refusing to sign.` |
| fee rate missing / `0` | `Fee estimate invalid: kovanica_estimateFee returned <value>. Refusing to sign.` |

`unbond` runs the guard before it reads `kovanica_getStake`.

## Frontend rules

- Vanilla TypeScript, no framework (matches `apps/console-desktop`).
- `tsconfig` has `strict`, `noUnusedLocals`, `noUnusedParameters` — no dead code.
- Dev server port **5175** (5173 = `apps/desktop`, 5174 = `apps/console-desktop`).
- Stake button disabled whenever `stake_available == false`.
- Validator toggle disabled whenever `can_toggle == false`; when off, it must be
  off-by-default on a fresh install.
- Exact option label text:
  `Run as a validator. There is no maximum. A 50,000 KUNA self-stake is required.`
- Exact checkbox label text: `Keep running after close`
- Unbond is a **separate** control and is never triggered by turning the
  validator option off.
- All rendering text that could contain a secret must never be interpolated.

## Rust shell rules

- Node is spawned as a child process on its own **process group**
  (Unix `setsid`/`setpgid` via `pre_exec`; Windows Job Object or `taskkill /T /F`).
- Stop must kill the group and block until `127.0.0.1:8545` is free, then return.
- Never write a validator key into the repo, installer, or CI.
- Keystore file mode `0600` on Unix; ACL-restricted on Windows.
- `run_validator = true` is written to the config only when self-stake is
  verified `>= MIN_STAKE_ATOMS` while the chain reports not catching up.
- Before every start: `kovanica-node --config <cfg> --check-config` then
  `kovanica-node --config <cfg> network-tag`; refuse unless the tag matches
  `EXPECTED_NETWORK_TAG`.
- Genesis files are copied from the bundle into `<data_dir>` before first start
  and an existing data dir is **never** overwritten.
