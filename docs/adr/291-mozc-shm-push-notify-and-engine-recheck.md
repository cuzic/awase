---
id: ADR-291
title: |-
  belief_v2(候補Z)にmozc-shm追随の手放しを反映させ、Engineの活性再評価を明示的に呼ぶ(BUG-231)
summary: |-
  ADR-290/BUG-230適用後のdragonflyg4実機(2026-11-11)で、ひらがなキー押下からEngine活性(Active)まで
  最大約5.3秒ローマ字がリテラルで漏れる新しい遅延([BUG-231](../known-bugs/BUG-231.md))を観測した。
  当初「Engineの活性再評価が物理キーで走らない」と誤診したが、Opus敵対的レビューround1で訂正:
  実際には毎物理キーが`Engine::on_input`→`check_active_transition`を再実行していた。本当の原因は
  `general.belief_v2`(既定true、ADR-282)下でEngineが読む候補Z(`scope_shadow`)の変換表
  (`scope_belief.rs::scope_event_of`)が、`ir_do_abandon_half_width_alnum_from_mozc_shm`が書く
  `InputModeApplied{UserTurnOnEisuReset}`を`_ => None`として無視し、Zが英数(Eisu)の主張のまま
  取り残されること(反映の欠落、観測で約3.7秒。ログの`[belief-v2]`食い違い行が直接の証拠)。
  round2レビューで、最初の修正案(mozc-shmの観測を素朴に`InputModeObserved{mode, MozcShm}`として
  Zへ書く)が**ADR-290自身が最初の実装で試して致命的と判定し却下した設計(D1′ F1: `Hiragana`を
  `ObservedKana`として書くとEngineが永久に`Inactive(NotRomajiInput)`のまま戻らない)をそのまま
  再現する**ことが発覚し、設計を訂正: モードの値を必ずromaji-capableな`AssumedRomaji
  {UserHalfWidthAlnumToggleOff}`に固定し、既存の`InputModeApplied{UserTurnOnEisuReset}`書き込み
  (旧モデル向け、置き換えずに維持——`belief_v2=false`環境の回帰を防ぐ)と**両方**を新設の専用
  メソッドから無条件にdispatchし、直後に`notify_engine_refresh()`を呼ぶ(D1′)。
  もう1つの原因(mozc-shmの2段ラッチが`TIMER_MOZC_SHM`500msポーリングのみに依存、検知遅延
  約1.6秒)については、round2で効果を「大半がMozc自身の切替時間でD2では縮まらない」と
  下方修正し、実測してから着手を判断する保留(D2)とした。D3(かなキー押下時のbelief即時楽観補正)は
  却下、D4(確定まで打鍵を保持)はADR-290自身の既存の見送り(D2依存)と同じ理由で見送り。
status: |-
  起草(2026-11-11)。Opus敵対的レビューround1(原因診断の誤り、F1)・round2(修正案自体がADR-290の
  既に却下済みの設計を再現する指摘、G1/G2)を反映して訂正済み。round3確認待ち、実装・実機確認とも
  未着手。
related_adr:
  - "ADR-290"
  - "ADR-282"
  - "ADR-244"
  - "ADR-268"
  - "ADR-270"
  - "ADR-265"
---

# ADR-291: belief_v2(候補Z)にmozc-shm追随の手放しを反映させ、Engineの活性再評価を明示的に呼ぶ

## 背景

