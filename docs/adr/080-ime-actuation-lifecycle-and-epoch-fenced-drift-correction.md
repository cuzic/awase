# ADR-080: IME actuation（VK送信/IMM32呼び出し）を型付きトランザクション化し、closed-loop/open-loop の区別と有限終端を構造で強制する

## ステータス

Phase 1 実装済み（2026-07-25、実機ソーク未実施）。当初 BUG-43 に対して先行適用した
暫定パッチ（`Runtime::last_drift_correction_send` による手作りクールダウン）は、
本 Phase 1 で `Actuation`/`FeedbackPolicy` 機構に置き換え、当該フィールドは撤去済み
（`docs/known-bugs.md` BUG-43 の「追記（恒久対応）」参照）。本 ADR は当初、その暫定
パッチが4件目の独立した手作りレート制限であるという事実を出発点に、同じ欠陥を持つ
経路が今後も追加され続けないようにするための構造的な後継案として提案されたもの。

**Windows 実機での実行検証は未実施**（このサンドボックスでは wine 未導入のため
Windows 実行不可）。クロスコンパイル（`x86_64-pc-windows-gnu`）＋ clippy（警告ゼロ）＋
Linux で実行可能なテスト部分集合のみ検証済み。`IME_ACTUATION_BLIND_MAX_ATTEMPTS = 5`
は実機ソークで裏付けの無い暫定初期値。以上は次回の Windows セッションで実機確認する
ものとしてフラグを立ててある。

### Phase 1 実装ファイル一覧（2026-07-25）

| 項目 | ファイル | 内容 |
|---|---|---|
| 純データ型 | `crates/awase-windows/src/state/ime_actuation.rs`（新設） | `FeedbackPolicy`（`Read`/`Blind`）、`Resolution`（`Confirmed`/`GaveUp`）、`ActuationAction`（`Send`/`GiveUp`）、純関数 `decide_actuation_action` ＋ 単体テスト |
| プロファイル方針 | `crates/awase-windows/src/state/app_ime_policy.rs` | `AppImePolicy.default_feedback: FeedbackPolicy` を追加、プロファイル別に設定。`IME_ACTUATION_BLIND_MAX_ATTEMPTS: u32 = 5`（実機未検証の暫定初期値と明記） |
| 鮮度フェンス API | `crates/awase-windows/src/state/observation_store.rs` | `ObservationStore::most_recent_trusted_after(now, since)` ＋ 単体テスト |
| アクセサ | `crates/awase-windows/src/state/platform_state.rs` | `ImeStateHub::default_feedback()`。**`check_drift_correction` 自体は未変更**（drift 検知は引き続き since フェンス無しの `most_recent_trusted`、意図的な非対称） |
| 進行中状態 | `crates/awase-windows/src/runtime/ime_actuation.rs`（新設） | `Actuation`（`target`/`policy`/`attempts`/`sent_at`/`gave_up_at`）、`Runtime::actuation_for`/`discard_actuation` |
| 所有フィールド | `crates/awase-windows/src/runtime/mod.rs` | `Runtime.active_actuation: Option<Actuation>` を追加。暫定パッチ `last_drift_correction_send` は撤去 |
| ゲーティング本体 | `crates/awase-windows/src/runtime/ime_refresh.rs` | `ir_apply_drift_correction` を `Actuation`/`FeedbackPolicy` 経由に書き換え（`Blind` は有界リトライ＋鮮度ベース resume、`Read` は観測確認で早期終了）。`ir_notify_focus_changed` は focus 変更時に `active_actuation` を破棄 |
| 不変条件ガード | `crates/awase-windows/tests/architecture_guard.rs` | 2 テスト追加（`apply_ime_open_with_belief_call_sites_are_accounted_for` = crate 全体の count guard・既知5箇所、`drift_correction_giveup_and_confirmed_do_not_write_observations` = 不変条件6 のガード）。計 13 テスト（従前 11） |

**Phase 1 のスコープ限界（意図的、Phase 2 に持ち越し）**: (1) `check_drift_correction`
は未変更で drift *検知* は unfenced のまま、*Read 収束確認* のみ新 API を使う（意図的な
非対称、「不変条件」節3参照）。(2) raw send 呼び出し（`set_ime_open`/
`apply_ime_open_with_belief`）は `ir_apply_drift_correction` 内にインラインで残し、
単一窓口への統合はしていない — 代わりに crate 全体の呼び出し箇所数を凍結する count
guard（既知5箇所）で担保。(3) `IME_ACTUATION_BLIND_MAX_ATTEMPTS = 5` は未検証の暫定値。

