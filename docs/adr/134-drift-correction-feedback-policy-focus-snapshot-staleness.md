# ADR-134: `app_policy` の `FeedbackPolicy` が正しく初期化・再導出されず、読み戻し不能な状態で `FeedbackPolicy::Read` の無条件再送に陥る（BUG-114）

## ステータス

**設計承認（Opus 敵対的レビュー round4 で承認、v5）。
検証計画0（D4 先行実装によるログ取得）に進める水準。**
BUG-113（[ADR-133](133-gji-ime-mode-key-sendinput-batch-shape.md)）の
実機調査中に発見。`docs/known-bugs.md` BUG-114 に実機ログを記録済み。

v1 は根本原因を「`FocusChanged` 発火の瞬間にクラス名の分類が一時的に
`Standard` へ倒れた」という**同一 tick 内のレース**と推定し、その推定の上に
D1（ライブ再導出）・D4（`FocusChanged` reducer への診断ログ）・案 B の
却下（「D1 実装後は再発窓が極めて狭い」）を組み立てていた。round1 の
敵対的レビューで、この推定が机上の空論ではなく**コード上ほぼ確実に
成立しない**こと、かつ**もっと単純で再現性の高い機構が別に存在する**こと
が判明したため、v2 で「問題」節・決定を全面的に書き直した。round2 では
v2 が新たに抱えた2件の Blocker（D4 が決定節から消えたまま本文に参照が
残る dangling reference、D1 本文とD1a結論の実装指示の矛盾）と、D1a の
影響範囲の過小評価・D1b の `Actuation` 構造体変更漏れ・第4の機構（IMM
学習閾値到達前の窓）・D1c の over-claim を修正した。round3 で
Blocker はゼロと確認されたが、実装指示の精度に関する Major 4件
——D4 が D1 実装後にスナップショットを読めなくなる点、D4 の出力項目
だけでは根本原因1/2-1/3 を判別できない点、D1b の backoff early-return
の位置を誤ると journal 汚染と force-ON 抑止が起きる点、D1a の不変条件
テストの正しい置き場所——を反映し、検証計画に「D1c 実装前の D4 先行
ログ取得」を追加した。

## 問題

### 症状（実機ログ、dragonflyg4、2026-09-05）

Windows Terminal + GJI で BUG-113 の再現手順（Engine 有効時に物理半角/全角
キーを1回押す）を実行すると、`runtime/ime_refresh.rs::ir_apply_drift_correction`
が **約14秒間、20〜90ms おきに連続発火**し、`VK_IME_OFF` を `SendInput` で
送り続ける（`gave up` ログは0件）。ログの `strategy=` タグは一貫して
`drift_correction_read`。

### 根本原因1（最有力・v1からの訂正）: `app_policy` は起動から最初のプロセス切替までの間、一度も初期化されない

`ImeModel::app_policy` の書き込み口は次の2箇所のみである。

- `state/ime_model.rs:283` — 初期値 `AppImePolicy::standard()` ＝
  `from_profile(ImmCross)` ＝ **`FEEDBACK_READ`**。
- `state/ime_model.rs:615` — `ImeEvent::FocusChanged` の reducer。

そして `ImeEvent::FocusChanged` の本番 dispatch 元は
`runtime/focus_tracking.rs:543` の1箇所だけで、これは
`apply_focus_probe_result`（`focus_tracking.rs:98`）が **プロセスが
変わったとき（`process_changed`）にしか**呼ばない `on_focus_process_changed`
の中にある。その `process_changed` は

```rust
// runtime/focus_tracking.rs:331
let process_changed = last_pid.is_some_and(|last| last != classified.process_id);
```

で、**起動直後の最初のフォーカスでは `last_pid == None` なので必ず false**
になる。起動経路の `establish_initial_focus_scope`（`focus_tracking.rs:108-145`）
は意図的に `FocusChanged` を dispatch しない
（`sync_initial_focus_fence` で `InitialFocusFenceEstablished` だけを流す。
`architecture_guard.rs::establish_initial_focus_scope_does_not_write_ime_belief`
がそれをテキスト検査で固定している）。

**帰結: awase 起動から「最初のプロセス切り替え」までの間、フォーカス中
アプリが何であろうと `app_policy.default_feedback == Read` である。**
Windows Terminal でも Chrome でも WezTerm でも同じ。ユーザーが
「Windows Terminal から awase を起動して、そのまま何もアプリを切り替えずに
入力を始める」という自然な操作をすると、これは例外ケースではなく**既定
ケース**になる。この機構は「一時的なクラス名の取り違え」を一切仮定せずに
BUG-114 の観測（`strategy=drift_correction_read` が一貫、
`Blacklist drift correction` ルーティングも一貫）を説明する。

### 根本原因2: `FocusChanged` を経由しない分類変化が構造的に少なくとも2経路ある

上記1が唯一の機構ではない。`FocusChanged` を伴わずに
「ライブ判定は正しく更新されるが `app_policy` は取り残される」経路が
コード上少なくとも2つ実在する。

1. **同一プロセス内のクラス変化**: `advance_focus_tracking` は毎回
   `update_focus_info_with_process_name` を呼んで `current.app_profile` を
   **再計算**するが、プロセスが変わらない限り `FocusChanged` は飛ばず、
   `notify_focus_hwnd_updated_if_needed`（`focus_tracking.rs:437-449`）が
   `FocusHwndUpdated` を出すだけ。Windows Terminal の
   `CASCADIA_HOSTING_WINDOW_CLASS`⇄`Windows.UI.Input.InputSite.WindowClass`
   の遷移はまさにこのパターンに該当しうる。
2. **IMM 能力の学習による降格**: `focus/tracker.rs:155-168` の
   `apply_learned_imm_capability` が `Standard → Imm32Unavailable` に
   ライブで降格させる（`[imm-learning] profile 降格` ログ）。これは
   `run_ime_refresh` が毎 tick 走らせるが、`app_policy` には一切反映
   されない（BUG-56/BUG-107 と同型の「学習キャッシュがスナップショットに
   波及しない」問題）。

