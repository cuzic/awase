# ADR-186 敵対的レビュー round1

対象: `docs/adr/186-gji-atok-mode-key-measured-matrix-and-belief-follow.md`（worktree `spike-ime-key-matrix`、commit `48476670`）

## 判定

**Blocker 3 / Must-fix 5 / Should-fix 7**

実測そのものは価値があるが、**表の読み取りに要件を直撃する誤りが1件（B1）**、**実測ログが自分の「全セル一致」主張を反証している箇所が1件（B3）**、**決定2が挙げる3キーのうち半角/全角は実装上まったく別経路（B2）**がある。決定2は「configフラグ1つ」で到達できる既存機構なので筋は良いが、ADRが引用している関数名・ADR番号・有効化手段がいずれも実体と違う。決定3は前提の実機確認（物理0xF2がGJIに届くか）が未了のまま「予測反転」を決めており、安全網の評価も過大。決定4は前提（「awaseは入力中を見られない」）が事実誤り。

**推奨**: 決定2のみを先に実機A/Bし、決定3は保留、決定4は「現状維持（composing中は発火しない）」へ倒す。これなら新しい型・フィールドは **0個** で済む（ADR-184の反省に沿う）。

---

## Blocker

### B1. 「直接入力→ONにしたときのconvはどのキーでも0x19(ひらがな)に戻る」は誤り。convはopen遷移で保存される

**根拠（実測）**: `186-measurements/round2-richedit.log`

| 行 | 押下 | 前 | +400ms |
|---|---|---|---|
| 147-151 | 変換（直接入力） | open=0 conv=**0x10** | open=1 conv=**0x10** |
| 157-161 | 変換（直接入力） | open=0 conv=**0x10** | open=1 conv=**0x10** |
| 284-288 | 変換（直接入力） | open=0 conv=**0x10** | open=1 conv=**0x10** |

直後の STEP16（152行）が自動分類で `状態=IME ON・半角英数・入力なし` になっていることが、ONになった先が半角英数だったことの独立な裏付け。逆に round1 の前半（9-60行）は conv=0x09 のまま ON/OFF を往復しており、**convは単に保存されている**（「0x19に戻る」のではなく、そのセッション区間ではたまたま0x19だっただけ）。

**根拠（上流ソース）**: `session/session.cc:1023-1034` — `Session::IMEOn` は `SetSessionState(PRECOMPOSITION)` するだけで、composition mode は **`command->input().key().has_mode()` のときだけ** `ApplyCompositionMode` する。その mode は `win32/base/keyevent_handler.cc:700-705` で `ime_state.visible_conversion_mode`（＝IMMのconv）から作られる。つまり **IMEOn は直前の表示conv（半角英数なら HALF_ASCII）をそのまま復元する**。ADRの記述と正反対。

**失敗シナリオ（要件を直撃する）**:
1. ユーザーが「かな→無変換→半角英数で英字を打つ」…実際はIME OFF（表どおり）。
2. 何らかの経路で conv=0x10 のまま IME OFF になっている状態から、ユーザーが 無変換 を押す。
3. 決定2により awase が `SetOpen(true)` を発行 → `state/eisu_recovery.rs:22-24` の対応表どおり `PostSetOpenEisuReset` が走り、`ObservedEisu → AssumedRomaji` に**消される**。
4. Engine ON。しかし実IMEは **ON・半角英数**。最初の1打から NICOLA のかな出力が半角英数IMEへ流れ、ローマ字がそのまま出る（ユーザー要件「英数のときEngine OFF」の真逆）。

この eisu reset の根拠コメントは `crates/awase-windows/src/runtime/key_pipeline.rs:1659-1661`:
> 「ユーザーが明示的に IME を ON にした時点で IME はひらがなモードで再開するため、過去の英数観測は stale」

この前提が **GJI/ATOK では成り立たない**ことを、本ADRの実測が初めて示した。ADRはこれを「訂正された前提」に挙げるべきなのに、逆向きの誤読（0x19に戻る）を書いてしまっている。

