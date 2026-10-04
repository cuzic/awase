---
id: ADR-221
title: |-
  MS-IME の OFF は VK_IME_OFF の後に IMC_SETOPENSTATUS(0) を同じ機構の第2ステップとして補う
summary: |-
  TSF の入力先(実 Chrome 等)に未確定 composition が残っている間に MS-IME が VK_IME_OFF(0x1A)を受けると、閉じずに conv が 25→16 になるだけで開いたままになる(OS への単独注入=awase なしでも同じ、確定。docs/tasks/msime-chrome-off-rca-2026-10-04.md)。IMC_SETOPENSTATUS(0) は composition が有っても 10/10 閉じる。`MsImeDirect` の OFF だけ、VK 送信後に同期 IMC(OFF) を足す。新しい合流点・新しい許可呼び出し元は増やさない。ADR-208 決定4 例外(a)の理由(環境制約)は誤りで、本 ADR で訂正する。
status: |-
  採用(2026-10-04、所有者決定: v2.0.0 に含める)。実装済み(`ime_controller.rs::apply_mechanism`)。CI の MS-IME × Chrome/tsf の OFF 系は試作で全収束を確認済み(本実装の CI は PR 参照)。実機(Windows 11)での「打っている途中で無変換」の確認は未実施。
related_adr:
  - "ADR-208"
  - "ADR-163"
  - "ADR-180"
  - "ADR-089"
---

# ADR-221: MS-IME の OFF に IMC(OFF) の第2ステップを足す

## 背景(確定した原因)

詳細な実験表は [docs/tasks/msime-chrome-off-rca-2026-10-04.md](../tasks/msime-chrome-off-rca-2026-10-04.md)。要点:

- MS-IME は、TSF の入力先(実 Chrome、RichEdit の TSF 窓)に**未確定の composition が残っている間に `VK_IME_OFF`(0x1A)を受け取ると閉じず**、conv を 25→16(半角英数)に変えるだけで開いたままにする。`IMC_GETOPENSTATUS` は 1 のまま。
- awase なしの OS 注入でも同じ(awase のせいではない)。二重送信(0〜400ms 間隔)でも閉じない。composition が無ければ 10/10 閉じる。IMM32 のみの EDIT 窓は composition が有っても閉じる。
- `IMC_SETOPENSTATUS(0)` は実 Chrome でも composition が有っても 10/10 閉じる。GJI では IMC は効かない(API は閉になるが打鍵は `か` のまま)。GJI は VK_IME_OFF だけで composition 有りでも 10/10 閉じる。
- 試作(VK の後に同期 IMC(OFF))で CI の MS-IME × Chrome/tsf の OFF 系が全収束し、MS-IME の `sc-*` 36 構成の比較で回帰なし(補完が走った 9 ジョブの範囲)。

## 決定

1. **`MsImeDirect` の OFF(`ImeOperation::Close`)だけ**、`VK_IME_OFF` の送信が成功した直後に、同じ機構の第2ステップとして `IMC_SETOPENSTATUS(0)` を同期で書く。ON 方向(VK_IME_ON は composition が有っても開く)と GJI(IMC が効かない)には入れない。
2. **「補完するか」は純粋関数 `state::key_sequence_policy::post_vk_followup(KeyMechanism, ImeOperation)`**(ungated、Linux でテスト)が決める。実行(Win32 呼び出し)は `ime_controller.rs::apply_mechanism` の `MsImeDirect` アームの 1 箇所(`msime_close_followup_imc`)。
3. **同期経路(`ImeController::apply`→`SyncChainWriter::write`)と非同期経路(`open_chain::fallback_write`)は、どちらも `apply_mechanism` を通る**ので、同じ決定になる(新しい分岐を `open_chain.rs` に足さない)。`ImmCross` が先頭の窓(Standard)は従来どおり IMC で閉じるので影響しない。
4. **ブロックの扱い**: 補完は `romaji_pre_write` と同じ前提(メインスレッド=フック/メッセージループ、`ActuationTarget::capture_blocking`=`GetGUIThreadInfo` 30ms+フォールバック、`SendMessageTimeoutW` 150ms=`send_ime_control_raw` が `SendHealth` へ実測を流す)。さらに `send_health::blocking_allowed` が偽(直近に slow 判定、cooldown 中)なら**発行せず従来(VK のみ)へ degrade**する。補完の失敗は outcome を変えない(VK は送信済み、best effort)。非同期化(offload)は、遅れて着弾した IMC が直後の ON キーを閉じる競合(BUG-34 型の fence 無し spurious write)を作るので採らない。
5. **合流点・許可リスト**: `RESTRICTED_CALLS` は変更しない。`architecture_guard.rs::sync_romaji_write_goes_through_a_captured_target` の `ActuationTarget::capture_blocking(` の件数ピンは 1→2(`romaji_pre_write` + `msime_close_followup_imc`、同じ捕獲規律)に更新した。補完は既存の `set_ime_open_for_target`(= `actuate_ime_control` の許可呼び出し元)経由で、`apply_mechanism` の呼び出し元(2 箇所)も増えない。`decide_attempt` が返す `MechanismCommand`(journal・replay の凍結コーパス)は変えない(補完は `(mechanism, open)` から決定的に導かれるので、`ActuationDecisionRecord` の再生差分は 0)。
6. 複雑性予算: 追加は純粋関数 1・関数 1・enum 1(合計約 +60 行、うちコメント多数)。新しい合流点・許可呼び出し元・tuning 定数は 0。

