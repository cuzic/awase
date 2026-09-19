---
id: ADR-186
title: |-
  GJI(ATOKプリセット)のモードキー動作を実機で全数測定し、awaseは押下時点でbelief(open軸/かな英数軸)を追随させてEngineを「かな=ON・英数(直接入力含む)=OFF」に保つ
summary: |-
  ADR-184/185の議論は「ATOKの無変換はIME ONのまま半角英数に変える」という前提に立っていたが、
  awase非依存のスパイク(`crates/awase-windows/examples/ime_key_matrix_spike.rs`)で
  素のWin32 EDITとRichEdit 5.0(TSFネイティブ)の2コントロールを同一手順で測った結果、
  **公開Mozcの`atok.tsv`がそのまま実機GJIの動作**だった(2コントロールの全セルが一致)。
  無変換/変換/半角全角はIME開閉のトグル(ON中→OFF、直接入力→ON)、ひらがなキーと
  Shift+無変換と「入力中の無変換」はIME ONのままかな⇔半角英数のトグル。
  「ひらがな中に無変換→IME ON半角英数」という観測は、実際はIME OFF(直接入力)だった可能性が高い
  (どちらも英字が出るため見分けがつかない)。ユーザー要件は「かなのときEngine ON、英数のとき
  Engine OFF」。現状はキー押下でbeliefが動かず、遅延観測(`idle-conv-check`、次の打鍵後)でしか
  Engineが切り替わらないため、押下直後の1打がNICOLAのまま処理される。本ADRは実測表を根拠として固定し、
  押下時点のbelief追随(open軸は既存のToggle経路、かな英数軸は`apply_input_mode_correction`)で
  この遅延を埋める最小変更を定める。
status: |-
  **ドラフトv1(実測完了・設計は未レビュー)**。実装前にopus敵対レビューを通すこと。
related_adr:
  - "ADR-090"
  - "ADR-176"
  - "ADR-179"
  - "ADR-184"
  - "ADR-185"
---

# ADR-186: GJI(ATOK)モードキーの実測表と、押下時点のbelief追随

## 背景

ユーザー要件: **かなのときEngine ON、英数のときEngine OFFを徹底する**。ここで英数は
「IME ONの半角英数」と「直接入力(IME OFF)」の両方を指す(awaseの挙動はどちらでも同じ=Engine OFFで
よい、ユーザー確認済み)。

現状の弱点: 無変換などの押下は、Passthrough設定では生キーとしてGJIに渡されるだけで、awaseの
beliefは動かない。Engineが切り替わるのは、次の打鍵の後に`idle-conv-check`が変換モードを読んで
`ObservedEisu`と判定したとき(遅延観測)。押下直後の最初の1打はEngine ONのままNICOLAで処理される
(2026-09-19の実機ログ: `Kana/roma → Eisu/roma (source=IdleCheck)`が押下の後の打鍵後に出る)。

これまで「ATOKの無変換はIME ONのまま半角英数にする」を前提に議論していた(ADR-184の症状節、
ADR-185)。この前提を実測で検証した。

## 実測

- 環境: dragonflyg4、Google日本語入力、`session_keymap = 1`(ATOK、`config1.db`をデコードして確認。
  オーバーレイなし。同ファイルに残るMS-IME風の`custom_keymap_table`はプリセット選択時は
  読まれない)、awase停止。
- 方法: awase非依存のスパイクアプリが`WH_KEYBOARD_LL`でキーを捕捉し、押下前・+400ms・+1500msの
  観測値を並べて記録(A=`ImmGet*`、B=`WM_IME_CONTROL`、T=TSFスレッドcompartment、
  G=TSFグローバルcompartment)。状態は変えず、案内に従ってユーザーがキーで作る。
  観測値A/B/Tは全件一致(Gは常に0で不採用。ADR-176の手法Cは`GetGlobalCompartment`という
  スコープの誤りだった)。
- 対象コントロール: 標準EDIT(IMM)とRichEdit 5.0(TSFネイティブ)。**全セルで挙動が一致**。
- 生ログ: `186-measurements/`(`round1-edit.log`、`round2-richedit.log`、探索時の
  `adhoc-unguided-run.log`)。

### 結果(ATOKプリセット)

| 状態 | 無変換 | 変換 | ひらがな(0xF2) | 半角/全角 |
|---|---|---|---|---|
| 直接入力 | IME ON | IME ON | 変化なし | IME ON |
| ON・かな・入力なし | **IME OFF** | **IME OFF** | ONのまま かな→半角英数 | IME OFF |
| ON・入力中(未確定あり) | ONのまま半角英数へ(未確定保持) | 変換(候補) | ONのまま かな⇔半角英数 | IME OFF(未確定破棄) |
| ON・半角英数・入力なし | **IME OFF** | **IME OFF** | かなに戻る | 未測定(OFFと推定) |

- Shift+無変換: ON中は、かな⇔半角英数のトグル。
- 直接入力→ONにしたときのconvは、どのキーでも0x19(ひらがな)に戻る。
- 未測定: 英数キー(0xF0、この環境に物理キー無し)、Shift+ひらがな(0xF1、カタカナ)のON中の効果、
  ON・半角英数での半角/全角。ATOKの`atok.tsv`にこれらの行が無いことから、効果なしと推定。
