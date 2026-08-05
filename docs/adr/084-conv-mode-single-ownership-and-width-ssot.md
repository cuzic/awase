# ADR-084: conv-mode の単一所有権と「出力の幅を IME に委譲しない」原則 — 物理シフト面・belief キャッシュ・送信保証の責務再配置

## ステータス

**提案（Draft、未実装）。北極星仕様。**

本 ADR は個別バグの修正手順書ではなく、**以後の実装判断を評価するための基準**である。`origin/develop`（`e99f20df`）時点のコードは原則 P1〜P5 のいずれにも完全には適合していない。既存コードの一括作り替えは求めない。求めるのは、**この領域に触れる変更が原則からの距離を縮めるか、少なくとも広げないこと**である。

対象領域: `crates/awase-windows` の conv-mode 制御・物理シフト面処理・文字送信経路。

**実機検証の状態**: 本 ADR の ms 値・因果の主張はすべて既存の `docs/known-bugs.md` / `tuning.rs` に記録済みの実測に由来する。本 ADR 自体は新規実測を行っていない（サンドボックスに Windows 実機が無い）。実装着手前に必要な実測は §7 に明示する。

**ブランチ注記**: 調査時点でリポジトリは `main` にチェックアウトされており、`e99f20df` は `origin/develop` にのみ存在する（ローカル `develop` は複数コミット遅れていた）。`vk.rs` は `main` と比べ純粋な追加（削除行なし）。実装は `origin/develop` から分岐すること。ADR 番号 083 は `origin/develop` で使用済み（`083-injection-mode-per-vk-unification-investigation.md`）のため 084 を採番した。

**成立の経緯**: 本 ADR は Opus・Fable・Codex の3系統に同一ブリーフを与え独立に草案を作成させ、収束した結論を軸に統合したものである。3系統とも独立に「conv-mode の書き込みと belief 無効化を不可分にする」「物理 Shift の意味づけが確定するまで外部状態への投機的書き込みを行わない」「記号の全角/半角を IME の conv-mode に委譲しない」という原則へ収束した。本稿は最も検証が深かった草案（`.yab` パーサの `Literal`/`KeySequence` 構文の発見を含む）を土台に統合している。

---

## 1. コンテキスト

### 1.1 発端となった不具合

小指シフト面（`layout/nicola.yab` の `[ローマ字小指シフト]`、物理 `VK_LSHIFT`/`VK_RSHIFT` を押しながら文字キーを打つ面）で「！」を入力すると、全角「！」ではなく半角「!」が出力される（2026-08-05 ユーザー報告、BUG-47 追補の残件）。

直接の因果:

1. 物理 Shift 押下 → `kp_shift_conv_guard_key_down`（`key_pipeline.rs`）が、MS-IME のときのみ IMC write で conv を `0x00000000`（IME-ON 半角英数）へ切り替える。実装は `set_ime_romaji_mode_with_target_async(Some(0))` → `WM_IME_CONTROL`/`IMC_SETCONVERSIONMODE` の **クロスプロセス `SendMessageTimeoutW`**（`ImmSetConversionStatus` でも TSF でもない）。`spawn_local` + `offload` で完全非同期（フックスレッドは BUG-34 のためブロックできない）。150ms 後に独立タスクが 1 回読み戻すが、これは**ログのみの診断であり、状態は一切書かない**。
2. `ImeModeFsm`（`tsf/ime_mode_fsm.rs`）の `confirmed` は**この時点で落ちない**。
3. engine がチョードを解決し「！」を出力する。`resolve_char('！')` → `CharResolution::Vk(0x31, true)` → `vk_pair_to_ascii(0x31, true) == Some('!')` → `send_romaji_batched("!")`（`e99f20df` の BUG-47 追補で合流させた経路）。
4. `ms_ime_gate_defer` が stale な `is_native_ready() == true` を信じて即時送信。
5. 実 IME は conv=0 のままなので Shift+1 が全角変換されず、半角 `!` が出る。

### 1.2 「一箇所を直す」修正が二度否定された事実

- **案A（Shift 押下時にも `ImeModeFsm::unconfirm()`）**: `MS_IME_READY_CONFIRM_MS = 400` が「打鍵時点」起点であり「Shift 解放時点」起点ではないため、Shift を ~300ms 超保持すると再現する。加えて期限到達で `ms_ime_gate_give_up` がラッチされ、フォーカス変更／次の `SetOpen(true)` まで MS-IME cold-start 保護（BUG-13）が無効化される。
- **案B（全角記号 21 種を `build_symbol_to_vk` から削除し Unicode へ）**: `e99f20df` は逆方向に、全記号を cold-start 保護へ合流させた。削除すると `defer_vk_if_probe_in_flight` の順序保証・`mark_composition_cold` の warmth 追跡が失われ「ば！」→「！ば」を生む。スコープも誤りで、`？（）｛｝～` は親指シフト面（`[ローマ字左親指シフト]`/`[ローマ字右親指シフト]` の数字段、両者同一行）でも使われ、そちらは正常動作している。

両案が失敗するのは、**どちらも「症状が出ている 1 箇所」を触っているのに、症状を生んでいるのは 4 つの関心事の境界の引き方だから**である。

### 1.3 発見した構造的欠陥

#### 欠陥1: conv mutation と belief 無効化が同一トランザクションになっていない

`ImeModeFsm::unconfirm()` の doc（`ime_mode_fsm.rs`）:

> 外部要因で conv が変わった可能性があるとき、belief の state は保ったまま unconfirmed 化する。
> 用途: Shift 解放時（MS-IME が Shift 単独タップと誤認して英数へ切り替える可能性があるタイミング）等、**awase の送信起点ではない conv 変化の疑い**。