**Codex CLI によるレビュー（2026-07-25、read-only、リポジトリ探索込み）を実施し、
指摘のうち事実確認が取れたものを本文に反映済み**（`AssumeAfter` の収束偽装リスク、
actuation 試行状態の永続化先の欠落、epoch fencing の対象 API 不一致、Phase 1 での
単一窓口強制の実現不可能性、`ImeActuatorKind::Standard` の欠落等）。指摘と反映内容の
詳細は各節末尾に記す。

## コンテキスト

### 症状: 同じ根から生えた正反対の2つのバグ

`state/platform_state.rs::check_drift_correction` は `desired_open`（設定値）と
`observations.most_recent_trusted()`（観測値）を比較し、乖離があれば
`runtime/ime_refresh.rs::ir_apply_drift_correction` が実際に IME 状態を actuate
（IMM32 呼び出し or VK SendInput）する、という汎用の「継続的照合ループ」を
持っている。このループが、観測能力が根本的に異なる3つの `AppImeProfile`
（ImmCross / Imm32Unavailable / TsfNative、いずれも `AppImePolicy::actuator_kind`
で区別）に一律に適用されていることから、正反対の2つのバグが別々の時期に生じた。

- **BUG-33**（`docs/known-bugs.md`）: Imm32Unavailable アプリで、実 IMM 読み取りが
  できないことを補うために「actuate した belief をそのまま低信頼度の観測として
  ストアに書き戻す」処理があった。これは定義上 `observed == desired` を作り出すため、
  `check_drift_correction` は**乖離を一度も検知できず、補正が一度も発火しない**。
- **BUG-43**（本会話で発見・暫定修正済み）: TsfNative/Blacklist アプリで、
  actuation（`apply_ime_open_with_belief` 経由の VK_IME_OFF 送信）の結果が
  `observations` ストアへ一切フィードバックされない（completion イベントが
  `generation: None` のため dispatch されない設計になっていた）。加えてこの
  ウィンドウクラスは実 IMM クエリ自体を構造的にスキップする。結果、乖離は
  正しく検知され続けるが**一度も収束せず、observe tick（~20ms）ごとに同じ VK を
  無限に再送**した（実機ログで 675ms 間に 16 回連続送信を確認）。

BUG-33 は「収束を偽装した」、BUG-43 は「収束を一度も記録できなかった」という、
同じ欠落を反対側から踏んだものである。

**本 ADR のスコープに関する注記（Codex レビュー指摘・反映、重要度: 中）**:
BUG-33 の実際の修正（`docs/known-bugs.md` BUG-33、2026-07-22 実装済み）は
drift correction 側ではなく、per-VK confirm の give-up 分岐から
`send_chrome_gji_reinit_and_poll` を直接呼ぶという、本 ADR とは別系統の
小さな修復パスだった。本 ADR が導入する `Actuation`/`FeedbackPolicy` は
BUG-43 のクラス（actuation の feedback 欠如によるタイトループ）は構造的に
再発不能にするが、BUG-33 のクラス（belief を自己参照的に低信頼度観測として
書き戻すことで乖離そのものが生成されなくなる問題）は**観測の書き込み側**の
欠陥であり、actuation 側の型変更だけでは塞がれない。BUG-33 型の再発を
構造的にも防ぐには、`.claude/rules/ime-belief-architecture.md` の禁止パターン2
（観測の偽装）を実際に検出する仕組み（例: belief 由来の値をそのまま観測として
書き込む construction-site を dylint で検出する）が別途必要であり、これは
本 ADR のスコープ外として切り出す（将来の別 ADR/dylint 拡張候補）。

### 手作りレート制限の重複発生（構造的欠陥の物的証拠）

調査の過程で、この「actuation ループが自分自身を止められない」問題に対する
場当たり的な対策が、少なくとも4箇所で**互いに無関係に**独立実装されていることが
判明した:

**actuation（実際に外部へ書き込む操作）の終端を止める目的の guard:**