### 根本原因3（round2 追加）: IMM 能力の学習が閾値に達する前は、live/snapshot が両方とも揃って誤っている

根本原因2-2 は「降格**後**にスナップショットが取り残される」問題だが、
その裏返しとして「降格**前**」（学習が `IME_DETECT_MISS_THRESHOLD` に
達するまでの窓、BUG-56 と同型）がある。この間は `current_app_profile()`
（ライブ）も `app_policy`（スナップショット）も揃って `Standard` であり、
**両者は一致しているのに両方とも誤っている**。これは SSOT 分裂ではない
ため、D1（ライブ再導出）でも D1c（bootstrap 初期化）でも直らない
——「ライブ判定に合わせる」系の修正全般が原理的に効かない、独立した
第3の機構として記録する。

v1 が「`FocusChanged` 発火の瞬間のクラス名誤分類」として単一の機構に
帰していた現象は、上記1（本命）・2・3の4経路すべての可能性を含む。
**確定はしていない**ため、D4（診断ログ、後述）でどの機構が実際に支配的
かを実機で切り分ける。

### なぜ v1 の「1回の構築分に縮小される」という説明は誤りか

`runtime/ime_actuation.rs::actuation_for` は次を明記する。

> 再利用時、引数 `policy` は無視される（既存試行が構築時に持った `policy`
> がそのまま使われ続ける）。

`Actuation`（進行中の drift correction 試行）は**構築時**の `policy` を
生存期間中ずっと保持する。`target`（`desired`）が変わるか `FocusChanged`
で discard・再構築されるため、v1 は「構築のたびにライブ判定を読み直せば
誤分類は1回の構築分に縮小される」と主張していたが、これは2点で誤って
いた（round1 レビュー Finding 3）。

**(a) `FocusChanged` と同一 tick 内では、ライブ値もスナップショットと同じ
値になる。** `ir_execute()` 内の実行順序は次の通り:

```
ir_stage_focus
├ apply_focus_probe_result
│   ├ advance_focus_tracking → update_focus_info_with_process_name
│   │     ← current_app_profile はここで確定
│   └ on_focus_process_changed → dispatch FocusChanged{profile}
│         → state/ime_model.rs:615 app_policy = from_profile(profile)
│               ← スナップショット確定
└ ir_notify_focus_changed → discard_actuation()
（同一 ir_execute() 呼び出し内で）
ir_stage_notify → ir_apply_drift_correction
├ let policy = ...（D1 案：ここでライブ再読）
└ actuation_for(desired, policy) ← 新しい Actuation を構築
```

`current_app_profile()` を再計算するコードはこの2点の間に存在しない。
したがって同一 tick 内で discard → 再構築が起きる限り、ライブ側とスナップ
ショット側は**必ず同じ値**になる。「`FocusChanged` の瞬間だけ一時的に
別のクラス名だった」という v1 の仮説が真であれば、その瞬間のライブ値も
同じく誤っているため、**ライブ再導出は no-op になる**。

D1（後述）が実際に効く理由は別にある。`check_drift_correction`
（`state/platform_state.rs:865-908`）は `drift_duration >=
DRIFT_CORRECTION_THRESHOLD_MS`（400ms、`tuning.rs:222`）を要求し、
`clear_on_focus_change` が `FocusChanged` のたびに drift 計測を
リセットする（`observation_store.rs:498-502`）。つまり `FocusChanged`
後の最初の `Actuation` 構築は通常 400ms 以上あとになり、その間に
`update_focus_info_with_process_name` が毎 tick 再分類する猶予がある
——これが D1 の効き目の実体である。**この依存は load-bearing だが、
今日既に破られているケースがある**: `platform_state.rs:877-881` は
`explicit_intent == Some(desired) && is_strong_intent` のとき閾値を
**0** にする。BUG-113/114 の再現操作（物理半角/全角キー1回押下）は
`UserIntentSource::PhysicalImeKey` の強い意図を立てるため、**まさに
この再現操作自体が、D1 の効き目が最も薄い経路である。**

**(b) `FeedbackPolicy::Read` は、誤った policy を一度掴むと被害継続時間が
一切縮まらない。** `decide_actuation_action` は `Read` に対し絶対に
`GiveUp` を返さない（`state/ime_actuation.rs:172`）。`Read` 分岐で
`discard_actuation` が呼ばれるのは `Resolution::Confirmed`（読み戻し
収束）のときだけで、読み戻し不能なアプリでは構造的に発生しない
（それが BUG-114 の本体）。したがって誤った `Read` を掴んだ `Actuation`
は「`target` が変わるか、次の**プロセス**切替が起きるまで」生存する
——同一プロセスに留まる限り数十分〜数時間、**v1 の修正前とまったく
同じ長さ**。D1 は「誤った値を引く確率」を下げるだけで、「引いてしまった
ときの被害継続時間」は縮めない。

## 決定

### D1: `ir_apply_drift_correction` の `policy` をスナップショットではなくライブ判定から導出する

`runtime/ime_refresh.rs:605` の

```rust
let policy = self.platform_state.ime.default_feedback();
```

を、`self.platform.current_app_profile()` から都度導出する形に変更する。

```rust
let live_profile: ImePolicyProfile = self.platform.current_app_profile().into();
let policy = AppImePolicy::from_profile(live_profile).default_feedback;
```

