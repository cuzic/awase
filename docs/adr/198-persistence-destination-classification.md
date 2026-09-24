---
id: ADR-198
title: |-
  永続化先の分類（config.toml / cache.toml / 学習表 JSON）と v2 での calibration の扱い
summary: |-
  v2 方針（2026-09-23 のユーザー決定、2026-09-24 に「変わらない」と再確認）のうち永続化先の分類だけを定める。
  (1) 永続化先は「ユーザー設定 / 観測キャッシュ / 学習表」の3分類で、軸は可搬性（dotfiles 同期してよいか）と再生成コスト。
  (2) 学習表は cache.toml の節にせず別 JSON のまま（ADR-195 段階3 が第一候補とし、実装が採用）。
  (3) calibration の扱いは手動較正パネル（ADR-176）の去就に条件付き: 撤去なら移設不要、残すなら書き先を cache.toml へ移す
  （ユーザー決定どおり）。**この点はユーザー再確認待ち**。(4) cache.toml の書込はアトミックにする（PR #302）。
  ConfirmMode の2択化は確定エンジンの設定で永続化先と無関係なので範囲外（後継 ADR は未起票）。
status: |-
  草案（2026-09-24）。opus-adversarial-consult round1 の指摘を反映済み、再確認待ち。決定3 はユーザー確認が確定条件。
related_adr:
  - "ADR-176"
  - "ADR-191"
  - "ADR-195"
  - "ADR-196"
  - "ADR-099"
  - "ADR-058"
  - "ADR-125"
  - "ADR-162"
---

# 永続化先の分類（v2 方針、俯瞰レビュー A-10 / C-4 / C-5）

## 背景

v2 では calibration を config.toml から cache.toml へ移す、というユーザー決定（2026-09-23）があった。
理由は**可搬性**である。config.toml は dotfiles で複数マシンに同期しうる「ユーザーの意図」で、
cache.toml は IME の版やキーボード配列に結びつく「機械依存の事実」という線引き。
この決定の出典はリポジトリ外のメモだけだったので、本 ADR が決定内容と理由を本文に書く。
メモの内容は現行の実装・ADR と次の2点で合わない。

- 「学習表も cache.toml の `[keymap_learn]` 節に置く」: ADR-195 段階3 は別ファイルを第一候補とし、実装は別 JSON を採った
  （`crates/awase-keymap-learn-win/src/main.rs`、`awase::fs_atomic::write_atomic`）。
- 「calibration を cache.toml へ移す」: `[[calibration]]` を読む本番コードが無い（適用側は ADR-191 で撤去、`9dc52c89`）。
  書き手は `crates/awase-settings/src/main.rs` の手動較正パネルだけが残っている。

## 範囲

IME・キー効果に関する永続化だけを対象にする。`update_check.json`、`layout/*.yab`、不具合報告の出力はこの3分類の対象外
（別の性質のファイルで、v2 の論点ではない）。

## 決定

### 決定1: 永続化先は3分類とする

| 分類 | 置き場 | 可搬性（同期してよいか） | 性質 | 例 |
|---|---|---|---|---|
| ユーザー設定 | `config.toml` | よい | ユーザーの意図。消えると困る | `[keys]`、`app_overrides`、`use_learned_keymap_table` |
| 観測キャッシュ | `cache.toml` | 不可（機械依存） | 再観測で戻る。消えても実害は初回のやり直しだけ。**手編集されうる**（`focus/tracker.rs` が誤学習時の手編集を案内している） | `[imm_capability]`（ADR-125）、`[injection_mode]`（ADR-058） |
| 学習表 | 別 JSON（`keymap-learn-table.json`、`keymap-learn-last-attempt.json`） | 不可（機械依存） | 再生成コストが大きい。表全体を1ファイルで持つ | ADR-195 段階3 |

置き場の解決規則は現状2つある。cache.toml は `current_exe()` の親（`app/bootstrap.rs`）、config.toml と学習表は
`awase::paths::resolve_relative_to_exe`（exe の隣、開発ビルドではワークスペースルート）。開発ビルドでは置き場が分かれる。
04 A-9 のクリアメニューなど cache.toml を触る新規実装は、cache.toml と同じ規則で解決すること。

**既知の不整合（本 ADR では直さない）**: 学習表 JSON は機械依存なのに config.toml と同じディレクトリにある。
ディレクトリごと dotfiles 同期すると別マシンの学習表が持ち込まれる。06（学習表のキーマップ指紋、失効検出）が
実装されれば検出できる。置き場を分ける案は「採らなかった案」に記録した。