ところが `shift-conv-guard` の conv=0 書き込みは、疑いでも外部要因でもなく **awase 自身が意図して行う conv mutation** である。にもかかわらず対応する無効化点が存在しない。`on_set_open_applied` は `SetOpen`（IME 開閉）用であり conv-mode 書き換え用ではない。

**決定的な事実**: `.unconfirm(` の呼び出しは **リポジトリ全体でただ 1 箇所**（`key_pipeline.rs`、理由文字列 `"shift-conv-guard release"`）。しかもそれは `kp_restore_kana_from_half_width` の中、すなわち **Shift 解放（または フォーカス変更等）時にしか到達しない**。同様に `confirmed = true` を書くのも `on_conversion_mode_read` ただ 1 箇所である。

つまり **awase には「自分で conv を書いたのに、どの belief キャッシュにもそれを通知しない」経路が存在する**。これが今回のバグの一般形である。

#### 欠陥2: 物理 Shift キーが 3 つの意味を同時に背負っている

| 意味 | 誰が解釈するか | 現状 |
|---|---|---|
| OS の修飾キー（Shift+Tab、Ctrl+Shift+T、直接入力の大文字） | OS / アプリ | 素通し |
| NICOLA 配列のシフト面セレクタ（小指シフト） | awase engine | チョード解決で consume |
| MS-IME の「単独タップで半角英数」トリガ | MS-IME（無効化不能） | awase が打ち消す |

awase がチョードを consume すると、OS からは「Shift down →（何も見えない）→ Shift up」に見え、MS-IME が単独タップと誤認する。`docs/known-bugs.md` は BUG-15・BUG-25 の両方でこれを **二重オーナー構造**と明示している。

なお `kp_stage_shift_conv_guard` は `kp_stage_post_decision` の最終段で走り、`engine.on_input`（チョード解決）より**後**、`kp_stage_execute`（出力）より**前**である。ただしこれはイベント単位の話であって、Shift **KeyDown** イベントの時点では後続の文字キーがまだ来ていないため、チョードか単独タップかは原理的に未知である。判定材料（`left_shift_tap_candidate`）が揃うのは Shift **KeyUp** の時点である。

#### 欠陥3: 出力文字の「幅」が SSOT から失われ、IME に再構成させている

最も本質的である。

`[ローマ字小指シフト]` は**全キーが全角**（`！＂＃＄％＆＇（）：＝～｜` / `Ｑ〜Ｐ＇｛` / `Ａ〜Ｌ＋＊｝` / `Ｚ〜Ｍ＜＞？＿`）。

さらに重要な事実として、**`.yab` は幅を宣言する構文を既に持っている**（`src/yab/mod.rs`）:

| 記法 | パース結果 | 出力 |
|---|---|---|
| `'！'`（クォート付き） | `YabValue::Literal("！")` | **全角のまま逐語出力**（幅変換しない） |
| `！`（クォートなし） | 全角 ASCII → 半角化 → `KeySequence("!")` | 半角 |
| `ｋａ`（クォートなし英字） | 半角化 → `Romaji("ka")` | かな変換 |

そして **`[ローマ字小指シフト]` は全セルがクォート付き**、すなわち全セルが `Literal` = 「全角で逐語出力せよ」という明示的宣言である。

ところが送信経路はこうなる:

```
'！'（Literal、レイアウトが「全角」と明示的に宣言）
  → resolve_char → build_symbol_to_vk: ('！', 0x31, true)
  → vk_pair_to_ascii(0x31, true) = '!'      ← ここで全角という宣言が消える
  → send_romaji_batched("!")                 ← 送るのは半角の '!'
  → IME が conv=NATIVE なら '！' に戻してくれる（はず）
```

**awase は「全角の！を出せ」という、レイアウトに明示的に書かれた確定した宣言を、いったん半角 `!` に落として送信し、その復元を外部プロセス（IME）の内部モードに委ねている。** そして同じ awase の別コンポーネント（`shift-conv-guard`）が、まさにその復元を壊すモードを IME に書き込んでいる。

歴史的経緯も判明している。BUG-15 の追補は当時こう書いていた:

> 「全角で出したい記号はクォート付き `'！'` で Shift 面に定義する。」

これは `KeyAction::Text`（Unicode 直接出力）経路が存在した時代の仕様である。その経路は BUG-15 追補 / BUG-25 で撤去された。**結果として、パーサには幅宣言の構文が残り、それを実行時に尊重する経路だけが消えた。** これが欠陥3 の正体である。

一方、同じ面の全角ラテン `Ｑ〜Ｚ` は `build_symbol_to_vk` に無く（このテーブルに全角ラテンのエントリは 1 つも無い）`CharResolution::Unicode` に落ち、conv-mode に一切依存せず正しく出る。**同一シフト面の中で、記号は conv 依存、ラテンは conv 非依存という分裂が起きている。**

BUG-15 の実機記録はこの論点に決定的な実測を残している: Windows Terminal では **ASCII の VK_PACKET は届かない**が、**全角 `Ｋ`（U+FF2B）は同じ経路で届いた**。「全角文字の Unicode 直接注入」はこの環境で実証的に成立する。

#### 欠陥4: 順序保証・warmth 保証が transport（`InjectionMode`）に紐づいている

- `defer_vk_if_probe_in_flight` / `defer_if_probe_in_flight` … `Vk`/`Tsf` 経路にのみ存在
- `unicode_cold_defer` / `unicode_cold_deferred` … `Unicode` 経路にのみ存在。しかも `injection_mode == Unicode` **かつ** `gji_is_next_key_long_cold()` のときだけ有効
- `mark_composition_cold` … VK 送信側にのみ存在

`ms_ime_gate_defer` の doc コメントはこの分裂を明文化してしまっている:

> `InjectionMode::Unicode` は IME composition を経由しないため元々このゲートを呼ばず、**対象外のままでよい**

