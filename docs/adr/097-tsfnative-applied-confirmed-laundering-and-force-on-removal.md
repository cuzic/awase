# ADR-097: TsfNative フォーカス復帰時の `applied` 偽装確定を止め、到達不能な force-on ブロックを撤去する（BUG-69）

## ステータス

**実装済み（クロスコンパイル検証のみ、Windows 実機未検証）。** Opus の architect ⇔ premortem reviewer 相互討議を3ラウンド実施し（それぞれ独立した Opus インスタンス、実コードを都度検証）、ラウンド3レビュアーの評決は「実装着手可、4巡目不要」。さらに全体俯瞰の architect ⇔ reviewer 討議で決定6(a/b/c) を追加確定した。**決定0・1-a・1-b・1-c・2・4・6-a・6-b・6-c を実装済み**（決定3・5 は元々 no-op/記録専用）。`cargo xwin check/build --tests/clippy --lib -D warnings` は全てクリーン、`cargo test -p awase-windows --lib`（427件）と全 architecture guard / golden / drift-correction-replay / journal-replay 系テスト（計77件）が成功。**Windows 実機での再現・検証・ソークは未実施。** 決定3-c（GJI warmup キー再選定）と決定4-b（enforce-OFF の settle-retry 化）は本 ADR のスコープ外として未着手のまま残る。詳細は `docs/known-bugs.md` BUG-69 の「実装済み（2026-08-21 追記）」節を参照。

## コンテキスト

### 発端

BUG-34 横展開（追補4、eisu ガード撤去）完了直後、ユーザーから「`send_eager_warmup` も GJI（Google 日本語入力）なら要らないのでは」という疑問が出た。調査の結果、逆に GJI にこそ必要な機構であること（BUG-02: TSF composition context の cold-start 対策）が判明したが、そこから「eager warmup・drift correction・TsfNative force-on ブロックの3機構は互いに冗長/衝突していないか」という監査に発展し、BUG-69（`docs/known-bugs.md`）として記録した。本 ADR はその決定を正式化する。

### 監査対象の3機構

1. **eager TSF warmup**（`Output::send_eager_tsf_warmup`, `output/mod.rs`）: `VK_DBE_HIRAGANA` を先回り送信し、TSF composition context の cold-start によるリテラル出力（BUG-02 系）を防ぐ。
2. **drift correction**（`ir_apply_drift_correction`, `ime_refresh.rs`）: `desired != observed` が閾値以上続いた場合に実 VK を再送する。**本 ADR では KEEP AS-IS**（後述）。
3. **TsfNative force-on ブロック**（`ir_post_focus_change_snapshot` 内）: `applied_ime_on && new_profile_is_tsf_native && !ime_apply_should_defer()` のとき `VK_IME_ON` を `shadow_on` を無視して強制送信。

### 用語

- **`applied`**（`ImeModel.applied: AppliedImeState`）= 「awase が OS へ actuation を行い、その結果がこうだった」という **actuation の記録**。
- **`effective_open()`**（`state/platform_state.rs:498-500`）= 「IME は今開いていると awase は考えている」という **belief**。

F2（後述）の本質は、この2つを取り違えた1行がある、という点にある。

### 確定した事実（F1〜F7）

- **F1: TsfNative force-on ブロックは（`skip_imm_query == true` の経路では）到達不能。**
  `ir_post_focus_change_snapshot` は `focus.focus_changed`（= プロセス変更）のときにのみ呼ばれ（`ime_refresh.rs:194-196`）、その直前の同一 tick で focus settle barrier（TsfNative は 200ms）が armed 済み（`ime_model.rs:516-523`、`app_ime_policy.rs:140-149`）。このブロックの前提 `new_profile_is_tsf_native` は必ず `skip_imm_query == true` を伴う（`can_use_imm32_cross_process()` が `Standard` でのみ true）ため、Stage1→Stage3 の間にブロッキング呼び出しが1つも無く（`ImeDiagnosticSnapshot::capture()` は `if !skip_imm_query` でスキップ）、`!ime_apply_should_defer()` は常に false。再試行のスケジュールも無い。
  追加の不備: このブロックは `apply_ime_open_with_applied` の戻り値を `let _ =` で捨てており、`on_ime_apply_complete` を呼ばない。仮に到達しても apply 完了が `applied` にも journal にも記録されない（決定2の追加根拠）。

- **F2（核心）: `mirror_applied_open(effective_open(), tick_ms)` が、実際には何も apply していないのに `applied` を `Confirmed{open: belief}` へ書き換える。**
  `tick_ms`（`GetTickCount64`）は常に非ゼロなので、`mirror_applied_open_with_ts` の規約上必ず `Confirmed` になる。これは**実装事故ではなく契約の誤用**である——ADR-044 §「状態遷移」が `mirror_applied_open(v) → Confirmed{open:v, at_ms:now}` を意図した契約として明文化しており、問題はその契約を「apply していない文脈」で呼んだこと＝ belief を actuation の記録として書いたことにある。
  `focus_tracking.rs:399-406` が TsfNative を hard pre-sync から明示除外している理由（「TsfNative は SSOT model: `applied=Unknown` のまま維持し、最初のキーで実際の SetOpen を発行する」）と真っ向から矛盾する。Stage1 が意図的に `Unknown` に残した `applied` を Stage3 が同一 tick 内で上書きしている。
  この偽装が `GjiDirectStrategy::apply`（`ime_controller.rs:108-113`）の `if open && view.control.shadow_on { return AlreadyMatched; }` を誤発火させる。F1 の force-on ブロックはその誤発火を打ち消すワークアラウンドとして書かれていた（＝ F1 は F2 の対症療法であり、その対症療法自体が到達不能）。
  `apply_force_on_for_imm_broken`（BUG-16 の修正）のスパムガードが `Confirmed{open:true}` で早期 return するため、TsfNative では事実上恒久的に不発になる。BUG-16 が塞いだはずの「settle 明け再試行が『何もしない関数』の再試行だった」失敗が、別経路で完全に再現している。
  副作用（未記載だった）: `mirror_applied_open_with_ts` は `pending.target == value` のとき `pending` も clear する。`ImeEvent::FocusChanged` の reducer は `applied` は `Unknown` にするが `pending` は clear しないため、この行はフォーカス跨ぎで残った `pending` を副次的に掃除してもいた（決定1で除去する際、D-prep の期限切れ purge・1秒が回収するため恒久固着にはならない）。

- **F3: TsfNative + GJI + TSF 注入モードのフォーカス復帰時、OS に実際に届く actuation は eager warmup だけになる（スコープ限定）。**
  「他の全 actuation は `ime_apply_should_defer()` でゲートされる」は一般命題としては誤り。同一関数内に defer 非ゲートの actuation が他に2つある: (1) enforce-OFF ブロック（決定4で扱う）、(2) `self.platform.gji_on_focus_change(mode)` — `InjectionMode::Unicode` かつ long-cold の分岐で `send_f22_f21_reinit()` という実 VK 送信が走る。ただし (2) は Unicode 限定（TsfNative は `InjectionMode::Tsf`）、(1) は `!new_profile_is_tsf_native` が条件なので、**結論そのものは TsfNative + Tsf 注入というスコープで生き残る**。`reschedule_ime_refresh` は TsfNative で周期リフレッシュを恒久停止するため、force-ON には打鍵以外のトリガーが `schedule_settle_retry` の一発しか無い。

- **F4: eager warmup の危険性は「受容中の既知リスク」——直すべき欠陥ではない。**
  `VK_DBE_HIRAGANA` は `MapVirtualKeyW(VK_DBE_HIRAGANA, MAPVK_VK_TO_VSC)` の**実行時値**（日本語106/109では0x70、レイアウト依存であり定数ではない）を `wScan` に載せて送信され、「開く」と「ひらがなに強制する」を1つの副作用に束ねている（BUG-50 の前提）。BUG-15 追補7 は「IME モードキーの注入は実 IME が確実に ON でない限りしてはならない」とこの危険性を名指しで警告しているが、`can_warmup()` は belief のみを見て real state を一切参照しない。ただし決定3のとおり、この安全則は TsfNative では原理的に満たせない（`FeedbackPolicy::Blind` ＝実 open 状態の読み戻し手段が構造的に無い）。

