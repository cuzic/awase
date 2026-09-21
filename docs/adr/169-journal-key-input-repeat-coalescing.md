---
id: ADR-169
title: |-
  journal `KeyInput` レーンの OS auto-repeat 畳み込みでダンプ予算窓を圧縮する
status: |-
  実装完了（決定1・決定1-b、ブランチ`feat/adr169-journal-key-input-repeat-coalescing`）。
  opus-adversarial-consult round1/round2で設計収束済み。実装後
  `/code-review opus`でKeyUp誤畳み込みの回帰を発見・修正済み
  （コミット`8ecacaeb`、詳細は「実装ノート」節）。Linux上で
  `cargo test --lib`/`cargo nextest run --workspace --lib`（1785件）・
  `cargo nextest run -p awase-windows --test architecture_guard --test
  golden_scenarios --test layer_boundary_guard`（124件、新設の
  `journal_key_input_construction_is_limited_to_key_pipeline`含む）全緑、
  windows target `cargo check`/`cargo clippy`/`cargo fmt --check`も全緑。
  実機ソーク・windows-build CI実行は未実施
related_adr:
  - "ADR-096"
  - "ADR-095"
---

# ADR-169: journal `KeyInput` レーンの OS auto-repeat 畳み込みでダンプ予算窓を圧縮する

## 背景

不具合報告 `report_id: 01M2CYZ0SFQH3560A0V1YSGKHP`（LINE で「ここここ」大量出力、
`docs/bug-reports-triage.md` に記録済み、原因未確定）を調査中、journal の
`KeyInput` レーンが**ダンプ時点で完全に満杯**（`emitted_entries` のうち
KeyInput 分328 + `DumpTruncated.dropped_key_input: 184` = 512、
`journal_policy.rs::LaneKind::KeyInput.capacity()` の固定値と一致）だった
ことが判明した。

**この184件のロスがどの段で起きたかについて、当初の草稿は因果を誤って
説明していた（opus-adversarial-consult round1 Blocker1で指摘・訂正）。**
`dropped_key_input` は `journal.rs::dropped_by_lane()` の実装上、
「ダンプ時点でレーンに**残っていた**512件のうち、200KiB の byte 予算に
入りきらず捨てた件数」であり、**リングバッファからの退避（`JournalLane::
push` の `pop_front()`）を1件も数えていない**。したがって今回実際に
確認できたロスは:

1. **byte 予算段のロス（実証済み）**: 512件が RAM 上に残っていたのに、
   200KiB の予算のうち KeyInput レーンに配分される分（`journal_policy.rs::
   RESERVED_PERCENT` で15% + 余剰分）に入り切らず328件しか出力されな
   かった。**この184件は byte 予算さえ上げれば実際に取り戻せていた**。
2. **リング退避（存在は確実、実害は未計測）**: `328+184=512=capacity` から
   「ダンプ時点でレーンが満杯だった」こと自体は分かるが、その手前で
   実際に何件が `pop_front()` で溢れて完全に失われたかは、現状どこにも
   カウンタが無く**測定できない**（下記 決定1-b 参照）。

この2層は原因も対策も別であり、本ADRは両方に手を当てる。

同じ report の journal を実際に読むと、512件の内訳の一端も分かる。
LINE（`profile=Imm32Unavailable`）でユーザーが Ctrl キーを約1秒間押し
続けた区間だけで、OS の auto-repeat による `{vk_code: 162 (VK_LCONTROL),
is_down: true}` のほぼ同一な `KeyInput` エントリが約50件連続して記録
されていた（`report_id: 01M2CYZ0SFQH3560A0V1YSGKHP` の journal 実物、
該当区間の seq 範囲は本ADR添付なしだが triage 記録済みの report から
再取得可能）。この1回のホールドだけで、当時レーンに残っていた512件の
うち約1割を占めていた。こうした「実質的に情報量ゼロな反復」が、200KiB
という**限られた出力窓**の中で本来より多くのバイトを占有し、結果として
同じ窓に収まる**異なる打鍵の種類数**を減らしていた、というのが今回
確認できた実害である（「リングから溢れて消えた」という当初の主張より
弱いが、実証されている分こちらの方が確実）。

## 問題

`crates/awase-windows/src/runtime/key_pipeline.rs`（`kp_run_inner`、
`JournalEntry::KeyInput` の構築点は同ファイル486行目の1箇所のみ）は、
フックが受け取った**すべての**物理キーイベントについて無条件に
`journal.record()` している。ここには OS auto-repeat による同一キーの
反復も、IME/NICOLA変換に一切関与しない素通りキーも区別なく含まれる。
200KiB のダンプ byte 予算は有限であるため、キーを長く押し続けるだけで
同じ予算窓の中の「異なる打鍵をカバーできる範囲」が狭まり、実際に
不具合の原因特定に必要な区間（IME ON直後の cold 期間の打鍵、親指シフト
同時打鍵のタイミング等）が出力から漏れる。