保証が transport ごとに別実装で存在するため、**「送信方式を変える」と付随する保証が黙って消える**。案B が危険なのはまさにこれであり、案B の直感（全角は IME を経由すべきでない）自体は欠陥3 の観点でむしろ正しい。誤っていたのは transport を替えれば保証も付いてくるという前提のほうである。

### 1.4 既存資産との関係

本 ADR は既存 ADR を否定しない。**既存 ADR が定めた「権限」を「単一の実行窓口」へ引き上げる**ものである。

| 既存 | 何を定めたか | 本 ADR との関係 |
|---|---|---|
| ADR-064 `ConvModePolicy` | conv を書いて**よいか**（`AwaseLocked`/`UserManaged`、`Output.conv_mutation_allowed: Cell<bool>`）。全 conv 書き込み経路が `!conv_mutation_allowed` で early return | **許可ゲートはあるが、実行点が集約されていない**。`shift-conv-guard` も正しくこのゲートを見ている。P1 で単一 actuator へ |
| ADR-072 conv authority 再同期 | 「遷移エッジではなく **apply 完了点**で同期する」 | **本 ADR の直接の先例**。今回のバグは「conv 書き込み時点ではなく Shift 解放エッジで無効化した」という同型の誤り |
| ADR-078 belief 3 分割 | `DesiredMode`/`EffectiveMode`/`ModeConstraint`（Phase 1a のみ実装、型分割は未実装） | P1 の belief 側受け皿。ADR-078 は既に `ModeConstraint` が「`key_pipeline.rs` の Shift 解放復元（BUG-15）が場当たりにやっていたことの一般化」だと明言している |
| ADR-083 `InjectionMode` per-VK 統一 | **NO-GO**。`InjectionMode` は `AppImeProfile` ではなく `AppKind` から決まる | P4 は per-VK confirm 統一を**提案しない**。NO-GO を尊重する |
| `.claude/rules/ime-belief-architecture.md` | Observe → Pure → Apply、`InputModeApplied`/`ObserverReported`/confidence、3 段構えの強制 | INV 群と強制メカニズムをこの語彙・機構にそのまま乗せる |

---

## 2. 決定

### 責務の再配置（目標状態）

| 関心事 | 所有コンポーネント | 禁止事項 |
|---|---|---|
| 物理シフトキーの意味づけ（配列シフト面 / 単独タップ / OS 修飾キー） | **入力解釈層の 1 箇所**。判定結果を型で下流へ渡す | 送信層・conv 制御層が `VK_LSHIFT` の生の押下状態を直接読んで分岐すること |
| conv-mode の所有権と実際の書き込み | **単一の `ConvModeActuator`**（新設）。ADR-064 の許可判定を内包 | 他のどこからも IMC write / `VK_DBE_*` 注入を直接呼ぶこと |
| conv 由来 belief キャッシュの鮮度 | `ConvModeActuator` が**書き込みと同一トランザクションで**無効化 | 呼び出し元が「無効化を忘れないよう気をつける」運用 |
| 出力文字の幅（全角/半角） | **レイアウト（`.yab`）が SSOT**。`Literal` の宣言を最後まで保持する | 幅の決定を IME の conv-mode に委譲すること |
| cold-start / warm-up 保護 | 送信層の**単一ゲート**。transport 非依存 | transport ごとに別実装の保護を持つこと |
| 送信順序保証 | 送信層の**単一キュー**。transport 非依存 | `Vk` だけ defer、`Unicode` だけ defer という分裂 |

### 原則

#### P1: conv-mode の書き込みは単一の actuator を通り、belief 無効化は書き込みと不可分である

conv-mode を変更しうるすべての機構（IMC write、`VK_DBE_HIRAGANA`/`VK_DBE_ALPHANUMERIC` 注入、将来の `ITfLangBarItemButton` 経路）を単一関数へ集約する。

```rust
/// conv-mode を変更する唯一の窓口。呼び出し元は「なぜ変えるのか」を型で申告する。
pub(crate) fn actuate_conv_mode(
    &mut self,
    target: ConvModeTarget,      // Kana{katakana} / HalfWidthAlnum / Restore
    reason: ConvMutationReason,
    tick: TickMs,
) -> ConvActuationOutcome;

pub(crate) enum ConvMutationReason {
    ShiftSoloTapCounter,   // MS-IME 単独タップ誤認の打ち消し（安全網）
    HalfWidthAlnumToggle,  // BUG-25 の持続トグル
    WarmupRestore,         // warmup 経路の復元
    DriftCorrection,       // 観測との乖離補正
}
```

`actuate_conv_mode` は**成功・失敗にかかわらず**、戻る前に必ず次を行う。

1. ADR-064 の `conv_mutation_allowed` を確認（`UserManaged` なら `Rejected` を返し何も書かない）。
2. 実際の書き込みを行う（IME 種別による分岐はこの内側に閉じ込める）。
3. **conv から導出されているすべての belief キャッシュを無効化する**。現時点では `ImeModeFsm::unconfirm(reason)` が該当。将来 ADR-078 の `EffectiveMode` が入ればその stale 化もここで。
4. `InputModeApplied { strategy, result }` を dispatch（`.claude/rules/ime-belief-architecture.md` の禁止パターン2 に従い観測を偽装しない）。
5. `ime_actuation` ジャーナル（ADR-080/082）へ記録。

> **なぜ「直後」ではなく「同一関数の中」なのか**: ADR-072 が既に同じ教訓を出している。`conv_mode_authority` は `EngineStateChanged`（遷移エッジ）に依存していたため経路によって更新が漏れた。修正は `record_ime_apply_result`（全経路が必ず通る唯一の apply 完了点）へ移すことだった。**今回の `unconfirm` も「Shift 解放」という遷移エッジに依存しているため、Shift を離さない限り無効化されない。** 同じ誤りの二度目である。

#### P2: 物理シフトキーは配列のシフト面セレクタとして awase が所有し、その事実を IME に見せない