- **F5: 「TsfNative」はコード上3つの別軸である。**

  | 軸 | 実装 | 該当クラス |
  |---|---|---|
  | (a) `AppImeProfile` | `from_class_name`（`focus/class_names.rs:142-150`）。`IMM32_UNAVAILABLE_CLASSES` を先に評価 | `TsfNative` になるのは `org.wezfurlong.wezterm` と `Windows.UI.Input.InputSite.WindowClass` の2つだけ |
  | (b) `ImePolicyProfile` | `From<AppImeProfile>`。`focus_settle_ms` / chain / feedback を決める | (a) の写像。`caps()`（`state/app_ime_policy.rs:110-148`）で `ImmCross=100` / `Imm32Unavailable=500` / `TsfNative=200` |
  | (c) `is_effectively_tsf_native` | `profile == TsfNative \|\| is_tsf_native_window(class)`（`class_names.rs:75-77`） | 上記2つ + `CASCADIA_HOSTING_WINDOW_CLASS`（Windows Terminal）・`Windows.UI.Core.CoreWindow`・`XamlExplorerHostIslandWindow` |

  **決定1が使うべきなのは (c) である**（`AppImeProfile::TsfNative` という単純 match を使うと Windows Terminal を誤って除外する——2026-07-05 に enforce-OFF ブロックで実際に踏んだ罠と同型、`ime_refresh.rs:653-666` 参照）。
  帰結（ソーク手順に直結）: 決定1が作る「`applied=Unknown` の窓」を閉じる settle 明け再試行（`focus_settle_ms + 50`）は2系統に分かれる: **250ms 系**（WezTerm / InputSite、(a)=TsfNative → settle 200ms）と **550ms 系**（Windows Terminal / CoreWindow / XamlExplorerHostIslandWindow、(a)=Imm32Unavailable → settle 500ms、しかし (c)=true なので決定1の対象）。

- **F6: `mirror_applied_open` を「apply していないのに呼ぶ」サイトは `ir_post_focus_change_snapshot` だけではない。全数:**

  | # | サイト | 実 apply を伴うか | 扱い |
  |---|---|---|---|
  | 1 | `ime_refresh.rs:431-433`（Stage3） | 伴わない | 決定1で TsfNative 時にスキップ |
  | 2 | `focus_tracking.rs:409`（非 TsfNative hard pre-sync） | 伴わない | KEEP（決定5、根拠は限定付きで今も生きている） |
  | 3 | `runtime/mod.rs:1017-1019`（`process_deferred_keys`） | 伴わない（ただし本番到達不能なデッドコード、決定5参照） | 修正不要 |
  | 4 | `key_pipeline.rs:891`（shadow toggle OFF、ImmCross async 前） | 伴う | KEEP・ラベル不一致のみ（決定5） |
  | 5 | `ime_refresh.rs:786-788`（drift correction、ImmCross） | 伴う | KEEP（`ts=0` で正しく `Optimistic`） |
  | 6 | `platform_state.rs:817`（`record_ime_apply_result`） | 伴う | KEEP（本来の契約サイト） |

- **F7: `executor.rs:708` の `applied_for_engine_key` は warmup 入力ではない。** `send_engine_state_ime_key` が VK_F4/VK_F3 を送るかの actuation 判断であり、`applied` を読むのが正しい。決定1の対象外。

**要約**: 死んでいる2つの機構（F1 の force-on、F2 により無効化された BUG-16 修正）を、未監査の副作用を持つ3つ目の機構（eager warmup）が偶然カバーしていた。

### ADR-087 との矛盾（本 ADR 起票の直接の理由）

[ADR-087](087-open-belief-actuation-warrant-separation.md)（IME open/close belief と actuation の根拠の分離、Phase 3 配線は未着手）は、`FeedbackPolicy::Blind`（TsfNative / Imm32Unavailable）向けの意図失効条件 (c) を設計する際、次の連言を前提に「(c) は一度も発火しない」と結論していた:

> (i) `AppliedImeState` が `Confirmed` に遷移する契機が無い **かつ** (ii) `FocusChanged` が `applied` を `Unknown` にリセットする

**両方とも現在のコードでは成立しない。**

- **(i) は F2 とは独立に、それ以前から誤っていた。** `record_ime_apply_result`（`state/platform_state.rs:774-824`）は `outcome ∈ {Applied, FallbackSent, AlreadyMatched}` のとき `feedback` の種別によらず `mirror_applied_open_with_ts(effective, ts)` を呼ぶ（`ts` は非ゼロ）。`Blind` プロファイルでも実 actuation は走る（`CHAIN_GJI_ONLY` / `CHAIN_MS_IME_ONLY`）ので `ImeApplySucceeded` は普通に届く。とりわけ `AlreadyMatched`（＝何も送っていない）も `effective = open` にマップされるため `Confirmed` になる。
  **機械可読な反証**: `crates/awase-windows/tests/golden_scenarios.rs:317` の `apply_succeeded_with_matching_generation_updates_applied` は `ImeApplyRequested{gen:5}` → `ImeApplySucceeded{gen:5}` だけで `model.applied.applied_open() == Some(true)` を assert しており、mirror も Stage3 も一切通らずに **Linux で今日も green**。
- **(ii) は文言としては真だが、F2 が同一 tick 内で無効化していた。** 決定1でこの無効化は TsfNative について解消される。

**したがって、決定1（F2 の修正）を適用しても ADR-087 の旧前提 (i) は回復しない——F2 は「前提が崩れていることを発見する契機」だったのであって、前提を崩した張本人ではない。** ADR-087 Phase 3 は、決定1-a + 決定1-c 適用後「`FeedbackPolicy::Blind` で `AppliedImeState` が `Confirmed` になる契機は『実際に actuation を試みた結果』だけになる（`record_ime_apply_result` 経由。ただし `Failed` が `Confirmed{open: !open}` を書く非対称は残る、下記 B-5 参照）」という形で意図失効条件 (c) を再評価する必要がある。ADR-087 側の該当箇所（§7 round3 の原文と、既存の2026-08-21付「前提の訂正」追補の両方——後者は誤りの原因を F2 だけに帰しており「決定1を適用すれば前提が回復する」と読めるため、併せて訂正する）は、決定1の実装案が固まった時点で行う。

## 決定

### 決定0: 不変条件 INV-A97-1（書き込み側）/ INV-A97-2（読み出し側）

**INV-A97-1**: `ImeModel.applied` は「実際に OS への actuation を試みた」経路だけが書いてよい。belief を `applied` へ写す書き込みは、その値を根拠として読む下流（スパムガード・`shadow_on` チェック・warmup ゲート）に対して belief を evidence に偽装するため禁止する。この不変条件は既に `focus_tracking.rs:399-406` が TsfNative について局所的に守っていたものを全体規約へ昇格させたもの。

**INV-A97-2**: eager warmup の `ime_on` 入力は、`applied` が既知ならそれを、`Unknown` のときだけ belief へフォールバックする**単一の解決関数**からしか作ってはならない。呼び出し側が `applied_open()` の生値を直接渡してはならない。

INV-A97-2 が重要な理由: 「belief を直接読むよう付け替える」という原則を�137箇所に個別適用すると、棚卸しの正しさに依存し1箇所でも漏らせば BUG-02 系の retreatが再燃する（実際、この討議のラウンド1がまさにこの漏れを起こした）。「解決関数が1つ」という形にすることで、将来サイトが増えても構造的に守られる。`applied` は belief への書き戻しをしない——読む向きだけ belief を参照する。この向きの違いが F2 との決定的な差である。

### 決定1: `mirror_applied_open` の TsfNative スキップ・warmup 入力の全数付け替え・force-ON 再試行の有界化

3点セット（同一コミットで入れる。理由は後述）。

#### 決定1-a: Stage3 の mirror を `is_effectively_tsf_native` でスキップする

`ime_refresh.rs:431-433`:

