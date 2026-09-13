# ADR-169 批判的レビュー（Opus、読み取り専用）

対象: `docs/adr/169-journal-key-input-repeat-coalescing.md`（起草・未実装）
日付: 2026-09-13
裏取りに使ったコード: `crates/awase-windows/src/journal.rs`,
`journal_policy.rs`, `runtime/key_pipeline.rs`, `hook.rs`, `bug_report.rs`,
`lib.rs`, `tests/architecture_guard.rs`, `docs/adr/096-*.md`,
`docs/bug-reports-triage.md`

## 総評

**方向（auto-repeat を1件に畳む）は妥当だが、ADR が主張している「なぜ効くのか」の
因果が今回の report の実データと合っていない。** 具体的には、この report で失われた
184 件は「リングバッファから溢れた」ものではなく「リングには残っていたのに byte 予算で
落とされた」ものである（Blocker B1）。その結果、決定3（byte 予算の引き上げを却下）が、
今回の report で唯一「184 件を実際に取り戻せた」レバーを、誤った根拠で却下している。

加えて、決定1の判定基準「同一 vk かつ間に key-up なし＝物理的に auto-repeat 以外
あり得ない」は、**journal が key-up を必ず見ているという前提**の上に立っており、その前提は
本リポジトリのコード（hook 層の swallow 6経路、key_pipeline の早期 return 2経路）と
ADR-096 自身の既知の限界記述に反する（B2）。また畳み込みで捨てる 4 フィールドが
「auto-repeat 中は不変」という主張も、修飾キー以外では成立しない（B3）。

実装場所の議論（質問2）は、既存 API を見ると「key_pipeline 側 or journal 側」という
二択自体が成立しない（M1）。テスト方針は Linux で 1 件も実行されない形になっている（M5）。
既存テスト・再生ハーネスへの後方互換影響は**ほぼ無い**（m3、質問3への回答）。

優先度: B1〜B3 は ADR 本文の書き直しが要る。M1〜M5 は実装前に決めておかないと
実装者が手戻りする。m1〜m5 は記述の正確性。

---

## Blocker

### B1. 決定3の却下根拠が、今回の report の損失の実体と逆になっている

ADR 背景（168:26-29）は

> `key_input` レーン自体がインメモリのリングバッファ（容量512）で、上限に達すると
> 古いエントリから無条件に消える（略）byte 予算をいくら引き上げても、この段階で
> 既に消えたエントリは戻らない

と書き、これを根拠に決定3（`LOG_EXCERPT_MAX_BYTES` 引き上げは却下）へ繋げている。
しかし `dropped_key_input` の定義を読むと、この数字はリング退避を **1 件も数えていない**。

`journal.rs:1432-1450` の `dropped_by_lane()`:

```rust
fn dropped_by_lane(serialized: &[SerializedEnvelope], selected: &[usize]) -> [(LaneKind, usize); 4] {
    // total は serialized（= entries_by_seq() = 「ダンプ時点でレーンに入っていた分」）の集計
    // emitted は select_tail_within_budget が選んだ分の集計
    (LaneKind::KeyInput, total[3].1.saturating_sub(emitted[3].1)),
}
```

`serialized` の元は `to_json_capped()` 冒頭の `self.entries_by_seq()`
（`journal.rs:1265-1270`）であり、**リングから既に消えたエントリはここに存在しない**。
つまり `dropped_key_input: 184` は「ダンプ時点で RAM に残っていた 512 件のうち、
200KiB の予算に入りきらず捨てた 184 件」という意味である。

帰結:

1. この 184 件は **byte 予算を上げれば実際に戻ってきた**。決定3 が却下したレバーは、
   今回の report に限れば唯一の実証済み回収手段である。「byte 予算をいくら上げても
   戻らない」という背景の文は、今回の損失には当てはまらない。
2. 逆に、`KEY_INPUT_LANE_CAPACITY` を 512→2048 に上げても、**添付される件数は増えない**。
   emit を律速しているのは byte 予算側であり（512 件中 328 件しか通っていない＝
   予算が先に尽きている）、リングを大きくしても `select_tail_within_budget` は
   新しい側から同じバイト数ぶんしか選ばない。決定2 の「容量を上げるかどうか」は、
   実は今回の症状に対してほぼ無意味なレバーである（結論「上げない」は正しいが、
   理由が違う）。
3. 決定1（畳み込み）が効く理由も書き換えが要る。効くのは「リング枯渇の解消」ではなく、
   **同じ 200KiB の emit 窓が、より多くの異なる打鍵をカバーするようになる**（1 回の
   Ctrl 長押しが 50 エントリ＝50 件分のバイトを食っていたのが 1 件になる）という
   圧縮効果である。この説明なら決定1 の価値は下がらず、かつ事実と整合する。

**要求**: 背景（168:22-29）と決定3（168:107-124）を、`dropped_by_lane` の実装に
基づいて書き直すこと。特に「ADR-096 の byte 予算優先度制御は正しく機能していた
（dropped_state/timing/actuation は全て 0）」→「だから前段に別のロスがある」という
推論は成立しない。他レーンの drop が 0 なのは単にそれらが小さいからであり、
KeyInput が 184 件落ちていることこそ「byte 予算段でまだ大量に落ちている」証拠である。

参考: リング退避が起きていたこと自体は `328 + 184 = 512 = capacity` から推測できる
（レーンが満杯だったのは事実）。しかし「満杯だった」ことと「今回失われた証拠が
リング退避で失われた」ことは別で、後者は現状のデータでは示せていない（→ M3 の
計器不在に直結する）。

