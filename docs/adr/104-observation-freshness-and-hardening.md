# ADR-104: 非同期観測の鮮度・Win32 戻り値・死んだ安全弁の整理

## ステータス

**提案（未実装、2026-08-26）。** 直近のコードレビューで確証された指摘のうち、「非同期観測の照合材料が bool/番兵に潰されている」「Win32 の失敗・契約違反を成功値として扱っている」「死んだ安全弁・重複判定が残っている」の3系統をまとめる。Opus 2体によるドラフト→敵対的レビューを4ラウンド実施し収束させた。関連: [ADR-102](102-startup-key-delivery-one-way-closure.md)、[ADR-103](103-warmup-probe-pending-integrity.md)。

## コンテキスト

対象の指摘:

- **observation_store.rs の drift confidence 無視**: `update_drift` が confidence を見ずに単発一致で無条件に drift をクリアするため、高 confidence 観測由来の乖離検知が低 confidence 観測1件で握りつぶされうる。
- **key_pipeline.rs の stale shadow_on**: `kp_stage_focus_probe` が spawn 前に取ったスナップショットを、probe 完了時に「フレッシュな観測」として書き込んでしまう。
- **message_handlers.rs の generation=0 番兵衝突**: `generation.unwrap_or(0)` と `0 => None` の復号が、正当な実 generation 値 `0` と衝突する。
- **key_pipeline.rs の同期 conv 読み取り**: メッセージループスレッド上で `SendMessageTimeoutW` ベースの同期読み取りを行い、BUG-34 と同種のハングを再導入している。
- **SendInput 戻り値の未チェック**: `send_input_safe` の実送信数が13箇所で捨てられている。
- **timer.rs の SetTimer 失敗未検査**: 戻り値 `0`（失敗）を無条件に有効な OS タイマー ID として登録する。
- **probe_fsm.rs の型で保証されない `unreachable!()`**: 別ファイルの実装上の性質だけを根拠にしている。
- **候補ウィンドウ veto の flicker**（調査の結果、撤回。後述）。
- **force_guard.rs の死んだ安全弁・focus/classifier.rs のフォールバック非対称・vk.rs 外への magic hex 直書き**。

これらは2つの失敗形に収束する。

3. **照合材料を bool や番兵に潰している**。`Option<u64>` の generation を `0` へ潰す、confidence を捨てて一致/不一致だけ見る、spawn 時スナップショットを「観測」として書く。いずれも「情報の欠落を、欠落として運べない」ことが原因。
4. **Win32 の失敗・契約違反を成功値として扱う**。`SendInput` の実送信数を捨てる、`SetTimer` の `0` を有効 ID として登録する、型で保証されない契約を `unreachable!()` で守る。最後のものは、フックスレッドと同一プロセスでの panic ＝ OS 全体のキーボード停止に直結しうる。

残りは死んだコード・重複判定の整理として決定11 にまとめる。

### 制約

- [ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md) の3層分離を破らない。「API を叩いていない値を観測として記録する」laundering は導入しない。
- タイミング定数は変更しない。値を動かす提案が出た場合は [tuning-constants](../../.claude/rules/tuning-constants.md) の実測義務を満たすこと。

## 不変条件

- **INV-C（欠落の保存）**: 「無い」「確度が低い」「失敗した」は `0`/`false`/`()` に潰さず、`Option`/`NonZero`/専用 enum で運ぶ。

---

## 決定6: 非同期観測の照合材料を1つの型に集約する

**6-a. `ImmLikeTicket` を `ObservationTicket { focus_epoch, focus_hwnd, intent_seq }` に拡張する。**

`kp_stage_focus_probe` が spawn 前に取った `shadow_on = effective_open()` を、probe 完了時に `probe_ime_on == None`（TsfNative/Imm32Unavailable）の場合「フレッシュな観測」として書き込む。既存の epoch 照合はフォーカス変更しかガードしないため、**同一ウィンドウ内で probe 飛行中にユーザーが物理 IME キーで ON にした**場合、古い OFF スナップショットが確定観測を上書きしうる。

