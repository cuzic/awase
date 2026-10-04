# awase — 親指シフト（NICOLA）キーボードリマッパー

*[English](README.en.md)*

**awase**（合わせ）は、Windows で親指シフト入力を実現するキーボードリマッパーです。

---

## 親指シフトとは

親指シフト（NICOLA 配列）は、スペースバー両隣の「変換」「無変換」キーを親指シフトキーとして使い、文字キーと同時押しすることでかな文字を直接入力する方式です。ローマ字入力より少ないキー操作で日本語を入力でき、習得後は高速・高効率なタイピングが可能です。

awase は低レベルキーボードフックで物理キー入力を横取りし、同時打鍵を検出して IME にローマ字として送信します。IME は通常どおり漢字変換します。

---

## 特徴

- **NICOLA 準拠の同時打鍵判定** — d1/d2 比較による 3 キー仲裁
- **2 つの確定モード** — wait / ngram\_predictive
- **n-gram 適応閾値** — Wikipedia コーパス由来の 2/3-gram で判定ウィンドウを動的調整し精度向上
- **やまぶき互換 `.yab` 配列ファイル** — 既存の配列データをそのまま利用可能
- **幅広いアプリ対応** — Win32 / UWP / TSF ネイティブ（Chrome・VS Code・WezTerm 等）を自動識別
- **多重耐障害設計** — フック死活監視・スリープ復帰・IME 検出失敗フォールバック・TSF コールドスタート自動回復を多段装備
- **非同期アーキテクチャ** — Windows メッセージループベースの非同期エグゼキュータ、ブロッキング API は別スレッドで隔離しタイムアウト保護
- **フォーカス自動検出** — テキスト入力欄以外では変換を自動停止
- **システムトレイ常駐** — 配列切替・設定画面・親指シフト入力／ローマ字入力トグル
- **US配列対応** — `keyboard_model = "us"` で US 物理配列に切替。無変換/変換キーが無いぶん、左右 Alt キーへの親指キーなりすまし・Space 親指キー化にも対応

技術的な設計の詳細は [ARCHITECTURE.md](ARCHITECTURE.md) を参照してください。

---

## v1 の保守終了と v2 への移行

awase v2.0.0 のリリース（2026-10-03）に伴い、**v1 系（1.x）は保守を終了しました**（保守終了日: v2.0.0 の公開日 2026-10-03）。
以後、v1 には不具合修正も新機能も入りません。v1 の最後のバージョンは 1.21.1 です。