```rust
// 変更前
let ime_on_now = self.platform_state.ime.effective_open();
let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
self.platform_state.ime.mirror_applied_open(ime_on_now, tick_ms);

// 変更後（ADR-097 INV-A97-1）
// 判定は `AppImeProfile::TsfNative` の単純 match ではなく
// `is_effectively_tsf_native` を使うこと——Windows Terminal
// (CASCADIA_HOSTING_WINDOW_CLASS) は AppImeProfile 上は Imm32Unavailable に
// 分類されるため、単純 match だと誤って対象から漏れる
// (2026-07-05、enforce-OFF ブロックで実際に踏んだ罠と同型、F5 参照)。
let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
let new_profile_is_tsf_native = crate::focus::class_names::is_effectively_tsf_native(
    self.platform.current_app_profile(),
    self.platform.focus.class_name(),
);
if !new_profile_is_tsf_native {
    let ime_on_now = self.platform_state.ime.effective_open();
    self.platform_state.ime.mirror_applied_open(ime_on_now, tick_ms);
}
```

`new_profile_is_tsf_native` の算出（現行 `:447-450`）を mirror の前へ移動する。前倒しは安全: 間に挟まる `mark_composition_cold_focus_change()` / `gji_on_focus_change(mode)` / `drain_journal_entries()` はいずれも `current_app_profile()` も `focus.class_name()` も動かさない。

兄弟ブロックへの波及は無い: 同関数の `applied_ime_on`（`:452-459`）は mirror の結果を読み直しており、TsfNative では `Unknown.unwrap_or(false) = false` になる。これを読むのは (i) force-on ブロック（決定2で撤去）と (ii) enforce-OFF ブロック（`!new_profile_is_tsf_native` が排他条件なので決定1が触る集合と交わらない、挙動不変）の2つだけ。

#### 決定1-b: `WarmupImeOn` 型と単一解決関数で warmup 入力サイトを全数付け替える

**型は `awase` lib クレート（`src/platform.rs`）に置く。** `crates/awase-windows` 側に置いてはならない——`on_passthrough_key`/`on_reinject_key` はプラットフォーム非依存 `awase` lib の `TsfComposition` トレイトメソッド（`src/platform.rs:262` にトレイト、`:297`/`:311` に該当2本）であり、依存方向が `awase-windows → awase` の一方通行（`Cargo.toml:16`）のため、`awase-windows` 側の型はトレイト定義に書けない。`TsfComposition` の実装は `WindowsPlatform` ただ1つで `awase-macos`/`awase-linux` は参照しないため、シグネチャ変更で他プラットフォームは壊れない。

```rust
// src/platform.rs、ImeOpenOutcome の直後、ForegroundInfo の前に挿入
/// eager TSF warmup に渡す「IME が開いている」という根拠（ADR-097 決定1-b、BUG-69）。
///
/// `Option<bool>` ではなく専用型にする理由: warmup 経路は `applied` が
/// `Unknown` のとき `None → unwrap_or(false)` に潰れる。決定1-a で TsfNative
/// の `applied` がフォーカス復帰後も `Unknown` のまま残るようになると、この
/// 潰れが「belief は ON なのに warmup が送られない」窓を複数箇所に開ける。
/// かといって呼び出し側に生の belief を渡させると、belief が evidence として
/// 再流入する BUG-19/33/48/68/69 と同じ欠陥を量産する。値の作り方を
/// コンストラクタ3種に限定し、フィールドを private にすることで
/// 「生値を渡す」経路をコンパイラで塞ぐ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarmupImeOn(bool);

impl WarmupImeOn {
    /// 実 actuation の結果として得た確定値（`on_ime_applied` の `effective` 等）。
    #[must_use]
    pub const fn from_actuated(open: bool) -> Self {
        Self(open)
    }

    /// `applied` があればそれを、`Unknown` のときだけ belief を使う（`applied ?? belief`）。
    #[must_use]
    pub const fn from_applied_or_belief(applied: Option<bool>, belief_open: bool) -> Self {
        match applied {
            Some(open) => Self(open),
            None => Self(belief_open),
        }
    }

    /// 「IME 状態不明・warmup しない」。到達不能な保険経路専用。
    #[must_use]
    pub const fn off() -> Self {
        Self(false)
    }

    #[must_use]
    pub const fn is_on(self) -> bool {
        self.0
    }
}
```

**トレイトシグネチャ変更（`src/platform.rs`）**: `TsfComposition::on_passthrough_key`（`:297-304`）と `on_reinject_key`（`:311-317`）の `_applied_ime_on: Option<bool>` を `_warmup_ime_on: WarmupImeOn` に変更。

**実装側の変更（すべて `crates/awase-windows/src/platform.rs` および `output/mod.rs`、引数型を `Option<bool>` → `WarmupImeOn` に変更）**: `impl TsfComposition for WindowsPlatform` の `on_passthrough_key`/`on_reinject_key`、`dispatch_composition_response`、`feed_composition_event`、`composition_confirm_key_up`、`composition_ctrl_up`、`composition_native_f2_down`、`send_eager_warmup`、`output/mod.rs` の `send_eager_tsf_warmup`、`tsf_readiness`。`tsf_readiness` だけは `ime_on: warmup_ime_on.is_on()` になり `unwrap_or(false)` はこの1箇所だけに残る。`output/mod.rs` の「`None` で latch にフォールバック」という stale な doc コメントも削除する。

**唯一の解決関数（`state/platform_state.rs`、`ImeStateHub`）**:

```rust
pub(crate) fn warmup_ime_on(
    &self,
    applied: crate::state::AppliedImeState,
) -> awase::platform::WarmupImeOn {
    awase::platform::WarmupImeOn::from_applied_or_belief(
        applied.applied_open(),
        self.effective_open(),
    )
}
```

`applied` を引数で受け取る（hub の live な `model().applied` を直接読まない）のは、executor が意図的にスナップショットしている意味論（`applied_snapshot`、batch 内で `Optimistic`/`Confirmed` に更新されうる）を壊さないため。

**なぜ belief 単独ではなく `applied ?? belief` か**: `applied ?? belief` は今日 warmup が飛んでいた瞬間で warmup を決して減らさない（単調性）。belief 単独だと「`applied=Some(true)` だが観測由来で `effective_open()=false`」の瞬間に warmup が消え、連続タイピング中に低信頼な OFF 観測が1つ紛れ込むだけで BUG-02 のリテラル化を再燃させうる。

**呼び出しサイトの全数付け替え（実コードで確認済み、10箇所）**:

| # | 場所 | 現在の値 | 変更後 | 理由 |
|---|---|---|---|---|
| 1 | `executor.rs:568` `composition_confirm_key_up` | `applied_snapshot.applied_open()` | `ime.warmup_ime_on(self.applied_snapshot)` | 打鍵駆動の warmup |
| 2 | `executor.rs:582` `composition_ctrl_up` | 同上 | 同上 | 「この→kお」対策 |
| 3 | `executor.rs:602` `on_passthrough_key`（呼び出し元 `handle_confirm_key_passthrough`） | 同上 | 同上 | 確定キー KeyDown |
| 4 | `executor.rs:643` `on_reinject_key`（物理 F2/TSF） | 同上 | 同上 | 再注入キー |
| 5 | `executor.rs:662` `on_reinject_key`（confirm キー） | 同上 | 同上 | 同上 |
| 6 | `platform.rs:1149` `feed_composition_event(comp_event, Some(effective))`（`on_ime_applied` 内） | `Some(effective)` | `WarmupImeOn::from_actuated(effective)` | 実 actuation の結果、bit-identical |
| 7 | `platform.rs:1157` `send_eager_tsf_warmup(Some(effective))`（同上、2発目の warmup） | 同上 | 同上 | 同上 |
| 8 | `ime_refresh.rs:516` `send_eager_warmup(applied_open)`（Site A 自身の warmup） | `applied.applied_open()` | `self.platform_state.ime.warmup_ime_on(applied)` | **ラウンド1が「決定1が warmup を殺す」と指摘した当該箇所。`from_actuated` で機械的に潰すと退行するため必ず `warmup_ime_on()`（=`from_applied_or_belief`）を使うこと** |
| 9 | `key_pipeline.rs:1733` `composition_native_f2_down(applied_open)`（物理 F2） | 同上 | `ime.warmup_ime_on(...)` | 同上の理由で `from_applied_or_belief` |
| 10 | `vk_send.rs:531` `send_eager_tsf_warmup(None)` | `None` | `WarmupImeOn::off()` | **今日は必ず no-op（到達不能な保険経路）。`from_applied_or_belief` にすると belief=ON で今まで一度も飛ばなかった warmup が新規発火するため `off()` 固定が必須** |
| 11 | `platform.rs:658` `feed_composition_event(FocusChange{..}, None)` | `None` | `WarmupImeOn::off()` | `FocusChange` arm は `EmitWarmup` を出さない（`composition_fsm.rs:181-189`）ため don't-care だが、明示的に `off()` にして意図を残す |