| 箇所 | 対象 |
|---|---|
| `Output::last_gji_reinit_ms`（BUG-33 の別経路の修正、`output/probe_io.rs`） | Chrome GJI reinit の give-up 連続発火防止 |
| `Runtime::last_drift_correction_send`（本会話で追加、BUG-43 暫定修正） | drift correction の再送抑制 |

**観測側のちらつき・過敏反応を抑える目的の guard（本 ADR の対象外、別カテゴリ）:**

| 箇所 | 対象 |
|---|---|
| `PlatformState::focus_debounce_ms` | フォーカス変更直後の反応抑制 |
| `ImeKindDebounce`（`tsf/gji_monitor.rs`） | IME 種別判定のちらつき抑制 |

（Codex レビュー指摘: 当初は4件を同列に並べていたが、`focus_debounce_ms`/
`ImeKindDebounce` は actuation の終端欠如とは別種の guard であり、根拠として
一緒くたに並べるのは筋が弱いという指摘を受け、2カテゴリに分離した。本 ADR が
直接解消を狙うのは前者のみ。）

`docs/adr/index.md` の「長期的な教訓」節は既に *"Sideband boolean guard は
edge case のたびに増える"* と *"scattered boolean フラグは FSM に吸収できる"*
と記録している（ADR-046 の GjiFsm 化を例に）。前者2件のタイムスタンプ変数群は
同じパターンの再演であり、対象を FSM ではなく「actuation の終端保証」に
置き換えれば同じ教訓が当てはまる。

### 独立レビュー（fable との壁打ち、2026-07-25）

同じ問題を Claude Fable 5 に独立に検討させたところ、私（Sonnet）の
「closed-loop 用の機構を open-loop 対象に適用しているミスマッチ」という
フレーミングとは異なる、直交する切り口が得られた: **actuation が終端契約
（bounded attempts で必ず何らかの outcome に到達する、という liveness 保証）を
持たない一級市民になっていない**、というもの。`.claude/rules/ime-belief-architecture.md`
が強制しているのは「状態が嘘をつかない」という*安全性*（safety）であり、
「補正は必ず有限回で収束する」という*活性*（liveness）は一切強制されていない。
BUG-33/BUG-43 は同一の欠落を安全性/活性それぞれの側から踏んだ、というのが
fable の指摘であり、私の見立てと矛盾せず補完し合う。

### 先例: conv-mode 軸では既に同種の対策が提案・一部実装済み（ADR-078）

`docs/adr/078-ime-mode-belief-desired-effective-constraint.md` は、conv-mode
（かな/カタカナ/英数等）の belief について「同一の物理キー押下から発生する
補正書き込みは最大1回」というサーキットブレーカー
（`Controlled → Uncertain → ObserverOnly` の段階的降格）を提案している
（**Codex レビュー指摘・反映**: ADR-078 のステータスは「一部実装済み
（Phase 1a のみ）、全体の型設計は未実装のまま提案中」であり、「既に採用済み」
と言い切るのは過大な表現だったため訂正した）。本 ADR が扱う ON/OFF
（`desired_open`）軸の actuation は、ADR-078 が conv-mode 軸で提案した
「盲目的な再試行をしない」という設計判断を、まだ持っていない。本 ADR は
この非対称性を解消し、両軸で使える共通の actuation 抽象を導入する。

## 決定

### 全体方針: Actuation を型付きトランザクション化し、「読み戻せるか」を型ではなくデータとして表現する

3つのプロファイルを別々のコントローラ実装に分離する案（後述「検討したが採用
しなかった案」B）は最も設計として正直だが、実装・検証コストが大きい。
代わりに、**単一の actuation 抽象に、feedback（収束確認）方針をプロファイルごとの
データとして渡す**方式を採用する。これにより ADR-078 と同種の「compiler が
強制する discipline」を、既存の `ime-belief-architecture.md` の3段防御
（コンパイラ／dylint／architecture_guard.rs）とほぼ同じ枠組みで、制御層にまで
拡張できる。

