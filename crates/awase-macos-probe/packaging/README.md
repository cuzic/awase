# awase-macos-probe — development `.app` packaging

This directory packages the `awase-macos-probe` diagnostic binary as a macOS
`.app` bundle with a **stable code identity**, and produces the artifacts a
human needs to verify that identity on real Apple hardware.

## Why this exists

macOS's TCC (privacy permission) database keys its grants — **Input
Monitoring**, **Accessibility**, and similar — off the *code identity* of the
signed program, not its path. The Probe needs those permissions to create a
`CGEventTap` and to observe/post events.

If the Probe is signed with bare ad-hoc signing (`codesign --sign -` with no
explicit identifier), the resulting code identity can change on **every
rebuild**. A human who granted Input Monitoring once would find the grant
silently revoked after the next `cargo build`, making TCC behavior impossible
to reason about during Phase M0.

So the distributable is a `.app` at
`…/awase-macos-probe.app/Contents/MacOS/awase-macos-probe` with:

- **Bundle Identifier:** `tools.awase.macos-probe`
- **Executable name:** `awase-macos-probe`
- **Code-signing identity:** a *stable, identical* identity across rebuilds.

This matches the design decision recorded in the project memory
(`macos_probe_interfaces`, the ".app identity" bullet) and the ChatGPT
technical review it summarizes.

## How to run (on macOS)

```sh
# Recommended: sign with a persistent local codesigning identity so TCC grants
# survive rebuilds. Create one once via Keychain Access
# ("Create a Certificate…", type: Code Signing), then pass its name:
./build_probe_app.sh --identity "My Dev Cert"

# Weaker fallback: ad-hoc signing with a fixed --identifier. More stable than
# bare ad-hoc, but TCC may still need re-granting occasionally.
./build_probe_app.sh
```

Options:

| Flag         | Default                | Allowed                                        |
| ------------ | ---------------------- | ---------------------------------------------- |
| `--target`   | `aarch64-apple-darwin` | `aarch64-apple-darwin`, `x86_64-apple-darwin`  |
| `--profile`  | `debug`                | `debug`, `release`                             |
| `--identity` | *(ad-hoc fallback)*    | any codesigning identity in your login keychain |

The script:

1. `cargo build -p awase-macos-probe --target <target> [--release]`
2. Assembles `target/<target>/<profile>/awase-macos-probe.app` with the binary
   at `Contents/MacOS/awase-macos-probe` and this dir's `Info.plist` at
   `Contents/Info.plist`.
3. Code-signs the bundle (local identity if `--identity`, else ad-hoc + fixed
   `--identifier tools.awase.macos-probe`).
4. Writes `codesign --display --verbose=4` output to `last_codesign_report.txt`
   (git-ignored) so you can **diff the code identity across rebuilds** and
   confirm it is stable — that stability is what keeps the TCC grant alive.
5. Prints the final `.app` path.

## Verifying identity stability

Build twice and compare the reports; the `CDHash`, `Identifier`, and (with a
real identity) `TeamIdentifier`/`Authority` lines should not change:

```sh
./build_probe_app.sh --identity "My Dev Cert"
cp last_codesign_report.txt /tmp/sign-1.txt
./build_probe_app.sh --identity "My Dev Cert"
diff /tmp/sign-1.txt last_codesign_report.txt   # expect no identity drift
```

## Not verifiable from Linux

This packaging **cannot be exercised on the Linux dev machine** — there is no
macOS, no `codesign`, and no Apple SDK. Only `bash -n` (syntax) and plist
well-formedness were checked here. A human must run `build_probe_app.sh` on
real macOS hardware and confirm code-identity stability **before** Task #4
(permissions) real-device verification — deferred to **project tracker task
#19** — is meaningful.

## Distribution (Phase M6, not yet actionable)

Everything above is **Phase M0** (local development identity: ad-hoc / local
signing, just enough for stable TCC grants while iterating). The two scripts
below are the **next tier up — public distribution** — and correspond to
**Phase M6** in the macOS port strategy (`docs/macos_port_strategy.md`;
M0 = this probe crate, M6 = distribution). They are written now because they
are cheap to scaffold, but they are **out of scope until real macOS hardware
plus Apple Developer Program credentials exist**.

Distribution requires:

- **Apple Developer Program** enrollment (~$99/year) to obtain a
  **"Developer ID Application"** signing certificate.
- A real Mac to run `codesign` / `notarytool` / `stapler` / `hdiutil`.

Neither is available in the current Linux dev environment, so — exactly like
`build_probe_app.sh` — both scripts have only been `bash -n` syntax-checked
here and **cannot be verified end-to-end** until Phase M6.

The pipeline:

```
build_probe_app.sh   ->   distribute.sh   ->   notarize.sh
(dev .app, M0)            (Developer ID          (notarized + stapled,
                           signed .zip/.dmg)      Gatekeeper-trusted)
```

### `distribute.sh`

Re-signs the release `.app` (from `build_probe_app.sh --profile release`) with
a **Developer ID Application** identity, enables the **hardened runtime**
(`codesign --options runtime --timestamp`, both required for notarization),
and packs it into a `.zip` (`ditto -c -k --keepParent`, Apple's
notarytool-safe zip) and optionally a `.dmg` (`hdiutil create`).

```sh
./distribute.sh --identity "Developer ID Application: Jane Dev (AB12CD34EF)" --dmg
```

| Flag         | Default                                                                 |
| ------------ | ---------------------------------------------------------------------- |
| `--identity` | *(required)* full `Developer ID Application: <name> (<TEAMID>)` cert   |
| `--app`      | `target/aarch64-apple-darwin/release/awase-macos-probe.app`           |
| `--out-dir`  | `packaging/dist`                                                        |
| `--dmg`      | also emit a signed `.dmg` (off by default)                              |
| `--version`  | `0.1.0` (keep in sync with `Cargo.toml` / `Info.plist`)                |

Ad-hoc / local dev identities are explicitly **rejected** — those are for
`build_probe_app.sh`, not distribution.

### `notarize.sh`

Wraps `xcrun notarytool submit … --wait` followed by `xcrun stapler staple`
on the `.zip`/`.dmg` from `distribute.sh`. **No secrets are hardcoded**; auth
is read from the environment. Two `notarytool` auth forms are wired (the one
to standardize on is left to whoever runs this on real hardware):

- **App Store Connect API key** (recommended for CI):
  `AC_API_KEY_ID`, `AC_API_ISSUER_ID`, `AC_API_KEY_PATH` (the `.p8` file).
- **Apple ID + app-specific password**:
  `AC_APPLE_ID`, `AC_TEAM_ID`, `AC_APP_PASSWORD` (an *app-specific* password
  from appleid.apple.com, never the account password).

```sh
export AC_API_KEY_ID=... AC_API_ISSUER_ID=... AC_API_KEY_PATH=/path/AuthKey_XXXX.p8
./notarize.sh dist/awase-macos-probe-0.1.0.dmg
```

Note: a `.zip` cannot be stapled directly (the ticket attaches to the `.app`
inside, fetched online at first launch); use `--dmg` for a directly
stapleable, offline-validatable artifact.