サイト1〜5の5つの private fn には `ime: &ImeStateHub` を引数追加する（呼び出し元に `ime` が在ることを確認済み: `run_passthrough_pipeline`/`try_pending_warmup_on_keyup`/`handle_ctrl_up_recovery`/`handle_confirm_key_passthrough` は `execute_relay` から、`handle_reinject` は `execute_one` から、どちらも `ime` を引数に持つ）。

`send_engine_state_ime_key`（`src/platform.rs:250`、`applied: Option<bool>`）は**変更しない**——warmup ではなく Engine 状態変化時のモードキー送信判定であり、`applied` が `Unknown` のとき belief にフォールバックすべきではない（「apply が既に IME 状態を確定させている場合は追加送信が有害」というロジックは「実際に apply したか」だけを見る必要がある）。

#### 決定1-c: force-ON の再試行をクールダウンで有界化する（20ms 無限ループの封鎖）

**この修正は決定1-a と同一コミットに入れなければならない。** 決定1-a 単独では以下のループが開く:

1. 決定1-a により TsfNative の `applied` は `Unknown` のまま。
2. settle 明けに `apply_force_on_for_imm_broken`（`runtime/mod.rs:678`）がスパムガード（`:700-707`、`Optimistic(true) | Confirmed{open:true}` のみを見る）を通過し `force_on_and_correct_romaji` を実行。
3. chain が `Failed` を返すと `record_ime_apply_result`（`state/platform_state.rs:812-817`）が `effective = !open = false` として `mirror_applied_open_with_ts(false, ts)` → `applied = Confirmed{open:false}`。`Failed` は TsfNative で到達可能——chain は belief 由来の `ImeKindId` で選ばれる（`CHAIN_GJI_ONLY`/`CHAIN_MS_IME_ONLY`）一方、`GjiDirectStrategy::is_applicable` は observed CLSID スナップショットを見るため、observer が揺れている間は食い違い得る。
4. `Confirmed{open:false}` はスパムガードを素通り。
5. `on_ime_apply_complete`（`runtime/mod.rs:410`）は outcome によらず `post_ime_refresh()` を呼ぶ（`:436`）→ 20ms タイマー。TsfNative では `reschedule_ime_refresh`（`:604-610`）が周期リフレッシュを早期 return するため 20ms を上書きするものが無く、**ループは実効 50Hz で回る**。
6. `on_ime_applied` の `if open`（`platform.rs:1153`）の `open` は**引数**（force-ON では常に `true`）なので `Failed` でもこの枝に入り、`mark_composition_cold(SetOpenTrue)` と2発目の eager warmup が毎回飛ぶ。**打鍵中かどうかを問わず cold-mark を 50Hz で撃ち続ける**という、BUG-31（Teams で文字消失）族の最悪形。

今日この症状が出ない理由は決定1-a が外そうとしている偽装（F2）そのものである。

**採る設計**: BUG-68 の `DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS`（`state/ime_actuation.rs:176-200`、`tuning.rs:289`）と同型の**クールダウンのみ**（試行回数上限は設けない——理由は下記）。

```rust
// state/ime_actuation.rs、blind_rearm_cooldown_elapsed の直後
/// force-ON（`apply_force_on_for_imm_broken`）の直近試行時刻（ADR-097 決定1-c、BUG-69）。
///
/// `ImeEvent::FocusChanged` でリセットする＝クールダウンの単位は「1 フォーカス」。
/// 試行回数の上限は**設けない**——`FocusChanged` はプロセス変更時にしか発火せず
/// （`focus_tracking.rs:197` の PID 比較）、同一プロセス内のウィンドウ/タブ切替
/// では 1 セッションが数十分続きうる。その間 `applied` は drift correction・
/// ユーザー明示操作等で繰り返し blocking 状態から外れる（F6 参照。
/// `process_deferred_keys` は本番到達不能なデッドコードのため対象外——
/// 決定5参照）。1 フォーカスあたりの試行回数に上限を設けると、observer
/// の揺れ（実測: GJI VK 受付 181ms、Chrome TSF 再初期化 326ms、F22 コールド
/// ~750ms）がクールダウン窓より長く続いた場合に予算を使い切り、そのプロセスに
/// 居る限り force-ON が二度と飛ばなくなる——BUG-16 の原症状
/// （settle 明け再試行の恒久 no-op）を作り直すことになる（ラウンド3 レビュー
/// で発見・rejected）。止めるべきは再試行そのものではなく再試行密度であり、
/// クールダウン単独で十分（50Hz → 1/3Hz 未満に落ちれば cold-mark の連打は消える）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ForceOnRetryState {
    last_attempt_ms: u64,
}

impl ForceOnRetryState {
    pub fn note_attempt(&mut self, now_ms: u64) {
        self.last_attempt_ms = now_ms;
    }
}

/// force-ON を今送ってよいか（ADR-097 決定1-c、BUG-69）。
#[must_use]
pub fn force_on_attempt_allowed(
    applied: AppliedImeState,
    retry: ForceOnRetryState,
    now_ms: u64,
    cooldown_ms: u64,
) -> bool {
    // (1) 既に ON を apply 済み → 送らない（500ms poll ごとの F2 再送スパム防止、BUG-16 由来）。
    if matches!(
        applied,
        AppliedImeState::Optimistic(true) | AppliedImeState::Confirmed { open: true, .. }
    ) {
        return false;
    }
    // (2) 未試行（last_attempt_ms == 0）は無条件に通す。
    //     決定1-a 適用後、TsfNative フォーカス復帰直後は必ずここを通る——これが決定1の主目的。
    if retry.last_attempt_ms == 0 {
        return true;
    }
    // (3) クールダウン: 20ms リフレッシュ連鎖に相乗りした高速再試行を潰す。
    //     GetTickCount の巻き戻りでは saturating_sub が 0 になり安全側（送らない）に倒れる。
    now_ms.saturating_sub(retry.last_attempt_ms) >= cooldown_ms
}
```

`ImeModel` に `force_on_retry: ForceOnRetryState` を追加し、`FocusChanged` reducer arm の `self.applied = AppliedImeState::Unknown;`（`ime_model.rs:509`）の**直後**でリセットする。`AppliedImeState::Unknown` の本番書き込みは `ime_model.rs:212`（init）と `:509`（FocusChanged）の2箇所だけ（grep で確認済み）であり、リセットをこの2箇所に併置する限り「予算はリセットされるが `applied` はされない」逆向きの破れは構造的に存在しない。

`ImeStateHub` に `force_on_attempt_allowed(now_ms)` / `note_force_on_attempt(now_ms)` を追加し、`apply_force_on_for_imm_broken`（`runtime/mod.rs:700-724`）のスパムガード部分を差し替える:

```rust
let tick_ms = crate::hook::current_tick_ms();
if !self.platform_state.ime.force_on_attempt_allowed(tick_ms) {
    return;
}
let outcome = self.force_on_and_correct_romaji(
    crate::state::ime_event::OpenApplyReason::ImmBrokenForceOn,
);
// UnsafeToToggle（Win キー保持等の genuine skip）は「送っていない」ので
// クールダウンの起点にしない——数えると Win 長押し中に次のフォーカス変更まで
// 再試行できなくなる。この場合 applied も更新されないため、次の 20ms
// リフレッシュがそのまま再試行する（自己回復、既存挙動を保存）。
if outcome != awase::platform::ImeOpenOutcome::UnsafeToToggle {
    self.platform_state.ime.note_force_on_attempt(tick_ms);
}
```

**クールダウン値は新規に発明せず、`DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS` と同じ 3000ms を再利用する**（`tuning.rs`）:

```rust
/// `apply_force_on_for_imm_broken` の再試行クールダウン (ms)（ADR-097 決定1-c、BUG-69）。
///
/// `DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS` と同値・同根拠。実測値ではなく
/// レート制限ポリシーだが、`.claude/rules/tuning-constants.md` の趣旨に合わせ
/// 導出を書く: 20ms 無限ループ（50Hz）を破ることが目的で、既存 in-tree の
/// observer 揺れ実測（GJI VK 受付 181ms・Chrome TSF 再初期化 326ms・F22
/// コールド ~750ms）を確実に上回る必要がある。新規の未実測値（当初案の
/// 250ms）は 3 回上限との組み合わせでのみ成立する短い窓であり、上限を
/// 撤廃した以上、既存前例の 3000ms に揃えるのが安全側かつ「実測なしの
/// 新規定数」を増やさない選択である。
///
/// # ソークで測ること
/// `force-ON (ImmBrokenForceOn): apply_ime_open(true) → Failed` の連続回数
/// と間隔。安定して収束するなら短縮を検討してよいが、実測を伴わない短縮は
/// 行わないこと。
pub const FORCE_ON_RETRY_COOLDOWN_MS: u64 = 3_000;
```

#### この決定1全体（1-a+1-b+1-c）が壊していないことの確認

- **crux（決定1の主目的）は保存される**: `applied == Unknown` かつ未試行は必ず force-ON を通す。決定1-a 適用後の TsfNative フォーカス復帰では `applied` は `Unknown`、`force_on_retry` は `FocusChanged` でリセット済み → 初回の force-ON は必ず飛ぶ。
- **F2 型の偽装を再導入していない**: `Confirmed{open:true}` は実 actuation が起きた結果としてのみ生成される。
- **`UnsafeToToggle` の自己回復は保存される**: `post_ime_refresh()` は `UnsafeToToggle` でも無条件に呼ばれる（`runtime/mod.rs:436`）ため 20ms 後に再試行される。クールダウンの起点にもしないため、Win キー保持中も次のフォーカス変更を待たずに回復する。
- **`ForceOnRetryState` は belief-as-evidence（BUG-19/33/48/68/69）系の6例目にならない**: `last_attempt_ms` は「自分が actuation を試みた時刻」という自己の行為の記録のみで、belief / observations / `effective_open()` へ一切書き戻さない。read-only の suppressor であり閉ループを構成しない。
- **決定1-a の非影響を1件確認**: `ime_refresh.rs:287-289` の `explicit_verify` は `!skip_imm_query` を先に要求するため TsfNative では評価されない。

#### 決定1で新たに warmup / force-ON が飛ぶ瞬間（正直な差分）

`applied ?? belief` は単調に増やす。増えるのは `applied==Unknown && belief==true && conv_mutation_allowed && needs_f2_probe() && is_tsf_mode` の瞬間で、内訳: (1) 決定1-a が新設する TsfNative フォーカス入場の窓 — 今日は F2 の偽装のおかげで同じ値が飛んでいたため実質差分ゼロ。(2) `UnsafeToToggle` 後の窓 — 今日は warmup が飛ばず、決定1後は飛ぶ（BUG-02 方向の改善だが新規 actuation としてソーク対象）。**同一プロセス内フォーカス移動での新規窓は存在しない**——`applied=Unknown` を書く唯一の場所（`FocusChanged` reducer）と `ir_post_focus_change_snapshot` の起動条件は同じ `process_changed` ゲートを共有するため。

force-ON 自体は、決定1-c 適用後は TsfNative フォーカス入場ごとに新たに発火するようになる（これが決定1の目的そのもの、BUG-16 の意図の回復）。

#### 「主たる挙動変化」（副作用の全展開）

`force_on_and_correct_romaji` → `on_ime_apply_complete` → `on_ime_applied`（`platform.rs:1083`）が実行する副作用（すべて `platform.rs` 内、行番号は現行コード）:

| # | 行 | 副作用 | 発火条件 |
|---|---|---|---|
| 1 | `:1112` | `reset_candidate_was_seen()` | `outcome != UnsafeToToggle`（`Failed` を含む） |
| 2 | `:1126` | `ime_mode_fsm.on_set_open_applied(open)` — unconfirmed 化 | `Applied \| FallbackSent` |
| 3 | `:1129` | `ms_ime_gate_give_up.set(false)` | `Applied \| FallbackSent` かつ `open` |
| 4 | `:1138-1139` | `confirm_gate_deadline_override_ms.set(0)` / `bump_shift_conv_guard_gen()` | `Applied \| FallbackSent` かつ `open` |
| 5 | `:1149` | `feed_composition_event(ImeOn/ImeOff)` | `outcome != UnsafeToToggle`（`Failed` を含む） |
| 6 | `:1153` | `mark_composition_cold(SetOpenTrue)` | `outcome != UnsafeToToggle` かつ**引数** `open==true`（`Failed` でも発火） |
| 7 | `:1156` | `receipt.settle(self)` → `gji_on_ime_on(mode)` | 全 outcome（`UnsafeToToggle` も settle 済み） |
| 8 | `:1157` | `send_eager_tsf_warmup(...)` — 2発目の `VK_DBE_HIRAGANA` | `outcome != UnsafeToToggle` かつ**引数** `open==true`（`Failed` でも発火） |

正しい記述: TsfNative フォーカス入場ごとに、settle 明け ~50ms の時点で `VK_IME_ON` + `VK_DBE_HIRAGANA` の2発の送信と、打鍵中かどうかを問わない composition cold-mark、および `ImeModeFsm`/shift-conv-guard/候補ウィンドウフラグの計4種のリセットが挿入される（決定1-c 適用後は3000msに1回以下）。#6/#8 が `Failed` でも発火する性質が、決定1-c が塞ぐループの「cold-mark 連打」の直接原因である。

### 決定2: 到達不能な TsfNative force-on ブロックを撤去する

`ime_refresh.rs:459-490` を撤去する。**「単独撤去は no-op であり危険は無い」という表現は使わない**——実行時は no-op だが、テキスト固定テスト（`crates/awase-windows/tests/architecture_guard.rs`、Linux で 34件 green を実測確認済み）**3本**が確実に落ちる:

| # | テスト | 固定している literal | 期待値の変化 |
|---|---|---|---|
| 1 | `ime_open_actuation_entry_points_are_accounted_for`（`:984`） | `ENTRY_POINTS` の `(".apply_ime_open_with_applied(", 1)` | 1 → 0 |
| 2 | `ir_post_focus_change_snapshot_write_call_sites_are_accounted_for`（`:1519`） | `"apply_ime_open_with_applied("` を1件で固定 | 1 → 0 |
| 3 | `force_write_paths_bypass_gji_shadow_on_via_none_applied`（`:1575`） | `".apply_ime_open_with_applied(order, None)"` を1件で固定 | 1 → 0（残る `build_ime_control_view(None)` の期待値1件だけで ADR-087 INV-28 を守る旨をテスト doc に明記） |

`apply_ime_open_with_applied`（定義 `platform.rs:1305`）の本番呼び出し元は `ime_refresh.rs:485` の1箇所のみ。撤去すると呼び出し元ゼロになるため**メソッドごと削除する**（未使用の force-write API を残すと、後日「良いアイデア」として再利用され同じ危険が再燃する——`.claude/rules/experiment-logging.md` が警告する反転パターン）。削除すると内部委譲していた `ENTRY_POINTS` の `.apply_ime_open_with_belief(` の期待値も 3 → 2 になる（`platform.rs:1313` が消えるため）。ADR-087 §5 item14 の「実 actuation 入口棚卸し表」も同時に更新する。

依存関係: 決定1より先に決定2を単独で入れない。決定1適用後、`shadow_on=false` になった通常 chain と、有効化された `apply_force_on_for_imm_broken` の両方が `VK_IME_ON` を送れるため、このブロックが担っていた機能は失われない。

### 決定3: eager warmup に `ime_apply_should_defer()` を追加しない

`ir_post_focus_change_snapshot` には settle 明けの再入経路が無い（`schedule_settle_retry` は `ir_stage_notify` へは再入するが `ir_post_focus_change_snapshot` へは再入しない、`focus_changed==false` のため）。ここに defer ゲートを足すことは「遅延」ではなく恒久的な除去であり、BUG-02 のリテラル化を確実に再燃させる。現行の3ゲート（`conv_mutation_allowed` / `needs_f2_probe()` / `can_warmup()`）を安全にゲートできる天井として維持する。F4 の残存ハザードは既知の限界として受け入れる（下記）。

