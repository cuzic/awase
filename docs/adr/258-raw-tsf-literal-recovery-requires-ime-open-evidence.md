---
id: ADR-258
title: |-
  raw TSF リテラル回収に「IME が開いている」証拠を要求する(BUG-200)
summary: |-
  BUG-200: IME が実際は OFF で awase の認識が ON のとき、TsfNative の窓で打った文字が raw TSF リテラル回収の Backspace で消える(実機で人工再現、tsf 形 4/4 で本文が空、IMM の edit 形は英字が残る)。
  回収は「GJI の動きの証拠が無い」ことだけで literal と判定するが、IME が OFF のときも証拠は出ないので区別できない。TsfNative の窓は実 IME の開閉を読めない(himc_null)。
  決定は2段: D0 で TSF の開閉を実際に読めるかを実機で測った(結果: 別プロセスの窓の開閉を読める手段は無い)。よって D2(give-up の Backspace だけの経路を止める)が主たる措置。D1(閉を観測したら回収せず journal に残す)は読み出し口が見つかったときの上乗せ。
status: |-
  起草(2026-10-12)。D0 の (a)(b)(c) を測定し、D1 は採れず D2 が主たる措置と暫定判断(同日)。実装なし。Opus レビュー未実施(D2 の実装前に回す)。
related_adr:
  - "ADR-246"
  - "ADR-212"
  - "ADR-100"
  - "ADR-188"
---

# ADR-258: raw TSF リテラル回収に「IME が開いている」証拠を要求する(BUG-200)

## 背景

所有者の報告(2026-10-12): 「IME OFF・エンジン ON のとき、半角英数を入れると、その英数が消された」。自然発生は再現しなかったが、
[BUG-200](../known-bugs/BUG-200.md) のとおり実機で人工的に再現でき、同じ時間帯の `awase.log` に同じ回収の警告が残っていた。

## 事実(`origin/develop` `7812c9df` の開発ビルド、dragonflyg4、GJI、2026-10-12)

- **再現**(各 n=4): `typing_stress --form=tsf --mode=drift-on`。IME を ON にそろえ、ハーネスが awase を経由せず実 IME を閉じる(`WM_IME_CONTROL`)。
  2 秒待っても `real_ime_open=false`(awase は ON に戻さない)。NICOLA 単打を1回打つと、本文が空(4/4)。同じ手順の `--form=edit`(IMM)は本文 `w`(英字が残る、4/4)。
- **ログ**(13:28:19Z、`target\debug\awase.log`): `[raw-tsf-literal] ... raw TSF literal suspected → backspace ×1 + re-send "ka" scheduled + mark cold`、
  続けて `consecutive raw-tsf-literal (count=2..5) → giving up, backs=1 cleanup only (no re-send)`。
- **判定の仕組み**(`output/probe_io.rs` の `ProbeAction::RawTsfLiteralRecovery` アーム、`tsf/literal_facts.rs::DetectEvidence`): 1打ごとに GJI の動きの証拠
  (`show_changed`・`candidate_visible`・`write_delta` など)を待ち、無ければ literal と判定する。`consecutive == 0` なら Backspace ×`backs` + romaji 再送、
  `consecutive != 0`(give-up)は **Backspace だけ(再送なし)**。
- IME が実際に OFF のときも、GJI の動きは無いので、証拠が無い点は「cold-start で literal 化した」ときと**区別できない**。
- TsfNative の窓は `himc_null=true`(`[comp-probe]` ログ)で、IMM では実 IME の開閉を読めない。
- 自然発生の窓ごとの内訳は未確認。同日 22:14(JST)の手動確認の `awase.log` に同じ警告が Chrome(`Chrome_RenderWidgetHostHWND`)と
  Windows Terminal(`Windows.UI.Input.InputSite.WindowClass`、TsfNative)で出ているが、所有者の消えた文字との対応は取れていない。
- 関連の既存決定: ADR-246 は回収の**宛先の窓**の照合だけを足した(IME の状態は見ない)。ADR-212 P3 は give-up の reinit(VK_IME_OFF→ON)を撤去し、
  Backspace だけを残した。ADR-100 決定3 は give-up で romaji が失われることを journal に残す。

## 問題

回収は「IME が ON なのに literal 化した」場合のための処理だが、**IME が実際に OFF のとき**にも発動する。そのときの Backspace は、
ユーザーが打った(OFF なので本来そのまま出てよい)英数を消す。awase の認識が実状から外れる(IME を awase 以外が閉じた)ときにだけ起きるが、
外れた状態を awase は 2 秒以上続けて ON のままにする(`drift_on_check` で戻らない)ので、窓は短くない。