### B2. 「間に key-up が無い＝物理的に auto-repeat」は、journal が key-up を必ず見る前提に依存しており、その前提が成立しない

ADR 168:36-39 / 85-90 は、この条件を「誤判定の余地がない」根拠として繰り返している。
しかし journal に記録されるのは `kp_run_inner` に到達したイベントだけで、key-up は
以下の経路で **journal に現れないまま消える**。

hook 層（`crates/awase-windows/src/hook.rs`、いずれも `key_pipeline` へ渡す前に return）:

| 行 | 経路 | 影響 |
| --- | --- | --- |
| `hook.rs:1049-1051` | 自己注入キー（`INJECTED_MARKER`）はそのまま OS へ | awase 自身の送信は不可視（これは正しい） |
| `hook.rs:1094-1096` | `focus_app_disabled`（`disable_apps`、既定 `mstsc.exe`、BUG-78） | **その間の down/up が丸ごと不可視** |
| `hook.rs:1141` / `1154` | foreign-injected `VK_KANA` / `Alt+VK_KANA` の swallow（BUG-08/62） | down も up も不可視 |
| `hook.rs:1203` | `VK_DBE_ROMAN`/`VK_DBE_NOROMAN` の swallow（BUG-62 追補4） | 同上 |
| `hook.rs:1218` / `1330` | Alt なりすまし経路 | 条件により別 VK へ書き換え |

ADR-096 自身が既知の限界としてこう書いている（`docs/adr/096-*.md:297-300`）:

> hook 層で swallow される外部注入キー（`VK_KANA`/`VK_DBE_ROMAN`/`VK_DBE_NOROMAN` 等）は
> **原理的に journal に現れない**

さらに `hook.rs:1012-1016` には「Alt down はあるが Alt up が（ログにすら）現れず
KeyUp だけ現れる現象が2回連続で観測された」という実測の記録がある。key-up の
到達は経験的にも保証されていない。

`key_pipeline.rs` 側にも journal 到達前の早期 return が 2 つある:

- `key_pipeline.rs:269-276` `try_hold_key`（TsfGate PendingWarmup）: `Consumed` で return。
  保留キーは後で `OUTPUT_PENDING_QUEUE` 経由で再処理されるため、**journal 上の順序が
  物理順と一致しない**。
- `key_pipeline.rs:410-423` IME OFF rescue の defer: `set_ime_off_rescue_pending(event)`
  して return（journal 記録は 486 行目なので未到達）。保留イベントは後で
  `key_pipeline.rs:300` の nested `kp_run_inner` で記録されるため、やはり順序が入れ替わる。

具体的な失敗シナリオ:

1. **RDP/disable_apps 復帰**: `mstsc.exe` にフォーカス中は全イベントが不可視。
   直前に `A` を押した状態のまま `mstsc` へ移り、そこで離す → key-up は不可視。
   数分後に戻って `A` を押す → 「直近 KeyInput は同一 vk・is_down:true」が成立し、
   **数分前の古いエントリ（古い seq）に畳み込まれる**。新しい打鍵が古い時刻位置へ
   消える、という診断上最悪の壊れ方をする。
2. **foreign-injected の連続 down**: PowerToys Mouse Without Borders（BUG-90、
   issue #136、この report の環境にも `競合ソフト: PowerToys` と記録がある、
   `docs/bug-reports-triage.md:79`）、AutoHotkey、VNC/ソフト KVM は down/up の対を
   保証しない。`hook.rs:1053-1061`（BUG-14）で foreign-injected の swallow は撤回済み
   なので、**これらは journal に到達する**。injected 由来の連続 down を auto-repeat と
   みなして畳むと、BUG-90/issue #136 系（`injected` が決め手のバグ、ADR-096 が
   わざわざ `KeyEventSummary.injected` を追加した理由そのもの）の証拠を潰す。
3. **キーボード2台/マクロパッド**: A が `a` を押しっぱなし（repeat 中）、B が同じ `a` を
   1回叩く。B の down は repeat 列の途中に紛れ、up が来る前に畳み込まれる＝実打鍵が 1 件消える。

**推奨（B2 の解決策、質問1・2 をまとめて解く）**: 「auto-repeat か」を journal 側で
推測せず、hook 層の既存 SSOT を使う。`hook.rs:73` の
`physical_key_state: [AtomicBool; 256]` は `hook.rs:1065-1070` で
**非 injected イベントのみ**更新されており、これは既に `alt_impersonation.rs:49-50` が
「新規押下か auto-repeat か」の判定に使っている実績のある機構である（`was_down`）。

- `hook.rs:1066-1068` の `slot.store(is_keydown, ...)` を `slot.swap(is_keydown, ...)` に
  変え、`was_down` を `RawKeyEvent`（`src/types.rs:263-`、既に `injected` /
  `modifier_snapshot` / `key_classification` 等の「プラットフォーム層が事前分類した事実」を
  運んでいる）に載せ、`KeyEventSummary`（`journal.rs:44-55`）へ写す。
- この設計だと injected は `physical_key_state` を更新しないので `was_down=false` →
  **injected 連打は自動的に畳み込み対象外**になり、シナリオ 2 が構造的に消える。
- `reset_physical_key_state()`（`hook.rs:455`、スタック修飾キー復旧）後は `was_down=false` に
  倒れる＝「畳まない」側に落ちるので安全側。
- ADR-019（core の OS 非依存）との関係: `was_down` は VK/scan の生値ではなく事前分類済みの
  bool なので `injected` と同カテゴリだが、`src/types.rs` に足す以上 core の変更になる。
  core に足したくないなら Windows 側で `RawKeyEvent` と並走させる選択肢も要検討（ADR に明記を）。

