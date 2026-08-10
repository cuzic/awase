# ADR-087: IME open/close belief における「内部信念」と「actuation の根拠」の分離（根拠軸の規律）

## ステータス

**ドラフト（Phase 0〜2' の純粋ロジックは実装・テスト済み、Phase 3 配線は
未着手。実機ソーク未実施）。Opus・Codex CLI による相互レビューを3巡
（round1〜3、設計レベル）＋タスク分解レビュー1巡＋実装後レビュー1巡
（round4、§8）を実施。round2 で「本 ADR が守ると宣言していた BUG-16
（TsfNative）が改訂版設計で退行する」という重大な欠陥が見つかり
`WarrantBasis::OwnSsot` を新設したが、round3 で**その `OwnSsot` 自体の
判定条件（`AppImeProfile` 分岐）が同じ BUG-16 を別の理由で再発させる**
ことが判明し、`policy.default_feedback` ベースの判定に訂正。round4 では
実装して初めて見える新規欠陥（`IntentStore` の OFF 意図が無期限に固着する、
診断 API `guard_override` が誤情報を返す 等）が見つかり修正済み。
「読んで想像するレビューが振動する」パターンを断つため Phase 0〜2' の
純粋ロジックを Linux 上でテストスイートとして実装し（§8、`docs/known-bugs.md`
BUG-63）、さらに `issue_open_warrant()` の Step 0〜4 ロジックについては
独立に書いたオラクルとの4608通り網羅比較テストで固定した（§8.6・§8.8）。
round4 の実装後レビュー（must-fix 4件）を修正し、round4 最終確認
（Opus）で must-fix ゼロ・「純粋ロジックとしては収束した」との評価を得た
（§8.8）。Phase 3（実配線）・実機ソークは次セッションへ持ち越し。
継続的な精査を歓迎する。**
Windows 実機での動作確認は未実施。本 ADR は ADR-078 の再開ではなく、
ADR-078 が明示的に対象外とした open/close 軸を新たにスコープする独立の ADR である
（理由は §1.5.1）。

---

## 1. コンテキスト

### 1.1 発端となった実バグ

2026-08-10、ユーザーが `Win+Ctrl+→`（仮想デスクトップ切替）で Windows Terminal
（TsfNative、`Windows.UI.Input.InputSite.WindowClass`）にフォーカスを移した直後、
半角のつもりで `mise` と入力したところ、IME が意図せず ON になり、後半が
「くした」というかな変換結果になった（半角入力ができなかった）。

### 1.2 コード読解で確定した原因

`crates/awase-windows/src/state/` の3層:

- `ime_model.rs` — `ImeModel`（`desired_open`/`input_mode` は private、`reduce()` のみが書く）
- `observation_store.rs` — 複数ソースからの観測（`ImeObservation`, confidence: High/Medium/Low）
  を保持し `derive_open()` / `most_recent_trusted()` で集約
- `platform_state.rs` — `effective_open()` 等の外部公開アクセサ

確定した因果連鎖:

```
Win+Ctrl+→
  → FocusChanged  (ime_model.rs:377) last_intent = None          ← 明示意図の記憶が消える
                  (ime_model.rs:380) observations.clear_all()    ← 観測プールが空になる
  → 1打鍵目 KeyDown（M）
     → kp_stage_idle_conv_check (key_pipeline.rs:128) が conv を async 読み
     → classify_conv_transition（conv_classify.rs）
        cm=NATIVE（IME が閉じていても保持される値、BUG-16 原因3） かつ effective_open=false
        → EngineSync::ReportOpenInference(NativeToggleShadowOff)
     → report_conv_open_inference (platform_state.rs:925) → ObserverReported
        { source: ConvOpenInference, confidence: Medium, expires_at: None }
        ← 空のプールに入った唯一の観測
     → derive_open() (observation_store.rs:259) = Some(true)
        ← コメントに明記の通り「Medium+ ソースの無競合多数決（1 ソースでも可）」
     → effective_open() (ime_model.rs:222) = true
        ├→ build_ctx().ime_on = true
        │    → NICOLA engine が「mise」の I/S/E をかな変換 → 「くした」
        └→ is_eligible_for_ime_force_on() (platform_state.rs:456) = true
             → apply_force_on_for_imm_broken (runtime/mod.rs:649)
             → force_on_and_correct_romaji → 実際に VK_DBE_HIRAGANA 等を SendInput
                → OS 側 IME が物理的に ON になる
```

「**FocusChanged が意図の記憶と観測を同時に全消去し、その真空を最初に届いた
最弱の観測が単独で埋める**」という構造が本質である。`desired_open` フィールド
自体は `.claude/rules/ime-belief-architecture.md` の三層分離規律どおり private で
守られているが、全 61 箇所の参照が実際に読むのは `effective_open()` であり、
そちらはこの保護の外にある。

### 1.3 対症療法が原理的に成立しない理由 — BUG-26 との対称性

`docs/known-bugs.md` の **BUG-26**「FocusChanged 直後 conv が既に NATIVE の場合、
idle-conv-check の steady-state 分岐が engine 復帰を永久に見送る」は、**今回と
同じアプリ・同じ conv 観測値（NATIVE, 0x19 系）で、実際の IME 状態が正反対**の
ケースである。

| | BUG-26（2026-07-17） | 本バグ（2026-08-10） |
|---|---|---|
| アプリ | Windows Terminal / InputSite | Windows Terminal / InputSite |
| conv 観測値 | NATIVE | NATIVE（IME が閉じていても保持される、BUG-16 原因3） |
| 実 IME の真の open | **開いていた** | **閉じていた** |
| 正解の挙動 | engine を ON に復帰させる | 何もしない |

**conv ビットには、この2状態を区別する情報が原理的に含まれていない。** BUG-26
の修正記述（`docs/known-bugs.md`）は「`derive_open()` は Medium confidence 単独
ソースでも即採用するため、この観測が記録された時点で engine の `ctx.ime_on` は
すぐに真に復帰する」という**まさにこの挙動に意図的に依拠している**。したがって:

- `ConvOpenInference` の confidence を Low に格下げ → **BUG-26 が確実に再発**
- `derive_open()` の Medium 段に2ソース以上を必須化 → TsfNative には第2の open
  観測ソースが構造的に存在しない（`FeedbackPolicy::Blind` を割り当てている理由
  そのもの）→ **同じく BUG-26 が確実に再発**
- 時間窓での合意形成を必須化 → 同上

**「信頼度モデルの調整」という軸では、この2バグのペアを同時に解決できない。**
情報が観測値に含まれていない以上、観測の重み付けをどう変えても分離できないため
である。これが本 ADR が対症療法（confidence 調整・corroboration 要求）を採用しない
理由であり、後述する「belief と actuation 根拠を型で分離する」方向を選ぶ根拠である。

### 1.4 追加で確認された構造的欠陥

1. **`effective_open()` は2つの異なる目的に同時に使われている**: (a) NICOLA engine
   の内部挙動決定（`build_ctx().ime_on`、誤りは可逆・低コスト）と (b) OS への実際の
   副作用の授権（`is_eligible_for_ime_force_on()` 経由、誤りは不可逆・高コスト）。
   同一の bool フリップが両方を同時に引き起こす。これが本バグの二重の症状
   （かな変換の混入 と 実 IME の望まぬ ON）の共通原因である。

2. **`most_recent_trusted()` に鮮度上限が無い**（`observation_store.rs:214-219`）。
   `derive_open()` は `FRESH = 3s` を持つが、`ObserverReported` の `expires_at` は
   常に `None`（`ime_model.rs:358`）。3秒後に `derive_open()` が `None` に落ちても、
   フォールバック2段目の `most_recent_trusted()` が**同じ ConvOpenInference を
   無期限に返し続ける**。TsfNative には競合する観測ソースが無いため、1件の conv
   推論がフォーカスセッション全体にわたって `effective_open()` を支配し続けうる。

3. **actuation 入口間でガードが非対称**。`check_drift_correction`
   （`platform_state.rs:545`）には既に
   `if trusted.source == ObservationSource::ConvOpenInference && explicit_intent.is_none() { return None; }`
   という正しいガードがある（BUG-19 対策）。**「明示意図が無い間、conv 由来の推論
   単独では actuate しない」という判断はこのリポジトリで既に合意済み**である。
   それが drift correction 側にしか実装されておらず、`is_eligible_for_ime_force_on()`
   側には無い。**なお実際の書き込み入口は2つに限らない**（`is_eligible_for_ime_force_on()`
   の呼び出し元だけで `runtime/mod.rs:668`（observe 経路）/`:825`（force 経路）/
   `:872`（`try_force_on_bootstrap`）の3箇所あり、これに drift correction・
   `EngineSync::DirectInput`・`ir_post_focus_change_snapshot` の直接書き込みを
   加えると6経路以上になる。§5 Phase 3 でこれらを force-write /
   observation-based correction のどちらに分類するかを個別に決める必要がある）。

4. **現行実装は ADR-086 §2.1 の force-write 定義自体に違反している**。ADR-086 は
   force-write を「外部状態の観測結果を**判断材料に使わずに**」行う操作と定義した。
   ところが `conv_mode_policy = force` 時の `consume_force_open_pending`
   （`runtime/mod.rs:813-866`、ADR-086 Phase 3、2026-08-08 実装）は
   `is_eligible_for_ime_force_on()` → `effective_open()` という観測混じりの値を
   ゲートにしている。**これは新方針の提案ではなく、既存の決定（ADR-086）に実装を
   一致させる修正として位置づけられる。**

5. **`consume_force_open_pending` は意図的に settle ガードを持たない**
   （ADR-086 §5 Phase 3 item H1「呼ぶと1打鍵目を必ず取りこぼす」ため）。
   `conv_mode_policy` の既定値は `Observe`（`src/config.rs:289`）だが、`Force`
   運用時はこちらが発火するため、observe 経路（`apply_force_on_for_imm_broken`、
   settle ガード有り）よりもさらにガードが薄い。**どちらの経路で発火したかは実機
   ログでしか確定できない**ため、§5 Phase 0 で診断能力を先に確保する。

6. **`effective_open()` は内部で `Instant::now()` を呼ぶ**（`ime_model.rs:223`）。
   同一 tick 内で2回評価すると異なる値になりうる。同じ問題は force-ON の可否を
   直接左右する `ime_apply_should_defer()`（`runtime/mod.rs:635`）・
   `is_focus_transition_settling(Instant::now())`（`key_pipeline.rs:125`）にもある。
   ADR-082 の journal replay で本バグを再現不能にし、
   `.claude/rules/fix-requires-evidence.md` が要求する回帰テストを、これらの
   経路に対しては書けない状態にしている。

7. **因果連鎖は同一 tick 内で完結しない**。`kp_stage_idle_conv_check`
   （`key_pipeline.rs:348`）は `idle_conv_check_in_flight` を立ててワーカー
   スレッドへ offload する（BUG-34 対策、`key_pipeline.rs:394-406`）。
   `report_conv_open_inference` はこの完了ハンドラ（`apply_idle_conv_check`）から
   後続の tick で dispatch されるため、`effective_open()` が反転するのは
   §1.2 の連鎖図が示唆するような単一 KeyDown の中ではなく、**数打鍵後**である
   （実際の症状が「mise」の**後半**から壊れたことと整合する）。§5 Phase 0 の
   journal フィクスチャは、単一 tick のスナップショットではなく tick をまたぐ
   順序付きイベント列として設計する必要がある。

### 1.5 既存資産との関係

本 ADR は既存 ADR を否定しない。ADR-078/086 が積み上げてきた規律に、
「根拠軸」という第4の軸を足すものである。

| 軸 | 問い | 定めた ADR |
|---|---|---|
| 時間軸 | いつの意図に対する actuation か | ADR-077 epoch admission / ADR-080 epoch fencing |
| 空間軸 | どこへの actuation か | ADR-086 INV-14 target identity |
| トリガー軸 | 何をきっかけに発火してよいか | ADR-086 INV-15 arm-on-focus / fire-on-intent |
| **根拠軸** | **どれだけの証拠に裏付けられて発火してよいか** | **本 ADR** |

| 既存 | 何を定めたか | 本 ADR との関係 |
|---|---|---|
| ADR-078 belief 3分割 | `DesiredMode`/`EffectiveMode`/`ModeConstraint`（conv/mode 軸限定、Phase 1a のみ実装） | **対象外だった open/close 軸を補完する。ADR-078 の再開ではなく新規 ADR とする**（§1.5.1） |
| ADR-081 プロファイル分離 | GJI/MS-IME/TsfNative 等のケーパビリティドライバ分離 | `WarrantBasis` の発行可否をプロファイルの capability から引ける（TsfNative は `DirectRead` basis を原理的に持てない、と宣言できる） |
| ADR-082 journal/event origin | actuation の記録先と origin 追跡 | `WarrantBasis` は journal が記録すべき「なぜこの書き込みが起きたか」の出所そのもの |
| ADR-086 force-write 規律 | トリガー軸（INV-15）・空間軸（INV-14）。**根拠軸は未定義のまま** | 本 ADR の直接の親。INV-20 以降は ADR-086 の番号空間を継承する（理由は ADR-086 §ステータスと同型） |
| `.claude/rules/ime-belief-architecture.md` | Observe → Pure → Apply の三層分離、confidence ガード、3段構えの強制 | 本 ADR が指摘する穴は、この規律の「偽装」ではなく「正規の `ObserverReported` 経路が持つ信頼度モデルの粒度不足」である点で、過去の対策（偽装封じ）と質的に異なる |

#### 1.5.1 なぜ ADR-078 の再開ではないか

ADR-078 の「適用範囲」節は対象を `AppImeProfile` の conv/mode 軸に明記しており、
open/close 軸（`desired_open`/`effective_open`）は一貫して対象外に置かれている。
ADR-078 Phase 1 が未完了なのは「却下された」のではなく「増幅ループの実質的撤去
（Phase 1a）のみ実装し、型分割は着手されないまま」という状態であり、ADR-081
Phase 1d も同様に「実機ソークが取れないから見送り」であって否定されていない。
つまり本 ADR は過去に却下された方向性と矛盾しないが、同時に「大きい設計を起票
したが実機制約で完走できなかった」という前例が2件あることを踏まえ、§5 の移行
計画は小さく検証可能な Phase に分割する。

---

## 2. 決定

### 2.1 用語