**round2 で訂正（実装指示の矛盾を解消）**: v2 は D1 本文で「述語ベース
（`can_use_imm32_cross_process()` の bool 分岐で `FEEDBACK_READ`/
`FEEDBACK_BLIND` を直接選ぶ）」を提示し、D1a の結論で「`.into()` 変換の
ままでよい」と真逆のことを書いており、同じ1行に対して排他的な2つの
実装が指定されていた。**`.into()` 版に一本化する**（上記コードブロック）。
述語版は不採用とする——理由は `app_ime_policy.rs:194-199` が明記する
INV-44/ADR-089 §2.5「`caps` が唯一の宣言点、`FEEDBACK_READ`/`FEEDBACK_BLIND`
リテラルを他所に二重に置くと `ImeProfileDriver` の parity テストと同じ
負債の3本目になる」という規約に、述語版（定数を `pub(crate)` にして
runtime 層で直接分岐）が抵触するため。

**D1 は D1a とセットでなければ正しくない**: `.into()` 版は
`ImePolicyProfile` に `InputRelay` が写像されていることに依存する
（D1a 参照）。D1a 抜きで D1 単体を実装すると、`InputRelay` プロファイルで
`ImmCross`（`FEEDBACK_READ`）に丸められる食い違いが「ライブ判定から
導出した結果として」再生産される。

**位置づけの訂正（round1 反映）**: D1 は根本原因1・2（bootstrap 窓・
`FocusChanged` を経由しない分類変化）に対しては有効だが、根本原因
（v1が想定した）「`FocusChanged` と同一 tick 内の一時的誤分類」、および
根本原因3（IMM 学習閾値到達前の窓）に対しては no-op であり、かつ
`Read` を一度でも誤って掴んだ場合の被害継続時間は一切縮めない（上記
「なぜ v1 の説明は誤りか」参照）。**D1 単独では不十分であり、D1a・D1b
（後述）とセットで初めて実効的な修正になる。**

### D1a: `ImePolicyProfile` に `InputRelay` を追加し、`caps()` に `Blind` の行を足す

D1 が `self.platform.current_app_profile().into(): ImePolicyProfile` で
`AppImeProfile → ImePolicyProfile` の写像を経由すると、`InputRelay` が
すり抜ける。写像（`focus/class_names.rs:286`）は

```rust
AppImeProfile::Standard | AppImeProfile::InputRelay => Self::ImmCross,
```

で `InputRelay` を `ImmCross`（→ `FEEDBACK_READ`）に丸めるが、ライブ述語
`can_use_imm32_cross_process()`（`focus/class_names.rs:216-221`）は
`InputRelay` を `false` と判定する。**`InputRelay` は「policy=Read かつ
read strategy=Blacklist」を構造的に生成する唯一の profile であり、D1を
上記の`.into()`変換で実装すると、`app_overrides.input_relay_apps` に
登録されたアプリ（MWB ヘルパ等、issue #136/ADR-119/BUG-90）にフォーカス
があるとき、BUG-114 と同じ食い違いを「ライブ判定から導出した結果として」
再生産する。**

ADR-119 の gate（`ImeController::apply`/`run_open_chain_async`/
`fallback_write`/`dispatch_ime_set_open`）が `NotOwned` を返すため
`VK_IME_OFF` の実注入までは起きないが、drift correction ループ自体は
止まらず、`actuation_for`/`journal.record`/`dispatch_event(DriftDetected)`/
`apply_ime_open_with_belief` が毎 tick 回り続ける。BUG-111（journal
汚染・「重い」系）の燃料になる。

**決定**: `ImePolicyProfile` に `InputRelay` variant を追加し、
`caps(InputRelay, _)` に `FEEDBACK_BLIND` の行を明示的に追加する
（`focus/class_names.rs:270-285` の「gate が先に効くので chain は
実行時には使われない」というコメントは `chain` については正しいが
`feedback` については正しくない——`feedback` は gate より手前の
`ir_apply_drift_correction:605` で読まれるため、このコメント自体も
修正する）。この変更により「同じ情報源だが違う述語」という食い違いの
根そのものを塞ぐ。

不変条件テストを追加する（`fix-requires-evidence.md` (a) を満たす、
Linux で `cargo test -p awase-windows` により実行可能）:

```rust
// AppImeProfile 全4値 × ImeKindId 全数
assert_eq!(
    matches!(caps(profile.into(), kind).feedback, FeedbackPolicy::Blind { .. }),
    !profile.can_use_imm32_cross_process()
);
```

**置き場所（round3 で訂正、レイヤ境界違反を回避）**: このテストは
`profile: AppImeProfile`（focus 層）と `caps`（state 層）を同時に参照
する。`focus/class_names.rs:262-266` の `From` 実装の doc が「変換は
runtime 境界で行い、state 層が focus 層に直接依存しない設計を維持する」
と明記しており、`state/app_ime_policy.rs` の既存テストは focus 型を
一切 import していない。**`app_ime_policy.rs` にこのテストを置くと
`#[cfg(test)]` とはいえ state→focus の依存を新設する**ため、正しい
置き場所は `focus/class_names.rs` の `mod tests`（focus→state の依存は
`impl From<AppImeProfile> for ImePolicyProfile` で既に存在する）。
`app_ime_profile_converts_to_expected_policy_profile`
（`class_names.rs:480-498`）の隣に置く。

このテストは `AppImeProfile` の4値だけを回すため
`ImePolicyProfile::{Plain, Unknown}` は対象外になる（`AppImeProfile` に
preimage が無い）。「`ImePolicyProfile` 側で全数を回す形に改善しては
いけない」——`Plain`/`Unknown` には `can_use_imm32_cross_process()` の
対応物が無く、不変条件そのものが定義できないため——という但し書きを
テストのコメントに添える。
現状この不変条件は `InputRelay` で falsify されるため、テストを書いた
時点で red になる——それが正しい設計圧である。

**影響範囲（round2 で明確化、過小評価の指摘への対応）**: `InputRelay`
variant の追加は `caps()` の1行では終わらない。最低限以下を実装時に
確認・決定する:

1. `state/app_ime_policy.rs:254` の `const ALL_PROFILES: [ImePolicyProfile; 5]`
   を6に拡張し、`all_profiles_covers_every_variant` の match にも
   arm を追加する。