### B3. 畳み込みで共有される 4 フィールドは「auto-repeat 中は不変」ではない

ADR 168:74-77 は

> `state_before`/`state_after`/`decision`/`physical` は auto-repeat 中は不変
> （`state_before == state_after == "Idle"` のケースが典型）なので実害なく共有できる

と断言している。これは Ctrl 長押しという **1 例**でしか検証されていない。`KeyInput` の
実フィールド（`journal.rs:208-214`）を見ると:

- `state_before`/`state_after` は `engine.debug_state_label()`（`src/engine/engine.rs:778`）の
  `String`。文字キーの auto-repeat は NICOLA FSM を通る（`key_pipeline.rs:426`
  `self.engine.on_input(event, &ctx)`）ため、`PendingChar`/`PendingThumb`・タイマー状態が
  repeat ごとに進みうる。同一 vk の repeat でも `decision` が
  `Consume`↔`PassThrough` で変わりうる。
- `physical` は `PhysicalKeyDisposition::plan()`（`key_pipeline.rs:455-471`）の結果で、
  `profile` / `shadow_toggled` / `f2_warmup_owned()` / `active_ime_kind` /
  `half_width_alnum_toggle_active` を見る。`half_width_alnum_toggle_before` は
  BUG-116/ADR-137 のガード用に「同一イベント処理中に false へ落ちうる」と
  `key_pipeline.rs:320-325` が明記している値であり、repeat 間で変わりうる。
- `KeyEventSummary.alt/ctrl/shift`（`journal.rs:51-53`）は repeat 中に変化する。
  典型: `a` を押しっぱなしのまま Shift を押す → 同一 vk・両方 down・key-up 無しのまま
  `shift: false→true`。畳むと「Shift を押した瞬間」が journal から消える。

**推奨**: 畳み込み条件に「payload が完全一致（vk/scan/key_class/injected/alt/ctrl/shift/
state_before/state_after/decision/physical）」を AND で加える。不一致なら新規エントリに割る。
Ctrl 長押し 50 件のような本来の標的はこの条件でも問題なく畳めるので、決定1 の効果は
ほぼ落ちない。ADR の「実害なく共有できる」は削除し、この一致条件を決定文に格上げすること。

---

## Major

### M1. 「key_pipeline 側 vs journal 側」は選択肢になっていない（質問2への回答）

ADR 168:69-73 は実装場所を `UnifiedJournal` か呼び出し側かの二択のように書くが、
既存 API を見ると **どちらを選んでも `journal.rs` に新 API が要る**。

- `UnifiedJournal` の公開口は `record()`（`journal.rs:1224-1229`）と
  `absorb()`（`journal.rs:1238-1247`）だけで、**直近エントリを読む/書き換える API は無い**。
- 実体の `JournalLane`（`journal.rs:488-491`）は private、`buffer: VecDeque<JournalEnvelope>` も
  private。`JournalLanes` も private。key_pipeline から末尾エントリを mutate する手段は無い。
- したがって最低限 `UnifiedJournal::record_key_input_coalescing(...)`（または
  「末尾が同一なら repeat_count++ して Ok(()) を返す」形の API）を journal.rs に足すことになる。
  ADR にはこの API 追加が一切書かれておらず、レビュー時に「軽微な変更」と誤認されうる。

**`absorb()` 側で畳むのは避けること**（2つの理由）:

1. `absorb` は `key_pipeline.rs:510-512` の `drain_journal_entries()` 経由で **遅延 envelope**
   （`JournalStamper` で先に採番済み、ADR-096 B-4）も受け取る。`JournalLane::push`
   （`journal.rs:493-507`）は `rposition` で seq 順に挿入し直す設計なので、
   「レーンの末尾＝直前に記録した KeyInput」という前提が成立しない。
2. `absorb` は ADR-139 決定4 の tracing fan-out 地点であり、doc コメント
   （`journal.rs:1230-1237`）が **「レーン容量超過で `JournalLane::push` が黙って捨てる
   エントリも tracing 側には出力される（意図的）」** と明記している。

この 2 点目は ADR の「挙動には一切影響しない」（168:78-80）に対する反例でもある:
**key_pipeline で `record()` 自体を省くと、今日は `app_log_excerpt` に残っている
auto-repeat の痕跡が消える**（journal と app_log の 2 系統のうち片方を失う）。
`emit_tracing` は通して journal レーンにだけ畳む、という形にすれば既存方針と整合する。
ADR にこの設計意図を明記すること。

なお `JournalEntry::KeyInput` の構築点は `key_pipeline.rs:486` の **1 箇所のみ**
（`grep` で確認）なので、判定に必要な直近状態を呼び出し側に置くこと自体は可能。
ただし B2 の通り、その状態は hook 層の `physical_key_state` の方が正しい情報源である。

### M2. `elapsed_ms` の上書きは seq と時刻の対応を壊す

ADR 168:74-75 は「末尾 `elapsed_ms` の更新」を求めるが、`JournalEnvelope`
（`journal.rs:440-445`）の `seq` は据え置きになる。出力は `entries_by_seq()` で
seq 昇順に並ぶため、**seq N のエントリの `elapsed_ms` が seq N+5 より新しい**という
非単調が発生する。これは:

- ADR-096 が「3系統の時間軸（`elapsed_ms`/`tick_ms`/`timestamp_us`）は統一せず
  `ClockAnchor` で相互変換可能にする」（`docs/adr/096-*.md:268-272`）とした読み方を壊す。
- `KeyEventSummary.timestamp_us`（初回のまま）と `elapsed_ms`（最後）が同一エントリ内で
  別の打鍵を指す、内部矛盾したレコードになる。