**belief（内部信念）**: NICOLA engine の挙動（かな変換するか否か）を決定するための
仮説的な状態。`effective_open()` が現在提供している値はこれに相当する。誤りは
可逆（次の打鍵で気づける）であり、要求する証拠は弱くてよい。BUG-26 はまさに
「弱くてよい」ことに依拠している。

**actuation warrant（外部書き込みの根拠）**: OS 側 IME 状態への実際の書き込み
（`SendInput` による VK 注入、IMC write）を許可する根拠。誤りは不可逆な外部状態
変更であり、他アプリ・他ウィンドウにも波及しうる。要求する証拠は強くあるべき。

**force-write**: ADR-086 §2.1 の定義を継承する。「外部状態の観測結果を判断材料に
使わずに、awase 自身の意図を根拠として外部状態を書き換える操作」。本 ADR の
文脈では、force-write の warrant basis は観測由来であってはならない（§4 INV-20）。

### 2.2 責務の再配置（目標状態）

| 関心事 | 所有コンポーネント | 禁止事項 |
|---|---|---|
| engine の挙動決定（belief） | `effective_open()`（`derive_open()` の Medium 単独多数決は**維持**、BUG-26 非退行のため） | — |
| 外部書き込みの授権（warrant） | 新設 `issue_open_warrant()`（`state/` の純粋関数、`Instant` 引数化） | `is_eligible_for_ime_force_on()` 等の actuation ゲートが `effective_open()` を直接読むこと |
| warrant の発行根拠 | `WarrantBasis`（限定された variant のみ）＋足し算での正当化（P16） | `ConvOpenInference` 単独・`HeuristicDefault`・belief 書き戻し由来の観測**だけ**を basis に含めること／正当な basis（明示意図・force guard）を実装から落として「引き算」にすること |
| 観測の鮮度管理 | `effective_open()` のフォールバック呼び出し（`most_recent_trusted`）にのみ鮮度上限を適用 | `most_recent_trusted()`/`most_recent_trusted_after()` の関数本体を書き換え、drift correction（独自の `DRIFT_CORRECTION_OBS_MAX_AGE_MS` を持つ）や ADR-080 収束判定（`most_recent_trusted_after` の「観測が無い＝未収束」という意味論）を巻き込むこと |
| 判定の決定論性 | `issue_open_warrant(..., now: Instant)` に加え `ime_apply_should_defer` / `is_focus_transition_settling` も `Instant` 引数化 | これらの関数内部で `Instant::now()` を呼ぶこと（journal replay 不能化を防ぐ） |

### 2.3 原則

（P1〜P5 は ADR-084、P6〜P10 は ADR-086 が使用済みのため P11 から採番する。）

#### P11: belief と actuation warrant は同じ bool を共有しない

`effective_open()` は engine の内部挙動決定にのみ使う。OS への実際の書き込みを
伴う経路（`apply_force_on_for_imm_broken` / `consume_force_open_pending` /
`try_force_on_bootstrap` / drift correction の実書き込み分岐）は、すべて
`issue_open_warrant()` が返す `Option<OpenWarrant>` を経由し、`None` なら書き込まない。

#### P12: warrant の根拠は限定列挙とし、観測由来の弱いソースを含めない

```rust
// state/open_warrant.rs（新設）
/// 「実 IME を外部から書き換えてよい」という授権。
/// 構築できるのは issue_open_warrant() のみ（フィールド private）。
pub struct OpenWarrant {
    /// この warrant が正当化する open 値。basis が示す観測/意図の target と
    /// 一致することを issue_open_warrant() 内で検証する（consume 側は
    /// target を再検証しない — 「値」と「根拠」を別々に持ち出せないようにする）。
    target: bool,
    basis: WarrantBasis,
    issued_at: Instant,
    /// `ObservationStore.current_focus_epoch`。実際の force 経路は別カウンタ
    /// `Output::ime_mode_focus_gen`（`runtime/mod.rs:779,844`）でもフェンスして
    /// いるため、consume 側は **両方**を照合する（どちらか一方に統合するかは
    /// 実装時に決める。本 ADR では「二重フェンスのまま両方見る」を暫定解とする）。
    focus_epoch: FocusEpoch,
}

pub enum WarrantBasis {
    /// ユーザーが SyncKey/PhysicalImeKey/Command で明示的に操作した。
    /// target/鮮度/scope を持つ `RecordedIntent` を保持する（単なる
    /// `UserIntentSource` ではなく、意図がどの対象向けだったかを検証できる形）。
    ExplicitUserIntent(RecordedIntent),
    /// High confidence の直接 API 読み取り（ImmGetOpenStatus 等）。
    /// **observation-based correction 経路専用**。ADR-086 §2.1 の
    /// force-write（観測を判断材料にしない）には使わない（INV-25）。
    DirectRead(ObservationSource),
    /// 独立した2ソース以上が一致（TsfNative では構造的に発行不可）。
    /// DirectRead 同様 observation-based correction 専用。
    Corroborated { a: ObservationSource, b: ObservationSource },
    /// PanicReset 等の安全弁。**`reason.overrides_explicit_intent() == true`
    /// の reason に限る**（`BrokenAppBootstrap` 等の非 override 系ヒューリス
    /// ティック guard はここに含めない、§7 round3 M4）。
    SafetyValve(ForceOnReason),
    /// 観測が一切無い状況での、profile 依存の安全デフォルト推測。
    /// `HeuristicDefault` 観測（`reset_stale_ime_on_for_imm_broken()` が
    /// Imm32Unavailable 入場時に記録）が実在すればそれを basis にする。
    /// `BrokenAppBootstrap` 等の非 override 系ヒューリスティック guard も
    /// ここに合流する（§7 round3 M4）。
    HeuristicGuess(ObservationSource /* or ForceOnReason 相当 */),
    /// **awase 自身の意図（`desired_open`）を、`HeuristicGuess` すら成立しない
    /// 状況での最終的な根拠として使う**。`AppImePolicy.default_feedback ==
    /// FeedbackPolicy::Blind`（実 IME の open 状態を直接観測する手段が構造的に
    /// 無いプロファイル）で `HeuristicGuess` の元になる観測が無いとき、
    /// `desired_open` こそが「観測を判断材料にしない」という ADR-086 §2.1 の
    /// force-write の定義に忠実な根拠になる（ただし §7 round3 シナリオ5参照:
    /// `desired_open` はターゲットにスコープされないグローバル単一フラグの
    /// ため、これは既存挙動の追認であって改善ではない）。§7 round2 M1・
    /// round3 M1 を経て、**発行条件を `AppImeProfile` の raw な一致判定に
    /// してはならない**（下記 `issue_open_warrant` 参照）。
    OwnSsot,
}

/// 唯一の発行点。純粋関数（now を引数に取る、Instant::now() を内部で呼ばない）。
/// is_japanese_ime はスコープ判定であり観測ではないため引数に含めてよい
/// （INV-25 の対象外、§4 参照）。**`profile: AppImeProfile` は引数に含めない**
/// （§7 round3 M1: `CASCADIA_HOSTING_WINDOW_CLASS` は `IMM32_UNAVAILABLE_CLASSES`
/// と `is_tsf_native_window` の両方に該当し、`AppImeProfile::from_class_name`
/// の優先順位で `Imm32Unavailable` に分類される——これは
/// `class_names.rs:65-77` が「2026-07-05 実機バグ」として明文で禁止している
/// 判定方法。`policy.default_feedback`（`FeedbackPolicy::Blind`/`Read`）を
/// 経由することで、この罠を踏まずに同じ区別ができる）。
pub fn issue_open_warrant(
    intent: Option<&RecordedIntent>,
    obs: &ObservationStore,
    guards: &ForceGuardSet,
    policy: &AppImePolicy,
    is_japanese_ime: bool,
    now: Instant,
) -> Option<OpenWarrant>;
```

`ConvOpenInference`（`KatakanaShadowOff`/`NativeToggleShadowOff`）、および belief
の自己確認由来の `FocusProbe`（BUG-33 が指摘した書き戻し混入）は、どの
`WarrantBasis` variant にも該当し得ない設計とする。
**ただし P16 の通り、これは「弱い basis を削る」だけでなく「正当な basis
（明示意図・真の安全弁・profile 依存の既定推測・awase 自身の SSOT）を明示的に
足す」作業とセットでなければならない**（§5 Phase 2' 参照。単独の引き算は既存の
force-ON を機能不全にする、§7 レビュー記録 round1 M1/M2、round2 M1）。

**`SelfApplied`（自己確認の連鎖）は本 ADR のスコープから外す**（§7 round2 M6）。
`AppliedImeState` に「直前の apply がどの basis で撃たれたか」を保存する場所が
無く、既存の force-ON 経路が `on_ime_apply_complete` に `generation: None` を渡す
ため provenance 制約（旧 INV-26）を検証する材料が存在しない。`FocusChanged` が
`applied` を毎回 `Unknown` にリセットする（`ime_model.rs:382`）ため、フォーカスを
またいだ自己確認の連鎖は構造的に1回も成立せず、実利が無いまま複雑さだけが残る。
将来 `AppliedImeState` に basis 保存を追加する具体的な必要性が出た時点で、
別途 ADR 追補として再検討する。

#### P13: BUG-26 が依拠する `derive_open()` の挙動は弱めない

`derive_open()` の Medium 単独多数決（belief 側の解決）は本 ADR の対象外とし、
現状のロジックを維持する。P11/P12 は「actuation 側の入口を絞る」ものであり、
「belief 側の解決ロジックを絞る」ものではない——**ただし `effective_open()` の
`has_user_explicit_intent()` 分岐の選択自体（どの経路を通るか）は §5 Phase 1'
（意図の永続化）が意図的に変更する**。これは `derive_open()` を弱めることとは
別軸であり、矛盾しない（詳細は §4 INV-21・§7 レビュー記録 round1 M3 の訂正参照）。

**既知の限界（§7 round2 シナリオ1、素直に記録する）**: P13 が `derive_open()` を
守る帰結として、**明示意図が一度も無い状態で本バグの操作列（IME キーは押さず、
仮想デスクトップ切替だけでフォーカスが TsfNative アプリへ移り、conv=NATIVE が
stale に残っている）を再現すると、engine の belief（`effective_open()`）は
依然として `ConvOpenInference` 1件で ON に復帰する**。これは「conv ビットだけ
では BUG-26（実際に開いている）と本バグ（実際は閉じている）を区別できない」
という §1.3 の情報論的な制約の直接の帰結であり、本 ADR のどの Phase でも
解消できない。本 ADR が実際に達成するのは、**(2) 実 IME への望まぬ書き込みを
確実に止めること**である。**(1) engine のかな変換の混入**は、直前に明示意図が
存在した場合（Phase 1' が効く）に限って直る。明示意図が一度も無い場合、
症状は「実 IME は閉じたまま、engine だけが一時的にかな変換モードだと誤信し、
生成されたかなのローマ字列がリテラル着弾する」という**より軽い**（外部 IME
状態は破壊されない）形に変わる——これは BUG-16 の症状（`korede` 等の部分的な
リテラル化）と同種であり、新しい壊れ方ではない。

#### P14（撤回）: 観測の鮮度上限は belief 側に置かない

**旧 P14（`effective_open()` のフォールバックに鮮度上限を導入する）は撤回する**
（§7 round2 M3）。理由は2つ:

1. **BUG-26 を部分的に再発させる**: `derive_open()` の `FRESH = 3s`
   （`observation_store.rs:260`）を過ぎた後、鮮度上限のある `most_recent_trusted()`
   フォールバックが `None` を返すと `effective_open()` は `unwrap_or(desired_open)`
   に落ちる。BUG-26 のシナリオでは `desired_open` が stale な `false` の
   ことがあり、engine が**文の途中で** OFF に戻る。しかも
   `should_run_idle_conv_check` は `output_idle_ms > TYPING_IDLE_MS` を要求する
   ため（`key_pipeline.rs:370-376`）、連続タイピング中は新しい conv 観測が
   届かず、次のタイピング休止まで回復しない。
2. **導入の動機が Phase 2' で既に消えている**: 旧 P14/INV-22 の動機（§1.4
   item 2）は「1件の conv 推論が `effective_open()` を無期限に支配し、その
   `effective_open()` が actuation の根拠にもなる」ことだった。Phase 2' で
   actuation ゲートが `effective_open()` を直接読まなくなった時点で、
   「belief 側の鮮度」を制限する理由は無くなっている。

**一般則（§7 round2 総評より）**: このリポジトリの belief 層は「情報が無い」
状態を一貫して OFF 方向に解決する設計になっている（`effective_open()` の
`unwrap_or(desired_open)`、`derive_open()=None` のフォールバック chain 等）。
**したがって belief 側で観測の鮮度を切ると、必ず OFF 方向への倒れ込みを
意味する。** 鮮度・失効の概念を持ち込みたい場合は、belief 側ではなく
**actuation の根拠（P15/`WarrantBasis`）側にのみ**置くこと。

#### P15: actuation ゲートは足し算で設計し、優先順位を明示する

`is_eligible_for_ime_force_on()`（および `issue_open_warrant()`）の再設計は、
「弱いソースを取り除く」引き算だけで完結させてはならない。現状の force-ON は
複数の弱い/暗黙のソースによって現に成立しており（BUG-16 の修正がこれに依拠
している）、それぞれ**別の profile 向け**である:

- `HeuristicDefault` 経由の `most_recent_trusted()` フォールバック
  （`reset_stale_ime_on_for_imm_broken()` が Imm32Unavailable 入場時に記録）
- `effective_open()` 末尾の `unwrap_or(desired_open)`（`FeedbackPolicy::Blind`
  向け。§7 round2 M1 で発見。**旧ドラフトはこちらを見落としていた**）

したがって新しいゲートは、**OR ではなく優先順位付きの段階評価**として設計する
（§7 round2 M4/M7、round3 M1/M4 — 旧版は単純な OR で書かれており、明示 OFF
意図と既定推測の ON 例外が衝突したときに優先順位が定まらず、かつ Step 4 を
`AppImeProfile` で分岐していたため round2 で塞いだはずの BUG-16 が round3 で
別の扉から再発していた）:

```
issue_open_warrant(...) の評価順序:

  Step 1（最優先、成立すればそれ以降を評価せず確定）:
    has_user_explicit_intent() が true
      → basis = ExplicitUserIntent(recorded_intent)（intent.target の値で確定）
      → 他のどの枝も、これを上書きしない。

  Step 2（Step 1 が不成立のときのみ評価。**「真の安全弁」限定**）:
    force_guards の中に reason.overrides_explicit_intent() == true の
    ものが active（例: PanicReset）
      → basis = SafetyValve(reason)
      → **`BrokenAppBootstrap` のような override 権限を持たないヒューリス
        ティック guard はここに含めない**（§7 round3 M4: 優先順位付き
        評価では Step 2 に到達した時点で `has_explicit_intent==false` が
        既に確定しているため、旧版の「`ForceGuardSet::effective_open()` と
        同じ述語にする」という round2 の修正は実質 `requires_on()` に
        潰れて no-op だった。override 権限の有無で明示的に絞る）。

  Step 3（Step 1/2 とも不成立のときのみ評価）:
    authority(source) == Actuating な観測が、derive_open() 相当の判定
    （High 単独 or Medium+ 無競合多数決）を満たす
      → basis = DirectRead(source) または Corroborated{..}

  Step 4（Step 1〜3 とも不成立のときのみ評価。profile 依存の既定推測 +
          override 権限を持たないヒューリスティック guard の合流点）:
    a. `HeuristicDefault` 観測（`reset_stale_ime_on_for_imm_broken()` が
       Imm32Unavailable 入場時に記録）が実在すれば → basis = HeuristicGuess
    b. override 権限を持たない `ForceGuard`（`BrokenAppBootstrap` 等）が
       active なら → basis = HeuristicGuess
    c. a/b のいずれも無く、`policy.default_feedback == FeedbackPolicy::Blind`
       （実 IME の状態を直接観測できないプロファイル。TsfNative も
       Imm32Unavailable も該当しうる——**`AppImeProfile` の値そのものでは
       分岐しない**、§7 round3 M1）なら → basis = OwnSsot（desired_open）
    d. いずれも成立しなければ None。

    **Step 4 全体（a/b/c）は、この対象（INV-24(b)）で明示 OFF 意図が記録
    された履歴が残っている間は評価しない**（§7 round3 シナリオ9。OFF 方向の
    抑制窓は ON 方向の TTL より長く/非対称に取る、INV-24(a) 参照）。

  上記いずれも成立しなければ None。