親指シフト面は、**同じ全角記号 `？（）｛｝～` を出力しているのに今日も正常に動いている**。理由は単純で、**親指キー（無変換/変換）は awase が完全に consume し MS-IME からは見えないため、打ち消すべき誤認がそもそも発生しない**からである。

これはこのリポジトリが既に採用している原則の再適用にすぎない（ImmCross アプリには物理 IME キーを見せない設計原則、spurious 連鎖を構造的に断つ）。

したがって北極星は、**小指シフトを親指シフトと同じ地位に置く**ことである。物理 Shift はレイアウトのシフト面セレクタとして awase が所有し、IME に「Shift が押された」事実を露出しない。合成が必要な瞬間は**同一 `SendInput` バッチの内側でのみ** `VK_SHIFT` を合成する（アトミックバッチ、ADR-048 と同手法。既に `send_vk_run_batch` が要素ごとの `needs_shift` を見て `VK_LSHIFT` down/up を挟む汎用実装になっている）。

これにより `shift-conv-guard` が打ち消そうとしている誤認**そのものが発生しなくなる**。打ち消しのための投機的書き込みも、その時間窓も、窓が生む stale belief も、まとめて消える。

**ただし無条件には成立しない。** 物理 Shift は OS 修飾キーでもあるため「常に飲み込む」ことはできない。飲み込みは engine がシフト面チョードとして consume した場合に限られ、それ以外は速やかに OS へ透過させる必要がある。この「保留して必要なら再生する」機構は既にある（`InputBarrier`、ADR-071 deferred VK queue）。**P2 の実装コストの本体はここであり、レイテンシと再生の正しさが最大のリスクである。**

#### P3: 出力文字の幅はレイアウトが SSOT であり、IME の conv-mode に委譲しない

engine が「全角！を出す」と決めたなら、その情報は送信の最後まで保持されなければならない。半角 `!` に落として IME に復元させる現在の設計は、**`.yab` に明示的に書かれた宣言を推測の対象に戻す**という点で誤りである。

原理的には:

- **かな**（`ｋａ` → 「か」）は IME を経由する必然性がある。ローマ字かな変換は IME の機能であり、変換後の文字列は後続入力で変わりうる（composition の一部）。awase は「か」を確定させたいのではなく、preedit に「か」を積みたい。
- **全角記号・全角ラテン**（`！`、`Ｑ`）にはその必然性がない。**それは変換の入力ではなく変換の結果そのもの**である。`！` を出すのに IME の変換テーブルを引く必要はない。IME を経由させているのは歴史的経緯（JIS で Shift+1 を打てば IME が全角にしてくれる）であって設計判断ではない。

したがって原理的な正解は「**幅が確定している文字は conv-mode に依存しない transport で送る**」である。案B の直感はここまで正しい。

**しかし P3 は単独で実装してはならない。** P4 なしには案B の欠点（順序保証・warmth 追跡の喪失）がそのまま出る。加えて実測制約がある。

- ASCII の VK_PACKET は Windows Terminal に届かない（BUG-15 追補、bare 化しても不達）。
- 全角 `Ｋ`（U+FF2B）は同じ経路で届いた（BUG-15 追補）。
- ADR-083 は Unicode 注入の composition 取り込みについて「証拠が支える強さを超えていた」と警告している。

本 ADR の立場:

> **全角記号は「composition に積む必要のない確定文字」であり、preedit が空のときに限り conv 非依存 transport で送ってよい。preedit が非空のときは composition の一部として扱う（= 現状の VK 経路を維持する）。**

preedit 状態を条件に含めるのは、「ば！」の順序逆転が composition 進行中に起きる問題だからである。preedit が空なら順序を保証すべき相手がいない。

#### P4: 順序保証と warmth 保護は transport ではなく送信要求に属する

送信層の入口を 1 つにし、`defer_vk_if_probe_in_flight` / `unicode_cold_defer` / `mark_composition_cold` を**その内側**に置く。

```rust
/// 送信要求。transport は決定の結果であって、保証の担い手ではない。
pub(crate) struct EmitRequest {
    ch: char,
    intent: EmitIntent,        // ComposeKana / CommitLiteral{width}
    ordering: OrderingClass,   // Sequenced / Independent
}
```

保証は `EmitRequest` を受けた単一ゲートが担う。transport（`Unicode`/`Vk`/`Tsf`）はゲート通過後に `AppKind` から決まる（ADR-083 の訂正どおり `AppImeProfile` ではない）。**これにより「transport を変える」変更が保証を落とすことが構造的に不可能になる。** P3 が安全に実施できるのは P4 の後だけである。

#### P5: 投機的な事前書き込み（speculative pre-write）を新設しない

「あとで打ち消すから先に書いておく」形の状態変更は、**必ず「書いてから打ち消すまで」の時間窓を作り、その窓で読まれる belief を stale にする**。

新たに外部状態を投機的に書きたくなったら、次の順で検討する。

1. **問題自体を消せないか**（P2 のように誤認が起きない構造にする）
2. **確定してから書けないか**（判定材料が揃うまで待つ。`left_shift_tap_candidate` は既に存在する）
3. どうしても必要なら**投機であることを型で表明し、窓の間その belief を読めなくする**

現行の `shift-conv-guard` は 3 すら満たしていない（窓の間 `is_native_ready()` が普通に読める）。

---

## 3. 代替案の比較（MS-IME「Shift 単独タップ→半角英数」への対抗策）

この領域は**このリポジトリで最も反転が繰り返された領域**である。IME OFF キー選択は 5 日間で 6 回反転し（`docs/experiments.md` エントリ01）、BUG-15 の対策自体も複数回の転換を経ている。**「良いアイデアに見えるか」ではなく「過去にどの条件で壊れたか」で評価する。**

### 案1: 現状維持（Shift down で投機的に conv=0、Shift up で復元）