## 副作用と影響範囲(「打っている途中で無変換」)

- IMC 経由の閉じは **composition を取り消す**(`compositionupdate`→`compositionend`、`text_post=''`)。従来の VK だけの OFF は composition を残して半角英数にしていた(ただし IME は開いたままで、OFF にならない)。したがって**打っている途中で OFF を押すと、未確定の文字が消える**。Standard プロファイル(ImmCross)の窓では従来から同じ(BUG-184 が同じ症状の報告)。
- 影響する操作: MS-IME 本体 × ImmCross を使わない窓(Imm32Unavailable=実 Chrome、TsfNative)の OFF 系(トグルの OFF、無変換単独タップ、Ctrl+無変換、ADR-208 D4)。GJI、ImmCross 先頭の窓は不変。
- 利用者に見える変化: 修正前は「OFF のつもりが半角英数モードに取り残され、ON キーでかなに戻る」(害は小さい=RCA R4)。修正後は「OFF が確実に効くが、未確定文字は消える」。**この取捨は所有者が v2.0.0 に含めると決定した**。
- 却下した代案: OFF の前に Enter 等で確定する(副作用が大きい)、`VK_KANJI`(確定して閉じるが、トグルで belief 依存、ADR-189)、OFF 前に `CPS_COMPLETE`(未検証、BUG-184 の次の一手)。未確定を残したまま閉じたい要望が実機で出たら、BUG-184 の「確定してから OFF」を別 ADR で扱い、本 ADR の補完の前段に置く。
- **戻す条件**: 実機で「打っている途中の OFF で、VK のみのときより利用者が困る」(例: 確定済みの文字まで消える)が確認された場合、本補完を外して ADR-208 例外(a)に戻す。

## ADR-208 への訂正

ADR-208 決定4 の例外(a)(MS-IME × 実 Chrome の `VK_IME_OFF`)は「環境の制約」と書かれていたが、原因は **MS-IME の挙動(composition が残る間の VK_IME_OFF は閉じない)× awase が VK だけで OFF していたこと**で、awase 側で直せる。例外(a)は本 ADR で撤回する(ADR-208 側に追記)。CI の実 Chrome ハーネスの `ensure` が毎回 `k,a` を打って composition を残すため、OFF が閉じない側だけが毎回再現して『環境固有』と見えていた。

## 検証

- 純粋関数: `key_sequence_policy.rs::post_vk_followup_only_for_ms_ime_close`。golden: `ime_key_sequence_golden.rs`(MS-IME の OFF の送信列は IMC(OFF) を含み、GJI と ON は含まない。Windows のみ)。
- CI(常設、`e2e-ime.yml`): `sc-offrca-fix-msime-awase`(Ctrl+無変換、composition を残す/打鍵後/打鍵なしで、made 全試行が閉じる、`check_offrca.py --expect-closed`)、`sc-offrca-fix-gji-awase`(GJI 対照)、`sc-keymatrix-*-chrome-msime` を observe から pass へ昇格。
- 実機: 未実施(Chrome で語を打っている途中に無変換 → 続く文字が ASCII か、未確定文字が消えるか)。