3-c（将来の実験、本 ADR では決定しない）: MS-IME の ON キーは 2026-08-06 に `VK_DBE_HIRAGANA` から `VK_IME_ON` へ移行して conv 破壊を解消した。eager warmup で同じ置換ができれば F4 は構造的に消えるが、`VK_IME_ON` が GJI の TSF composition context 再初期化を BUG-02 と同等にトリガーするかは未検証であり実機実験が必要。**実験対象は GJI のみ**——`MsImeDirectStrategy` は `needs_f2_probe()` が false のため eager warmup は元々1度も送られない。`docs/experiments.md` に起票する。

### 決定4: enforce-OFF ブロックに `ime_apply_should_defer()` を追加しない

`ime_apply_should_defer` の doc コメント（`runtime/mod.rs:646-658`）がこのブロックを呼び出し元として名指ししているのは doc の側の誤りである。理由: このブロックが実効するのは `Standard`（ImmCross）のみで、到達までに `ImeDiagnosticSnapshot::capture()` が最大 ~250ms ブロックしうる（ImmCross の settle は100ms）。defer ゲートを足すと「診断キャプチャがどれだけブロックしたか」で発火が決まる非決定的な挙動になり、正常時は不発・IMM がハングして capture が遅延したときだけ発火するという意図と正反対になる。加えて `ImeDiagnosticSnapshot::capture` は BUG-34 が撤去/非同期化の対象としているのと同族の同期 `SendMessageTimeoutW` を含むため、BUG-34 が進めばこのゲートは静かに「常に不発」へ反転する（本ブランチの作業と結合している）。doc コメントから当該記述を削除する（挙動変更なし）。

### 決定5: 記録のみ・本 ADR では修正しない belief 混入サイト

- **`focus_tracking.rs:409`（F6 #2）は KEEP する。** 根拠「`VK_KANJI` トグルの冗長送信防止」は **`Imm32Unavailable`/`TsfNative` に限れば stale だが、`Standard`（素の Win32 アプリ）+ MS-IME では今も生きている**——`CHAIN_IMM_CROSS_THEN_KANJI`（`app_ime_policy.rs:122-127`）が `KanjiToggle` を含むため。ガード自体のコード上のコメント「Imm32Unavailable (Chrome 等) のみ」は実際の条件（`!is_effectively_tsf_native`、`Standard`/`ImmCross`/`Plain`/`Unknown` も通る）と食い違っているため訂正する。**後続 ADR でこの pre-sync を撤去する場合、条件を `matches!(profile, ImmCross | Plain | Unknown)` に絞る形にすること。`!is_effectively_tsf_native` のまま撤去すると素の Win32 + MS-IME で `VK_KANJI` 二重トグルを再燃させる。**
  副次効果として、`Imm32Unavailable`（Chrome/Edge）では、フォーカス入場時点で belief が ON だった場合に限り、この pre-sync が `applied=Confirmed{true}` を書いて force-ON のスパムガードを塞ぐ（「100%不発」ではない——belief がフォーカス**後**に反転した場合は `applied` が `Unknown` のままなので force-ON は発火する）。**決定1は Chrome/Edge のこの不発を直さない**——本 ADR の対象は TsfNative 側の `mirror_applied_open`（`ime_refresh.rs:433`）のみである。
- **`runtime/mod.rs:1017-1019`（`process_deferred_keys`、F6 #3）は本番到達不能なデッドコードであることを確認した。修正不要。** `process_deferred_keys` を呼ぶ前提となる `SyncKeyGate::activate()`/`try_push()`（`hook_state.rs:63/81`）はリポジトリ全体で呼び出し元ゼロ（定義のみ）——`SyncKeyGate::is_active()`/`has_deferred_keys()` は恒久的に `false` を返すため、`message_handlers.rs:135-137` のゲートは一度も通らない。したがって「決定1の効果を部分的に打ち消す既知の限界」という前バージョンの記述は誤りであり撤回する。**ソーク手順から関連項目を削除**——生きていないコードを追わせる指示になっていたため。参考: この書き込みは仮に到達したとしても純粋 belief である（`effective_open()` が第一分岐で `has_user_explicit_intent()` を見るため、直前の `poll_and_classify_ime` の新鮮な観測は explicit-intent 分岐に捨てられる）。将来 sync key gate が再有効化される場合にのみ、`focus_tracking.rs:409` と同型のプロファイル分岐を検討すること。
- **`key_pipeline.rs:891`（F6 #4）はラベル不一致のみ**（コメントは「楽観的 C」だが実際は `Confirmed{false}` を書く。挙動は正しい——直前に `!effective_open()` を確認済みで直後に実 ImmCross apply が走るため実 apply を伴う）。`AppliedImeState::Optimistic` が存在するのに使っていない不整合として記録するのみ。
- **`record_ime_apply_result` の `Failed → Confirmed{open:!open}` と `on_ime_applied` の「`Failed` は belief を汚さない」という非対称**（決定1-c B-5）。同一 outcome に対し state 層と platform 層の扱いが矛盾しているが、修正は全プロファイルの `applied` 意味論に影響するため本 ADR では触れず後続課題とする。

### 決定6（俯瞰的討議による後続増分、決定1〜5と同一スコープで実装する）: `applied` 書き込みの `ts==0` センチネル廃止・呼び出し箇所数ガードの新設・読み出し側の分割

決定0〜5は BUG-69 という1インスタンスを直すが、「belief が evidence として再流入する」というクラス自体は閉じない（F2 はこのクラスの5例目、過去に BUG-19/33/48/68 で独立に4回再発見されている）。ユーザーの依頼を受け、architect/reviewer 2エージェントによる俯瞰的討議を実施した。

**検討して不採用にした案（`ActuationRecord` 型 + `&ActuationOrder` アンカー）**: 全ての `applied` 書き込みを「`ActuationOrder` を提示しないと作れない」型に強制する案を検討したが、reviewer が実コードで以下を確認し不成立と判定した:
- 正典の書き込みサイト `record_ime_apply_result`（`platform_state.rs:774`）には `&ActuationOrder` が構造的に届かない。async 完了は Win32 メッセージ境界を跨ぐため（`handle_wm_async_ime_apply_complete` は wparam/lparam のビットから状態を再構成するだけ）、かつ **force-ON・shadow-toggle 経路（ADR-097 が対象とする経路そのもの）は `generation: None` を渡す**ため generation キーでの照合すら効かない。
- 相乗りを想定した既存の architecture_guard（`actuation_is_only_requested_through_actuation_order`）は `ActuationOrder` の構築を一切制約しておらず、テスト自身の doc コメントが「これを INV-47 遵守の証拠と読むな」と明記している。
- `ActuationOrder::issue` は `pub`、`WarrantContext` は全フィールド `pub`、依存する4型は全て `Default` 構築可能なため、ダミー `ActuationOrder` は約8行で作れる——型の壁ではなく段差でしかない。
- 「例外2件」の想定で設計した `BeliefWaiver` は、実際には F6 の6サイト中4サイト（`ime_refresh.rs:433` の非 TsfNative 分岐・`focus_tracking.rs:409`・`record_ime_apply_result` 自身・`runtime/mod.rs:1019`）が waiver 行きになり、`Option<bool>` が担っていた役割をより高い儀式コストで再現するだけに終わる。

**採用する代替**: 型を新設せず、このリポジトリに既にある2つの実績あるパターン（`Observed<E>`・`record_explicit_intent_call_sites_are_limited_to_real_user_actions` 系の text-scan ガード）に相乗りする。いずれも挙動変更ゼロ・実機ソーク不要。

#### 決定6-a: `ts==0` センチネルの廃止

`mirror_applied_open_with_ts(&mut self, value: bool, ts: u64)`（`state/platform_state.rs:178-187`）を、`ts` のマジック値ではなく**構築子名**で `Optimistic`/`Confirmed` を選ぶ2メソッドに分割する:

```rust
// state/platform_state.rs、ImeStateHub の impl 内
/// 非同期送信済み・未確認の actuation を記録する（`Optimistic`）。
pub(crate) fn record_optimistic(&mut self, open: bool) {
    self.shadow_model.applied = crate::state::ime_model::AppliedImeState::Optimistic(open);
    self.clear_pending_if_matches(open);
}

/// 完了が確認された actuation を記録する（`Confirmed`）。
pub(crate) fn record_confirmed(&mut self, open: bool, at_ms: u64) {
    self.shadow_model.applied = crate::state::ime_model::AppliedImeState::Confirmed { open, at_ms };
    self.clear_pending_if_matches(open);
}

fn clear_pending_if_matches(&mut self, value: bool) {
    if let Some(p) = &self.shadow_model.pending {
        if p.target == value {
            self.shadow_model.pending = None;
        }
    }
}
```