**推奨修正**:
- 実測節の該当行を「**convはopen遷移をまたいで保存される**（IMEOnは直前のvisible convを復元する。session.cc:1023/keyevent_handler.cc:700）」に差し替える。
- 決定2を採るなら、**この経路でのeisu resetを抑止する**（＝ObservedEisuを消さない）ことを決定に含める。これは分岐の**削除側**の変更なので複雑性は増えない。
- `eisu_recovery.rs` の対応表と `tests/architecture_guard.rs::user_ime_on_paths_are_paired_with_eisu_reset` に、GJI例外を書くか「この経路は対で配線しない」ことを明記する必要がある（ガードテストが落ちる）。

### B2. 決定2の「半角/全角もToggle経路」は実装と一致せず、しかも実測が現行モデルを反証している

**実装の事実**:
- `crates/awase-windows/src/vk.rs:104-108, 132-133` — 0xF3=`Deactivate`、0xF4=`ActivatePair`。`shadow_effect()`（139-151行）で 0xF3→`TurnOff`、0xF4→`TurnOn`。**Toggleではなく一方向キーとしてモデル化済み**。
- `crates/awase-windows/src/runtime/transport.rs:405-418` — GJI（`gji_direct_applicable`）のとき 0xF0/0xF1/0xF3/0xF4 の KeyDown は `shadow_toggled` に関わらず**常に Suppress**（既定 `DbeModeKeyPolicy::Suppress`）。つまり**物理半角/全角キーはGJIに届かない**。実際に切り替えるのは awase 自身。
- `delegate_to_open_axis` は `henkan`/`muhenkan`/`hiragana`/`katakana` の4つだけ（`src/engine/engine.rs:184-259`）。半角/全角の枠は存在しない。決定2が言う「既存のToggle経路」は半角/全角には**物理的に到達しない**。

**実測の反証**: `round2-richedit.log:142-146` — **0xF4 を IME ON 中（open=1, conv=0x10, comp="下"）に押したら IME が OFF になった**（+400msでopen=0、未確定も破棄）。押下時刻は≈14:35:42.2（記録時刻−1.5s）で、直前の変換記録の+1500msスナップショット（≈14:35:42.26、140行）が open=0 を捉えているのと整合する。

**なぜそうなるか**: `win32/base/keyevent_handler.cc:315-316` が **0xF3 も 0xF4 も `KeyEvent::HANKAKU`** に潰している（同ファイル59行のコメントが明示）。`atok.tsv:29/75/107` は Composition/Conversion/Precomposition のすべてで `Hankaku/Zenkaku → CancelAndIMEOff`。つまり **GJI にとって 0xF4 は「IME ON」ではなく、ON中なら OFF にするキー**。awase の `0xF4 = ActivatePair(TurnOn)` は MS-IME 由来の一般論であり GJI では誤り。

**失敗シナリオ**: かな入力中に 半角/全角 を押す → OSが 0xF4 を配送するケース（実測で発生） → awase は `TurnOn` と解釈して belief 一致で no-op、物理キーは transport で Suppress 済み → **誰もIMEを切り替えず、Engineもかなのまま**。ユーザーは「半角/全角を押したのにかなのまま」を見る。

**推奨修正**:
- 結果表に **VK列**（そのセルで実際に届いたのが 0xF3 か 0xF4 か）を必ず入れる。現在の表は「半角/全角」1列にまとめており、この矛盾が構造的に見えない。
- 決定2から半角/全角を外す。別の小項目として「GJI使用時は 0xF3/0xF4 とも Toggle 扱いが正しい（keyevent_handler.cc:315-316 + atok.tsv:29/75/107 + round2:142）」を立て、直す場所を `vk.rs::shadow_effect` か `transport.rs` かで決める（IME種別依存なので `vk.rs` の静的表では表現できない点に注意）。
- 未測定として残っている「ON・半角英数 × 半角/全角」も、0xF3/0xF4 のどちらで測るかを決めないと意味を持たない。

### B3. 「2コントロールの全セルが一致」は自分のログが反証している（Composition と Conversion の混同）