- **v2 への更新をおすすめします。** ダウンロードは [GitHub Releases](https://github.com/cuzic/awase/releases) から。v1 と v2 の違い全体は [v1 から v2 への違いと移行ガイド](docs/migration-v1-to-v2.md) にあります。
- v1 で直らない既知の問題は [v1 に残る既知の問題](#v1-に残る既知の問題) にまとめています。
- 設定ファイル（`config.toml`）はそのまま引き継げますが、一部の設定は v2 で変わります。
  [v1 から v2 への移行で設定が変わる点](#v1-から-v2-への移行で設定が変わる点) を確認してください。
- v1.21.1 のアプリ内更新通知では v2 は通知されません。このページか GitHub Releases で確認してください。

### v2 で修正した主な不具合

実機（物理キー・実アプリ）で確認できていないものは、その旨を書いています。詳細は各 BUG の記録（`docs/known-bugs/`）にあります。

- Windows Terminal など TSF ネイティブなアプリ + Google 日本語入力で、カタカナのまま戻れなくなる不具合（BUG-173）を修正しました。実機での最終確認は未了です。
- Chrome + Google 日本語入力で、高速打鍵時に入力中の文字が消える不具合（BUG-168）を修正しました。CI・実機での確認待ちです。
- Chrome 系ウィンドウ + Google 日本語入力で、他のプログラムが注入したキーで IME を閉じられたあと、NICOLA 入力が ON のままローマ字が入る不具合（BUG-172）を修正しました。CI（GitHub Actions の Windows 実機）で確認済みで、実機の Edge でも効果を確認しましたが、実機で「IME が開いたままなのに NICOLA が OFF になる」現象を 1 回見ており原因は未特定です（BUG-176）。MS-IME は対象外です。
- 起動直後に、IME を閉じていても awase が開けに行く不具合（BUG-163）を修正しました（実機未検証）。
- `config.toml` のキー名の書き方がどの設定項目でも同じ規則で読まれるようになり、間違った設定は黙って無視されず警告が出ます。
- 無変換／変換の単独タップの動作を整理しました（下記の移行の節を参照。Windows Terminal + Google 日本語入力で「@」が出る件への効果は実機未確認）。

### v1 に残る既知の問題

v1.21.1 には次の問題が残り、v1 では直りません。

| 起きること | どんなとき | 回避策 | v2 での状況 |
|---|---|---|---|
| ひらがなに戻れず、カタカナで入力され続ける（BUG-173） | Google 日本語入力 + Windows Terminal など（TSF ネイティブ）で、いったんカタカナになったあと | 半角/全角キーで IME を切り替え直す | 修正（実機未確認） |
| 入力中の文字が超高速打鍵で消える（BUG-168） | Chrome・Edge 系 + Google 日本語入力 | 打ち直す | 修正（CI・実機確認待ち） |
| 途中の語で入力中の文字が消える（BUG-171） | Chrome・Edge 系 + Google 日本語入力で、しばらく使っていない状態から打つとき | 打ち直す。一度確定してから続ける | v2 でも未修正 |
| 他のプログラムが IME を閉じたあと、NICOLA 入力に戻らずローマ字がそのまま入る（BUG-172） | Chrome 系 + Google 日本語入力 | 物理キー（半角/全角など）で切り替える | 修正（MS-IME は対象外） |
| MS-IME 本体で最初のひらがなキーが効かず IME が閉じることがある（BUG-152） | IME への切り替え要求がタイムアウトした最初の 1 回だけ | もう一度ひらがなキーを押す | 修正 |
| 起動直後、IME を閉じても何度か ON に戻される（BUG-163） | IME を閉じた状態で awase を起動した直後 | 起動後しばらくしてから切り替える | 修正（実機未検証） |
| 半角/全角キーを連打すると IME が ON に固まる（BUG-142） | Windows Terminal + Google 日本語入力 | `keys.ime_detect.toggle` を設定する | 恒久修正は未 |
| MS-IME 本体 + Chrome で、入力中の文字が残っている間に OFF を押すと IME が閉じず半角英数になる（BUG-185） | MS-IME 本体 + Chrome 系 | ON キーでかなに戻る。Chrome では Google 日本語入力をおすすめします | v2 でも対応しない（MS-IME の挙動） |
| 設定画面で n-gram ファイル欄を空にしても既定値に戻り、無効にできない（BUG-169） | 設定画面 | `config.toml` で直接設定する | v2 でも未修正 |

### v1 から v2 への移行で設定が変わる点

`config.toml` は引き継げます。次の設定は v2 で扱いが変わります。

- **`keys.ime_toggle` の既定値が空になりました。** 以前の既定は `VK_KANJI`（半角/全角）でした。`config.toml` に `VK_KANJI` を明示している場合は、その設定を尊重します（消しません）。ただし v1 の設定画面が書き出した旧既定と同じ値（`["VK_KANJI"]`）は空として扱われ、保存すると消えます。
- **`keys.ime_detect.on` / `off` の既定値が空になりました**（以前は `IMEオン` / `IMEオフ`）。IMEオン／IMEオフキーへの追随は指定しなくても自動で行われます。書いてある値はそのまま使われます。
- **`keys.engine_on_ime_key` / `engine_off_ime_key` を撤去しました。** 2026-08-15 より前に設定画面で保存した人は、旧既定値が `config.toml` に残っていて、エンジンの ON/OFF に合わせて全角/半角モードキーを送る機能が有効だった場合があります。v2 ではその機能が止まります（`config.toml` に残っていても無視され、起動時に通知が出ます。設定画面で保存するとその行は消えます）。代わりの設定はありません。IME の開閉は `keys.ime_on` / `ime_off` で設定できます。
- **`muhenkan_solo_tap_ime_action` / `henkan_solo_tap_ime_action` を撤去しました。** 読み込み時に `keys.ime_*` 相当へ移行し、警告が出ます（同じキーの設定が既にあれば移行しません）。無変換／変換の単独タップは、Suppress／Passthrough の設定に従います。
- **確定モード `confirm_mode` は `wait` と `ngram_predictive` の 2 択になりました。** 旧い値は読込時に `wait` として扱われ、保存時に `wait` へ書き換わります。
- **`config.toml` のキー名の解釈が統一されました。** これまで黙って無視されていた書き方（`VK_` 無し、小文字、日本語名など）が有効になります。心当たりのある再割り当てが、更新しただけで効き始めることがあります。旧表記 `[[keymap]]`（正しくは `[[keymaps]]`）も効き始めます。

---

## 動作環境

- Windows 10 / 11（64 ビット）
- Google 日本語入力（推奨）または MS-IME
- Rust 1.85 以上（ビルド時のみ）

---

## クイックスタート

### 1. ビルド

```sh
cargo build --release --target x86_64-pc-windows-msvc
```

生成物: `target/x86_64-pc-windows-msvc/release/awase.exe`

### 2. ファイル配置

以下の構成で配置します。

```
awase.exe
config.toml          ← 設定ファイル
layout/
  nicola.yab         ← NICOLA 配列（Backspace/Escape 代用版）
  nicola_keytop.yab  ← NICOLA 配列（記号キートップ通り出力版、新規インストールの既定）
  nicola_kakutei.yab ← NICOLA 配列（記号キートップ通り出力版 + 句読点で確定）
  nicola_f.yab       ← 富士通純正親指シフトキーボード(FKB7628-801等)向け
  nicola_kb232.yab   ← 富士通純正親指シフトキーボード(FMV-KB232)向け
  nicola_us.yab      ← US 配列
data/
  ngram_hiragana.csv.gz  ← n-gram コーパス（任意）
```

### 3. 起動

`awase.exe` をダブルクリックするとシステムトレイに常駐します。

### 4. 親指シフト入力にする

デフォルトのキーバインド：

| 操作 | キー |
|------|------|
| 親指シフト入力にする | **Ctrl+Shift+変換** |
| ローマ字入力にする | **Ctrl+Shift+無変換** |
| IME ON | **Ctrl+変換**（IME が既に ON の場合はひらがな・ローマ字・CapsLock OFF へリセット） |
| IME OFF | **Ctrl+無変換** |
| IME-ON 半角英数トグル（MS-IME のみ） | **左Shift 単独タップ**（他キーを介さず押して離す。もう一度タップで解除） |
| アプリ別動作を手動切替 | **Ctrl+Shift+F11** |

> トレイアイコンを右クリック → 「設定」から GUI で変更できます。

### 5. 親指キーの確認

デフォルトは「無変換」が左親指、「変換」が右親指です。  
`config.toml` の `left_thumb_key` / `right_thumb_key` で変更できます。

---

## 設定ファイル (config.toml)

最小構成：

<!-- example-begin: readme-minimal asis -->
```toml
[general]
simultaneous_threshold_ms = 100   # 同時打鍵判定の閾値（ms）。NICOLA 規格は 100ms
left_thumb_key  = "無変換"
right_thumb_key = "変換"
layouts_dir     = "layout"
default_layout  = "nicola_keytop.yab"
```
<!-- example-end -->

フルサンプルは同梱の `config.toml` を参照してください。

### 主なオプション

| キー | デフォルト | 説明 |
|------|-----------|------|
| `simultaneous_threshold_ms` | 100 | 同時打鍵と判定する時間幅（ms） |
| `left_thumb_key` | `無変換` | 左親指シフトキー |
| `right_thumb_key` | `変換` | 右親指シフトキー |
| `confirm_mode` | `wait` | 確定モード（後述） |
| `engine_toggle_hotkey` | なし | 親指シフト入力／ローマ字入力トグルホットキー |
| `keyboard_model` | `jis` | 物理キーボード配列。US 配列なら `"us"`（`default_layout` も `nicola_us.yab` に変更） |

### 確定モード

| モード | 特徴 |
|--------|------|
| `wait` | タイムアウトまで待機。最も正確、わずかに遅延あり |
| `ngram_predictive` | Wikipedia 由来の n-gram 統計で閾値を動的調整（n-gram ファイル推奨） |

迷ったら `wait` から始め、遅延が気になったら `ngram_predictive` を試してください。

> 旧バージョンの `speculative` / `two_phase` / `adaptive_timing` は廃止されました。`config.toml` に残っていても読込時に警告を出し、`wait` として扱います。

n-gram の仕組みの詳細は [ARCHITECTURE.md](ARCHITECTURE.md#n-gram-による同時打鍵判定の精度向上) を参照してください。

### アプリ別設定 ([app_overrides])

特定アプリで動作が合わない場合に強制指定します。

<!-- example-begin: readme-app-overrides asis -->
```toml
[app_overrides]
# 常にテキスト入力として扱う
force_text = [
    { process = "myapp.exe", class = "Edit" },
]
# 常にローマ字入力にする（awase を素通しする）
force_bypass = [
    { process = "launcher.exe", class = "LauncherClass" },
]
# TSF ネイティブモード（WezTerm 等）
force_tsf = [
    { process = "wezterm-gui.exe", class = "org.wezfurlong.wezterm" },
]
```
<!-- example-end -->

プロセス名とクラス名は `RUST_LOG=debug awase.exe` のログで確認できます。

---

## 配列ファイル (.yab)

やまぶき互換の CSV 形式で配列を定義します。`layout/` に `.yab` ファイルを置くとトレイメニューから切り替えられます。

設定画面（`awase-settings.exe`）の「配列編集」タブから、テキストエディタで CSV を直接編集する代わりに、キーボード風グリッドをクリックしてビジュアルに編集・保存もできます。

```
; コメント行はセミコロンで始める
[ローマ字シフト無し]
'。',ka,ta,ko,sa, ra,ti,ku,tu,'，','、',無
u, si,te,ke,se, ha,to,ki, i, nn, 後, 逃
...

[ローマ字左親指シフト]
...

[ローマ字右親指シフト]
...
```

NICOLA 標準配列は JIS 配列として2種類同梱しています。両方ともかな44キー配置は NICOLA 本家仕様と同一で、本家仕様が未定義のまま余っている物理キー位置（数字段12-13列目・Q段11-12列目・A段〈ホームロー〉11-12列目）の扱いだけが異なります。

- `layout/nicola_keytop.yab`（**新規インストールの既定**）: 標準 JIS キーボードのキートップに実際に印字されている記号（＠／［／］／：／￥／＾）を出力します。ただし ＠ は物理 @ キーが NICOLA 本家仕様で「、」（読点）に割り当てられているため、無変換/変換キーを併用した親指シフト時のみ ＠ が出ます（単独タップは引き続き「、」）。
- `layout/nicola.yab`（v1.16.1 以前の既定）: 同じ物理キー位置をソフトウェアで Backspace/Escape に代用します。既存インストールをアップグレードしても `layout/nicola.yab` の中身は変わりません（ユーザーが配列編集タブで直接編集している可能性があるファイルを無言で上書きしない設計のため）。新しい記号キートップ通り出力に乗り換えたい場合は、`default_layout` を `"nicola_keytop.yab"` に手動で変更してください。

### 句読点で確定するレイアウト

`layout/nicola_kakutei.yab` は `nicola_keytop.yab` と同一の配列に、「。」「、」の
2セルだけ変更を加えたものです。この2キーを押すと、句読点を出力した直後に
Ctrl+M（IME の全確定ショートカット）を送るようになります。やまぶき／やまぶきR・
DvorakJ にある「句読点で確定」機能と同じ仕組みです。

設定画面の「レイアウト」で `nicola_kakutei.yab` を選ぶだけで使えます
（打鍵列機能は既定 On）。

使用する IME（Google 日本語入力 / MS-IME）側で Ctrl+M が「全確定」に
割り当てられていることを確認してください。他のアプリで Ctrl+M が別機能に
割り当てられている場合は競合します。

US 配列では `layout/nicola_us.yab` を使用します。無変換/変換キーが物理的に無いため、設定画面で左右 Alt キーを親指キーとしてなりすまさせる、または Space キーを親指キーに割り当てることができます。

富士通純正親指シフトキーボード（FKB7628-801 等）向けに `layout/nicola_f.yab` も同梱しています。物理キー配列・スキャンコードは JIS キーボードと共通のため `keyboard_model` は `"jis"` のままで構いません。`default_layout` を `"nicola_f.yab"` に変更してください。

富士通純正親指シフトキーボード「FMV-KB232」は `nicola_f.yab` とも記号配置が異なるため、専用の `layout/nicola_kb232.yab` を用意しています。`default_layout` を `"nicola_kb232.yab"` に変更してください（`keyboard_model` は `"jis"` のまま）。ユーザー1名の実機環境から提供された配列のため、他の個体・型番違いで記号配置が一致しない可能性があります。

---

## アプリ対応

awase はフォーカス中のアプリを自動識別し、出力方式を切り替えます。手動設定は不要です。

| アプリ種別 | 例 | 出力方式 |
|-----------|-----|---------|
| Win32 / WinForms | メモ帳、Word、Excel | Unicode 直接注入 |
| TSF ネイティブ | Chrome, Edge, VS Code, WezTerm, Electron 系 | VK キーストローク |
| UWP / XAML | Windows ストアアプリ | Unicode 直接注入 |

識別結果はアプリのクラス名ごとに学習・キャッシュされ（`cache.toml`）、再起動後も維持されます。自動識別が合わない場合は `[app_overrides]` で手動指定できます。

---

## トラブルシューティング

**文字が入力されない / おかしな文字になる**  
→ ローマ字入力になっている可能性。Ctrl+Shift+変換 で親指シフト入力にする。

**特定アプリで動作しない**  
→ `RUST_LOG=debug awase.exe` で起動してログを確認し、`[app_overrides]` に追加。

**IME が自動で ON/OFF される**  
→ `config.toml` の `[keys.ime_detect]` でシャドウ追跡キーを確認する。

**同時打鍵の誤判定が多い**  
→ `simultaneous_threshold_ms` を 80〜120ms の範囲で調整する。

**IME や FSM が壊れた状態になった**  
→ トレイアイコンを右クリック → 「内部状態をリセット」で全内部状態を初期化できます。

---

## ライセンス

[Apache License, Version 2.0](LICENSE-APACHE) または [MIT License](LICENSE-MIT) のいずれかを選択できます。
