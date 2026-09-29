---
id: ADR-205
title: |-
  Imm32Unavailable(Chrome 等)の窓で、外部注入の IME キーで閉じられた IME を ADR-187 の通過マーク機構で実状態へ追随する(BUG-172)
summary: |-
  実 Chrome × GJI で他プロセスが注入した 0xF3/VK_IME_OFF により IME が閉じても、awase の belief は ON のまま(observed=0、10/10)。原因は Blacklist 分岐が毎 refresh の
  prefetch 済み snapshot を捨てていることと、注入キー後の refresh が SkipTyping+明示意図でのポーリング停止で Blacklist 分岐に届かないこと。
  本 ADR は、目印なしの外部注入 IME キーで既存の通過マーク(ADR-187)を立て、「同じフォーカス世代の直前の読みが 1」のときだけ readable として、prefetch 済み snapshot で
  実状態へ追随する(意図を捨て desired を揃える。awase は開け直さない)。新 I/O・新 actuation 合流点・新定数なし。
status: |-
  起草(2026-09-29)。opus-adversarial-consult round1 反映済み、round2 待ち。実装未着手。
related_adr:
  - "ADR-029"
  - "ADR-089"
  - "ADR-178"
  - "ADR-191"
---

# ADR-205: 外部から閉じられた IME を Imm32Unavailable の窓で観測する

## 背景(BUG-172、実 Chrome の測定)

[docs/known-bugs/BUG-172.md](../known-bugs/BUG-172.md) の実 Chrome 測定(2026-09-29、GitHub windows CI、各10試行、`cal-driftrec-chrome-real-*`):

- **GJI × 実 Chrome は、他プロセスが注入した 半角/全角(0xF3)・VK_IME_OFF で 10/10 再現**。IME は `IMC_GETOPENSTATUS` で 1→0 に閉じ、3秒後も閉じたまま(`open_at_probe=Some(0)`)、
  打鍵は `kiu`(ローマ字のまま。Engine は ON のまま=直接入力に落ちる)。
- awase の Chrome 開閉の**観測件数 = 0**(`ObserverPoll`=0、`Imm32Unavailable` 39件)。注入キーは hook で見えている(`injected=true`)が、
  `key_pipeline.rs` の BUG-14 分岐(`event.injected` → 「ユーザー意図に昇格させない — belief 追従は may_change_ime refresh 観測に委譲」)で昇格せず、
  委譲先の refresh が Chrome では `ir_stage_observe` の `Blacklist` 分岐(`Skipping IMM query for known-broken class`)で読まないため、belief は古い ON のまま。
- 物理キー(awase 経由)では起きない(awase が shadow-toggle で Engine も OFF にする)。メモ帳経由(アプリ間)は Chrome に影響しない(IME 開閉は窓/スレッド単位)。
- MS-IME × 実 Chrome は注入の 0xF3/0x1A を効かせず再現できていない。対象はまず GJI。
- 「ゲートへ開閉を要求する」案は実 Chrome の症状を直さないため却下済み(BUG-172)。
- 撤去済みの ADR-178 領域A(reassert・force-on)を戻しても差は出ない(対照 `ci/bug172-pre-teardown`)。回復手段の欠如ではなく**観測の欠如**が原因。

**観測経路に乗ったかの確認**(教訓): 上の 0/10 は「起きなかった」ではなく `observed=0`(判断に届いていない)。本 ADR の効果判定は verdict でなく observed 件数で行う。

## 前提となる既存コードの事実(round1 で独立再検証済み)

- **開閉は毎 refresh で既に読まれ、Blacklist 分岐が捨てている**(round1 B0、本セッションでコード確認): `runtime/mod.rs::spawn_ime_refresh` は全プロファイルで
  `read_ime_state_full_async()` を prefetch し、`ime.rs::read_ime_state_full` は `is_tsf_native_window` 以外(Chrome 含む)で `detect_ime_open_for_hwnd`(`IMC_GETOPENSTATUS`、50ms)まで読む。
  `ir_stage_observe` の `Blacklist` 分岐は `ime_snap` を参照しない。よって新しいクロスプロセス読み取り関数は要らない(初稿の `read_open_status_ungated_async` は取り下げ)。