### 決定2: 学習表は cache.toml の節にしない

再生成コストが大きい。学習は自動測定だが、ウィンドウとキーボードを専有する（所要時間は、段階1の全数学習だけで17分相当
〈ADR-195 成功基準〉、段階2は未計測、MS-IME 本体は20分基準の対象外）。消されうる cache.toml に混ぜる理由がない。
粒度も違う（ADR-195 段階3）。

### 決定3: calibration の扱い（条件付き、ユーザー再確認待ち）

2026-09-23 のユーザー決定は「calibration を cache.toml へ移す」で、2026-09-24 に「変わらない」と再確認されたが、
そのときは「読む側が無い」ことを前提にしていなかった可能性がある。実装側の判断で反転させないため、条件付きで書く。

- 手動較正パネルを撤去する（07 タスク (2)）なら、`[[calibration]]` は書き手も読み手も無くなるので、移設は不要になる。
  ユーザー決定の**目的**（config.toml から機械依存データを追い出す）は撤去で達成される。
- パネルを残すなら、`[[calibration]]` の書き先を config.toml から cache.toml へ移す（ユーザー決定どおり）。
  代替として、読み手が無いので**書き込みだけ止めて表示のみにする**第三案が最も安い。
- 07 (2) は判断保留中なので、現時点では上の3つのどれとも確定しない。**ユーザーの選択が確定条件**。
- どの場合も、`AppConfig` のフィールドを消すと、awase-settings の保存（`AppConfig::save` はファイル全体を書き直す）で
  既存の `[[calibration]]` は消える。読む側が無いので消えてよい。読み込みは `deny_unknown_fields` が無いので壊れない。
- ADR-176 の frontmatter status は今も「適用あり」の状態を書いている。適用は ADR-191（`9dc52c89`）で撤去済みで、
  status の訂正は 07 (2) で行う。

### 決定4: 優先順位

ユーザー設定（config.toml）は観測キャッシュ・学習表に常に勝つ。現状の実装もそうなっている（`app_overrides` が先で、
`InjectionModeStore` は未マッチのときだけ参照する）。学習表を使うかどうかは config.toml の `use_learned_keymap_table`
（既定 true、opt-out が常に勝つ）が決める。ADR-196 T4 が要確認判定を表 JSON の中で「採用」に書き換える設計は、
ユーザーの明示判断を表ファイルに置くことになる。再学習・再検証で採用が消えない扱いは ADR-196 T4/T5 で決める（本 ADR では決めない）。

### 決定5: cache.toml の書込はアトミックにする

`save_section` は `awase::fs_atomic::write_atomic` を使う（PR #302、本 ADR と同じブランチ。マージ順は同時とする）。
代償として `sync_all` と rename 再試行（最大約200ms）が、フォーカス切替時のメインスレッドで発生しうる。
頻度は process/class ごとの学習時だけなので受け入れる。オフロードは別判断。
プロセス間ロックは無いが、cache.toml に書くのは awase.exe だけなので不要。

読込失敗時に他セクションが全消えする問題（07 (7)）は未解決で、**「セクション追加時」ではなく次のどれかが起きた時点で決める**:
04 A-9 のクリアメニューを実装するとき、または tracker.rs の手編集案内を残す限り。手編集の構文ミスで他セクションが消える経路は、
アトミック化では防げない。

## 範囲外

- ConfirmMode の2択化（確定エンジンの設定）。04 T2（推奨モード統一）の後に、新規 ADR として起票する。担当は 07 (5) の後継。
  04 が「07 の ADR で覆す」と書いている箇所（`review-2026-09-24-04` の推奨モード統一に関する2行）は、その新規 ADR を指すよう直す。
- `keys.ime_detect.*`、`keys.ime_on`、`*_solo_tap_ime_action`、`keyboard_model` などの config 簡略化（v2 の別論点）。
- 手動較正パネルの撤去そのもの（07 (2)、03・06 の判断待ち）。

## 採らなかった案

- (a) calibration を無条件に cache.toml へ移す: 読み手が無いデータを移すことになる。パネルを残す場合だけ決定3で復活する。
- (b) 学習表を cache.toml の `[keymap_learn]` 節に置く: 粒度が違い、cache.toml は手編集・クリアで消えうる。
- (c) 機械依存データ（cache.toml・学習表）を `<config dir>/state/` や `%LOCALAPPDATA%\awase\` へ分ける: 可搬性の軸には最も合う。
  ただし既存ユーザーのファイル移行と、置き場の解決規則の統一が要る。v2 の実装計画で再評価する（本 ADR では採らない）。