2. `state/ime_profile_driver.rs:223` に**2つ目の** `ALL_PROFILES:
   [ImePolicyProfile; 5]` が存在する（コメントに「あちらは private
   なのでここでも持つ」と明記）。**こちらも6へ拡張する**——片方だけ
   直すと全数テストが穴を持ったまま green になる。
3. `state/ime_profile_driver.rs:197-205` の `driver_for(profile)` は
   網羅 match のため `InputRelay` 追加でコンパイルエラーになるが、
   **どのドライバに写すかは設計判断**である（ADR-090 決定F-2 で凍結
   された領域に手を入れることになる点を実装コミットに明記する）。
4. `assert_owns_kanji_parity`（`ime_profile_driver.rs:236-243`）が
   `AppImePolicy::from_profile(profile).owns_physical_kanji` と driver
   の値を突き合わせるため、3の選択と整合させる。
5. `caps(InputRelay, ·)` の `chain` に何を置くかを決める。`Plain`/
   `Unknown` と同じ「到達しない安全既定」として扱うことを明記し、
   `caps_chains_have_no_unreachable_trailing_element`（INV-44）が
   検査する末尾要素の到達可能性と矛盾しない選択にする。
6. `AppImePolicy::from_profile` の
   `owns_physical_kanji: !matches!(profile, ImePolicyProfile::TsfNative)`
   （`app_ime_policy.rs:220`）は `InputRelay` に対し `true` を返す。
   `AppImeProfile::InputRelay` の doc（`class_names.rs:144-147`、
   「IME actuation を所有せず、物理モードキーも suppress せず」）とは
   字面上矛盾するが、現状も `InputRelay → ImmCross → true` であり
   **回帰ではない**（実効的な suppress 判定は `transport.rs::plan` が
   握るため実害は限定的、BUG-46 の既存整理どおり）。D1a はこの写像に
   手を入れる変更なので、`true` のまま据え置くか `false` に変えるかを
   明示的に決めて記録する。

### D1b: `FeedbackPolicy::Blind` の `backoff` を実際に使う

D1・D1a を実装しても、「有界な `GiveUp`」にはならず「3秒おきの5連射が
永続する」形に留まる（round1 レビュー Finding 5）。

1. `FeedbackPolicy::Blind { max_attempts, backoff }` の `backoff`
   フィールドは**構築されるだけで一度も読まれていない**
   （`tuning.rs:283-288` が BUG-68 記録時点の「既知の限界」として
   自ら明記済み）。`ir_apply_drift_correction` は
   `FeedbackPolicy::Blind { .. }` とワイルドカードで分配束縛し
   （`ime_refresh.rs:620`）、`decide_actuation_action` も
   `max_attempts` しか見ない（`ime_actuation.rs:165`）。結果、
   `GiveUp` までの最大5回の送信は refresh tick 間隔（実測20〜90ms）で
   連続し、~100〜450ms の間に `VK_IME_OFF` × 5 のバーストになる。
2. `GiveUp` 到達後、`blind_rearm_cooldown_elapsed`
   （`DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS = 3_000`、
   `tuning.rs:289`）で3秒間は再武装しないが、3秒経過後
   `ReadBackQuery::AnyFreshEvidence` は「`gave_up_at` 以降に非失効の
   観測が1件でもあれば**値を問わず**」再武装を許す
   （`observation_store.rs:643-648`）。ところが `ImeReadStrategy::Blacklist`
   分岐自体が、active IME が GJI のとき毎 tick `write_observer_poll`
   で観測を record している（`ime_refresh.rs:166-184`）。つまり
   `AnyFreshEvidence` は事実上常時成立し、**3秒周期で永久に再武装する**。
   `tuning.rs:271-278` が想定していた「クールダウンが無効化される」
   ケース（フォーカス変更）とは別の、「観測が自動生成され続けるので
   毎回再武装する」ケースであり、これは記録されていない盲点。

**決定**: `backoff`（既存フィールド、既定 400ms、新しい定数値の追加や
値の変更ではなく**既存宣言済みの値を初めて消費する**）を
`ir_apply_drift_correction` の `Blind` 送信間隔に反映させ、バースト
内5回を無間隔ではなく `backoff` 間隔にする。

**実装上必須の変更（round2 で追加、D2 の記述と整合させる）**: `backoff`
を送信間隔として使うには「前回送信した時刻」が要るが、`Actuation`
（`runtime/ime_actuation.rs:25-44`）が持つのは `sent_at`（**この試行が
最初に** actuate した時刻、`Read` 分岐の収束フェンスとして
`read_back(now, act_sent_at, ...)` に使われている、`ime_refresh.rs:749`
付近）だけである。`sent_at` を送信のたびに更新すると、この収束フェンスが
前進してしまい `Read` 経路の意図しない挙動変更になる。したがって
`Actuation` に **新フィールド `last_sent_at: Option<Instant>`** を追加し、
`backoff` の間隔判定はこちらを見る。

**early-return の位置（round3 で明確化・実装すると状態を壊す旨を追記）**:
「backoff 待機でその tick の送信を見送る場合は `advance_epoch` を呼ばない」
だけでは不十分である。`ir_apply_drift_correction` の送信部分
（`ime_refresh.rs:768` 以降）は次の順で実行される:

```
768: log::warn!("[drift] correction: ... → set_ime_open({desired})")
776-786: journal.record(JournalEntry::ImeActuation { .. })  // action = Send
788-795: dispatch_event(ImeEvent::DriftDetected { desired, observed, duration_ms })
796-821: 実送信（ImmCross / Blacklist 分岐）
828-830: advance_epoch()
```