- `read_ime_state_fast`(`ime.rs:873-885`)の「Chrome は常に 0」ゲートはこの prefetch 経路に無い。コメントの出典は `36593bcd`→`d6442c3b`(2026-04-10)で、元コミット本文の根拠は「unreliable or outright blocking」「leaked threads」であり
  「常に 0」は書かれていない。ブロック懸念は現在の offload+`run_with_timeout` で受け入れ済み。値の信頼性は下の D3 で局所的に担保する。
- BUG-14 分岐(`key_pipeline.rs:1092-1101`)は `event.injected` で `return false` するが、`kp_stage_post_decision`(`:1610-1617`)は `may_change_ime && KeyDown && !consumed` で `schedule_ime_refresh(20)` を呼ぶ。
  その 20ms 後の refresh は `ir_decide_read_strategy` で `idle_ms≈20 < TYPING_IDLE_MS(500)` かつ `explicit_verify=false`(`!skip_imm_query` が必要)なので **`SkipTyping`**(round1 B1)。
  さらに `reschedule_ime_refresh`(`runtime/mod.rs:1128-1133`)は、読めない窓で明示意図があればポーリングを止める。ハーネスの `ensure()` は目印付き VK_IME_ON で明示意図を立てるため、
  **注入後に Blacklist 分岐へ届く refresh は再現条件の中で一度も無い**(= BUG-172 の `observed=0` の本当の内訳:「Blacklist が読まない」ではなく「SkipTyping で届かない」+「明示意図でポーリング停止」+「届いても snapshot を捨てる」)。
- 効果の判定は明示意図が支配する: `effective_open`(`state/ime_model.rs:458-479`)は明示意図があれば `desired_open` を返し、観測は無視される。`desired_open` は `HwndCacheRestored`(`:727-734`)で ON に復元されうる。
  `check_drift_correction`(`state/drift_correction.rs:46-133`)は明示意図が無くても `desired` と観測の乖離で発火し(閾値 400ms〈`DRIFT_CORRECTION_THRESHOLD_MS`〉)、Blacklist では `ir_apply_drift_correction`(`:966-995`)が
  `BlacklistDriftCorrection` として実送信する(round1 B3)。**単に `ObserverPoll(false)` を書くだけでは belief は OFF に追従せず、awase が IME を開け直す**。
- `ObserverPoll` は 1 ソース 1 スロット(`observation_store.rs:451-475`)で、Blacklist 分岐内の GJI I/O 観測(`observe_gji_after_focus`)も同じスロットに `true` を書く(round1 M2)。
- TSF `ITfCompartmentEventSink` は ADR-029 で削除済み。ADR-191 の 2〜5ms 実測は自プロセス・自スレッドの compartment。BUG-172 のメモ帳測定(メモ帳で閉じても Chrome は `open=Some(1)`)が、開閉が窓/スレッド単位であることの一次根拠。

## 案の比較