```

**Step 1/2 が Step 3/4 より必ず先に評価される**ことで、明示 OFF 意図・真の
安全弁が既定推測に踏み潰されることを構造的に防ぐ。Step 4 の a/b/c は
**プロファイル名ではなく `policy.default_feedback`（`FeedbackPolicy::Blind`/
`Read`）と「observation/guard が実在するか」だけで分岐する**——このリポジトリで
アプリを識別する正しい鍵は `AppImeProfile` の生の一致判定でも PID でもなく、
目的ごとに用意された述語（`is_effectively_tsf_native()`、
`AppImePolicy.default_feedback` 等）であり、新しい判定を書く前にまず既存の
述語を探すこと（§7 round3 総評の一般則。round1 の「情報が無い状態は OFF に
倒れる」と対になる教訓）。

#### P16: force-write は ADR-086 §2.1 の定義どおり、観測を「書き込む値」の判断材料にしない

`consume_force_open_pending`（ADR-086 Phase 3）の eligibility 判定を
`issue_open_warrant()` 経由に差し替え、`WarrantBasis` が `ExplicitUserIntent` /
`SafetyValve` / `HeuristicGuess` / `OwnSsot` のいずれかであることを要求する
（`Corroborated`/`DirectRead`/`SingleIndirect` は observation-based correction 専用であり、
force-write の趣旨（観測を信じない）とは別文脈のため force-write 経路では
使わない）。**round2 版はここに `HeuristicGuess`（当時の `HeuristicDefault`）を
含め忘れていた**（§7 round3 Codex シナリオ3 の High 指摘。P15 Step 4 は
Imm32Unavailable でこの basis を許すのに、INV-25 側の許可リストから漏れていた）。
`HeuristicGuess`/`OwnSsot` は「観測を実際の判断材料にしない」という定義に
（前者は観測ゼロという事実だけに基づくポリシー既定値、後者は awase 自身の
意図のみを根拠にする点で）忠実な basis である。

**ただしこれは「書き込む値（open=true にするかどうか）を観測から導かない」という
意味であり、「スコープ判定（`can_use_imm32_cross_process()` によるプロファイル
判定、`is_japanese_ime()` によるレイアウト判定、`policy.default_feedback` に
よる observability 判定）を排除する」という意味ではない**（§7 レビュー記録
round1 S7）。スコープ判定は観測ではなく静的な設定・分類であり、
`issue_open_warrant()` の引数として残してよい（P12 のシグネチャ参照）。

---

## 3. 代替案の比較

`.claude/rules/experiment-logging.md` の教訓（「良いアイデアに見えるか」ではなく
「過去にどの条件で壊れたか」で評価する）に従う。

### 案A: confidence 調整による最小修正

案A は性質の異なる2つの調整を含んでいたため、レビュー（§7 must-fix 相当）を経て
分離する。

**案A1（confidence/corroboration の調整）**: `ConvOpenInference` を Low に
格下げする、または `derive_open()` の Medium 段に2ソース以上の corroboration を
要求する。**これは `effective_open()`（belief 側）そのものを弱める。**

**評価: 却下（再提案禁止）。** §1.3 の通り、いずれの調整も **BUG-26 の再発を
確実に招く**。conv ビットには BUG-26 と本バグを区別する情報が原理的に含まれて
いないため、confidence の重み付けをどう変えても分離できない。

**案A2（force-on 側への source-aware gate 追加）**: drift correction が既に持つ
`ConvOpenInference && explicit_intent.is_none() → return None` と同じ判断を、
`effective_open()` 自体は変更せず、**actuation の入口側にだけ**追加する。

**評価: 採用する（ただし単独では不十分）。** これは P15/P16（§2.3）が定める
「足し算のゲート」の一部として引き継ぐ。単独で「弱いソースを削る」だけを
やると、force-ON が現に依拠している正当な弱いソース（`HeuristicDefault` 経由の
Imm32Unavailable 入場時デフォルト）まで一緒に削ってしまい BUG-16 を再発させる
（§7 レビュー記録 M1/M2）。§5 Phase 2' で、削る作業と足す作業を同一コミットで
行う。

### 案B: `OpenWarrant` 型の新設（**本 ADR が採用**）

**内容**: §2.3 の P11〜P15。`effective_open()` は残すが、実 actuation 入口は
すべて `issue_open_warrant()` を経由させる。

**根本性**: BUG-26（belief 側は無変更）と本バグ（actuation 側を絞る）を同時に
満たす。「近道」が構文的に存在しなくなる — 新しい actuation 経路を書こうとした
瞬間に「warrant をどこから持ってくるか」を考えざるを得ない。これは
`.claude/rules/ime-belief-architecture.md` が既に採っている「コンパイラ（最強）」
層の考え方をそのまま適用したものである。

**実装コスト**: 中。実 actuation 入口は `tests/architecture_guard.rs` の
`apply_ime_open_with_belief_call_sites_are_accounted_for`（`EXPECTED_TOTAL` 固定）
で既に棚卸しされており、箇所数は限定的。`issue_open_warrant` は純粋関数のため
Linux で全数テスト可能。

**リスク**: TsfNative では `DirectRead`/`Corroborated` が構造的に発行できないため、
warrant の源泉が実質 `ExplicitUserIntent` と `SafetyValve` のみになる。ところが
`FocusChanged` が `last_intent` を消すため、**フォーカス直後は
`ExplicitUserIntent` basis も使えない**。単独では TsfNative で force-on が
恒久的に沈黙するリスクがあり、案D（意図の永続化）とセットでなければ機能しない。

### 案C: `ObservationSource` に authority 属性を持たせる（案Bの軽量前段）

**内容**（`ObservationSource` は `state/ime_event.rs:89-149` の11 variant を
**すべて**網羅する。§7 レビュー記録 M4 で `ConvBitsInference`/`GjiIoInference`
の欠落が指摘されたため修正済み）:

```rust
impl ObservationSource {
    pub const fn authority(self) -> ObservationAuthority {
        match self {
            Self::ImmGetOpenStatus | Self::ImmCrossProbe
            | Self::ObserverPoll | Self::Gji | Self::Tsf => ObservationAuthority::Actuating,
            Self::ConvOpenInference | Self::HeuristicDefault | Self::HwndCache
            | Self::FocusProbe /* BUG-33: belief 書き戻し混入 */
                => ObservationAuthority::BeliefOnly,
            // ConvBitsInference/GjiIoInference は input_mode 専用ソースで、
            // PerSourceObservations::get/set（observation_store.rs:79-113）が
            // 明示的に None/no-op を返すため open 観測には現れない
            // （open 軸の derive_open()/most_recent_trusted() からは常に不可視）。
            // authority() は ObservationSource 全体で定義するため、到達しない
            // 枝であっても網羅的に BeliefOnly を割り当てておく（安全側デフォルト）。
            Self::ConvBitsInference | Self::GjiIoInference => ObservationAuthority::BeliefOnly,
        }
    }
}
```

`derive_open()` を `derive_open_for_belief()` / `derive_open_for_actuation()` に
分割**しない**（§7 レビュー記録 M1 を踏まえた訂正: 単純な2分割では
`has_user_explicit_intent()`/`force_guards`/`HeuristicDefault` 例外を落として
しまう）。`authority()` は P15/P16 が定める「足し算のゲート」（§5 Phase 2'）の
**判定材料の1つ**として使う。

**根本性**: `ForceOnReason::overrides_explicit_intent()` という**既存の先例**
（「ソースごとにどの権限を持つかを属性として宣言する」パターン）の一般化であり、
`WarrantBasis` の判定ロジック部分を型を増やさずに先行実装できる。

**評価**: **§5 Phase 2' の判定材料として採用する（単独の Phase としては採用しない）。**
単独ではコンパイラ強制ではなく（新しい actuation 経路が `effective_open()` を
直接読む近道は依然として書ける）、`architecture_guard` の出現数固定で補う必要がある。

### 案D: 明示意図をフォーカスをまたいで永続化する

**内容**: `FocusChanged` が `last_intent = None` にすることで作る「真空」を、
対象（hwnd/profile）ごとの `IntentStore` への引き継ぎに置き換える。既にこの
必要性は部分的に認識されており、`persistent_explicit_off_ms`
（`platform_state.rs:409`）というアドホックなパッチが「複数の rapid focus 変化
（仮想デスクトップ切替等）では2回目以降の guard が機能しない」と明記している
——**今回のバグの再現操作そのもの**。`HwndCacheRestored` も同じ問題への別の
アドホック解だが `last_intent` を設定しないため保護にならない。両者を
`IntentStore` に統合する。

**根本性**: 真空そのものを無くすため、engine 側の症状（(1) かな変換の混入）を
**明示意図があった場合に限り**根治できる（§7 round2 シナリオ1 で判明した限定、
下記「評価」参照）。案B（actuation 側の授権を絞る）は (2) 実 IME への望まぬ
書き込みは止めるが、(1) は残る（IME は閉じたままなので、生成されたかなの
ローマ字列がリテラル着弾し「kusita」のような別の壊れ方に化けるだけ）。

**評価**: **§5 Phase 1'（本 ADR のレビューを経て最優先の Phase に繰り上げ）として、
TsfNative での案Bの前提を成立させるために必須**。単独の大規模刷新（ADR-078/081 の
スコープ）ではなく、「`has_user_explicit_intent()` がフォーカス直後も真を返せる
ようにする」という一点に絞って切り出す。**意図の永続化は単独リリースでも、明示
意図があった場面での本バグの症状 (1)（かな変換の混入）を直す価値があり、退行
方向（真空が埋まる方向）も BUG-26 と紛れにくい**（§7 round1 M6 の推奨順序）。
ただし無条件の永続化は別の固着バグ（BUG-19 の中核防御を破壊する固着、§7 round2
M2）を作りうるため、TTL・対象一致（3段判定）・**actuation 完了時刻基準**での
より新しい High confidence 観測による失効の3条件を必ず付ける（§4 INV-24）。

**round2 で判明した限界**: **明示意図が一度も無い場合（本バグの実際の再現操作列
がまさにこれ）、案D は症状 (1) を直せない。** conv ビットは BUG-26（実際に開いて
いる）と本バグ（実際は閉じている）を区別する情報を持たないため（§1.3）、意図と
いう外部情報が無ければ engine belief の誤りは原理的に検出不能。本 ADR が現実的に
約束できるのは「(2) 実 IME への書き込みは意図の有無によらず必ず止める」ことと、
「(1) は意図がある場合に治る」ことの2点であり、両方を無条件に治せるとは主張しない。

### 比較表

段階移行の順序は §7 の相互レビューを経て、当初案（B/C を Phase 1、D を Phase 2）
から**D 系（意図永続化）を先に、C 系（authority 属性）を後に**入れ替えた
（詳細は §5・§7 round1 M1/M2/M6）。round2 レビューを経て `OwnSsot` basis
（TsfNative の SSOT フォールバック）を追加したため、「TsfNative で機能」列を
更新した。

| 案 | BUG-26 非退行 | 今回のバグ (2) 実IME書込み | 今回のバグ (1) かな変換混入 | TsfNativeで機能 | 実装コスト | 採否 |
|---|---|---|---|---|---|---|
| A1 confidence/corroboration調整 | ✗（再発確実） | — | — | — | 小 | **却下（再提案禁止）** |
| A2 actuation側source-aware gate | ○ | ○（優先順位付き足し算とセットなら） | — | △ | 小 | **採用（Phase 2' の一部）** |
| D 意図永続化 | ○ | — | △（明示意図がある場合のみ） | ○ | 中 | **採用（Phase 1'、最優先）** |
| C authority属性 + 優先順位付きゲート（`OwnSsot`込み） | ○ | ○ | ✗（単独では） | ○（`OwnSsot` により回復） | 小〜中 | **採用（Phase 2'）** |
| B OpenWarrant 型化 | ○ | ○ | △（D 前提、D の限界を継承） | ○（D とセットで） | 中 | **採用（Phase 3）** |

---

## 4. 不変条件（invariant）

ADR-084 の INV-1〜11、ADR-086 の INV-12〜19 を継承し、INV-20 から採番する
（採番理由は ADR-086 §ステータスと同型: 同一の名前空間に属し、後日の grep で
一意に辿れることが規約の実効性そのものであるため）。

- **INV-20（根拠軸）**: 外部 IME open/close 状態への書き込みは `OpenWarrant` を
  要求する。`WarrantBasis` は P15（優先順位付きの Step 1〜4 ゲート）が列挙する
  variant に限り、`ConvOpenInference` 単独、belief 書き戻し由来の `FocusProbe`
  を basis に含めてはならない。`HeuristicGuess`/`OwnSsot`（Step 4）は明示的な
  例外として許容する。**`OpenWarrant.target`（書き込む値）は `basis` が示す
  観測/意図の target と一致することを `issue_open_warrant()` 内で検証しなければ
  ならない**（§7 レビュー記録 round1 Codex #1: target 検証が無いと
  `DirectRead(open=true)` が `open=false` の書き込み根拠にもなり得る）。
  **`WarrantBasis` の発行条件を `AppImeProfile` の raw な一致判定で分岐しては
  ならない**（§7 round3 M1: `class_names.rs:65-77` が「2026-07-05 実機バグ」
  として明文で禁止している判定方法そのものであり、Windows Terminal の外側
  ウィンドウ（`CASCADIA_HOSTING_WINDOW_CLASS`）で誤判定し BUG-16 を再発させる。
  `AppImePolicy.default_feedback` や `is_effectively_tsf_native()` 等、
  目的別に用意された述語を使うこと）。

- **INV-21（`derive_open()` の Medium 単独多数決は弱めない）**: 本 ADR のどの
  Phase も、`derive_open()` の「Medium+ ソースの無競合多数決（1ソースでも可）」
  という判定ロジック自体を弱めてはならない（BUG-26 が依拠する挙動）。**これは
  「`effective_open()` の分岐選択（`has_user_explicit_intent()` の真偽）を
  一切変えない」という意味ではない** — INV-24（意図の永続化）は
  `has_user_explicit_intent()` がフォーカス直後に真を返す範囲を意図的に広げる。
  この2つは独立した軸であり矛盾しない（§7 レビュー記録 round1 M3 の訂正）。
  **ただし §2.3 P13 が明記する既知の限界（明示意図が一度も無い場合、症状(1)
  は本 ADR のどの Phase でも解消できない）を隠さないこと。**

- **INV-22（撤回、belief 側に鮮度上限を置かない）**: 旧 INV-22
  （`most_recent_trusted()` フォールバックへの鮮度上限）は**撤回**する
  （§7 round2 M3。理由は §2.3 P14 参照）。**belief 側で観測の鮮度を切る変更は
  再提案禁止**——このリポジトリの belief 層は情報が無い状態を一貫して OFF
  方向に解決するため、鮮度による失効は必ず OFF 方向への意図しない倒れ込みを
  生む。鮮度・失効の概念が必要な場合は actuation の根拠側（`WarrantBasis`）
  にのみ導入すること。

- **INV-23（根拠判定の決定論性）**: `issue_open_warrant()` に加え、force-ON の
  可否を左右する `ime_apply_should_defer()`（`runtime/mod.rs:635`）・
  `is_focus_transition_settling()`（`key_pipeline.rs:125` の呼び出し）も
  `Instant` を引数に取る形へ拡張し、内部で `Instant::now()` を呼んではならない
  （ADR-082 journal replay での再現可能性を維持するため。§7 レビュー記録
  round1 S2）。

- **INV-24（明示意図の scoped 永続化、対象一致粒度とTTLを明示）**:
  `FocusChanged` は `UserIntentSource` の記憶を無条件に破棄してはならないが、
  無条件に維持してもならない。意図は対象ごとの `IntentStore` に保持し、
  次の条件で失効させる。
  - **(a) TTL は ON/OFF で非対称にする**: Step 4（`HeuristicGuess`/`OwnSsot`）
    の既定推測は「観測ゼロなら ON 寄りに倒れる」バイアスを持つため（一部は
    `desired_open` 中立だが `HeuristicGuess` は ON バイアス）、失効コストが
    方向によって異なる（ON 意図の失効は Step 4 と同じ結論になり実害が薄いが、
    OFF 意図の失効は Step 4 が正反対の結論を出す）。**OFF 方向の意図の保持
    窓は ON 方向より長く取る**、あるいは対象不一致以外では失効させない
    （§7 round3 M/シナリオ9）。既存の `EXPLICIT_OFF_CACHE_SUPPRESS_MS`
    （`focus_tracking.rs:15`、現行10秒）は元々 OFF 専用の抑制窓として設計
    されている——この非対称性をそのまま踏襲し、ON 意図にまで同じ 10 秒を
    機械的に流用しない。具体的な値は実測してから確定する
    （`.claude/rules/tuning-constants.md`）。
  - **(b) 対象一致（2段判定。3段目は round3 で削除）**: 「① 同一 hwnd なら
    無条件一致。② それ以外は不一致」の**2段**とする。round2 で提案した
    「③ 同一プロセス + 同一 `is_effectively_tsf_native()`」という3段目
    （tier②）は round3 で**削除**する: (i) 前提が誤りだった——`FocusChanged`
    は `focus_tracking.rs:232`（`on_focus_process_changed`）の1箇所からのみ、
    PID 変化時にしか発火せず、Windows Terminal の
    `CASCADIA_HOSTING_WINDOW_CLASS`→`Windows.UI.Input.InputSite.WindowClass`
    は**同一プロセスのため FocusChanged 自体が発火しない**（＝意図はそもそも
    失われず、この tier を必要としない）。UWP の `ApplicationFrameWindow`→
    `CoreWindow` は**別プロセスかつ `is_effectively_tsf_native()` の値も
    異なる**ため、この tier では救えない。(ii) 実害がある——UWP アプリの
    外枠 `ApplicationFrameWindow` は複数の無関係なアプリで同一プロセス
    （`ApplicationFrameHost.exe`）を共有するため、tier②は**無関係な別アプリ
    の意図を「同一対象」と誤判定**する（§7 round3 M2/M3）。
  - **(c) より新しい観測による失効（`FeedbackPolicy::Read` のみに限定）**:
    失効の基準時刻は「意図が**記録された**時刻」ではなく「意図に対応する
    actuation が**完了した**時刻」（`AppliedImeState::Confirmed{at}` 相当）に
    する。ADR-080/BUG-43 が同じ問題に対して既に採用している
    `most_recent_trusted_after(now, act_sent_at)`（`ime_refresh.rs:666,688`）
    と同じ意味論に揃える（§7 round2 M2、既存の pinned テスト
    `ime_model.rs:1151`, `platform_state.rs:1371` が固定している挙動を守る）。
    **この失効条件は `policy.default_feedback == FeedbackPolicy::Read`
    のプロファイルにのみ適用する。** `FeedbackPolicy::Blind`（TsfNative /
    Imm32Unavailable）では apply の成否を観測で確認する手段が構造的に無く
    （`AppliedImeState` が `Confirmed` に遷移する契機が無い上、`FocusChanged`
    が `applied` を `Unknown` にリセットする）、(c) は一度も発火しない
    ——それを「常に失効しない」と安全側に読むか「基準時刻が無いので常に
    失効する」と誤読するかは実装の書き方次第で分かれるため、`Blind` では
    (c) を評価しないと明示する（§7 round3 M/シナリオ8。曖昧なまま実装すると
    BUG-19 が再発しうる）。

- **INV-25（force-write は ADR-086 §2.1 の定義に従う）**: `conv_mode_policy = force`
  時の open/close 軸 force-write（`consume_force_open_pending`）が要求する
  `WarrantBasis` は `ExplicitUserIntent` / `SafetyValve` / `HeuristicGuess` /
  `OwnSsot` に限る（`DirectRead`/`SingleIndirect`/`Corroborated` は
  observation-based correction 専用）。**round2 版は `HeuristicGuess`
  （当時の `HeuristicDefault`）をこの
  リストから漏らしており、P15 Step 4 との間に矛盾があった**（§7 round3 Codex
  シナリオ3、High）。**この制約は「書き込む値を観測から導いてはならない」と
  いう意味に限定され、スコープ判定（プロファイル capability・レイアウト・
  `default_feedback` 等の静的分類）を `issue_open_warrant()` の引数から
  排除する意味ではない**（§7 レビュー記録 round1 S7）。

- **INV-26（撤回、`SelfApplied` は本 ADR のスコープ外）**: 旧 INV-26
  （`SelfApplied` の provenance 制約）は撤回する。`WarrantBasis::SelfApplied`
  自体を §2.3 P12 の通りスコープから外したため対象が無い（§7 round2 M6）。
  将来再導入する場合は、`AppliedImeState` に basis 保存を追加する設計を
  先に固めてから、別途 ADR 追補として起票する。

- **INV-27（force_open_pending の policy 世代管理）**: `conv_mode_policy` の
  force⇔observe 切替（`reload_config`）は `force_open_pending`（および将来の
  `OpenWarrant` 発行済みキュー）を無効化しなければならない。放置すると、
  force→observe 切替直後に武装済み pending が残って observe 経路と二重に
  force-ON が発火する窓ができ、observe→force 切替直後は次の `FocusChange`
  まで force-ON が完全に無効になる（§7 round2 M8。本 ADR 以前からの既存の
  穴だが、Phase 3 で `OpenWarrant` 層を導入する際にこの曖昧さを持ち越さない
  こと）。

- **INV-28（warrant はゲートであって送信保証ではない。strategy 側の no-op
  条件を bypass すること）**: `issue_open_warrant()` が `Some` を返すことは
  「actuation を試みてよい」という**許可**でしかなく、実際に OS へ VK が
  送信される**保証**ではない。GJI 経路（`GjiDirectStrategy::apply`）は
  `view.control.shadow_on == true` のとき `VK_IME_ON` を送らず
  `AlreadyMatched`（no-op）を返す（`ime_controller.rs:110`）。warrant が
  `OwnSsot`/`HeuristicGuess` 由来で「実 IME は閉じているはず」と判断して
  いても、`shadow_on`（awase 内部の shadow 状態）が古い ON のままだと
  この no-op に阻まれ、BUG-16 が実装レベルで再発する（§7 round3 Codex
  シナリオ6、Critical）。force-ON の送信経路（`force_on_and_correct_romaji`）
  は、warrant による許可が下りた場合に限り、この `shadow_on` no-op を
  明示的に bypass しなければならない（`applied` を `None` にする既存の
  コメント（`runtime/mod.rs:722`）は実装時に `shadow_on` を見落としており、
  実際に効いていない——Phase 3 実装時にこの食い違いを解消すること）。

**明示的に却下する方向（再提案禁止）**:
- `ConvOpenInference` の confidence を Low へ格下げすること、または
  `derive_open()` の Medium 段に2ソース以上の corroboration を必須化すること
  （§1.3・§3 案A1。いずれも BUG-26 の再発を招く）
- `is_eligible_for_ime_force_on()` 等の actuation ゲートを、既存の正当な弱い
  ソース（`HeuristicGuess`/`OwnSsot` 等）を代替せずに単純に「引き算」で
  絞ること（P15。BUG-16 の再発を招く、§7 round1 M1/M2、round2 M1、round3 M1）
- `WarrantBasis`/Step 4 の発行条件を `AppImeProfile` の raw な値で分岐すること
  （INV-20。`class_names.rs:65-77` が禁止する判定方法そのもので、Windows
  Terminal で BUG-16 を再発させる、§7 round3 M1）
- INV-24(b) に「同一プロセス」を対象一致の条件として含めること（tier②の
  復活。共有ホストプロセスで無関係な別アプリの意図が混同される、
  §7 round3 M2/M3）
- belief 側（`effective_open()`/`most_recent_trusted()`）に観測の鮮度上限を
  導入すること（INV-22。BUG-26 の部分的再発を招く、§7 round2 M3）
- `has_user_explicit_intent()` の失効基準を「意図の記録時刻」基準にすること
  （INV-24(c)。BUG-19 の中核防御を破壊する、§7 round2 M2）
- `consume_force_open_pending` に settle ガードを戻すこと（ADR-086 §5 Phase 3 H1
  が明示的に否定済み — 1打鍵目の取りこぼしを招く）
- 本 ADR の目的のために新規 dylint crate を追加すること
  （`.claude/rules/ime-belief-architecture.md` の判断基準に照らして過剰投資。
  本 ADR の穴は「意味論的偽装」ではなく「型の不在」であり、private 化 +
  `architecture_guard` の出現数固定で足りる）

---

## 5. 移行計画

**§7 の相互レビュー（Opus/Codex 双方の指摘、特に M1/M2/M3/M6）を経て、
当初ドラフトの Phase 順序（C/B を先、D を後）を Phase 1' と Phase 2' の間で
入れ替えた。** 実際の依存関係は「Phase 0（診断）→ Phase 1'（意図永続化）→
Phase 2'（actuation ゲート、足し算で再設計）→ Phase 3（型化）」である。
各 Phase は独立してリリース可能で、後の Phase が実機で否定されても前の Phase は
残る、という主張はこの順序でのみ成立する（旧 Phase 1 は単独では BUG-16 を
再発させるため独立リリース不可だった、§7 M1）。

### Phase 0（記録と観測性の確保）

**0a（Linux で完結、実機不要）**:

1. 本 ADR を登録する（`docs/adr/index.md`、ADR-078/086 に相互参照の1行を追記）。
2. `effective_open()` を `effective_open_at(now: Instant)` に引数化し、内部の
   `Instant::now()` 呼び出しを排除する（INV-23 の前提）。あわせて
   `ime_apply_should_defer()` / `is_focus_transition_settling()` も同様に
   `Instant` を引数化する（§7 S2）。
3. 解決の内訳を返す診断 API を追加する: `resolve_open_at(now) -> OpenResolution
   { value, decided_by: DecidedBy }`。`DecidedBy` は
   `{ base: BaseDecision, guard_override: Option<ForceGuard> }` のように、
   `force_guards.effective_open()` が最後に被せる override を base と別枠で
   持つ（`ime_model.rs:233` の実際の構造に合わせる、§7 N3）。
   `BaseDecision::{DeriveHigh(src), DeriveMedium(src), MostRecentTrusted(src),
   DesiredFallback}`。
4. これを journal（ADR-082）に記録し、本バグを `tests/journals/` のフィクスチャ
   として固定する（`.claude/rules/fix-requires-evidence.md` の (a) を満たす）。
   **§1.4 item 7 の通り、この連鎖は単一 tick では完結しない**（`kp_stage_idle_conv_check`
   の非同期 offload を経て後続 tick で `report_conv_open_inference` が発火する）。
   フィクスチャは tick をまたぐ順序付きイベント列として設計する（§7 S1）。
5. `docs/known-bugs.md` に本バグを起票し、BUG-16/19/26/33 との family 関係を
   明記する（`.claude/rules/fix-requires-evidence.md` の (b)）。

**0b（実機ログが必要）**:

6. 実機ログを取得し、`apply_force_on_for_imm_broken`（observe policy）と
   `consume_force_open_pending`（force policy）のどちらが実際に発火したかを
   確定する。**設定値だけである程度絞り込める**: `arm_force_open_pending`
   （`runtime/mod.rs:776`）は `is_force_policy() && !can_use_imm32_cross_process()`
   で武装し、`can_use_imm32_cross_process()` は `Standard` プロファイルのみ真
   （`focus/class_names.rs:157`）。Windows Terminal の
   `Windows.UI.Input.InputSite.WindowClass` は TsfNative のため、force policy
   運用中なら `consume_force_open_pending` 側、observe policy（既定）なら
   `apply_force_on_for_imm_broken` 側と、ほぼ機械的に決まる（両者は
   `is_force_policy()` で相互排他、§7 N4）。ログはこの推定の確認として取る。

### Phase 1'（案D: 明示意図の scoped 永続化。旧 Phase 2 から繰り上げ）

**本節は round1/2 時点の計画であり、round3/4 で実装内容が変わった箇所が
ある（§8.7 M-D 参照）。実際に実装されたものは §8.2/§8.6 の表と
`state/intent_store.rs` を正とする。**主な相違点:

- 「`persistent_explicit_off_ms`/`HwndCacheRestored`/`last_intent` を統合
  した `IntentStore`」という記述は round4 で撤回した。`IntentStore` は
  これら3つとは**統合せず**、`HwndId` キーの独立した新設データ構造として
  確定させた（§8.7: `HwndImeCache` は「アプリ単位の記憶」、`IntentStore` は
  「actuation の対象同一性」で役割が異なるため）。
- 対象一致は「3段判定」ではなく round3 で削除された**2段判定**（同一
  `HwndId` のみ一致、それ以外は不一致、INV-24(b)）を実装した。
- TTL は「`EXPLICIT_OFF_CACHE_SUPPRESS_MS` と統合」ではなく、round4 で
  `tuning::EXPLICIT_ON_INTENT_TTL_MS`/`EXPLICIT_OFF_INTENT_TTL_MS`
  （ON/OFF 非対称、OFF は `HWND_CACHE_MAX_AGE_MS` と同値）として独立に
  実装した（§8.6 M-A）。「actuation 完了時刻以降の観測失効」（INV-24(c)）は
  未実装のまま（`FeedbackPolicy::Read` プロファイル限定という設計のみ
  記録、§4 INV-24(c) 参照）。

以下は元の計画テキスト（履歴として残す）:

7. ~~`FocusChanged` で `last_intent = None` にする代わりに、
   `persistent_explicit_off_ms` / `HwndCacheRestored` / `last_intent` を
   統合した `IntentStore` を導入する。対象一致は INV-24(b) の3段判定
   （同一 hwnd／同一プロセス＋同一 `is_effectively_tsf_native()`／
   それ以外）を実装する。`RecordedIntent` は target・記録時刻・
   `focus_epoch` を持ち、INV-24 の3条件（TTL＝`EXPLICIT_OFF_CACHE_SUPPRESS_MS`
   と統合／対象不一致／**意図に対応する actuation 完了時刻以降**の
   より新しい High confidence 観測の到着）のいずれかで失効する。~~
   → 実際は上記の相違点のとおり実装。
8. `has_user_explicit_intent()` がフォーカス直後も、対象が一致する範囲で
   真を返せるようにする。（Phase 3 スコープ、`IntentStore` 自体は実装済み
   だが `ImeModel.last_intent` との統合配線は未着手）

**効果の範囲を正確に述べる**（§7 round2 M9/シナリオ1）: これで本バグの
(1)「かな変換の混入」が直るのは**直前に明示意図があった場合に限る**。
ユーザーが直前に IME OFF を明示していれば、フォーカス変更後もその意図が
保持され、conv 推論が `effective_open()` を上書きできない。**本バグの
実際の再現操作列（仮想デスクトップ切替のみ、IME キーは一度も押していない）
のように明示意図が一度も無い場合、症状(1) は Phase 1' だけでは直らない**
（P13 の既知の限界を参照。本 ADR のどの Phase でも情報論的に解消できない）。
それでも**単独でリリースする価値はある**（§7 round1 M6: 意図がある場面での
退行方向が「真空が埋まる」方向で BUG-26 と紛れにくく、Phase 2'/3 を待たずに
価値がある）。

### Phase 2'（旧 Phase 1 を「優先順位付きの足し算」に書き直したもの）

9. `ObservationSource::authority()` を導入する（§3 案C、11 variant 全網羅）。
10. `is_eligible_for_ime_force_on()` を、P15 の **Step 1〜4 の優先順位付き
    評価**に書き直す（Step 2 は override 権限を持つ真の安全弁のみ、Step 4 の
    a/b/c は `policy.default_feedback`/observation の実在で分岐し
    `AppImeProfile` の raw な値では分岐しない——詳細は P15 参照）。
    `derive_open_for_belief()`/`derive_open_for_actuation()` という単純な
    2分割はしない（round1 M1 の教訓）。
    **`reset_stale_ime_on_for_imm_broken()` は「置き換え」ではなく「観測の
    記録はそのまま維持し、actuation 側での消費だけを Step 4 に移す」**
    （§7 round3 M/シナリオ10）。この関数が記録する `HeuristicDefault` 観測は
    actuation の根拠だけでなく、`focus_tracking.rs:343-352` の
    「Imm32Unavailable hard pre-sync」（`effective_open()` 経由で `applied` を
    先同期し、初回キーでの spurious `VK_KANJI` を防ぐ）にも使われている
    ——観測の記録自体を撤去すると P13（belief 側は触らない）と矛盾し、
    Chrome の初回キーで意図しないトグルが起きる。
11. **旧 item 11（`most_recent_trusted()` への鮮度上限導入）は削除する**
    （INV-22 撤回、§7 round2 M3）。
12. `tests/architecture_guard.rs` の `conv_open_inference_source_is_limited_to_report_and_gate`
    を更新し、observation_store.rs の pinned テストに「belief 用であって
    actuation 用ではない」意図を明記する。
13. `reload_config` で `force_open_pending`（および将来の `OpenWarrant`
    発行済みキュー）を無効化する（INV-27、§7 round2 M8）。

これで本バグの (2)「実 IME への望まぬ書き込み」が、既存の force-ON 回復力
（BUG-16 の非退行。Imm32Unavailable 側の `HeuristicGuess` と `FeedbackPolicy::Blind`
側の `OwnSsot` の**両方**を維持することで実現する）を保ったまま止まる。Phase 1' が
先に入っていることが前提（意図永続化により、そもそも conv 推論や `OwnSsot` が
真空を埋める機会自体が減っている）。

**TsfNative での非退行はコードレビューだけで論証できる（§7 round3 シナリオ6）**:
明示意図・force guard・観測いずれも無い状態では、現行の
`is_eligible_for_ime_force_on()` と Phase 2' の Step 4 (c) はどちらも
`desired_open` を採用する点で**値・条件とも bit-identical**である。Phase 2'
が TsfNative に対して実際に変える点は「`ConvOpenInference` 観測を actuation
の根拠から外す」ことただ1点であり、BUG-16 が依拠する `desired_open`
フォールバック経路そのものには一切手を触れない。ただし §2.3 P12 の
`OwnSsot` 発行条件を `AppImeProfile` の raw な値で分岐すると、この
bit-identical 性は成立しない（round3 M1 参照）——実装レビューでは
「`AppImeProfile` を条件式に使っていないか」を機械的にチェックすること。

### Phase 3（案B: `OpenWarrant` 型による恒久化）

14. `OpenWarrant` / `WarrantBasis` を新設する（`SelfApplied` はスコープ外、
    P12 参照）。実 actuation 入口の棚卸しをまず正確にやり直す（§7 round1 M5:
    既存の `apply_ime_open_with_belief_call_sites_are_accounted_for` は
    `apply_ime_open_with_belief(` の呼び出しだけを数えており、
    `apply_ime_open_with_view`（`force_on_and_correct_romaji` 経由、
    `runtime/mod.rs:733`、`runtime/executor.rs:887`）、
    `apply_ime_open_with_applied`（`runtime/ime_refresh.rs:499`）、
    `set_ime_open`（`platform.rs:729`）、`ir_post_focus_change_snapshot:499`
    の直接 `apply_ime_open_with_applied(true, None)`（§7 round2 シナリオ8）は
    対象外。ガードの doc コメント自体も stale。§1.4 item 3 が挙げた drift
    correction・`EngineSync::DirectInput` も含め、各経路を force-write /
    observation-based correction のどちらに分類してから warrant 必須化の
    対象を決める）。
15. `consume_force_open_pending` の eligibility 判定を `issue_open_warrant()`
    経由に差し替える（INV-25、P16）。
16. **warrant による許可と実送信の間にある no-op suppression を bypass する**
    （INV-28、§7 round3 Codex シナリオ6、Critical）。`GjiDirectStrategy::apply`
    は `view.control.shadow_on == true` のとき `VK_IME_ON` を送らず
    `AlreadyMatched` を返す（`ime_controller.rs:110`）。force-ON 経路
    （`force_on_and_correct_romaji`）は、warrant が下りた場合にこの no-op を
    確実に bypass するよう修正する（現状のコメント「`applied` を `None` に
    して bypass する」（`runtime/mod.rs:722`）は実際には `shadow_on` を
    見ておらず効いていない）。
17. `effective_open()` に「これは engine の内部挙動決定用であって外部書き込みの
    根拠ではない」という doc コメントを追加する（名称変更は既存呼び出し
    （src 全体で 60 件超、テスト・定義を含む）への影響が大きいため、本 Phase
    では見送り、必要なら別途検討する、§7 round1 N2）。

Phase 1'/2' で確立した判定ロジックを型で固めるだけであり、動作変更を伴わない
リファクタとして提出できる（ADR-081 Phase 1d が躓いた「実機なしで本番経路を
書き足す」問題を回避する）。ただし item 16（GJI `shadow_on` bypass）は
挙動変更を伴う実装であり、この点だけは Phase 3 の中でも別途動作確認が必要。

---

## 6. 次にやるべきこと

- Phase 0a（Linux で完結する部分）から着手する。
- Phase 0b の実機ログ取得は、Phase 0a の診断 API・journal フィクスチャが揃った
  後に行う。`.claude/rules/fix-requires-evidence.md` の要求に従う。
- `docs/known-bugs.md` への本バグの起票（Phase 0a item 5）。
- 本 ADR は §7 の相互レビューを3巡した状態。さらに深掘りが必要な残課題は
  §7.8「未反映・要検討（round3 時点）」を参照。
- 実装に着手する場合は Phase 2' の Step 4（§2.3 P15、`policy.default_feedback`
  ベースの判定）と Phase 3 の INV-28（GJI `shadow_on` bypass）を最優先で
  正しく実装すること — round2 M1・round3 M1 の通り、ここを誤ると本 ADR が
  修正対象とする BUG-16 を**別の理由で2度**退行させた実例がある。実装
  レビュー時は「`AppImeProfile` の値そのものを条件式に使っていないか」を
  機械的にチェックすること。

---

## 7. 相互レビュー記録

本 ADR のドラフトは、Opus（`Plan` agent, model: opus）と Codex CLI（`codex exec
-s read-only`）に独立にレビューさせ、両者の指摘を統合して改訂した。

### 7.1 経緯

1. 起点のバグ調査自体も Opus と Codex CLI に独立相談し、両者とも「belief と
   actuation の根拠を型で分離すべき」という結論で収束した（Codex 案B
   `ActuationReadiness` ≒ Opus `OpenWarrant`）。Opus は追加で、BUG-26 との
   対称性（§1.3）という、対症療法を原理的に排除する反例を発見した。
2. この統合結果を初版ドラフトとして本ファイルに起票した。
3. 初版ドラフトを Opus・Codex CLI 双方に再度レビューさせた（実コードの再確認を
   含む）。両者からの指摘を本節以下に記録し、ドラフト本文（§1〜5）に反映した。

### 7.2 Codex CLI の指摘（要約、4 must-fix / 5 should-fix / 3 nice-to-have）

- **must-fix**: `issue_open_warrant()` が target/`OpenApplyReason` 等の呼び出し
  文脈を引数に持たず、target 検証ができない → **P12 の `OpenWarrant.target`
  検証要件、INV-20 に反映**。
- **must-fix**: Phase 3 の actuation 入口棚卸しが `apply_ime_open_with_belief`
  以外（`with_applied`・`force_on_and_correct_romaji`・`try_force_on_bootstrap`・
  `EngineSync::DirectInput`・drift correction・`ir_post_focus_change_snapshot`）
  を欠く → **Phase 3 item 13 に反映**（Opus M5 の具体的な行番号でさらに補強）。
- **must-fix**: §3 案A が「confidence 調整」と「actuation gate 追加」を混同 →
  **案A1/A2 に分割**。
- **must-fix**: Phase 1（旧）の `derive_open_for_actuation()` が ADR-086 §2.1
  force-write 定義と衝突しうる（`DirectRead`/`Corroborated` は observation-based
  correction 専用） → **P12 の `WarrantBasis` variant ごとの用途限定、INV-25 に
  反映**。
- **should-fix**: Phase 0「実機不要」と item 6「実機ログ取得」の矛盾 →
  **Phase 0a/0b に分割**。
- **should-fix**: 旧 Phase 1 の「独立リリース可能」が TsfNative の自動回復力を
  弱めるリスクを過小記述 → Opus M1 でこの懸念が「リスク」ではなく「確実な
  退行」であることが確定し、**Phase 順序の入れ替えで対応**。
- **should-fix**: `WarrantBasis::ExplicitUserIntent` に target/鮮度が無い →
  **`RecordedIntent` を basis に持たせる形に変更**。
- **should-fix**: INV-24（旧）が「無条件維持」で強すぎる → **INV-24 を TTL/対象
  一致/新観測失効の3条件付きに変更**。
- **should-fix**: 「2つの actuation 入口」が過少 → **§1.4 item 3 を6経路以上に
  修正**。

### 7.3 Opus の指摘（要約、6 must-fix / 7 should-fix / 5 nice-to-have）

Codex の指摘と大きく重なるが、実コードの再確認によりさらに具体的・決定的な
欠陥を発見した。

- **M1（most critical）**: Phase 1（旧）の `derive_open_for_actuation()` は、
  `reset_stale_ime_on_for_imm_broken()`（`platform_state.rs:779`）が
  Imm32Unavailable 入場時に記録する `HeuristicDefault`（Low confidence）を
  actuation から排除する。これは**現在 force-ON を成立させている唯一の経路**
  であり、除去すると BUG-16（「これで」→「korede」）が確実に再発する。
  → **P15（足し算のゲート）を新設し、Phase 2' に明示的な例外として残す形に
  再設計**。
- **M2**: `derive_open_for_actuation()` は `has_user_explicit_intent()` や
  `force_guards`（`PanicReset` 等）を経由しないため、最も正当な warrant
  （ユーザーの明示操作、安全弁）が Phase 1（旧）で実装されないまま弱い basis
  だけが消える。→ **P15 の (a)(b) 項として明示的に組み込み**。
- **M3**: INV-21（旧、「`effective_open()` の解決ロジックを変更しない」）と
  Phase 2（旧、意図永続化）が直接矛盾する。意図永続化は BUG-26 と同型の
  「stale な明示意図が観測を永久に上書きする」リスクも作る。→ **INV-21 を
  `derive_open()` の Medium 単独多数決に限定し、INV-24 に失効条件（TTL/対象
  一致/新観測）を追加**。
- **M4**: `authority()` の match が `ObservationSource` の11 variant 中
  `ConvBitsInference`/`GjiIoInference` を欠き、コンパイルが通らない。→
  **§3 案C のコード例に全 variant を追加**。
- **M5**: 「actuation 入口は architecture_guard で棚卸し済み」が事実誤認。
  `apply_ime_open_with_belief_call_sites_are_accounted_for` は
  `apply_ime_open_with_belief(` の呼び出しのみを数え、`apply_ime_open_with_view`
  （`runtime/mod.rs:733` 経由・`runtime/executor.rs:887`）、
  `apply_ime_open_with_applied`（`runtime/ime_refresh.rs:499`）、
  `set_ime_open`（`platform.rs:729`）は対象外。ガード自身の doc コメントも
  stale。→ **Phase 3 item 13 に実際の call site を列挙**。
- **M6**: Phase の独立性の主張が成立していない。実際の依存順序は
  「Phase 0 → Phase 2（意図永続化）→ Phase 1（ゲート）→ Phase 3（型化）」。
  → **Phase 順序を入れ替え（Phase 1' = 意図永続化、Phase 2' = ゲート）**。
- **S1**: 因果連鎖は同一 tick で完結せず、`kp_stage_idle_conv_check` の
  非同期 offload を経て後続 tick で発火する。→ **§1.4 item 7 として追加、
  Phase 0 の journal フィクスチャ設計に反映**。
- **S2**: `Instant::now()` は `effective_open()` 以外に `ime_apply_should_defer()`・
  `is_focus_transition_settling()` にもある。→ **INV-23、Phase 0a item 2 に
  反映**。
- **S3**: 鮮度上限を `most_recent_trusted()` 本体に入れると drift correction・
  ADR-080 収束判定に副作用が出る。→ **INV-22 をスコープ限定に修正**。
- **S4**: `issue_open_warrant()` のシグネチャに `is_japanese_ime`/プロファイル
  capability が不足。→ **P12 のシグネチャに `is_japanese_ime: bool` を追加**。
- **S5**: `OpenWarrant.focus_epoch`（`FocusEpoch`）と実際の force 経路が使う
  `ime_mode_focus_gen` が別カウンタで二重化する。→ **P12 に「暫定的に両方
  照合する」旨を明記（恒久的な統合は実装時に判断）**。
- **S6**: `WarrantBasis::SelfApplied` が provenance 制約を持たないと自己増幅
  ループを再生産しうる。→ **INV-26 として新設**。
- **S7**: INV-25（旧）が「書き込む値の根拠」と「スコープ判定」を混同しやすい。
  → **INV-25 に「スコープ判定は対象外」を明記、P16 に追記**。
- N1〜N5（行番号確認・語句の精度・`DecidedBy` の構造・実機ログ不要な判定の
  提示）→ 該当箇所に反映済み（§1.5.1 の config.rs パス、§5 Phase 0b item 6 の
  N4 準拠、Phase 0a item 3 の `DecidedBy` 構造を N3 準拠に修正、Phase 3 item 15
  の N2 準拠）。

### 7.4 未反映・要検討（round1 時点、round2 で一部解消）

- Opus S5（`focus_epoch` の二重化）は「暫定的に両方照合する」という妥協で
  ドラフトに反映したが、恒久的にどちらか一方へ統合するかは Phase 3 実装時に
  改めて判断する（round2 でも未解決のまま）。
- `architecture_guard.rs` 自体の stale なコメント修正（Opus round1 N5）は本
  ADR の記述には反映したが、実際のコード修正はまだ行っていない（Phase 3
  item 14 で行う）。

### 7.5 round2: シナリオシミュレーションレビュー（Codex・Opus、独立実施）

round1（記述の静的整合性）を踏まえた改訂版に対し、round2 では「IME・対象アプリ・
awase 内部の動作を具体的に想像し、実際のキー入力/フォーカス遷移シーケンスに
沿って設計を1ステップずつ手でトレースし、(1) 正しく動作するか (2) 改訂が新たな
不具合を生まないか」を検証するよう依頼した。**round1 で見えなかった、設計の
「原理的な帰結」に踏み込んだ欠陥が複数見つかった。**

#### Codex CLI（10シナリオ、3 must-fix / 4 should-fix / 1 nice-to-have）

- **must-fix**: 明示意図が無い本バグの再現シナリオでは、actuation（(2)）は
  止まるが engine belief（(1)）は `derive_open()` により ON に復帰してしまう
  → **§3 案D・§5 Phase 1' に効果範囲の限定を明記**（round2 総括の最重要点、
  Opus シナリオ1・3 とも一致）。
- **must-fix**: `IntentStore` の対象一致粒度が未定義で、UWP 親→`InputSite` 子の
  2段フォーカスで意図が失効/誤爆する → **INV-24(b) に3段判定を追加**
  （Opus シナリオ4・M5 と完全に一致、独立検証で確度が高い）。
- **must-fix**: `conv_mode_policy` の force⇔observe 切替時に `force_open_pending`
  が同期されない → **INV-27 を新設**（Opus シナリオ9・M8 と一致）。
- **should-fix**: TTL 切れ時の BUG-16 先頭リテラル化リスク、`SelfApplied` の
  継承形式が未指定、`HeuristicDefault` 例外が明示 OFF 運用を TTL 後に踏み潰す、
  GJI warmup との二重 VK 抑止が未定義 → いずれも Opus のより詳細な指摘
  （M2/M6/シナリオ7/シナリオ8）に統合。

#### Opus（10シナリオ、5 must-fix / 4 should-fix / 1 nice-to-have）

Codex の指摘と大きく重なるが、実コードを再度精読し、より根本的な欠陥を特定した。

- **M1（今回のラウンドで最も重要）**: 改訂版 P15 の足し算ゲート（(a)〜(d)）には
  **`desired_open`（awase 自身の SSOT）へのフォールバック枝が無く、TsfNative
  （Windows Terminal）での BUG-16 回復経路が丸ごと消える**。round1 M1 が
  指摘した「Imm32Unavailable 側の `HeuristicDefault` の欠落」は塞いだが、
  「TsfNative 側は `effective_open()` 末尾の `unwrap_or(desired_open)` に
  依拠している」という**別の**弱いソースを見落としていた。**本 ADR が守ると
  宣言している BUG-16 のまさに実機記録シナリオ（Windows Terminal）で退行する**
  という、最も重大な指摘。→ **`WarrantBasis::OwnSsot` を新設、P15 Step 4 に
  追加**。
- **M2**: INV-24(c)（より新しい High confidence 観測で意図を失効）が、
  BUG-19 の中核防御を破壊し、既存 pinned テスト2件
  （`ime_model.rs:1151`, `platform_state.rs:1371`）を fail させる。基準時刻が
  「意図の記録時刻」になっているため、ユーザー操作から実 IME 反映までのラグの
  間に読まれた観測（意図の結果を反映していない）で意図が失効してしまう。
  → **INV-24(c) の基準時刻を「actuation 完了時刻」に変更**（ADR-080/BUG-43 の
  `most_recent_trusted_after` と同じ意味論に統一）。
- **M3**: 旧 P14/INV-22（`most_recent_trusted()` への鮮度上限）が **BUG-26 を
  部分的に再発**させる（engine が数秒後、文の途中で OFF に戻り、連続タイピング
  中は次の休止まで回復しない）。しかも Phase 2' でゲートが分離された時点で
  導入動機自体が消えている。→ **INV-22/P14 を撤回、再提案禁止**。
- **M4**: P15(b) の `force_guards.requires_on()` が `overrides_explicit_intent()`
  を無視し、`BrokenAppBootstrap` のようなヒューリスティック guard がユーザーの
  明示 OFF を踏み越える。既存の `try_force_on_bootstrap` の用法とも符号が逆転し、
  自己再発火の恐れがある。→ **P15 Step 2 の述語を `ForceGuardSet::effective_open()`
  と同一にする**。
- **M5**: INV-24(b) の「対象一致」粒度が未定義（Codex と独立に同じ結論）。
  → 3段判定を追加。
- **should-fix**: `SelfApplied` は現行データ構造で構築不能（`generation` が
  常に `None`、basis の保存場所が無い） → **`SelfApplied` を本 ADR のスコープ
  から除外**（INV-26 撤回）。
- **should-fix**: P15 の (a)〜(d) に優先順位が無い → **Step 1〜4 の優先順位付き
  評価に変更**。
- **should-fix**: policy 切替時の `force_open_pending` 未同期（Codex と一致）
  → INV-27。
- **should-fix**: Phase 1' の効果範囲が「明示意図がある場合限定」であることが
  本文から読み取れない（Codex と一致）→ §3 案D・Phase 1' に明記。
- **nice-to-have**: `ir_post_focus_change_snapshot:499` の直接書き込みが Phase 3
  の棚卸しに未記載 → 追加。
- **総評からの一般則**: 「このリポジトリの belief 層は情報が無い状態を一貫して
  OFF 方向に解決する設計になっているため、鮮度で情報を消すと必ず OFF 方向へ
  倒れる。鮮度上限は belief ではなく actuation の根拠（warrant）側に置くべき」
  という一般原則を得た。**INV-22 の撤回理由として §2.3 P14 に明記した。**

#### round2 で確定した設計変更（反映済み）

1. `WarrantBasis` に `OwnSsot { profile }` を新設し、`SelfApplied` を除去（P12）。
2. P15 を「OR の4分岐」から「優先順位付き4ステップ評価」に全面書き直し
   （Step 2 の述語修正、Step 4 に `OwnSsot` 追加）。
3. P14/INV-22（belief 側の鮮度上限）を撤回し、再提案禁止に追加。
4. INV-24(b) に3段の対象一致判定を明記、INV-24(c) の失効基準を
   actuation 完了時刻ベースに訂正、INV-24(a) の TTL を
   `EXPLICIT_OFF_CACHE_SUPPRESS_MS` と統合する方針を明記。
5. INV-26 を撤回（対象の `SelfApplied` が無くなったため）。
6. INV-27（policy 切替時の pending 無効化）を新設。
7. §3 案D・§5 Phase 1' に「効果は明示意図がある場合限定」という誠実な
   スコープ表明を追加。P13 に同旨の「既知の限界」を明記。
8. §5 Phase 3 item 14 の棚卸しに `ir_post_focus_change_snapshot:499` を追加。

### 7.6 未反映・要検討（round2 時点、round3 で解消/訂正）

- round2 M1 で新設した `OwnSsot` basis 自体の弱点 → **round3 で検証、
  実は判定条件（`AppImeProfile` 分岐）自体に致命的なバグがあると判明**
  （§7.7 参照）。
- INV-24(b) の3段判定の誤爆リスク → **round3 で検証、tier② の前提自体が
  誤りだったと判明し削除**（§7.7 参照）。
- Phase 2' が実機で BUG-16 を再発させないか → **round3 で「TsfNative では
  Phase 2' が bit-identical である」というコードレビューだけで完結する
  論証を得た**（§7.7 シナリオ6。ただし `OwnSsot` の判定条件バグを直した
  前提での話であり、そのバグが残ったままだと逆に確実に再発する）。

### 7.7 round3: round2 修正そのものの検証（Codex・Opus、独立実施）

round2 の修正（`OwnSsot` 新設、Step 1〜4 優先順位化、INV-22 撤回、INV-24 改訂、
`SelfApplied` 除去、INV-27 新設）自体が意図通り機能するか、新たな不具合を
生んでいないかを、round2 と同じシナリオシミュレーション手法で再検証した。
**round2 で「BUG-16 を守った」つもりの修正が、round3 で見ると別の理由で
同じ BUG-16 を再発させていた**、という発見が今回の中心。

#### Codex CLI（12シナリオ、Critical 1 / High 2 / Medium 2 / Low 1）

- **Critical**: `OwnSsot` で warrant が発行されても、GJI の
  `GjiDirectStrategy::apply` は `view.control.shadow_on == true` のとき
  `VK_IME_ON` を送らず no-op（`AlreadyMatched`）を返すため、**GJI では
  BUG-16 が実装レベルで再発しうる**（MS-IME では発生しない、GJI 固有）。
  → **INV-28 を新設**。
- **High**: P15 Step 4 が Imm32Unavailable に許す `HeuristicDefault` が、
  P16/INV-25 の許可リストから漏れていた（私自身の round2 編集の矛盾）。
  → **INV-25 に追加（`HeuristicGuess` として統合）**。
- **High**: INV-24(b) の「同一プロセス」判定が広すぎる（Opus の指摘と同型、
  独立検証で確度が高い）。
- Medium/Low の指摘（`OwnSsot` の stale true、INV-27 未実装、TTL 境界）は
  Opus のより詳細な指摘に統合。

#### Opus（10シナリオ、4 must-fix / 4 should-fix）

Codex とは異なる角度から、round2 の2つの新設機構（`OwnSsot`、INV-24(b)）が
**「対象をどの軸で識別するか」を誤っている**という、より根本的な欠陥を発見。

- **must-fix（最重要）**: `OwnSsot` の発行条件を `AppImeProfile` で判定して
  いたため、`CASCADIA_HOSTING_WINDOW_CLASS`（Windows Terminal 外側ウィンドウ）
  が `AppImeProfile::from_class_name` の優先順位で `Imm32Unavailable` に
  分類され（`IMM32_UNAVAILABLE_CLASSES` と `is_tsf_native_window` の両方に
  該当するクラス名がある場合、前者が勝つ）、Step 4 が `HeuristicDefault`
  枝に入るが、この枝を実際に発火させる `reset_stale_ime_on_for_imm_broken()`
  は TsfNative SSOT 分岐（CASCADIA はここに来る）では呼ばれないため観測が
  存在せず、warrant が `None` になる。**リポジトリの `class_names.rs:65-77`
  が「2026-07-05 実機バグ」として明文で禁止している、まさにその判定方法**を
  round2 の新機構がそのまま踏んでいた。→ **`issue_open_warrant` から
  `profile: AppImeProfile` 引数を削除し、`policy.default_feedback` と
  「`HeuristicDefault` 観測が実在するか」で分岐する設計に変更**。
- **must-fix**: INV-24(b) tier②（同一プロセス + 同一 tsf-native 性）の
  設計根拠が事実誤認。`FocusChanged` は PID 変化時にしか発火しないため、
  ADR が例示した「Windows Terminal の2段フォーカス」はそもそも発火せず
  tier② を必要とせず、「UWP の2段フォーカス」は別 PID かつ tsf-native 性も
  異なるため tier② では救えない——**ADR が挙げた2つの実例のどちらも
  tier② の適用対象になっていなかった**。→ **tier② を削除、2段判定に**。
- **must-fix**: tier② が実害を持つ——`ApplicationFrameHost.exe` のような
  共有ホストプロセスが複数の無関係な UWP アプリのフレームウィンドウを
  1プロセスに集約するため、tier② は「別アプリ」を「同一対象」と誤判定する。
- **must-fix**: round2 M4 の Step 2 述語修正（`ForceGuardSet::effective_open()`
  と同じにする）が、優先順位付き評価では実質 no-op だった——Step 2 到達時点で
  `has_explicit_intent==false` が既に確定しているため、述語は自動的に
  `requires_on()` に潰れる。`BrokenAppBootstrap`（override 権限なし）が
  TTL 切れ後に明示 OFF を踏み越える。→ **Step 2 を override 権限を持つ
  reason のみに限定し、`BrokenAppBootstrap` 等は Step 4 に降格**。
- **should-fix**: `OwnSsot` の根拠 `desired_open` はグローバル単一フラグで
  ターゲットにスコープされない（Chrome の ON 意図が WezTerm への書き込み
  根拠になりうる）。ただし**これは現行と完全に同一の既存挙動**であり新規
  退行ではない——ADR の「force-write 定義に最も忠実」という評価が
  ADR-086 INV-14（空間軸）を無視した片面評価だったことが問題。→ **P12 の
  doc コメントを正直に書き換え**。
- **should-fix**: INV-24(c) の「actuation 完了時刻」基準は `FeedbackPolicy::
  Blind`（TsfNative/Imm32Unavailable）では構造的に到達不能。→ **(c) は
  `FeedbackPolicy::Read` のプロファイルにのみ適用すると明示**。
- **should-fix**: TTL が ON/OFF 意図に対称だが、Step 4 の既定推測は ON
  方向にのみバイアスを持つため失効コストが非対称。→ **INV-24(a) に
  ON/OFF 非対称の原則を明記**。
- **should-fix**: 「`reset_stale_ime_on_for_imm_broken()` の置き換え先」
  という記述が、同関数の第2の消費者（`focus_tracking.rs:343-352` の
  hard pre-sync）を見落としており、撤去すると Chrome の初回キーで
  spurious `VK_KANJI` が飛ぶ。→ **Phase 2' item 10 の記述を「観測記録は
  維持、消費のみ移す」に訂正**。
- **重要な自己訂正**: round2 で Opus 自身が提案した INV-24(b) tier② は、
  round2 での「hwnd 粒度だと UWP 親→子で毎回失効する」という懸念に基づいて
  いたが、round3 の検証で**その懸念自体が不正確だった**と判明（同一
  プロセスなら `FocusChanged` が発火しないため、そもそも失効しない）。
  round2 の指摘が全て正しいとは限らないことを示す実例として記録する。

#### round3 で確定した設計変更（反映済み）

1. `issue_open_warrant` から `profile: AppImeProfile` 引数を削除。
   `WarrantBasis::OwnSsot` を `{ profile }` 無しの unit variant に変更し、
   発行条件を `policy.default_feedback` ベースに変更（P12、P15 Step 4）。
2. `WarrantBasis::HeuristicGuess` を新設し、`HeuristicDefault` 観測と
   override 権限を持たない `ForceGuard`（`BrokenAppBootstrap` 等）の両方を
   ここに統合（P12、P15 Step 4）。
3. P15 Step 2 を「override 権限を持つ reason のみ」に限定。
4. P15 Step 4 全体に「対象で明示 OFF 意図の記録履歴が残っている間は評価
   しない」というガードを追加。
5. INV-24(a) に ON/OFF 非対称の原則を明記。
6. INV-24(b) から tier②（同一プロセス+tsf-native性）を削除、2段判定に。
7. INV-24(c) を `FeedbackPolicy::Read` のプロファイル限定に明記。
8. INV-25 に `HeuristicGuess` を追加。
9. INV-28（warrant は送信保証ではない、GJI `shadow_on` no-op の bypass が
   別途必要）を新設、Phase 3 item 16 に追加。
10. Phase 2' item 10 の「置き換え先」表現を「観測記録は維持、消費のみ移す」
    に訂正。
11. Phase 2' に「TsfNative では Phase 2' は bit-identical」という
    round3 シナリオ6 の論証を追記（§7.6 項目3 への回答）。
12. 一般則を追加（P15 末尾）: 「アプリを識別する鍵は `AppImeProfile` の
    raw な一致でも PID でもなく、目的別に用意された述語
    （`is_effectively_tsf_native()`、`AppImePolicy.default_feedback` 等）
    である。新しい判定を書く前にまず既存の述語を探すこと」。

### 7.8 未反映・要検討（round3 時点）

- INV-28（GJI `shadow_on` no-op の bypass）は Phase 3 スコープとして記録した
  が、実装の詳細（`view.control` をどう force-ON 専用に上書きするか）は
  未設計。次のレビュー候補。
- `WarrantBasis::HeuristicGuess` に `BrokenAppBootstrap` 等の `ForceGuard`
  を統合したことで、`ObservationSource` ベースの型（コメントに
  `/* or ForceOnReason 相当 */` と残した）が曖昧なまま。Phase 3 実装時に
  `HeuristicGuess` の内部表現を確定させる必要がある。
- round3 のレビューはコードレビューの範囲に留まる。Phase 0b（実機ログ）・
  Phase 2'/3 実装後の実機ソークは依然として未実施。

## 8. 実装記録（Phase 0〜2' 純粋ロジック、2026-08-10）

round1〜3 の「読んで想像してトレースする」レビューが振動する
（round2 の修正が round3 で新たなバグを生む）パターンを踏まえ、方針を
「争点になった純粋ロジックを Linux 上で replayable なテストスイートとして
実装し収束させる」に転換した。実装前にタスク分解を Opus にレビューさせ
（下記 §8.1）、指摘を反映してから実装した（§8.2）。

### 8.1 実装前タスクレビュー（Opus、5 must-fix / 7 should-fix / 3 nice-to-have）

主な指摘と対応:

- **M1**: `issue_open_warrant` に「要求する値」の引数が無いと mise バグと
  BUG-16 のシナリオが両立しない → `requested: bool` を追加し、`basis` の
  示す値と不一致なら `None` を返す設計に変更（実装 `finalize()`）。
- **M2（最重要）**: Step1（明示意図）を Step2（guard）より先に評価すると
  `PanicReset` の安全弁が壊れる（`ForceOnReason::overrides_explicit_intent()`
  は「明示意図があっても override する」がその逆になっていた）→ Step 順序を
  「Step0（override 可能な安全弁）→ Step1（明示意図）→ Step3（観測）→
  Step4（既定推測）」に修正。
- **M3**: `IntentStore` と既存 `RecordedIntent` で時計が違う（`Instant` vs
  `u64` tick）→ `IntentStore` は `TickMs` を採用（既存 `RecordedIntent`/
  `EXPLICIT_OFF_CACHE_SUPPRESS_MS` と同系統）。
- **M4**: `ForceGuardSet.guards` は private で `iter()` が無い → `iter()` を
  足さず、目的別アクセサ `active_override_reason()`/`active_heuristic_reason()`
  を追加（private 化の意図を保つ）。
- **M5**: `AppImePolicy` のコンストラクタ名・`FeedbackPolicy` の比較方法が
  実コードと不一致。起動直後は `Read`（`OwnSsot` 不発火）である点も
  テストで固定する必要 → 実コードを確認して修正、専用テストを追加。
- **S1**: `TargetId` を新設せず既存の `HwndId`（ungated、`ObserverReported`
  の hwnd と同じ型）を再利用。
- **S2**: TTL 定数は `tuning.rs` に配置（`.claude/rules/tuning-constants.md`
  の適用範囲を満たす）。
- **S5**: `derive_open()` と Step3 のロジックが将来乖離しないよう、
  `derive_open_filtered()`/`DeriveOutcome` として共通化。
- **S6/S7**: `ObserverPoll` が Step3 を発火させる前提、ImmCross プロファイルの
  シナリオをテストに追加。

### 8.2 実装内容

Windows ゲート無し（`#[cfg(windows)]` を付けない）新規/変更モジュールとして、
ADR §2.3 P11〜P16 の純粋ロジックを実装した:

| ファイル | 内容 |
|---|---|
| `state/ime_model.rs` | `resolve_open_at`/`effective_open_at`（`Instant` 引数化、INV-23）、`OpenResolution`/`DecidedBy`/`BaseDecision` 診断型 |
| `state/observation_store.rs` | `derive_open_filtered`/`DeriveOutcome`（`derive_open()` 本体と共通化、S5） |
| `state/ime_event.rs` | `ObservationAuthority`/`ObservationSource::authority()` |
| `state/force_guard.rs` | `active_override_reason`/`active_heuristic_reason` |
| `state/intent_store.rs`（新設） | `IntentStore`/`RecordedTargetIntent`（`HwndId` キー、ON/OFF 非対称 TTL、INV-24） |
| `state/open_warrant.rs`（新設） | `OpenWarrant`/`WarrantBasis`/`HeuristicGuessSource`/`issue_open_warrant()`（Step0〜4、INV-20・25） |
| `tuning.rs` | `EXPLICIT_ON_INTENT_TTL_MS`（未実測の暫定値と明記） |

`state/mod.rs` に `#[allow(dead_code)]`（`ime_profile_driver`/`gji_direct_mechanism`
と同じ「配線待ちの純粋モジュール」パターン）で登録。**`runtime/`・
`platform_state.rs` への実配線（既存 `last_intent`/`is_eligible_for_ime_force_on()`
の置き換え）は行っていない**（Phase 3 スコープ、Windows専用コードのため
このセッションでは検証不能）。

**§5 Phase 0a item1（`resolve_open_at`/`Instant` 引数化）は完了。item2
（`ime_apply_should_defer`/`is_focus_transition_settling` の `Instant` 引数化）は
両関数が `runtime/`・`key_pipeline.rs`（Windows専用）にあるため未実施のまま。**

### 8.3 テスト

各モジュール内の `#[cfg(test)] mod tests` に pinned test として配置した
（round1〜3 の議論を反映し、golden_scenarios.rs 形式の独立統合テストファイルは
作らなかった——`issue_open_warrant` に単一の event 駆動エントリポイントが
無く、同じ純粋関数を同じフィクスチャで呼ぶだけの重複になると判断したため、
§7.6 の当初案から変更）:

- `state/open_warrant.rs`: 14 test（mise バグ本体〈`step3_conv_open_inference_never_wins_actuation`〉、
  BUG-16 非退行〈`step4c_own_ssot_for_blind_profile_matches_bug16`〉、
  `AppImeProfile` 誤判定の pinned test〈`step4c_branches_on_default_feedback_not_raw_profile_value`〉、
  PanicReset/BrokenAppBootstrap 優先順位のペア、Corroborated、
  `is_japanese_ime=false` 等）
- `state/intent_store.rs`: 8 test（対象分離〈UWP 共有ホストプロセス誤爆回避相当〉、
  TTL 非対称、同一対象の意図置換）
- `state/ime_model.rs`: 5 test 追加（`resolve_open_at`/`DecidedBy` の内訳、
  guard_override）
- `state/observation_store.rs`: 4 test 追加（`derive_open_filtered`/`DeriveOutcome`）
- `state/ime_event.rs`: 6 test 追加（`authority()` の全 variant、`Gji`/`Tsf` が
  production では未使用の dead variant である旨をコメントで明記）
- `state/force_guard.rs`: 4 test 追加（新設アクセサ）

**実行結果（全緑、2026-08-10）**: `cargo test -p awase-windows --lib`
326件、`--test golden_scenarios` 22件、`--test architecture_guard` 21件、
`--test journal_replay` 1件、`--test drift_correction_replay` 2件、
`--test layer_boundary_guard` 8件。既存テストは1件も変更しておらず、
全て非退行のまま通過している（`effective_open()` のラッパー化・
`derive_open()` の共通化がいずれも既存の pinned test で挙動不変を
確認済み）。`cargo clippy -p awase-windows --lib --tests`
（`pedantic`/`nursery` deny、`crates/awase-windows/Cargo.toml` の
`[lints.clippy]`）で新規ファイル（`intent_store.rs`/`open_warrant.rs`）は
指摘ゼロ。既存コードに残る pedantic 指摘（`gji_fsm.rs` 等）は本セッションの
変更と無関係の既存債務のため対象外とした。

`docs/known-bugs.md` に **BUG-63** として起票済み
（`.claude/rules/fix-requires-evidence.md` (a)(b) 両方を満たす）。

### 8.4 残された制約（正直な記録）

- **Windows 実機での検証は一切行っていない**。`runtime/`・`platform_state.rs`
  はこのセッションのサンドボックスでコンパイルすらできない
  （`#[cfg(windows)]`）。
- Phase 3（実配線）・INV-28（GJI `shadow_on` bypass）・INV-23 後半
  （`ime_apply_should_defer`/`is_focus_transition_settling`）は未実装のまま。
- 実装した純粋ロジックが「論理的に正しいこと」はテストで固定したが、
  「実際の runtime イベント順序・タイミングと整合すること」は配線するまで
  確認できない。§7 の round1〜3 で見つかった「読んで想像するレビューには
  天井がある」という限界は、実装後も配線前の段階では完全には解消されて
  いない。
- 次のレビュー（§8.5、Opus に実装差分を渡す）は、テストで表現しきれていない
  設計判断（`WarrantBasis::HeuristicGuess` の内部表現の曖昧さ§7.8、
  `focus_epoch`/`ime_mode_focus_gen` の二重フェンス§7 round1 S5 等）を
  優先して見てもらう。
- **網羅オラクルテストが守る範囲と守らない範囲**（round4 最終確認、Opus の
  総評より）: `exhaustive_step_priority_matches_independently_written_oracle`
  が防ぐのは**実装のドリフト**（Step の順序を書き間違える、分岐条件を
  取り違える、リファクタで挙動が変わる）であり、round2→round3 で起きた
  振動の大半はこの型だった。一方、**仕様そのものの誤り**は防げない——
  オラクルも実装も同じ ADR §2.3 P15 の文面から同じセッションで書かれて
  いるため、文面自体が間違っていれば両方が同じ間違いをする（round3 M2 の
  Step 順序逆転は、当時の ADR 本文が誤った順序を明記していたため、この型の
  テストだけでは検出できなかったはずで、実際に検出したのはコードベースに
  既に存在していた不変条件——`force_guard.rs` の `overrides_explicit_intent()`
  の doc と `ForceGuardSet::effective_open()` の実装——との突き合わせだった）。
  したがって一般則として: **網羅オラクル = 仕様が正しい前提で実装のドリフトを
  防ぐ**、**既存実装との差分テスト = 仕様そのものが既存の不変条件と矛盾
  していないかを防ぐ**、の2種類は別の防御であり、両方揃って初めて
  「テストで収束した」と言える。今回は前者を手に入れ、後者も部分的に
  手に入れている（`ForceGuardSet::resolve()` への一本化によって
  `ime_model` 側が独自に述語を複製し乖離する道を塞いだ）。Phase 3 の
  最初のタスクとして、「既存の `is_eligible_for_ime_force_on()` と
  `issue_open_warrant()` が同じ入力に対して同じ結論を出すこと」を確認する
  差分テストを書くことが推奨されている（§8.8）。

### 8.5 実装後の最終レビュー記録（round4、Opus）

§8.1〜8.3 の実装（T1〜T10）を Opus に渡し、コードレベルの最終レビューを
依頼した。round1〜3 のレビューへの反映確認（13件中12件は正しく反映）に
加え、**実装して初めて見える新規の欠陥を4件（must-fix）発見した**。

- **M-A（最重要）**: `IntentStore` の OFF 意図に有効な失効条件が1つも無かった
  （TTL なし・eviction なし・HwndId 再利用の考慮なし）。round3 で「OFF は
  無期限」と決めた前提（`last_intent` と統合されフォーカス単位で有界になる）
  が、対象ごとに永続する実装では成立せず、drift correction が永久に
  再同期できない固着を作りうる。既存 precedent `HwndImeCache`
  （`focus/hwnd_cache.rs`、`HWND_CACHE_MAX_AGE_MS` で必ず期限を切る）との
  乖離も指摘された。
- **M-B**: `Instant`/`TickMs` を注入できる形にした（INV-23）にもかかわらず、
  実際に時刻を変えて挙動が変わることを確認するテストが1本も無かった
  （全テストが `Instant::now()` をそのまま渡していた）。
- **M-C**: `resolve_open_at()` の `guard_override` が、guard が active な
  だけで実際には値を override していない場合にも reason を返す誤情報
  バグ。診断 API 自身が「判定根拠が失われていた」という本 ADR の動機に
  反する形で嘘をつく状態だった。`ForceGuardSet::effective_open()` の述語を
  手書きで複製したことが原因（`platform_state.rs:1300-1304` に同型の
  既知パターンあり）。
- **M-D**: `IntentStore`（`HwndId` キー）・`HwndImeCache`（`(pid, class_name)`
  キー）・`persistent_explicit_off_ms`（グローバル単一値）の3者が異なる
  粒度で、ADR §5 Phase 1' が謳う「統合する」が現状の実装のままでは
  成立しない。

should-fix 6件（`DeriveOutcome`/`BaseDecision` のホットパス Vec 確保、
`WarrantBasis::DirectRead` の命名が Medium 単独ソースにも使われ実態と
不一致、Step 4a の鮮度窓非対称の doc 不足、guard 期限非考慮の doc 不足、
`#[allow(dead_code)]` の範囲過大、優先順位ペアテスト3組の不足）・
nice-to-have 3件（10引数関数、doc 見出し不一致、未使用 sort）も指摘された。

総評: 「Step 評価ロジックについては収束したが、意図のライフサイクル
（M-A）と対象同一性（M-D）は Phase 3 に持ち越すのではなく、このセッション
（Linux で完結する）で決めてテストに焼き込むべき」という評価だった。

### 8.6 round4: 指摘への対応

M-A/M-B/M-C の3件と should-fix 全件・nice-to-have 全件を実装した
（M-D は設計決定そのものを次節 §8.7 に記録し、キー統一の実装は見送った
——3つの機構の統合は本 ADR のスコープを超える別課題と判断したため、
「決定しないまま放置する」ことは避け、少なくとも粒度の不一致を明文化した）。

| 指摘 | 対応 |
|---|---|
| M-A | `tuning.rs` に `EXPLICIT_OFF_INTENT_TTL_MS`（`HWND_CACHE_MAX_AGE_MS` と同値、ON より意図的に長い）を新設。`IntentStore::lookup`/`record` を ON/OFF 共通の `ttl_for()`/`RecordedTargetIntent::is_expired()` に統合。`record()` のたびに他対象の期限切れエントリも掃除する（`HwndImeCache::save()` と同じパターン） |
| M-B | `resolve_open_at`/`issue_open_warrant` それぞれに、`now`/`now_ms` を実際にずらして FRESH ウィンドウ・TTL を跨ぐと結果が変わることを確認するテストを追加（`resolve_open_at_now_argument_actually_affects_result`、`now_argument_gates_step3_via_fresh_window`、`now_ms_argument_gates_step1_via_intent_ttl`） |
| M-C | `ForceGuardSet::resolve(desired_open, has_explicit_intent) -> (bool, Option<ForceOnReason>)` を新設。「実際に override した場合のみ `Some`」を返す。`effective_open()` はこの `.0` を返す薄いラッパーに変更。`resolve_open_at` もこれ経由に統一し、手書き複製を廃止 |
| S-A | `DeriveOutcome::MediumConsensus`/`BaseDecision::DeriveMedium` を `Vec<ObservationSource>` から `{ first, second: Option<_> }` の固定2フィールドに変更（`effective_open()` は全 `KeyDown` で呼ばれるホットパスのため） |
| S-B | `WarrantBasis` に `SingleIndirect(ObservationSource)` を新設し、`DirectRead` は High confidence 専用に戻した |
| S-C | `ObservationStore::heuristic_default(now)` アクセサを新設し、`per_source` への直接アクセスをやめた。Step 3 と異なり FRESH 窓を適用しない設計意図を doc に明記 |
| S-D | `active_override_reason`/`active_heuristic_reason` の doc に「`expires_at` を見ない（`effective_open()` と同じ意味論）」を明記 |
| S-E | `state/mod.rs` の `#[allow(dead_code)]` を `intent_store`/`open_warrant` モジュールから外した（`pub mod` はクレート公開 API のため実際には dead_code 警告が出ないことを確認） |
| S-F | 網羅テスト（§8.6 末尾）で自動的にカバー（個別3本ではなく全組み合わせで検証） |
| N-A | `issue_open_warrant` の10引数を `WarrantContext<'a>` 構造体に集約（`requested`/`target` のみ残す） |
| N-B | module doc の見出しを「Step 0〜4」に修正 |
| N-C | 未使用の `sort_by_key` を削除し、`PerSourceObservations::iter()` の宣言順に基づく決定的な `assert_eq!` に変更 |

### 8.7 M-D の設計決定（キー粒度の不一致を明文化）

`IntentStore`（`HwndId`）・`HwndImeCache`（`(pid: u32, class_name: String)`）・
`persistent_explicit_off_ms`（グローバル単一値）は**異なる対象識別の粒度**を
持つ。§5 Phase 1' item 7 が「これらを統合する」と書いているが、round4 の
実装は `HwndId` を選んだのみで、統合は行っていない。

**このセッションでの決定**: 統合は行わず、`IntentStore` は `HwndId` の
まま Phase 1' の成果物として確定させる。理由:

1. `HwndId` は `ObservationSource` の観測（`ImeObservation.hwnd: HwndId`）と
   同じ型であり、「対象識別の鍵は観測と意図で揃える」という一貫性がある。
2. `(pid, class_name)` への統一は `HwndImeCache` 側の役割（HWND の短命性・
   再利用に対する耐性）を `IntentStore` にも持ち込む設計変更になり、
   M-A で入れた TTL + eviction（HWND 再利用への対策）と役割が重複する。
   TTL による有界化で HWND 再利用の実害は緩和されているため、
   `(pid, class_name)` への統一は必須ではなく、Phase 3 で実際に3機構を
   配線する際に改めて判断する。
3. グローバル単一値（`persistent_explicit_off_ms`）は `IntentStore` の
   対象別管理そのものが上位互換であり、統合というより「置き換え」になる
   （Phase 3 スコープ）。

**Phase 3 着手前に必ず決定すべき事項として記録する**（次のレビューで
再確認されるべき）: `HwndImeCache` と `IntentStore` を実際に統合するか、
別々のまま運用するか。統合しない場合、両者が異なる結論を出す状況
（例: `HwndImeCache` はヒットするが `IntentStore` はミスする、または逆）
が実機で発生しうることを許容する設計になる。

### 8.8 round4 最終確認（Opus）— must-fix ゼロ

§8.5〜8.6 の対応を Opus に再度確認させた。**13件すべて正しく反映を確認、
must-fix は残っていない。**

should-fix/nice-to-have を3件（G1〜G3）指摘され、いずれも対応済み:

- **G1**: 網羅テストの `category()` が `DirectRead`/`SingleIndirect`/
  `Corroborated` を "Observation" 1categoryに畳んでいたため、round4 で
  新設したばかりの `SingleIndirect`/`Corroborated` の区別が検証対象外
  だった。加えて `ALL_STEP3_CASES` に「Medium 2ソースが**一致**する」
  ケースが無く、`Corroborated` 分岐がどの組み合わせからも到達されて
  いなかった。→ `Step3Case::MediumAgree(bool)` を追加、`category()` を
  5種（`ExplicitUserIntent`/`SafetyValve`/`DirectRead`/`SingleIndirect`/
  `Corroborated`/`HeuristicGuess`/`OwnSsot`、実質7種）に分割。
- **G2**: `is_japanese_ime` が網羅テスト内で `true` に固定されていた。
  → 9軸目としてループに追加（4608通りに拡大）。
- **G3**: 時間軸（`now`/`now_ms`）が凍結されていることが明示されて
  いなかった。→ オラクル関数の doc に「時間軸は本テストでは凍結する、
  FRESH/TTL の境界は個別テストが担当する」と明記。

補足2件も対応: `ForceOnReason::ProfilePolicy` が `issue_open_warrant` 経由
では一度も試されていなかった点（`resolve_profile_policy_also_overrides_explicit_intent`
を追加）、`ForceGuardSet::resolve()` が複数 guard 同時発火時に挿入順で
弱い reason を報告しうる点（override 権限を持つ reason を優先するよう修正、
`resolve_prefers_override_reason_over_heuristic_when_both_active` で固定）。

round4 修正後、`cargo test -p awase-windows --lib`（338件）・
`--test golden_scenarios`（22件）・`--test architecture_guard`（21件）・
`--test journal_replay`（1件）・`--test drift_correction_replay`（2件）・
`--test layer_boundary_guard`（8件）全緑。`cargo clippy -p awase-windows
--lib --tests`（pedantic/nursery deny）で新規・変更ファイルへの指摘ゼロ
（既存債務は対象外）。

網羅的組み合わせテスト（`exhaustive_step_priority_matches_independently_written_oracle`、
4608通り × 独立記述のオラクルとの突合）は、`issue_open_warrant` の実装を
一切参照せずに ADR §2.3 P15 の仕様文から直接書いたオラクルと突き合わせる
方式のため、この関数単体については「読んで想像するレビュー」ではなく
「機械的網羅」で収束したと言える（§8.4 に「守る範囲と守らない範囲」の
一般則を追記済み）。対象は `issue_open_warrant` の Step 0〜4 ロジックのみ
であり、`IntentStore`/`ObservationStore`/`ForceGuardSet` 個々の実装の
正しさや、Phase 3 配線後の実際の runtime イベント順序との整合性までは
検証範囲に含まれない。

**Opus の総評**: 「純粋ロジックとしては収束したと言える」「Phase 3 への
持ち越しに同意する」。理由: (1) Phase 3 で必要な判断（対象 `HwndId` への
到達経路、`app_policy` の同一プロセス内追随、INV-27 の `reload_config`
対応）はすべて `#[cfg(windows)]` 側の設計判断でこのセッションでは検証
できない、(2) §8 が引き継ぎ文書として十分機能する、(3) 既存 326→338
テストが1件も変更されずに通っている＝既存挙動に対して完全に加算的な
変更であり、リスクなくマージできる状態。**Phase 3 の最初のタスク**として、
「`IntentStore`/`app_policy` が既存の `last_intent`/`AppImePolicy` と一致する
入力に対して、`issue_open_warrant(requested=true, ..).is_some()` が既存の
`is_eligible_for_ime_force_on()` と一致すること」を確認する差分テストが
推奨されている（§8.4 の「既存実装との差分テスト」に対応）。

M-D（キー粒度を統合しない判断）についても「目的が違うものを統合しない
のは妥当」と支持された——`HwndImeCache` は「アプリ単位の記憶」、
`IntentStore` は「actuation の対象同一性」（ADR-086 INV-14 の空間軸）で
役割が異なる。§5 Phase 1' item 7 の文言（「3つを統合した `IntentStore` を
導入する」）は §8.7 の決定に合わせて訂正済み。