不具合報告機能（ADR-095）は runtime の挙動を変えずに事後診断のための
証拠を残すことが目的（ADR-096 冒頭も同旨）であり、これは「診断の目的を
果たせていない」という意味での不具合と位置づける。

## 決定

### 決定1（主要）: OS auto-repeat の連続 `KeyInput` を1エントリへ畳み込む

**判定の情報源（round1 Blocker2で全面変更）**: 当初案は「journal 上で
同一 vk・両方 `is_down:true`・間に `is_down:false` が無ければ auto-repeat」
と journal 側だけで判定しようとしたが、これは**journal が key-up を
必ず見る**という誤った前提に依存していた。実際には次の経路で key-up が
journal に到達しないまま消えることがある:

- `hook.rs` の `focus_app_disabled`（`disable_apps`、既定 `mstsc.exe`、
  BUG-78）中は全イベントが `key_pipeline` に届かず、その間の down/up が
  丸ごと journal から見えない。
- `hook.rs` の foreign-injected 系 swallow（`VK_KANA` 等、BUG-08/62）。
- `key_pipeline.rs` 内の2箇所の早期 return（`try_hold_key` の
  `PendingWarmup` 保留、IME OFF rescue の defer）。これらは journal
  記録前に return し、実処理は後で `kp_run_inner` の再入時に別 seq で
  記録されるため、journal 上の順序が物理順と一致しない。

加えて PowerToys Mouse Without Borders・AutoHotkey・ソフト KVM 等の
foreign-injected 入力は down/up の対を保証しない（BUG-90/issue #136。
本 report の環境自体が `docs/bug-reports-triage.md` に「競合ソフト:
PowerToys」と記録されている）。これらは `injected` フラグが立った
まま journal に到達し、`injected` はまさに BUG-90/issue #136 系の
診断で決め手となる情報なので、誤って auto-repeat とみなして畳んでは
ならない。

**採用する判定**: journal 側で推測せず、`hook.rs` が既に持つ物理キー
状態の SSOT を使う。

- `hook.rs::HOOK_STATE.physical_key_state`（`[AtomicBool; 256]`、
  非 injected イベントのみで更新）は、フック内で **`disable_apps` の
  早期 return より前**に更新される（`hook.rs` 該当コメント: 「前に
  置くと無効アプリに入る直前から押していたキーの KeyUp が記録されず、
  スタックを新規に生む」）。つまりこのビットは、journal がそのイベントを
  見られるかどうかとは独立に、物理的な押下状態を正しく追い続けている。
- 現在は `slot.store(is_keydown, ...)` だが、これを `slot.swap(is_keydown,
  ...)` に変え、戻り値（更新前の値 = そのイベント直前の物理状態）を
  `was_down` として捕捉する。
- `was_down` をイベントに載せて `key_pipeline.rs` まで運ぶ。運搬先は
  `RawKeyEvent`（`src/types.rs`）の新規フィールドとする。**コスト**:
  `RawKeyEvent` はリテラル構築のみ（`#[derive(Default)]` 無し）で、
  構築箇所はリポジトリ全体で61箇所（19ファイル、`crates/awase-linux/
  src/hook.rs`・`tests/scenarios.rs` を含むテストの直接構築含む、
  round2 V3で実数確認済み）ある。既存の `injected` 等の追加時と同様、
  全箇所を機械的に更新する必要がある（実装コミットの diff サイズとして
  許容する）。`crates/awase-linux`/`crates/awase-macos` 側のプラット
  フォームスタブにも `was_down: false` 相当の埋め込みが要る。
  - **round2 R2-3への対応（`was_down`は既存パターンの単純な踏襲ではない
    ことの明記）**: `injected`/`modifier_snapshot`/親指タイムスタンプは
    いずれも **core（`awase` クレート）自身が読んで判断に使っている**
    値（`src/types.rs`/`src/engine/engine.rs` の `event.injected` ゲート、
    `InputContext`/`NicolaFsm::phys` への `modifier_snapshot` 供給等）。
    対して `was_down` は **core のどのロジックも参照しない、Windows側
    journal 専用の診断フィールド**であり、性質が異なる。ADR-019が禁じる
    のは `windows-rs`/`#[cfg(target_os)]`/生VKマジックナンバーであって
    bool の追加自体ではないため違反ではないが、レビューで必ず問われる
    点なので明記しておく。それでも「journal 記録箇所だけのローカル
    side-channel」ではなく `RawKeyEvent` に正式に載せる理由は、
    ADR-129が扱った「`INPUT_DEFER`/`OUTPUT_PENDING_QUEUE` 経由の drain
    replay 時にライブ再取得すると、replay を実行している"今"の値を
    読んでしまう」事故と同型の罠を避けるため——`was_down` も
    capture 時点（hook コールバック内）のスナップショットとして
    確定させ、後から再クエリしない設計にする必要がある。
- 畳み込み対象は **「`!event.injected` かつ `event.was_down == true`
  かつ `event.event_type == KeyDown`」** の場合のみ。`injected` な
  イベントは `was_down` の値に関わらず常に畳み込み対象外とする
  （foreign-injected 連打を確実に除外するため、`was_down` の値だけに
  頼らない二重ガードにする）。

