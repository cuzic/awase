# ADR-106: fence 識別子の所有権是正と観測プロブナンスの型強制

## ステータス

**提案（未実装、2026-08-26）。** [ADR-104](104-observation-freshness-and-hardening.md) に対し
Opus 2体による独立レビュー（ラウンド1）→相互攻撃（ラウンド2）の敵対的レビューを実施した結果、
ADR-104 の決定6-a・6-c・7 は「9件が2つの失敗形に収束する」という自己申告に反し、
実際には**根本原因に到達していない症状パッチ**と判定された。本ADRはその根本原因に対応する
代替設計を定め、ADR-104 の該当決定を置き換える。

ADR-104 の他決定の扱い:

- **決定6-b**（drift confidence 3値化）: 本ADRの対象外。fence とは別軸の問題であり、
  「効く窓の大きさ」が一度も見積もられていないため、まず計装で基準線を作ってから
  ADR-104 側で単独判断すること（後述「却下した代替案」参照）。
- **決定8（Win32戻り値の型化）・決定9（`unreachable!()` の型による排除）・決定10（死んだ
  リセット削除）・決定11-a（`ProfilePolicy` 削除）・決定11-c（`vk.rs` 集約）**: 本ADRの対象外。
  2ラウンドのレビューで「`docs/known-bugs.md` に対応する実機不具合報告が無い」「fence/観測の
  根本原因と無関係」と判定された。ADR化せず、通常の掃除 PR として扱ってよい。
- **決定11-b**（`force_vk` が UWP InputSite フォールバックの対象外という設定非対称）: 本ADRの
  対象外だが、2ラウンドのレビューで**唯一「再現可能な現行バグ」**（ユーザーが `force_vk` に
  実クラス名を設定しても UWP アプリで効かない）と判定されたため、掃除 PR に埋めず**独立した
  fix として BUG 起票の上で着手すること**を推奨する。

## コンテキスト

### レビュー経緯

ADR-104 に対し、Opus 2体（以下 r1a・r1b）が独立に実コード（`state/observation_store.rs`、
`runtime/key_pipeline.rs`、`runtime/message_handlers.rs`、`state/platform_state.rs`、
`state/ime_event_log.rs`、`runtime/focus_tracking.rs`、`state/probe_admission.rs`、
`state/belief.rs`、`state/ime_model.rs` 等）と `docs/known-bugs.md` を突き合わせるラウンド1
レビューを行った。r1a は「ADR-104 は本当の単一の根本原因を捉え損ねている」という仮説で、
r1b は「ADR-104 は実害ゼロの理論的ハードニングが大半で過剰設計」という仮説で、それぞれ
独立に検証した。

ラウンド2では互いの主張を突き合わせ、r1a が r1b の最強の主張（決定6-cの前提「generation=0が
正当値と衝突する」は事実誤認で実質到達不能）を実コードで再検証した結果、**r1bのその主張こそ
誤りであり、ADR-104の記述の方が正しかった**ことが判明した。ただしその過程で、両者が見落として
いた**より深い欠陥**が見つかった（後述「原因A」）。

### 原因A: fence 用の識別子が、別目的の数を「借用」している

`generation` は `state/platform_state.rs:687` の `allocate_event_generation()` が
`state/ime_event_log.rs` の `next_seq()`（`ImeEventLog.next_seq`、診断用リングバッファの
通し番号、`0` 始まり）をそのまま返しているだけで、**`&self` であり増分しない**。一意性は
呼び出し元が直後に必ず `dispatch_event` して `next_seq` を実際に進める、という**型で守られ
ない契約**にのみ依存している（この依存は `runtime/mod.rs:860-868` のコメントが「generation を
割り当てるだけで `ImeApplyRequested` を dispatch しないと `record_ime_apply_result` の
generation 照合が常に不一致になる」と round-2 premortem の指摘として明記している）。