## 決定

### D0: 測る(最初。実装の前に)

TsfNative の窓で、**IME を閉じたときに、実 IME の開閉を観測で読めるか**を実機で測る。読み出し口の候補:
(a) TSF の `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`(`tsf/observer/`、compartment の通知は 2〜5ms で届くという既存の実測)、
(b) `ImeStateHub` の直接観測(ADR-188 の窓の中の読み)、(c) 既定 IME ウィンドウへの `IMC_GETOPENSTATUS`(`startup_typed.real_ime_open_before_type` が使う読み)。
測る対象は `tsf` 形と実 Chrome(`chromepage`)、IME は GJI(と、可能なら MS-IME)。IME を閉じる方法は、ハーネスの `WM_IME_CONTROL`(今回)と、物理の半角/全角キー。
記録するもの: 閉じてから各読み出しが `false` になるまでの ms、`Some(false)` と `None`(読めない)の別、読みが実際の状態と食い違った回数。
**API の読みだけで状態を断定しない**(実タイピングの結果と突き合わせる)。

**D0 の途中結果(2026-10-12、dragonflyg4、`typing_stress --form=tsf --mode=drift-on`、GJI、n=4 の1走行分の `awase.log`、13:28:14〜13:28:21Z)**:

- (b) awase の観測(`[external-change]`、`ObserverPoll`): ハーネスが物理の `VK_IME_OFF`/`VK_IME_ON` を押したとき(13:28:14.28 と 14.95)は、押下の約 26〜40ms 後に
  `open=false`/`open=true` を観測し、`Engine deactivated`/`activated` に切り替えた。**awase を経由しない直接 close**(13:28:16.458、`WM_IME_CONTROL`)のあとは、
  2 秒以上 `[external-change]` も `Engine deactivated` も出なかった(`drift_on_check` の 500ms・2000ms の時点でも awase は ON のまま)。観測はキー駆動で、awase の外からの閉じ方には追随しない。
- (c) `IMC_GETOPENSTATUS`(ハーネス側の読み)は `false` を返した(`real_ime_open=false`)が、これは閉じた手段(`WM_IME_CONTROL`)と同じ経路の読みで、実際の状態の独立な確認ではない。
  awase 側は、この窓(`Chrome_RenderWidgetHostHWND`、`profile=Imm32Unavailable`、`himc_null=true`)では IMM の読みを使わず、回収の判定時のログも `open=-`(読んでいない)。
- (a) TSF の `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`: **未測定**(下の「残りの D0」)。

結論(暫定): 「観測が閉を示せば回収しない」(D1)は、現状の観測が awase の外からの閉じ方に追随しないため、そのままでは効かない。D1 を採るなら、回収の判定時に開閉を読み直す
読み出し口((a) か、TsfNative でも使える別の読み)が要る。読み出し口が決まるまで、D2(give-up の Backspace を止める)が、観測に依らず取れる最小の措置。

**D0 (a) の測定(2026-10-12、dragonflyg4、GJI、`compartment_notify_probe --seq=16,close,open,close,1A,16,close --richedit`、awase 停止、n=1 走行・close 3回)**:
`close`/`open` は既定 IME ウィンドウへの `WM_IME_CONTROL(IMC_SETOPENSTATUS)`(awase も TSF も経由しない閉じ方)。

- **スレッド compartment**(`KEYBOARD_OPENCLOSE`、窓を持つスレッドで読む): `close` に 0〜1ms で通知(`OPENCLOSE=0`)、`open` に 0〜1ms で `=1`、50ms ポーリングで 20〜51ms。IME ON キー(`0x16`)は 1〜6ms。
  awase の外からの閉じ方にも追随する。**ただし窓を持つスレッドの中でしか読めない**(`ITfThreadMgr` はスレッド単位。awase のプロセスから、別プロセスの Chrome のスレッドの値は読めない)。
- **大域 compartment**(`ITfThreadMgr::GetGlobalCompartment` 経由、`KEYBOARD_OPENCLOSE`): 取得は成功し、初期値 `Some(0)`。close・open・IME ON キーのいずれでも**一度も変化しなかった**(`GPOLL` が0件)。実際の開閉を反映しない。
- したがって、awase のプロセスから別プロセスの TsfNative の窓の開閉を TSF の compartment で読む手段は、この測定の範囲では**無い**。