`advance_epoch` だけをスキップして768行以降をそのまま通すと、backoff
待機中の tick ごとに次の2つが起きる: (1) `ActuationRecord::new` が
`decide_actuation_action(policy, attempts)` から `action` を導出するため
（`ime_actuation.rs:340-348`）、**送っていない actuation が journal に
`action: Send` として記録される**（`drift_correction_replay` フィクスチャの
「`attempts` と `epoch` が歩調を合わせる」健全性も壊れる、BUG-33 型の
記録と実態の乖離）。(2) `ImeEvent::DriftDetected` の reducer
（`ime_model.rs:772-777`）が `self.applied =
AppliedImeState::Optimistic(desired)` を毎 tick 上書きする。これは
`force_on_attempt_allowed`（`ime_actuation.rs:251-258`）の分岐(1)が
`Optimistic(true)` を「既に ON を apply 済み」として force-ON を抑止する
条件に使われているため、**ON 方向の drift で backoff 待機している間、
`apply_force_on_for_imm_broken` が抑止され続ける**（BUG-16/BUG-69 の
抑止経路に触れる、無害ではない）。

**決定**: backoff の early-return は **768行より前**（`match act_policy`
の `Blind` アーム内、または `match` 直後）に置く。`journal.record`/
`dispatch_event(DriftDetected)`/実送信のいずれも、backoff 待機中の tick
では一切呼ばない。あわせて `ir_notify_drift_giveup_diagnostic`（617行、
duration ベースの通知のため待機中も duration は伸び続ける）をこの
early-return の前に置くか後に置くかも実装時に決める（トレイ通知の
タイミングが待機で早まる/遅れるという観測可能な差になる）。

**効果の見積もりの訂正**: バースト内の瞬間密度は激減する
（~100〜450ms/5発 → ~1.6〜2s/5発）が、`GiveUp` 到達自体が backoff 分
遅れるため `gave_up_at` も後ろにずれ、1周期（送信バースト＋3秒
クールダウン）は ~3.1s から ~4.6s 程度に伸びる。**総送信数の削減幅は
約30%に留まる**（「単位時間あたりの衝撃を大幅に下げる」が「密度」の
話であって「総量」の話ではないことを明記する）。総量をさらに削るには
D1b 単体では足りず、下記案B'（`Read.deadline` の消費）や
`AnyFreshEvidence` の除外（本 ADR ではスコープ外、上記記載）が必要に
なりうる。

`AnyFreshEvidence` から「読み戻し手段が構造的に無いと自ら宣言している
Blacklist 経路が生成した観測」を除外する案（`ReadBackQuery` に新
variant を追加、BUG-68 と同型の「新情報の代理指標として機能しない」
問題への対処）は、`ReadBackQuery`/`Resolution` の意味論を追加変更する
ため、本 ADR ではスコープに含めない。D1b 実装後の実機ソークで
3秒周期の再武装バーストが依然として実害を持つと確認された場合に、
別 ADR として起票する（**「今回はやらない」という判断を明示的に記録**
——round1 レビューが要求する最低限の透明性）。

### D1c（新規・主対策）: `app_policy` を起動時に正しく初期化する

D1/D1a/D1b は根本原因1（bootstrap 窓）そのものを塞がず、「窓の間に
誤った policy を掴んでしまう確率」を下げるだけである（上記「なぜ v1の
説明は誤りか」(a)(b) 参照）。根本原因1を直接塞ぐ、より筋の良い対策を
主対策として追加する。

`establish_initial_focus_scope`（`focus_tracking.rs:108`）が
`sync_initial_focus_fence` 経由で既に dispatch している
`ImeEvent::InitialFocusFenceEstablished` に起動時点の `profile` を
載せ、その reducer で `app_policy = AppImePolicy::from_profile(profile)`
を設定する。

利点（round1 Finding 6 末尾の提案を採用）:

- 根本原因1（bootstrap 窓）を確率的緩和ではなく直接解消する。
- `default_feedback` だけでなく `focus_settle_ms`/`owns_physical_kanji`
  という**同じ窓で誤ったままの他2フィールド**（下記 D3 参照）も同時に
  正しくなる。
- 何もライブ化しないため、案A（下記却下）が抱える懸念
  （ADR-089 §2.5 の K非依存性・スナップショット設計の変更）に
  一切触れない。

**round2 で削除した誤った主張**: v2 はここに4番目の利点として「起動時点
で既に正しい値が入っていれば、その後 `FocusChanged` が発火しても
スナップショットが誤った値で上書きされることはない」と書いていたが、
これは誤り。`state/ime_model.rs:615` の `FocusChanged` reducer は
`self.app_policy = AppImePolicy::from_profile(profile);` という無条件
代入であり、ガードも比較も無い。起動時に正しい値が入っていても、後続の
`FocusChanged` が誤った `profile` を運べば普通に上書きされる。D1c が
防ぐのは bootstrap 窓（起動〜最初のプロセス切替）だけであり、その後の
`FocusChanged` スナップショットの誤りに対する防御力はゼロである。

**実装位置（round2 で明確化）**: `establish_initial_focus_scope`
（`focus_tracking.rs:108-145`）内で、:122 の
`advance_focus_tracking(&classified, true)`（`current_app_profile()` を
確定させる）より**後**、:137 の `sync_initial_focus_fence` 呼び出しと
同じタイミング（またはその直後）に置く。:122 より前に置くと
`current_app_profile()` がまだ正しい値を返さない。

**残余ケース（round2 で明確化）**: `classify_focus_probe` が `None` を
返す早期 return（`focus_tracking.rs:112-114`、probe タイムアウトや pid
取得失敗）では `sync_initial_focus_fence` に到達しないため、D1c も走らず
bootstrap 窓がそのまま残る。この残余は D1（ライブ再導出）が拾う
——D1c と D1 を併用する理由がもう1つ増える。

注意点: `architecture_guard.rs::establish_initial_focus_scope_does_not_write_ime_belief`
は `dispatch_event(` の出現をテキスト検査するガードであり、
`establish_initial_focus_scope` 系の関数に新しい `dispatch_event` 呼び出し
（または既存呼び出しの意味変更）を追加する際はこのガードの対象範囲を
確認すること。`app_policy` は `desired_open`/`applied`/`observations`
のような「belief」ではなく「policy」だが、ガード名が意図する範囲との
整合は実装時に見直す（`notify_focus_hwnd_updated_if_needed` が同種の
理由で関数分離されている前例、`focus_tracking.rs:425-436` 参照）。