[ADR-290](290-half-width-alnum-exit-via-mode-key-and-gji-confirm.md)/[BUG-230](../known-bugs/BUG-230.md)で、
半角英数トグルをひらがなキー(F2)で抜けたときに`toggle_held`が解除されない問題を、
mozc-shm(mozc-readableの共有メモリ)の観測ベースの2段ラッチで修正し、developへマージ・
dragonflyg4へ投入した(PR #933、commit `e2f438871`)。

2026-11-11、dragonflyg4の実機ログで、この修正そのものが持つ新しい遅延を観測した
([BUG-231](../known-bugs/BUG-231.md)に詳細)。ユーザーからは「もっとmozc-readableからawaseに
即時のイベントを通知することはできないか、もしくはひらがなキー押下後のbeliefを適切にするとか」
という2方向の改善案が提案された。本ADRはログの実測とソース読みでこの2案(および気付いた3つ目の
独立原因)を検証し、採否を決める。

## 観測した事実(dragonflyg4、2026-11-11T05:47:16〜05:47:24Z、BUG-231より)

| 時刻(UTC) | 事象 |
|---|---|
| 05:47:16.733 | 左Shift単独タップ→半角英数トグルON |
| 05:47:16.869 | `Engine deactivated (reason=Inactive(NotRomajiInput))` |
| 05:47:18.911 | ひらがなキー(vk=0xF2)押下1回目、PassThrough |
| 05:47:20.544 | (+1.63秒) `[shift-conv-guard]` mozc-shmの観測でかなへの追随を確認→トグルを手放す。`apply_input_mode_correction`実行。**旧`ImeModel`はromaji-capableへ補正されるが、Zは英数のまま**(下記原因2) |
| 05:47:20.544〜24.195 | バックスペース×4・T×2・Space・Enter等、複数の物理キー押下。**いずれも`Engine::on_input`→`check_active_transition`を実際に再実行していた**(当初「再実行しない」と誤診、Opusレビューで訂正)。再チェックした先のZが英数のままだったので非活性が続いた |
| 05:47:24.195 | ひらがなキー押下2回目(ユーザーの再押下)。打鍵時予測が`scope_shadow_key`でZを直接kanaへ動かす |
| 05:47:24.231 | (手放しから+3.65秒) `Engine activated (reason=Active)`(直後に発行された`notify_engine_refresh()`で検知) |

1回目のかなキー押下(18.911)からEngine活性(24.231)まで**合計約5.32秒**。このうち約1.63秒が
mozc-shm観測の検知遅延(大半はMozc自身の切替時間)、約3.65秒がbelief_v2(Z)への反映の欠落という、
独立した2つの原因で構成される。

## 検討した案

### 案A′(採用・D1′): 手放し処理でZが受理する形でモード軸を書き、かつ`notify_engine_refresh()`を呼ぶ

当初「`ir_do_abandon_half_width_alnum_from_mozc_shm`が`notify_engine_refresh()`を呼んでいない
から、次の物理キー/フォーカス変更までEngineに伝わらない」と考えた(案A)。これはOpus敵対的
レビューで**factとして誤り**と指摘された: `key_pipeline.rs`の特殊キー以外は全て
`Engine::on_input`を通り、`on_input_body`のPhase2が非活性の早期returnより前に
`check_active_transition(ctx)`を呼ぶ。実際、BUG-231のBS×4・T×2・Space・Enterはすべて
`check_active_transition`を再実行していた。

**本当の原因**: `general.belief_v2`(既定`true`、[ADR-282](282-per-scope-latched-ime-belief-zero-base.md)
V2-3、所有者決定O8)が真の間、Engineが読むbeliefは`platform_state.rs::engine_belief()`経由で
候補Z(`scope_shadow`)。`crates/awase-ime-core/src/state/scope_belief.rs::scope_event_of`は
`ImeEvent::InputModeApplied`のうち`strategy`が`UserHalfWidthAlnumToggle`・`PanicReset`のものだけ
を`ScopeEvent::Explicit`としてZへ写し、**それ以外(`UserTurnOnEisuReset`を含む)は`_ => None`で
無視する**(ADR-268がmozc-shmの open軸だけをZへ接続した設計の裏返しで、mode軸はこれまで
Zへ届く経路が無かった)。`ir_do_abandon_half_width_alnum_from_mozc_shm`が書く
`InputModeApplied{UserTurnOnEisuReset}`はこの穴に落ち、旧`ImeModel`はromaji-capableへ補正
されるのに、Engineが実際に読むZは英数のまま取り残される。2回目のかなキー押下の打鍵時予測
(`scope_shadow_key`)がZを直接動かすまで、Engineの再評価は毎回「正しく再実行されるが、古い
入力を読んでいるので結果が変わらない」状態が続いた。

**対比(なぜADR-244 D3は同じ穴に落ちないか)**: `ir_follow_direct_mode_key_read`
(ADR-244 D3、直接観測による追随)は`InputModeObserved`を書く。これは`scope_event_of`で
`ScopeEvent::Observed`に写り、Zが受理する。mozc-shm経由の手放しだけが`InputModeApplied`
(Zの変換表の対象外の`strategy`)を使っていたため、この穴に入った。

**同じ穴を持つ既知の箇所(今回のADRでは修正対象にしないが記録する)**: [BUG-222](../known-bugs/BUG-222.md)の
`kp_exit_lang_detect_toggle_on_hiragana`(`key_pipeline.rs`)も`UserTurnOnEisuReset`を書くため、
belief_v2が真の環境では同じ理由でZに伝わらない可能性が高い(未検証、別途確認が必要)。
一般化すると、**`{UserHalfWidthAlnumToggle, PanicReset}`以外の`InputModeApplyStrategy`はすべて
Z配下でEngineに無視される**。これはADR-282自身の設計上の既知の穴であり、本ADRの範囲を超える
ため、ADR-282の追補課題として別途記録することを推奨する(本ADRでは直接の修正対象(手放し経路)
のみ扱う)。

**修正の方向、round2レビューでさらに訂正(重要: 単純に`InputModeObserved{mode, MozcShm}`を
書くだけでは、ADR-290が既に試して致命的と判定した設計をそのまま再現する)**:

`architecture_guard.rs::input_mode_observed_construction_sites_are_accounted_for`のコメントに
「ADR-290(BUG-230)はこの軸にInputModeObservedを構築しない設計(D1″、Opusレビュー後)に
落ち着いた」と明記されているとおり、**ADR-290自身が最初の実装(D1′、「第1版」)でまさに
`InputModeObserved{source: MozcShm}`をbeliefへdispatchする設計を試し、Opus敵対的レビューで
2件の欠陥により却下している**(`docs/adr/290-*.md`「却下した案」節):

- **ADR-290 D1′ F1(致命的)**: Mozcの`CompositionMode::Hiragana`を、素朴に
  `InputModeState::ObservedKana`(「JISかな直接入力と確認できた」、`is_romaji_capable()`が
  `false`、ADR-091決定2でサポート対象外)として書いていた。`Hiragana`はローマ字テーブル入力
  (NICOLAの前提)とJISかな入力を区別できないため、手放しに成功するたびにbeliefが
  `ObservedKana`になり`engine.rs::compute_state`が`Inactive(NotRomajiInput)`を返し続け、
  **BUG-230を直すはずの変更がむしろ恒久的に悪化させていた**。
- **ADR-290 D1′ F2(高)**: この観測が`report_mozc_shm`内の`decide_use`のHeld/Split判定を
  素通りしていたため、トグルに入った直後、共有メモリにまだ残っているエントリ**前**の古い
  `hiragana`値を「かなへの追随」と誤認し、入った直後にトグルを即座に手放す経路があった。

**本ADRが提案する書き込みがこの2件に当たらない理由(必ず明記すること)**:
- F1対策: モードの値を`ObservedKana`では**なく**、[ADR-244](244-bug186-msime-chrome-persistent-toggle-follow.md)
  D3の前例(`follow_direct_read_in_scope`、`platform_state.rs:904-918`、非英数側で
  `InputModeState::AssumedRomaji { reason: AssumedReason::ImmBridgeBroken }`を書く)と同じ形で、
  **`InputModeState::AssumedRomaji { reason: AssumedReason::UserHalfWidthAlnumToggleOff }`
  (既存のreason、`src/engine/mode_state.rs:37`)に固定する**。これはromaji-capable(`is_romaji_capable()`
  が`true`)なので、Zの`mode_class_of`は`RomajiInputMethod`系になり、Engineが
  `Inactive(NotRomajiInput)`に落ち込まない。「Mozcが実際にかな入力方式(ローマ字でなくJISかな)
  だったらどうするか」は、v5共有メモリブロックの`prefer_kana_input`フィールド
  (`crates/awase-ime-core/src/state/mozc_shm.rs:197,209`)を読めば区別できるが、現在
  `mode_read`は`(CompositionMode, update_tick_ms)`しか運んでいない(`mozc_shm_shadow.rs:89-90,160`)
  ため、安い拡張でなければ今回は見送り、常に保守的な`AssumedRomaji`を書く。
- F2対策: 本ADRの書き込みは`report_mozc_shm`の毎tickの`decide_use`には一切相乗りせず、
  既に実装済みの2段ラッチ(`plan_mozc_shm_follow`が`Abandon`を返した、そのtickのみ)でしか
  発火しない。したがってエントリ前の古い`hiragana`値を誤認するF2の経路は生じない。

**G2(新規): (a)がInputModeApplied書き込みを置き換えると`belief_v2=false`環境を壊す**:
現在の`InputModeApplied{UserTurnOnEisuReset}`は旧`ImeModel::reduce`が**無条件**に適用する
(`crates/awase-ime-core/src/state/ime_model.rs:1014-1019`)。これを`InputModeObserved`で
**置き換える**と、旧モデルは`reconcile_key_effect_mode`(ADR-191決定3の柵、
`KEY_EFFECT_SETTLE_MS = 170ms`、`tuning.rs:479`)を通るため、直近170ms以内に打鍵の予測が
あると観測が**拒否される**。BUG-231は連続して打鍵していた場面であり、手放しが170ms以内の
打鍵とかち合うことは珍しくない。その場合、`belief_v2=false`(旧モデルのみを読む)利用者の
Engineは英数のまま固定され、drift correctionも古いeisuを根拠に動き続ける——**新しい回帰**。
したがって**既存の`InputModeApplied{UserTurnOnEisuReset}`書き込みは維持し、Zのための
`InputModeObserved`書き込みを追加する(置き換えではない)**。旧モデルを`eisu_reset_on_turn_on_while_open`
でゲートしている現在の条件は、Zとの食い違いを考慮していないため、**Z向けの書き込みは
このゲートの外で、手放し(Abandon)自体を条件に無条件で行う**。

**(a)と(b)のどちらを使うか(G4で再確認): 既存の再利用先は無く、両方とも新しい書き込み口が必要**。
`follow_direct_read_in_scope`(ADR-244、`ConvBitsInference`専用、dylintで制限)を転用すると
mozc-shm由来の値をIMM観測のふりで書くことになり偽装(禁止パターン2)に当たる。`report_mozc_shm`
に相乗りするとF2がそのまま再現する。したがって、`platform_state.rs`の`follow_direct_read_in_scope`
の隣に**新しい専用メソッド**(例: `follow_mozc_shm_mode_on_abandon(tick_ms)`)を追加し、
`ir_do_abandon_half_width_alnum_from_mozc_shm`からそこだけを呼ぶ形にする。このメソッドが:
1. `InputModeObserved { mode: AssumedRomaji{UserHalfWidthAlnumToggleOff}, source: MozcShm,
   confidence: High }`をdispatch(Zへ)、
2. `InputModeApplied { 同じmode, UserTurnOnEisuReset, Applied }`をdispatch(旧モデルへ、無条件)、
3. 呼び出し元向けに、Z/旧モデルがromaji-capableへ終わったかを返す(ログ用)。

(a)を(b)(`UserHalfWidthAlnumToggle`、ゲート無しで無条件に書く案)より推奨する追加の理由:
(b)はZの変換表で`ScopeEvent::Explicit`になり、`Basis::Explicit`は`read_started_ms >=
entry + settle_ms`の柵を経てからでないと後続の観測に上書きされない(`scope_belief.rs:367,786-793`)。
つまり(b)を選ぶと、手放し直後のしばらくの間、mozc-shmからの食い違う実観測が無視される
settle窓ができる。(a)(`ScopeEvent::Observed`)はこの柵を持たず、次の実観測を即座に反映できる。

**`notify_engine_refresh()`自体は必要(元のD1の後半は正しい)**: `ir_execute`の実行順は
`ir_stage_notify()`(`ir_notify_engine_refresh`を呼ぶ)→`mozc_shm_on_tick`→
`ir_abandon_half_width_alnum_from_mozc_shm_if`であり、**同一tick内で手放しがEngine通知より
後に起きる**ため、通常のリフレッシュtick経由でも手放しの直後にEngineは再評価されない
(次のtickか次のキーまで待つ)。さらに`TIMER_MOZC_SHM`専用タイマー経由(ADR-268 D5、
`explicit_intent`でリフレッシュtickが止まっている間の経路)では`ir_stage_notify`自体が
一切呼ばれないため、`notify_engine_refresh()`を明示的に呼ばない限りEngine通知が永久に
発生しない。したがって、モード軸をZへ届ける(上記(a)/(b))ことと、`notify_engine_refresh()`
を呼ぶことの**両方が必要**(片方だけでは直らない)。

### 案B(保留・D2計測が先): mozc-readableのPulseイベントを待ち受けて即時に読み直す

`~/mozc-readable-release/repo/src/win32/tip/tip_state_publisher.cc`は、共有メモリへの書き込み
(`Flush()`)ごとに`Pulse()`(named event `Local\mozc-readable-ime-changed`、`CreateEventW`で
session-local・sharable security descriptor付きで作成、`SetEvent`→`ResetEvent`)を呼んでいる。
awase側の`crates/awase-windows/src/observer/mozc_shm.rs`はこのイベントを一切使わず、
`TIMER_MOZC_SHM`(`MOZC_SHM_TIMER_MS = 500ms`)の定期ポーリングのみで共有メモリを読んでいる
(`grep -n "ChangeEvent|WaitForSingleObject|OpenEventW|CreateEventW"`が0件)。

**実装方針**: `crates/win32-worker`の`WorkerThread`/`ShutdownToken`(常駐ワーカースレッド+
シャットダウン用named eventのRAII、既存)を土台に、`OpenEventW(SYNCHRONIZE, FALSE,
"Local\\mozc-readable-ime-changed")`で取得したハンドルと既存のshutdownイベントの両方を
`WaitForMultipleObjects`で待つ常駐スレッドを1本追加する。変化イベントが発火したら、
メインスレッドへカスタムウィンドウメッセージ(`PostMessageW`)を送り、ハンドラで既存の
`mozc_shm_on_timer`相当の読み直し(`mozc_shm_read_once`→`ir_abandon_half_width_alnum_from_mozc_shm_if`)
を1回走らせる。`TIMER_MOZC_SHM`の定期ポーリングは**そのまま残す**(下記のPulseの既知の
レース、およびmozc-readable未導入環境でのフォールバックのため)。

**Pulseパターンの既知のレース、および想定より効果が小さい可能性**:
1. `Pulse()`は手動リセットイベントに対する`SetEvent`直後の`ResetEvent`であり、これはWin32の
   `PulseEvent()`API自体が持つのと同型のレースを内包する。BUG-231のF2押下では、`key_seq`更新
   (打鍵自体)の書き込みとモード変化の書き込みが短時間に連続しうるため、ワーカースレッドが
   1回目のPulseで起きて`PostMessageW`し、`WaitForMultipleObjects`に戻る前に2回目のPulseが
   素通りする取り逃しが、まさに狙いたい場面(モード変化の検知)で起きうる。
2. **ポーリングが足せる遅延は最大500ms(平均約250ms)のみ**。BUG-231の1.63秒のうち、
   少なくとも約1.13秒はMozc自身の実際の切替時間であり([BUG-230](../known-bugs/BUG-230.md)の
   実測、F2押下から`hiragana`表示まで約2秒、と整合)、D2はこれを一切縮められない。つまりD2が
   実際に削れる幅は「ポーリング由来の部分」だけで、見積もりは「約1.6秒の大部分短縮」ではなく
   **「最大500ms程度」**に下方修正する必要がある。
3. 起動順序(awaseはログオン時に起動し、mozc-readableのTIPがまだ`CreateEventW`していない
   可能性がある)、AppContainer化されたTIPホスト(`Local\`が`BaseNamedObjects`ではなく
   AppContainer専用名前空間に解決され、Pulseが届かない)、取り逃し対策の再読み直し猶予
   (新しいtuning定数が必要)など、設計の詳細が未決定のまま残る。

**mozc-readable未導入環境への影響なし**(この点は変わらず): `OpenEventW`はmozc-readableのTIP
プロセスが実際に走っていて初めて名前付きイベントが存在するため、未導入環境では単に開けず、
既存のポーリングのみで従来どおり動作する。

**保留の理由**: 上記2により、D2の実装コスト(`win32-worker`への`WaitForMultipleObjects`対応
追加、常駐スレッド、カスタムウィンドウメッセージ、DACL/AppContainerの実機確認、取り逃し対策)
に対して得られる実益が「最大500ms」に留まる可能性が高く、`.claude/rules/complexity-budget.md`
の精神(まだ強制発効はしていないが)に照らしても投資に値するか怪しい。**着手前に、
mozc-shmのブロックが持つ`update_tick_ms`(Mozc自身の`GetTickCount64`)と手放し検知時の
`now_ms`の差を1行ログに出し(既に両方の値は`ir_do_abandon_half_width_alnum_from_mozc_shm`
到達時点で手元にある)、実際のポーリング由来の遅延がどの程度かを複数回の実機観測で測ってから
着手を判断する**。小さければ、`TIMER_MOZC_SHM`間隔を半角英数トグルが有効な間だけ短縮する
(実測を伴う`tuning.rs`変更、`.claude/rules/tuning-constants.md`準拠)方が遥かに安い代案になる。

### 案C(却下、ただし理由を訂正): かなキー押下時にbeliefを即座に楽観補正する(ユーザー提案の2つ目)

ひらがなキー押下を検知した時点で、Mozcの実際の切替完了を待たずにbelief(`input_mode`・Engine活性)
を即座に書き換える案。却下は維持するが、**理由をBUG-230との比較だけで済ませるのは不正確**:
現状でも、Mozcの実際の切替が完了する前のあの数秒間は、Engineが非活性なので打鍵がそのまま
qwertyローマ字でOSへ素通りしている(BUG-230タイムライン項目4「F押下→変換されず」がまさにこれ)。
つまりD3はこの期間の出力を「ローマ字のまま漏れる」から「NICOLA変換されて半角英数Mozcへ送られる」
に変えるだけで、どちらも誤った出力である点は変わらない。**却下すべき本当の理由は、確認なしに
確定させたbeliefが、確認が来なかった場合(取りこぼされたキー、Mozcが実際には切り替わらない、
共有メモリが読めない窓)に、Engine-activeのまま半角英数Mozcへの誤送信が有限の数秒ではなく
無期限に続く、訂正不能な状態を作ることにある**(BUG-192/ADR-290 round1 F1/F2が指摘したのと
同じ種類のリスク)。

### 案D(新規・今回は見送り): 確定するまで打鍵を保持する

もう1つの選択肢: ひらがなキー押下を検知してから、mozc-shmの手放し確認が出るまで、以降の打鍵を
(既存の`INPUT_DEFER.replay_later`・`focus_switch_hold`・`resume_hold`・ADR-263のepisode hold
と同系統の機構で)保持し、確認が出たら保持列をEngine活性状態で再生する。タイムアウトで
素通りへフォールバックする安全弁付き。これはD1′/D2と違い、**確認が出るまでの区間自体の
誤出力(現状のローマ字漏れ)も無くせる唯一の案**。

**ADR-290自身が同じ案を既に検討・却下している**(`docs/adr/290-*.md`却下した案「半角英数トグルを
抜けるまでNICOLAの送信自体を保留する(holdキュー)」): 実装コストが高く、ADR-290のD2
(GJI脱出直後のMozc確認・回収への相乗り)で同じ結果(最終的に正しい文字が残る)が得られる見込み、
という理由で見送られている。**この見送りはADR-290 D2の実装を前提にしており、D2は現在も
未着手のまま残る**([未解決]参照)。したがって本ADRのD4見送りと、ADR-290の過去の見送りは
**同じ未実装の前提(ADR-290 D2)に依存している**——D4を今回も見送るなら、D2が未着手である限り
「確認までの区間の誤出力」自体への対策は依然どこにも存在しないことを、両ADRのどちらかが
明記しておく必要がある。本ADRでは、D1′(belief_v2の穴)を先に塞ぐ方が優先度が高く、保持機構の
追加はタイムアウト定数の実測や既存の保持機構との競合確認などスコープが大きいため見送るが、
`.claude/rules/fix-requires-evidence.md`のdefer/replay系ファミリーの延長として今後の選択肢に残す。

## 決定

- **D1′(採用、round2で設計を訂正)**: `ir_do_abandon_half_width_alnum_from_mozc_shm`から、
  `platform_state.rs`に新設する専用メソッド(`follow_mozc_shm_mode_on_abandon`等、
  `follow_direct_read_in_scope`の隣)を呼ぶ。このメソッドは
  (1)`InputModeObserved { mode: AssumedRomaji{UserHalfWidthAlnumToggleOff}, source: MozcShm,
  confidence: High }`をZへ、(2)`InputModeApplied { 同じmode, UserTurnOnEisuReset, Applied }`を
  旧`ImeModel`へ**両方・無条件に**dispatchする(置き換えではなく追加。旧モデル向けの書き込みを
  やめると`belief_v2=false`環境が壊れる、G2参照)。モードの値は**常に`AssumedRomaji`に固定し、
  `ObservedKana`は書かない**(ADR-290 D1′ F1が実証した致命的回帰の再現を防ぐ)。この書き込みの
  直後に`self.notify_engine_refresh()`を呼ぶ(両方必須、片方だけでは直らない)。
- **D2(保留)**: mozc-readableのPulseイベントによる即時通知は、まず`update_tick_ms`との差で
  ポーリング由来の遅延を実機で複数回測定し、実益が投資に値するか確認してから着手を判断する。
- **D3(却下)**: かなキー押下時のbelief/Engineの即時楽観補正。理由は「訂正不能な無期限の
  誤送信を作る」こと(BUG-192/ADR-290 round1と同種のリスク)。
- **D4(見送り)**: 確定まで打鍵を保持する案。ADR-290自身が同じ案を既に見送っており、その見送りは
  ADR-290 D2(未着手)の実装を前提にしている。本ADRでもD1′を優先し見送るが、D2(本ADRのD2・
  ADR-290のD2のどちらも)が未着手である限り、確認が出るまでの区間自体の誤出力への対策は
  依然存在しないことを記録する。

## 影響

- **変わる挙動**: belief_v2(既定true)環境で、半角英数トグルの手放し検知からEngine活性までの
  約3.65秒の反映欠落がD1′で解消される見込み(実機確認後に確定)。`belief_v2=false`環境の挙動は
  変わらない(旧モデル向けの書き込みは従来どおり維持)。D2は保留のため、mozc-shm自体の検知遅延
  (最大500ms程度+Mozc自身の切替時間)は本ADRの範囲では変わらない。
- **新しいコード**: `platform_state.rs`に新規メソッド1つ(`InputModeObserved`構築箇所が
  同ファイルで3→4)、`ir_do_abandon_half_width_alnum_from_mozc_shm`からの呼び出し1箇所+
  `notify_engine_refresh()`呼び出し1行。新しい`EngineCommand` variant・新しい`ImeEvent` variantは
  不要。
- **更新が必要な既存のガード/文書**: `architecture_guard.rs::input_mode_observed_construction_sites_are_accounted_for`
  の`platform_state.rs`期待値(3→4)と`:695-699`のコメント(「ADR-290はこの軸に構築しない設計」
  という記述を本ADRでの変更とあわせて訂正)。`mozc_shm_reader_touches_belief_only_through_report_mozc_shm`
  のdocコメント(「beliefへの入口は`report_mozc_shm`の1つだけ」という記述。このガード自体は
  `mozc_shm_shadow.rs`/`observer/mozc_shm.rs`しか走査しないため新メソッドでは落ちないが、
  文書としての不変条件は変わるので明記する)。ADR-268本文(D1/D3、「mode軸は意図的に繋がない」
  としていた記述)。dylint `observation_source_guard`の`RESTRICTED_SOURCES`に`MozcShm`は現在
  含まれないため変更不要だが、将来の自由な拡散を防ぐため新メソッドへの限定を追加するのは任意。
- **変わらないもの**: IMEへの能動的な送信(D1′はbeliefの正確さと再評価のタイミングだけを変え、
  OSへの新しい書き込みを増やさない)。

## 未解決

1. **dragonflyg4の`belief_v2`設定値の確認が望ましい**: 既定は`true`だが、config.tomlでの直接
   確認はしていない。ただし、BUG-231のタイムラインで`[belief-v2] 食い違い開始`/`食い違い終了`の
   ログ行が実際に出力されており(`platform_state.rs:481`の`scope_shadow_compare`が有効である
   証拠)、設定が`true`だったことの直接証拠として既に確認済みと言える。
2. **回帰テストはbelief_v2有効のハーネスで書く**: 既存の`UserTurnOnEisuReset`のgoldenは旧
   `ImeModel`のreducerのみを通す(`run_reducer`)ため、このバグがあっても通ってしまう
   (`fix-requires-evidence.md`の(a)要件を満たすには、`tests/support/harness.rs`の
   `set_engine_belief_z(true)`を使う構成が必須)。最低限、(i) Z有効ハーネスで「手放し後、
   追加のキー無しでEngineが活性化する」こと、(ii) 旧モデルで「打鍵直後170ms以内の手放しでも
   romaji-capableへ終わる」こと(G2回帰の固定)、(iii) 書き込むモードが常にromaji-capableで
   `ObservedKana`にならないこと(ADR-290 F1回帰の固定)、の3つを追加する。
3. **[BUG-222](../known-bugs/BUG-222.md)の`kp_exit_lang_detect_toggle_on_hiragana`も同じ
   `UserTurnOnEisuReset`を書くため、belief_v2有効時は同じ穴を持つ可能性が高い**(未検証、
   BUG-231の該当F2の`[key-effect-predict]`が`mode=None`だったのと同じ着眼点でBUG-222自身の
   ログから確認可能)。本ADRでは手放し経路(BUG-231の直接原因)のみ修正し、BUG-222側は別途確認・
   必要なら追随修正する。
4. `{UserHalfWidthAlnumToggle, PanicReset}`以外の`InputModeApplyStrategy`がすべてZ配下で
   Engineに無視されるという一般化した穴(ADR-282自身の設計)は、本ADRの範囲外として
   ADR-282の追補課題に記録することを推奨する。
5. 本ADRはBUG-231(手放し後の遅延)のみを対象とし、[BUG-230](../known-bugs/BUG-230.md)のD2
   (GJI側の抜け出し送信にMozcの実際の切替完了の確認を足す、ADR-290で未着手のまま残る項目)には
   踏み込まない。両者は別の未解決項目として残す。D4(本ADR)もADR-290のD2に依存する(上記決定参照)。