`generation = 0` は理論上の懸念ではなく到達可能である。起動シーケンス
（`app/bootstrap.rs` → `establish_initial_focus_scope`、`runtime/focus_tracking.rs:107-133`）
は `process_changed` を捨てて `FocusChanged` を dispatch しないため、起動直後に IME refresh
（`runtime/ime_refresh.rs::ir_poll_and_learn`）が IMM クエリを blacklist/skip 判定で
スキップすると `ImeEvent` が1件も record されないまま `try_force_on_bootstrap()` が
`allocate_event_generation()` を呼び、`next_seq == 0` のとき `generation = 0` が払い出される。
ADR-104 決定6-c が「正当な generation 値 `0` と衝突する」と書いた懸念は、経路の名指し
（ImmCross ではなく Bootstrap force-on）こそ不正確だったが、主張の実質は正しかった。

同種の借用は `focus_epoch`（「プロセスが変わった回数」を「フォーカス対象の同一性」判定に
流用）にも存在する。借用した識別子は共通して3つの病を発症する:

1. **自分の都合で進む／進まない**（`next_seq` は全 `ImeEvent` で進む、`focus_epoch` は
   同一プロセス内のウィンドウ移動では進まない）
2. **初期値 `0` が意味を持ってしまう**（`next_seq: 0` が「まだ何も起きていない」と
   「正当な最初の値」の両方を意味してしまう）
3. **割り当てと消費の間に型で守られない契約が生じる**（`allocate_event_generation` は
   読むだけで、進めるかどうかは呼び出し元のマナーに依存する）

ADR-104 決定6-a はこの病を正しく認識していながら（「カウンタは `event_log.next_seq()` を
流用しない」と明記して `intent_seq` を新設する理由に挙げている）、**`generation` 自体が
既にこの病に罹っていることに気づいていない**。同じ ADR の中で、避けるべきアンチパターンを
正しく回避した決定（6-a）と、そのアンチパターンそのものである既存コード（`generation`）を
放置する決定（6-c）が同居している。

### 原因B: 観測できない状況を「観測」として記録している

TsfNative/Imm32Unavailable では IMM32 API が使えず ON/OFF を直接観測できない。
`runtime/key_pipeline.rs` の `apply_focus_probe` はこの窓で `shadow_on = effective_open()`
（belief 由来の値）を `apply_effective_ime()` 経由で `write_focus_probe()` に渡し、
観測プールに実際の `ImeObservation` として書き込む（`used_shadow_fallback` はこの経路の
発生を示すフラグとして `key_pipeline.rs` 内で実際に消費されている＝**死んだコードではない**）。
この観測は定義上 `open == desired` になるため、`check_drift_correction` の
`if trusted.open == desired { return None; }` に毎回引っかかり、drift correction は
**構造的に一度も発火し得ない**（`docs/known-bugs.md` BUG-33 で既に確定済みの事実）。

ADR-104 自身が INV-C（「無い」「確度が低い」「失敗した」を `0`/`false`/`()` に潰さず
`Option`/`NonZero`/専用 enum で運ぶ）を掲げているが、これを適用すべき最大の対象は
WM ペイロードのビット幅（決定6-c、しかも前述のとおり前提の一部が不正確）ではなく、
**観測ストアそのもの**である——「TsfNative では open が観測できない」を型で `None` として
運べば、drift correction は「判定不能」を返し、下流の誰も偽の一致に騙されない。

加えて、`ObservationStore::current_focus_epoch` を更新する `clear_on_focus_change` は
`ImeEvent::FocusChanged`（`on_focus_process_changed` からのみ dispatch）でしか呼ばれないため、
**同一プロセス内でのウィンドウ移動では観測プールが一度もクリアされず、`derive_any()` の
epoch フィルタも素通りする**。ADR-104 決定6-a の `focus_hwnd` 拡張は書き込み側（spawn 時に
古い hwnd のスナップショットを書かせない）しか塞がず、**既に書き込まれている古い hwnd 由来の
観測が読み出され続ける**という読み取り側の穴は残る。`ImeObservation` は既に `hwnd` フィールドを
持つ（`record_any` が埋めている）が、フィルタには一切使われていない。