- **`focus_hwnd` を持つ**: `focus_epoch` が進むのは `on_focus_process_changed`、すなわちプロセスが変わったときだけである。Windows Terminal のペイン移動のように**同一プロセス内で別ウィンドウへフォーカスが移る**ケースを epoch は捕まえない。チケットに spawn 時の `focus_hwnd` を入れ、完了時に一致を要求する。
- **なぜ「`advance_focus_tracking` ごとに進むカウンタ」ではないか**: `apply_focus_probe_result` は周期的な IME refresh のたびにも呼ばれる。無条件にカウンタを進めると飛行中の非同期観測が毎回 stale 判定されて全部捨てられる。hwnd という「変わったときだけ変わる identity」で照合するのが正しい。

**カウンタは `event_log.next_seq()` を流用しない**。`dispatch_event` は無条件に記録を行うため `next_seq` は全 IME イベントで進み、周期観測だけで probe の飛行時間を跨いで進む。結果 shadow フォールバック経路が恒常的に抑止され、TsfNative/Imm32Unavailable では drift 追跡の唯一の材料が消え、drift 補正が発動しなくなる。

```rust
// state/ime_model.rs: reduce() の中でだけインクリメントされる。
pub struct ImeBelief { /* ... */ intent_seq: u64 }
```

`intent_seq` を進めるのは「ユーザーが open の意図を変えた」イベント（`UserImeSetIntent`、`PanicReset`）だけに限る。`ObserverReported`/`EngineActivationSync`/`HwndCacheRestored`/`FocusChanged` では進めない。

admission は2段にする: `focus_epoch` または `focus_hwnd` 不一致 → 従来どおり全体を棄却。両方一致かつ `intent_seq` が進んでいる → **shadow フォールバック書き込みのみ抑止**（`probe.ime_on` が `Some` の実観測は `intent_seq` に関係なく採用する）。

**なぜ完了時に belief を読み直して一致確認しないか**: belief-as-evidence の閉ループ（BUG-19/BUG-33/BUG-48/BUG-68/BUG-69 と同型）の再演になる。`intent_seq` 照合は「スナップショット取得後にユーザーが意図を変えたか」だけを見て、変わっていたら何も記録しない——読み戻さない点が違う。

**6-b. drift のクリアに confidence 順序を持ち込む。「低 confidence の一致」は起点をリセットする。**

`state/observation_store.rs:391` の `update_drift` は実際の乖離判定（`most_recent_trusted()`、confidence 優先）と非対称で、経過時間の計測だけが confidence 非考慮である。Medium 観測で始まった drift が Low 観測1件の一致で握りつぶされうる。

クリア条件を単純に「値一致 かつ confidence 以上」に狭めると、その窓で以後 `started_confidence` 以上の観測が来なくなった場合 **drift が二度とクリアされなくなる**（TsfNative 窓では常時これが起きうる）。`drift_duration` だけが伸び続け、後から本物の不一致が観測されると、既に閾値を大幅に超えているため補正が即座に発火する——400ms のデバウンスが実質しきい値0のヘアトリガーになる。

クリア条件を3値にし、低 confidence の一致を「解消」でも「無視」でもなく**起点のリセット**として扱う。

```rust
impl Confidence { pub const fn rank(self) -> u8 { /* Low=0, Medium=1, High=2 ... */ } }

pub struct ImeDrift { started_at: Instant, started_confidence: Confidence }

pub const fn update_drift(
    &mut self,
    desired: bool,
    observed_open: bool,
    observed_confidence: Confidence,
    now: Instant,
) {
    match (desired == observed_open, self.drift) {
        (false, Some(_)) => {}
        (false, None)    => self.drift = Some(ImeDrift { started_at: now, started_confidence: observed_confidence }),
        (true, Some(d)) if observed_confidence.rank() >= d.started_confidence.rank() => self.drift = None,
        (true, Some(d))  => self.drift = Some(ImeDrift { started_at: now, started_confidence: d.started_confidence }),
        (true, None)     => {}
    }
}
```