`mirror_applied_open`/`mirror_applied_open_with_ts` は削除し、全呼び出し元（F6 の6サイト）を移行する:

| サイト | 変更前 | 変更後 |
|---|---|---|
| `platform_state.rs:817`（`record_ime_apply_result`） | `mirror_applied_open_with_ts(effective, ts)` | `ts==0` の分岐を保持しつつ内部で `record_optimistic`/`record_confirmed` に分岐（`ts` は `current_tick_ms()` 由来で実質常に非ゼロだが、`UnsafeToToggle` 以外の呼び出し元を精査し `ts==0` を渡すケースが無いことを確認したうえで `record_confirmed` 固定にする——無ければ単純化、あれば分岐維持） |
| `focus_tracking.rs:409` | `mirror_applied_open(true, tick_ms)` | `record_confirmed(true, tick_ms.0)` |
| `runtime/mod.rs:1017-1019`（`process_deferred_keys`、デッドコード） | `mirror_applied_open(observed_ime_on, tick_ms)` | `record_confirmed(observed_ime_on, tick_ms.0)`（デッドコードのため優先度低いが、削除ではなく型を揃える——将来 sync key gate が復活した際に旧 API の亡霊が残らないようにする） |
| `key_pipeline.rs:891` | `mirror_applied_open(false, tick_ms)` | `record_confirmed(false, tick_ms.0)`（コメント「楽観的 C」も `record_confirmed` の実体に合わせて訂正——決定5 が記録したラベル不一致がこの機会に解消される） |
| `ime_refresh.rs:788`（drift correction） | `mirror_applied_open_with_ts(desired, 0)` | `record_optimistic(desired)` |
| `ime_refresh.rs:433`（Stage3、決定1-a でスキップ条件が付く） | `mirror_applied_open(ime_on_now, tick_ms)` | `record_confirmed(ime_on_now, tick_ms.0)`（`!new_profile_is_tsf_native` のときのみ実行） |

**この分割だけで F2 の発生機構が構造的に消える**——「時刻を渡したつもりで `Confirmed` を名乗る」という事故は、`record_confirmed`/`record_optimistic` のどちらを呼ぶかを呼び出し元が明示的に選ぶことでしか発生しなくなる。

#### 決定6-b: `mirror_applied_open` 系呼び出し箇所数を固定する text-scan ガード

`mirror_applied_open`/`mirror_applied_open_with_ts` は34本の architecture_guard のうち**0本にも守られていない**（`rg mirror_applied_open crates/awase-windows/tests/` が no match、実測確認済み）。決定6-a の改名後、新名 `record_optimistic`/`record_confirmed` の呼び出し箇所数を固定するテストを1本追加する。既存の `record_explicit_intent_call_sites_are_limited_to_real_user_actions`（同種の「呼び出し元の数が固定されていなかった穴」を発見・記録した前例）と同型:

```rust
// tests/architecture_guard.rs
#[test]
fn applied_state_recorders_call_sites_are_accounted_for() {
    // F6 (ADR-097) の6サイト + 決定6-a で揃えた呼び出し形。
    // 数が動いたら「新しい belief-laundering サイトが無審査で増えた」
    // 可能性を疑うこと（BUG-20/69 と同型の穴）。
    let production = production_code_only();
    assert_eq!(count_real_calls(&production, ".record_optimistic("), 2, "...");
    assert_eq!(count_real_calls(&production, ".record_confirmed("), 4, "...");
}
```

（正確な期待値は実装時に確定させる——決定1-a/2 の変更後の実サイト数と一致させること。）

#### 決定6-c: `applied_open()` の証拠用・情報用分割

決定1-b により、本番の `.applied_open()` 読み手は 10 箇所から **3 箇所**（`executor.rs:708` `applied_for_engine_key`、`ime_refresh.rs:456` `applied_ime_on`、`message_handlers.rs:444`）まで減る。全て「抑制器/トリガー」（F7 参照）であり、belief フォールバックがあってはならない用途である。

`applied_open() -> Option<bool>` の名前と型はそのまま維持しつつ（3箇所とも既に `Option<bool>` を正しく扱っており、フォールバックしていない）、doc コメントで「これは証拠用アクセサであり belief フォールバックを持たない。情報用途（warmup 等）には `WarmupImeOn`/`warmup_ime_on()` を使うこと」と明記する。**新しい型は導入しない**——残存箇所が3つまで絞られた時点で、コメントによる意図の明文化がコストに見合う唯一の追加である。

### 実装順序・スコープの確認

決定6-a/6-b はソーク不要（挙動変更ゼロ、Linux で完結する text-scan ガードのみ）。決定1〜5 と同一コミットで実装してよい。決定6-c はコメントのみ。

## 保持するもの（変更しない）

- **drift correction は KEEP AS-IS。** 3機構のうち唯一生きている機構であり、ADR-080 の型付き `Actuation`/`FeedbackPolicy::Blind` 設計が BUG-33 型の失敗を構造的に防いでいる。BUG-68 の `DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS` は適切なレート制限（決定1-c がまさにこれを再利用した）。`Blind::backoff` が未消費、`FocusChanged` が `gave_up_at` を破棄する、という2つの軽微な既知の残課題はあるが本 ADR の対象外。
- **eager warmup 自体の存在も、そのゲート条件（現行3ゲート）も KEEP。** BUG-02 の実測（~326ms）は今も有効であり、決定3のとおり現行3ゲートがブロッキング読みを復活させずに取りうるゲートの上限である。
- **`focus_tracking.rs:409` の非 TsfNative hard pre-sync は KEEP**（決定5参照、根拠は Standard/ImmCross に限定して今も生きている）。

## 関連 ADR への影響

- **ADR-087**: 上記「ADR-087 との矛盾」節のとおり、Phase 3 着手前に決定1（F2 の修正）を先に適用し、`FeedbackPolicy::Blind` の意図失効条件 (c) を「`Confirmed` に遷移する契機は実際に actuation を試みた結果のみ（`Failed` の非対称は残る）」という正しい前提で再評価する。ADR-087 §7 round3 の原文と、既存の2026-08-21付「前提の訂正」追補の両方を、決定1の実装案が固まった時点で差し替える。
- **BUG-16**: 本 ADR の F2 は BUG-16 が塞いだ穴が `mirror_applied_open` という別経路で再現していたことを示す。決定1により BUG-16 の修正が TsfNative で初めて実効する（ただし Imm32Unavailable 側は決定5のとおり引き続き部分的に不発）。
- **BUG-19/BUG-33/BUG-48/BUG-68**: 「belief が evidence として再流入する」同一欠陥パターンの過去4例。F2 はその5例目（新設した `ForceOnRetryState` は read-only の suppressor でありこのパターンに該当しないことを検証済み）。
- **BUG-50**: 決定3-c の実験の直接の根拠。
- **BUG-34**: 決定4 は BUG-34 の進行（`ImeDiagnosticSnapshot::capture` の非同期化）と直接結合している。

## 実装順序・テスト

### コミット1（決定1、単独で実機ソークする）

1. `ime_refresh.rs`: `new_profile_is_tsf_native` の算出を前倒しし、mirror を `is_effectively_tsf_native` で条件付きにする（決定1-a）。
2. `WarmupImeOn` を `src/platform.rs` に新設。`TsfComposition` トレイト2本のシグネチャ変更。実装側9箇所・呼び出し側11箇所（上記表）を付け替え。
3. `ForceOnRetryState` / `force_on_attempt_allowed` を `state/ime_actuation.rs` に追加。`ImeModel.force_on_retry` フィールド追加・`FocusChanged` reducer での併置リセット。`ImeStateHub` にアクセサ2本。`apply_force_on_for_imm_broken` の差し替え。`tuning.rs` に `FORCE_ON_RETRY_COOLDOWN_MS = 3_000`。
4. `output/mod.rs` の stale doc（「`None` で latch にフォールバック」）削除。

**Linux で書ける自動回帰**:

- `state/ime_actuation.rs` に `force_on_attempt_allowed` の純粋関数テスト6件（`blind_rearm_cooldown_elapsed` のテスト群に並べる）: (1) `applied=Unknown`・未試行 → true（crux、これが落ちたら決定1が無意味化している）。(2) `Confirmed{open:true}` → false（従来のスパムガード保存）。(3) `Confirmed{open:false}`・経過 20ms → false（20ms ループ封鎖、本 must-fix の回帰テスト）。(4) 同上・経過 3000ms → true（過渡的失敗からの復帰）。(5) 未試行から2回目の呼び出し・経過 3000ms未満 → false（クールダウン継続）。(6) tick ラップ（`now_ms < last_attempt_ms`）→ false（安全側）。
- `crates/awase-windows/tests/architecture_guard.rs` に text-scan ガード1本: `apply_force_on_for_imm_broken` 本体内で `force_on_attempt_allowed(` と `note_force_on_attempt(` の出現回数がそれぞれ1件であること（理由メッセージに「0 になると決定1-c のループ封鎖が外れる」と明記）。

**`state/platform_state.rs`/`platform.rs`/`runtime/`/`tsf/` は `#[cfg(windows)]` のため、`mirror_applied_open` の呼び出し有無・`shadow_on` の値・`WarmupImeOn` の解決結果を Linux で直接検証することはできない。** `ime_model.rs`（ungated）で書ける `reduce(FocusChanged)` → `applied==Unknown` は現行コードでも既に真でありF2を検出しない。したがって本コミットの `fix-requires-evidence.md` (a) は上記の純粋関数テスト6件 + architecture_guard 1件に限定し、(b)（`docs/known-bugs.md` BUG-69 への追記）を実挙動の記録として主とする。

### コミット2（決定2、コミット1のソーク完了後）

force-on ブロック撤去 + `apply_ime_open_with_applied` 削除 + `architecture_guard.rs` 3件の期待値更新（`ENTRY_POINTS` の `.apply_ime_open_with_belief(` 3→2 を含む）+ ADR-087 §5 item14 表更新。

### コミット3（決定4のdoc修正、いつでも可・挙動変更なし）

## ソーク手順

2系統に分けて実施する（F5参照）。系統A（250ms系）: WezTerm。系統B（550ms系）: Windows Terminal。IME は Google 日本語入力、Engine ON。

| # | 観測項目 | 期待 |
|---|---|---|
| 1 | force-ON がフォーカス入場ごとに発火 | `[ime-apply] ... ImmBrokenForceOn` が出る（決定1-cの実効確認。0回なら決定1が効いていない） |
| 2 | 発火時刻 | 系統A ≈ 250ms / 系統B ≈ 550ms |
| 3 | `→ Failed` の連続と間隔 | 3000ms 間隔を空けて再試行されること。連続する場合はクールダウンが機能していない |
| 4 | VK 送信は2発（`VK_IME_ON` + `VK_DBE_HIRAGANA`） | force-ON 直後に1回のみ。3秒未満で再送が出たら決定1-c の回帰 |
| 5 | 打鍵中の cold-mark（BUG-31族、最重要） | `[composition] ... marking cold` がフォーカス入場直後のみ。連続入力中に出たら決定1は差し戻し |
| 6 | 文字欠落・リテラル化 | 「これ」→ Enter →「で」で `de` にならない（決定1がなければ再燃する所見） |
| 8 | shift-conv-guard の巻き添え | フォーカス入場直後の Shift+記号出し分けが壊れていないこと |
| 9 | Alt+Tab の余分な settle retry | `XamlExplorerHostIslandWindow` 経由で1往復入るが収束すること |
| 10 | `UnsafeToToggle` 窓 | Win+Ctrl+→ で切替→復帰→即打鍵。warmup は飛ぶが force-ON は次の 20ms リフレッシュで再試行されること |
| 11 | GjiFsm 同期（`sync_ime_kind_from_observation`、下記「既知の限界」参照） | TsfNative+GJI でフォーカス直後に `WM_IME_KIND_CHANGED` が届いても `applied=Unknown` のため即時同期しないが、後続の force-ON/明示操作で `ActuationReceipt.settle()` 経由の `GjiFsmSync::OnImeOn` が同期すること。GjiFsm が OffCold に残留し続けたら BUG-18 型の退行 |

## 既知の限界・未検証事項

- **実装済み（2026-08-21）。** 決定0・1-a・1-b・1-c・2・4・6-a・6-b・6-c はコードに反映し、`cargo xwin check/build --tests/clippy --lib -D warnings` 全クリーン、Linux で実行可能な7テストバイナリ（`awase-windows --lib` 427件含む計504件）が全件成功することを確認済み。決定3・5 は元々 no-op/記録専用（下記参照）。実装は決定1（1-a+1-b+1-c）と決定2 を分割コミットせず一括で行った——決定2（force-on ブロック撤去）は F1（到達不能）により実行時 no-op のため、分割しないことによる regression リスクは無いと判断した。
- Windows 実機での再現・検証・ソークは未実施（上記「ソーク手順」は未消化のまま残る）。
- **cooldown 満了後の再武装はイベント駆動であり周期的ではない。** TsfNative では `reschedule_ime_refresh` が早期 return するため、`apply_force_on_for_imm_broken` の再試行は「`may_change_ime` キーの passthrough」（`key_pipeline.rs:1110`）または BUG-51 の `ReportOpenInference`（`key_pipeline.rs:619`）による `schedule_ime_refresh(20)` 経由でのみ再武装される。**ユーザーが打鍵を止めている間は自動的な再試行が発生しない。** これは修正前（force-ON が `mirror_applied_open` の偽装により実質恒久的に不発だった状態）からの後退ではなく厳密な改善だが、「50Hz → 1/3Hz に落ちる」という上記コメント表現は周期ポーリングを連想させ誤解を招く——実態は「継続的な打鍵がある限り最短3秒間隔」である。tuning.rs の doc コメントにこの制約を追記済み。打鍵が無い状態での自動再武装が必要になった場合は、cooldown 満了時に `schedule_ime_refresh` を明示的に再アームする設計を別途検討する（未実装、実測を伴う premortem が必要）。
- 決定1により、Chrome/Edge（`Imm32Unavailable`）の force-ON 不発は直らない（決定5参照）。
- `record_ime_apply_result` の `Failed → Confirmed{open:!open}` と `on_ime_applied` の非対称は未修正（後続課題）。ADR-087 側「前提の訂正」追補も本非対称を明記する形に併せて訂正した。
- `FORCE_ON_RETRY_COOLDOWN_MS = 3_000` は実測なしのレート制限ポリシー（`DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS` と同値を援用）。ソークで安定して収束するなら短縮を検討してよいが、実測を伴わない短縮は行わないこと。
- 決定3-c（GJI warmup を `VK_IME_ON` に置き換えられるかの実機実験）は未実施。`docs/experiments.md` に起票済み。
- 決定1〜2の実装順序を守らないと、単独修正が別の未監査の穴を露出させ regression する構造（BUG-34 追補4の3ラウンド premortem と同型）である。決定1（1-a+1-b+1-cを同一コミット）→決定2の順で進めること（実装ではこの順序で行い、分割コミットはしなかった）。
- **`sync_ime_kind_from_observation`（`runtime/message_handlers.rs:444`）への波及は未検証。** この関数は `applied.applied_open() == Some(true)` を条件に `gji_on_ime_on(mode)`（GjiFsm 遷移トリガー）を呼ぶ。決定1-a により TsfNative では `applied` がフォーカス入場後 `Unknown` のまま残るため、`WM_IME_KIND_CHANGED`（GJI 検出）がこの関数を real actuation より先に呼んだ場合、この経路単独では即時に GjiFsm を同期しなくなる。ただし ADR-089 §2.4（INV-42）の `ActuationReceipt.settle()` → `GjiSyncSink::sync_gji(GjiFsmSync::OnImeOn)`（`platform.rs:1057`）が実際の actuation 完了時に独立して GjiFsm を同期するため、force-ON（決定1-c）や drift correction が一度でも実行されれば追いつくはずだが、この2経路の相互作用は実機で未検証。GjiFsm が OffCold に残り続けたら BUG-18 型の退行として扱い、`sync_ime_kind_from_observation` 側にも `warmup_ime_on()` 相当の belief フォールバックを追加するか検討すること（ソーク項目#11）。