D1c と D1（D1a込み）は排他ではなく併用する。D1c が bootstrap 窓を
塞ぎ、D1 が D1c で拾いきれない根本原因2（`FocusChanged` を経由しない
ライブ分類変化）を拾う。

### D2: `actuation_for` の「reuse 時は policy を無視する」不変条件は変更しない

D1/D1a/D1c いずれも、この不変条件を利用する側の変更であり、
`actuation_for` の**「reuse 時 policy を無視する」という振る舞い**は
変更しない。**例外は D1b で、`Actuation` に `last_sent_at:
Option<Instant>` フィールドを追加する**（`sent_at` は `Read` の収束
フェンスとして既に使われているため流用できない、D1b 参照）。
フィールド追加に伴い `actuation_for` の構築式
（`runtime/ime_actuation.rs:73-80`）も変更対象になる（round3 で明確化:
「`actuation_for` は変更しない」と読めた記述を「reuse 時の振る舞いは
変更しない」に訂正——構築式へのフィールド追加自体は行う）。

### D3: `open_warrant.rs` は本 ADR のスコープ外だが、実害の種類を known-bugs.md に正確に記録する

`state/open_warrant.rs:201-204` の
`matches!(ctx.policy.default_feedback, FeedbackPolicy::Blind { .. })`
（Step 4c、`OwnSsot`）も同じ `AppImePolicy` スナップショットを参照する。

**round1 レビューで訂正**: v1 は「この消費者では実害が確認されていない」
と書いたが、これは「実害が無い」ことの根拠にならない——**この消費者の
実害は drift correction とは種類が異なるため、同じ手掛かり（無条件
再送ストーム）を探しても見つからないだけである。** `OwnSsot` の doc
（`open_warrant.rs:85-92`）が言う通り、これは「実 IME の open 状態を
直接観測する手段が構造的に無いプロファイルでの、`HeuristicGuess` すら
成立しない状況での最終的な force-ON 根拠」である。根本原因1・2の窓では
`default_feedback` が誤って `Read` に固定されるため、**Chrome/Windows
Terminal/WezTerm という「Step 4c がまさに守るべきアプリ」でちょうど
Step 4c が無効化され、`issue_open_warrant` が `None` を返して force-ON
が否認される。** 症状は BUG-16/BUG-02 系（cold-start のリテラル化、
IME が ON にならない、最初の数文字が欠落）であり、再送ストームの
**真逆**である。

D1c が実装されれば bootstrap 窓由来のこの実害も同時に解消されるが、
根本原因2（`FocusChanged` を経由しないライブ分類変化）由来の分は
D1c では解消されない。`open_warrant.rs` 側を D1 と同様にライブ化する
かは別途判断が必要なため、本 ADR のスコープには含めない。

**`docs/known-bugs.md` への追記は「実害が無い」ではなく「実害の種類が
違う（force-ON warrant の否認であって再送ストームではない）」と正確に
記録する。**

同じ窓で誤ったままの消費者は `default_feedback` だけではない:

| フィールド | bootstrap 窓での値 | 本来あるべき値（TsfNative/Imm32Unavailable 時） | 影響 |
| --- | --- | --- | --- |
| `default_feedback` | `Read` | `Blind` | BUG-114 本体 + `open_warrant` Step 4c 無効化 |
| `focus_settle_ms` | 100 | Imm32Unavailable=500 / TsfNative=200 | `settle_until` が短すぎ、`ime_apply_should_defer` の保護が早く切れる |
| `owns_physical_kanji` | `true` | WezTerm(TsfNative)=`false` | 静的軸。実効判定は `transport.rs::plan` が握るため実害は限定的だが値としては誤り |

D1c はこの3消費者すべてを同時に正しくする。

### D4（round2 で復活・必須）: 診断ログを `ir_apply_drift_correction` の消費点に追加する

**round2 レビューで発見された Blocker への対応**: v1 にあった D4
（診断ログ）が、v2 で「計装先を `FocusChanged` reducer から消費点へ
移せ」という round1 の指摘を受けた際、移設ではなく削除されてしまい、
「問題」節が参照する D4（根本原因1・2・3 のどれが支配的かを実機で
切り分ける手段）が決定節に存在しない dangling reference になっていた。

`runtime/ime_refresh.rs:605` の直後（`policy` 取得の直後、`actuation_for`
呼び出しの前）に、挙動を変えない診断ログを1行追加する（ADR-131 と同じ
「計装のみ」パターン）:

**実装は2段階になる（round4 で明確化・検証計画0を実行可能にするための
必須修正）**: 検証計画0（D4 のみを先行実装し、D1/D1a/D1b/D1c を一切
入れない状態でログを取る）を素直に書こうとすると、D4 のコード片が
D1 の導入する `live_profile` を前提にしており、**D1 抜きの段階では
コンパイルできない**。さらに意味の面でも、D1 抜きの段階では
`policy`（= `ime_refresh.rs:605` の現行値）はスナップショットそのもの
なので、`policy` と `snapshot_policy` が常に同一値になり、この ADR の
中核である「live と snapshot の食い違い」がログから消える。したがって
D4 は次の2段階の対応関係を明示して書く:

```rust
// D4 単独先行段階（検証計画0、D1 未適用）: live 側は D4 自身が算出する。
let live_profile: ImePolicyProfile = self.platform.current_app_profile().into();
let live_policy = AppImePolicy::from_profile(live_profile).default_feedback;
let snapshot_policy = self.platform_state.ime.default_feedback(); // = この段階の `policy`
if live_policy != snapshot_policy || /* actuation_for が新規構築した tick */ {
    log::debug!(
        "[bug114-diag] snapshot_policy={:?} live_policy={:?} class_name={:?} \
         current_focus={:?} detect_miss_count={:?} imm_capability={:?}",
        snapshot_policy, live_policy, self.platform.focus.class_name(),
        self.platform_state.ime.model().current_focus(),
        self.platform_state.ime.detect_miss_count(),
        self.platform.focus.imm_capability(&process_name, &class_name),
    );
}
// D1 適用後: `live_policy` はそのまま `policy`（line 605 の戻り値）になり、
// `snapshot_policy` だけが D4 専用の追加読み戻しとして残る。
```

`ime_refresh.rs` に `ImePolicyProfile`/`AppImePolicy` の `use` 追加が
必要になるのは D1 の段階ではなく**この D4 の段階**である点も実装時に
留意する。

**round3 で出力項目を訂正（round4 でさらに1項目追加）**: v3 は
`snapshot_profile_stale` をプレースホルダのまま挙げていたが、D1 が
`ime_refresh.rs:605` のスナップショット読み口自体を削除するため、この
関数内には比較対象が残らない。上記のとおり `default_feedback()` を
診断目的で明示的に読み戻す（`backoff`/`deadline` が「誰にも読まれない
まま放置された」のと同じ轍を踏まないよう、この行は D4 専用である
とコメントする）。

**根本原因2-2の判別には `detect_miss_count` だけでは不十分（round4 で
追加）**: `detect_miss_count` は、降格が成立した後は
`current_app_profile()` が `Imm32Unavailable` になり
`ir_decide_read_strategy` が `Blacklist` を返すため、それ以上更新
されない（`ir_poll_and_learn` は `OsPoll` アームでしか呼ばれない、
`ime_refresh.rs:209-212`）。加えて `FocusChanged` の reducer が
`record_success()` でリセットする。したがって drift tick の時点では
0 に戻っている可能性がある。より直接的な witness は学習済み
capability そのもの——`self.platform.focus.imm_capability(&process_name,
&class_name)`（`learn_imm_capability_from_miss` が使っているのと同じ
呼び出し）が `ImmCapability::Unavailable` を返せば根本原因2-2が確定
する。`detect_miss_count` は「学習途中かどうか」の判定には引き続き
有効なので両方出す（上記コード片に反映済み）。

さらに、`policy`/`live_profile`/`class_name`/`can_use_imm32` の4項目
だけでは根本原因1・2-1・3 を判別できない（根本原因1と2-1はどちらも
「live≠snapshot」としか出ず、根本原因3は「live==snapshot、policy=Read」
という点で Notepad 等の正当な ImmCross アプリと区別できない）。
以下2項目を追加する（いずれも既存配線の再利用で新規配線は不要）:

- `self.platform_state.ime.model().current_focus()`
  （`ime_model.rs:301`、`Option<HwndId>`）: `FocusChanged` の reducer
  でのみ書かれるため、`None` は「起動以来 `FocusChanged` が一度も
  発火していない」の決定的な witness——根本原因1の直接証拠になる。
- `self.platform_state.ime.detect_miss_count()`
  （`platform_state.rs:681`）: 根本原因3（IMM 学習が閾値未到達）かの
  切り分け。`ir_stage_observe` が既に同じ値を読んでいる
  （`ime_refresh.rs:210`）。

これにより実機ログ1本で、根本原因1（起動後 `FocusChanged` 未発火、
`current_focus==None`）・根本原因2-1（同一プロセス内クラス変化、
`current_focus.is_some()` かつ live≠snapshot）・根本原因2-2（IMM学習
降格、`detect_miss_count` が閾値超え）・根本原因3（学習閾値到達前、
live==snapshot==Read だが `detect_miss_count` が閾値未満）を一意に
判別できる。**D1c 実装前に1回、実装後に1回取得すれば、修正の効果の
帰属（検証計画0・2参照）も同時に取れる。**

**ログレベルと寿命（round3 で追加）**: drift tick 間隔（20〜90ms）で
`log::debug!` を無条件に出すと、BUG-114 発生中は毎秒10〜50行になり
BUG-111（「重い」系）調査の交絡要因になりうる。「`live≠snapshot` の
とき、または `actuation_for` が新規構築した tick のときだけ出す」形に
絞る。ADR-131 の計装が恒久なのに対し、これは調査用の計装であるため、
支配的な機構が確定した後は撤去するか出力頻度をさらに絞ることを
実装メモに残す。

## 却下した代替案

### 案 A: `AppImePolicy` 全体を毎 tick ライブ判定に変える（否定した理由を訂正）

v1 は「`focus_settle_ms` はフォーカスセッション開始時点で確定している
ことを前提にした値であり、ライブ化すると `settle_until` の算出や
`Blind { max_attempts }` の試行カウントと組み合わさったときの挙動が
現行と変わる」という機構を却下理由に挙げていたが、**round1 レビューで
この機構自体が成立しないと判明した**: `settle_until` は `FocusChanged`
の reducer で**1回だけ**計算されて `InputBarrier::FocusTransition` に
格納される（`ime_model.rs:650-656`）。`focus_settle_ms` をライブ化しても
既に計算済みの `settle_until` が遡って動くことはない（影響が出るのは
`platform_state.rs:410`/`schedule_settle_retry` の再計算箇所のみ）。
`Blind { max_attempts }` の試行カウントも `Actuation.attempts` が持ち
`AppImePolicy` の生存期間とは独立である。

**却下の結論自体は維持する**が、根拠を書き直す: 影響範囲が
`focus_settle_ms`/`owns_physical_kanji` を含む `AppImePolicy` 全体の
読み手すべてに及び、それらが本当に副作用なくライブ化に耐えるかの検証
コストが本 ADR のスコープ（drift correction の feedback 判定）を大きく
超える。D1c が bootstrap 窓由来の3消費者の誤りを個別に直すため、
「全体をライブ化する」動機自体が小さくなっている。