```rust
/// プロファイルごとの feedback 方針テンプレート。`Copy` な純データで、
/// `AppImePolicy`（既存、`state/app_ime_policy.rs`）に `default_feedback` として追加する。
/// 実行中の試行状態（attempts 等）は一切持たない — Codex レビュー指摘: 当初案は
/// この区別が無く `Actuation` 自体に policy と実行時状態を同居させていたため、
/// `Copy` な `AppImePolicy` の責務が崩れる懸念があった。
#[derive(Clone, Copy)]
enum FeedbackPolicy {
    /// 実読み戻しが可能（ImmCross / Standard）。既存の drift correction 相当を維持。
    Read { source: ObservationSource, deadline: Duration },
    /// 読み戻し手段が構造的に存在しない（Imm32Unavailable / TsfNative、いずれも
    /// `docs/known-bugs.md` BUG-33 が確認した通り open/close の実観測を持たない）。
    /// 有限回で必ず打ち切り、以降は `desired` が変化するまで再送しない。
    Blind { max_attempts: u32, backoff: Duration },
}

/// 進行中の actuation 試行そのもの（`Copy` ではない、生存期間を持つ状態）。
/// 所有者は `Runtime`（新設 `runtime/ime_actuation.rs` 相当のモジュールが
/// 提供する非公開フィールド）。`desired_open` が変化したとき、`FocusChanged` の
/// とき、`Resolution` が確定したときにのみ破棄・再構築する（詳細は後述
/// 「状態の永続化先」節）。
struct Actuation {
    target: bool,
    policy: FeedbackPolicy,
    attempts: u32,
    /// この試行が最初に actuate した時刻。drift 判定が参照してよい観測の
    /// 下限（タイムスタンプ・フェンシング、後述）としても使う。
    sent_at: Instant,
}

/// `Actuation` の帰結。`Confirmed` 以外（`GaveUp`）は「収束したかどうか分からない
/// まま打ち切った」ことを意味し、**`observations` ストアには一切書き込まない**
/// （後述の不変条件参照。書き込むと BUG-33 と同型の収束偽装になる）。
enum Resolution {
    Confirmed, // Read: 実観測が target と一致した
    GaveUp,    // Blind: max_attempts 到達、以降 desired 変化まで再試行しない
}
```

`ir_apply_drift_correction` は `AppImePolicy::default_feedback` を使って
`Actuation` を構築する。`ImeActuatorKind::Standard`/`ImmCross`（=「安全デフォルト」
と既存コメントが呼ぶ、実 IMM32 読み取りが効くプロファイル）は `Read` を、
`Imm32Unavailable`/`TsfNative` は `Blind` を返す。

**Codex レビュー指摘・反映（重要度: 高）**: 当初案は `Feedback::AssumeAfter(Duration)`
という第三の分岐（「読み戻せないが一定時間後は効いたはずとみなす」、
`Imm32Unavailable` に割り当てる想定）を持っていた。しかし `docs/known-bugs.md`
BUG-33 の実態は「GJI I/O からの間接観測（`GjiIoInference`）は input_mode 判定
専用で、open/close の観測ストアには一度も書き込まれない」（`observation_store.rs`
の `PerSourceObservations::set`/`get` が `ConvBitsInference`/`GjiIoInference` を
明示的に無視する設計）——つまり `Imm32Unavailable` にも `TsfNative` と同様、
open/close の実観測経路は現状存在しない。`AssumeAfter` を独立の分岐として残すと、
「一定時間後は確定とみなす」という判定がどこかで `observations` への書き込みに
横流しされた瞬間に BUG-33 と同型の収束偽装が再発しかねない。したがって
`AssumeAfter` は分岐として廃止し、`Imm32Unavailable` も `Blind` に統合した
（実際に信頼できる open/close 観測源が将来 `Imm32Unavailable` 向けに見つかった
場合のみ、その時点で `Read` へ個別に格上げする）。

`FeedbackPolicy::Blind` は `max_attempts` に達したら `Resolution::GaveUp` を返して
終了し、`desired` が変化するまで（＝新しい `Actuation` が構築されるまで）
二度と actuate しない。これが BUG-43 が起こしたタイトループを**型レベルで
不可能にする**（`last_drift_correction_send` のような手作りタイムスタンプに
頼らない）。

### 状態の永続化先（Codex レビュー指摘・反映、重要度: 高）