この設計により、round1 で指摘された3シナリオはそれぞれ:

1. **disable_apps 越しの再押下**: `physical_key_state` は disable_apps
   中も正しく更新され続けるため、実際に離されていれば `was_down=false`
   になり畳み込まれない（解消）。
2. **foreign-injected 連続 down**: `!event.injected` ガードで常に除外
   （解消）。
3. **複数キーボード/マクロパッドが同一 vk を同時に扱う**: OS の
   キーボードフック自体が送信元デバイスを区別しないため、この
   ケースは `physical_key_state` ベースでも原理的に解決できない
   （許容する残存限界として明記する。実害は「同時に無関係な2台目の
   キーボードで同じキーが単発で押された場合、その1件が直前の
   auto-repeat 列に紛れて独立エントリを失う」に留まり、BUG-90 系の
   ような時系列の捏造は起きない）。
4. **（round2 R2-8で追加）`hook::reset_physical_key_state()` を挟んだ
   直後の repeat**: スタック修飾キー復旧・フック再導入時に256ビット
   全クリアされるため、押しっぱなしのキーの次の repeat は
   `was_down=false` になり畳み込まれない。安全側（畳まない方向）に
   倒れるため実害は「エントリが1件余分に増える」だけで、上記1〜3と
   異なり誤って畳み込む方向の事故ではない。

**畳み込み条件に payload 完全一致を追加（round1 Blocker3）**: 上記の
`was_down`/`injected` ゲートを満たしても、以下がすべて一致する場合に
限り畳み込む。1つでも異なれば新規エントリとして記録する。

`vk_code` / `scan_code` / `key_class` / `alt` / `ctrl` / `shift` /
`state_before` / `state_after` / `decision` / `physical`

理由: auto-repeat 中でも NICOLA FSM の状態遷移（`PendingChar`/
`PendingThumb` 等）や `decision`（`Consume`↔`PassThrough`）、修飾キー
状態（例: `a` 押しっぱなしの途中で `Shift` を追加で押す）は変化しうる。
これらが変化した瞬間は「auto-repeat だが診断上意味のある変化点」であり
畳み込んではならない。Ctrl 長押しのような典型的な無駄反復はこの条件でも
問題なく畳めるため、決定1 の効果はほぼ落ちない。

**畳み込みの実装場所（round1 Major1）**: `UnifiedJournal` の既存公開口
（`record()`/`absorb()`）はどちらもこの用途に使えない。

- `record()`/`absorb()` はどちらも「新規エントリを追加する」前提の
  API で、直近エントリを読む/書き換える手段が無い。
- `absorb()` は `drain_journal_entries()` 経由で**遅延 envelope**
  （seq 順に `rposition` で挿入し直す）も受け取るため、「レーン末尾＝
  直前に記録した `KeyInput`」という前提が成り立たない。畳み込みには
  使わない。
- `absorb()`/`record()` は ADR-139 決定4 の tracing fan-out
  （`emit_tracing`）の起点でもある。畳み込みで `record()` 呼び出し
  自体を省略すると、`app_log_excerpt`（tracing 経由）側から auto-repeat
  の痕跡が消え、journal と app_log の2系統のうち片方の証拠を失う
  （当初案の「挙動には一切影響しない」という記述はこの意味で不正確
  だった）。

  **採用する設計**: `UnifiedJournal` に新規メソッド（例:
  `record_key_input(event: KeyInputRecord)`）を追加し、内部で
  (a) `emit_tracing` は**畳み込みの有無に関わらず毎回呼ぶ**
  （app_log 側の記録を維持）、(b) `key_input` レーンへの追記だけを
  `JournalLane` 内の専用ロジック（`buffer.back()` を見て
  `journal_policy::coalesce_key_input()` の判定結果に応じて
  `back_mut()` を更新、または通常の `push()`）に振り分ける。
  この専用ロジックは `absorb()` の遅延 envelope 処理とは完全に分離し、
  `record()` の通常経路にのみ適用する。

  **round2 R2-2への対応（`back()` 前提の不変条件を固定する）**: この
  設計は「`key_input` レーンの `buffer.back()` は直前に `record()` した
  `KeyInput` である」という不変条件（`JournalEntry::KeyInput` の構築点が
  `key_pipeline.rs:486` の1箇所のみであること、かつこのレーンに
  `absorb()` 経由の遅延 envelope が流れ込まないこと、の2点に依存）の
  上に立つ。`JournalStamper`/`absorb()` はいずれも `pub` であり、
  将来「hook スレッド側で `KeyInput` を stamp してから後で absorb する」
  といった配線が入ると、`JournalLane::push` の `rposition` 挿入により
  `back()` が直前の `KeyInput` でなくなり、無関係なエントリへ
  `repeat_count` が加算される（round1 B2で避けたはずの時系列捏造が
  別ルートで復活する）。このリポジトリで同型の不変条件を守る既存手法
  （`tests/architecture_guard.rs` の出現数固定テスト、
  `panic_reset_event_is_limited_to_apply_panic_reset` 等）に倣い、
  実装コミットに `JournalEntry::KeyInput {` の本番構築点が
  `runtime/key_pipeline.rs` の1件だけであることを固定するガードテストを
  追加する。あわせて `record_key_input()` の doc comment に
  「この不変条件が壊れると `back()` 前提も壊れる」ことを明記する。