### 原因Bが決定7をも巻き込む理由

決定7（`[focus-conv-check]` の同期 conv 読み取りを非同期 offload する）は `ConvModeMgr`
（`state/conv_mode.rs`、`Cell<Option<ConvMode>>` のみで時刻も epoch も hwnd も持たない）
への書き込みを2箇所（idle-conv-check、focus-conv-check）から行っている。現状は両方が同期
だから順序が保証されているが、決定7 が片方だけを非同期化すると last-writer-wins 競合が
生まれる。これは `docs/known-bugs.md` の BUG-34 追補（2026-08-19）が「site A の offload を
見送った理由」として既に記録している懸念であり、`key_pipeline.rs::apply_focus_probe` 直上の
コメントも「実機ソーク無しにここだけ先走ると新しい race を作り込む恐れがある」として同じ
判断を明示的に残している。ADR-104 決定7 はこの既存の見送り判断を、それを覆す新しい根拠
なしに反転させている。

### 制約

[ADR-104](104-observation-freshness-and-hardening.md) と同じ制約を継承する。

- [ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md) の3層分離を
  破らない。「API を叩いていない値を観測として記録する」laundering は導入しない
  （本ADRはむしろこの laundering を1つ物理的に消す）。
- タイミング定数は変更しない。

## 不変条件

- **INV-C（継承・具体化）**: 観測できない状況は `bool` に潰さず、型で「判定不能」として
  運ぶ。特に observation store への書き込みに適用する（ADR-104 が WM ペイロードのビット幅
  だけに適用していた範囲を、観測の入口へ広げる）。
- **INV-D（新設）**: fence/一意性を担う識別子は、その目的のためだけに所有された専用の型で
  払い出す。診断用・別目的のカウンタを比較や照合に流用しない。

---

## 決定1: `ApplyGeneration(NonZeroU64)` 専用アロケータ（ADR-104 決定6-c を置き換える）

`ImeEventLog.next_seq` から独立した、`&mut self` の専用アロケータを `ImeModel` に持たせる。

```rust
// state/generation.rs（ungated、Win32 非依存）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ApplyGeneration(NonZeroU64);

#[derive(Debug)]
pub struct GenerationAllocator { next: NonZeroU64 }

impl GenerationAllocator {
    pub const fn new() -> Self { Self { next: NonZeroU64::MIN } }
    /// `&mut self` により「読むだけで進まない」ことが型で不可能になる。
    pub fn allocate(&mut self) -> ApplyGeneration {
        let g = self.next;
        self.next = self.next.checked_add(1).unwrap_or(NonZeroU64::MIN);
        ApplyGeneration(g)
    }
}
```

**効果**:

1. 「割り当てたのに `dispatch_event` しないと壊れる」契約が消える。`ImeEventLog` から
   独立するため、診断ログの記録有無と generation の一意性が無関係になる。
2. `NonZeroU64` により `0` を「generation なし」の番兵として使うのが型として正しくなる。
   `Option<ApplyGeneration>` は無損失で `0 = None` にエンコードできる。
3. ADR-104 決定6-c が要求していた `EncodeDegradation` enum・`has_generation` ビット・
   「収まらない値は劣化させる」全数往復テストは**不要になる**。`generation` は
   `runtime/message_handlers.rs:409-441` の wparam エンコード（現状 bits2.. に詰めている、
   実測で 62bit 分の空間があり、ADR-104 が想定した「bits3.. に 61bit」より広い）が、
   `NonZeroU64` は `checked_add` で折り返す設計にするため原理的にオーバーフローしない。
4. reason（`OpenApplyReason::Bootstrap`/`EngineDecision`）のビット幅拡張し忘れ問題は、
   `message_handlers.rs:364-372` の `encode_outcome` が既に採用している**網羅 match**
   （variant 追加時にコンパイルエラーで追従を強制する）を `decode_reason` にも適用すれば
   十分であり、codec 型を新設する必要はない。