当初案には `Actuation`（`attempts` を含む実行時状態）を**誰が・どこで・いつまで
保持するか**の記述が欠けていた。`check_drift_correction`（既存、
`state/platform_state.rs:421-463`）は `now` と `observations` だけから毎回
`Some`/`None` を計算する**純粋関数**で、過去に `GaveUp` した試行の記憶を一切
持たない。この構造をそのまま流用すると、observe tick（~20ms）ごとに
`ir_apply_drift_correction` が**新しい** `Actuation` を毎回構築してしまい、
`max_attempts` が実質的に tick のたびにリセットされる — BUG-43 とまったく同じ
タイトループが `Actuation` という型を被っただけで再発する。

したがって明示的に定める: `Actuation` の所有者は `Runtime`
（`runtime/ime_actuation.rs` 相当の新設モジュールが提供する非公開フィールド、
現行の暫定パッチ `last_drift_correction_send` と同じ置き場所）とし、
`ir_apply_drift_correction` は**既存の `Actuation` があればそれを使い、
無い場合にのみ新規構築する**。破棄条件は次のいずれか:

1. `desired_open` が前回の `Actuation.target` と異なる値に変わった。
2. `FocusChanged`（新しいアプリ/ウィンドウへの遷移、`observations.clear_on_focus_change`
   と同じタイミング）。
3. `Resolution::Confirmed` または `Resolution::GaveUp` が確定した。

### 有限 `Blind` からの復旧条件（Codex レビュー指摘・反映、重要度: 中）

当初案は「`desired` が変わるまで二度と送らない」とだけ書いており、外部要因
（一時的な送信失敗、ユーザーが言語バーで直接操作した、等）からの回復手段が
無かった。現行の暫定パッチ（400ms 間隔で再試行し続ける）より活性が弱くなり得る。
`GaveUp` からの再開条件を明示する: 上記「状態の永続化先」の破棄条件（1〜3）に
加えて、4番目の条件を追加する。

**実装時に判明した訂正（重要度: 高、Phase 1 実装セッションで発覚）**: 当初この節は
「新しい `trusted` 観測が `Actuation.target` と異なる値を報告した場合」を復旧条件と
書いていたが、これは IME open/close が単純な `bool` であることを見落とした誤りだった。
drift correction は `observed != desired`（乖離）が続く間しか走らないため、bool の
「間違った値」は `!desired` の1通りしか存在しない。したがって「target と異なる値の
観測が来たら復旧」はほぼ毎 tick 真になり、`GiveUp` を次の tick で即座に無効化して
しまう（それは乖離の定義そのものだから）。

正しい復旧シグナルは**観測の「値」ではなく「鮮度」**である: `GiveUp` に到達した
時刻を `Actuation.gave_up_at: Option<Instant>` として記録し、以降の tick で
`ObservationStore::most_recent_trusted_after(now, gave_up_at)` が `Some(_)` を返せば
（**値は問わない**、`gave_up_at` より後に何らかの trusted 観測が record された
という事実だけを見る）「外部で何かが動いた証拠」とみなし、`Actuation` を破棄して
次の observe tick で新規構築（`attempts` リセット）させる。「一度諦めたら永久に
補正しない」という過剰な硬直化を避けるという目的自体は当初案のままだが、達成手段
（値の比較ではなく鮮度の比較）を訂正した。

### Actuation 送信時刻によるタイムスタンプ・フェンシング（当初「Epoch fencing」から改称）

**Codex レビュー指摘・反映（重要度: 高）**: 当初案は本節を「Epoch fencing」と呼び、
「ADR-077 の `FocusEpoch` admission パターンをそのまま再利用する」と書いていたが、
これは事実誤認だった。
`ObservationStore::derive_open()`（`observation_store.rs:236-255`）の epoch
フィルタは `ImmCrossProbe`/`FocusProbe` の2ソースにしか適用されておらず、
`check_drift_correction` が実際に呼ぶ `most_recent_trusted()`
（`observation_store.rs:214-219`）は confidence と `is_expired` しか見ない
**epoch もタイムスタンプ下限も一切考慮しない**関数である。BUG-43 の直接トリガーは
`ConvOpenInference`（epoch フィルタ対象外のソース）が stale なまま居座ることだった
ため、「既存の epoch admission を再利用する」という記述では実際には何も塞げない。