**envelope を触らない（round1 Major2）**: 当初案の「末尾 `elapsed_ms`
の更新」は、`JournalEnvelope.seq` が据え置きのまま `elapsed_ms` だけ
進むため、seq 昇順の出力の中で時刻が非単調になる内部矛盾を生む
（ADR-096 が定義した `elapsed_ms`/`tick_ms`/`timestamp_us` の相互変換
可能性の前提も壊す）。`JournalEnvelope` 自体は一切変更せず、
`JournalEntry::KeyInput` 側に以下を追加する:

```rust
KeyInput {
    event: KeyEventSummary,
    state_before: String,
    state_after: String,
    decision: DecisionKind,
    physical: PhysicalDispositionSummary,
    repeat_count: u32,        // 追加。1 = 初回のみ（畳み込みなし）
    last_timestamp_us: u64,   // 追加。畳み込んだ最後のイベントの生時刻（T2系）
    last_elapsed_ms: u64,     // 追加（round2 R2-5）。同じく最後のイベントの
                              // envelope 時間軸（elapsed_ms系）での値
}
```

`repeat_count` を1から始め、畳み込むたびに加算する。「初回のみ」の
場合に `repeat_count` を出さない（`Option`/`skip_serializing_if`）
選択肢は、実装時に `size_of::<JournalEntry>()==264` の const assert
（`journal.rs:426`）への影響と天秤にかけて決める（後述テスト方針参照）。

**`last_elapsed_ms` を併記する理由（round2 R2-5）**: `last_timestamp_us`
だけでは、journal の他のエントリが並ぶ軸（`envelope.elapsed_ms`、
畳み込み後も初回時刻のまま）との突き合わせに `ClockAnchor` 経由の
系変換が要る。「Ctrl を離した瞬間が、どの `ImeEvent`/`TsfProbe` の
前後だったか」を読み手が直接比較できるよう、1フィールド追加のコストで
`last_elapsed_ms` も持たせる（V6で確認済みの通り `size_of` には余裕が
ある）。

**畳み込み時に seq を消費するか（round2 R2-4への対応）**: `emit_tracing`
は `JournalEnvelope`（`seq`/`elapsed_ms` を保持）のメソッドであり、
畳み込まれた repeat 側にも `JournalStamper` で seq を採番する
（tracing 側は「フィルタなしの人間向けチャネル」という ADR-139 の
役割分担に合わせ、1物理イベント=1 tracing 行を維持する）。この結果
`key_input` レーンの journal 出力には seq の穴が空く（app_log には
N 行、journal エントリは1件に畳まれる）ため、repeat の tracing 行に
`coalesced_into_seq`（畳み込み先エントリの seq）フィールドを足して
2系統を突き合わせ可能にする。「`KeyInput` レーンの seq の穴は畳み込みに
よるものであり drop ではない」ことを、本ADR・`docs/journal-replay-guide.md`
相当の読み手向け記述の両方に残す。

### 決定1-b（新規・round1 Major3への対応）: レーン別 eviction カウンタの新設

決定2（容量据え置き）の判断も、次回同種の report が来たときの再判断も、
「リング退避が実際に何件起きたか」を測る計器が無いままでは実測に基づけ
ない。`DumpTruncated.dropped_key_input` は byte 予算段の指標であり
リング退避を代替できないことは背景で確認済み。

`JournalLane::push()` が容量超過で `pop_front()` する箇所（`journal.rs:513`
相当）、および「レーンが満杯かつ来た envelope が `front` より古い」ため
無条件に捨てる箇所（`journal.rs:511`相当）の両方に、レーン単位の
`evicted: usize` カウンタ（`JournalLane` に持たせる）を追加する。

**出力先を `DumpTruncated` にしない（round2 Major R2-1への対応）**:
`DumpTruncated` は `to_json_capped()` が **切り詰めを実際に行った場合
にしか生成しない**合成ヘッダであり（`journal.rs:1283-1290` 付近、
JSON 総バイト数が予算内に収まった場合はヘッダを作らず早期returnする）、
ホットキー経由のフルダンプ（`to_json()`）はそもそも capped 経路を
通らずこのヘッダ自体を生成しない。つまり `evicted_*` を
`DumpTruncated` だけに出す設計だと、**決定1の畳み込みが効いて
200KiBに収まるようになった report ほど、測りたいはずの数字
（畳み込み後もリング退避が残っているか）が出力から消える**という
逆説的な構造になる。

