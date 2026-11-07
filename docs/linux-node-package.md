# Linux Node package plan

Status: proposal for Toni. Nothing in this document is packaged, installed, or released.

Base: `c09fe0d51d6b1479f19aacaef355653d7dfaf1c3` (`origin/main`, merge of [PR #7](https://github.com/KovanicaDAG/kovanica-apps/pull/7)). That commit matches the reference. No drift. Main `node-desktop` run [42396544477](https://github.com/KovanicaDAG/kovanica-apps/actions/runs/42396544477) succeeded. This plan does not repeat the chain-5 recovery work merged there.

Scope: the Linux supervisor and its read-only monitor in [`node-desktop`](../node-desktop/README.md). Not Wallet, mining UI, Android, Windows, Comms, `console-desktop`, L1, chain-2 activation, or a cleanup of old data directories.

## Recommendation

Ship one format: a Debian `.deb`, for **Debian 12 (bookworm) amd64** only.

Do not describe it as a generic Linux package. No other distribution or architecture has a measured runtime link, and no `.deb` has been built.

Why this and not a portable archive:

| | Debian 12 amd64 `.deb` | Portable archive (tar of the binary) | AppImage |
|---|---|---|---|
| Already selected | [`tauri.conf.json`](../node-desktop/src-tauri/tauri.conf.json) `bundle.targets` is `["deb"]` only | No | No. `console-desktop` lists `appimage`; this app does not, and that app is out of scope |
| Runtime libraries | Declared `Depends`, installed by apt from Debian 12 | The caller must already have the same sonames | Would vendor WebKit. Not built or tested |
| Uninstall tracking | `dpkg` removes only the files it installed | None, unless a later script is invented | None in this repo |
| Evidence | Dynamic `NEEDED` of the existing debug GUI binary was read on Debian 12 amd64 (below). The `.deb` itself was not produced | A tarball of that binary still needs the distro WebKit stack, so it is not portable | No AppImage exists for this app |

Ubuntu 24.04 is the current CI runner ([`.github/workflows/node-desktop.yml`](../.github/workflows/node-desktop.yml)). That job compiles and tests. It does not bundle, install, or open a window. glibc and WebKit there are not the Debian 12 versions measured below. A binary built on the CI runner is not the Debian 12 package. Do not publish one `.deb` for both.

A later Ubuntu package is a new measurement (`objdump -p` on a release binary built on that release), not a rename of this one.

## What already exists

Use these. Do not re-specify them as new behavior.

| Capability | Where |
|---|---|
| Explicit absolute executable, config, and data directory. No `PATH` lookup, no download, no build, no config rewrite | [`policy.rs`](../node-desktop/src-tauri/src/policy.rs) `check_setup`; [`index.html`](../node-desktop/index.html) setup note |
| Start calls `check_setup` again. It does not trust an older Check | [`main.rs`](../node-desktop/src-tauri/src/main.rs) `node_start` |
| Any `KOVANICA_*` name blocks Check and Start. Values are not shown and are not unset | `kovanica_env_names`, `current_kovanica_env` |
| Child is `<selected-binary> --config <selected-config>`, its own session (`setsid`). Stop is `SIGTERM`, 10s, then `SIGKILL`, only for that process group | [`supervisor.rs`](../node-desktop/src-tauri/src/supervisor.rs) `start`, `STOP_GRACE` |
| Close uses that stop. Unconfirmed stop keeps the window open. Parent-death reaper is the same binary, same grace, pid and start time checked | [`IPC-CONTRACT.md`](../node-desktop/IPC-CONTRACT.md), `enable_parent_death_reaper` |
| No shell plugin and no filesystem plugin | [`capabilities/default.json`](../node-desktop/src-tauri/capabilities/default.json) |
| Window copy already says paths are absolute and the app does not search `PATH`, download a binary, or write a config | [`index.html`](../node-desktop/index.html) |
| Icons listed for the bundler are placeholders, not the KUNA logo | [`icons/README.md`](../node-desktop/src-tauri/icons/README.md) |
| Version `0.2.0` in `package.json`, `src-tauri/Cargo.toml`, and `tauri.conf.json` | those three files |
| Recovery command and exit 2 = BLOCKED | [`README.md`](../node-desktop/README.md), `npm run recovery` |
| CI gates, no node process, no physical desktop | [`.github/workflows/node-desktop.yml`](../.github/workflows/node-desktop.yml) |

`bundle.linux.deb.depends` is `[]`. That is a gap, not a finished dependency list. See Runtime dependencies.

Reviewed L1 source pin `74045cf3e27a70569c32af35aec24ada5dd9ff79` (`REVIEWED_NODE_SOURCE`) stays a policy constant. It is not a file inside the package. The historical binary checksums in the README are records of earlier runs. They are not approved contents, and the active DAA working tree is not an approved binary.

## Package contents

Identifier `online.kovanica.node`. Product name stays `Kovanica Node`. Cargo bin name stays `kovanica-node-desktop`.

Tauri CLI 2.12.4 (the installed `@tauri-apps/cli`) builds the deb in `node-desktop/src-tauri/target/release/bundle/deb/`. From that CLI's bundler (`tauri-cli-v2.12.4` `crates/tauri-bundler/src/bundle/linux/debian.rs` and `freedesktop/mod.rs`):

| Control or path | Expected value | Confirmed by a built deb? |
|---|---|---|
| `Package` | `heck::AsKebabCase` of the product name. For `Kovanica Node` that should be `kovanica-node`. Assert it on the first build | No |
| `Version` | `0.2.0` | Field exists in config; deb not built |
| `Architecture` | `amd64` when the binary arch is x86_64 | Not built |
| Filename | `Kovanica Node_0.2.0_amd64.deb` (product name is not sanitized in the filename, so it contains a space) | Not built |
| `/usr/bin/kovanica-node-desktop` | Release supervisor. Also the parent-death reaper (`--parent-death-reaper`) | Not built |
| `/usr/share/applications/Kovanica Node.desktop` | Generated desktop entry | Not built |
| `/usr/share/icons/hicolor/<size>/apps/kovanica-node-desktop.png` | PNGs from `bundle.icon` | Not built |

The installed tree must contain only supervisor files under `/usr`. It must not contain:

- a `kovanica-node` binary (`/usr/bin/kovanica-node` must be absent; the package name `kovanica-node` is not that binary)
- a genesis file, chain database, `funded_accounts.toml`, validator seed, wallet, or key
- anything under `/etc`, `/var`, or `/home`
- a systemd unit, D-Bus service, or autostart entry

No `bundle.resources` and no `externalBin` exist today. The implementation must not add them. `beforeBuildCommand` stays `npm run build`. It must not download a node.

Desktop entry, as that CLI generates it unless a template is added:

```ini
[Desktop Entry]
Categories=
Exec=kovanica-node-desktop
StartupWMClass=kovanica-node-desktop
Icon=kovanica-node-desktop
Name=Kovanica Node
Terminal=false
Type=Application
```

`Exec` is the supervisor's file name. The desktop session finds `/usr/bin/kovanica-node-desktop` through the normal path. That is not discovery of the L1 node. The entry must not pass `--config`, must not name a data directory, and must not set any `KOVANICA_*` variable. There is no `Environment=` line in the Tauri template.

`Categories=` is empty because `tauri.conf.json` sets no category. The implementation sets `bundle.category` to `Utility` and then reads the installed desktop file. `Categories=` must be non-empty and include `Utility`. If this CLI emits a different token, stop and record the actual line. Do not add a second category to force a menu.

`Icon` and the hicolor file name follow the binary name, `kovanica-node-desktop`. Keep the placeholder PNGs already listed in `bundle.icon`. Do not draw a new logo in the packaging change. `icon.icns` and `icon.ico` are already in the config for other Tauri targets; they are not a reason to put those files on Linux, and they are not the KUNA mark.

The bundler writes `Depends` only when `deb.depends` is non-empty. An empty array omits `Depends`. There is no `dpkg-shlibdeps` step in that CLI. The implementation replaces `[]` with the explicit list below. It does not add maintainer scripts (`preinst`, `postinst`, `prerm`, `postrm`). A script is the wrong place to stop a process or delete a directory.

The bundler does not write a copyright file. This repo has no `LICENSE` file. `Cargo.toml` says `GPL-3.0-or-later`. Do not invent a different license, and do not add the GPL body until the maintainer decision below is made.

## Runtime dependencies

Measured on the machine that produced this plan, not on a clean install and not on Ubuntu:

| Fact | Value |
|---|---|
| OS | Debian GNU/Linux 12 (bookworm), `amd64`, `x86_64` |
| glibc | `2.36-9+deb12u14` (`libc6`) |
| GTK | `pkg-config` 3.24.38, package `libgtk-3-0` `3.24.38-2~deb12u3` |
| glib | `pkg-config` 2.74.6, package `libglib2.0-0` `2.74.6-2+deb12u9` |
| WebKitGTK 4.1 | `pkg-config` 2.50.6, package `libwebkit2gtk-4.1-0` `2.50.6-1~deb12u2` |
| JavaScriptCore | package `libjavascriptcoregtk-4.1-0` `2.50.6-1~deb12u2` |
| libsoup 3 | `pkg-config` 3.2.3, package `libsoup-3.0-0` `3.2.3-0+deb12u2` |
| OpenSSL | `libssl3` `3.0.22-1~deb12u2` is installed. The debug binary does **not** `NEEDED` `libssl.so.3` |
| rustc | 1.99.0, same as CI |
| Binary inspected | `node-desktop/src-tauri/target/debug/kovanica-node-desktop` (debug, 236 MB). Not a release binary and not a package |

Direct `NEEDED` (`objdump -p`), mapped with `dpkg -S` on that Debian 12 system:

| Soname | Debian package |
|---|---|
| `libc.so.6` | `libc6` |
| `libgcc_s.so.1` | `libgcc-s1` |
| `libglib-2.0.so.0`, `libgobject-2.0.so.0`, `libgio-2.0.so.0` | `libglib2.0-0` |
| `libgtk-3.so.0`, `libgdk-3.so.0` | `libgtk-3-0` |
| `libgdk_pixbuf-2.0.so.0` | `libgdk-pixbuf-2.0-0` |
| `libcairo.so.2` | `libcairo2` |
| `libdbus-1.so.3` | `libdbus-1-3` |
| `libX11.so.6` | `libx11-6` |
| `libwebkit2gtk-4.1.so.0` | `libwebkit2gtk-4.1-0` |
| `libjavascriptcoregtk-4.1.so.0` | `libjavascriptcoregtk-4.1-0` |
| `libsoup-3.0.so.0` | `libsoup-3.0-0` |

`libayatana-appindicator3` is not linked. It was not installed here. CI installs `libayatana-appindicator3-dev` before the GUI compile. That can stay a **build** package. It is not a runtime dependency unless a future release binary's `NEEDED` lists it.

Transitive libraries (WebKit's gstreamer, ICU, and the rest of `ldd`) stay dependencies of those Debian packages. Do not copy the `ldd` closure into `Depends`.

Proposed `bundle.linux.deb.depends`, versions unpinned except glibc, so a Debian 12 security update of WebKit still satisfies the package:

```text
libc6 (>= 2.36), libgcc-s1, libglib2.0-0, libgtk-3-0, libgdk-pixbuf-2.0-0, libcairo2, libdbus-1-3, libx11-6, libwebkit2gtk-4.1-0, libjavascriptcoregtk-4.1-0, libsoup-3.0-0
```

`libc6 (>= 2.36)` matches the glibc this binary was linked against. It does not mean the binary runs on every newer distro. Ubuntu 22.04 is glibc 2.35 and is out of scope. Nothing newer than Debian 12 was tested.

After the first **release** bundle, repeat `objdump -p` on `/usr/bin/kovanica-node-desktop` inside the deb (extract it; do not install over a real data directory). If `NEEDED` differs from the debug list, fix `Depends` from the release list before calling the package done. Do not add `libssl3` unless that release binary needs it. `reqwest` is built with `rustls-tls-native-roots`. The monitor talks to loopback `http` only.

Build packages are the ones CI already installs, which are a superset of what the debug compile used here. This Debian 12 machine had `build-essential`, `pkg-config`, `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`, `libjavascriptcoregtk-4.1-dev`, `libsoup-3.0-dev`, and `libssl-dev`. It did **not** have `libayatana-appindicator3-dev`, `librsvg2-dev`, `libxdo-dev`, or `patchelf`. Those four are in the CI install list and are required for the bundling command below even though the debug `cargo build` succeeded without them. `patchelf` is required by the Tauri Linux bundler.

## Reproducible build commands

From a fresh `kovanica-apps` checkout on Debian 12 amd64, with no sibling L1 tree. Rust 1.99.0 with `rustfmt` and `clippy`. Node 22, matching CI. Do not set `KOVANICA_*`.

```sh
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  build-essential \
  curl \
  file \
  libayatana-appindicator3-dev \
  libgtk-3-dev \
  libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev \
  libssl-dev \
  libwebkit2gtk-4.1-dev \
  libxdo-dev \
  librsvg2-dev \
  patchelf \
  pkg-config \
  wget

cd node-desktop
npm ci
npm test
npm run lint
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib --no-default-features
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --lib --no-default-features -- -D warnings
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --features gui --bins --tests -- -D warnings
cargo build --manifest-path src-tauri/Cargo.toml --locked --features gui --bins
cargo test --manifest-path src-tauri/Cargo.toml --locked --features gui --test reaper_cli
npm run tauri -- build --bundles deb --ci -f gui -- --locked
```

`npm run tauri` is the existing script (`tauri`). `--bundles deb` matches the config. `--ci` skips prompts. `-f gui` matches the feature CI passes to cargo. Arguments after `--` go to cargo; `--locked` is required so the bundle cannot float off `Cargo.lock`. `beforeBuildCommand` runs `npm run build` again.

That command was checked against `package.json` and `npx tauri build --help` (CLI 2.12.4). It was not executed. Expected output path, untested:

```text
node-desktop/src-tauri/target/release/bundle/deb/Kovanica Node_0.2.0_amd64.deb
```

Quote the path. It has a space. Then:

```sh
sha256sum "node-desktop/src-tauri/target/release/bundle/deb/Kovanica Node_0.2.0_amd64.deb"
dpkg-deb -I "node-desktop/src-tauri/target/release/bundle/deb/Kovanica Node_0.2.0_amd64.deb"
dpkg-deb -c "node-desktop/src-tauri/target/release/bundle/deb/Kovanica Node_0.2.0_amd64.deb"
```

Do not commit the deb, the release binary, or `sha256sum` output. The checksum is a log line for that build.

`npm run recovery` is not part of the package build. It stays the live recovery command. Without `NODE_RECOVERY_BIN` it exits 2 and writes a BLOCKED report. That is not a package pass.

## First-run guidance

The package does not remember a node. There is no filesystem plugin, no settings file, and no `localStorage` of the three paths. Every launch opens the same empty form. Start stays disabled until Check setup accepts the paths still in the fields. Editing a field drops an in-flight acceptance. That behavior is already implemented and tested. Packaging must not add a default path, a PATH search, a download, a generated config, an automatic Start, or a clearer for `KOVANICA_*`.

Guidance is text. It does not widen `check_setup`. Proposed addition to the existing setup note in `index.html`, and the same words in `node-desktop/README.md`, when implementation is approved:

> This package is the supervisor and the read-only monitor. It does not include `kovanica-node`, a genesis file, a key, or a chain database. Type the absolute path of a node executable you already have, the absolute path of a config file you already wrote, and the absolute data directory named in that file. Check setup accepts or refuses those three paths. Start stays off until the check matches the paths still in the form. This window does not search `PATH`, download or build a node, rewrite the config, or unset `KOVANICA_*`. If any `KOVANICA_*` name is set in the environment that launched this window, Check setup refuses and does not show the value. Unset those names outside this app and leave the file as it is.

No file dialog in the first package. A dialog that returned an absolute path would still have to call `check_setup` unchanged, and the current tests cover typed fields. Adding a plugin would change `capabilities/default.json`, which this plan does not allow.

The desktop entry launches the supervisor only. The user still has to bring their own binary and their own files. A missing binary, a relative path, `run_validator = true`, mining enabled, a non-loopback `rpc_addr`, a mismatched `data_dir`, chain id 1, or an unknown chain id keeps the existing error strings from `check_setup`. The file is not modified.

## Upgrade and uninstall

The package owns `/usr/bin/kovanica-node-desktop`, the desktop file, and the hicolor icons. It owns nothing else.

| Action | Allowed effect | Forbidden |
|---|---|---|
| Upgrade (`dpkg -i` of a newer deb, or apt upgrade) | Replace those `/usr` files | Deleting or editing a user config, database, wallet, or key. Killing any process. Rewriting `Depends` into a `prerm` that signals by name |
| Remove / purge | `dpkg` deletes the files it installed | `rm` of `$HOME`, of a data directory, or of `~/.config`. `killall`, `pkill`, or `kill` of `kovanica-node` or of any pid the package did not record |
| App close | Existing Stop, 10 second grace, then SIGKILL, owned process group only. Reaper if the GUI dies | Changing that grace or signaling a bystander |

Do not add maintainer scripts. Tauri writes them only when paths are configured. Leave those paths unset.

`dpkg` will not stop a running supervisor. The running process keeps its old inode until it exits; its reaper is that same binary. The user closes the window, which is the existing Stop. If the package is removed while the window is still open, the process is allowed to remain. The uninstall test must show that, then the test itself may signal only the pid it started.

User configs and chain data never lived in the package, so upgrade cannot migrate them and must not try. The next launch still asks for the three absolute paths. `check_setup` still refuses to rewrite the config.

Replacing `/usr/bin/kovanica-node-desktop` does not retarget an already published reaper. Normal Stop disarms the reaper without signaling, as it does today. A later Start from the upgraded binary arms a new reaper from the new file. Do not split the reaper into a second package.

## Acceptance

Two layers. A green headless run is not a GUI smoke pass. An Xvfb smoke pass is not a physical desktop pass. A missing external node is not a recovery pass.

### Headless, no L1 binary, suitable for CI

These do not start a display and do not need `kovanica-node`.

| Check | How | Pass |
|---|---|---|
| Existing policy, stop, bystander, sequence, and UI tests | the `npm` and `cargo` commands in the build section | exit 0 |
| Control fields | `dpkg-deb -I` | `Package: kovanica-node` (or stop if the actual name differs), `Version: 0.2.0`, `Architecture: amd64`, `Depends` contains every package in the list above, no node package |
| Payload | `dpkg-deb -c` | `usr/bin/kovanica-node-desktop` present. `usr/bin/kovanica-node` absent. No `genesis`, `funded_accounts`, `.toml` key material, wallet, or database. No path outside `usr/` except the deb's own `DEBIAN` metadata |
| Desktop file | extract and read it | `Exec=kovanica-node-desktop`, no `--config`, no `Environment=`, no `KOVANICA_`, `Terminal=false`, `Categories=` contains `Utility` |
| Release `NEEDED` | `objdump -p` on the extracted binary | every soname is owned by a package named in `Depends` on Debian 12; no unexpected `libssl` or appindicator unless `Depends` was updated to match |
| Checksums | `sha256sum` of the deb and of the extracted binary | printed in the job log, not committed |
| Absent node is still installable | the supervisor package does not depend on a node package | `Depends` has no `kovanica-node` binary package |
| Invalid setup and missing file | already in `policy.rs` tests (`rejects_relative_binary_and_unknown_chain` and the validator, mining, data-dir, and env cases) | those tests stay green. They do not need the deb |
| Stop does not kill a bystander | existing supervisor and `reaper_cli` tests | unchanged |

A missing-library **install** (apt refusing the deb on a root without `libwebkit2gtk-4.1-0`) needs a disposable Debian 12 root. The control-file assertion above is the CI substitute. The real apt refusal is part of the smoke below, not a reason to mark CI green when the field is missing.

### GUI / package smoke

Not the current CI job. Not a physical monitor, keyboard, or session. If it is run, record `Xvfb`, the display number, and that the window was not touched on hardware. Previous milestone notes that used Xvfb `:99` and xdotool stay historical. They are not this package's acceptance.

Disposable directories only. Do not point the smoke at a real wallet, a real chain database, or `/tmp/kovanica`.

| Check | Needs an external L1 binary? | Pass |
|---|---|---|
| `dpkg -i` or `apt-get install` of the deb on Debian 12 amd64 with the `Depends` available from Debian | No | `command -v` resolves `/usr/bin/kovanica-node-desktop`. `dpkg -L kovanica-node` is only `/usr` files |
| Install on a root that lacks `libwebkit2gtk-4.1-0`, using apt so dependencies are not ignored | No | apt exits non-zero and does not leave `/usr/bin/kovanica-node-desktop` |
| Launch `Kovanica Node.desktop` under Xvfb (`gtk-launch 'Kovanica Node'` or the equivalent). The file name has a space because the product name does | No | Window title `Kovanica Node`. The three fields are empty. Start is disabled. No process named `kovanica-node` appears |
| Check setup with a missing absolute path | No | Existing error that the executable does not exist. No download and no new file |
| Check setup with `run_validator = true`, a relative `data_dir`, or any `KOVANICA_*` name in the environment | No | Existing refusal. Config bytes unchanged. Values of `KOVANICA_*` not shown |
| Start / Stop / close against an absolute executable script that sleeps, plus a config `check_setup` already accepts (loopback, `run_validator = false`, mining off, absolute `data_dir`) | No. This is not chain evidence | Owned pid exits. A bystander pid stays. A marker file in the disposable data directory stays |
| Upgrade: install the deb again over itself while that marker and config exist | No | Marker and config bytes unchanged. No extra pid disappears |
| Uninstall without closing a supervisor the test started | No | After `dpkg -r kovanica-node`, `/usr/bin/kovanica-node-desktop` is gone, the marker remains, and the test's child pid is still alive. The test then signals only that pid |
| Live monitor, committed height, balance | **Yes.** Absolute executable chosen outside this repo, plus a config and data directory the operator already trusts | Not part of package CI. Use `npm run recovery` with `NODE_RECOVERY_BIN` and `NODE_RECOVERY_FUNDED_ACCOUNTS`. Exit 2 is BLOCKED, not a pass. Do not point it at an unreviewed DAA tree and do not set `KOVANICA_TEST_TARGET_BITS` |

## CI

Keep the existing `ubuntu-24.04` job. It remains the gate for `npm ci`, `npm test`, `npm run lint`, `npm run build`, `cargo fmt --check`, locked library tests, both clippy invocations, the locked GUI build, and `reaper_cli`. It must continue not to start `kovanica-node` and not to treat a missing binary as success.

Add a second job only in the implementation PR, after this plan is approved:

- Runs in a Debian 12 container, not on the Ubuntu host's libraries. Pin the image digest in that PR. This plan does not choose a digest.
- Same contents permission as today: `contents: read`. No release token, no signing key, no apt upload.
- Installs the build packages listed above, then runs the build commands, including `npm run tauri -- build --bundles deb --ci -f gui -- --locked`.
- Runs the headless table (`dpkg-deb -I`, `dpkg-deb -c`, desktop-file text, `objdump -p`, `sha256sum`).
- Prints the checksums in the log. Does not `gh release`, does not sign, and does not `actions/upload-artifact` the deb. The runner's copy is discarded with the job.
- Does not run Xvfb. An Xvfb smoke, if someone runs it by hand, is labeled Xvfb and is not physical-desktop validation.
- Does not run `npm run recovery` and does not download a node. A missing live binary must not fail this job and must not be reported as recovery.

If the container cannot install the WebKit **development** packages or Rust 1.99.0, the job fails visibly. Do not fall back to bundling on Ubuntu 24.04 and calling the result a Debian 12 package.

## Gaps

| Gap | Why it matters |
|---|---|
| `deb.depends` is `[]` | The current config would emit a deb with no `Depends` |
| No `.deb` has been built | Paths and the kebab package name are from the CLI source, not from an artifact |
| Only a debug binary was linked | Release `NEEDED` can differ. The implementation re-checks |
| `Categories=` would be empty | Menu entry is harder to find. Set `Utility` and assert it |
| Desktop file name and deb file name contain a space | Product name `Kovanica Node`. Scripts must quote them |
| Placeholder icons | Honest for this milestone. Not a brand pass |
| No copyright file and no repo `LICENSE` | Crate license is GPL-3.0-or-later. Do not add the text until the maintainer line is decided |
| `patchelf` and three other CI `-dev` packages are absent on the Debian 12 box that linked the debug binary | The bundle command is not known to succeed here until they are installed |
| CI is Ubuntu 24.04 | Cannot be the Debian 12 build host |
| The app does not persist paths | Correct for this milestone, and easy to mistake for a broken install. The first-run note has to say so |
| Xvfb evidence is not a hardware session | Earlier README runs stay labeled that way |

## Alternatives not taken

- **AppImage or a tar "for Linux".** The supervisor is dynamically linked to WebKitGTK 4.1 and glibc 2.36. An archive without those libraries is not portable. Vendoring WebKit was not tested.
- **Build the deb on `ubuntu-24.04` because CI is already there.** That would support a different libc and WebKit than the ones measured, or it would mislabel an Ubuntu binary as Debian 12.
- **Bundle or download `kovanica-node`.** Out of scope, and it would hide the explicit path Check setup requires.
- **File dialog, PATH search, or unset `KOVANICA_*` from the app.** Each one weakens the current gate.
- **`prerm` that stops the node by name.** Can kill an unrelated process. The owned-child stop already exists inside the running supervisor.
- **Upload the deb as a release or as a CI artifact.** Not approved. Checksums in the log are the record.
- **Reuse `console-desktop` bundling.** That config targets `deb` and `appimage`, still describes the old monorepo layout, and is not this supervisor.

## Implementation checklist

Do this only after the decisions below are approved. One implementation PR. No release.

1. Branch from current `main`. Do not edit L1. Do not touch Wallet, Android, Windows, Comms, or `console-desktop`.
2. In `node-desktop/src-tauri/tauri.conf.json`, replace `deb.depends: []` with the explicit list. Set `bundle.category` to `Utility`. Set `bundle.shortDescription` to a single sentence that this is a local supervisor for one external `kovanica-node` and a read-only loopback monitor. Do not add resources, sidecars, file associations, or maintainer scripts.
3. Add the first-run paragraph to the setup note and to `node-desktop/README.md`. Do not change `check_setup`, Start, or capabilities.
4. Leave the placeholder icons. Leave `productName` as `Kovanica Node` unless the filename decision changes.
5. On Debian 12 amd64, run every command in the build section. Then run the headless acceptance table. Re-read release `NEEDED` and adjust `Depends` only if the release binary differs.
6. Add the Debian 12 CI job as specified. Do not weaken the Ubuntu test job and do not upload the deb.
7. Gates that PR must run, and must be green before review:
   - `npm test`
   - `npm run lint`
   - `npm run build`
   - `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
   - `cargo test --locked --lib --no-default-features`
   - `cargo clippy --locked --lib --no-default-features -- -D warnings`
   - `cargo clippy --locked --features gui --bins --tests -- -D warnings`
   - `cargo build --locked --features gui --bins`
   - `cargo test --locked --features gui --test reaper_cli`
   - the headless deb checks (`dpkg-deb`, desktop file, `objdump -p`, `sha256sum`)
   - optional Xvfb package smoke, recorded as Xvfb, separate from the headless result
8. Do not commit `target/`, `dist/`, the deb, or recovery artifacts. Do not commit `Cargo.lock` unless a command actually changed it. The plan does not expect a lockfile change.
9. Open the PR. Do not merge it from the implementation agent unless Toni asks. Do not tag a release.

## Decisions needed

1. Debian 12 amd64 `.deb` only, as specified. Reject if the first target should be Ubuntu 24.04 instead. Do not ask for both in the first PR.
2. The explicit `Depends` list, with glibc `>= 2.36` and every other package unversioned.
3. Keep the spaced product name (`Kovanica Node.desktop`, `Kovanica Node_0.2.0_amd64.deb`) so the menu name stays `Kovanica Node`. The alternative is a different `productName` and a custom desktop template. Not recommended here.
4. Maintainer line. Cargo.toml authors is `Kovanica Team`, which is not an email. The implementation must not invent one. Confirm the control-file maintainer before a deb is handed to anyone else. Candidate already used as the git author of this repo: `Kovanica Team <KovanicaDAG@protonmail.com>`. Leave it unset in the config until you say that string is acceptable.
5. Placeholder icons stay for this package.
6. CI prints checksums and discards the deb. No release, no signing, no artifact upload.
7. No maintainer scripts, so upgrade and remove never delete user files and never signal a process.

## What this plan checked

Checked while writing this file:

- `origin/main` is `c09fe0d51d6b1479f19aacaef355653d7dfaf1c3`. Working tree was clean before the branch.
- Main CI run 42396544477 is success.
- `package.json` scripts `test`, `lint`, `build`, `tauri`, and `recovery` match the commands above.
- `npx tauri build --help` on CLI 2.12.4 accepts `--bundles deb`, `--ci`, `-f`, and passes args after `--` to cargo.
- `tauri.conf.json` targets `deb` only, `depends` is `[]`, version `0.2.0`, icons on disk.
- Debian 12 amd64 package versions and the debug binary's `NEEDED` list, as tabulated.
- Tauri CLI 2.12.4 bundler source for deb paths, omitted `Depends` when the list is empty, and the desktop template. No deb was produced, so those paths are not an artifact listing.

Not run: `npm test`, `npm run lint`, `npm run build`, cargo fmt/clippy/tests, `tauri build`, `dpkg-deb`, apt install, Xvfb, `npm run recovery`. The cargo and npm gates are the green main CI, not a new local run. There is no markdown link checker in this repo.

- npm test / lint / build and cargo gates were not re-run locally; existing CI on this PR is the check
- tauri build was not run (no deb)