`Confidence` の比較は `const fn rank(self) -> u8` によるプリミティブ比較にする（`PartialOrd::ge` は const 文脈で呼べない）。呼び出し点は `ImeEvent::ObserverReported` の reduce アーム（`state/ime_model.rs:511`）の1箇所だけ。低 confidence の一致で `started_at` を `now` へ倒すことで、「Low の一致1件で握りつぶされる」ことも「クリア不能になってヘアトリガー化する」ことも同時に防げる。デバウンスの意味は「最後に一致を観測してから 400ms 乖離が続いたら補正する」に変わるが、**`DRIFT_CORRECTION_THRESHOLD_MS` の値は変えない**（解釈が変わる点はソークで発火回数の前後比較を取ること）。

**6-c. WM ペイロードの `0` 番兵をやめ、ビット幅の飽和も欠落として運ぶ。**

`runtime/message_handlers.rs:365` の `generation.unwrap_or(0)` と `0 => None` は、正当な generation 値 `0`（プロセス起動後最初の ImmCross 非同期 SetOpen）と衝突する。衝突すると staleness 照合がスキップされ、本来 stale と判定されるべき完了が無条件適用される。

```rust
pub struct AsyncImeApplyPayload { pub open: bool, pub reason: OpenApplyReason, pub generation: Option<u64> }
impl AsyncImeApplyPayload {
    /// bit0=open, bit1=reason, bit2=has_generation, bits3..=generation
    /// generation が bits3.. に収まらない場合は `None` を返す（切り捨てではなく明示的な劣化）。
    pub fn to_wparam(self) -> (usize, EncodeDegradation);
    pub const fn from_wparam(w: usize) -> Self;
}
pub enum EncodeDegradation { Exact, GenerationDropped }
```

encode/decode を1つの型に閉じることで、「reason のビット幅拡張し忘れで未知の reason が静かに丸められる」落とし穴も、片方だけ直すことが構造的に難しくなる。往復テスト（`None`/`Some(0)`/`Some(1)`/収まる最大値/収まらない値 × reason 2値 × open 2値の全数）を型の隣に置く。

**証拠義務**: IME belief ファミリー。`intent_seq`（`ObserverReported` を100回流しても進まない／`UserImeSetIntent` で1進む）、`update_drift`（Medium で始まった drift を Low の一致がクリアしない／Low の一致が `started_at` を倒すので直後の不一致で即発火しない／Medium 以上の一致はクリアする、の3系列）、`ObservationTicket`（同一 pid・別 hwnd で棄却される）、codec 往復（劣化ケース含む）を固定する。`docs/known-bugs.md` に暫定 **BUG-81** を起票し、BUG-69/BUG-68 の系譜として相互参照を張る。

---

## 決定7: メッセージループから同期 conv 読み取りを追い出す

`runtime/key_pipeline.rs:2075` の `apply_focus_probe` 内 `[focus-conv-check]` は、TsfNative へのフォーカス時にメッセージループスレッド上で同期 IPC（`SendMessageTimeoutW(SMTO_ABORTIFHUNG)`）を呼ぶ。BUG-34 で確認済みのとおり、ハング判定が確定するまで実質 timeout_ms を無視して数秒ブロックしうる。フリーズ気味のアプリへ Alt+Tab した瞬間にキーボード入力全体が停止する。

既存の idle-conv-check と同じ offload 経路に合流させる（新しい機構は作らない）。