**反証**: 同じセル「ON・入力中(未確定あり) × 無変換」で2ラウンドの結果が**違う**。

| | 記録 | 前 | +400ms | 結果 |
|---|---|---|---|---|
| R1 STEP13 | round1-edit.log:108-112 | open=1 conv=0x19 comp="か" | 変化なし | **Δなし** |
| R2 STEP11 | round2-richedit.log:90-94 | open=1 conv=0x19 comp="か" | conv=0x10 | **半角英数へ** |

**原因（推測ではなく確定できる）**: Mozc は「未確定あり」を **Composition（変換前）** と **Conversion（変換中）** の2状態に分ける。
- `atok.tsv:35` `Composition Muhenkan → ToggleAlphanumericMode`
- **`Conversion` に `Muhenkan` 行は存在しない**（grep済み）→ 無変換は no-op
- `atok.tsv:42` `Composition Space → Convert` — R1 は STEP13 の直前 14:27:14 に Space を押している（round1:103-107）ため **Conversion 状態**だった。
- 決定的証拠: R1 STEP14 の 変換 が候補「🉑」を出した（round1:113-117）＝ `Conversion Henkan → ConvertNextPage`（atok.tsv:76）。一方 R2 STEP12 は「下」＝ `Composition Henkan → Convert`（atok.tsv:30）。

つまり ADR の表は **状態軸が Mozc の状態空間の分割になっていない**まま「全セル一致」と書き、かつ矛盾する2件のうち R2 の値だけを採用している。「上流`atok.tsv`との照合: 全セルが一致した」も同様に成り立たない（Conversion×Muhenkan は keymap に行が無いのに表は「ONのまま半角英数へ」と書いている）。

同型の問題がもう1セルある: 「ON・入力中 × ひらがな」は R1 STEP15（round1:143-147、comp="下"＝**Conversion**）の1件のみで、R2 STEP13 は SKIP（round2:120）。`atok.tsv` に `Conversion Kana` 行は無いので、観測された conv 0x10→0x19 は Mozc ではなく **OS/IMM のネイティブ DBE 効果**（`transport.rs:410-418` が警告している「素通しすると実IMEが標準仕様どおり能動的に適用する」あれ）である可能性が高い。つまり効果の出所が違う。

**推奨修正**:
- 表の行を「ON・変換前(Composition)」「ON・変換中(Conversion)」に分割し、Conversion×無変換＝**効果なし**を明記する。
- 「全セル一致」は撤回し、一致したセルとしなかったセルを列挙する。決定1（この表を一次情報として固定する）は、この分割が済むまで保留。
- ADR-184 の症状「ATOKの無変換はIME ONのまま半角英数」は、**Composition中なら正しい**（atok.tsv:35）。前提の訂正1は「誤り」ではなく「状態を取り違えていた」に書き換えるのが正確。

---

## Must-fix

### M1. 引用している関数名・ADR番号が実体と違う。ただし「未解決事項」は確定回答できる

- `resolve_delegate_to_open_axis` という関数は**リポジトリに存在しない**（grep でヒットするのは ADR-186 本文のみ）。実体は `src/engine/nicola_fsm.rs:2130 resolve_pending_thumb_as_single` の `special.delegate_to_open_axis` 分岐（2251-2273行）。
- **ADR-179 決定2 は `ModeKeyActuationOwner` の新設**であって delegate ではない（`docs/adr/179-...md:163`）。しかも ADR-179 は「スコープ外と明示的に宣言する隣接機構（撤去しない・変更しない）」として **delegate-to-open-axis 一式を明示的に除外**している（同ファイル 106-113行）。ADR-186 の決定2 はその除外対象の有効化を提案しているので、ADR-179 との関係は「決定2を使う」ではなく「ADR-179 がスコープ外にした機構を別ADRで有効化する」と書くべき。