**証拠義務**: `state/generation.rs` は ungated のため Linux で単体テスト可能。
`allocate()` の単調増加・折り返し・`Option<ApplyGeneration>` のエンコード往復を固定する。
`docs/known-bugs.md` への追記は不要（挙動変化はテスト強化のみ、実害の追記対象ではない）。

**コスト/リスク**: 小/小。既存 `generation: 1`/`10`/`9` を使うテスト群はそのまま
`NonZeroU64` 化で通る想定。

---

## 決定2: 観測不能プロファイルの型による明示化（ADR-104 決定6-a のうち shadow write 部分を置き換える）

`sanitize_focus_probe_open_status` 相当の戻り値を、欠落の理由を運ぶ enum に変える。

```rust
// state/observation_store.rs 付近
pub enum FocusProbeOpenStatus {
    Read(bool),
    NotObservable(AppImeProfile),  // TsfNative / Imm32Unavailable
}
```

`Observed<FocusProbe>`（もしくは既存の `write_focus_probe` が受け取る型）は `Read` からしか
構築できないようにする。これにより `apply_focus_probe` の
`self.apply_effective_ime(shadow_on, ...)`（belief 由来の値を観測として書く経路）は
**コンパイルエラーとして落ちる**。`kp_stage_focus_probe` の `shadow_on = effective_open()`
キャプチャも合わせて削除できる。**ADR-104 決定6-a が `intent_seq`（2段階 admission）で
守ろうとしていた唯一のサイトが、これによって消える。`intent_seq` は不要になる。**

**「唯一の観測源が消える」という懸念について**: 成立しない。BUG-33 が確定させたとおり、
その観測源は定義上 `desired` と一致する自己参照値であり、`check_drift_correction` は
現状既に毎回 `None`（不一致なし＝補正不要）を返している。本決定は drift correction の
能力を減らさず、**減っていた事実を可視化するだけ**である。

**guard 解除の副作用に関する注意点（撤去と同じコミットで扱うこと）**: 現在
`apply_effective_ime(effective)` は `effective == true` のとき `reset_detect_state()`
（observe-miss リセット＋force guard 全解除）を呼んでいる。これは観測記録とは別の副作用
なので、shadow 経路の撤去時に黙って落とすと `BrokenAppBootstrap` guard 等の解除タイミングが
失われる。撤去と同じコミットで「この経路で guard を解除すべきか」を明示的に判断し、
必要なら独立した呼び出しとして残すこと。

**計装（決定6-bの代替材料）**: `check_drift_correction` が「観測不能」で `None` を返した
回数と、これらのプロファイルで実際に回復を担っている既存経路（per-VK confirm give-up →
`send_chrome_gji_reinit_and_poll`、focus-resync + idle-conv-check、`ConvOpenInference` +
明示意図）の発火回数を journal に数える。ADR-104 決定6-b の「発火回数の前後比較をソークで
取る」は比較の基準線が無いまま提案されていたが、本決定によって基準線が作れる。

**証拠義務**: `state/observation_store.rs` は ungated。`FocusProbeOpenStatus::NotObservable`
から `Observed<FocusProbe>` が構築できないことを型レベルで（コンパイルが通らないことを
確認するテストとして）固定する。`docs/known-bugs.md` に暫定 **BUG-81** として、BUG-33 の
追補（「shadow フォールバックの循環を型で閉じた」）を起票する。

**コスト/リスク**: 中/中。`effective_open()` の解決順は変わらないが、「3秒 FRESH を超えて
凍った古い shadow 観測」が消える分だけ挙動が変わりうる。`tests/drift_correction_replay.rs`・
`tests/journal_replay.rs` で前後比較を必ず取ること。

---

## 決定3: `FocusIdentity { epoch, hwnd }` への格上げ（ADR-104 決定6-a のうち hwnd 部分を置き換える）

書き込み側だけでなく読み取り側も直す。

