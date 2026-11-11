---
id: ADR-291
title: |-
  belief_v2(候補Z)にmozc-shm追随の手放しを反映させ、Engineの活性再評価を明示的に呼ぶ(BUG-231)
summary: |-
  ADR-290/BUG-230適用後のdragonflyg4実機(2026-11-11)で、ひらがなキー押下からEngine活性(Active)まで
  最大約5.3秒ローマ字がリテラルで漏れる新しい遅延([BUG-231](../known-bugs/BUG-231.md))を観測した。
  当初「Engineの活性再評価が物理キーで走らない」と誤診したが、Opus敵対的レビューで訂正: 実際には
  毎物理キーが`Engine::on_input`→`check_active_transition`を再実行していた。本当の原因は
  `general.belief_v2`(既定true、ADR-282)下でEngineが読む候補Z(`scope_shadow`)の変換表
  (`scope_belief.rs::scope_event_of`)が、`ir_do_abandon_half_width_alnum_from_mozc_shm`が書く
  `InputModeApplied{UserTurnOnEisuReset}`を`_ => None`として無視し、Zが英数(Eisu)の主張のまま
  取り残されること(反映の欠落、観測で約3.7秒)。もう1つの原因は、mozc-shmの2段ラッチが
  `TIMER_MOZC_SHM`(500ms)ポーリングのみに依存すること(検知遅延、観測で約1.6秒。ただし大半は
  Mozc自身の切替時間でawase側の遅延ではない)。
  決定: (1)手放し処理でZが受理する形(`InputModeObserved`等)でモード軸を書き、かつ既存の
  `notify_engine_refresh()`を呼ぶ(D1′、両方必須)。(2)mozc-readableの既存Pulseイベントでの
  即時通知(D2)は、ポーリング由来の遅延の実測(大半がMozc自身の時間なら価値が低い)を先に取ってから
  着手を判断する(保留)。(3)かなキー押下時のbelief即時楽観補正(D3)はBUG-192/ADR-290 round1と
  同型のリスクで却下。(4)確定まで打鍵を保持する案(D4)を新たに検討対象として追加、今回は見送り。
status: |-
  起草(2026-11-11)。Opus敵対的レビューround1で原因診断の誤り(F1)を指摘され訂正。round2レビュー
  予定、実装・実機確認とも未着手。
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

**修正の方向(2案、優先順)**:
- (a, 推奨) `ir_do_abandon_half_width_alnum_from_mozc_shm`が、mozc-shmで実際に読んだ
  `effective_mode = Hiragana`の高信頼度観測として`ImeEvent::InputModeObserved { mode, source:
  ObservationSource::MozcShm, confidence: High }`を書く。これは観測の偽装ではない
  (`.claude/rules/ime-belief-architecture.md`の禁止パターン2に当たらない) ——
  この手放し自体が実際にmozc-shmから読んだ`effective_mode`に基づいて発火しているため、
  「観測した」という表現は事実と一致する。`ObservationSource::MozcShm`はADR-268でopen軸の
  観測源として既に確立済みで、mode軸への拡張は同じ信頼度区分の自然な拡張。コスト:
  `architecture_guard`の`InputModeObserved`構築箇所数の固定テストの更新、ADR-268本文
  (「mode軸は意図的に繋がない」としていた記述)の訂正。
- (b) `InputModeApplyStrategy::UserHalfWidthAlnumToggle`を(`UserTurnOnEisuReset`の代わりに)
  使う。ただし`eisu_reset_on_turn_on_while_open(true, self.platform_state.ime.input_mode())`
  による現在のゲート(旧beliefを読む)はZと食い違いうるため、ゲートを外して無条件に書く必要が
  あり、「観測に基づく追随」を「Explicit(確定的な主張)」としてZに記録するという意味の変化を伴う。
- どちらを選んでも、**この書き込みの直後に`notify_engine_refresh()`を呼ぶ(D1の元の提案は
  この部分だけ正しかった、下記案A元の後半を参照)**。モード軸がZに届いても、Engineの再評価
  自体は引き続きEngineCommand経由でしか起きないため、この呼び出しは依然必要(下記参照)。

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
誤出力(現状のローマ字漏れ)も無くせる唯一の案**。今回は、D1′(belief_v2の穴)を先に塞ぐ方が
優先度が高く、保持機構の追加はタイムアウト定数の実測や既存の保持機構との競合確認など
スコープが大きいため見送るが、`.claude/rules/fix-requires-evidence.md`のdefer/replay系
ファミリーの延長として今後の選択肢に残す。