**未解決事項「決定2で生キーを消費するか素通しか」への確定回答**（実装から読める、実機確認不要）:
delegate 分岐は `ResolvedAction { actions: SmallVec::new(), output: OutputUpdate::None }` を返す（`nicola_fsm.rs:2263-2271`）＝合成送出なし。物理配送側は `transport.rs:365-372` で 無変換/変換 は既定 Allow だが、親指キーとして押された打鍵は `Decision::Consume` で relay されない（transport.rs:345-352 のコメントが同じ構造を説明）。したがって **二重actuationは起きない**。ただし前提が2つある:
1. **無変換/変換が親指キーとして設定されていること**（そうでなければ `resolve_pending_thumb_as_single` に到達しない）。ADR に前提として明記すること。
2. composing 中は delegate が発火しない（M5 参照）。この場合は Passthrough 設定が生き、生キーがGJIへ行く＝GJI側が ToggleAlphanumericMode する一方 awase の belief は動かない。

### M2. 決定2の有効化手段が誤り。「Passthrough設定」ではなく `gji_thumb_key_ime_toggle = true`

- ATOK の Toggle は `gate_thumb_key_ime_actions`（`crates/awase-windows/src/gji_charset_autodetect.rs:429-459`）と `ime_toggle_kind_to_shadow_action`（461-472行）の **opt-in ゲートで既定 None に落とされている**。有効化に必要なのは `GeneralConfig::gji_thumb_key_ime_toggle`（`src/config.rs:427`）。
- `mode_key_config` の Passthrough は**優先順位4**で、delegate（優先順位3）が発火すると**到達しない**（`nicola_fsm.rs:2251` が先、2274 が後）。「Passthrough設定で有効にする」は仕組みとして成立しない。むしろ決定2を入れると、ユーザーが設定した Passthrough が（composing中を除いて）効かなくなる副作用がある。これは挙動変更なので ADR に書くこと。
- このフラグは **変換と無変換の両方**を Toggle にする（ATOK は両方 Toggle なので gate は両方通す）。BUG-115 が挙げた却下理由4点（`src/config.rs:410-422` に列挙）のうち本ADRの実測で潰せたのは「4. GJIが本家atok.tsvと一致する保証がない」だけ。残る「1. Toggleの非冪等性」「2. 親指キー2本への露出倍増」「3. ATOKプリセット選択者全員に自動適用（今回はopt-inなので緩和）」への評価を決定2に書くこと。特に `config.rs:452-455` が警告する「TSFネイティブアプリ（`FeedbackPolicy::Blind`）では実IME状態を読み戻せないため belief がズレると逆方向へ切り替わる」は、Toggle 採用の中心的リスクであり、ユーザー原則「IME ON/OFFは安定して観測できない」と真っ向から関係する。

### M3. 決定3の安全網（idle-conv-check）の評価が過大

`src/engine/idle_check.rs:33-69` の5ガード:
- **ガード2（48行）: `is_tsf_native` でないと走らない** → 素のWin32/IMMアプリ（今回測ったEDIT相当）・Uwpでは**訂正が一度も走らない**。
- **ガード5（66行）: `is_ime_mode_key` の打鍵自身はスキップ** → ひらがな/Shift+無変換を押したその打鍵では絶対に読まない（BUG-113対策で意図的にそうしてある）。
- ガード4（60行）: explicit IME操作後 `EXPLICIT_IME_SUPPRESS_MS`（1500ms）は抑止。
- ガード3（54行）: `TYPING_IDLE_MS`（500ms）以上のタイピング停止が必要。

つまり予測が外れた場合の露出は「次の打鍵まで」ではなく **「次に500ms以上手が止まり、かつ最後の明示IME操作から1500ms経過するまで」** 続く。連続入力中はずっと誤ったEngine状態。ADRの「安全網は今のまま」「受動観測が訂正する」は、この条件を書いた上で「非TsfNativeでは訂正されない」ことを明記すること。

### M4. 決定3は既存 UserTurnOnEisuReset と衝突する。かつ物理0xF2がGJIに届くか未確認