- **利点**: 実機実績がある。BUG-15 の症状（数秒〜十数秒のかな入力破壊）を実際に抑えている。
- **欠点**: P5 違反。時間窓が stale belief を生む（本件）。GJI では **entry 機構が存在しない**（IMC write は mozc の TIP では UI 表示専用の一方向ミラーで実コンポーザに伝播しない、`VK_DBE_ALPHANUMERIC` 注入は awase 自身のフックにすら届かない — BUG-25 追補）。**現状の対策は MS-IME でしか動いていない。**
- **さらに**: entry は IME 種別ゲートされているが **restore はされていない**。GJI では entry が起きていないのに、Shift 解放のたびに `VK_DBE_HIRAGANA` 注入 + 複数回の IMC write が走る。また `shift_conv_guard_pending` は IME 種別に関係なく立つため、GJI でも idle-conv-check / ime_refresh のポーリングが凍結される。
- **評価**: 短期的には維持せざるをえない。北極星ではない。

### 案2: 案A（Shift 押下時にも `unconfirm()`）

- **欠点**: §1.2 のとおり。長押しで再現し、`ms_ime_gate_give_up` のラッチで BUG-13 保護を殺す。
- **評価**: **却下（再提案禁止）**。ただし「conv を書いたら belief を無効化すべき」という直感は正しく、それは P1 が `actuate_conv_mode` の内側で満たす。案A が失敗するのは、無効化を**投機的書き込みと組み合わせたまま**行うと、無効化後の再確認期限が Shift 保持時間と無関係だからである。**P1 単独導入でもこの副作用は同じく出る**ため、P1 は §5 の deadline 起点是正（INV-9）とセットでなければならない。

### 案3: 確定してから書く（Shift up で単独タップ確定後にのみ conv を触る）

- **内容**: Shift down では何も書かない。`left_shift_tap_candidate` が Shift up 時点で true なら初めて半角英数トグルを起動。チョードなら conv に一切触れない。
- **利点**: P5 適合。時間窓が消える。判定材料は既にある（実装は小さい）。本件のバグは消える。
- **欠点（致命的、実測済み）**: **BUG-25 の設計判断が明示的にこれを否定している。** (1) を撤去すると、「本物の単独タップだけに反応する」新トグルでは Shift+文字キーのチョードを engine が consume する際に MS-IME の誤検知を打ち消す仕組みが無くなり、**BUG-15 の症状がそのまま再発する**。awase が「書かない」を選んでも **MS-IME は勝手に conv を 0 にする**（実測: Shift up の 478ms 後に conv=0x0000 を観測）。無条件復元の安全網が必要なのはこのためである。
- **評価**: **単独では却下**。ただし「entry を消し、exit（復元）だけ残す」非対称な形なら成立しうる。復元は冪等な verify-retry（160ms×4、実測上限 478ms をカバー）で実装済みであり、これは投機ではなく**観測に基づく是正**である。§5 Phase 1 はこの形を採る。

### 案4: 物理 Shift を IME に見せない（P2、**推奨する北極星**）

- **利点**:
  - MS-IME の誤認が**原理的に発生しない**。安全網も投機的書き込みも不要になり、`shift-conv-guard` を削除できる。
  - **親指シフト面が既にこの方式で正常動作しているという、同一リポジトリ内の実証がある。**
  - GJI にも効く。現状 GJI には entry 機構が無いため案1〜3 の系譜は GJI を救えないが、案4 は IME 種別に依存しない。
- **欠点・リスク**:
  - 物理 Shift は OS 修飾キーでもある。Shift+Tab / Ctrl+Shift+T / Shift+矢印 / 直接入力の大文字を壊さずに「シフト面として使われたときだけ飲み込む」必要があり、保留と再生が要る。
  - 再生のレイテンシが体感に出る可能性。
  - StickyKeys との相互作用は BUG-25 から**未検証**のまま。
  - 他プロセスが `GetAsyncKeyState` で Shift 状態を読む場合の不整合（Alt なりすまし対応で同種の問題が既知 — vk 書き換えだけでは不十分で複数箇所の補正が必要だった）。
  - この領域では VK と scan の取り違えが実バグを生んでいる（`lints/no_vk_as_scan`）。合成時の scan 指定に注意（BUG-15: **scan=0 の注入は MS-IME に無視される**）。
- **評価**: **北極星として採用。ただし実機ソーク必須で段階的にしか入れられない。** §5 Phase 3。

### 案5: シフト面の全角出力を conv 非依存 transport へ寄せる（案B の正しい形）

- **内容**: P3+P4。幅が確定した文字は composition を経由せず送る。ただし保証は P4 の単一ゲートが担う。
- **利点**: `shift-conv-guard` が conv=0 を書いていようがいまいが出力が変わらなくなる（**対策の有無から独立に正しくなる**）。全角ラテン `Ｑ〜Ｚ` が既にこの方式で正しく動いている実証がある。`.yab` の `Literal` 宣言を素直に実行するだけでもある。
- **欠点**: ADR-083 の警告。preedit 非空時の挙動が最大の未知数。P4 なしでは案B の欠点がそのまま出る。
- **評価**: **P4 実施後に、preedit が空の場合に限定して採用**。§5 Phase 2。

### 案6: ユーザー設定で無効化する

- **却下（再提案禁止）**。BUG-15 に明記のとおり「Shift キー単独で英数モードに切り替える」は新 IME（Win11 標準 MS-IME）では無効化できない。したがって awase 側カウンターが唯一の防御であり、「設定を切ればよい」という提案は選択肢にならない（再提案しないこと）。

### 比較表

