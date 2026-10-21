---
id: ADR-276
title: |-
  Unicode 注入(既定の注入モードと Tsf モード内の eager かな送信)を撤去する(ADR-083 の申し送りを所有者判断で覆す)
summary: |-
  Win32/UWP の既定の注入モード `InjectionMode::Unicode` と、Tsf モード内の eager かな送信(`PendingGjiConfirm`)を撤去し、`force_tsf` に無い窓は Vk(IME composition 経由のローマ字)で送る。コードの削減が目的(約 800 行)。
  根拠: (1) 既定になった理由は歴史的で(初期の二分岐「Chrome は VK、他は Unicode」)、Unicode でなければならない積極的な理由の記録が無い(ADR-0004・006・023・083 を調べた)。(2) 実測(2026-10-21、GitHub の Windows ランナー、`sc-preedit-edit-gji`・`-msime`・`rich`): Unicode 注入されたかなは IME に入力として受け取られ、composition になる。つまり撤去しても漢字変換できる範囲は変わらない(ADR-0004 の「IME を経由しない」は IMM32/TSF の標準の入力欄では実際と違った)。(3) 所有者の判断: GJI が応答しない異常時はローマ字が残ってよい。
  残すもの: `.yab` の `\u`・クォートの直接指定と、表に無い文字(`－` など)の `KEYEVENTF_UNICODE`(BUG-066)、他アプリが注入した `VK_PACKET` の再注入(ADR-253)。
  A1・G1(GjiFsm への ImeOn 通知 2 か所)は、VK 注入だけの世界では G2 と二重であることをモデル検証(`tsf/gji_sync_model.rs`)で厳密に示した。Unicode 撤去と同時に外せる。
status: |-
  草案(2026-10-21)。所有者の方針は決定済み(「それでも Unicode 注入は撤去したい。コードを削減したいから」)。比較実験の一部(標準フォーム・LibreOffice は PASS)は反映済み。残りの入力先の結果と Opus レビューを反映して確定する。
related_adr:
  - "ADR-0004"
  - "ADR-006"
  - "ADR-023"
  - "ADR-062"
  - "ADR-083"
  - "ADR-203"
  - "ADR-253"
  - "ADR-270"
---

# ADR-276: Unicode 注入を撤去する

## 背景

`InjectionMode` は Unicode / Vk / Tsf の 3 つ。`injection_mode_for`(`output/types.rs`)は、`force_tsf` の窓を Tsf、`force_vk` の窓と TsfNative(Chrome 系)を Vk、**それ以外の Win32/UWP を Unicode** にする。
Unicode は `KEYEVENTF_UNICODE` で、ローマ字から変換したかな(`kana_table`)をそのまま送る。

### なぜ Unicode が既定になったか(履歴)

| 出典 | 内容 |
|---|---|
| ADR-0004 | 初期(`3e444b4`、2026-04-01)は「Chrome は VK、それ以外は Unicode」の二分岐。Win32/UWP では `KEYEVENTF_UNICODE` で全角文字が問題なく入る、というのが理由(ADR-023)。**IME の composition を考える前の設計が、そのまま既定として残った。** |
| ADR-006 | PowerToys Command Palette で、他のキーボードフックとの干渉によりバッチ化した VK ローマ字が取りこぼされた。Unicode で解決した。**当時の VK 経路での話で、その後 VK 経路は大きく変わっている。** |
| ADR-023 | 「全アプリ VK 方式は、UWP で VK が正しく処理されない」と却下。**再現の記録は無い。** |
| ADR-083 | 注入モード統一は NO-GO。理由は VK 側の機構(per-VK confirm の構造的欠陥 BUG-045、warm 時の待ち時間が未測定)。「`InjectionMode::Unicode` の廃止」「`UnicodeLiteralObserverFsm` の撤去」を「明示的にやってはいけないこと」に挙げた。**リスクの回避で、Unicode の価値の主張ではない。** |

Unicode でなければならない積極的な理由の記録は無い。

### 実際の使われ方(2026-10-21 の調査)

- `force_tsf` に無い Win32/UWP の窓(メモ帳・Word・Excel・LINE・awase-settings・ストアアプリなど)は、**初回だけでなく毎回** Unicode で送る。
- 事後昇格(ADR-062、`UnicodeLiteralObserverFsm`)は `KeyAction::Romaji` を送ったときにしか動かない。同梱の配列 6 本はローマ字が全部かな表で解決して `KeyAction::Char` になるので、**昇格は起きない**。CI のログ(run 39185861786 ほか)でも昇格は 0 件。
- 所有者の「緊急避難」の認識に合うのは、`InjectionMode::Unicode` ではなく **Tsf モード内の eager かな送信**(Windows Terminal・WezTerm で GJI が 7 秒以上 idle のあとの最初の 1 文字)。こちらは `PendingGjiConfirm` とともに撤去する(下記 D4)。

### 実測: Unicode 注入されたかなは composition になる

`sc-preedit-*`(GitHub の Windows ランナー、run 39597451760)で、EDIT 窓(Unicode 注入)に awase が `Char('か') via Unicode` を送った直後に、IME の未確定文字が `composing:か`、続けて `composing:かか`、Enter で確定して本文 `かか` になった。GJI(ATOK プリセット)・MS-IME・RichEdit・tsf で同じ。
つまり `VK_PACKET` で送ったかなを IME が入力として受け取り、composition に積む。**撤去しても漢字変換できる範囲は変わらない**(ADR-0004 の「IME を経由しない」、`config.toml` の「`force_tsf` が漢字変換を有効にする」は、標準の入力欄については実際と違った)。WezTerm の「Unicode では変換できない」は、WezTerm 固有の TSF 実装の話として残る。

## 決定