1. 読み取りを `get_ime_conversion_mode_raw_timeout_async(10)` に置き換える。
2. spawn 時に `ObservationTicket`（決定6-a）と `conv_mutation::current()` を捕捉する。
3. 完了時に spawn 時の前提を全部再評価し、1つでも崩れていれば破棄する: `focus_epoch` 一致、**`focus_hwnd` 一致**（`prev_conversion_mode` はプロセス変更の有無に関わらず全 `advance_focus_tracking` で無効化されるため、hwnd 項が無いと古いウィンドウ由来の値が書き戻される経路が残る）、`conv_mutation_seq` 一致、**`output_in_flight_ms() != u64::MAX` の再評価**（同期であることに依存して省略されていた条件）。
4. 上記を通ったときだけ `ConvModeMgr` と `prev_conversion_mode` を更新する。
5. 多重 in-flight 防止フラグは idle-conv-check と**別フィールド**で持つ（共有すると無関係なキーが先に in-flight を掴んで一方を飢餓させる、BUG-77 の教訓）。
6. `get_ime_conversion_mode_raw_timeout`（同期版）の `runtime/` 層からの呼び出しを `tests/architecture_guard.rs` で 0 件に固定する。

**なぜ `send_health::blocking_allowed()` の強化ではないか**: サーキットブレーカは一度ハングした後にしか効かない。初回のハングがそのまま最大5秒のキーボード停止になり、それが BUG-34 の実害そのものである。

**証拠義務**: focus 遷移 + conv ファミリー。fence の純粋テスト（epoch 不一致／同一 pid・別 hwnd／conv_mutation 進行／in-flight 化 の4系列でいずれも破棄）。`docs/known-bugs.md` の既存 BUG-34 エントリに横展開完了を追記する。実機確認項目: フリーズ中のアプリへ Alt+Tab した際にキーボードが止まらないこと。

---

## 決定8: Win32 の戻り値を型で受ける

**8-a. `send_input_safe` の報告を捨てられなくする。**
`win32::send_input_safe` は `#[must_use]` で実送信数を返すが、`let _ =` で捨てている箇所が13箇所ある（`output/`, `tsf/`, `runtime/` 各所）。UIPI ブロック等で部分送信になると、合成キーの DOWN/UP ペアの片方だけが送られ、押しっぱなしのまま検知も回復もされない。

```rust
#[must_use]
pub struct SendInputReport { requested: usize, sent: u32 }
impl SendInputReport {
    pub fn is_complete(&self) -> bool;
    pub fn missing(&self) -> usize;
    /// 意図的に無視する唯一の口。副作用を持たない（ログも出さない）。
    pub fn ignore_partial(self, why: &'static str);
}
```

`ignore_partial` を無副作用のマーカーにしたのは、フックコールバック上の呼び出し（`hook.rs:291`、既に `sent=` ログを出している）にログ追加を強制しないため。部分送信の検知・報告は呼び出し側の責務とし、適用対象はキー列の DOWN/UP ペアを送る出力層に限る。`tests/architecture_guard.rs` の役割は「最も出やすい書き方を0件に固定する」ことであり、無視を不可能にすることではない——真の歯止めは 8-b の経路が存在することである。

**8-b. 部分送信からの回復は純粋関数で計画し、自分のフックを再入させない。**

```rust
pub fn plan_stuck_key_release(batch: &[(VkCode, KeyDir)], sent: usize) -> Vec<(VkCode, KeyDir)>;
```

`[0..sent)` の中で DOWN が送られ対応する UP が送られていない VK にのみ UP を1回だけ送る。回復 UP は必ず元バッチと同じ `dwExtraInfo`（`INJECTED_MARKER` 系）を付ける（フック冒頭の `is_self_injected` で弾かれないため）。`OUTPUT_GATE` は元バッチのスコープを抜ける前に送る。回復送信自体は `ignore_partial("stuck-key release best-effort")` とし、再帰・リトライはしない（無限再送は BUG-27追補2 の失敗形）。UIPI ブロックでは典型的に0件挿入で返るため `plan_stuck_key_release` は空を返しこの経路は動かない可能性が高い——まずは `is_complete() == false` の**発生件数を journal に数える**ことを実装に含め、一定期間0のままなら回復ロジックごと削除する判断材料にする。