| 案 | 本件を直すか | GJI でも効くか | 時間窓を消すか | 実装量 | 実機検証容易性 | 反転リスク |
|---|---|---|---|---|---|---|
| 1 現状維持 | ✗ | ✗ | ✗ | — | — | — |
| 2 案A | 部分的 | ✗ | ✗ | 小 | 中 | **高（BUG-13 保護を殺す）** |
| 3 確定後に書く | ○ | ✗ | ○ | 小 | 中 | **高（BUG-15 再発が実測済み）** |
| 4 Shift を見せない | ○ | ○ | ○ | **大** | 低（実機ソーク必須） | 中（barrier は既存資産） |
| 5 conv 非依存 transport | ○ | ○ | ○（無関係化） | 中 | 中 | 中（ADR-083 の警告） |
| 6 設定 | ✗ | ✗ | — | — | — | **禁止** |

---

## 4. 不変条件（invariant）

- **INV-1（conv 単一窓口）**: conv-mode を変更する実行経路は `actuate_conv_mode` ただ 1 つ。IMC write（`set_ime_romaji_mode_with_target_async` 等）および conv を変える VK（`VK_DBE_HIRAGANA`/`VK_DBE_ALPHANUMERIC`/`VK_KANA`）の注入は、この関数の内部にのみ出現する。

- **INV-2（書き込みと無効化の不可分性）**: awase が conv-mode を意図的に変更したなら、**その変更を前提とするすべての belief キャッシュは、変更と同一の関数呼び出しの中で無効化されなければならない**。無効化を後続イベント（キー解放、フォーカス変更、タイマー）に依存させてはならない。
  *一般形*: 外部状態 S を awase 自身が書き換えたとき、S から導出されたキャッシュ C の無効化は書き換えと同一トランザクションでなければならない。C の無効化を「別イベントの副作用」に置くと、そのイベントが来ない経路が必ず存在する（ADR-072 と本件の 2 例）。

- **INV-3（投機の禁止）**: 「あとで打ち消す前提の外部状態変更」を新規に追加しない。既存の `shift-conv-guard` entry は例外として明示的に許容するが、**新しい呼び出し元を増やしてはならない**。

- **INV-4（幅の SSOT）**: 出力文字の全角/半角は `.yab` レイアウトが決定する。`YabValue::Literal` は「逐語出力せよ」という宣言であり、送信経路のいかなる段階でもこの宣言を破棄して IME の conv-mode に委譲してはならない。`vk_pair_to_ascii` による全角→半角の写像は**transport 都合の一時的表現**であり、その復元が IME モードに依存する事実を呼び出し元は型で認識していなければならない。

- **INV-5（保証の transport 非依存）**: 順序保証と cold-start 保護は `InjectionMode` の選択より**手前**の単一ゲートに属する。transport 別に独立した defer 機構を新設してはならない。

- **INV-6（観測を偽装しない）**: `actuate_conv_mode` は awase 自身の能動的変更であるから、必ず `InputModeApplied { strategy, result }` で表現する。`InputModeObserved` を使ってはならない。新しい理由には `InputModeApplyStrategy` の新 variant を追加する（既存: `ImmBrokenCorrection`/`PanicReset`/`CacheRestore`/`PostSetOpenEisuReset`/`UserImeOnEisuReset`/`UserHalfWidthAlnumToggle`）。

- **INV-7（IME 種別の非対称を隠さない）**: GJI には conv entry 機構が存在しない（BUG-25）。IME 種別で効果が異なる対策は**効かない側で「効いたことにしない」**。さらに、**entry と restore の IME 種別ゲートは対称でなければならない**（現状は entry のみ MS-IME 限定、restore は無条件という非対称がある）。`half_width_alnum_toggle_active` を GJI で立てると素通しした生ローマ字が GJI のひらがなエンジンに入りかな入力が壊れる（実機確認済み）。

- **INV-8（シフト面の一貫性）**: 同一シフト面に属する文字は同一の送信保証を受けなければならない。`[ローマ字小指シフト]` の `！` が conv 依存で `Ｑ` が conv 非依存、という分裂を許さない。

- **INV-9（deadline の起点）**: 再確認ゲートの期限（`MS_IME_READY_CONFIRM_MS`）は、**belief を無効化した時点**を起点に測る。無効化の原因となった操作の完了時点（Shift 解放等）でも、打鍵時点固定でもない。案A が失敗したのはこの不変条件の欠如による。

- **INV-10（診断と判定の分離）**: 診断目的の読み戻し（現行の entry 150ms verify のようにログのみ出す読み取り）を、後から判定ロジックに流用してはならない。流用する場合は epoch/世代照合を必ず伴わせる（ADR-077/083 の観測フェーズと同じ規律）。

---

## 5. 移行計画

各 Phase は独立してリリース可能で、後の Phase が実機で否定されても前の Phase は残る。

### Phase 0（記録のみ、実機不要）

本 ADR を `docs/adr/084-*.md` として追加、`docs/adr/index.md` に登録。`docs/known-bugs.md` の BUG-47 に「恒久対応方針は ADR-084」と追記。**コード変更なし。**

### Phase 1（P1: conv actuator の集約、中リスク）

1. `actuate_conv_mode` を新設し、既存の conv 書き込み経路（`shift-conv-guard` entry、`kp_restore_kana_from_half_width`、warmup 経路）をすべて通す。
2. 関数内で `ImeModeFsm::unconfirm(reason)` を必ず呼ぶ（INV-2）。
3. **同時に INV-9 を実装**: `ms_ime_gate_defer` の deadline を「`unconfirm` された時点 + `MS_IME_READY_CONFIRM_MS`」に変更する。これをやらないと案A と同じ失敗を踏む。
4. `ms_ime_gate_give_up` のラッチは、**give-up の原因が conv actuation 由来なら次の actuation で解除する**（現状はフォーカス変更／次の `SetOpen(true)` まで解除されない）。
5. entry/restore の IME 種別ゲートを対称化する（INV-7）。

**この Phase 単独で本件（！→!）は直る。** ただし時間窓は残る（P5 違反のまま）。