- 上流`atok.tsv`(Mozc master `13c98988`)との照合: 全セルが一致した。Windowsのキー→Mozcキー名の
  対応は`win32/base/keyevent_handler.cc`(0x1C=HENKAN、0x1D=MUHENKAN、0xF2=KANA(`"kana"`/`"hiragana"`
  はどちらも`KeyEvent::KANA`)、0xF3/0xF4=HANKAKU)。IME OFF中にMozcへ届くのは`DirectInput`行の
  キーのKeyDownだけ(`keyevent_handler.cc:680`)。

## 前提の訂正

1. **「ひらがなからの無変換単独打鍵はIME ONのまま半角英数になる」は誤り**。実測ではIME OFF。
   過去の観測は、どちらも英字が出るためIME OFFと区別できていなかった可能性が高い。
   (`gji_thumb_key_ime_toggle=true`で「無変換で半角英数にならない」と見えたのも、awaseが開閉トグルとして
   IME OFFを送っただけで、この表どおりの動作。)
2. **「直接入力からの無変換は何も起きない」は、素のWin32/RichEditでは成立しない**(ONになる)。
   メモ帳・Windows Terminalでの観測との差は未解明(本ADRの未解決事項)。
3. `classify_mode_key_ime_action`のATOK分類: 無変換/変換=`Toggle`(開閉トグル)は**正しい**。
   ひらがな=`None`(開閉に影響しない)も正しい。ADR-184議論中の「ATOK判定が誤分類」という
   疑いは撤回する。
4. ADR-185(半角英数を検出しても、awaseからIME OFFを送らない)は、この表と矛盾しない。
   撤去したのは「convからopen軸を推測して書き・送る」こと。キー押下に基づくbelief追随とは別。

## 決定

**決定1 — この表を一次情報として固定する。** 本ADRと`186-measurements/`を、ATOKプリセットの
動作の根拠とする(公開`atok.tsv`と一致することも確認済み)。ADR-184の症状節の前提は本ADRで訂正する。

**決定2 — open軸トグル(無変換/変換/半角全角、入力なし)は、既存のToggle経路を使う。**
ADR-179決定2の`resolve_delegate_to_open_axis`のToggle分岐(awaseが自分のbeliefに従って
ON/OFFを明示actuate)を、ATOKプリセットのPassthrough設定で有効にする(ADR-184の配線)。
これで押下時点でopen軸のbeliefが動き、Engineが即座に切り替わる。ユーザーの原則
(トグルはawaseが自分のbeliefに従ってactuate、冪等キーはbelief追随のみ)と一致する。

**決定3 — かな英数軸のトグル(ひらがな、Shift+無変換)は、押下時点でinput_mode beliefを予測反転する。**
IME ONのとき、かな系(`ObservedRomaji`/`ObservedKana`/`AssumedRomaji`)⇔`ObservedEisu`を、
既存の`apply_input_mode_correction`で押下時に書く(`InputModeApplyStrategy`に「かな英数トグルキー」
を1つ追加)。予測が外れたときは、既存の`idle-conv-check`の受動観測が訂正する(安全網は今のまま)。
IME OFFへの操作は一切しない(ADR-185の原則)。

**決定4 — 「入力中の無変換」は開閉トグルとして扱う。**
awaseは入力中かどうかを確実には見られない。入力中の無変換はGJIではかな英数トグル(ONのまま)だが、
awase側では開閉トグルとして扱うと、どちらでもEngine OFFになる点は一致する。差(IMEがOFFか
半角英数ONか)は、次の遅延観測が訂正する。

## 非決定(やらないこと)

- 新しい型・軸の区別・ウィザード連携は作らない(ADR-184の方針)。
- 英数キー(0xF0)・カタカナ(Shift+ひらがな)は、未測定のため対象外。
- MS-IMEプリセットなど他プリセットの表は測っていない(本ADRの対象外。必要なら同じスパイクで測る)。
- 変換キーは無変換と同じ扱い(表の全セルで同じ結果)。

## 検証計画

1. 実機A/B(メモ帳・Windows Terminal、awase起動・debug): 無変換/ひらがな/半角全角の押下から
   Engine状態の変化までの時間を、ログの押下時刻とEngine activated/deactivatedで測る(現状=次の打鍵後)。
2. ゴールデン/ユニットテスト: 上の表の各セルについて、期待するbelief遷移を`state/`の純粋関数
   または`src/engine/tests.rs`に固定する。
3. `fix-requires-evidence`の再発ファミリー(IME belief/キー選択)に該当するため、回帰テストを必ず添える。

## 未解決事項

- メモ帳・Windows Terminal(TSFネイティブ)での実測: スパイクではRichEditまでしか測れていない。
  「直接入力で無変換が効かない」観測との差を、awaseのdebugログか専用のTSFテキストサービス
  相当で確認する必要がある。
- 決定2で生キーを消費するか素通しにするか(既存のToggle分岐の挙動)は、実装時に確認して本ADRへ追記する。
- 入力中の無変換の扱い(決定4)が、実運用で問題になるか。