**推奨**: envelope は一切触らず、`KeyInput` 側に `repeat_count: u32` と
`last_elapsed_ms`（または `last_timestamp_us`）をフィールドとして持たせる。
「いつ押し始めていつ離れたか」は診断上そこそこ重要なので、count だけにしないこと。

### M3. 決定2 の「効果を実測してから」に、測る計器が存在しない

決定2（168:92-105）は「畳み込み後もなお溢れた件数を根拠に再検討する」と書くが、
**リング退避の件数はどこにも記録されていない**。

- `JournalLane::push`（`journal.rs:497-502`）の `self.buffer.pop_front()` は無カウント。
- 同 `493-497` の「full なレーンに、front より古い遅延 envelope が来たら黙って捨てる」も無カウント。
- `DumpTruncated.dropped_key_input` は B1 の通り byte 予算段の指標。

ADR-096 も同じ先送りをしている（`docs/adr/096-*.md:239-241`「レーン容量・予備枠は
実測前のため tuning-constants に従い変更せず。実機測定3項目を残した」、
同 `273-275`「実運用での発火頻度を見て調整が要る」）。計器を作らないまま
「実測したら再検討」を繰り返すと、次の report でも同じ推測（328+184=512 という
算術からの逆算）しかできない。

**推奨**: 本 ADR で `JournalLane` にレーン単位の `evicted: usize` /
`stale_dropped: usize` を持たせ、`DumpTruncated` に 4 レーン分のフィールドを追加する。
これは決定1 より小さい変更で、かつ決定1 の効果測定（畳み込みで evicted が何件減ったか）と
決定2 の判断根拠を同時に作る。tuning-constants ルールの精神（実測してから動かす）を
本当に守るなら、計器の方が先。

### M4. 決定2 の tuning-constants 類推は成立しない（結論は別の理由で正しい）

`tuning-constants.md` が禁じているのは「効かないから増やす」型のタイミング定数の
エスカレーションで、その理由は **レイテンシ悪化と別の spurious を誘発するから**。
リングバッファ容量は挙動を一切変えず、コストはメモリのみである
（`JournalEntry` は 264 バイト固定、`journal.rs:426` の const assert で固定されている。
`String` 2 本のヒープを足しても 1 件 ~300B、512→2048 で **+約 0.5MB**）。
「実測なしのエスカレーション禁止」の精神をそのまま持ち込むのは筋が違う。

一方で B1 の通り、**容量を上げても添付件数は増えない**（emit を律速しているのは
byte 予算）。決定2 の結論「据え置き」は正しいが、根拠は「tuning-constants の精神」ではなく
「emit の律速が別の層にあるため容量増加は今回の症状に効かない」と書くべき。

### M5. テスト方針の「Linux 上の `cargo test --lib` で実行可能」は成立しない

ADR 168:143-145 は `journal.rs` または `key_pipeline.rs` の `#[cfg(test)]` として
Linux で走らせると書いているが、`crates/awase-windows/src/lib.rs` を見ると:

- `lib.rs:57-58`: `#[cfg(windows)] pub mod journal;`
- `lib.rs:76-77`: `#[cfg(windows)] pub mod runtime;`

いずれも Windows ゲート配下であり、CLAUDE.md が警告している
「native-Linux テストバイナリに**そもそも存在しない**（`cargo test --list` にも出ない、
エラーもスキップ表示も出ない）」ケースそのものになる。

**推奨**: 判定を `journal_policy.rs`（`lib.rs:29` で ungated、ADR-096 が
「Linux CI で回帰テストできるように」という理由で新設したモジュール、
`docs/adr/096-*.md:202-204`）の純粋関数に切り出す。例:

```rust
// journal_policy.rs
pub struct KeyInputIdentity { /* vk, is_down, was_down, modifiers, class, decision, physical, states */ }
pub enum CoalesceOutcome { NewEntry, MergeIntoPrevious }
pub fn key_input_coalesce(prev: Option<&KeyInputIdentity>, next: &KeyInputIdentity) -> CoalesceOutcome;
```

これなら B3 の「payload 完全一致」条件も Linux 上のユニットテストで固定できる。
既存の `probe_tick_is_notable` / `literal_detect_is_notable` と同じ形で、
レイヤー境界にも沿う。

またテスト項目のうち「`DumpTruncated.dropped_key_input` が畳み込み前より減ること」
（168:153-154）は、M3/B1 の通り測っている対象が違う（減るのは事実だが、それは
byte 予算段の話であり、ADR が主張しているリング枯渇の改善の証拠にはならない）。
M3 の evicted カウンタを入れるなら、そちらを検証項目にすべき。

---

## Minor / 記述の正確性

### m1. `size_of::<JournalEntry>() == 264` の const assert に当たりうる

`journal.rs:426`:

```rust
const _: () = assert!(size_of::<JournalEntry>() == 264);
```

ADR-163 Part D の `/code-review` 指摘で入った回帰ガード。`KeyInput` に
`repeat_count`/`last_elapsed_ms` を足しても、現状 `KeyInput` は最大 variant では
ないと見積もられる（`KeyEventSummary` + `String`×2 + 小 enum×2 ≒ 140B 前後）ため
おそらく通るが、**通らなければコンパイルエラーになる**。ADR の実装手順に
`cargo check --target x86_64-pc-windows-msvc -p awase-windows --lib` を明記すること
（この sandbox では link.exe が無いため `--no-run` でも失敗する点は CLAUDE.md の通り）。

### m2. `architecture_guard.rs` の emit_tracing ガードに引っかかる書き方がある