```rust
// focus/ に新設。FocusStore を唯一の生成元にする。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FocusIdentity { pub epoch: FocusEpoch, pub hwnd: HwndId }
```

`ImmLikeTicket.focus_epoch` → `FocusIdentity`、`AcceptedObservation.focus_epoch` →
`Redeemed<FocusIdentity>`相当、`ObservationStore.current_focus_epoch` →
`current_focus_identity`、`derive_any()` のフィルタを `o.identity == current` に変更する。
`ImeObservation.hwnd` は既に `record_any` が埋めているため、**新しいデータは増えない**——
既に持っていて使っていなかった値を読み取り側フィルタでも使うだけである。

**ADR-104 決定6-aとの関係**: hwnd 項がこれで丸ごと置き換わる。決定2で shadow write 自体が
消えるため、決定6-aが新設しようとした `ObservationTicket` は不要——既存 `ImmLikeTicket` の
中身を `FocusEpoch` → `FocusIdentity` に差し替えるだけで済む。

**証拠義務**: `focus/` は ungated。「同一プロセス内で hwnd だけ変わると観測が失効する」
ケースを固定するテストを追加する。挙動変化は「同一プロセス内でウィンドウが変わると観測
プールが失効する」ことで、これは BUG-18（AppKind Uwp 往復での文字欠落）の周辺に触れるため、
`clear_on_focus_change` を同一プロセス内でも呼ぶかどうかは実機ソークで確認すること。
`docs/known-bugs.md` の BUG-18 エントリに本決定との関係を追記する。

**コスト/リスク**: 中/中。

---

## 決定4: `ConvModeMgr` → `ConvObservation`（決定7 の前提工事。決定7 自体は本ADRでは実施しない）

```rust
pub struct ConvObservation {
    mode: ConvMode,
    read_at: TickMs,
    focus_epoch: FocusEpoch,  // 決定3後は FocusIdentity
    hwnd: HwndId,
    source: ConvReadSource,   // IdleCheck / FocusCheck
}
```

`update_from_conv(u32) -> bool` を `observe(ConvObservation) -> bool` に置き換え、
`read_at` が現在値より古い観測と `focus_epoch`/`hwnd` が現在と異なる観測を棄却する
（monotonic guard）。in-flight フラグは `bool` ではなく `Option<u64>`（spawn 時刻）+
stale 自己回復（`docs/known-bugs.md` BUG-34 追補2 が `idle_conv_check_in_flight` について
既に一度直した欠陥——`bool` だと `with_app` 再入失敗1回でプロセス寿命いっぱいラッチする——
と同型の再発を防ぐ）とする。

**本ADRのスコープ外にする理由**: この工事が入って初めて、ADR-104 決定7（focus-conv-check
の非同期 offload）が「新しい race を作らずに」着手可能になる。しかし決定7 自体（実際に
非同期化する変更）は、`send_health::blocking_allowed()` ブレーカが既にこの箇所に配線済み
であり緊急性が最上位ではないこと、および `.claude/rules/experiment-logging.md` が警告する
「見送り判断の根拠なき反転」を避けるため、**本ADRでは実施しない**。決定4のみを先行実装し、
決定7 は「実機ログで既存ブレーカの不足を確認してから」着手すること。

**証拠義務**: `state/conv_mode.rs` は ungated、Linux で単体テスト可能
（`.claude/rules/fix-requires-evidence.md` の「Linux で実行できるものを優先」に合致）。
monotonic guard の全数テスト（古い/新しい×epoch一致/不一致×hwnd一致/不一致）を追加する。

**コスト/リスク**: 中/小。純粋な型格上げで、既存挙動は monotonic guard 分しか変わらない。

---

## 決定5: fence/観測 admission の統一抽象——`Lease<P>` パターンは共有するが、admission と actuation target は別型に保つ