**8-c. OS タイマー ID を `NonZeroUsize` にする。**
`timer.rs:33` は `SetTimer` の戻り値 `0`（失敗）を無条件に有効 ID として登録する。以後 `resolve(0)` が意味のないマッピングを返す。`OsTimerId(NonZeroUsize)` を導入し、`NonZeroUsize::new(os_id)` が `None` なら登録しない（warn ログのみ）。

**証拠義務**: キー選択/force-write ファミリー。`plan_stuck_key_release` の全数テストを追加し、`tests/ime_key_sequence_golden.rs` の期待値に変化が無いことを確認する（送信キー列そのものは変えない変更なので、golden が動いたら設計ミスのシグナル）。

---

## 決定9: 型で保証されていない契約を panic で守らない

`tsf/warmup/probe_fsm.rs:412` は `DetectionResult::SuspectedLiteral => unreachable!(...)` と書いており、根拠は別ファイル（`tsf/probe.rs`）の実装上の性質だけである。per-VK confirm は実運用でヒットしやすい経路であり、契約が破れるとフックスレッドと同一プロセスで panic する。

戻り値の集合を狭める。

```rust
pub enum VisibleFencingVerdict { CompositionConfirmed, StaleConfirm }
impl From<VisibleFencingVerdict> for DetectionResult { /* 広げるのは呼び出し側 */ }

fn visible_fencing_verdict(&self, deadline_ms: u64) -> Option<VisibleFencingVerdict>;
```

`unreachable!()` は消え、契約違反はコンパイルエラーになる（[ADR-101](101-bug74-giveup-retry-with-focus-guard.md) のコードレビュー訂正6と同じ手法）。`VisibleFencingVerdict` と `From` 変換は ungated な `tsf/literal_facts.rs` に置く。

**なぜ `log::error!` + 安全側の既定値に置き換えないか**: panic は消えるが「型が保証していない契約」は残り、次の変更で静かに誤った既定値へ落ちる。

---

## 決定10: 候補ウィンドウ veto の flicker 対策は撤回し、死んだリセットだけを消す

`tsf/warmup/literal_detect_fsm.rs:437` の `veto_decision` が不可視 tick で `veto_started_at_ms = None` にリセットするため、候補ウィンドウが flicker すると `GJI_CANDIDATE_VETO_CAP_MS` に到達しない、という懸念があった。**調査の結果、これは再現しないと判明した**。実コードでは `SuspectedLiteral` 時に `veto_decision` が `NotApplicable` を返した枝は必ず `ProbeAction::Done` を含む回収へ進む。つまり**候補ウィンドウが不可視に見えた最初の tick で検出セッションはその場で終了し**、通常の回収（backspace + romaji 再送）に進む。「flicker で `Hold` が無期限に続き cap に到達しない」破綻シナリオは起き得ない。

さらに、当初の対策案（積算の起点をセッション単位に変える）を入れると**新しい破綻が生まれる**: 同一 romaji のまま veto 条件を一度満たした後、時間が経ってから再び候補ウィンドウが可視になると、経過時間が既に 300ms を超えているため即 `Expired` となり、`Expired` の枝は「無回収で打ち切り」なのでリテラル化した文字列が画面に残ったまま回収されない。

**改訂した決定（最小限）**: `GJI_CANDIDATE_VETO_CAP_MS = 300ms` は変更しない。積算の起点も変更しない。`veto_decision` の `NotApplicable` 枝にある `self.veto_started_at_ms = None;` は、セッションがその tick で終了するため到達しても意味が無い死んだリセットである。削除し、`veto_started_at_ms` は生成時にのみ初期化される形にする。doc コメントに「`NotApplicable` は検出セッションを終了させるため状態のクリアは不要」と理由を書き残す。この指摘は `docs/known-bugs.md` には起票しない（撤回理由は本 ADR の記録で足りる）。実機ログで候補ウィンドウの flicker と veto の異常が観測されたら、そのときにログを添えて起票し直すこと。