コードの削減のため、Unicode 注入を撤去する。

- **D1.** `injection_mode_for` の既定を Vk にする(`force_tsf` は Tsf、TsfNative と `force_vk` は Vk のまま)。`InjectionMode::Unicode`・`UnicodeSender`・`send_romaji_as_unicode`・`send_romaji_per_key` を消す。
- **D2.** 事後昇格の一式を消す: `UnicodeLiteralObserverFsm`、`observe_unicode_literal`、`ProbeAction::UpgradeToTsf`・`StageEndReason::UpgradedToTsf`、`InjectionModeStore` の**書き込み側**。`cache.toml` の既存の `[injection_mode]`(学習済みの Tsf)は**読み取りだけ残す**(既存の利用者の設定を壊さない)。
- **D3.** Unicode 注入があるために存在した同期・ゲートを消す: G2(`needs_belief_sync_on`)の `injection_is_unicode` 引数、A1(`presync_applied_open_on` の GjiFsm への ImeOn)、G1(`sync_ime_kind_from_observation` の ImeOn)、long-idle 閾値の Unicode 腕、Unicode での GjiFsm の即時 warm 化。A1・G1 は VK 注入だけの世界では G2 と二重であることを、`tsf/gji_sync_model.rs` のモデル検証(実物の `GjiFsm` との照合つき)で示した。
- **D4.** Tsf モード内の eager かな送信と `PendingGjiConfirm` を消す(`decide_transmit_plan` の eager 判定、`TsfSendPipeline::transmit` の unicode 腕、`warm_send_plan` の PendingGjiConfirm、`used_eager_path` の受け渡し)。**GJI が応答しない異常時に、ローマ字がそのまま残ることを許容する**(所有者判断、2026-10-21。「そんな異常事態は起きないはずだけど、起きたときはしかたない」)。
- **D5.** 残すもの: `.yab` の `\u` とクォートによる直接指定、表に無い文字(`－` など)の `KEYEVENTF_UNICODE`(`CharResolution::Unicode`、BUG-066)、他アプリが注入した `VK_PACKET` の再注入(ADR-253、BUG-198)。

ADR-083 の「やってはいけないこと」(`InjectionMode::Unicode` の廃止・`UnicodeLiteralObserverFsm` の撤去)は、所有者の判断でこの ADR により覆す。

## 影響・リスク

1. **Win32/UWP の窓が、Chrome 向けに調整した cold-start の機構へ初めて入る。** GJI は per-VK confirm(StaleConfirm→ESC、give-up の reinit)、MS-IME は IME ON 直後の最大 400ms の保留、long-cold の閾値は 5 秒(Unicode は 10 秒)。対象はメモ帳・Word・Excel・LINE(Qt)・PowerToys など。
2. **belief のずれが見えるようになる。** 「Engine ON・実 IME 閉」のとき、今は Win32 窓にかなが入って隠れている(BUG-166)。撤去後は `ka` のようなローマ字が入る。IME 無効の入力欄(パスワード欄など)もローマ字になる。
3. **アプリごとの VK の扱いの差。** ADR-023(UWP で VK が正しく処理されない、再現記録なし)、ADR-006(PowerToys Command Palette の取りこぼし)。MS-IME では小指シフト面の全角記号が半角になる型の問題(BUG-049)が Win32 窓にも出うる。
4. IME OFF のときの出力は、Unicode 注入が担っていない(Engine が `Inactive(ImeOff)` で物理キーを渡す)ので、別手段は要らない。

## 検証

- **モデル検証(Linux、`tsf/gji_sync_model.rs`、PR #765):** 実物の `GjiFsm` の OffCold 性が 2 状態の抽象に全イベント列(長さ 6)で従うことを確かめた上で、A1・G1・G2 の有無ごとに到達可能な状態を厳密に列挙した。VK 注入では A1・G1 を外しても害は増えない。Unicode 注入では増える(= Unicode を残す限り外せない)。
- **比較実験(GitHub の Windows ランナー、`a13-default-vk.sh`、`-dvk` 構成):** 既定を Vk にしたビルドと現状を、同じ入力先・同じ打鍵で並べる。入力先は標準フォーム(edit・multi・rich)と、実アプリ(`forms.toml`: LINE 再現の Qt〈line.exe〉、LibreOffice/OpenOffice、WinUI3、WPF、WinForms、Notepad++、Java)。結果(run 39598656539、2026-10-21): 既定を Vk にした `-dvk` 構成は、rich(GJI 5ms)・edit(MS-IME 20ms)・LibreOffice Writer(GJI 30ms、2 回)の typing で `verdict=PASS`(fail=0・loss=0・extra=0・reorder=0・literal=0・substitute=0、INVARIANTS=OK、VALIDITY=CLEAN)、preedit 構成も観測できた。Qt〈line.exe〉・WinUI3・WPF・Java など残りの入力先は未実施で、P1 の PR で測る。
- **段階 PR ごとの検証:** Linux の単体テスト・モデル検証、`architecture_guard`、e2e-ime の既存構成(`sc-reopen-*`、`ts-*`、`tsx-ext-*`)。

## 段階

1. **P1** 既定を Vk にし、到達しなくなる Unicode 専用の枝を消す(D1・D2 の一部)。
2. **P2** 事後昇格の書き込み側と `UnicodeLiteralObserverFsm` を消す(D2)。
3. **P3** A1・G1・G2 の Unicode 除外・long-idle の Unicode 腕を消す(D3)。モデル検証のテストは「VK だけの世界」の形に更新する。
4. **P4** eager かな送信と `PendingGjiConfirm` を消す(D4)。
5. 各 PR で `architecture_guard` の件数、e2e の構成、古い記述(README・`config.toml`・ADR-0004)を更新する。