`evicted_*` は代わりに **常に生成される場所**に出す:
`to_json_capped()`/`to_json()` の戻り値である `CappedJson` に
`evicted_by_lane: [(LaneKind, usize); 4]` を追加し（早期return側の
`dropped_by_lane` 相当の戻り値にも実値を詰める）、呼び出し元
（`bug_report.rs`）が `state_snapshot` 相当の常設フィールドとして
払い出す。`DumpTruncated` に切り詰め発生時の参考情報として重複して
出すこと自体は構わないが、そちらを**唯一の出力先にはしない**。

これは決定1本体より小さい変更であり、決定1の効果測定（畳み込みで
`evicted_key_input` が何件減ったか）と決定2の再判断の両方に使う。

### 決定2: `KEY_INPUT_LANE_CAPACITY` は据え置く

**当初案の根拠を訂正（round1 Major4）**: 当初「実測なしのエスカレー
ション禁止」という `tuning-constants.md` の精神をそのまま類推していた
が、これは筋が違う。`tuning-constants.md` が禁じるのはタイミング定数の
引き上げで、理由は「レイテンシ悪化と別の spurious 誘発」。リング容量は
挙動を一切変えず、コストはメモリのみ（`JournalEntry` は264バイト
固定、512→2048で概算+約0.5MB）であり、そもそも比較にならない。

**据え置く本当の理由**: 今回の report で emit を律速していたのは
byte 予算（KeyInput レーンは15%の予約枠 + 余剰分でしか予算をもらえず、
512件中328件しか通っていない＝予算が先に尽きている）であり、
リングバッファを大きくしても `select_tail_within_budget` は新しい側
から同じバイト数ぶんしか選ばない。**容量を上げても今回のような report
での添付件数は増えない。** 据え置くという結論は変えないが、根拠は
「emit を律速しているのが別の層にあるため、この容量を上げても今回の
症状には効かない」という消去法に置き換える。決定1-bの `evicted_key_input`
カウンタが実際に高止まりを示した場合に限り、容量引き上げを再検討する
（この場合は真にリング側の問題なので、tuning-constants 的な「実測して
から動かす」判断が意味を持つ）。

### 決定3: `LOG_EXCERPT_MAX_BYTES`（現在200KiB）の一律引き上げは行わない

`LOG_EXCERPT_MAX_BYTES` は過去に **256KiB から 200KiB へ引き下げられた**
経緯があり（`bug_report.rs` のテスト
`full_size_journal_and_app_log_fit_within_max_body_bytes_without_shrinking`
がその回帰テスト）、journal 200KiB + app_log 200KiB で Cloudflare
Worker 側 `MAX_BODY_BYTES`（512KiB、`services/report-worker/src/
index.ts`）にほぼ余裕なく収まるよう意図的に調整されている。journal 側
だけを単純に引き上げると、この値を200KiBへ下げるきっかけになった
「送信のたびに自動切り詰めが発生する」問題を再発させる。

**当初案からの訂正（round1 Blocker1/m6）**: 背景で確認した通り、今回
実際に失われた184件は byte 予算側で回収可能だった実績があるため、
「byte 予算を上げても無意味」という当初の却下理由は誤り。ただし
`MAX_BODY_BYTES` 自体に触れない、より筋の良い代替が最低3つあり、
これらは**本ADRのスコープ外の次点候補**として名前を残す（決定1の
効果測定後、まだ不足するなら次のADRで検討する）:

1. journal/app_log の予算配分を非対称にする（`build_payload_with_
   log_budget` は既に予算を引数化済みなので、journal 250KiB / app_log
   150KiB のような配分は `MAX_BODY_BYTES` の合計を変えずに実現できる。
   既存の回帰テスト `full_size_journal_and_app_log_fit_within_max_
   body_bytes_without_shrinking` が守っているのは journal と app_log
   の**合計**が `MAX_BODY_BYTES` に収まることであり、合計を変えない
   配分変更はこのテストと衝突しない——round2 R2-9。次にこの案を検討
   する人が「あの回帰テストがあるから無理」と早合点しないための
   注記）。
2. `journal_policy::RESERVED_PERCENT`（KeyInput は現在15%）の見直し。
   ただし leftover pass で KeyInput は最後に回る設計のため、予算全体が
   逼迫している状況での効果は限定的で要実測。
3. **1エントリあたりのバイト削減**（決定1と同じ「1件を小さくする」
   方向で、auto-repeat 以外のエントリにも効く）: `state_before`/
   `state_after` は `String`（`debug_state_label()` 由来）で
   KeyInput 1件の JSON バイトの相当部分を占める。両者が等しい場合に
   片方を省略する等が候補。

いずれも本ADRでは実装しない。決定1（畳み込み）と決定1-b（計器）を
先に入れ、その効果を実測してから要否を判断する。

## 検討したが採らなかった案