決定2〜4の各サイトに個別実装したあと、リポジトリには依然として「spawn 時にコンテキストを
捕捉し、完了時に前提の非崩壊を再検証する」という同型パターンが複数箇所に存在し続ける
（`ImmLikeTicket`、idle-conv-check の再検証、`FocusResyncGate` の generation+CAS、
ADR-101 の `ime_mode_focus_gen`、ADR-103 の `PendingGjiReinit.focus_gen`、決定1の
`ApplyGeneration` 照合）。レビュー段階では「統合対象の範囲」——特に「観測の受理」
（`ImmLikeTicket`、idle-conv-check）と「actuation の着弾先同一性」（`ime_mode_focus_gen`/
`PendingGjiReinit.focus_gen`）を同じ統合抽象に含めるか——で意見が割れていたが、
`docs/known-bugs.md`・既存 ADR に対するバグ考古学（2026-08-26 実施）により決着した。

### バグ考古学で得た証拠

**「別目的の値・状態を1つの機構に共有する」ことが実際にバグを起こした前例が2件ある:**

1. **BUG-77**（`docs/known-bugs.md:10192-10202`）: idle-conv-check のスパムガード用
   フラグ `idle_conv_check_in_flight_since_ms` を resync トリガーと共有していたため、
   無関係な修飾キー付きキーが先に in-flight を掴むと resync が「既に in-flight」と
   誤判定し、conv 読み取りを一度も行わないまま gate を閉じて**本バグ自体が再発**した。
   修正は「resync はこの共有フラグを一切読み書きしない」——別目的の状態を共有しない
   ことだった。
2. **ADR-087 §1.4-1**（`docs/adr/087-open-belief-actuation-warrant-separation.md:110-114`）:
   `effective_open()` が (a) engine 内部挙動の決定と (b) OS への実書き込みの授権という
   **2つの異なる目的に同時に使われ**、「同一の bool フリップが両方を同時に引き起こす」
   ことが 2026-08-10 の実バグ（Windows Terminal でかな変換混入＋意図せぬ IME ON）の
   共通原因と特定された。修正は `OpenWarrant`/`WarrantBasis` という別型への分離。
   これは「観測の受理」と「actuation の根拠」を同じ機構に混ぜるとどうなるかの、
   本ADRの対立点と同型の失敗パターンである。

**一方、「同じ形（パターン）を複数の別型として再利用する」ことは既に成功前例がある:**

3. **ADR-086 §3 案T4/T5**（`docs/adr/086-force-write-trigger-and-target-identity.md:583-591`）:
   世代カウンタ単独（案T4、現状の `ime_mode_focus_gen`）は時間軸しか守れないため
   「単独では却下」とし、`ActuationTarget { hwnd, focus_gen }`——**epoch(時間軸)と
   hwnd(空間軸)を1つの型に同梱し、使用直前に再検証する**——を採用した。これは決定3の
   `FocusIdentity { epoch, hwnd }` と構造的に同一の形であり、「capture して使用時に
   再検証する」というパターン自体は ADR-086 が既に独立に一般原則化している
   （INV-14「非同期に実行される外部書き込みは時間軸と空間軸の両方をフェンスしなければ
   ならない」）。
4. **ADR-087 §1.5.1 の軸分離表**（`docs/adr/087-open-belief-actuation-warrant-separation.md:174-179`）:
   「時間軸(ADR-077 epoch admission)・空間軸(ADR-086 target identity)・トリガー軸
   (ADR-086 INV-15)・根拠軸(ADR-087)」を明示的に別軸として立てている。観測の
   admission（時間軸寄り）と actuation の着弾先同一性（空間軸、ADR-086 固有）は、
   このリポジトリの設計語彙において**既に別概念として確立**している。
5. **`docs/adr/index.md` 長期的教訓**: 「`belief.ime_on` のような優先度型は『状態の
   責務分離』を阻む」——ADR-032 で Intent/Observation/Transition/Barrier の4カテゴリに
   分解した効果として記録されている、同型の一般原則。

### 決定