### 案 B: `FeedbackPolicy::Read` にも `Blind` と同様の有界打ち切りを追加する

却下する。理由: `Resolution::GaveUp` は現状 `Blind` 専用の帰結として
設計されており、`Read` に追加すると `ConvergedReceipt`/
`decide_actuation_action` の意味論変更が必要で影響範囲が広がる。

**round2 で訂正**: v2 は却下理由の(2)として「D1c/D1 実装後は `Read`
が誤って選ばれる窓が大幅に縮小される」を挙げていたが、これは
「なぜ v1 の説明は誤りか」(b) で自ら認定した事実（誤った `Read` を
一度掴むと被害継続時間は数十分〜数時間、修正前と同じ長さ）への回答に
なっていない——**発生確率を下げても、発生したときのコストが有界で
ないなら、安価な上限を置く価値は下がらない。** この論法は round1 で
崩れた理由と同型であり、単独では却下理由として使わない。却下の本体は
上記（`Resolution`/`ConvergedReceipt` の意味論変更コスト）のみとする。

### 案 B'（新規・round2 追加）: 未消費の `Read.deadline` を消費する

`FeedbackPolicy::Read { source, deadline }` の `deadline` フィールドも
`Blind::backoff` と同様に**本番で一度も消費されていない**（`grep` 済み:
本番参照ゼロ、`state/ime_actuation.rs:432`・`state/actuation_chain.rs:906/920`
のテストフィクスチャのみ。値は `DRIFT_CORRECTION_THRESHOLD_MS`=400ms が
既に格納されている）。「`deadline` を超えても収束観測が来ない `Read` は
`Blind` と同じく送信を止める（または backoff 間隔に落とす）」という形に
すれば、`Resolution` に新 variant を追加する必要も `ConvergedReceipt`
の意味論を変える必要もなく、案Bの却下理由がそのまま消える。

D1b が採った「既存宣言済みの値を初めて消費する」というパターンを
`Read` 側にも同じ形で適用できるにもかかわらず、**本 ADR では実装せず
記録のみに留める**——D1c/D1/D1a/D1b の組み合わせでどこまで実害が
縮小するか実機ソークで確認してから、必要なら別 ADR として起票する。
`backoff`（D1bで消費）と `deadline`（本ADRでは未消費のまま）という
同じ性質の2フィールドに対して非対称な判断をしていることを明示的に
記録する（round2 レビュー指摘への対応）。

## 検証計画

0. **（round3 で追加・最優先）D4 のみを先行実装し、D1/D1a/D1b/D1c を
   一切入れない状態で BUG-114 を再現させてログを取得し、支配的な機構
   （根本原因1/2-1/2-2/3 のどれか）を確定する。** D4 は挙動を変えない
   計装のみなので単独先行投入のリスクはゼロ。D1c/D1/D1a/D1b を全部
   入れてから検証すると、症状が消えても「どの機構だったのか」を
   永久に判別できなくなる（v1 が推測で組み立てて round1 で崩れたのと
   同じ轍を踏まないため、このログ取得を他の決定より先に行う）。
1. Windows Terminal + GJI で BUG-113/BUG-114 の再現手順を実行し、
   **14秒の観測窓における `VK_IME_OFF` の `SendInput` 送信回数**を
   定量的に計測する（成功条件は「`strategy=drift_correction_read` の
   ログが出ないこと」ではなく、この回数が有界に収まることに変更する
   ——round1 レビュー Finding 5 の指摘どおり、D1 系だけでは完全ゼロには
   ならない可能性があるため）。
2. awase 起動直後、他アプリへのフォーカス切り替えを一切行わずに
   Windows Terminal で BUG-113/114 の再現手順を実行し、D4 の診断ログで
   「最初の drift tick の時点で `policy=Blind` になっていること」を
   確認する（D1c により bootstrap 窓が解消されたことの直接検証。D4
   無しでは D1c と D1 のどちらが効いたか帰属できない）。
3. `app_overrides.input_relay_apps` に登録したアプリにフォーカスした
   状態で、drift correction ループが**有界であること**（`gave up`
   ログが出て無限再送にならないこと。D1a 適用後は `Blind` になるため
   ループ自体は発生する——5回送信して park、3秒後に再武装、を繰り返す
   のは想定どおり。ADR-119 の gate により実注入は起きないため、
   observable な指標は journal の `ImeActuation` 件数）を確認する
   （D1a の検証、InputRelay の不変条件テストが green であることも
   確認する）。
4. 通常の ImmCross アプリ（メモ帳等）で IME ON/OFF の収束確認
   （`Read` の本来の使われ方）に退行が無いことを確認する。
5. Chrome/Edge（Imm32Unavailable）・WezTerm（TsfNative）で同様に
   drift correction が正常動作すること（既存の `Blind` 挙動に変化が
   無いこと）を回帰確認する。
6. `EventOrigin.source` の strategy 文字列が `drift_correction_read`
   から `drift_correction_blind` へ変わりうる（round1 Finding 7）。
   `tests/journals/*.json`・`tests/drift_correction_replay.rs` 等の
   既存フィクスチャがこの文字列・policy 値に依存していないか確認し、
   必要なら期待値を更新する。
7. **D1 単独では BUG-113（「@」混入）自体が閉じない可能性がある**
   （D1b 実装前は「3秒おきの5連射」が残るため）。ADR-133 側の実機
   検証で、D1b 実装後にバースト規模がどこまで縮小されれば「@」が
   再現しなくなるかを確認する。

## 関連

[ADR-133](133-gji-ime-mode-key-sendinput-batch-shape.md)（BUG-113、
本 BUG の発見元となった調査）、BUG-110/[ADR-132](132-uncorroborated-physical-ime-key-engine-lockout.md)
（`IntentStore`/`last_intent` の絶対的権威化が真因の、同じ症状クラスの
別原因——独立した原因である点に注意）、
[ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md)。
