# Android release builds

`android-wallet` and `android-comms` are built by `.github/workflows/android-release.yml`.

The workflow has two jobs:

| Job | Trigger | Output |
|---|---|---|
| `verify` | pull requests, pushes to `main` | unsigned debug APK (artifact `wallet-debug-apk` / `comms-debug-apk`) |
| `release` | `workflow_dispatch`, tags matching `android-v*` | signed release APK + AAB (artifacts `wallet-release` / `comms-release`) |

The `release` job uploads workflow artifacts only. It does **not** create or modify
GitHub Releases, and it does not upload to Play. Publishing is a separate, explicit step.

## Signing material

Signing material is never committed. `android-wallet/app/build.gradle.kts` and
`android-comms/app/build.gradle.kts` read it from the environment at build time:

| Variable | Secret | Meaning |
|---|---|---|
| `KEYSTORE_FILE` | `ANDROID_KEYSTORE_BASE64` | path to the keystore; CI writes the decoded secret to `$RUNNER_TEMP/release-signing.jks` |
| `KEYSTORE_PASSWORD` | `KEYSTORE_PASSWORD` | keystore password |
| `KEY_ALIAS` | `KEY_ALIAS` | key alias |
| `KEY_PASSWORD` | `KEY_PASSWORD` | key password |

If any of the four values is missing the release build type stays **unsigned** so
debug work and pull-request builds still succeed. The `release` job refuses to run
without all four secrets.

Store the actual keystore and passwords outside the repository (for example in an
operator password manager and an off-repo secrets directory). To load a keystore
into CI, base64-encode it and save the result as the `ANDROID_KEYSTORE_BASE64`
repository secret:

```sh
base64 -w0 /path/outside/repo/release-signing.jks > /tmp/release-signing.b64
gh secret set ANDROID_KEYSTORE_BASE64 --repo KovanicaDAG/kovanica-apps < /tmp/release-signing.b64
rm -f /tmp/release-signing.b64
```

Rotating the key means re-issuing all four secrets and re-publishing builds; users
must uninstall the previous certificate's build before installing the new one.

## Artifact names

| App | APK | AAB |
|---|---|---|
| wallet | `Kovanica-Wallet-<versionName>-arm64-v8a.apk` | `Kovanica-Wallet-<versionName>.aab` |
| comms | `kovanica-comms-<versionName>.apk` | `kovanica-comms-<versionName>.aab` |

`<versionName>` comes from `app/build.gradle.kts` (`versionName = "…"`). Bump
`versionCode` and `versionName` in that file before cutting a release tag.

## Native library

The wallet APK packages `libkovanica_light_ffi.so` for `arm64-v8a`, built from
`KovanicaDAG/kovanica` crate `kovanica-light-ffi` with 16 KB page alignment.
`android-wallet`'s `checkNativeSymbols` task fails the build if the packaged library
is missing symbols that the UniFFI bindings register at launch. Comms does not
package a native library.

## Local signed build

```sh
cd android-wallet
KEYSTORE_FILE=/path/outside/repo/release-signing.jks \
KEYSTORE_PASSWORD=… KEY_ALIAS=… KEY_PASSWORD=… \
./gradlew :app:assembleRelease
```

Without those variables the same command produces an unsigned release APK, which is
what local debugging uses.
