# revert コミットの記録規約

## ルール

IME 制御・warmup・focus 分類・キー選択に関わる変更を **revert（取り下げ）** する
コミットは、本文（コミットメッセージの body）に **観測された失敗条件** を必ず書く。
最低限、次の 3 点を具体的に記述すること。

1. **アプリ**: どのアプリで起きたか（Chrome / Edge / Windows Terminal / WezTerm /
   LINE / Teams / VS Code など、実際に再現した対象）
2. **IME**: どの IME か（Google 日本語入力 / MS-IME）と、その状態（ON/OFF・conv mode・
   cold/warm・idle 秒数など、関係するもの）
3. **再現手順 / 症状**: 何をしたら何が壊れたか（例:「フォーカス変更時に spurious
   `apply_ime_open(false)` が発火して直接入力に落ちた」「28 秒 idle 後の最初の文字が
   `bあ` と部分リテラル化した」）

対応する実験ログ（[docs/experiments.md](../../docs/experiments.md)）にも 1 行追記する。

## なぜこのルールが必要か（背景）

このリポジトリでは「IME OFF に何のキーを送るか」だけで **5 日間に 6 回**、採用と撤回が
反転した（`534051a` → `098c663` → `adb856c` → `b271aee` → … → `489cdf1`、前史
`d4d9e27`）。詳細は [docs/experiments.md エントリ 01](../../docs/experiments.md)。

反転が繰り返された最大の理由は、**なぜ前回それを捨てたのか**がコミット本文から
辿れなかったこと。revert コミットが「`X` を revert」とだけ書いてあると、後日別の
セッションで同じ `X` を「良いアイデアだ」と再導入し、同じ失敗を踏む。実際に
`VK_DBE_ALPHANUMERIC` は複数回 IME OFF キーとして採用・撤回され、その都度
「これは半角英数(IME ON)であって直接入力ではない」という同じ事実を再発見していた。

失敗条件（アプリ × IME × 再現手順）が本文に残っていれば、次に同じ変更を検討した
ときに `git log --grep` や `git log <file>` で「これは前に Chrome で効かないと確認済み」
と即座に分かる。revert は「失敗の証拠」であり、その証拠を捨てないための規約。

### 良い revert 本文の例（実在コミット）

`098c663`（`revert(gji): VK_DBE_ALPHANUMERIC → F22 に戻す`）の本文は、
アプリ（GJI）・状態（フォーカス変更時）・症状（spurious `apply_ime_open(false)` の
即時発火で直接入力へ切替）・**なぜ元に戻すと直るのか**（F22 のコールド ~750ms 遅延が
spurious OFF の実害を防いでいた）・根治の方針（spurious apply の抑制は別途）まで
書かれている。この水準を最低ラインとする。

### 悪い例（避ける）

`668a131`（`Revert "fix(ms-ime): ..."`）のように、GitHub / `git revert` が自動生成する
「This reverts commit ...」だけの本文。**何がどのアプリで壊れたのか分からない**ため、
同じ選択肢が再浮上したときの歯止めにならない。自動生成のままにせず、失敗条件を追記する。

## 適用範囲

- 対象ファイルの目安: `output/`（vk_send / probe_io / ime_apply_planner / tsf_warmup 系）、
  `tsf/`、`focus/`、`state/ime_*`、`runtime/ime_coordinator.rs`、`ime_controller.rs`、
  `tuning.rs`。
- 純粋なリファクタや docs のみの revert には適用しない（挙動が変わらないため）。
- 定数値の変更を含む場合は [tuning-constants](./tuning-constants.md) の実測義務も併せて満たすこと。