| 案 | 内容 | 実 Chrome の症状に効くか | 副作用・リスク | 判定 |
|---|---|---|---|---|
| A | `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` の通知購読(WH_CALLWNDPROC による他プロセスへの注入も含む) | 効かない見込み。thread/global compartment に Chrome のスレッドの変化は載らないと考えるのが自然(メモ帳/Chrome 独立の測定、ADR-029) | COM sink・寿命管理、注入は侵襲的すぎる | 不採用(再検討防止のため記録) |
| **B'** | **prefetch 済み snapshot の開閉を、外部注入 IME キー後の限定窓で ADR-187 の通過マーク機構に載せて追随する**(D1〜D4) | 効く見込み(実 Chrome で 1→0 が読めることは測定済み、awase 自身の prefetch も同じ読み) | 偽の OFF(D3 で局所化) | **採用** |
| C | フォーカス復帰時に HwndCache の ime_on を鮮度切れ扱い | 効かない(再現条件はフォーカス復帰を伴わない) | 復帰のたびに belief が揺れる | 不採用 |
| D | 何もしない・既知の制限として記録 | 効かない | なし | B' が却下された場合の代替 |
| E | `SetWinEventHook`(`EVENT_OBJECT_IME_*`) | 効かない見込み(IME UI/変換対象のイベントで開閉ではない、GJI TSF では発火しない) | — | 不採用 |
| F | 常時ポーリング拡大・打鍵ごとの読み取り | 効くが、`TYPING_IDLE_MS` の設計と refresh 頻度を悪化させる(読み自体はワーカー offload 済みでフックはブロックしない) | 高 | 不採用 |
| G | Blacklist 分岐で毎回 snapshot を belief に反映 | 効きうるが偽 OFF が常時露出(入力欄/本文で HIMC の付け外しが起きる仮説、M1) | 高 | 不採用(B' は外部注入キー後の窓内に限定) |

## 決定

方針: **「IME の実状態が真実」**(ADR-191)。外部が閉じたなら awase の belief と desired を実状態へ揃え、awase が開け直して外部の操作に逆らうことはしない(BUG-14: 注入キーはユーザー意図に昇格させない、の延長)。
これは ADR-187(物理モードキー通過後の追随)と同じ扱いを、外部注入の IME キーと Blacklist 窓へ広げるもの。

### D1. 外部注入 IME キーで通過マークを立てる(Blacklist 窓でも)

`kp_stage_post_decision` の `may_change_ime && KeyDown && !consumed` 経路(BUG-14 分岐の下流)で、`event.injected` かつ **目印なし**(awase 自身の注入でない)かつ Blacklist 窓(`!can_use_imm32_cross_process()`)のとき、
`arm_mode_key_pass_mark(now, readable)` を呼ぶ。`readable` は D3 の判定。窓は既存の `MODE_KEY_PASS_MARK_WINDOW_MS`(300ms)、読み直しは既存の `MODE_KEY_PASS_REREAD_MS`(60ms)。**新しい定数は作らない**。
`VK_IME_ON/OFF`(0x16/0x1A)は `is_followed_mode_key` が除外するキーだが、外部注入では方向を awase が決めないので、注入経路では除外しない(0xF3・0x1A がハーネスの再現キー)。

### D2. `readable` のとき、Blacklist 分岐が prefetch 済み snapshot を使って追随する

- `ir_decide_read_strategy`: `explicit_verify` の条件に「Blacklist 窓かつ通過マークが有効かつ readable」を足し、打鍵中でも `Blacklist` を返す(snapshot は既に読まれているので追加 I/O なし)。
- `ir_stage_observe` の `Blacklist` 分岐: 通過マークが有効かつ readable のとき、`ime_snap.ime_on`(`Some(v)`)を `classify_blacklist_open_read`(純粋関数、下)で判定し、`ObserveOpen(v)` なら OsPoll 分岐と同じ順で
  `write_observer_poll` → `invalidate_intents_if_mode_key_pass_live` → 窓終了後の `align_after_expired_mode_key_pass`(`ModeKeyPassedThrough{align_desired}`、既存の唯一の構築点〈`pass_through_observed`〉経由)を呼ぶ。
  これで明示意図が捨てられ、`desired_open` が観測へ揃い、drift correction は乖離を見ない(開け直しは起きない)。
- `reschedule_ime_refresh`: 読めない窓で通過マークが有効かつ readable のとき、ADR-187 の読み取り可能窓と同じ読み直し予約を通す(現状は「読めない窓は何も予約しない」)。
- 反映順は `observe_gji_after_focus` の後(同じ `ObserverPoll` スロットで、実読み取りが同じ tick の GJI I/O 推測に勝つ。round1 M2)。順序はユニットテストで固定する。
- 反映は `AcceptedObservation::for_sync(self.focus_fence())`(prefetch は focus probe と同じ spawn 内で取得済み。round1 M3)。

純粋関数 `observer/ime_observer.rs::classify_blacklist_open_read(snap_open: Option<bool>, readable: bool) -> BlacklistOpenVerdict`(`NoEvidence` / `ObserveOpen(bool)`)。`readable=false` は常に `NoEvidence`。

### D3. `readable` は「同じフォーカス世代の直前の読みが 1 だった」ことで決める(クラス単位のセッション latch にしない)

「Chrome は常に 0」説と「1→0 が読める」説を、コードで解消せず両立させる。
- `note_blacklist_open_read(snap)`: 全 refresh(SkipTyping を含む。prefetch は常に走る)の入口で、Blacklist 窓の `ime_snap.ime_on` を「フォーカス世代ごとの直近値」として**記録するだけ**(belief 非書き込み、純粋な記録)。
- `readable = 直近値が Some(true)`(同じフォーカス世代)。注入キー自身の 20ms refresh の読み(閉じる前の 1)も直近値になり、ハーネスの流れで検証の機会が生まれる(round1 B2 への対応)。
- 「常に 0」の窓では直近値が 0 なので `readable=false` → 従来と同じ挙動(通過マークは立つが intent は捨てない。ADR-187 が「破棄するとCIのblind条件でEngineずれが0→22〜25%に悪化」と記す安全側)。
- 偽 OFF の露出は「直近値 1 かつ外部注入 IME キーの直後 300ms」に限られる。入力欄/本文の HIMC 付け外し(M1 仮説)で本文が 0 を返す窓では、本文にいる間に直近値が 0 になり readable にならない。
- 限界: フォーカス世代の初回の読みが 0(IME が既に閉じた状態でフォーカスした)なら追随しない。許容。

### D4. 書き込み・合流点・定数

新しい actuation 合流点なし。既存の `ModeKeyPassedThrough`(唯一の構築点は `pass_through_observed`)と `write_observer_poll` を再利用し、`ImeEvent` の新 variant なし。新しい `_MS` 定数なし。
ADR-178 領域A撤去・ADR-191 の方針(能動書き込みを足さず観測に従う)に沿う。**awase が IME を開け直す経路は本 ADR では足さない**(従来どおり明示意図が残る窓では drift correction が働くが、外部注入キー直後は D2 で意図が捨てられる)。

### D5. 対象範囲

`Imm32Unavailable` の窓。`TsfNative`(RichEdit スーパークラス等)は `read_ime_state_full` が早期 return するため `ime_on=None` で D3 の `readable` にならず、影響を受けない。MS-IME は注入キーが効かないので追随の機会がなく、効果は主張しない。

## リスクと検証計画

| リスク | 対策・確認 |
|---|---|
| 偽の OFF で Engine が誤って OFF になる(最大のリスク) | D3: 直近値 1 かつ外部注入 IME キー直後 300ms の窓に限定。通常打鍵の e2e で「追随」ログが 0 件であること、加えて idle 500ms 超を挟み「ページ本文→入力欄→即打鍵」「omnibox 往復」のシナリオで偽 OFF が 0 件であること(未実施、要追加)。 |
| 読み取りがブロックする | 既存 prefetch(50ms + offload)のまま。新しい I/O は無い。 |
| AutoHotkey 等で意図して閉じた IME を awase が開け直す | 開け直さない(D4)。desired を観測へ揃える。 |
| 通過マークの副作用(明示意図の破棄) | readable のときだけ破棄。読めない窓は従来どおり(ADR-187 の blind 条件の悪化を避ける)。 |
| 物理 IME キー(目印付き)の経路への影響 | D1 は「目印なしの注入」のみ。目印付きは従来の shadow-toggle(awase が Engine も OFF)。 |
| MS-IME・edit・他アプリ | Standard(OsPoll)は無変更。MS-IME は注入キーが効かないため追随の機会なし。 |
| 既存の複数窓口(fix-requires-evidence の表) | 通過マークの arm/consume/expire の3窓口(`kp_stage_post_decision`、`ir_stage_observe`、`reschedule_ime_refresh`/`ir_stage_notify`)すべてに配線したか、architecture_guard の件数で固定する。 |

**実装の第0段(コード変更なし、推奨)**: 既存ハーネス(`cal-driftrec-chrome-real-hz-ext-gji`)を trace レベル(`ime.rs::detect_ime_open_for_hwnd` の `CrossProcess(hwndFocus)`)で1回走らせ、
注入前後の prefetch 値(1→0)、注入後の refresh が `SkipTyping` であること、明示意図が `Some(true)` であることを確認する。前提(B0/B1/B3)の実測での裏取りで、「観測経路に乗ったか」の確認を兼ねる。

**回帰テスト**(host で走るもの): `classify_blacklist_open_read` の表、`note_blacklist_open_read` の世代・直近値、通過マークの arm→consume→align の Blacklist 版(`state/mode_key_pass.rs` 既存テストに readable=Blacklist 条件を追加)、
`state/drift_correction.rs` の closed_loop で「観測 OFF 追随後は drift が発火しない」「読めない窓(readable=false)は従来どおり発火する」を固定、`ObserverPoll` の書き込み順(GJI I/O の後)。architecture_guard: `arm_mode_key_pass_mark` 呼び出し件数、`ModeKeyPassedThrough` の構築点が不変であること。

**実機・CI**: 測定用ブランチから `gh workflow run e2e-ime.yml --ref <branch> -f only='cal-driftrec-chrome-real-*'`(乱発しない)。
ハーネスの PASS は「開け直して NICOLA が出た」を意味し(`got == Class::Nicola`)、本 ADR の期待(追随して Engine も OFF、`ka` で一貫。物理キー対照と同じ)とは逆なので、**判定を書き換える**:
`kiu`(不整合)0/10 を合格、`ka`(一貫した OFF)を許容、`Nicola`(開け直し)は想定外として別計上。効果指標は新経路専用ログタグ(追随した件数・`readable=false` で見送った件数)で数え、`ObserverPoll` の総数は使わない(GJI I/O の `true` が混ざるため)。
対照: 目印付き 0xF3 と MS-IME が退行しないこと、通常の Chrome 打鍵(`ts-chrome`)で追随ログ 0 件。

## 未決事項(所有者判断の候補)

1. 方針「IME の実状態が真実、awase は開け直さない」(D4)の確認。従来の drift correction 型の「開け直し」(ハーネスの旧 PASS 定義)を望む場合は設計が変わる。
2. D3 の限界(フォーカス世代の初回の読みが 0 なら追随しない)の許容。
3. ADR-187 の通過マークを外部注入キーへ広げること(BUG-14 の「注入はユーザー意図にしない」との整合。意図は昇格させず、実状態への追随だけを行う)。

## 敵対レビューの記録

### round1(Opus、2026-09-29): blocker 4・major 3・minor 3

- B0 `read_ime_state_full` の prefetch が Chrome でも開閉を読み、Blacklist 分岐が捨てている → 反映(新 I/O 関数を取り下げ、snapshot 再利用)。
- B1 注入後の refresh は `SkipTyping`+明示意図でポーリング停止 → 反映(D1/D2: 通過マーク、`explicit_verify` の拡張、読み直し予約)。
- B2 フォーカス確定後の読みではハーネスで検証されない → 反映(D3: 全 refresh で直近値を記録)。
- B3 単に `ObserverPoll(false)` を書くと awase が開け直す → 反映(方針決定と D2 の align)。
- B4 ハーネスの PASS 定義が逆 → 反映(合格条件の書き換え)。
- M1 クラス単位 latch の穴 → 反映(フォーカス世代の直近値に変更)。M2 `ObserverPoll` スロット競合 → 反映(書き込み順を固定)。M3 fence → 反映。
- m1〜m3(参照の正確さ、案 G/H の見落とし、ガバナンス)→ 反映。
