---
id: ADR-281
title: |-
  GJI/Mozc 系で、Ctrl を押したままのひらがな・カタカナ・英数・半角/全角(0xF0〜0xF4)は素のキーとして予測する。それ以外の修飾付きは測るまで抑止し、組成開始の証拠は観測だけにする
summary: |-
  BUG-221(Ctrl を押したままのひらがなキー〈F2〉で実 IME は ON になるのに、予測が修飾付きを全部止めるため belief は OFF のまま残り、
  直後の最初の NICOLA 打鍵「た」が素の E になる)の修正方針。初版は「設定と学習に根拠がある範囲だけ予測、根拠が無ければ組成の開始で追随」
  としたが、Opus レビューで両方とも BUG-221 の症状(最初の打鍵)を直さないと判明し、方針を反転した。Mozc のクライアントは
  この 4 キーの修飾をソースの上で意図的に捨てるので、修飾付きの割り当てという概念がこのキーに無い。範囲を限った「素のキーとして予測」(A′)を採る。
status: |-
  提案(2026-10-26)。Opus レビュー 1 回目を反映(初版から反転)。2 回目の再確認待ち。実装は未着手。
related_adr:
  - "ADR-199"
  - "ADR-186"
  - "ADR-174"
  - "ADR-187"
  - "ADR-202"
  - "ADR-213"
  - "ADR-256"
  - "ADR-258"
  - "ADR-263"
---

# ADR-281: GJI/Mozc 系の「ひらがな系 4 キー+Ctrl」は素のキーとして予測する

## 背景(BUG-221)

dragonflyg4(Windows Terminal×GJI、TSF ネイティブの窓)で、Ctrl+無変換で IME を閉じた直後、Ctrl を離す前にひらがなキー(F2、0xF2)を押した
(2026-10-26 02:36:30 UTC、`awase.log`)。実 IME は ON になった(直後の `E` で候補窓が出た)が、awase の belief は OFF のまま Engine も OFF で、
最初の NICOLA 打鍵「た」は変換を通らず素の `E` として IME に入った(「え」)。「げ」の右親指(変換 0x1C)の予測が効いて初めて追随した。