`crates/awase-windows/tests/architecture_guard.rs:4740`
`journal_emit_tracing_has_no_debug_display_sigils_or_wildcards` は、
`emit_tracing` ブロック内の `_ =>` / `.. =>` を禁止している。`KeyInput` の
match アーム（`journal.rs:783-789`）は 5 フィールドを明示分解しているので、
フィールド追加時に `..` で済ませると **このテストが落ちる**（意図通りの挙動）。
フィールドを足したら `emit_tracing` にも明示的に足すこと。`?`/`%` シギル禁止も同様
（`repeat_count` は数値なのでそのまま `repeat_count,` で載る）。

### m3. 既存テスト・再生ハーネスへの後方互換影響は実質ゼロ（質問3への回答）

調査結果:

- `crates/awase-windows/tests/` 配下と `crates/awase-windows/tests/journals/` 配下に
  文字列 `KeyInput` の出現は **0 件**。golden_scenarios / journal_replay /
  drift_correction_replay / architecture_guard のいずれも `KeyInput` の JSON 形状に
  依存していない。
- `journal_replay.rs:1-27` の doc が明記する通り、あのハーネスが読むのは
  `ConvClassifyCall` → `ConvClassifyFixture` のみ。
- ADR-159/163 の actuation decision 再生（`tests/journals/actuation_decision/*.json`）は
  `ActuationDecisionRecord` 専用で、KeyInput を含まない。
- `JournalEntry` は `Serialize` のみ（`Deserialize` を derive していない）ため、
  JSON にフィールドが増えても読み手側の破壊は起きない。R2 側の worker
  （`services/report-worker/src/index.ts`）もサイズ検証のみでスキーマ検証をしていない。
- 影響を受けるのは `journal.rs:1671-1689` の `make_key_input_entry()`（同ファイル内の
  `#[cfg(test)]` ヘルパー）だけ。ただし m5/M5 の通り、これは Windows ターゲットでしか
  コンパイルされない。

ADR にこの調査結果を 1 行入れておくと、実装者が後方互換の心配で時間を使わずに済む。

### m4. 背景の数値の一部は repo 内から裏取りできない

- 「Ctrl 約1秒ホールドで約50件」「512件中約1割」（168:33-39）は report 実物依存。
  `docs/bug-reports-triage.md:79` が記録しているのは `dropped_key_input=184`
  （全512件中約36%）のみ。
- 「`emitted_entries` のうち KeyInput 分 328」（168:19）も同様。

ADR 本文で「report `01M2CYZ0SFQH3560A0V1YSGKHP` の journal 実物から読み取った値」と
出典を明示するか、該当 seq 範囲を引用しておくこと（後から検証する人が repo だけでは
確認できない主張が混ざると、`feedback_prefer_current_head_verification_over_pr_diff_trust`
と同型の問題になる）。

### m5. 「検討したが採らなかった案」の引用が実在の記録と一致しない

168:128-134 は Passthrough 一律除外の却下理由として
「BUG-105（3鍵仲裁）・`CtrlMuhenkanImeOff` chord（BUG-49関連）等で Passthrough 分類の
KeyInput エントリが実際に決め手として使われた実績がある」と書くが:

- `docs/known-bugs/BUG-105.md` の決め手は `compute_prefer_char1()` の早期 return と
  char/thumb のタイミングであり、**Passthrough 分類のキーではない**（char キーと親指キーは
  `KeyClassification::Char`/`LeftThumb`/`RightThumb`）。
- `docs/known-bugs/BUG-049.md` は小指シフト面の全角記号半角化（`shift-conv-guard`）で、
  `CtrlMuhenkanImeOff` chord とは別件。

結論（Passthrough は残す）には賛成だが、根拠は差し替えるべき。より強い根拠は:

- 本 ADR の背景そのもの — **Ctrl 押下は Passthrough 分類**であり、それが「ノイズの主因」
  かつ「BUG-116/ADR-137 や issue #136 のような修飾キー絡みの証拠」でもある。
  だからこそ「除外」ではなく「畳み込み」が正解、という筋の方が自明で強い。
- `key_pipeline.rs:408-423` の Ctrl+無変換 IME OFF 救済（`ctrl_consumed_since_down`）は
  Ctrl の down/up 列そのものが診断対象。

### m6. 決定3 は「LOG_EXCERPT_MAX_BYTES を上げる」しか検討しておらず、偽の二択になっている

B1 で述べた通り byte 予算側にこそ回収余地がある。ADR が検討していない、
`MAX_BODY_BYTES`（512KiB、`bug_report.rs:32` / `services/report-worker/src/index.ts:7`）を
一切触らずに済む選択肢が少なくとも 3 つある:

1. **journal/app_log の非対称配分**: 現状は両方 `LOG_EXCERPT_MAX_BYTES`（200KiB、
   `bug_report.rs:27`）。`build_payload_with_log_budget`（`bug_report.rs:542-556`）は
   既に予算を引数で受けられるので、journal 250KiB / app_log 150KiB のような
   配分変更は合計を変えずに実現できる（回帰テスト
   `full_size_journal_and_app_log_fit_within_max_body_bytes_without_shrinking`
   〈`bug_report.rs:1194`〉が守っているのは**合計**が収まること）。
2. **`RESERVED_PERCENT` の見直し**（`journal_policy.rs:121-126`、KeyInput は 15%）。
   ただし leftover pass で KeyInput は最後（`journal_policy.rs:184-191`）に回るため、
   今回のように予算全体が枯渇している状況では効果は限定的。要実測。