**衝突**: 物理ひらがな(0xF2)は `vk.rs:104` で `ImeKeyKind::Activate`、`shadow_effect()` で **`TurnOn`**。`kp_stage_shadow_ime_toggle` は open が既に ON のとき `eisu_reset_on_turn_on_while_open` を呼び、**`ObservedEisu → AssumedRomaji` の片方向**を書く（`key_pipeline.rs:1606-1620`）。ATOKのひらがなキーは**双方向トグル**なので:
- 「半角英数 → ひらがなキー → かな」: 現行の片方向リセットがたまたま正しい。
- 「かな → ひらがなキー → 半角英数」: 現行コードは何も書かない（belief はかなのまま）→ Engine ON のまま → **要件違反がそのまま残る**。決定3はこの半分を直す提案だが、ADRは既存の片方向リセットを**置き換える**のか**併存させる**のかを書いていない。併存させると「かな→英数」を書いた直後に別経路の TurnOn が AssumedRomaji へ戻す競合が起きる。

**さらに併存する分岐**: `half_width_alnum.is_toggle_active()`（左Shift単独タップの半角英数持続トグル）のときは `kp_restore_kana_from_half_width` へ委譲する（`key_pipeline.rs:1606-1612`）。決定3が書く ObservedEisu はこのゲートを通らないため、状態が2系統になる。

**未確認の前提（ADRの未解決事項に無い）**: `transport.rs:197-206` — 「TSF mode かつ `f2_warmup_owned=true`（GJI戦略）: Down/Up 共に Suppress」。GJI環境では **物理 0xF2 が GJI に届かない可能性が高い**。届かないなら GJI 側の かな⇔英数 トグルは起きず、決定3の「予測反転」は**常に外れる**（belief だけ反転して実態は不変）。決定3を実装する前に、実機debugログで「ひらがなキー押下時に GJI の conv が実際に変わるか」を確認すること。これは決定3の成否そのものを決める。

### M5. 決定4の前提「awaseは入力中かどうかを確実には見られない」は事実誤り

`resolve_pending_thumb_as_single` は `composing: bool` を引数に取り、delegate 分岐は**composing 中は意図的に発火しない**（`nicola_fsm.rs:2261-2273`）:
> 「composing 中は fail-closed に倒す。誤って true でも suppress に落ちるだけだが、誤って false で TurnOff/Toggle(→OFF) すると composition を復旧不能に破棄する。」

決定4（入力中の無変換も開閉トグル扱い）を実装するには、この fail-closed ガードを**外す**ことになる。その結果:
- ATOK の実動作（Composition: ToggleAlphanumericMode、未確定は保持）と違い、awase が `SetOpen(false)` を送る。
- IME OFF 送出時に未確定文字列がどうなるかは**本ADRで実測していない**。参考として 半角/全角（CancelAndIMEOff）では **未確定が破棄された**（round1:160-164、comp="か"→""）。awase の SetOpen(false) が同じ挙動になるなら、ユーザーの入力中文字列が消える経路を新設することになる。

**推奨**: 決定4は「composing 中は現状維持（delegate は発火させない、ModeKeyConfig に委ねる）」に倒す。ADRの論拠「どちらでもEngine OFFになる点は一致する」は、Engine の話であって**ユーザーの未確定文字列の話ではない**。

---

## Should-fix

### S1. 決定3の「`InputModeApplyStrategy` に1つ追加」は不要（既存variantで足りる）

既存の `InputModeApplyStrategy::UserHalfWidthAlnumToggle` が、まさに「open を一切動かさず kana⇔eisu の belief だけを動かす」ための variant として存在する（`key_pipeline.rs:2246/2267/2645`）。`state/eisu_recovery.rs:62-73` に「この variant は対応表の対象外（SetOpen を発行しないため）」という設計意図まで文書化済みで、決定3が必要とする性質と完全に一致する。新 variant を足すと、同じ意味の variant が2つになり `eisu_recovery.rs` の対応表と `architecture_guard` の期待値を二重管理することになる。ADR-184 で型・フィールドを積み増して押し戻された経緯と同型なので、**既存variantの再利用**を決定3に明記すること。

### S2. 「観測値A/B/Tは全件一致」は正確でない（327スナップショット中3件が不一致、原因はフォーカス外れ）