- **`Passthrough`（IME非関与の素通りキー）を `KeyInput` レーンから
  一律除外する**: 当初「BUG-105/BUG-049 で決め手になった実績」を根拠に
  挙げていたが、これは誤引用だった（round1 m5で指摘・訂正。BUG-105の
  決め手は `Char`/`Thumb` 分類のタイミング比較であり `Passthrough`
  ではない。BUG-049は本件と無関係の別バグ）。正しい却下理由は、
  本ADRの背景そのものにある——**Ctrl 押下はまさに `Passthrough`
  分類**であり、それが今回の「ノイズの主因」であると同時に、
  BUG-116/ADR-137・issue #136 のような修飾キー絡みの証拠でもある
  （`key_pipeline.rs` の Ctrl+無変換 IME OFF 救済 `ctrl_consumed_
  since_down` は Ctrl の down/up 列そのものを診断対象にしている）。
  だからこそ「除外」ではなく「畳み込み」が正解であり、一律除外は
  採らない。
- **時間ウィンドウベースの重複排除**（例: 同一vkが50ms以内に再度来たら
  間引く）: OS auto-repeat の周期（初回遅延後は数十msおき）に近い
  時間で発生する正当な別入力（親指シフトの同時打鍵、高速タイプ）を
  誤って間引く恐れがあり、`was_down` という判別可能な物理状態が既に
  あるためこちらを採用しない。

## テスト方針

**判定ロジックは `journal_policy.rs`（Windows非依存、`lib.rs` で
ungated）の純粋関数として切り出し、Linux上の `cargo test --lib` で
実行可能にする**（当初案の「`journal.rs`/`key_pipeline.rs` に
`#[cfg(test)]` を置く」は、両モジュールとも `lib.rs` で
`#[cfg(windows)]` 配下にあり Linux のテストバイナリに一切現れない
ため誤りだった。round1 Major5で指摘・訂正。CLAUDE.mdが警告する
「`cargo test --list` にも出ない、エラーもスキップ表示も出ない」
パターンそのもの）。

```rust
// journal_policy.rs
pub struct KeyInputIdentity<'a> {
    // vk_code, scan_code, key_class, alt/ctrl/shift, decision, physical の値を保持。
    // state_before/state_after は &str で借用する（round2 R2-6:
    // key_pipeline.rs:486 は全打鍵が通るホットパスであり、比較のためだけに
    // String を2本 clone すると1打鍵あたり2回の余分なヒープ確保が増える）。
    pub state_before: &'a str,
    pub state_after: &'a str,
    // was_down/injected は KeyInputIdentity に含めず別引数で渡す
    // （payload一致条件と物理状態ゲートを明確に分離するため）。
}
pub enum CoalesceOutcome { NewEntry, MergeIntoPrevious }
// 関数名を戻り値の列挙型と対応させる（round2 R2-6: 既存の
// journal_policy.rs は probe_tick_is_notable/literal_detect_is_notable が
// bool、order_violation が Option<_> と、名前と戻り値の形が一致している）。
pub fn coalesce_key_input(
    prev: Option<&KeyInputIdentity>,
    next: &KeyInputIdentity,
    next_was_down: bool,
    next_injected: bool,
) -> CoalesceOutcome;
```

テスト項目:

- `was_down=true` かつ `injected=false` かつ payload 完全一致なら
  `MergeIntoPrevious`。
- `injected=true` なら `was_down` の値に関わらず常に `NewEntry`
  （foreign-injected 連打の保護）。
- payload の一部（`decision`/`state_after`/`shift` 等）が異なれば
  `was_down=true` でも `NewEntry`。
- `prev=None`（レーン先頭）は常に `NewEntry`。

`journal.rs`/`hook.rs` 側の配線（`physical_key_state` の `swap`化、
`RawKeyEvent.was_down` の追加、`UnifiedJournal::record_key_input` の
新設）は Windows ゲート配下のため Linux では実行できない。CLAUDE.md の
既定方針どおり `cargo check --target x86_64-pc-windows-msvc -p
awase-windows --lib` でコンパイル確認に留め、実行確認は次回 Windows
実機セッションに委ねる。

追加の実装チェック項目（round1 m1/m2）:

- `size_of::<JournalEntry>() == 264` の const assert（`journal.rs:426`）
  に `repeat_count`/`last_timestamp_us`/`last_elapsed_ms` 追加後も
  抵触しないか `cargo check` で確認する。round2 V6 の概算
  （現状の `KeyInput` は`KeyEventSummary`(40)+`String`×2(48)+
  `DecisionKind`(16)+`PhysicalDispositionSummary`(16)+タグ≒128バイト、
  3フィールド追加後も~152バイトで264には届かない見込み）では抵触しない
  想定だが、**もし抵触した場合は値を黙って書き換えて終わらせない**
  （round2 R2-7）。この assert は ADR-163 Part D の `/code-review`
  指摘で「`KeyInput` が最大 variant になった＝全4レーンの `VecDeque`
  の1件あたりメモリが増えた」ことを検知するために置かれており、
  抵触時はどの variant が新たに最大になったかを確認し、増加が妥当か
  判断した上で値を更新し、その判断根拠を実装コミット本文に残すこと。