**D0 の結論(暫定)**: D1(閉を観測したら回収しない)は、TsfNative の窓で使える信頼できる読み出し口が無いため、現状では採れない。
(c) の `IMC_GETOPENSTATUS` は、ハーネスの読みで `false` を返したが、閉じた手段と同じ IMM の経路で、実際の状態との独立な照合は取れていない(Chrome で信頼できるかは未測定)。
awase が `Imm32Unavailable` の窓で IMM を読まない設計判断は、その信頼性への疑いに基づく。以上から**主たる措置は D2**(観測に依らず、取り返しのつかない削除を避ける)とし、
D1 は将来、信頼できる読み出し口が見つかったときの上乗せとして残す。

**残りの D0**: (1) (a) の compartment を、`tsf` 形(自プロセスの窓)で、閉じた直後・ON に戻した直後に読む(`typing_stress` に読み出しを足す。`ITfThreadMgr` はスレッド単位のため、窓を作った UI スレッド側で読む)。
(2) 回収の判定の時点で awase が何を見ているかを知るための診断ログ(`RawTsfLiteralRecovery` アームの直前に、観測の値と古さを `info` で出す。挙動は変えない)。
(3) 実 Chrome で、閉じた直後に実際に打って英字が出る(= 本当に OFF)ことを確かめる(awase なし、`--no-awase`)。

### D1(D0 で閉を読めたとき): 閉を観測したら回収しない

`RawTsfLiteralRecovery` の実行前に、新しい(窓の長さ以内の)観測が `open=false` なら、Backspace も再送もせず、journal に
`Skipped(ImeClosedObserved)` を残す。観測が `None`(読めない)のときは現行どおり回収する(読めない窓でのこれまでの挙動を変えない)。
判断は純粋関数にする(`LiteralDetectFsm` の `emit_recovery_actions` の入力に「観測の開閉」を足す。`output/probe_io.rs` は Facts を渡すだけ)。

### D2(D0 で閉を読めないとき): give-up の Backspace だけの経路を止める

`consecutive != 0`(give-up)の「Backspace だけ(再送なし)」は、正しく literal 化していた場合も文字を失うだけで回復しない(再送が無いため)。
IME の状態が分からないなら、**取り返しのつかない削除を避ける**側に倒し、give-up では Backspace を送らず journal に残すだけにする。
`consecutive == 0` の1回目(Backspace + 再送)は変えない(再送で本文が戻るため、IME が OFF でも英字が重なるだけで文字は失われない見込み。D0 の記録で確かめる)。

### 非目標

- awase の認識が実状から外れたままになること自体(drift correction が TsfNative で ON に戻さない件)は、本 ADR では扱わない(ADR-188・ADR-212 の領域)。本 ADR は「外れたときに文字を消さない」だけ。
- 回収そのものの撤去(ADR-212 の方向)は、Chrome の cold-start literal(BUG-002 系)への影響を測っていないので扱わない。

## 検証計画

1. D0 の実測(上)。結果を本 ADR に追記して D1/D2 を選ぶ。
2. 回帰テスト(`fix-requires-evidence.md` の warmup/belief 系): 純粋関数の単体テスト(`literal_detect_fsm.rs`、観測 `Some(false)`・`Some(true)`・`None` × `consecutive` 0/1以上 の表)。
3. 実機/CI: `typing_stress --form=tsf --mode=drift-on` を使う。修正後の期待は「本文が空でない(英字が残る)」かつ `awase.log` に `backspace ×` が出ないこと(D1)。
   現状の失敗(本文が空、4/4)が回帰の対照になる。CI の `e2e-ime.yml` への構成追加は、D0 で読み出し口が決まってから。
4. 副作用の確認: cold-start の literal 回収(BUG-002 系、`sc-` の Chrome 構成)が従来どおり働く(`CompositionConfirmed` の割合が変わらない)。

## 未決(Q)

- Q1: D0 の読み出し口(a)(b)(c)のどれが、TsfNative の GJI で実際の開閉に追随するか。追随しない(TIP が実状を反映しない)読み出し口は採らない。
- Q2: `consecutive == 0` の1回目の Backspace + 再送は、IME が OFF のとき本当に本文を変えないか(再送が literal として戻るだけか)。D0 の実機記録で確かめる。
- Q3: 実 Chrome・Windows Terminal・MS-IME で同じ消え方をするか(今回は GJI の `tsf` 形だけ)。
- Q4: 所有者が自然発生で見た消え方は、本件と同じか。次に起きたら不具合報告(ADR-095)を取る。

## 出典

- [BUG-200](../known-bugs/BUG-200.md)(再現手順・ログ)。
- `crates/awase-windows/src/output/probe_io.rs`(`RawTsfLiteralRecovery` アーム)、`crates/awase-windows-core/src/tsf/literal_facts.rs`、`crates/awase-windows/examples/typing_stress/main.rs`(`--mode=drift-on`)。