3. **1 エントリあたりのバイト削減**（最も効く可能性が高い）: `state_before`/`state_after` は
   `String`（`journal.rs:210-211`、`debug_state_label()` 由来）で、KeyInput 1 件の
   JSON バイトの相当部分を占める。`state_before == state_after` のとき片方を
   `skip_serializing_if` で省く、ラベルを短縮する、`scan_code`/`timestamp_us` を
   必要時のみにする、等で emit 窓は直接広がる。決定1 の畳み込みと同じ「1 件あたりを
   小さくする」方向であり、auto-repeat 以外のエントリにも効く。

決定3 を維持するなら、「`LOG_EXCERPT_MAX_BYTES` の一律引き上げは却下」に限定し、
上記 3 案は別 ADR ではなく本 ADR の「未検討／次の候補」として名前だけでも残すこと。

---

## 実装前に決めておくべきこと（チェックリスト）

1. B1 を受けて、ADR の因果説明（背景・決定3）を書き直す。決定1 の効能は
   「リング枯渇の解消」ではなく「emit 窓の圧縮」。
2. B2 を受けて、auto-repeat 判定の情報源を `hook.rs` の `physical_key_state`
   （`swap` で `was_down` を取り、`RawKeyEvent` に載せる）に変える。journal 側の
   「直近 vk/is_down」記憶は採用しない。
3. B3 を受けて、畳み込み条件に payload 完全一致を加える。
4. M1 を受けて、`UnifiedJournal` に追加する API を ADR に明記する。
   `emit_tracing` は repeat でも従来通り出す（app_log 側の証拠を消さない）。
5. M2 を受けて、`envelope.elapsed_ms` は書き換えず `repeat_count` +
   `last_elapsed_ms` を持つ。
6. M3 を受けて、レーン単位の eviction カウンタを追加し `DumpTruncated` に出す
   （決定2 の前提条件）。
7. M5 を受けて、判定ロジックを `journal_policy.rs` の純粋関数に置き、
   Linux CI で回帰テストする。
8. m1/m2 を受けて、`size_of` const assert と `emit_tracing` ガードの更新を
   実装手順に含める。

---

# round2（2026-09-13、改訂版 ADR-169 の再確認）

改訂版を読み、round1 の Blocker 3件 / Major 5件 / Minor 6件がコードと整合する形で
塞がれているかを再度コードで裏取りした。

## 結論

**round1 の指摘はすべて実質的に塞がれている。** 特に決定1の判定情報源の差し替え
（`physical_key_state` の `swap` 化 → `RawKeyEvent.was_down`）は、前提としている
「hook がフック内の早期 return より前にビットを更新している」がコード上で正しいことを
確認できた（下記 V1）。新規に見つかった問題は **Major 2件・Minor 5件**で、いずれも
方針の変更ではなく「決定1-b が実際に測れる形になっているか」と「新設した不変条件が
守られ続けるか」という実装詰めの話である。

## 改訂版の主張のうち、コードで裏取りできたもの（再指摘なし）

- **V1（決定1の前提、168:101-106）**: `HOOK_STATE.physical_key_state` の更新
  （`hook.rs:1064-1067`、`if !is_injected` 内の `slot.store(is_keydown, ..)`）は、
  `focus_app_disabled` の早期 return（`hook.rs:1094-1096`）より **前**にある。
  さらに `VK_KANA` swallow（`hook.rs:1141`/`1154`）・`VK_DBE_ROMAN`/`NOROMAN` swallow
  （`hook.rs:1203`）・Alt なりすまし分岐（`hook.rs:1218`）よりも前である。
  逆に自己注入の早期 return（`hook.rs:1049-1051`）は更新より前なので、awase 自身の
  送信はビットを汚さない。**改訂版の前提は正しい。**
- **V2**: `JournalEntry::KeyInput` の本番構築点は `key_pipeline.rs:486` の1箇所のみ
  （`journal.rs` 側の出現は定義208行・`lane_kind` 565行・`emit_tracing` 783行・
  `#[cfg(test)]` ヘルパー1672行のみ）。したがって `key_input` レーンに `absorb()` 経由の
  遅延 envelope が入ることは現状あり得ず、`buffer.back()` を「直前の KeyInput」とみなす
  新設ロジックは成立する（ただし R2-2 参照）。
- **V3**: `RawKeyEvent` の構築箇所「61箇所・19ファイル」は正確（`grep -rn "RawKeyEvent {"`
  で 61 / 19、`crates/awase-linux/src/hook.rs` と `tests/scenarios.rs` を含む）。
- **V4**: `last_timestamp_us` は `RawKeyEvent.timestamp` 由来＝**T2系**であり、同一
  エントリ内の `KeyEventSummary.timestamp_us`（同じく T2系）との減算は
  ADR-129 が禁じる系またぎに当たらない。設計として正しい（補足は R2-5）。
- **V5**: `JournalLane::push` の該当箇所は `journal.rs:501-521`。ADR が「502-506 相当」と
  書いた2つの捨て口は、正確には **511行（満杯＋古い遅延 envelope を無条件 return）** と
  **513行（`pop_front()`）**。決定1-b がこの2箇所を対象にするという記述自体は正しい。
- **V6（round1 m1 の自己訂正）**: `size_of::<JournalEntry>() == 264`（`journal.rs:426`）に
  **抵触しない**見込み。現在の `KeyInput` variant は概算で
  `KeyEventSummary`(40) + `String`×2(48) + `DecisionKind`(16) +
  `PhysicalDispositionSummary`(16) + タグ ≒ 128 バイトであり、`repeat_count: u32` +
  `last_timestamp_us: u64` を足しても ~144 バイトで 264 には届かない。ただし R2-7 の
  但し書きを入れること。