**証拠1・2は「同じ値・同じフィールドを2つの異なる目的で共有する」ことへの警告であり、
証拠3・4は「同じ形のパターンを、別々の型として複数箇所で再利用する」ことへの支持である。**
この区別に基づき次のように決定する。

- **`Lease<P: Precondition>`（または `Captured<T>`）という capture→redeem の**型パターン**
  は採用する。** `ImmLikeTicket` と idle-conv-check の再検証をこのパターンで統合する
  （どちらも「観測を belief に入れてよいか」という同じ述語を守っている、同一責務内の
  重複であるため——ADR-077 自身の発端「観測の信用度判断が分散している」と同じ理由で
  統合が正当化される）。
- **`ime_mode_focus_gen`/`PendingGjiReinit.focus_gen`（actuation の着弾先同一性）は
  `Lease<P>` の型ファミリーには含めるが、admission 側とは別の具体型・別のストレージ・
  別の呼び出し経路として実装し、2つが同じ `admit()`/`redeem()` API を共有すること
  （＝同一の値・同一の判定関数を両者が読み書きすること）は禁止する。** 実装上は
  `Lease<FocusIdentity>`（観測 admission 用）と、将来 ADR-086 の `ActuationTarget` を
  同じパターンで再実装するなら `Lease<ActuationTargetIdentity>`（actuation ターゲット用、
  別モジュール・別トレイト実装）のように、**型は同じジェネリック構造から生成されるが
  インスタンスとストレージは完全に分離する**。
- **`FocusResyncGate` の generation + CAS**（one-shot 消費による相互排他が本質）は
  引き続き統合対象から除外する。`Lease`/`Captured` の「同じなら通す」意味論は排他とは
  異なり、統合すると BUG-77 で確立した排他設計を壊すリスクがある（両陣営で既に一致
  していた点、バグ考古学でも覆らなかった）。

**着手条件**: 決定2〜4により `intent_seq`・shadow write・hwnd 単独照合が消え、統合すべき
対象サイトが `ImmLikeTicket` と idle-conv-check の2箇所に絞られてから実施する。

---

## 実施順序

| Phase | 内容 | 依存 |
| --- | --- | --- |
| 1 | 決定1（`ApplyGeneration`） | 依存なし。単独で即実施可 |
| 2 | 決定4（`ConvObservation`） | 依存なし。単独で即実施可、Linux テストのみで検証可能 |
| 3 | 決定2（観測不能プロファイルの型明示化） | 依存なし。決定3と同時でも別でもよい |
| 4 | 決定3（`FocusIdentity` 読み取り側統合） | 決定2と合わせて実施すると効果検証しやすい |
| 保留 | ADR-104 決定7（focus-conv-check offload） | 決定4完了後、実機ログでブレーカ不足を確認してから別途判断 |
| 5 | 決定5（`Lease<P>` への `ImmLikeTicket`/idle-conv-check 統合。actuation ターゲット同一性は含めない） | 決定2〜4完了後、統合対象サイトが2箇所に絞られてから |

決定11-b（`force_vk` の UWP InputSite フォールバック非対称）は上記と独立して、BUG 起票の
上で並行して着手してよい。

## 却下した代替案

- **ADR-104 決定6-a の `ObservationTicket`（`focus_epoch`+`focus_hwnd`+`intent_seq` の
  2段階 admission）をそのまま実装する**: 決定2（shadow write の型による消去）を先に
  行えば `intent_seq` が守るべきサイト自体が消えるため、2段階 admission という複雑な
  分岐を実装する必要が無い。「守るべき観測源」の実効性（BUG-33 により既に機能していない）
  を確認しないまま、それを守るための照合機構を複雑化する判断は採らない。
- **ADR-104 決定6-c の `EncodeDegradation` + ビット幅劣化設計**: 決定1（`NonZeroU64`
  専用アロケータ）により、劣化ケース自体が原理的に到達不能になる。到達不能なケースの
  ために enum variant と全数往復テストを新設するのは、CLAUDE.md の「起こり得ない
  シナリオのためのフォールバックを追加しない」に反する。