正しくは新しい API を追加する必要がある: `ObservationStore` に
`most_recent_trusted_after(&self, now: Instant, since: Instant) -> Option<&ImeObservation>`
を新設し、`o.at >= since` も追加条件に含める（全ソース対象、`ConvOpenInference`
を除外しない）。`ir_apply_drift_correction` は `Actuation.sent_at` を `since` として
渡す。これにより BUG-43 の直接トリガーだった「補正前の stale な
`ConvOpenInference` 観測が居座り続ける」問題を、既存 API の意味を変えずに
（`derive_open()`/`most_recent_trusted()` はそのまま残し、新関数を追加する形で）塞ぐ。

### 既存の raw actuation 経路をこの窓口に統合する（段階的、ADR-040 準拠）

**Codex レビュー指摘・反映（重要度: 高）**: 当初案は不変条件1（後述）を
「生の SendInput/ImmSetOpenStatus 呼び出しは窓口の外で構築禁止」と無条件に
書いていたが、これは Phase 1 の時点では実現不可能だった。実際に raw actuation
を直接呼ぶ経路は本 ADR の直接対象（drift correction）以外に少なくとも2つ既存する:

- `apply_force_on_for_imm_broken`/`try_force_on_bootstrap`
  （`runtime/mod.rs`）— `apply_ime_open_with_belief(true, None, belief)` を直接呼ぶ。
- `send_chrome_gji_reinit_and_poll`（`output/probe_io.rs`）—
  VK_IME_OFF/VK_IME_ON の SendInput を直接呼ぶ。

**追記（2026-07-25、Phase 1 タスク分解時の追加調査で判明。当初の一覧は過小だった）**:
`platform.rs` には `apply_ime_open_with_belief`/`apply_ime_open_with_applied` の
さらに下に `apply_ime_open_with_view`（`platform.rs:963`）という第三の姉妹関数が
あり、これが実際の最下層エントリポイントである。`runtime/executor.rs`
（`DecisionExecutor`、NICOLA チョード判定に基づく**通常のユーザー起点 IME
トグル**が通る本流パス）はこの `apply_ime_open_with_view` を直接呼ぶ。加えて
`runtime/key_pipeline.rs:641`（ObservedEisu 検出時の DirectInput 補正）、
`runtime/ime_refresh.rs:479`（`apply_ime_open_with_applied` 経由の force-on 経路）
も `apply_ime_open_with_belief` 系列を呼ぶ独立した呼び出し元である。

つまり raw actuation の呼び出し元は「drift correction ＋ 2経路」という当初の
想定より広く、かつ `apply_ime_open_with_view` は**バグの多い recovery 系経路
ではなく、正常に機能している通常の user-intent 駆動パス**である。

**再訂正（2026-07-25、Phase 1 実装セッションで判明。上記の直後に書いた「スコープを
`ir_apply_drift_correction` 自身の直接呼び出し禁止に絞る」という記述も誤りだった）**:
`ir_apply_drift_correction` を実装した結果（task #14）、raw send 呼び出し
（`set_ime_open`/`apply_ime_open_with_belief`）は**同関数内にインラインで意図的に
残す**設計になった。Phase 1 が変えたのは「送るか否か・どのくらいの頻度で送るか」
という判断であり、「raw send をどこに書くか」という物理的な配置ではない。
したがって「`ir_apply_drift_correction` が raw actuation 関数を直接呼ばなくなった
こと」を検証するテストは、正しい実装に対して即座に失敗する誤ったテストになる。

Phase 1 の construction-site テスト（task #17）が実際に実装したのは、**単一窓口への
統合でも「drift correction だけ呼ばない」ことの検証でもなく、`apply_ime_open_with_belief`
系列の呼び出し箇所**数**を crate 全体で凍結する count guard**である
（`tests/architecture_guard.rs::apply_ime_open_with_belief_call_sites_are_accounted_for`、
2026-07-25 時点で 5 箇所: `platform.rs::apply_ime_open_with_applied`、
`runtime/mod.rs` の force-on 経路2箇所、`runtime/key_pipeline.rs` の ObservedEisu
補正、`runtime/ime_refresh.rs::ir_apply_drift_correction`）。この5箇所は Phase 1
時点でレビュー済みの既知の呼び出し元として許容し、将来これが未レビューのまま
増えたら気づけるようにする、という「唯一の窓口」より緩いが実装可能な保証である。
全呼び出し元（`apply_ime_open_with_view` を含む）の網羅的な棚卸しと単一窓口への
統合は Phase 2 の計画時にあらためて行う（Phase 2 着手前に `apply_ime_open_with_view`
の呼び出し元一覧も再調査すること）。