## 決定

- **D1′(採用)**: `ir_do_abandon_half_width_alnum_from_mozc_shm`で、(a推奨)`InputModeObserved
  { mode, source: ObservationSource::MozcShm, confidence: High }`、または(b)ゲートを外した
  `UserHalfWidthAlnumToggle`のいずれかでZが受理する形にモード軸を書き、**かつ**その直後に
  `self.notify_engine_refresh()`を呼ぶ(両方必須、片方だけでは直らない)。
- **D2(保留)**: mozc-readableのPulseイベントによる即時通知は、まず`update_tick_ms`との差で
  ポーリング由来の遅延を実機で複数回測定し、実益が投資に値するか確認してから着手を判断する。
- **D3(却下)**: かなキー押下時のbelief/Engineの即時楽観補正。理由は「訂正不能な無期限の
  誤送信を作る」こと(BUG-192/ADR-290 round1と同種のリスク)。
- **D4(見送り)**: 確定まで打鍵を保持する案。今回はD1′を優先し、scope超過のため見送るが、
  選択肢として記録する。

## 影響

- **変わる挙動**: belief_v2(既定true)環境で、半角英数トグルの手放し検知からEngine活性までの
  約3.65秒の反映欠落がD1′で解消される見込み(実機確認後に確定)。D2は保留のため、mozc-shm
  自体の検知遅延(最大500ms程度+Mozc自身の切替時間)は本ADRの範囲では変わらない。
- **新しいコード(D1′のみ、今回実装する分)**: `ir_do_abandon_half_width_alnum_from_mozc_shm`
  内の書き込みの変更1箇所+`notify_engine_refresh()`呼び出し1行。新しい`EngineCommand` variantは
  不要。(a)を選ぶ場合、`InputModeObserved`構築箇所数を固定している`architecture_guard`のテストと
  ADR-268本文(open軸のみ接続としていた記述)の更新が必要。
- **変わらないもの**: IMEへの能動的な送信(D1′はbeliefの正確さと再評価のタイミングだけを変え、
  OSへの新しい書き込みを増やさない)。

## 未解決

1. **dragonflyg4の`belief_v2`設定値の確認が必要**: 既定は`true`だが、BUG-231の実機が実際に
   この設定だったかをconfig.tomlで確認していない(ログの`[belief-v2]`食い違い行の存在自体が
   有効だった強い手がかりだが、設定ファイルでの直接確認が望ましい)。
2. **回帰テストはbelief_v2有効のハーネスで書く**: 既存の`UserTurnOnEisuReset`のgoldenは旧
   `ImeModel`のreducerのみを通す(`run_reducer`)ため、このバグがあっても通ってしまう
   (`fix-requires-evidence.md`の(a)要件を満たすには、`tests/support/harness.rs`の
   `set_engine_belief_z(true)`を使う構成が必須)。
3. **[BUG-222](../known-bugs/BUG-222.md)の`kp_exit_lang_detect_toggle_on_hiragana`も同じ
   `UserTurnOnEisuReset`を書くため、belief_v2有効時は同じ穴を持つ可能性が高い**(未検証)。
   本ADRでは手放し経路(BUG-231の直接原因)のみ修正し、BUG-222側は別途確認・必要なら追随修正する。
4. `{UserHalfWidthAlnumToggle, PanicReset}`以外の`InputModeApplyStrategy`がすべてZ配下で
   Engineに無視されるという一般化した穴(ADR-282自身の設計)は、本ADRの範囲外として
   ADR-282の追補課題に記録することを推奨する。
5. 本ADRはBUG-231(手放し後の遅延)のみを対象とし、[BUG-230](../known-bugs/BUG-230.md)のD2
   (GJI側の抜け出し送信にMozcの実際の切替完了の確認を足す、ADR-290で未着手のまま残る項目)には
   踏み込まない。両者は別の未解決項目として残す。