機械的に再集計した結果: round1 141スナップショット中 **3件**（`round1-edit.log:94,95,96`）で A/B/T が不一致（`A(open=? conv=?) B(open=0 conv=0x00) T(open=1 conv=0x19)`）。round2 は 186件中 0件。

原因は `take_snapshot` が `GetFocus()` の戻り値（null なら親ウィンドウ）を観測対象にしていること（`examples/ime_key_matrix_spike.rs:445-447`）。この記録は `tail="9→0x19\r\n"` ＝**ログ欄のEDITを見ている**。同様に `round1:242`・`round2:294-318` は `tail="ase 非依存)"` ＝**ウィンドウタイトル**を見ており無効。

実害: この無効な3件が「直接入力で0xF3を押しても何も起きない」ように読める**唯一の記録**でもある。B2 の判断材料から除外する必要がある（＝**直接入力×0xF3 の有効な測定は1件も無い**）。ADR は「A/B/Tは一致（フォーカスが入力欄から外れた無効レコード3件を除く）」と条件付きで書き、無効レコードの行番号を列挙すること。

### S3. 2ラウンドは「同一手順」ではない

round1 のログヘッダは「全2ラウンド×**24**ステップ」でステップ表記も `STEP n/24`、round2 は `STEP n/20`。`KEYS` 定数を変更した別ビルドで走らせている（commit `9555d705`「英数を対象外、カタカナはShift併用を許可」、`5bf5028f`）。ADR summary の「同一手順で測った結果」は訂正を。

### S4. conv の 0x09 と 0x19（ROMANビット）の違いに言及が無い

測定開始時は conv=0x09（`NATIVE|FULLSHAPE`＝**かな入力**、ROMANなし）。最初に半角英数へ往復した後（`round1:76-80` の 0x10→0x19）以降は 0x19（+ROMAN）になり、元に戻らない。awase はローマ字を送る前提なので、この差はEngine動作に直結する。表の各セルがどちらの conv で測られたかを明記すること。ADR の「conv の目安」行は 0x19/0x10 しか説明しておらず、ログの大半を占める 0x09 が説明されていない。

### S5. Shift+ひらがな(0xF1) を「未測定」と「keymap上未割当」で書き分ける

両ラウンドで SKIP（`round1:82,211`、`round2:74,121,283`）。round1:201-205 に ON 中の1件があるが、1.37秒後の Shift+無変換（206行）で +1500ms が汚染されており根拠にならない。`atok.tsv` には `Katakana` 行が一つも無い（`ConvertToFullKatakana` は Ctrl+i / F7 のみ）ので「keymap上は未割当＝効果なしのはず」と書くのは妥当だが、「未測定」と区別すること。なお実際には OS の DBE ネイティブ効果が乗る可能性がある（B3 で挙げた Conversion×Kana と同型）。

### S6. 参照ADRがこのブランチに存在しない／ADR-179のマージ前TODOと矛盾する

`spike/ime-key-matrix` の `docs/adr/` に 179/181/183/184/185 のいずれも無い（179と181/183/184はメインworktreeの**未追跡ファイル**、185は別worktree）。決定2が言う「既存のToggle経路」「ADR-184の配線」が develop に無い可能性があるので、status 節にどのブランチを前提にしているかを書くこと。

さらに ADR-179 の frontmatter（23-26行）は **「Passthrough設定を前提にした実験4件が現在も有効。developへマージする前に実験を撤去して既定（Suppress）へ戻す必要がある」** と明記している。決定2が「Passthrough設定で有効にする」と書いている限り、この撤去TODOと正面衝突する（M2のとおり Passthrough は本来不要なので、記述を直せば衝突は消える）。

### S7. 複雑性: 3決定を同時に入れると分岐が2本増える。最小案は決定2のみ

現状でも「誰がactuateし誰がbeliefを書くか」は `delegate_owned` / `explicit_ime_action_consumed` / `auto_delegate_open_axis_consumed` / `ModeKeyActuationOwner`（ADR-141/147/153/154/179）が積み重なっており、`.claude/rules/fix-requires-evidence.md` の「IME actuation 合流点」行に6つの独立入口が列挙されている領域。決定2+3+4 を同時に入れると、ここに「かな英数軸の予測反転」と「composing中のfail-closed解除」という2本の新分岐が加わる。