- 予測が止まった理由: `key_effect_predictor.rs::modifiers_suppress_prediction` が、表にあるモードキーでも Ctrl/Alt/Shift/Win 押下中は予測と追跡を止める。
- 読み直しで直らない理由: TSF ネイティブの窓は `read_ime_state_full` が `ime_on=None`。
- 再現: `closed_loop_scenarios.rs::bug221_*`(PR #849、修正前は `belief=false 真値=true` で赤)。

**受け入れ条件: 修飾付きモードキーの直後の最初の NICOLA 打鍵が、素のキーとして漏れない。**

## 経緯: 初版(設定・学習・組成開始)から反転した理由

初版は「修飾付きの効果は IME の設定で決まる。設定と学習に根拠がある範囲だけ予測し、無ければ組成の開始で追随する」(D2a/D2b/D3)とした。
Opus の敵対的レビュー(2026-10-26)で、次が分かった。

- **Mozc のクライアントは、この 4 キーの修飾をソースの上で意図的に捨てる。** `win32/base/keyevent_handler.cc:330-352` の `ClearModifyerKeyIfNeeded` は、
  special key が `EISU`(VK_DBE_ALPHANUMERIC 0xF0)・`KATAKANA`(0xF1)・`KANA`(VK_DBE_HIRAGANA 0xF2)・`HANKAKU`(0xF3/0xF4)のとき `modifiers->clear()`
  (呼び出しは `:440-443`、コメント「the Mozc server expects that the modifier keys are always empty for some special keys」)。
  設定 GUI も `gui/config_dialog/keybinding_editor.cc:195-205` で「We don't accept any modifier keys for Hiragana, Eisu, Hankaku/Zenkaku keys」。
  つまり `Ctrl Hiragana` という割り当ては**作れず**、届いた `Ctrl+ひらがな` は素の `Hiragana` として扱われる(IME が閉じているときも `:680-685` で素のキーとして
  DirectInput の行〈`IMEOn`〉に一致する)。確認した版は `~/mozc-readable-release/repo/src`(mozc-readable のフォーク。win32 クライアントは上流と同じ)。GJI はクローズドソースだが、
  今回の実機の挙動(Ctrl+F2 で ON)はこの解釈と矛盾しない。
- したがって初版の D2a(設定の明示行)はこの 4 キーでは発火せず、D2b(修飾付きの学習)は不要になる。
- 初版が案 A(修飾付きでも素のキーと同じ)を退けた理由も、この 4 キーには当たらない。「ATOK では素の F2 も開かない」は案 A への反証ではなく(案 A は ATOK でも「開かない」と正しく予測する)、
  「割り当てか、Ctrl の KeyUp 後に F2 が処理された競合か」の問いも、どちらでも Mozc は素の F2 として扱うので結果が同じ。
- 初版の D3(組成の開始を証拠に追随)は、報告された打鍵列を一つも直さない(次の「げ」は既存の変換キーの予測で既に直っている)。また証拠として弱い(後述)。

## 決定(提案)

**D1. 範囲を限った案 A′: 次の 3 条件をすべて満たす打鍵は、修飾による抑止をせず素のキーとして予測する。**
1. IME の種別が GJI/Mozc 系(`ActiveImeKind::GoogleJapaneseInput`。#663 で Mozc 系もここに入る)。
2. 届いた VK が 0xF0/0xF1/0xF2/0xF3/0xF4 のいずれか(キーボードのレイアウトが修飾で VK を変える〈例: 日本語 106/109 では Shift+ひらがな が 0xF1、Alt+ひらがな が VK_DBE_ROMAN/NOROMAN、Alt+半角/全角 が 0x19、ADR-202〉ので、物理キーではなく届いた VK で判定する)。
3. 押されている修飾は Ctrl だけ。Alt/Win/Shift 付きは従来どおり抑止する(Shift は VK が変わる場合に別キーになるので届いた VK で足りるが、まず Ctrl に限る)。

実装の形: `KeyTrackFacts` に IME 種別(または「修飾を捨てる IME か」の真偽値)を 1 つ足し、`plan_key_effect_track`/`modifiers_suppress_prediction` の述語に条件を足す。
書き込みは既存の `KeyEffectPredicted`(`apply_key_effect_prediction`)を通るので、belief の規約の範囲に収まり、新しいイベントや `ObservationSource` は要らない。
最初の打鍵は F2 の KeyDown の時点で ON と予測されるので、漏れない。

**D2. MS-IME 本体・ATOK 製品・Mozc 以外は、測るまで従来どおり抑止する。** MS-IME 本体だけは CI の格子(実機の利用者側ではない)で、
`Ctrl × {0xF0, 0xF1, 0xF2, 0xF3, 0xF4}` の 5 キーを、閉じた状態と開いた状態について測る。セルは「届いた VK × 修飾」で定義し、素のセル(`msime-native.json`)と一致するかを見る。
一致すれば D1 の条件 1 に MS-IME 本体を足す。MS-IME 本体では「Ctrl の KeyUp より後に F2 が処理された競合」がまだ意味を持つので、競合の有無もここで測る。

**D3. 組成開始(候補窓の SHOW)の証拠は、観測だけの段階 0 にする(ADR-258 段階 0 と同じ形)。** belief には入れない。入れない理由は次の「却下した案」。
「belief OFF 中の SHOW」を数えるカウンタとログだけを入れ、読める窓で後の読み取りと突き合わせて誤 ON の率を測る。採るときの条件は、(i) 候補窓の持ち主のスレッド/プロセスが前面の窓と一致、
(ii) ラッチに `FocusGen` を付け世代が違えば捨てる、(iii) 利用者のモードキー(素通しした Ctrl+無変換 を含む)で閉じたときもラッチを捨てる。

**D4. 検証。**
- Linux の単体テスト(`key_effect_predictor.rs`): (a) GJI 系で Ctrl+0xF2 → 予測が出る。(b) MS-IME 本体で Ctrl+0xF2 → 予測しない。(c) GJI 系で Ctrl+変換(0x1C。修飾を消さないキー)→ 予測しない。
  (d) GJI 系で Alt+0xF2 → 予測しない。
- 閉ループ(`bug221_*`): 判定を終わりの belief だけでなく、**最初の打鍵〈`K`〉を押した時点の `input_open` が真値と一致する**ことにする(`PredictionRecord::input_open`)。今の判定は、最初の打鍵を直さない修正でも緑になる。
  あわせて `PredictInput { unreadable: false }` の固定値を、本番の読めない窓に合わせる(予測が `unreadable` で分岐するなら別の枝を通っているため)。
- 実機: 修正後に dragonflyg4 で Ctrl+F2 の直後の最初の打鍵を確認する。

## 却下した案

- **初版 D2a(設定の明示行を読む)**: ひらがな系 4 キーでは割り当て自体が作れないので発火しない。他のキー(F1〜F10 など)の修飾付きの行は、役割を決める読み手が捨てているが、BUG-221 には関係しない。
- **初版 D2b(プリセットの修飾付きを学習で測る)**: 費用に見合わず危険。学習キーは今 14 キー(`LEARN_KEYS`)で F1〜F10 を含まない。レイアウトが修飾で VK を変えるので「物理キー×修飾」の格子は成り立たない。
  利用者の機械で修飾付きを打つと Alt+F4(学習窓を閉じる)・Ctrl+Alt+無変換(単語登録)・Alt+ひらがな(ローマ字/かな入力の切替=設定の変更)などを踏む。`KeyId` に修飾を混ぜると永続化の版・指紋・不具合報告のセル数の上限も変わる。
- **初版 D3(組成開始で belief を ON にする)**: 証拠が弱く、誤って ON にする道が既知のものだけで 4 つある。(1) 判定は GJI の候補窓のクラス名 `GoogleJapaneseInputCandidateWindow` だけ(`win_event_obs.rs:20`)で、Mozc の `MozcCandidateWindow` は見ず、MS-IME・ATOK にも無い。
  (2) `observation_event_proc` はシステム全体の窓を拾い、持ち主や前面の窓と照合しない。(3) 古いラッチが残る(`tsf/observer.rs:681-690`、ADR-213)。BUG-221 のように利用者自身の Ctrl+無変換 で閉じた場合は破棄されない(`platform.rs:693-700`)。
  (4) 候補窓の SHOW は組成開始と同じではない(サジェストを切っていれば、Space で変換するまで出ない)。`.claude/rules/ime-belief-architecture.md` は同種の素の `AtomicBool` を「弱い代理指標」と呼び、それで belief を書いたことが実機バグ 2 件の根本原因だったと記録している。
  また `StartComposition` を配る `drain_pending_composition_events` は `send_keys`/`advance_tsf_probe` の末尾(`platform.rs:756-764, 872`)でしか呼ばれず、Engine OFF の間は届かないことがある。
- **最初の打鍵の保持(ADR-263 型)**: IME を問わず救えるが、期限の定数に実測が要り(tuning-constants)、遅延が増える。D1 で GJI/Mozc 系は足りるので、MS-IME 本体などで同じ症状が報告されたときに改めて検討する。

## 事前に測ること

- M1(読み取りのみ。D1 の確認): 読める窓(Win32 EDIT/RichEdit、ImmCross)で、実 GJI について Ctrl を押す → F2 を押して離す → **Ctrl を押したまま 500ms 以上待つ** → IME の開閉を読む → Ctrl を離す → 読む。
  Ctrl を押している間に ON なら競合ではなく、Mozc と同じく修飾を捨てている。Windows Terminal(TSF ネイティブ)では IME の状態が読めないので、この窓では測れない。
- M2: MS-IME 本体の `Ctrl × 5 キー` の格子(D2)。
- M3: 利用者の `config1.db` は、`session_keymap` の値と、ADR-174 の実機観測(`session_keymap` が MSIME=2 でも表の行が効く)と ADR-186 決定(c)〈ATOK 限定〉との食い違いの再確認に限る(キーマップの行以外は読まない)。BUG-221 の結論には効かない。

## 未決(所有者に確認したい点)

- Q1. D1 の条件 3(Ctrl のみ)を、Shift や Alt まで広げる価値があるか。まずは Ctrl だけで十分とする。
- Q2. GJI はクローズドソースなので、M1 で修飾を捨てる挙動を 1 回確認してから有効にするか(安全側)、確認を実装後の実機確認にするか。

## 影響

- `ime-belief-architecture`: A′ は既存の `KeyEffectPredicted` を通るので規約の変更は要らない。D3 を将来 belief に入れるときは、新しい `ObservationSource` の variant、`state/evidence.rs` の宣言表の行、`architecture_guard` の件数の更新が要る。
- 抑止を前提に書かれた doc の追随: `crates/awase-ime-core/src/state/mode_key_pass.rs:262, 483` と `crates/awase-windows/src/runtime/key_pipeline.rs:1816`。
- `fix-requires-evidence`: IME belief・キー選択の再発ファミリー。BUG-221 の再現テストが (a) に当たる。判定を最初の打鍵に直す(D4)。
- 時間の窓(ラッチの期限・保持の期限)を `tuning.rs` に足す案は採っていないので、tuning-constants の実測義務は発生しない。