- **決定6-b（drift confidence 3値化）を本ADRに含める**: `.claude/rules/tuning-constants.md`
  は値変更前の実測義務を要求するが、決定6-bは「効く窓の大きさ」自体を見積もっていない
  （明示意図がある窓は既に閾値0のヘアトリガーであり、6-bが効くのはそれ以外の窓に限られる）。
  決定2で作られる計装（観測不能→`None`の発火回数）が基準線を提供したあと、ADR-104側で
  単独に扱うべきであり、fence の所有権是正とは別軸の問題として本ADRからは切り離す。

## 未解決の疑問（実機ソークで確認すること）

- 決定2により「3秒 FRESH を超えて凍った古い shadow 観測」が消えることで、TsfNative/
  Imm32Unavailable の `effective_open()` 解決結果が変わるケースが実機で発生するか。
- 決定3の「同一プロセス内 hwnd 変更で観測失効」が BUG-18（AppKind Uwp 往復）の挙動と
  干渉しないか。
- ADR-104 決定7 の非同期化を実施する場合、決定4後も残る `conv_mutation_seq`・in-flight
  再評価のみで十分か、実機ログでの再検証が必要。
- （決定5の統合範囲は2026-08-26のバグ考古学で確定済み。将来 ADR-086 の `ActuationTarget`
  を `Lease<P>` パターンで再実装する場合でも、admission 側とストレージ・API を共有しない
  ことを実装レビューで確認すること。）

## 設計の経緯

ADR-104 に対し Opus 2体（r1a・r1b）でラウンド1（独立レビュー）→ラウンド2（相互攻撃・
実コード再検証）の敵対的レビューを実施した。転換点: (1) r1bが「決定6-cの前提は事実誤認で
到達不能」と主張したが、r1aがラウンド2で `allocate_event_generation`/`next_seq`/
bootstrap 経路を実際に追跡し**ADR-104の記述の方が正しい**と再確定させた。(2) その過程で
両陣営が見落としていた「`allocate_event_generation` は割り当てではなく読み取りである」
という、決定6-cより一段深い欠陥（原因A）が発見された。(3) r1bの「shadowフォールバックは
BUG-33により既に死んでいる」という主張はr1aがラウンド2で実コード（`used_shadow_fallback`
の消費箇所）を確認し誤りと判定したが、その生きている機構が原因Bの実害（drift correctionの
構造的不発火）を持つこと自体はr1a・r1b双方が独立に到達した結論として残った。(4) 決定7に
ついてはr1a（「実機ソーク無しの race」というコード内コメント）とr1b（`ConvModeMgr`の
last-writer-wins）が独立に同一の既存見送り判断を発見し、根拠を補完し合う形で収束した。
(5) 統合抽象（決定5）の範囲については敵対的レビューの2ラウンドでは収束せず、当初は
明示的に保留としていた。

**2026-08-26 追記（バグ考古学による決定5の確定）**: 統合範囲の判断材料として、実機
スパイクではなく `docs/known-bugs.md`・既存 ADR に対する考古学（過去に同種の統合/分離が
実際にバグを生んだか・防いだかの事例調査）を実施した。実機での挙動確認では決定5の対立点
（型設計・保守性の判断）に有効な情報が得られないため、より投資対効果の高い手段として選んだ。
BUG-77（共有 in-flight フラグが resync を誤って早期終了させた）と ADR-087
（`effective_open()` の二重用途が 2026-08-10 の実バグの共通原因だった）という「別目的の
値を1つの機構に共有して実際に壊れた」2件の前例と、ADR-086 の `ActuationTarget{hwnd,
focus_gen}`（「同じ形のパターンを型として再利用する」ことには成功前例がある）を突き合わせ、
「値の共有」と「パターンの再利用」を区別する基準を得た。この基準により、admission と
actuation ターゲット同一性は型ファミリー（`Lease<P>`）は共有してよいが、具体型・
ストレージ・呼び出し経路は分離する、という形で決定5を確定した。
