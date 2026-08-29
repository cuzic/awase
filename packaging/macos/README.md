# awase on macOS — packaging and autostart

## Build the app bundle

```sh
./packaging/macos/make-app.sh
cp -R dist/Awase.app /Applications/
```

The bundle embeds `config.toml` and `layout/` under `Contents/Resources/`
(the binary also finds them next to the executable or in the current
directory, in that order of precedence — see `resolve_resource` in
`crates/awase-macos/src/main.rs`).

## Grant permissions (first launch)

1. Launch `/Applications/Awase.app` once; macOS shows the Accessibility prompt.
2. System Settings > Privacy & Security > **Accessibility** — enable Awase.
3. If the event tap still fails, also enable Awase under **Input Monitoring**.
4. Relaunch the app. An 「あ」 icon appears in the menu bar.

**Note on rebuilds:** the default ad-hoc signature changes on every rebuild,
so macOS silently drops the Accessibility grant — remove Awase from the list
and re-add it after installing a new build. For a stable identity, create a
self-signed code-signing certificate (Keychain Access > Certificate Assistant,
type "Code Signing", e.g. named `awase-codesign`) and build with:

```sh
CODESIGN_IDENTITY=awase-codesign ./packaging/macos/make-app.sh
```

For distributing binaries to others, sign with a Developer ID Application
certificate, enable the Hardened Runtime, and notarize — an app holding
Accessibility access should have a verifiable publisher (see
[TN3127](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements/)).

## Start at login

Pick **one** of the following:

- **Login Items** (simplest): System Settings > General > Login Items >
  add `Awase.app`.
- **LaunchAgent** (restarts on crash, logs to `~/Library/Logs/Awase/`):

  ```sh
  mkdir -p ~/Library/Logs/Awase && chmod 700 ~/Library/Logs/Awase
  sed "s|USERNAME|$USER|g" packaging/macos/com.github.cuzic.awase.plist \
    > ~/Library/LaunchAgents/com.github.cuzic.awase.plist
  launchctl load ~/Library/LaunchAgents/com.github.cuzic.awase.plist
  ```

  Unload with `launchctl unload ~/Library/LaunchAgents/com.github.cuzic.awase.plist`.

## Notes

- Quit from the menu bar icon (awase を終了) or `launchctl unload`.
- Thumb keys default to 英数 (left) / かな (right); the IME must be in
  romaji-input hiragana mode for kana-kanji conversion to work.
- Secure input fields (password boxes) bypass the event tap by OS design.