**実測義務**（`.claude/rules/tuning-constants.md`）: Phase 1 は deadline の起点を変える。「`actuate_conv_mode` の IMC write 完了から、IMC read で目標 conv が確認できるまでの実測 ms」を Windows 実機で計測し、コミット本文に記載すること。既存の 400ms は「IME OFF→ON 遷移」の実測であり、**conv-mode 書き換えの実測ではない**。流用してはならない。

### Phase 2（P4 → P3: 保証の統合と幅の SSOT 化、中リスク）

1. まず P4: `EmitRequest` 単一ゲートを導入し、`defer_vk_if_probe_in_flight` / `unicode_cold_defer` / `mark_composition_cold` をその内側へ移す。**この時点では transport 選択規則も出力バイト列も変えない**（純粋なリファクタとして検証可能にする）。
2. 次に P3: preedit が空である場合に限り、`YabValue::Literal` 由来の幅確定文字を conv 非依存 transport で送る。preedit 非空なら従来どおり。
3. `vk_pair_to_ascii` は削除しない。preedit 非空時の経路として残る。ラウンドトリップ不変条件と `vk_pair_to_ascii_covers_every_build_symbol_to_vk_pair` も維持する。

**先に P4、次に P3。順序を逆にしてはならない**（案B の失敗そのものになる）。

### Phase 3（P2: 物理 Shift の所有、高リスク・実機ソーク必須）

小指シフトを親指シフトと同格に扱う。`shift-conv-guard` を削除できるのはこの Phase 完了後のみ。

**着手前に必須の実機確認**: Shift+Tab / Ctrl+Shift+T / Shift+矢印 / Alt+Shift、直接入力時の大文字、StickyKeys 有効時の挙動、他プロセスの `GetAsyncKeyState` から見た Shift 状態の整合、barrier 保留による体感レイテンシ。

Phase 3 が実機で否定されても Phase 1+2 で本件は解決済みのため**撤退可能**である。この撤退可能性が段階分割の主目的である。

### revert する場合の義務

`.claude/rules/experiment-logging.md` に従い、本 ADR 由来の変更を revert するコミットは本文に **アプリ / IME（種別と状態）/ 再現手順と症状** を必ず記載する。この領域は反転を繰り返しており、「なぜ前回それを捨てたのか」が辿れないことが反転の最大の原因だった。

---

## 6. 強制メカニズム

`.claude/rules/ime-belief-architecture.md` 末尾の3段構えに倣う。同ルールの判断基準に従い、**dylint の新設は「型では防げない意味論的偽装」にのみ投資する**。

### 段1: コンパイラ（最強、可能な限りここへ寄せる）

- **INV-1**: conv を書く低レベル API を `ConvModeActuator` を持つモジュールの **private** にする。`Output` からの直接呼び出しがコンパイルエラーになる（`ForceGuardSet.guards` を private 化して `clear()` を唯一の口にしたのと同じ手法）。
- **INV-2**: 低レベル write 関数が `&mut ImeModeFsm` を引数に要求するようにする（**無効化せずに書くことが型として書けない**）。戻り値は `#[must_use]` の `ConvActuationOutcome`。
- **INV-4**: `vk_pair_to_ascii` の戻り値を `Option<char>` ではなく `Option<ConvDependentAscii>` にし、「この文字は IME が NATIVE のときだけ意図した幅になる」ことを型名で表明する。
- **INV-6**: `InputModeApplyStrategy` に新 variant を追加させる（既存の運用どおり）。

### 段2: dylint（HIR レベル、意味論的偽装の検出のみ）

**新規 dylint crate は原則作らない**（型シグネチャ変更や private 化で防げるものへの dylint 投資は過剰）。既存 crate の拡張のみ:

- `lints/observation_source_guard` を拡張し、**`InputModeApplied { strategy: ConvActuation.., .. }` が `actuate_conv_mode` 以外で構築されたら warning**。`lints/ime_event_guard` が `PanicReset`/`HwndCacheRestored` を designated 関数に限定しているのと同型で、追加コストが小さい。

### 段3: CI テスト（Linux で実行可能、`tests/architecture_guard.rs`）

既存の「テキスト走査による出現数固定」手法に倣う:

1. `conv_mutation_call_sites_are_accounted_for` — IMC write / conv 系 VK 注入の出現箇所数を固定。**INV-1**
2. `conv_actuation_always_unconfirms_belief` — `actuate_conv_mode` の本体に `unconfirm` 呼び出しが出現することを固定。**INV-2**
3. `no_new_speculative_conv_prewrite` — `shift_conv_guard` 系シンボルの参照箇所数を固定。**INV-3**
4. `defer_mechanisms_are_transport_independent` — `unicode_cold_defer` / `defer_vk_if_probe_in_flight` の参照が単一ゲート内に限られることを固定。**INV-5**
5. `conv_actuation_entry_and_restore_are_ime_kind_symmetric` — entry と restore の IME 種別分岐が対称であることを固定。**INV-7**

`tests/layer_boundary_guard.rs` の module doc の警告（「ルールを弱めないこと」）は本 ADR のガードにもそのまま適用する。

### 段4: golden テスト

`.claude/rules/fix-requires-evidence.md` の要求（回帰テストか known-bugs 記録の少なくとも一方）を満たす。

- `tests/ime_key_sequence_golden.rs` に**シフト面の各文字に対する送信バイト列**の golden を追加。`！` が conv 依存経路と非依存経路のどちらを通るかが期待値として固定され、transport を変える変更が CI で可視化される。**INV-8**
- `tests/golden_scenarios.rs` に「conv actuation → belief 無効化 → 次の送信がゲートを通る」シナリオを追加（シナリオ15 が `half_width_alnum_toggle` の belief 遷移を既に固定しているのでその隣に置ける）。**INV-2/INV-9**