- `tests/architecture_guard.rs` の
  `journal_emit_tracing_has_no_debug_display_sigils_or_wildcards`
  （`emit_tracing` 内で `_ =>`/`..=>` を禁止するガード）に抵触しない
  よう、`KeyInput` の match アームに `repeat_count`（採用するなら
  `coalesced_into_seq`）を含む新規フィールドを明示的に追加する
  （`?`/`%` シギルは付けない。`repeat_count` は数値なのでそのまま）。
- `record_key_input()` を追加した後、`key_pipeline.rs:486` 以外から
  `JournalEntry::KeyInput` を `record()` する経路が新たに増えていない
  ことを、決定1の実装コミットに含める architecture_guard テスト
  （上記「採用する設計」段落参照）で固定する。

**既存テスト・再生ハーネスへの後方互換影響（round1 m3、調査済み）**:
`crates/awase-windows/tests/` 配下・`tests/journals/` 配下に文字列
`KeyInput` の出現は0件。`journal_replay.rs` は `ConvClassifyCall` のみ、
ADR-159/163 の actuation decision 再生は `ActuationDecisionRecord`
専用で、いずれも `KeyInput` の JSON 形状に依存しない。`JournalEntry`
は `Serialize` のみで `Deserialize` を derive していないため、フィールド
追加で読み手側が壊れることもない。影響を受けるのは `journal.rs` 内の
`#[cfg(test)]` テストヘルパー（Windows専用）のみ。

`fix-requires-evidence.md` の再発ファミリー表には journal 自体は含まれて
いないが、診断基盤の不具合を再発させないという同種の観点から、上記を
本ADR実装コミットに含める。

## 実装ノート（設計との差分）

- **`was_down` の運搬先**: 設計どおり `RawKeyEvent`（core）へ追加。
  構築箇所61箇所（19ファイル）を機械的に更新（`was_down: false`固定、
  実際に物理状態を反映するのは `hook.rs::build_raw_key_event` の1箇所のみ）。
- **R2-4（`coalesced_into_seq`）は簡略化**: `record_key_input` は
  `emit_tracing` を毎回呼ぶ前に `JournalStamper::stamp` で毎回新しい
  `seq` を採番する設計にしたため、tracing/app_log には物理イベントごとに
  異なる `seq` がそのまま残る。畳み込まれた repeat はその `seq` を持つ
  journal エントリを**作らない**（直前の `KeyInput` エントリの
  `repeat_count` へ吸収される）ため、明示的な `coalesced_into_seq`
  フィールドを追加しなくても「`KeyInput` レーンの seq の穴＝畳み込みに
  よるもの」は、直前エントリの `repeat_count` から追跡できる。
- **決定1-bの出力先**: `CappedJson::evicted_by_lane`（`bug_report.rs`
  経路）に加え、`JournalEntry::DumpTriggered`（ダンプのたびに必ず1件
  記録される、既存の呼び出し箇所2箇所）にも `evicted_state`/
  `evicted_timing`/`evicted_actuation`/`evicted_key_input` を追加。
  R2-1が懸念した「畳み込みが効くほど計器が消える」問題を、この2箇所
  常設化で解消。
- **決定3の代替案（journal/app_log予算配分見直し・`RESERVED_PERCENT`
  見直し・1エントリあたりバイト削減）は未実装のまま**（本ADRのスコープ外、
  次点候補として名前のみ残す）。
- 新規 architecture_guard テスト
  `journal_key_input_construction_is_limited_to_key_pipeline` は、
  `journal.rs` 自身が内部で `JournalEntry::KeyInput` を分解（パターン
  マッチ）する箇所と区別するため、フルパス表記
  `crate::journal::JournalEntry::KeyInput {`（外部モジュールからの
  construction は必ずこの形になる）のみを数える設計にした。
  `size_of::<JournalEntry>() == 264` の const assert は変更不要
  （3フィールド追加後も最大 variant は更新されなかった）。

### 実装後レビュー（`/code-review opus`）で発見・修正した回帰（コミット`8ecacaeb`）

初回実装は `KeyInputIdentity` に `is_down`（KeyDown/KeyUp の区別）を
含めておらず、`coalesce_key_input` も `event_type` を確認していなかった。
`hook.rs::HOOK_STATE.physical_key_state` の `swap` は KeyDown/KeyUp
**両方**のイベントで「直前の物理押下状態」を返すため、ごく普通の
1タップ（KeyDown→KeyUp）でも KeyUp 時点では `was_down: true` になる
（直前は押されていたので当然、auto-repeatの証拠ではない）。この結果、
他フィールドが一致する（アイドル中の `Passthrough` キーではほぼ常に
一致する）限り、**実質すべての単発タップで KeyUp が直前の KeyDown へ
誤って畳み込まれ**、journal 上は「押しっぱなしで一度も離されていない」
という誤った記録になっていた——決定1本文（168:104-107時点の草稿）が
明記していた「畳み込み対象は `event_type == KeyDown` の場合のみ」という
条件を、実装時に取りこぼしていた。