---

## 決定11: 死んだ安全弁を撤去し、重複した判定を1箇所へ

**11-a. `ForceOnReason::ProfilePolicy` を削除する。**
`state/force_guard.rs:29` の `ProfilePolicy` は `PanicReset` と同格の「ユーザーの明示 OFF 意図すら上書きする最上位安全弁」として設計・文書化されているが、production の構築点は導入時から一度も存在しない。ADR-098 が force-on 経路を縮小する方向を決めていること、「消費ロジック無しの予備実装を置かない」（撤回済み CharsetSlot と同型の失敗）という方針から、削除を選ぶ。将来 profile 由来の恒久 force-on が必要になったら、構築点・actuation 経路・eisu 救済の配線・テストを同じコミットで入れる。

doc の追従を同じコミットで行う（`state/open_warrant.rs`、`docs/known-bugs.md`、`docs/ime-control-overview.md`、`docs/adr/038-force-guard-drift-monitor.md`、`docs/adr/087-open-belief-actuation-warrant-separation.md` の該当記述を「削除済み（本決定）」へ更新——消さずに履歴として残す）。variant だけ消して doc を残すと、後日「設計上あるはずの安全弁が実装から消えている」と解釈され再実装されるおそれがあるため。

**11-b. force 系の一致判定を集約する。ただし hint 系と kind 系でフォールバックの適用可否を分ける。**
`focus/classifier.rs:123` の `injection_hint` は `force_tsf` にだけ UWP の InputSite フォールバックを適用し、`force_vk` には適用していない。UWP アプリでフォーカスが `Windows.UI.Input.InputSite.WindowClass` になっている間、ユーザーが `force_vk` に実クラス名を書いても効かない。

```rust
unsafe fn matches_hint_entry(entries: &[AppOverrideEntry], process_name: &str, class_name: &str) -> bool {
    matches_override_entry(entries, process_name, class_name)
        || input_site_fallback_matches(entries, class_name, process_name)
}
```

適用先は **`force_tsf` と `force_vk`（hint 系）のみ**。`force_text`/`force_bypass`（kind 系）には広げない——hint 系の誤マッチは「注入方式が変わる」だけだが、`force_bypass` は `FocusKind::NonText` を返し、NonText 早期 return で**全キーが OS へ素通し＝NICOLA が丸ごと無効**になる判定であるため。設定していないユーザーには既存の早期 return によりコストは乗らない。

**11-c. VK の magic hex を `vk.rs` へ戻す。**
`ime.rs:1659`/`:1674`/`:1686` の `0x14` を `vk::VK_CAPITAL` に、`hook.rs:844` の `matches!(vk.0, 0x12 | 0xA4 | 0xA5)` を `vk.rs` のヘルパー（`is_alt_variant`、無ければ追加）に置換する。`vk.rs` には既に `classify_modifier`・`is_ctrl_variant` があり、Alt だけが外で手書きされている。`vk.rs` 外の VK magic hex 出現数を `tests/architecture_guard.rs` で固定する。

**[ADR-102](102-startup-key-delivery-one-way-closure.md) との関係（依存なし）**: 当初 `os_owned_key`（旧ADR-102決定1-b-2、OS所有マスク）がこの `is_alt_variant` の唯一の新規利用者になるとしてPhaseを先行させる予定だったが、(1) Opus敵対的レビューで `os_owned_key` が必要とする左右正規化は `state/alt_impersonation.rs:20` の既存 `classify_alt_side`（既に `pub(crate) const fn`、ungated）で足りると判明し、(2) さらにADR-102は[ADR-105](105-engine-thread-notification-via-hwnd.md)を前提とした全面改訂でOS所有マスク自体（旧決定1-b-2）を撤去した。したがって11-cは独立した `vk.rs` 集約クリーンアップとして、任意の順序で実施してよい（依存する側が存在しない）。

