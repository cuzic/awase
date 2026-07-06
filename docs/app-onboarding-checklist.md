# 新アプリ / 新 IME オンボーディングチェックリスト

新しいアプリ（ブラウザ・ターミナル・チャットクライアント等）や新しい IME を awase で
安定動作させるときの、**一本道の手順書**。フォーカス分類 → IME ON/OFF に効く VK の
実測 → cold-start 挙動の確認 → 戦略テーブル + テストへの反映、の順に進める。

根拠 ADR: [ADR-060](adr/060-competing-software-detection.md)（競合検出）、
[ADR-063](adr/063-ms-ime-tsf-separation.md)（TSF 共通層 / IME 固有層の分離）、
[ADR-066](adr/066-gji-clsid-ime-detection.md)（GJI CLSID 検出）、
[ADR-075](adr/075-imm-cross-probe-belief.md)（ImmCrossProbe belief 補正）。
関連ルール: [fix-requires-evidence](../.claude/rules/fix-requires-evidence.md)、
[tuning-constants](../.claude/rules/tuning-constants.md)、[experiment-logging](../.claude/rules/experiment-logging.md)。

---

## Step 0: 前提の確認

- [ ] 対象アプリと同時に、競合エミュレータ（やまぶき / やまぶきR / 紅皿）が動いて
      いないか確認する。ADR-060 の通り awase は起動時に検出して警告するが、検証中の
      誤動作の切り分けのため手動でも確認する。
- [ ] 使う IME を確定する（Google 日本語入力 / MS-IME）。ADR-066 の通り「GJI が今
      ユーザー入力を処理しているか」は `gji_is_active_ime()`（プロセス稼働 **かつ**
      CLSID 一致）で決まる。GJI をインストールしていても既定 IME が MS-IME なら
      MS-IME 経路になる。

## Step 1: フォーカス分類を決める

awase は「どのアプリか」を複数の軸で分類する。新アプリはこの各軸のどこに入るかを
確定する。

- [ ] **AppKind**（`focus/kinds.rs`）: `Win32` / `TsfNative` / `Uwp` のどれか。
      Chrome・Edge・VS Code・Electron・WezTerm 等は `TsfNative`。
- [ ] **InjectionMode**（`output/types.rs`）: 文字の送り方。
      `Unicode`（Win32/UWP 既定）/ `Vk`（Chrome/Edge/Electron — IME composition 経由）/
      `Tsf`（WezTerm — TSF 直結、VK Sequential）。
- [ ] **IMM32 クロスプロセス制御が効くか**: 子 hwnd に `ImmGetOpenStatus` 等で
      読み書きできるアプリ（IMM 系）か、できない TSF アプリ（Chrome/Edge 等）か。
      ADR-075 の通り、効くアプリは `ImmCrossProbe`（High confidence 観測）で belief を
      補正できる。効かないアプリは probe / warmup 経路に頼る。
- [ ] 判定根拠（クラス名・`WS_EX_NOIME`・`ES_READONLY`・MSAA ロール）は
      `focus/classify.rs` の `FocusReason` に沿って確認する（テキスト入力を受け付けるか＝
      `FocusKind::TextInput` / `NonText` / `Undetermined`）。

## Step 2: IME ON/OFF に効く VK を実測する

ここが最も間違えやすい。**単一の「正解キー」は無い**（[docs/experiments.md](experiments.md)
エントリ 01 参照）。対象アプリ × IME の組み合わせで実際に送って挙動を見る。

- [ ] 候補キーごとに、送信後の実際の状態を確認する:
  - `VK_IME_ON`(0x16) / `VK_IME_OFF`(0x1A): GJI・MS-IME にネイティブに効き **冪等**。
    ただし **Chrome では受け付けない**（`d4d9e27` で確認済み）。
  - `VK_DBE_HIRAGANA`(0xF2) / `VK_DBE_ALPHANUMERIC`(0xF0): MS-IME 冪等。ただし
    `VK_DBE_ALPHANUMERIC` は **「半角英数」= IME ON のまま**であり直接入力（OFF）
    **ではない**。TsfNative で「OFF にしたつもり」の典型的な落とし穴。
  - `VK_KANJI`(0x19): トグル。TSF compartment を閉じられるが、shadow との desync に弱い。
  - `VK_IME_ON/OFF` が効かないアプリ（Chrome 等）では別戦略（ImmCross / probe）に委ねる。
- [ ] 「OFF にした」結果が **半角英数(IME ON)** ではなく **直接入力(DirectInput, conv=0)**
      になっているかを必ず確認する。conv mode を見る（`state/conv_mode.rs`）。
- [ ] 実測した結果を [docs/experiments.md](experiments.md) に 1 行追記する
      （アプリ × IME × idle × 送ったキー × 観測 × 判定）。

## Step 3: cold-start 挙動を確認する

長時間 idle 後の最初の入力が、IME が温まる前に送られてリテラル化（例: `という→toいう`、
`こ→ko`、`bあ`）していないかを確認する。BUG-01 / BUG-02（[docs/known-bugs.md](known-bugs.md)）
が同種の既知問題。

- [ ] idle 秒数を変えて（数秒 / 10 秒超 / 30 秒超 / 80 秒超）最初の 1〜2 文字を確認する。
- [ ] リテラル化する場合、それが「待ち時間不足」なのか「probe の起点／発火条件のズレ」
      なのかを切り分ける（`79134f5` の教訓: 起点ズレを値上げで塗ることを避ける）。
- [ ] タイミング定数を変える場合は [tuning-constants](../.claude/rules/tuning-constants.md)
      に従い **実測 ms** をコミット本文に残す。盲目的なエスカレーション禁止。
- [ ] MS-IME は常に warm 扱いで SacrificialWarmup を走らせない（ADR-063 変更点 3）。
      新 IME がこのどちらに該当するか（cold probe が要るか）を決める。

## Step 4: 戦略テーブルとテストに反映する

- [ ] IME 固有の制御が必要なら、戦略チェーンに位置づける（ADR-063）:
      `ImmCross → GjiDirect → MsImeDirect → KanjiToggle`。新 IME 用の
      `*DirectStrategy` を足す場合は `is_applicable` の条件（active_ime_kind、
      IMM 制御可否）を明示する。
- [ ] warmup 戦略の分岐（`set_active_ime_kind()`）に新 IME を登録する
      （cold probe あり = GjiFsm 系 / 常時 warm = MsImeStrategy 系、ADR-063 変更点 3）。
- [ ] **KeySequencePolicy**（`ime_controller.rs`、送信キー列の SSOT）に反映し、
      **golden テスト** `crates/awase-windows/tests/ime_key_sequence_golden.rs` に
      期待キー列を追加する（`characterize_strategy` が SSOT）。
- [ ] cold-start / リテラル化のケースを踏んだら、再現手順を
      [docs/known-bugs.md](known-bugs.md) に追記するか、ジャーナルリプレイ / golden の
      回帰テストを足す（[fix-requires-evidence](../.claude/rules/fix-requires-evidence.md)）。
- [ ] IMM クロスプロセスで belief 補正できるアプリなら、`ImmCrossProbe`（High）の
      トリガー（フォーカス入場時など、ADR-075）を設定する。

## 完了の目安

- 対象アプリ × IME で、IME ON → NICOLA 入力 → IME OFF（直接入力）→ 英数入力、が
  cold（長時間 idle 後）でも warm でも一貫して動く。
- 送信キー列が golden テストで固定され、`cargo test -p awase-windows` が通る。
- 試行と実測が experiments.md に、既知の残課題が known-bugs.md に記録されている。