## Major（改訂版で新たに見つかった穴）

### R2-1. 決定1-b の計器は「ダンプが予算に収まったとき」に消える（測りたい状況で測れない）

決定1-b（168:204-218）は `evicted_*` を **`DumpTruncated` に出力する**設計になっている。
しかし `DumpTruncated` は切り詰めが発生したときにしか生成されない。

`journal.rs:1283-1290`:

```rust
let total_json_bytes = json_array_len(serialized.iter().map(|e| e.json.len()));
if total_json_bytes <= max_bytes {
    return Ok(CappedJson { json: ..., total_entries, emitted_entries: total_entries,
                           dropped_by_lane: lane_counts() });   // ← ヘッダを作らずに return
}
```

`truncation_header_json()`（=`DumpTruncated` の唯一の生成点、`journal.rs:1470-1490`）は
この early return の後ろにしかない。加えて、ホットキー経由のファイルダンプが使う
`to_json()`（`journal.rs:1263-1266`）は capped 経路を通らないので、そちらにも
`DumpTruncated` は出ない。

失敗シナリオ: 決定1 の畳み込みが効いて JSON が 200KiB に収まるようになった report では、
`DumpTruncated` が生成されない → `evicted_key_input` も出ない → **「畳み込み後に
リング退避が何件残っているか」という、決定2 の再判断に必要なまさにその数字が
取れない**。決定1 が効くほど計器が消えるという逆説的な構造になっている。

**推奨**: `evicted_*` は `DumpTruncated`（切り詰め時のみの合成ヘッダ）ではなく、
**常に出る場所**に置く。候補:

1. `CappedJson` に `evicted_by_lane: [(LaneKind, usize); 4]` を足し、
   `bug_report.rs` が `state_snapshot` 相当の常設フィールドとして払い出す
   （early return 側の戻り値にも同じく詰める。`lane_counts()` を返している箇所に
   実値を入れるだけ）。
2. もしくは `JournalEntry::DumpTriggered`（ダンプのたびに必ず1件入る、
   `journal.rs:415` 付近）にレーン別 evicted を相乗りさせる。

「`DumpTruncated` にも出す」のは併用してよいが、それ**だけ**にしないこと。

### R2-2. `back()` 前提の畳み込みは、現状どこにも強制されていない不変条件に依存する

改訂版の実装設計（168:170-178）は
「`key_input` レーンの `buffer.back()` は直前に `record()` した `KeyInput` である」
という不変条件の上に立つ。V2 の通り現時点では成立するが、これは

- `JournalEntry::KeyInput` の構築点が1箇所であること、
- `key_input` レーンに `absorb()`（`JournalStamper` で先に採番された遅延 envelope）が
  流れ込まないこと

という**2つの偶然の性質**に依存している。`JournalStamper`（`journal.rs:1150-1168`）も
`absorb()`（`journal.rs:1238`）も `pub` であり、将来「hook スレッド側で KeyInput を
stamp して後から absorb する」といった配線が入ると、`JournalLane::push` は
`rposition` で seq 順に挿入し直す（`journal.rs:515-520`）ため `back()` が
直前の KeyInput でなくなり、**無関係なエントリの `repeat_count` が加算される**
（= 時系列の捏造。round1 B2 で避けたはずの壊れ方が別ルートで復活する）。

このリポジトリは同型の不変条件を `tests/architecture_guard.rs` の出現数固定テストで
守る運用が定着している（`panic_reset_event_is_limited_to_apply_panic_reset`、
`autostart_register_call_sites_are_limited_to_tray_click_handler` 等）。

**推奨**: 決定1 の実装コミットに、
`JournalEntry::KeyInput {` の本番構築点が `runtime/key_pipeline.rs` の1件だけであることを
固定する architecture_guard テストを1本追加する（`ime-belief-architecture.md` が言う
「3. private 化できない場合は出現数固定テスト」に相当）。あわせて
`record_key_input()` の doc に「この不変条件が壊れたら `back()` 前提も壊れる」ことを
書く。

## Minor

### R2-3. `RawKeyEvent` への `was_down` 追加は「既存パターンの踏襲」より一段踏み込んでいる

168:110-114 は `injected`/`modifier_snapshot`/親指タイムスタンプと同じパターンだと
書くが、コードを見るとこれらは**すべて core（`awase` クレート）自身が読んでいる**:

- `src/types.rs:346` — `&& !self.injected`
- `src/engine/engine.rs:974` / `1010` / `1041` — `event.injected` による BUG-14/ADR-119 ゲート
- `modifier_snapshot` / `left_thumb_down_snapshot` — `InputContext`/`NicolaFsm::phys` へ供給

一方 `was_down` は **core のどのコードも読まない、Windows 側 journal 専用の診断フィールド**に
なる。ADR-019 が禁じているのは「`windows-rs` / `#[cfg(target_os)]` / 生の VK マジック
ナンバー」であって bool の追加ではないので違反ではないが、「core は事前分類済みの
入力事実だけを受け取る」という説明とは性質が違う。レビューで必ず突っ込まれる点なので、
ADR に

- `was_down` は診断専用であり core のロジックは参照しないこと、
- それでも side-channel ではなく `RawKeyEvent` に載せる理由（`INPUT_DEFER` /
  `OUTPUT_PENDING_QUEUE` の replay 時にライブ再取得すると ADR-129 が扱った
  「replay を実行している"今"の値を読む」事故になるため、capture 時点の
  スナップショットでなければならない）、
- `crates/awase-linux/src/hook.rs`・`crates/awase-macos` 側のスタブも埋める必要があること

を1段落で明記しておくこと。