**証拠義務**: `[15]` は force-write/actuation ファミリー（削除により既存テスト2件が影響を受けるため、`PanicReset` 側の同等テストが残ることを確認）。`[16]` は focus 遷移ファミリー。`matches_hint_entry` の全数テストと、**`force_text`/`force_bypass` にフォールバックが適用されないことを固定するテスト**を置く。

---

## テストの置き場所

[fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md) は「Linux で実行できるものを優先する」と明記している。`intent_seq`/`update_drift`（`Confidence::rank`）/codec/`matches_hint_entry`/`VisibleFencingVerdict` の純粋部は既に ungated な `state/`・`focus/`・`tsf/literal_facts.rs` に置ける。`ObservationTicket` の非同期結合部分と `SendInputReport` の実送信確認は Windows 側の結合テストになる。

## 実装順序

| Phase | 内容 |
| --- | --- |
| 任意（依存なし） | 決定11-c（`vk.rs` 集約。[ADR-102](102-startup-key-delivery-one-way-closure.md) のOS所有マスク（旧決定1-b-2）は全面改訂で撤去済みのため、依存関係自体が無い） |
| 1 | 決定6（6-a → 6-b → 6-c） |
| 2 | 決定7（決定6-a の `ObservationTicket` に依存） |
| 3 | 決定8-a/8-b、決定9、決定10、決定11-a/11-b（掃除と歯止め。architecture_guard の追加は最後にまとめて入れる） |

## 新たな panic 経路を持ち込まないことの確認

減る: `probe_fsm.rs:412` の `unreachable!()`（決定9、型で消滅）。増えない: `NonZeroUsize::new` は `Option` を返す（8-c）。`plan_stuck_key_release` はスライスを iterate するのみ（8-b）。payload codec は収まらない generation を `None` へ劣化させるだけ（6-c）。`Confidence::rank()` は `match` のみ（6-b）。

## 却下した代替案

- **`[4]` を送信ヘルスのブレーカ強化だけで済ませる**: 初回のハングを防げず、BUG-34 の実害そのものが残る。
- **veto 積算をセッション単位にする（当初案）**: 実コードでは不可視 tick が検出セッションを終了させるため前提が成立せず、入れると「無回収打ち切り」の早期化という退行になる。
- **`started_confidence` に時間ベースのダウングレード規則を入れる**: 新しい ms 定数が必要になり、その値を決めるための実測対象を定義できない。

## 未解決の疑問（実機ソークで確認すること）

- `intent_seq` の抑止が実際に `[17]` の実機症状の窓で発火しているか、ログで確認する。
- 決定6-b は `DRIFT_CORRECTION_THRESHOLD_MS` の解釈を「乖離開始から400ms」から「最後の一致観測から400ms」へ変える。発火回数の前後比較をソークで取ること。
- 決定7の offload 化により `[focus-conv-check]` の結果反映が数ms遅れる。フォーカス直後の最初の1打鍵がこの遅延を追い越した場合の挙動を確認する。
- 決定8-bの部分送信はUIPIブロック時には典型的に0件挿入となるため実際にはほぼ起きない可能性がある。件数をjournalに数え、一定期間0のままなら回復ロジックごと削除する。
- `GJI_CANDIDATE_VETO_CAP_MS = 300ms` の妥当性は依然として未実測である（決定10を撤回したため据え置き）。

## 設計の経緯

Opus 2体でドラフト→敵対的レビューを4ラウンド実施した。主な転換点: (1) `event_log.next_seq()` を drift 検知の照合カウンタに流用する初期案が、TsfNative/Imm32Unavailable の唯一の観測源を恒久停止させると判明し、専用の `intent_seq` を新設。(2) drift クリア条件を「confidence 以上の一致」に単純に狭める案が、二度とクリアされなくなる対称なリスクを生むと判明し、3値のリセット方式に改訂。(3) 候補ウィンドウ veto の flicker（`[13]`）は実コード調査の結果再現しないと判明し撤回、死んだリセットの除去のみに縮小。