> **注意**: `kp_stage_shift_conv_guard` のタップ/チョード判定そのものは Windows 実機フック依存で自動テスト不可（BUG-25 に明記）。**Phase 3 は golden で守れない**。この Phase に限り、機械的強制の代わりに known-bugs.md への記録と実機ソークが防衛線になることを受け入れる。

---

## 7. 未解決の論点

1. **Phase 3（P2）の実現可能性が最大の未知数。** 物理 Shift を保留して必要時に再生する方式が NICOLA の高速打鍵で体感可能なレイテンシを生まないか未実測。既存の `InputBarrier`/deferred VK queue がそのまま使えるかも未調査。

2. **preedit が空かどうかの判定手段。** P3 の条件は `capture_composition_snapshot`（ADR-083 で診断配線済み、判定には未使用）に依存する可能性が高い。ADR-083 は**この関数が IMM32 互換アプリで何を返すか実機測定するフェーズを GO としたばかり**である。**その測定結果が出るまで Phase 2 の P3 部分に着手してはならない。** ADR-083 の観測フェーズが本 ADR の前提条件になっている。なお INV-10 により、この診断配線を判定に昇格させる際は epoch 照合を伴わせること。

3. **GJI の conv entry。** 次の候補は `ITfLangBarItemMgr`/`ITfLangBarItemButton` 経由の言語バーボタン起動（mozc の `TipTextService` が実登録しており本物のクリックと同じ `SwitchInputModeAsync` を通るはず）だが**未着手・未検証**。案4（P2）が成立すれば GJI の entry 自体が不要になるため、**P2 の検証を先に行い、失敗した場合にのみこちらへ投資する**。他社 IME の私的 IPC を覗く/模倣する方向は取らない（公開 Win32 API の範囲で解決する）。

4. **`MS_IME_READY_CONFIRM_MS` の起点変更に伴う再測定。** §5 Phase 1 の実測義務。conv 書き換えの反映時間は IME ON/OFF 遷移の反映時間とは別物であり流用してはならない。

5. **StickyKeys との意味論的競合。** StickyKeys 自体が「Shift 単独タップ」を検出してラッチする。BUG-25 から未検証のまま繰り越されている。P2 では物理 Shift の扱いを根本的に変えるためここで初めて実害が出る可能性がある。

6. **ADR-078 との統合順序。** P1 は ADR-078 の `EffectiveMode`/`ModeConstraint` を前提にすると綺麗に書けるが、ADR-078 は Phase 1a（`needs_conv_restore_write`/`mark_conv_restore_written` による増幅ループ抑止）のみ実装済みで型分割は未実装。**本 ADR は P1 の先行を推奨する**（`actuate_conv_mode` は ADR-078 の有無にかかわらず必要であり、むしろ ADR-078 Phase 1 の着手点を 1 つ具体化する）。特に ADR-078 は `ModeConstraint` を「`key_pipeline.rs` の Shift 解放復元が場当たりにやっていたことの一般化」と位置づけており、本 ADR の P1/P2 と同じ対象を別角度から扱っている。

7. **小指シフト面のレイアウト設計そのものの再検討。** `[ローマ字小指シフト]` が全セル `Literal`（全角）であるという事実は、この面が「IME の変換を経由する必要がまったく無い面」であることを意味する。この面を丸ごと conv 非依存と宣言できるなら、P3 の preedit 判定すら不要になる可能性がある。ただしユーザーがこの面にかなを割り当てる自由を奪ってよいかは別途判断が要る。**`.yab` の面ごと、あるいはセルごと（`Literal` かどうか）に「conv 依存/非依存」を導出する**案は検討に値するが、本 ADR では決定しない。

8. **`build_symbol_to_vk` の多対一崩壊。** `＂` と `"`、`＇` と `'`、`～` と `~` が同一の `(vk, shift)` に落ちるため、`vk_pair_to_ascii` は多対一の逆写像である。全角/半角の区別は意図的に破棄され IME の変換モードに委譲されている。P3 はこの崩壊を `Literal` 宣言の保持によって回避するが、**移行期間中は同じ VK に 2 つの意図が乗る**ことを実装者は意識する必要がある。

---

## 8. 関連

- ADR-064: `ConvModePolicy` による conv mutation ゲート（許可の明示化。本 ADR は実行窓口の集約へ拡張）
- ADR-072: `conv_mode_authority` を apply 完了ごとに再同期（**遷移エッジ依存の誤りの先例。本件は同型の 2 例目**）
- ADR-078: IME mode belief の Desired/Effective/Constraint 分割（P1 の belief 側受け皿、Phase 1a のみ実装）
- ADR-080/082: actuation ライフサイクルとジャーナル（`actuate_conv_mode` の記録先）
- ADR-083: `InjectionMode` per-VK 統一の検討（**NO-GO**。P4 は統一を提案せず、観測フェーズの結果を Phase 2 の前提とする）
- ADR-048: アトミックバッチ送信（P2 の `VK_SHIFT` 合成手法）
- ADR-071: deferred VK queue の所有権（P2 の barrier 実装資産）
- ADR-033: `AppImeProfile`（`Standard`/`Imm32Unavailable`/`TsfNative`）
- `docs/known-bugs.md`: BUG-13（MS-IME cold-start リテラル化・confirm-then-transmit）、BUG-15（Shift 解放で英数落ち、二重オーナー構造）、BUG-25（持続トグル、安全網撤去不可の設計判断、GJI entry 撤回）、BUG-47（記号の cold-start 半角化、`vk_pair_to_ascii` 追補）、BUG-01/02/03（cold-start リテラル化ファミリ）
- `.claude/rules/ime-belief-architecture.md` / `experiment-logging.md` / `tuning-constants.md` / `fix-requires-evidence.md`
- `lints/no_vk_as_scan`（VK と scan の取り違え検出。P2 の Shift 合成で関連）