Phase 割り当ては次の通り: drift correction（`ir_apply_drift_correction`）が
Phase 1（本 ADR の直接動機）、GJI reinit give-up
（`probe_io.rs::send_chrome_gji_reinit_and_poll`、`last_gji_reinit_ms`）と
broken-app-bootstrap force-on（`apply_force_on_for_imm_broken`/
`try_force_on_bootstrap`）が Phase 2。

`ImeKindDebounce`/`focus_debounce_ms` は actuation ではなく観測側のデバウンスの
ため対象外（別の抽象化課題として切り離す）。

`docs/adr/040-incremental-refactor-strategy.md` の段階的遷移パターン（debug_assert
による並行稼働 → 切り替え）に倣い、Phase 1 は drift correction のみを新窓口に
移行し、他の2経路は既存のまま残す。Phase 1 で discipline が実際に有効と
確認できてから Phase 2 に進む。

### 「B: プロファイル別コントローラの完全分離」「D: 継続ループの全廃」は今回は採用しない

## 検討したが採用しなかった案

- **B: `ClosedLoop`/`FencedOpenLoop`/`EventDriven` の3コントローラを別実装として
  完全分離する。** 最も設計として正直で、fable が「意図的に極限まで推し進めた形」
  と評した案。しかし `check_drift_correction` を3通りに分岐・重複実装する
  コストと、プロファイルごとの挙動が乖離するリグレッションリスクが大きい。
  上記の「`FeedbackPolicy` をデータとして渡す」方式で設計意図の大半は達成できる
  ため、今回は採用しない。将来 Phase 1/2 の運用で `Read`/`Blind` の2分岐だけでは
  表現しきれない挙動差が見つかった場合の エスカレーション先として記録しておく。
- **D: 継続的照合ループ自体を全廃し、全プロファイルをイベント駆動 actuation
  にする。** 現在の 20ms drift correction ループが実際に何を救っているか
  （BUG-19/BUG-20 で確立された ImmCross 側の実挙動を含む）を棚卸ししないまま
  廃止すると退行リスクが読めない。Phase 1/2 で `FeedbackPolicy::Read` 側の
  実利用状況を観察したうえで、ImmCross についてもループ頻度を落とせるか
  改めて判断する（本 ADR のスコープ外、フォローアップ）。
- **BUG-33 と同型の「actuate した belief を観測として書き戻す」方式の踏襲。**
  BUG-33 の原因そのものであり、`.claude/rules/ime-belief-architecture.md`
  禁止パターン2（観測の偽装）に該当するため最初から不採用。
- **`AssumeAfter`（「読み戻せないが一定時間後は効いたはずとみなす」第三分岐）。**
  Codex レビュー指摘を受け撤回（詳細は「決定」節の `FeedbackPolicy` 定義箇所）。
  `Imm32Unavailable` に信頼できる open/close 観測源が無い現状では `Blind` と
  実質的な違いを安全に作れず、書き込み先を誤ると BUG-33 型の偽装に転びかねない。

## 不変条件（実装時にテスト/型で強制する）

1. **（2026-07-25 実装セッションで再訂正）** `apply_ime_open_with_belief` 系列の
   呼び出し箇所は crate 全体で construction-site **数**を凍結する（count guard、
   `tests/architecture_guard.rs::apply_ime_open_with_belief_call_sites_are_accounted_for`）。
   Phase 1 時点の既知5箇所（`ir_apply_drift_correction` を含む、内訳は「決定」節
   「既存の raw actuation 経路をこの窓口に統合する」参照）は許容済みとしてカウントに
   含める。**「`Actuation` 窓口モジュールの外で raw 呼び出しを構築禁止」という
   単一窓口強制は Phase 1 のスコープではない**（`ir_apply_drift_correction` 自身が
   raw send をインラインで呼ぶ設計だから — 当初この不変条件はそれを禁止するかの
   ように書かれていたが誤りだった）。新しい呼び出し元が追加されカウントが変化したら
   test が fail し、`Actuation` ベースのゲーティングが必要か検討を促す、という
   「気づける」レベルの保証に留める。単一窓口への統合は Phase 2。
