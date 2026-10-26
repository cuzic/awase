---
id: ADR-281
title: |-
  GJI/Mozc 系で、修飾(Ctrl/Shift)を押したままのひらがな・カタカナ・英数・半角/全角(0xF0〜0xF4)は素のキーとして予測する。それ以外の修飾付きは測るまで抑止し、組成開始の証拠は観測だけにする
summary: |-
  BUG-221(Ctrl を押したままのひらがなキー〈F2〉で実 IME は ON になるのに、予測が修飾付きを全部止めるため belief は OFF のまま残り、
  直後の最初の NICOLA 打鍵「た」が素の E になる)の修正方針。初版は「設定と学習に根拠がある範囲だけ予測、根拠が無ければ組成の開始で追随」
  としたが、Opus レビューで両方とも BUG-221 の症状(最初の打鍵)を直さないと判明し、方針を反転した。Mozc のクライアントは
  この 4 キーの修飾をソースの上で意図的に捨てるので、修飾付きの割り当てという概念がこのキーに無い。範囲を限った「素のキーとして予測」(A′)を採る。
status: |-
  採用・実装済み(2026-10-26、#849・#856)。dragonflyg4 で所有者が実機確認済み(BUG-221、Ctrl+F2)。CI 実測は e2e-ime run 40272737475(16 構成 × 3 回)。未実施: M2 の残り(MS-IME 本体の 0xF0・0xF1・0xF3・0xF4)、D2'(Converter I/O による確認)。単体テスト (a)〜(j) は追加済み。
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
  - "ADR-209"
  - "ADR-173"
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

## Mozc のソース調査(Shift/Alt まで広げてよいか。所有者の指示で確認、2026-10-26)

問い: 0xF0〜0xF4 で、Shift や Alt の付いた打鍵に独自の動作を設定できるか。できないなら、Ctrl と同じく素のキーとして予測してよい。確認した版は `~/mozc-readable-release/repo/src`。

- `win32/base/keyevent_handler.cc:376-387`: 修飾は `keyboard_status`(その時点のキーボード状態)から SHIFT・CTRL・ALT(VK_MENU)・CAPS を集める。Win は見ない。
- 同 `:330-352, 440-443`: special key が EISU/KATAKANA/KANA/HANKAKU(= VK 0xF0〜0xF4)のとき、`modifiers->clear()`。`clear()` は**集めた修飾すべて**(Shift・Ctrl・Alt・Caps)を消すので、Ctrl だけが特別ということはない。
- `gui/config_dialog/keybinding_editor.cc:195-205`: 設定 GUI はこの 4 キーに修飾を付けた割り当てを作らせない。手で `Shift Hiragana` のような行を書いても、クライアントが常に修飾なしで送るので一致しない。
- 修飾によって**届く VK が変わる**のは Windows/キーボード配列の側(日本語 106/109 で Shift+ひらがな → 0xF1、Alt+ひらがな → VK_DBE_ROMAN/NOROMAN、Alt+半角/全角 → VK_KANJI〈0x19〉、ADR-202)で、Mozc の前の段。
  そのため判定は物理キーではなく**届いた VK**で行う(D1 条件 2)。VK_DBE_ROMAN/NOROMAN(0xF5/0xF6)と VK_KANJI は TSF の「予約キー」として別経路で扱われ(`win32/tip/tip_text_service.cc:326-356`、`TF_MOD_IGNORE_ALL_MODIFIER`)、これも修飾を問わないが、本 ADR の対象外(0x19 は shadow toggle の経路〈`kanji_shadow_action`、能動〉で扱われ予測の段に来ない、0xF5/0xF6 は開閉を変えない)。

結論: この 4 キーでは、**キーが key handler に届いたなら**、Shift・Alt・Ctrl のどれが付いても独自の動作を設定する手段が Mozc(と、同じクライアントを持つと考えられる GJI)に無い。
Shift と Ctrl は届くので広げる。Alt は届くかを確かめていないので除く(D1 条件 3)。GJI はクローズドソースなので、実機の確認(M1)は残す。

## 決定(提案)

**D1. 範囲を限った案 A′: 次の組み合わせの打鍵は、修飾による抑止をせず素のキーとして予測する(`ModeKeyModifiers`)。**

| IME | キー(届いた VK) | 修飾 | 根拠 |
|---|---|---|---|
| GJI/Mozc 系(`ActiveImeKind::GoogleJapaneseInput`。#663 で Mozc 系もここに入る) | 0xF0〜0xF4 | Ctrl・Shift・Alt のどれか、またはその組み合わせ | Mozc のソース(`keyevent_handler.cc:330-352`)と CI(GJI の F2・F3 が none/Ctrl/Shift/Alt のどれでも開く、3/3) |
| Microsoft IME 本体(CLSID で本体と同定できたときだけ) | 0xF2 | Ctrl のみ | CI(Ctrl は開き、Shift・Alt は開かない、3/3)。0xF0・0xF1・0xF3・0xF4 は未測定なので含めない |
| 上記以外(ATOK 製品、IME 種別の未検出、MS-IME 本体の未測定のキー・修飾) | — | — | 従来どおり抑止する(`Unknown`) |

共通: **Win 付きは抑止する**(OS のショートカットが IME より先に食う可能性があり、未確認)。修飾によって届く VK が変わる(日本語 106/109 では Shift+ひらがな が 0xF1、Alt+ひらがな が VK_DBE_ROMAN/NOROMAN、
Alt+半角/全角 が 0x19、ADR-202)ので、判定は物理キーではなく**届いた VK**で行う。VK_DBE_ROMAN/NOROMAN(0xF5/0xF6)と VK_KANJI(0x19)は TSF の予約キーとして別経路で扱われるので含めない。

Alt の扱いの経緯: 初版は Ctrl・Shift だけにした。Mozc が修飾を消すのはキーが key handler に届いた後の話で、Alt 押下中の打鍵は `WM_SYSKEYDOWN` になり TIP に届かない可能性があった
(Mozc は素の F10 や Alt+` を予約キーとして登録している)ため。CI の実測(GJI で Alt を押したままの VK 0xF2・0xF3 も開く、3/3)でこの懸念は当たらなかったので、GJI/Mozc 系に Alt を加えた。
実キーボードの Alt+ひらがなは別の VK になって条件 2 で除かれるので、Alt を足しても実機のキーの挙動は変わらず、リマップした構成でだけ効く。

Shift の注意: ADR-187 の追随(`mode_key_pass.rs:257-267` `mode_key_follow_admits_modifiers`)は Shift で止まるので、読める窓での外れの訂正は `may_change_ime` の 20ms の読み直し(`key_pipeline.rs:1646-1652`)に頼る。

実装の形: `KeyTrackFacts` に `mode_key_modifiers: ModeKeyModifiers`(`Unknown` / `DroppedByGjiMozc` / `MsImeNativeCtrlOnHiragana`)を持たせ、`plan_key_effect_track` の述語で上の表を判定する。
殻は `ActiveImeKind::mode_key_modifiers(ms_ime_native_identified)` で作る。書き込みは既存の `KeyEffectPredicted`(`apply_key_effect_prediction`)を通るので、belief の規約の範囲に収まり、新しいイベントや `ObservationSource` は要らない。
最初の打鍵は F2 の KeyDown の時点で ON と予測されるので、漏れない。

**D2. MS-IME 本体・ATOK 製品・Mozc 以外の残りは、測るまで従来どおり抑止する。** MS-IME 本体は CI で 0xF2 の Ctrl だけ測れて加えた(上の表)。残りは、
`{0xF0, 0xF1, 0xF3, 0xF4} × Ctrl`、および Shift・Alt が開かない理由(MS-IME 本体は別の割り当てを持つ)を、CI の格子で閉じた状態と開いた状態について測ってから足す。セルは「届いた VK × 修飾」で定義し、素のセル(`msime-native.json`)と一致するかを見る。
MS-IME 本体では「Ctrl の KeyUp より後に F2 が処理された競合」がまだ意味を持つので、競合の有無もここで測る。

**D2'. 安全網と GJI の確認(任意の追加)。** A′ が外れた場合(GJI の閉じたバイナリが Mozc と違い、実際には開かなかった)は、既存の cold の per-VK confirm と回収(`tsf/gji_fsm.rs:532-539`、`tsf/warmup/probe_fsm.rs`)が働く。
`ImeOn` が GjiFsm に届くと cold に入って probe を張るため、追加の仕組みは要らない。ただし IME が実は OFF だった場合、回収は「t・BS・t・BS」で打ち切って「た」を消し、読めない窓では打ち切り後の追随(ADR-227)も照合できない(Opus レビューの追跡)。
GJI では、F2 の後の Converter I/O(`gji_last_write_ms`、10ms 周期)が増えたかで、予測を確定または取り消せる(閉状態の Mozc の TIP は直接入力の行にあるキーだけを Converter へ送る、`keyevent_handler.cc:680-704`)。これは A′ の確認として検討する(以下は Opus レビューの推定で、**未確認**: 読めない窓では打ち切り後の追随〈ADR-227〉が照合できず belief が ON のまま残る)が、待ち時間の定数を実測で決める必要があり(tuning-constants)、DirectInput の行が IMEOn 以外のキー(変換 = Reconvert)でも I/O が出る点に注意する。本 ADR では採るかどうかを決めず、A′ の実機確認の後に判断する。先に観測だけで入れる案もある(「予測した ON に対して、F2 の後 N ms 以内に I/O が増えたか」をカウンタとログで数える。待ち時間を判定に使わないので tuning の定数は要らず、A′ の外れ率が M1 を待たずに分かる、ADR-258 段階 0 と同じ形)。

**D3. 組成開始(候補窓の SHOW)の証拠は、観測だけの段階 0 にする(ADR-258 段階 0 と同じ形)。** belief には入れない(理由は「却下した案」)。
「belief OFF 中の SHOW」を数えるカウンタとログだけを入れ、読める窓で後の読み取りと突き合わせて誤 ON の率を測る。採るときの条件は、(i) 候補窓の持ち主のスレッド/プロセスが前面の窓と一致、
(ii) ラッチに `FocusGen` を付け世代が違えば捨てる、(iii) 利用者のモードキー(素通しした Ctrl+無変換 を含む)で閉じたときもラッチを捨てる。

**D3'(根拠が無いときの「試験的 ON」)は採らない。** 理由は「却下した案」。

**D5. MS-IME 本体の新 UI・ATOK 製品・未知の構成は、根拠による予測では最初の打鍵を救えない。同じ症状が報告されたときの順序は、まず D2 の CI 測定(測れれば A′ の条件 1 を広げるだけで済み、配送の判断に手を入れずに済む)、その次にこの入れ直し案とする。** 手段は「Ctrl を一時的に離して F2 を素のキーとして入れ直す」(`output/held_modifiers.rs`、ADR-114 決定3 の部品)だけだが、
物理キーの配送の判断(`state/physical_disposition.rs::plan_core`、BUG-46/52/116 の再発ファミリー)と actuation 合流点の許可リストに手が入るので、本 ADR では扱わず、必要になったとき別の ADR で所有者が判断する(opt-in の設定にする案)。

**D4. 検証。**
- Linux の単体テスト(`key_effect_predictor.rs`):
  (a) GJI 系で Ctrl・Shift・Alt+0xF2(と組み合わせ)→ 予測が出る。(b) MS-IME 本体で Ctrl+0xF2 → 予測が出る。Shift・Alt+0xF2、Ctrl+(0xF0・0xF1・0xF3・0xF4)→ 予測しない。(c) GJI 系で Ctrl+変換(0x1C。修飾を消さないキー)→ 予測しない。
  (d) どの IME でも Win+0xF2 → 予測しない。(e) GJI 系で Ctrl+0xF5(VK_DBE_ROMAN。範囲外)→ 予測しない。
  (f) 0xF3/0xF4 + Ctrl: GJI で役割が `ImeToggle` のとき、`shadow_action=None` → `plan_key_effect_track` が `Some` → トグルの予測が 1 回だけ出て、shadow toggle は発火しない(二重にならない)。
  (g) 0xF3/0xF4 を Ctrl 付きで Down し、Ctrl を離してから Up する。Up も Allow のまま(既存のラッチのテストがあれば参照で足りる)。
  (h) IME が ON のときの Ctrl+0xF2: 閉ループで Engine ON・IME ON から押して belief が ON のままで、開閉が反転しない。
  (i) 種別が未検出(`tsf_active_kind=0`、`MicrosoftIme` の安全デフォルト)のとき Ctrl+0xF2 で予測しない(起動直後の約 2〜4 秒の挙動)。
  (j) Shift+0xF1(Shift+ひらがな が 0xF1 として届いた場合): カタカナの素のセルが引かれる。
- 閉ループ(`bug221_*`): 判定を終わりの belief だけでなく、**最初の打鍵〈`K`〉を押した時点の `input_open` が真値と一致する**ことにする(`PredictionRecord::input_open`)。今の判定は、最初の打鍵を直さない修正でも緑になる。
  `PredictInput { unreadable: false }` の固定値は、F2 の予測には効かない(`unreadable` が効くのは 0x1C だけ、ADR-209、`key_effect_predictor.rs:1361-1365`)ので、本件の再現には関係しない。
  修正と同じ PR で `#[ignore]` を外し、判定を最初の打鍵の時点に変える(外さないと、症状を直さなくても緑のまま残る)。
- 実機: 修正後に dragonflyg4 で Ctrl+F2 の直後の最初の打鍵を確認する。

## 却下した案

- **初版 D2a(設定の明示行を読む)**: ひらがな系 4 キーでは割り当て自体が作れないので発火しない。他のキー(F1〜F10 など)の修飾付きの行は、役割を決める読み手が捨てているが、BUG-221 には関係しない。
- **初版 D2b(プリセットの修飾付きを学習で測る)**: 費用に見合わず危険。学習キーは今 14 キー(`LEARN_KEYS`)で F1〜F10 を含まない。レイアウトが修飾で VK を変えるので「物理キー×修飾」の格子は成り立たない。
  利用者の機械で修飾付きを打つと Alt+F4(学習窓を閉じる)・Ctrl+Alt+無変換(単語登録)・Alt+ひらがな(ローマ字/かな入力の切替=設定の変更)などを踏む。`KeyId` に修飾を混ぜると永続化の版・指紋・不具合報告のセル数の上限も変わる。
- **初版 D3(組成開始で belief を ON にする)**: 証拠が弱く、誤って ON にする道が既知のものだけで 4 つある。(1) 判定は GJI の候補窓のクラス名 `GoogleJapaneseInputCandidateWindow` だけ(`win_event_obs.rs:20`)で、Mozc の `MozcCandidateWindow` は見ず、MS-IME・ATOK にも無い。
  (2) `observation_event_proc` はシステム全体の窓を拾い、持ち主や前面の窓と照合しない。(3) 古いラッチが残る(`tsf/observer.rs:681-690`、ADR-213)。BUG-221 のように利用者自身の Ctrl+無変換 で閉じた場合は破棄されない(`platform.rs:693-700`)。
  (4) 候補窓の SHOW は組成開始と同じではない(サジェストを切っていれば、Space で変換するまで出ない)。`.claude/rules/ime-belief-architecture.md` は同種の素の `AtomicBool` を「弱い代理指標」と呼び、それで belief を書いたことが実機バグ 2 件の根本原因だったと記録している。
  また `StartComposition` を配る `drain_pending_composition_events` は `send_keys`/`advance_tsf_probe` の末尾(`platform.rs:756-764, 872`)でしか呼ばれず、Engine OFF の間は届かないことがある。
- **D3'(読めない窓では修飾付きモードキーの後を「不明」とし、Engine ON 扱いで最初の打鍵を送る)**: 害が「た」の漏れより大きい。GJI で IME が実は OFF だと、`t`・BS・`t`・BS の後に打ち切りとなり「た」が消え(vim・less・TUI では `t` も BS もコマンドとして実行される)、
  読めない窓では打ち切り後の追随が照合できず belief が ON のまま残って後続の打鍵も消える。Mozc(fork 以外)と MS-IME 本体には回収自体が無く `ta` が残る。閉じるキーの後にも適用すると英字が消える。
  ADR-199 決定 6-3(不明なときに能動側へ倒さない)と衝突し、`.claude/rules/ime-belief-architecture.md:21-23` の 2026-07-05 の事故(観測が無いことを根拠に belief を決めた)と同じ型になる。
- **最初の打鍵の保持(ADR-263 型)**: IME を問わず救えるが、期限の定数に実測が要り(tuning-constants)、遅延が増える。D1 で GJI/Mozc 系は足りるので、MS-IME 本体などで同じ症状が報告されたときに改めて検討する。

## 事前に測ること

- M1(読み取りのみ。D1 の確認。**CI で GJI について解決**、下の「CI 実測の結果」): 読める窓(Win32 EDIT/RichEdit、ImmCross)で、実 GJI について Ctrl を押す → F2 を押して離す → **Ctrl を押したまま 500ms 以上待つ** → IME の開閉を読む → Ctrl を離す → 読む。
  Ctrl を押している間に ON なら競合ではなく、Mozc と同じく修飾を捨てている。Windows Terminal(TSF ネイティブ)では IME の状態が読めないので、この窓では測れない。
- M1'(Alt。**CI で GJI は開く(3/3)。GJI/Mozc 系に加えた**): Alt を押したまま 0xF0〜0xF4 が届く構成(外付けキーボードやリマップ)で IME の状態を読む。結果を見て条件 3 に Alt を足すか決める。
- M2(**CI で観測、F2 は Ctrl だけ開く(3/3)。0xF2 の Ctrl を加えた。残りのキーは未測定**): MS-IME 本体の `Ctrl × 5 キー` の格子(D2)。
- M3: 利用者の `config1.db` は、`session_keymap` の値と、ADR-174 の実機観測(`session_keymap` が MSIME=2 でも表の行が効く)と ADR-186 決定(c)〈ATOK 限定〉との食い違いの再確認に限る(キーマップの行以外は読まない)。BUG-221 の結論には効かない。

## CI 実測の結果(2026-10-26、e2e-ime、`sc-adr281-*`)

スパイクの `--chord=<修飾VK>,<キー>`(閉じた状態〈`--chord-prep=1A`〉から、修飾を押したまま VK をタップ)。GitHub Windows ランナーの自前の窓(読める窓)。判定は `check_modifier_mode_key.py`。
1 回目(run 40271764639、各 1〜2 回)で傾向を見て、2 回目(run 40272737475、**16 構成 × 3 回、48 ジョブ、すべて同じ結果で INVALID なし**)で確定した。

実 IME が開くか(awase なし。「開く」は 3/3):

| IME・キー | none(対照) | Ctrl | Shift | Alt |
|---|---|---|---|---|
| GJI(MS-IME プリセット)・ひらがな 0xF2 | 開く | 開く | 開く | 開く |
| GJI(MS-IME プリセット)・半角/全角 0xF3 | 開く | 開く | 開く | 開く |
| Microsoft IME 本体・ひらがな 0xF2 | 開く | 開く | **開かない** | **開かない** |

awase 起動中(GJI の MS-IME プリセット、F2。KeyDown の `mods(` が物理の修飾付きで届くことも確認):

| 修飾 | `[key-effect-predict] vk=0xF2` | belief → ON | Engine 活性化 |
|---|---|---|---|
| none | open=Some(true) | あり | あり |
| Ctrl | open=Some(true) | あり | あり |
| Shift | open=Some(true) | あり | あり |
| Alt | 予測なし(2 回目の測定時点。広げる前の挙動) | あり(読める窓の観測の経路) | あり |

読み取れること:
- **M1(GJI)は解決**: Ctrl・Shift・Alt のどれを押しても、F2 と F3 で開く(3/3)。Mozc のクライアントが 0xF0〜0xF4 の修飾を捨てるというソースの読みと矛盾しない。
- **A′ は実 Windows で働く**: Ctrl・Shift 付きの F2 で、予測 → belief ON → Engine 活性化が起きる(3/3)。
- **M1'(Alt、GJI)**: Alt を押したままの VK 0xF2・0xF3 も IME に届いて開く(3/3)。`WM_SYSKEYDOWN` が TIP に届かないという Opus の懸念は、この注入(`SendInput`)では当たらなかった。
  実キーボードの Alt+ひらがなは別の VK(VK_DBE_ROMAN/NOROMAN)になり、A′ の VK の条件(0xF0〜0xF4)で除かれるので、Alt を足しても実機のキーの挙動は変わらない(リマップした構成でだけ効く)。
- **M2(MS-IME 本体)**: Ctrl は素のキーと同じで開く(3/3)が、Shift・Alt では開かない(3/3)。A′ を広げるなら Ctrl だけ。測ったのはひらがな(0xF2)だけで、英数・カタカナ・半角/全角(0xF0・0xF1・0xF3・0xF4)は未測定。
- この CI は「F2 の予測が最初の文字より前に起きる」ことまでを見ており、修飾付きの F2 の直後に文字を打つ手順(最初の NICOLA 打鍵が漏れないこと)は含まない。

この結果を受けて、PR #856 で A′ を広げた: GJI/Mozc 系の 0xF0〜0xF4 に Alt を加える(Win は除外のまま)。Microsoft IME 本体は CLSID で本体と同定できたときだけ、0xF2 の Ctrl を加える。

### 拡大後の再確認(e2e-ime run 40274039585、awase あり、7 構成 × 3 回、すべて期待どおり)

| IME | 修飾 | 実 IME(F2) | `[key-effect-predict] vk=0xF2` | Engine |
|---|---|---|---|---|
| GJI | none・Ctrl・Shift・**Alt** | 開く | open=Some(true) | 活性化 |
| Microsoft IME 本体 | Ctrl | 開く | open=Some(true) | 活性化 |
| Microsoft IME 本体 | Shift・Alt | 開かない | 予測なし | 活性化しない |

MS-IME 本体の Shift・Alt で誤って ON と予測しないこと(実 IME が開かないのに Engine を活性化しない)を、`--expect=nopredict` で pass の構成にした。

## 未決(所有者に確認したい点)

- Q1(解決、2026-10-26). Shift・Alt まで広げるか → Mozc のソースで 4 キーは全修飾を捨てると確認したので、Ctrl・Shift・Alt に広げる(Win は除く)。
- Q2. GJI はクローズドソースなので、M1 で修飾を捨てる挙動を 1 回確認してから有効にするか(安全側)、確認を実装後の実機確認にするか。

## 既存の前提を引き継ぐ点

- IME 種別は `active_ime_kind()`(`tsf/observer.rs:339-345`)で、起動直後と TIP の切り替え直後の約 2〜4 秒(`gji_monitor.rs:256-290` の `Debounce`)は GJI と判定されず、その間は今と同じ挙動(安全側)。
  GJI から MS-IME 本体へ切り替えた直後は逆に GJI のまま残るが、素のキーの予測も同じ表を使う既存の制約で、新しい危険ではない。値はアプリごとではなく、アプリ別に入力方式を変える設定の利用者では前面の窓と食い違いうる(既存の制約)。
- Mozc 系の利用者に対して `key_effect_keymap.get_gji` がどの `config1.db` を読むかは確かめていない(既存の前提を引き継ぐ。GJI のパスに無ければ既定の MS-IME プリセットになる)。
- 0xF3/0xF4(半角/全角)はトグルで、予測の向きは打鍵時点の belief で決まる。読めない窓で belief が既に間違っていれば予測も逆向きになる(素のキーの予測と同じ性質)。
- Ctrl 付きの 0xF3/0xF4 は `shadow_action=None` になり、物理キーは Allow、IME が自分で切り替える。awase は書かず、A′ の予測で belief が追随するだけで、書き込みと予測の二重にはならない。
- 別件(本 ADR では直さない): 0x19 の Ctrl/Shift/Win 付きを受け身にする `kanji_role_plan`(ADR-202 決定3「別意味を持ちうる」)は、Mozc の TIP が VK_KANJI を `TF_MOD_IGNORE_ALL_MODIFIER` の予約キーとして登録している事実(`tip_text_service.cc:331-339`)と食い違う。Ctrl+0x19 は Ctrl+Alt+半角/全角 でしか届かず実害は小さい。

## 影響

- `ime-belief-architecture`: A′ は既存の `KeyEffectPredicted` を通るので規約の変更は要らない。D3 を将来 belief に入れるときは、新しい `ObservationSource` の variant、`state/evidence.rs` の宣言表の行、`architecture_guard` の件数の更新が要る。
- 抑止を前提に書かれた doc の追随: `crates/awase-ime-core/src/state/mode_key_pass.rs:262, 483`、`crates/awase-windows/src/runtime/key_pipeline.rs:1816`、`key_effect_predictor.rs` の `plan_key_effect_track` の doc(`:866`)・コメント(`:878`)・`:1129`。
- `fix-requires-evidence`: IME belief・キー選択の再発ファミリー。BUG-221 の再現テストが (a) に当たる。判定を最初の打鍵に直す(D4)。
- 時間の窓(ラッチの期限・保持の期限)を `tuning.rs` に足す案は採っていないので、tuning-constants の実測義務は発生しない。