**最小案**:
1. **決定2のみ**（`gji_thumb_key_ime_toggle = true` + B1のeisu reset抑止）。コード変更は eisu reset の抑止条件1つ（**分岐の追加ではなく既存分岐への条件追加**）。新しい型・フィールドは0。
2. 決定3は M4 の実機確認（物理0xF2がGJIに届くか、届かないなら何も起きない）が済むまで保留。やる場合も S1 のとおり既存 `UserHalfWidthAlnumToggle` を再利用し、既存の片方向 `UserTurnOnEisuReset` を**置き換える**（併存させない）。
3. 決定4は「現状維持」に倒す（M5）。

---

## 要件充足の評価（決定2のみを入れた場合）

| ユーザー操作 | ATOK実動作 | awaseの結果 | 要件充足 |
|---|---|---|---|
| かな中に無変換 | IME OFF | belief OFF（Toggle）→ Engine OFF、押下時点 | **満たす** |
| 半角英数(ON)中に無変換 | IME OFF | 同上 | **満たす** |
| 直接入力で無変換 | IME ON（**convは直前値を復元**） | belief ON → Engine ON。実IMEが半角英数なら破綻 | **B1修正が必須** |
| 入力中に無変換(Composition) | ONのまま半角英数 | delegate発火せず（fail-closed）→ Passthrough設定ならGJIが切替、beliefは動かず → 次のidle-conv-checkまでEngine ON | 満たさない（既知の遅延のまま） |
| 入力中に無変換(Conversion) | **効果なし** | 同上 | — |
| ひらがなキー | かな⇔半角英数 | 現状は `TurnOn` 扱い（片方向eisu reset）。決定3を入れないと「かな→半角英数」が追随しない | 満たさない |
| 半角/全角 | IME ON/OFF トグル | 0xF3なら TurnOff で正しい。**0xF4が届くとno-op**（B2） | 部分的 |
| belief誤予測時 | — | Toggleなので**逆方向へactuate**。訂正は非TsfNativeでは来ない（M3） | リスク残 |
| フォーカス切替直後 | — | `kp_trigger_focus_resync` はTsfNative限定・ガード4/5は効く | リスク残 |

---

## 実測の再集計（独立に再構成）

記録時刻は**押下の約1.5秒後**。`gap` は前の記録との差なので、gap < 1.5s の記録は前のキーの効果が混入している。STEP行のうち gap が十分（>2s）かつ次の押下まで1.5s以上あるものだけを「クリーン」とした。

### 状態 × キー（クリーンな STEP のみ、出典行つき）

