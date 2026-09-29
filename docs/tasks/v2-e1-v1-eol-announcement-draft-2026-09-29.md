---
title: v2 リリース E1 — v1 保守終了の告知文 下書き（README・更新通知・Scoop・既知の問題）
status: 下書き（2026-09-29）。実ファイル（README・docs/*.html・worker・Scoop）は未変更。「要確認」の決定待ち
created: 2026-09-29
related_adr: ["ADR-205", "ADR-206", "ADR-207"]
related_bugs: ["BUG-110", "BUG-142", "BUG-152", "BUG-163", "BUG-168", "BUG-169", "BUG-171", "BUG-172", "BUG-173"]
---

# v2 リリース E1: v1 保守終了の告知文 下書き

[v2 リリースチェックリスト](v2-release-checklist-2026-09-29.md) E1 の成果物。所有者決定（2026-09-29）: v2 リリースで v1 を保守終了にする。BUG-173 は v1 へ backport せず、v2 への移行を案内する。素材は [v1 backport 棚卸し](v2-e2-v1-backport-inventory-2026-09-29.md)。

書式の約束: 事実はコード・docs から取った。決まっていない点は本文中では `【要確認】` と書き、末尾「要確認一覧」に集約した（日付・バージョン呼称・Scoop 方針は仮置きにせず空欄）。版数は本文では「v2」と書く（正式呼称は未確定。チェックリストは v2.0.0 としている）。

## 1. README に載せる節の案

差し込み先: `README.md` / `README.en.md` の「動作環境」の直前（冒頭の特徴の後）。見出しは `## v1 の保守終了と v2 への移行`。下の文言をそのまま使える。

### 日本語（README.md）

```markdown
## v1 の保守終了と v2 への移行

awase v2 のリリースに伴い、**v1 系（1.x）は保守を終了します**（保守終了日: 【要確認】）。
以後、v1 には不具合修正も新機能も入りません。v1 の最後のバージョンは 1.21.1 です。

- **v2 への更新をおすすめします。** ダウンロードは [GitHub Releases](https://github.com/cuzic/awase/releases) から。
- v1 で直らない既知の問題は [v1 に残る既知の問題](#v1-に残る既知の問題) にまとめています。
- 設定ファイル（`config.toml`）はそのまま引き継げますが、一部の設定は v2 で変わります。
  [v1 から v2 への移行で設定が変わる点](#v1-から-v2-への移行で設定が変わる点) を確認してください。

### v2 で良くなった点（確認できたものだけ）

- Windows Terminal など TSF ネイティブなアプリ + Google 日本語入力で、カタカナのまま戻れなくなる不具合（BUG-173）を修正しました。
- Chrome + Google 日本語入力で、高速打鍵時に入力中の文字が消える不具合（BUG-168）を修正しました。
- Chrome + Google 日本語入力で、他のアプリなど外部から IME を閉じられたあと、NICOLA 入力に戻らない不具合（BUG-172）を修正しました（Google 日本語入力 + Chrome 等に限ります。MS-IME は対象外）。
- `config.toml` のキー名の書き方がどの設定項目でも同じ規則で読まれるようになり、間違った設定は黙って無視されず警告が出ます。
- 無変換／変換の単独タップの動作を整理しました（下記の移行の節を参照）。

> 上の項目のうち実機で確認できていないものは、各 BUG／ADR に「実機未検証」と記載があります。

### v1 から v2 への移行で設定が変わる点

`config.toml` は引き継げます。次の設定は v2 で扱いが変わります。

- **`keys.ime_toggle` の既定値が空になりました。** 以前の既定は `VK_KANJI`（半角/全角）でした。`config.toml` に `VK_KANJI` を明示している場合は、その設定を尊重します（消しません）。
- **`keys.ime_detect.on` / `off` の既定値が空になりました**（以前は `IMEオン` / `IMEオフ`）。IMEオン／IMEオフキーへの追随は指定しなくても自動で行われます。書いてある値はそのまま使われます。
- **`keys.engine_on_ime_key` / `engine_off_ime_key` を撤去しました。** 2026-08-15 より前に設定画面で保存した人は、旧既定値が `config.toml` に残っていて、エンジンの ON/OFF に合わせて全角/半角モードキーを送る機能が有効だった場合があります。v2 ではその機能が止まります（`config.toml` に残っていても無視され、起動時に通知が出ます。設定画面で保存するとその行は消えます）。代わりの設定はありません。IME の開閉は `keys.ime_on` / `ime_off` で設定できます。
- **`muhenkan_solo_tap_ime_action` / `henkan_solo_tap_ime_action` を撤去しました。** 読み込み時に `keys.ime_*` 相当へ移行し、警告が出ます（同じキーの設定が既にあれば移行しません）。無変換／変換の単独タップは、Suppress／Passthrough の設定に従います。
- **確定モード `confirm_mode` は `wait` と `ngram_predictive` の 2 択になりました。** 旧い値は読込時に `wait` として扱われ、保存時に `wait` へ書き換わります。
- **`config.toml` のキー名の解釈が統一されました。** これまで黙って無視されていた書き方（`VK_` 無し、小文字、日本語名など）が有効になります。心当たりのある再割り当てが、更新しただけで効き始めることがあります。
```

（補足: 「動作環境」節には Rust のビルド要件が書かれているだけで v2 の記述は無い。`README.md` にはバージョンごとの節も現状無い。）

### English (README.en.md)

```markdown
## End of maintenance for v1, and moving to v2

With the release of awase v2, **the v1 line (1.x) is no longer maintained** (end-of-maintenance date: TBD).
v1 will receive no further bug fixes or features. The last v1 release is 1.21.1.

- **We recommend upgrading to v2.** Download it from [GitHub Releases](https://github.com/cuzic/awase/releases).
- Known problems that will not be fixed in v1 are listed in [Known issues remaining in v1](#known-issues-remaining-in-v1).
- Your `config.toml` carries over, but a few settings change in v2. See
  [Settings that change when moving from v1 to v2](#settings-that-change-when-moving-from-v1-to-v2).

### What is better in v2 (only what we could verify)

- Fixed: with Google Japanese Input in TSF-native apps such as Windows Terminal, the input could get stuck in katakana with no way back (BUG-173).
- Fixed: with Google Japanese Input in Chrome, fast typing could erase the composition in progress (BUG-168).
- Fixed: with Google Japanese Input in Chrome, after an external program closed the IME, NICOLA input did not resume (BUG-172). This covers Google Japanese Input in Chrome-like windows only, not MS-IME.
- Key names in `config.toml` are now read by one common rule for every setting, and mistakes produce a warning instead of being silently ignored.
- The single-tap behavior of Muhenkan/Henkan was reorganized (see the migration section).

> Items not yet checked on a real machine are marked "not verified on a real device" in their BUG/ADR entries.

### Settings that change when moving from v1 to v2

Your `config.toml` carries over. These settings behave differently in v2.

- **The default of `keys.ime_toggle` is now empty.** It used to be `VK_KANJI` (Hankaku/Zenkaku). If your `config.toml` explicitly contains `VK_KANJI`, it is respected and kept.
- **The defaults of `keys.ime_detect.on` / `off` are now empty** (they used to be `IMEオン` / `IMEオフ`). Following the IME On/Off keys works automatically without them. Values you wrote are used as before.
- **`keys.engine_on_ime_key` / `engine_off_ime_key` were removed.** If you saved settings in the settings app before 2026-08-15, the old defaults may remain in your `config.toml` and the feature that sends a full-width/half-width mode key when the engine turns on/off may have been active. In v2 it stops (leftover values are ignored and a notice appears at startup; saving from the settings app removes the lines). There is no replacement. Use `keys.ime_on` / `ime_off` to open/close the IME.
- **`muhenkan_solo_tap_ime_action` / `henkan_solo_tap_ime_action` were removed.** They are migrated to the equivalent `keys.ime_*` entries when loaded, with a warning (not migrated if the same key already has an entry). A single tap of Muhenkan/Henkan now follows the Suppress/Passthrough setting.
- **`confirm_mode` now has two choices: `wait` and `ngram_predictive`.** Old values are treated as `wait` on load and rewritten to `wait` on save.
- **Key-name parsing is now uniform.** Spellings that were silently ignored before (no `VK_`, lowercase, Japanese names, ...) now take effect, so a remap you forgot about may start working after the update.
```

CHANGELOG との整合: 上の移行項目は、本 PR で追記した `CHANGELOG.md` の Unreleased（ime_toggle 既定・単独タップ再設計と `*_solo_tap_ime_action` 撤去・`confirm_mode` 2 択・`ime_detect` 既定空・`engine_on/off_ime_key` 撤去・キー名規則）と突き合わせ済み。

## 2. アプリ内更新通知

### 現行の仕組み（コードから）

| 項目 | 内容 | 根拠 |
|---|---|---|
| 問い合わせ | 設定アプリ側が `https://report.awase.cc/v1/latest-release?current_version=<自バージョン>` を GET | `crates/awase-settings/src/update_check.rs` |
| 応答の中身 | `schema_version`(=1) と `latest_version` の 2 つだけ。**文言・URL・お知らせ本文を返す欄は無い** | 同 `fetch_latest_version`、`services/report-worker/src/index.ts` の `latestReleaseResponse` |
| 保存 | `latest_version` を SemVer として解釈し `update_check.json` の `last_seen_latest` に保存 | `src/update_state.rs` |
| 表示の判定 | `last_seen_latest` が自分より大きい場合だけ `Display::Available`。等しいか小さければ「更新は見つかりませんでした」 | `update_state::display` |
| 表示される文言 | トレイ右クリックメニュー「新しいバージョン {version} があります...」、およびダイアログ「新しいバージョン {version} があります（最終確認: 約N分前）」。**文言は exe に埋め込みで固定**。クリックすると `https://github.com/cuzic/awase/releases/tag/v{version}` を開く | `crates/awase-windows/src/tray.rs`（メニュー約 689 行、ダイアログ約 900 行）、`src/version.rs::release_url` |
| ライン判定 | worker は `current_version` が `1.90.0` 以上なら v2 ライン、未満なら v1 ラインとし、`/releases` 一覧から自ラインの最大バージョンだけを返す。`current_version` が無い・読めない旧クライアントには従来どおり全体の latest を返す | `services/report-worker/src/index.ts`（`V2_LINE_MIN_VERSION`、`0a38590a`） |
| 自バージョンを送るクライアント | **v1.21.1 以降**（`6b47d1c8` は v1.21.1 のタグに含まれる）。v1.21.0 以前は送らない | `git show v1.21.0:` / `v1.21.1:crates/awase-settings/src/update_check.rs` |

### v1 ユーザーに出せる範囲と決定

v1 のバイナリは変えられず、通知の文言も変えられない（常に「新しいバージョン {version} があります」）。

| v1 のバージョン | v2 リリース後に今の worker が返すもの | v1 ユーザーに見えるもの |
|---|---|---|
| 1.21.1（`current_version` を送る） | v1 ライン内の最大 = 1.21.1 | 何も通知されない |
| 1.21.0 以前（送らない） | 全体の最新 = v2 のバージョン | 「新しいバージョン {v2} があります」が出て、v2 のリリースページが開く |

**決定済み（所有者、2026-09-29）: v1.21.1 の利用者には v2 を通知しない。report worker は変更しない。** v1.21.1 の利用者への案内は README と GitHub Release 本文に頼る。v1.21.0 以前の利用者には今の仕組みのまま v2 が通知される。

採らなかった案: v1 ラインの問い合わせにも v2 の最新を返すよう worker を変える（案 A）。

### GitHub Release 本文に載せる文言の案（v1.21.0 以前は通知のクリック先にもなる）

日本語:

```text
awase v2 へようこそ。v1 系（1.x）は保守を終了しました（最後の版は 1.21.1）。
v1 で直らない既知の問題は README の「v1 に残る既知の問題」を、設定が変わる点は「v1 から v2 への移行で設定が変わる点」を確認してください。
設定ファイル（config.toml）は引き継げます。
```

English:

```text
Welcome to awase v2. The v1 line (1.x) is no longer maintained (the last release is 1.21.1).
See "Known issues remaining in v1" and "Settings that change when moving from v1 to v2" in the README.
Your config.toml carries over.
```

v2 側（新しいバイナリ）の通知は、v2 のバイナリが自バージョンを送るので、v2 ラインの最新だけが返り、v1 の話は一切出ない。

## 3. Scoop

### 現状（コード・docs から確認できること）

- Scoop バケットは別リポジトリ `cuzic/scoop-awase`。`.github/workflows/release.yml` の「Update Scoop manifest」が、**リリースのタグを push するたびに** `bucket/awase.json`（バージョン・URL・ハッシュ）を上書きして push する（`SCOOP_BUCKET_TOKEN` が無ければ何もしない）。バケットにはラインの区別が無く、常に「最後にタグを push した版」が latest になる。
- `docs/index.html` / `docs/index.en.html` のインストール節では、Scoop の手順は「Scoop 未対応のため非表示」のコメントアウトのまま（機能一覧には「Scoop でのワンコマンドインストールに対応」の行が残っている）。README には Scoop の記述が無い。実際に Scoop 経由の利用者がどれだけいるかは、このリポジトリからは分からない。
- `.claude/skills/release-v1develop-to-v1main/SKILL.md`「v1固有の注意点」3: v2 リリース後に v1 のタグを push すると Scoop の latest が巻き戻る、Scoop／更新通知はライン識別を持たない設計、と警告している。**本決定（v1 のタグを push しない＝v1 パッチを出さない）が前提なら、この巻き戻りは起きない。**

### 方針（決定済み・所有者、2026-09-29）

1. `scoop-awase` は v2 を latest にする。v1 のタグ・GitHub Release は今後 push しない（前提）。バケットの `awase.json` はタグ push で自動更新されるため、`scoop update awase` で v2 に上がる。`persist` に `config.toml` `layout` `data` などが入っているので設定は引き継がれる。
2. **v1 用バケット（`awase-v1`）は残さない。** v1 に戻りたい人は GitHub Releases の v1.21.1（zip／MSI）を手で取得する（保守なしの旨を明記）。
3. 告知文（`scoop-awase` の README／`awase.json` の `description`）の案:
   - 日本語: `awase v2 をインストールします。v1 系は保守を終了しました（最後の版 1.21.1。戻したい場合は GitHub Releases から取得）。設定は引き継がれます。v1 との違いは https://github.com/cuzic/awase の README を参照。`
   - English: `Installs awase v2. The v1 line (1.x) is no longer maintained (last release 1.21.1; get it from GitHub Releases if you need it). Your settings are kept. See the README at https://github.com/cuzic/awase for the differences from v1.`
4. Scoop の `persist` に v2 で新しく増えるファイルがあるかは未確認（現行は `config.toml` `layout` `data` `keymap-learn-table.json` `keymap-learn-last-attempt.json`）。

## 4. v1 に残る既知の問題（ユーザー向け）

出典: [v1 backport 棚卸し](v2-e2-v1-backport-inventory-2026-09-29.md)「E1 に載せる『v1 に残る既知の問題』」の 10 件。内部の BUG 番号は末尾のリンクにとどめた。「v2 での状況」は同棚卸しと各 BUG／ADR の記載どおり（勝手に補っていない）。

| # | 起きること（平易な説明） | どんなとき | 回避策 | v2 での状況 | 詳細 |
|---|---|---|---|---|---|
| 1 | ひらがなに戻れず、カタカナで入力され続ける | Google 日本語入力 + Windows Terminal など（TSF ネイティブ）で、いったんカタカナになったあと | 半角/全角キーで IME を切り替え直す | 修正済み | [BUG-173](../known-bugs/BUG-173.md) |
| 2 | 入力中の（未確定の）文字が消える | Chrome・Edge 系 + Google 日本語入力で、しばらく使っていない状態から打つとき（途中の語）や、超高速で打ったとき | 打ち直す。消えやすい場面では一度確定してから続ける | 超高速打鍵での消失は修正済み（[BUG-168](../known-bugs/BUG-168.md)）。途中の語での消失は v2 でも未修正（[BUG-171](../known-bugs/BUG-171.md)） | BUG-168 / BUG-171 |
| 3 | 他のプログラムが IME を閉じたあと、Chrome で NICOLA 入力に戻らずローマ字がそのまま入る | 外部から IME を閉じられたとき（実 Chrome） | 物理キー（半角/全角など）で切り替える | Google 日本語入力 + Chrome 等では修正済み。MS-IME などは対象外 | [BUG-172](../known-bugs/BUG-172.md) |
| 4 | 無変換・変換・ひらがな・英数キーで IME の状態を変えても、awase の入力モードがついてこないことがある | Engine が ON のままなど | 親指シフトの入力／ローマ字入力の切替（トレイ・ホットキー）で合わせる | v2 で追随の仕組みが入っている（v1 には無い）。個別の BUG は棚卸し参照 | 棚卸し「v1 に関係しない」節 |
| 5 | MS-IME（本体）で最初のひらがなキーが効かず、IME が閉じることがある | IME への切り替え要求がタイムアウトした最初の 1 回だけ | もう一度ひらがなキーを押す | 修正済み | [BUG-152](../known-bugs/BUG-152.md) |
| 6 | 起動直後、IME を閉じても何度か ON に戻される | IME を閉じた状態で awase を起動した直後（過渡的） | 起動後しばらくしてから切り替える | 実装済み・実機未検証 | [BUG-163](../known-bugs/BUG-163.md) |
| 7 | 半角/全角キーを連打すると IME が ON に固まる | Windows Terminal + Google 日本語入力 | `keys.ime_detect.toggle` を設定する | 恒久修正は未（設定の既定値変更待ち） | [BUG-142](../known-bugs/BUG-142.md) |
| 8 | NICOLA 変換が効かなくなり、ウィンドウを切り替えるまで戻らない | IME キーの検出が低い確度になったとき（頻度不明） | 別のウィンドウへ移って戻る | 根本修正は未着手 | [BUG-110](../known-bugs/BUG-110.md) |
| 9 | 設定画面で n-gram ファイル欄を空にしても既定の値に戻ってしまい、無効にできない | 設定画面 | `config.toml` で直接設定する | v2 でも未修正 | [BUG-169](../known-bugs/BUG-169.md) |
| 10 | v1.21.1 のアプリ内更新通知では v2 が通知されない | v2 リリース後。Scoop は常に最新（v2）になる | README・GitHub Releases の告知を見る | — | 本書 2・3 節 |

リンク先の BUG-110・142・152・163・168・169・171・172・173 のファイルは `docs/known-bugs/` に存在することを確認した。「v2 での状況」の列は棚卸し（2026-09-29）時点の記述で、BUG 個別ファイルの最新状態との突き合わせは未了。

## 要確認一覧

1. v2 の正式なリリース日、および v1 の保守終了日（README 冒頭・GitHub Release に入れる日付）。
2. v2 の呼称。チェックリストは「v2.0.0」。本文は「v2」と書いた。`awase.cc`／Scoop の文言も合わせる。
3. 「v2 で良くなった点」の 3 件目（BUG-172）と、BUG-163 の状況（実装済み・実機未検証）の書き方: 実機未検証をどこまで README に書くか。
4. 本書 4 節の各「v2 での状況」を、リリース時点の BUG 個別ファイルと突き合わせて最新化する（棚卸し作成時点の記述）。
5. README の差し込み位置（「動作環境」直前を仮置き）と、英語版のアンカー名。
6. docs（`docs/index.html`／`index.en.html`）に同内容を載せるか。載せる場合の Scoop の記述（現状は非表示コメントと機能一覧の行が食い違っている）。
7. v1 の最後のパッチを出すか（チェックリスト E1 の記述）。本書は「出さない（タグを push しない）」を前提にした。出す場合は Scoop の巻き戻りに関する SKILL.md の警告が現実になる。

## 決定済み（所有者、2026-09-29）

- 更新通知: v1.21.1 の利用者には v2 を通知しない。report worker は変更しない。案内は README と GitHub Release 本文。
- Scoop: `scoop-awase` を v2 の latest にし、v1 用バケット `awase-v1` は残さない（v1 は GitHub Release から取得）。