`KeyInputIdentity` に `is_down: bool` を追加（`PartialEq` 比較に自動的に
含まれる）し、`coalesce_key_input` にも `next.is_down`/`prev.is_down` の
明示ガードを二重に追加（`is_down` 以外の全フィールド一致に頼る設計への
将来的な変更でも安全なように）。回帰テスト2件
（`coalesce_never_merges_keyup_into_preceding_keydown_even_if_was_down`・
`coalesce_never_merges_keydown_into_preceding_keyup`）を追加。
`src/types.rs::RawKeyEvent::was_down` のdoc commentも、KeyUpでも
更新される事実を明記するよう訂正した。

### 実装後レビュー第2ラウンド（コミット`ef1d8197`）: 契約違反時のパニック誘発とevicted位置依存

再度 `/code-review opus`（正しいブランチを対象に再実行）で3件指摘・修正:

1. `record_key_input()` の「契約違反（非KeyInput）」フォールバックが
   `key_input` レーンへ無条件 push していたため、次回呼び出しの
   `key_input_identity()` が `unreachable!()` でパニックする経路が
   存在した（`debug_assert` はリリースビルドで無効化されるため実害が
   残る）。`absorb()` と共通の `route_to_lane()`（`lane_kind()` に
   基づく正しいレーン振り分け）に置き換え、`absorb()` 側にも
   `KeyInput` 混入を検知する `debug_assert` を追加。
2. `evicted_by_lane()` が `[(LaneKind, usize); 4]` を位置依存
   （`evicted[0].1` 等）で消費されていたため、named struct
   `EvictedByLane { state, timing, actuation, key_input }` に置き換え、
   将来の並び順変更がコンパイルエラー無しに誤対応する危険を解消。
3. `key_pipeline.rs` が渡す `repeat_count`/`last_timestamp_us`/
   `last_elapsed_ms` の初期値は `record_key_input()` が常に上書きする
   死んだ値であることをコメントで明記。

`record_key_input()` 自体のユニットテスト4件（畳み込み成立・
`was_down=false`での非畳み込み・KeyUpの非畳み込み・契約違反時の
パニック確認）を追加。指摘のうち「`KeyInputDecisionShape`/
`KeyInputPhysicalShape` が `DecisionKind`/`PhysicalDispositionSummary`
を複製している」点は、決定1本文が既に述べている `journal_policy.rs`
非ゲート化とのトレードオフとして意図的に受け入れ、変更しなかった。

### 実装後レビュー第3ラウンド（コミット`8cba3b94`）: 直前injectedエントリへの誤畳み込み

`/code-review opus` を5観点並列で再実行し、以下を発見・修正:

- **[重要]** `KeyInputIdentity` に `injected` が含まれておらず、
  `coalesce_key_input` は `next_injected`（これから記録するイベント側）
  しか確認していなかった。foreign-injected な KeyDown（BUG-90/issue #136）
  が偶然レーン末尾に居るとき、直後に届いた**本物**の物理 auto-repeat
  （`next_injected: false`）が、他フィールド一致だけでその injected
  エントリへ誤って畳み込まれうる欠陥だった——is_down の欠落
  （round1発見）と対称の、`prev` 側を見落とすバグ。`injected` を
  `KeyInputIdentity` に追加し、`coalesce_key_input` にも
  `!prev.injected` の明示ガードを二重に追加。回帰テストを追加。
- `JournalLane::push` の `capacity == 0` 早期return が `evicted` を
  計上していなかった（他2つの喪失経路は計上済み）。網羅性のため修正
  （本番では到達しない経路）。
- `record_key_input` の `MergeIntoPrevious` 枝で、既に束縛済みの
  `event` を使わず `envelope.entry` を再度matchしていた冗長な分解を
  削除（reuse/simplification観点、`/code-review` 指摘）。

その他の指摘（`repeat_count`等3フィールドの手動複製をヘルパー化する案、
`dropped_by_lane` も `EvictedByLane` 型に揃える案、placeholder値を
専用コンストラクタで型的に保証する案）は、正当な指摘だが本ADRのスコープ
（バグ修正）を超える設計改善として今回は見送り、次のリファクタ候補として
記録のみ残す。

## 関連

[ADR-096](096-journal-priority-tiers-multi-lane-ring-buffer.md)（本ADRが
対象とする4レーン優先度リングバッファとbyte予算選定の導入元）、
[ADR-095](095-tray-bug-report-cloudflare-intake.md)（`LOG_EXCERPT_MAX_
BYTES`/`MAX_BODY_BYTES` の由来）、[docs/bug-reports-triage.md](../bug-reports-triage.md)
の `01M2CYZ0SFQH3560A0V1YSGKHP` 行（本ADRの発端）、
`docs/design/opus-review-adr169.md`（round1/round2 レビュー全文。
本ADRは元々168番で起票したが、`docs/adr/168-actuation-boundary-small-
cleanups.md`〈PR #213、developマージ済み〉と番号が衝突していたため
169へ採番し直した）。