| 状態 | キー(届いたVK) | R1 | R2 | 一致 |
|---|---|---|---|---|
| 直接入力 | 無変換 | ON（r1:9-13） | ON（r2:9-13） | ✓ |
| 直接入力 | 変換 | ON（r1:19-23） | ON（r2:19-23） | ✓ |
| 直接入力 | ひらがな 0xF2 | 変化なし（r1:24-28） | 変化なし（r2:29-33） | ✓ |
| 直接入力 | Shift+ひらがな 0xF1 | 未測定(SKIP) | 変化なし（r2:34-38） | — |
| 直接入力 | 半角/全角 **0xF4** | ON（r1:56-60） | ON（r2:39-43） | ✓ |
| 直接入力 | 半角/全角 **0xF3** | **有効な測定なし**（r1:93-97, 242-246 はフォーカス外れ） | 無効（r2:314-318） | — |
| ON・かな | 無変換 | OFF（r1:61-65） | OFF（r2:44-48） | ✓ |
| ON・かな | 変換 | OFF（r1:66-70） | OFF（r2:54-58） | ✓ |
| ON・かな | ひらがな 0xF2 | conv 0x09→0x10（r1:71-75） | conv 0x09→0x10（r2:64-68） | ✓ |
| ON・かな | 半角/全角 **0xF3** | OFF（r1:83-87） | OFF（r2:75-79） | ✓ |
| ON・**Composition** | 無変換 | 測定なし（※下記） | conv 0x19→0x10（r2:90-94） | — |
| ON・**Conversion** | 無変換 | **Δなし**（r1:108-112） | 測定なし | **✗ 矛盾** |
| ON・Composition | 変換 | 測定なし | comp か→下＝Convert（r2:95-99） | — |
| ON・Conversion | 変換 | comp か→🉑＝ConvertNextPage（r1:113-117） | 測定なし | — |
| ON・Conversion | ひらがな 0xF2 | conv 0x10→0x19（r1:143-147） | SKIP | 1件のみ |
| ON・入力中 | 半角/全角 | **0xF3**: OFF+未確定破棄（r1:160-164） | **0xF4**: OFF+未確定破棄（r2:142-146） | VKが違う |
| ON・半角英数 | 無変換 | OFF（r1:175-179） | OFF（r2:152-156） | ✓ |
| ON・半角英数 | 変換 | OFF（r1:180-184） | OFF（r2:162-166） | ✓ |
| ON・半角英数 | ひらがな 0xF2 | conv 0x10→0x19（r1:185-189） | SKIP | 1件のみ |
| ON・半角英数 | 半角/全角 | 未測定 | 未測定 | — |
| ON（入力なし） | Shift+無変換 | conv 0x10→0x19 / 0x19→0x10（r1:76-80, 190-194） | 同（r2:69-73, 105-109, 172-176） | ✓ |

※ R1 STEP13/14 が Conversion 状態だった根拠: 直前 14:27:14 の Space（r1:103-107、`atok.tsv:42 Composition Space → Convert`）＋ STEP14 が ConvertNextPage 相当の「🉑」を返したこと（`atok.tsv:76`）。

### open遷移に対する conv の挙動（B1の根拠、全OFF→ON遷移）

| 出典 | キー | 前 conv | ON後 conv |
|---|---|---|---|
| r1:56-60 | 0xF4 | 0x09 | 0x09 |
| r1:88-92 | 0xF4 | 0x19 | 0x19 |
| r1:155-159 | 0xF4 | 0x19 | 0x19 |
| r2:9-13 | 無変換 | 0x09 | 0x09 |
| r2:147-151 | 変換 | **0x10** | **0x10** |
| r2:157-161 | 変換 | **0x10** | **0x10** |
| r2:284-288 | 変換 | **0x10** | **0x10** |

→ **convは保存される。「0x19に戻る」ケースは1件も無い。**

### atok.tsv との照合（grep 実施済み）

| 状態 | 行 | 割当 | 実測との整合 |
|---|---|---|---|
| DirectInput | 95,96,98 | Hankaku/Zenkaku, Henkan, Muhenkan → **IMEOn** | ✓ |
| DirectInput | （Kana行なし） | — | ✓（0xF2 変化なし） |
| Precomposition | 107,108,111 | Hankaku/Zenkaku, Henkan, Muhenkan → **CancelAndIMEOff** | ✓ |
| Precomposition | 105,109,115 | Eisu, Kana, Shift Muhenkan → **ToggleAlphanumericMode** | ✓ |
| Composition | 30 | Henkan → Convert | ✓ |
| Composition | 32,35,40 | Kana, Muhenkan, Shift Muhenkan → ToggleAlphanumericMode | ✓（R2 STEP11） |
| Composition | 29 | Hankaku/Zenkaku → CancelAndIMEOff | ✓ |
| **Conversion** | 76 | Henkan → ConvertNextPage | ✓（R1 STEP14） |
| **Conversion** | （Muhenkan行なし） | — | ✓（R1 STEP13 Δなし）だが**ADRの表と矛盾** |
| **Conversion** | （Kana行なし） | — | **✗ R1 STEP15 は conv が変化した**（OS側DBEネイティブ効果の疑い） |
| 全状態 | 29,75,107 | Hankaku/Zenkaku → CancelAndIMEOff | ✓。0xF3/0xF4 とも HANKAKU（keyevent_handler.cc:315-316） |