### R2-4. 畳み込み時に seq を消費するか／app_log と journal の突き合わせ方を決めていない

168:170-176 は「`emit_tracing` は畳み込みの有無に関わらず毎回呼ぶ」と決めたが、
`emit_tracing` は `JournalEnvelope`（`seq` と `elapsed_ms` を持つ）のメソッドであり
（`journal.rs:781`、`JournalStamper::stamp` が `next_seq.fetch_add`）、
**畳み込まれた repeat にも seq を採番するのか**が未定義。

- 採番する場合: `key_input` レーンの seq に穴が空く（app_log には N 行、journal には
  1 エントリ）。読み手が「journal のこの穴は drop か？」と誤解する余地がある。
- 採番しない場合: tracing 行に出す seq を何にするかを決める必要がある。

**推奨**: 採番する（tracing は「フィルタなしの人間向けチャネル」という ADR-139 の
役割分担に合う）方に倒し、repeat の tracing 行に
`coalesced_into_seq`（畳み込み先エントリの seq）フィールドを足して2系統を突き合わせ
可能にする。加えて「KeyInput レーンの seq の穴は畳み込みによるもので、欠落ではない」
ことを ADR と `docs/journal-replay-guide.md` 相当の読み手向け記述に残す。

### R2-5. `last_timestamp_us` だけだと「ホールドの終わり」を他レーンと突き合わせにくい

V4 の通り T2 系内の減算は正しい。ただし journal の他のエントリが並ぶ軸は
`envelope.elapsed_ms` であり、畳み込み後もこれは**初回の時刻のまま**である。
「Ctrl を離した瞬間が、どの `ImeEvent`/`TsfProbe` の前後だったか」を読むには
`ClockAnchor`（`journal.rs:401`）経由の変換が要る。1フィールド足すだけなので
`last_elapsed_ms: u64` も併記することを推奨（`size_of` には余裕がある、V6）。

### R2-6. `should_coalesce_key_input` のシグネチャ2点

168:311-316 の純粋関数について:

- 名前（`should_*`）と戻り値（`CoalesceOutcome` enum）が食い違う。`coalesce_decision` /
  `classify_key_input_coalescing` 等にするか、戻り値を `bool` にする。
  既存の `journal_policy.rs` は `probe_tick_is_notable` / `literal_detect_is_notable` が
  `bool`、`order_violation` が `Option<_>` と、名前と戻り値が一致している。
- `KeyInputIdentity` は `state_before`/`state_after` を **`&str` で借用**する形にすること。
  ここは全打鍵が通る入力ホットパス（`key_pipeline.rs:486`）であり、比較のためだけに
  `String` を2本 clone すると1打鍵あたり2回のヒープ確保が増える（現状の
  `debug_state_label()` 由来の2確保は既存で、そこに追加してはいけない）。

### R2-7. const assert に抵触した場合の対処を「値の更新」で終わらせない

168:338-341 は「抵触する場合は assert の値を実測に基づいて更新する」とだけ書く。
V6 の通り実際には抵触しない見込みだが、もし抵触したら **`KeyInput` が最大 variant に
なった＝全4レーンの `VecDeque` の1件あたりメモリが増えた**ことを意味する。
`journal.rs:415-425` のコメント（ADR-163 Part D の `/code-review` 指摘で入った経緯）が
まさにその回帰を検知するために置かれた assert なので、値を黙って書き換えると
assert の存在意義を消す。「抵触したらどの variant が最大になったかを確認し、
増加が妥当か判断した上で値を更新し、その判断を ADR/コミット本文に残す」まで書くこと。

### R2-8. 残存限界に `reset_physical_key_state()` を追記する

168:132-138 は複数キーボードの限界を明記していて良いが、もう1件ある。
`hook.rs:455` の `reset_physical_key_state()`（スタック修飾キー復旧 / フック再導入時に
256ビットを全クリア）を挟むと、押しっぱなしのキーの次の repeat は `was_down=false` に
なり畳み込まれない。**安全側（畳まない方向）に倒れるので実害は「エントリが1件余分に
増える」だけ**だが、実装者が「なぜここで畳まれないのか」を後から追えるよう、
残存限界の箇条書きに1行足しておくこと。

### R2-9. 決定3 の代替案1（予算の非対称配分）に、既存回帰テストとの関係を一言

168:259-261 の代替案1 は正しい（`build_payload_with_log_budget` は既に予算を引数化
済み、`bug_report.rs:542-556`）。補足として、回帰テスト
`full_size_journal_and_app_log_fit_within_max_body_bytes_without_shrinking`
（`bug_report.rs:1194`）が守っているのは **journal と app_log の合計**が
`MAX_BODY_BYTES` に収まることなので、合計を変えない配分変更はこのテストを壊さない
（テスト内の2本の生成量は要調整）。次の ADR で検討する人が「あの回帰テストがあるから
無理」と早合点しないよう、1行入れておくと良い。

## 再確認しておくべき点（round2 では指摘しないが、実装時に見ること）

- `record_key_input()` を足したあと、`key_pipeline.rs:486` 以外から
  `JournalEntry::KeyInput` を `record()` する経路が残っていないこと（R2-2 のテストで固定）。
- `emit_tracing` の `KeyInput` アームに `repeat_count`（と採用するなら
  `coalesced_into_seq`）を明示追加し、`architecture_guard.rs:4740` のワイルドカード禁止
  ガードを通ること。
- 決定1-b で `JournalLane` にフィールドを足す際、`UnifiedJournal` の `Debug` 実装
  （`journal.rs:1169-1179`、`finish_non_exhaustive()`）に evicted も出しておくと
  実機デバッグで効く（任意）。