2. `FeedbackPolicy::Blind { max_attempts, .. }` は `max_attempts` 到達で必ず
   `Resolution::GaveUp` に遷移する。以降は `Actuation.target` が変化するか、
   **`Actuation.gave_up_at` より後に何らかの trusted 観測が record される**
   （**値は問わない** — 観測の「鮮度」のみを見る。IME open/close は bool のため
   「target と異なる値」はほぼ常に真になり得ず意味を成さない、詳細は「有限 `Blind`
   からの復旧条件」節の訂正参照）まで、新規 `Actuation` を構築しない。
3. `Read` policy の収束確認は `ObservationStore::most_recent_trusted_after(now, since)`
   （新設 API）が返すもの、すなわち対象 `Actuation.sent_at` 以降に record された
   ものに限る（`sent_at` より前の観測は棄却。ソース種別を問わず全ソースが対象）。
   **この since フェンシングは `Read` の収束確認にのみ適用される**——乖離の
   「検知」自体を行う `check_drift_correction`（`state/platform_state.rs`、Phase 1
   では変更しない）は引き続き since フェンシングなしの `most_recent_trusted` を
   使う。この非対称は意図的（「決定」節「Actuation 送信時刻によるタイムスタンプ・
   フェンシング」参照）。
4. `Actuation` は observe tick ごとに使い回す（「状態の永続化先」節の破棄条件
   1〜3 に該当するときのみ再構築する）。observe tick のたびに無条件で新規
   `Actuation` を構築する実装は、`max_attempts` を実質的に無効化するため禁止する。
5. 新しい actuation 経路を追加する際、`FeedbackPolicy` の指定を省略できない
   （コンパイルエラーになる型設計にする — オプショナルなデフォルト値で
   なし崩しに `Read` 相当が選ばれる余地を残さない）。
6. `Resolution::GaveUp`（および `Read` の deadline 超過）は、いかなる場合も
   `observations` ストアへの書き込みを発生させない（`ObserverReported` 等の
   イベントを dispatch しない）。これに違反すると BUG-33 と同型の収束偽装になる。

## 関連

- `docs/known-bugs.md` BUG-43（本 ADR の直接の動機、暫定パッチの詳細）、
  BUG-33（逆方向の症状）、BUG-20（actuation 送信側の対称バグ）。
- `docs/adr/077-observation-admission-epoch.md`（`FocusEpoch` による admission
  パターン。本 ADR のタイムスタンプ・フェンシングは `FocusEpoch` 型そのものは
  再利用しないが、「送信より前の証拠を無効化する」という考え方はこの ADR の
  一般化。Codex レビューで「型を再利用する」という当初の記述が事実誤認と
  判明したため訂正済み — 詳細は「決定」節参照）。
- `docs/adr/078-ime-mode-belief-desired-effective-constraint.md`
  （conv-mode 軸で提案・一部実装済みの「補正は最大1回」サーキットブレーカーの
  先例。本 ADR はこれを ON/OFF 軸 + actuation 層一般に拡張する）。
- `docs/adr/079-epoch-fenced-literal-recovery-with-replay.md`
  （`cold=N` を epoch として使う fencing を per-VK confirm に適用した近縁の設計。
  本 ADR のタイムスタンプ・フェンシングも思想的にはこのパターンの仲間）。
- `docs/adr/040-incremental-refactor-strategy.md`（Phase 1/2 の段階移行方針）。
- `docs/adr/046-gji-fsm-warm-cold-ssot.md`（"scattered boolean フラグは FSM に
  吸収できる" という先例、本 ADR は同じ教訓を actuation 層に適用）。
- `.claude/rules/ime-belief-architecture.md`（状態安全性の3段防御。本 ADR は
  同じ強制手法を「activation の liveness」に拡張する）。
- `.claude/rules/tuning-constants.md`（新規タイミング定数の実測義務。本 ADR は
  既存の `DRIFT_CORRECTION_THRESHOLD_MS` 等を再利用し、新規定数追加を最小化する
  方針を維持する）。
