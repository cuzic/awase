# ADR-128: recovery resend が自分自身の実送信より前に `pending_deferred` を drain し、出力順を反転させたうえ直後の per-VK confirm の証拠を汚染して `StaleConfirm`→`VK_ESCAPE` を誘発する（ADR-123 決定4-3 の回帰）

## ステータス

**root cause・decision 確定、実装未着手。opus-adversarial-consult
round1〜round3 完了・収束（round3 の唯一の blocker——cold 分岐での
flush 実行主体の取り違え——を反映済み。レビュアーは「このリストの後に
続きはありません」「round4 は不要」と round3 で明言済み）。**
[BUG-109](../known-bugs.md)（`report_id: 01M1MW0KSY5KWVYSGPGRBTNSPA`）から起票。

当初の草案（第1版）は「`pending_deferred` の flush は正しく起きているのに、
flush 後に生えた元モーラの stale-confirm 回収が composition ごと巻き込んで
壊す」という因果を主張していたが、**Opus 敵対的レビュー（round1）が journal
の `deferred_flushed: 0`（`seq 2450`）と app_log の実際の行順から、この因果が
逆であることを実証した。** 本版はその指摘を反映して全面的に書き直した。

## 問題（訂正版）

ADR-123（issue #148、PR #155、develop マージ済み）の決定4-3
「drain-before-send」は、`pending_deferred` が非空かつ probe/recovery が
blocking していない「queue-only」状態のとき、**新規モーラの送信前に**
キューを flush する（`output/vk_send.rs::
drain_pending_deferred_before_send_if_queue_only`）。この判定
（`is_probe_or_recovery_blocking(true)`）は `gate`（`DeferGate::Enforced`/
`Exempt`）を一切見ない——`defer_respecting_gate`（同ファイル、ADR-123
決定4-2）が `gate` で defer 方向の挙動を切り替えるのと非対称である。

その結果、**recovery resend（`resend_gji_reinit_retry_romaji`。give-up 後の
GJI reinit retry が `Confirmed` した際、保留していた元モーラを通常経路へ
1回だけ戻す、ADR-101 決定3）自身の送信**も「新規モーラの送信」と誤認され、
まだ確定していない元モーラの再送**より前**に `pending_deferred` を drain
してしまう。BUG-109 の実機ログでは、「な」の recovery resend
（`00:09:01.754`）の直前に、待避されていた「ま」「え」（3VK）がこの経路で
flush された。

## なぜ実害になるか（journal + app_log の実測、Opus round1 で確定）

`report_id: 01M1MW0KSY5KWVYSGPGRBTNSPA` の journal
（`KeyInput.timestamp_us` 基準、`elapsed_ms` は absorb 時刻であり使わない）
と app_log_excerpt を突き合わせたタイムライン:

1. `00:09:00.883` 「な」送信（cold、`prepend_f2_warmup=true`。ただし
   このフラグ名は**実際の F2 送信を意味しない**——予防的 F2 送信は
   2026-07-18 に撤去済みで、実際には `[h1-probe] cold=18
   idle_at_cold=11125ms F2/probe待機省略 → per-VK confirm へ` という経路を
   通る）→ probe cold_seq=18 で `SuspectedLiteral` → `01:266` backspace×1
   + "na" 再送予約、cold mark。
2. `01:294`-`01:297` "na" 再送、probe cold_seq=19 開始。
3. `01:312`/`01:315` 「ま」「え」送信 → probe/recovery in-flight のため
   `pending_deferred` へ退避（累計3VK: M,A,E）。
4. `01:668` cold_seq=19 が2回連続 `SuspectedLiteral` で give-up
   （backspace×1、再送なし）→ `01:676` GJI reinit
   （`VK_IME_OFF`→`VK_IME_ON`）送信・IMC ポーリング開始。
5. `01:751` ポーリングで Hiragana 確認 → `Confirmed`。
6. **`01:754` `[chrome-reinit-retry] retry romaji via normal path: "na"`**
   ログ直後に `[tsf-probe] deferred 3 VK(s) を romaji 直後に送出` が出るが、
   これは `send_romaji_batched_gated` 内で
   `drain_pending_deferred_before_send_if_queue_only()`
   （`vk_send.rs:216`）→ 実送信（`assess_warmth()` 以降、`:241〜`）という
   呼び出し順のため、**ログの見た目に反して drain は "na" の実送信より
   前に発火している**（`vk_send.rs:83` のログ出力位置が発火順を誤解させる
   ——`key_injector.rs:307` のログ文言 `deferred N VK(s) を romaji 直後に
   送出` も、drain 経路（romaji の**前**）と本来の「romaji 直後」経路の
   両方から同一文言で呼ばれ、区別できない）。journal
   `seq 2450 GjiReinitRetryCompleted ... deferred_flushed: 0` がこれを
   裏付ける——**`flush_stale_deferred_vks_after_recovery`（BUG-38 の
   同期点）はこの時点で何も flush していない**。実際に「ま」「え」を
   流したのは ADR-123 decision4-3 の drain である。
7. `01:797` "na" の実送信（`warm=false`、`[h1-probe] cold=20
   idle_at_cold=343ms F2/probe待機省略 → per-VK confirm へ`）→
   probe cold_seq=20（probe_id=37）開始。
8. `01:789`/`01:820`/`01:851` 手順3・6 で先に注入済みの「ま」「え」の
   GJI I/O が3回連続で観測される（`[gji-io] WRITE`）。
9. `01:866`-`01:877` "na" の1文字目 'n'(0x4E) が送信され、直後の per-VK
   confirm が「候補ウィンドウが既に見えている」ことを根拠に
   literal-detect 待ちをスキップして confirmed 扱いにする
   （`[gji-obs] candidate SHOW #80: last_gji_write=15ms ago`＝起点
   `01:855`）。しかしこの起点は 'n' 送信（`01:866`）より**前**の write
   （手順8の一連の「ま」「え」の書き込み、直近は `01:851`）に由来する
   ——**'n' の処理結果ではあり得ない**。手順3・6 の drain が、"na" 自身
   の per-VK confirm の
   証拠を汚染した。
10. `01:939` "na" の2文字目 'a'(0x41) で epoch fencing が「直近の GJI I/O
    が送信時刻に追いついていない」矛盾を検出し `StaleConfirm` と判定
    → `per_vk_recovery_params(is_stale=true, failed_idx=1)` が
    `escape_composition=true` を返す（`literal_detect_fsm.rs:89-92`。
    **`escape` は `failed_idx > 0` のみで決まり、`is_stale` 自体は
    escape に無関係**）。
11. `01:947` `flush_raw_tsf_literal_backspaces`（`tsf/output.rs`）が
    `escape_composition=true` を見て `VK_ESCAPE` を送信し、「現在の
    composition を（何文字分かに関わらず）確実に破棄する」。この時点で
    composition には手順3・6で注入済みの「まえ」も含まれており、
    「な」の未確定分と一緒に破棄される。直後 `01:952` に "na" のみを
    再送し、これが最終的に確定（`gji-fsm` は `01:944` に既に `OnWarm`
    復帰済み——「warm になるまで待つ」対策ではこの窓を防げない）。
12. `03:167` 4モーラ目「ま」送信。この時点で GJI は `OnWarm`・probe
    非稼働のため defer されず、warm 経路でそのまま送信・確定。これが
    出力に現れた「ま」。

**因果の要点（訂正、round2/round3 で保護理由を訂正済み）:** drain（手順6）
が無ければ手順8〜11は起きない。ただし drain をスキップした場合に実際に
何が起きるかは、`resend_gji_reinit_retry_romaji`（`platform.rs:967`、
手順6の "na" 再送）が **cold 経路を通るか warm 経路を通るか** で分岐する
うえ、**キューを守る述語は時点によって交代する**（`01:754` 時点は
`has_pending_tsf()`、`01:944` 時点は `raw_recovery_owns_deferred()`/
INV-F。`01:754` 時点で `raw_recovery_owns_deferred()` が false なのは
`RAW_TSF_LITERAL` が `01:671` に swap 済み、`pending_gji_reinit` が
`platform.rs:943` の `take_gji_reinit_completion` で take 済みのため）:

- **本件のように cold 経路の場合（実際に起きたケース）**: "na" 再送が
  `install_pending_tsf`（`vk_send.rs:282`）を同期実行し
  `has_pending_tsf()` を true にする。`platform.rs:967` 直後の
  `:969 drain_output_post_send_effects()` → `:971
  flush_deferred_vks_after_gji_reinit_completion()`（無条件に呼ばれる
  **第2の flush 地点**）は `take_pending_deferred_if_probe_idle`
  （`tsf_warmup_coord.rs:414-420`）が `has_pending_tsf()=true` により
  `None` を返すため、この時点では何も flush しない。その後 `01:944`
  に "na" 自身の probe（cold_seq=20）が終了し `finish_probe_stage`
  （`output/mod.rs:1566-1577`）に到達するが、その時点では `01:941` の
  `set_raw_literal(0,"na",true)` により `RAW_TSF_LITERAL.romaji` が
  非空になっているため `raw_recovery_owns_deferred()` が**true**——
  INV-F によりここでも flush しない（実ログ `[stage-end] ProbeDone:
  deferred の解放は raw recovery 側に委ねる`）。**実際に flush するのは
  `WM_DRAIN_OUTPUT_QUEUE` 経由の `flush_raw_tsf_literal_recovery`**
  （`output/mod.rs:1786-1802`）で、ESC → "na" 再送 → 末尾の
  `flush_stale_deferred_vks_after_recovery`（`:1877-1891`、BUG-38 が
  確立した順序保証）という順に実行される。**`dispatch_probe_actions` の
  `TransmitTsf`/`TransmitChrome`/`TransmitSingleVk` ハンドラは ADR-103
  決定4-b で deferred 解放を `finish_probe_stage` へ一元化済みのため、
  ここでは flush しない**（`probe_io.rs:583`/`:676` のコメント。
  `tsf_warmup_coord.rs:408-410` の doc コメントは決定4-b 以前の記述の
  まま残っており、修正時に一緒に訂正が必要）。この経路なら「な」+
  「まえ」+「ま」=「なまえま」と正しく出力されていた可能性が高い
  （反実仮想、コードパスを実際に辿って確認済み）。
- **"na" 再送が warm 経路を通る場合（本件では起きなかったが起こりうる、
  実際に手順11の最終再送 `01:952 warm=true prepend_f2_warmup=false` が
  この経路）**: probe が張られないため `has_pending_tsf()` は false の
  まま、`platform.rs:971` が**即座に** `pending_deferred` を flush する。
  この場合、順序は正しく（な→まえ）、per-VK confirm 自体が発生しないため
  証拠汚染も起きない——**本件の2つの実害（順序反転・証拠汚染による
  false-confirm）はどちらも解消する**。ただし「probe を経由しない raw VK
  注入」という残存リスク（`output/mod.rs:1871-1876` が記録、対抗仮説節
  参照）自体は残る。

**副次的な実害（未言及だった別欠陥）:** drain は「まえ」を「な」より
**先に**出力している。「な」自身は `01:671` に一旦 backspace で消えて
いる（reinit 前のクリーンアップ）ため、ESC が無ければ画面には
「まえな」という**順序反転**が見えていたはずである。
`journal_policy::order_violation`（`output/mod.rs:1918-1934`）はバッチ
**内**の `order_token` 単調性しか見ないため、この「drain されたキューと
recovery resend との相対順序」の反転は構造的に検出できない。

## 除外した対抗仮説

**「`01:797` の `prepend_f2_warmup=true` による F2 warmup が composition を
reset/commit したのでは」は成立しない。** 予防的 F2 送信は 2026-07-18 に
撤去済み（`vk_send.rs:255-265`）で、実ログも `[h1-probe] idle_at_cold=343ms
F2/probe待機省略 → per-VK confirm へ` と記録している。`prepend_f2_warmup=true`
/ `forces_f2=true` というログ・変数名は実際の F2 送信を意味しない
（将来の調査者が同じ誤読をしないよう、非 blocker としてログ文言の見直しも
検討する）。`01:754`〜`01:947` の間に外へ出た注入は M,A,E と N,A の5個の
みで、他の注入経路は確認できない。

**composition の実内容は Chrome では読めない**（`himc_null=true`
`comp=- comp_read=-`）ため、「ESC が『まえ』を消した」自体は状態遷移から
の逆算（drain 後に GJI I/O が観測される・その後 ESC が発火する・その後の
出力に「まえ」が無い）であり、composition バッファの直接観測ではない
（BUG-75「推論（一次証拠なし）」節と同じ構造的制約）。

## 決定

### 採用: drain を `DeferGate::Enforced`（通常のユーザー入力）限定にする

`drain_pending_deferred_before_send_if_queue_only` に `gate: DeferGate`
引数を追加し（`ms_ime_gate_defer`/`defer_respecting_gate` が既に採用して
いる「呼び出し元から `gate` をそのまま引き継ぐ」パターンに揃える、
`vk_send.rs:499` 参照）、冒頭で早期 return する（round2 指摘: 「呼び出し
側で `if gate == Enforced { self.drain...() }` と分岐する」形では既存の
2ユニットテストが `o.drain_pending_deferred_before_send_if_queue_only()`
を引数なしで直接呼んでおり（`:797`/`:812`）、修正後の関数は
`install_pending_tsf`/`SendInput` に到達する呼び出し元経由でしか
テストできなくなる。関数シグネチャに `gate` を持たせることで、既存
テストは引数追加のみで通り、新規テストも1行の直接呼び出しで書ける）:

```rust
fn drain_pending_deferred_before_send_if_queue_only(&self, gate: DeferGate) {
    if gate != DeferGate::Enforced {
        return;
    }
    if self.is_probe_or_recovery_blocking(true) || self.pending_deferred_len() == 0 {
        return;
    }
    let n = self.flush_pending_deferred_vks();
    if n > 0 {
        log::debug!(
            "[pending-deferred] drain-before-send: 新規モーラの前に取り残し {n} VK(s) をflush"
        );
    }
}
```

呼び出し元 `output/vk_send.rs::send_romaji_batched_gated`（`:216`）と
`send_romaji_as_tsf_gated`（`:392`）は、既に保持している `gate` 引数を
そのまま渡す（`Exempt`——recovery resend・ADR-101 決定3 retry——では
drain しない）。

**根拠:**

- ADR-123 決定4-3 自身の意図（ログ文言 `新規モーラの前に取り残し…をflush`、
  ADR-123 本文「新しいモーラを追加でキューに積むのではなく先に flush」）は
  「新規モーラ」を対象にしている。recovery resend は新規モーラではない。
- `defer_respecting_gate`（ADR-123 決定4-2）は既に `gate` で
  `raw_recovery_owns_deferred()` の扱いを切り替えている。drain 側だけが
  この非対称を欠いているのは実装漏れであり、意図された設計ではない。
- drain をスキップしても、キューを解放しうる経路は他に4つ残るため、
  永久に滞留する（飢餓する）ことはない（round2 指摘: 当初列挙した3経路
  は不完全だった）:
  1. 新規の本物のモーラの drain-before-send（`gate=Enforced` の次回呼び出し）
  2. `finish_probe_stage`（`output/mod.rs:1566-1577`、INV-F 解放）
  3. `flush_raw_tsf_literal_recovery` 末尾（`output/mod.rs:1786-1802`、
     give-up 後のクリーンアップ）
  4. `flush_deferred_vks_after_gji_reinit_completion`
     （`platform.rs:971`（Confirmed 分岐）・`:977`（Timeout 分岐）、
     reinit retry 完了直後の無条件 flush——上記「因果の要点」で説明した
     "na" 再送が warm 経路を通る場合はここが実質的な主 flush 地点になる）
  5. `discard_pending_deferred_after_stale_gji_reinit`（`platform.rs:985`、
     flush ではなく破棄だが、キューを空にする経路として飢餓判定には
     含める）

  `DEFERRED_QUEUE_CAP=32` の上限安全弁も維持される。

### 却下: 案(a)（`VK_ESCAPE` の代わりに backspace で元モーラ分だけ消す）

3点で成立しない:

1. **既存の不可侵 invariant と正面衝突する。**
   `literal_detect_fsm.rs:63-84` の doc が「`VK_ESCAPE` の破壊スコープは
   pending composition 内に閉じているのに対し、`VK_BACK` は composition
   スコープ外の既に確定済みのテキストにも届く唯一の不可逆操作」と明記し、
   BUG-33 追補3・4（「リーク」の「ク」、「cold 」の末尾スペースを誤消去
   した実害2件）を根拠に `is_stale || failed_idx > 0` では `backs=0` を
   選ぶ設計にしている。案(a)はこの2件を再導入する。
2. **消すべき文字数を知る手段が原理的に無い。** VK 数とかな数は一致しない
   （"na" 2VK→1かな、"ltu" 3VK→1かな、`literal_detect_fsm.rs:50-53` が
   自認）。加えて Chrome では composition 文字列を読めない
   （`himc_null=true`）。
3. **消す向きが入力順序に依存する。** ESC 時点の composition が「まえ+な」
   なら backspace は末尾の「な」から正しく消せるが、drain が正しい順序
   （な, まえ）で流していれば残すべき「まえ」が末尾に来て破綻する。
   「バグった順序の上でだけ成立する」修正は採らない。

### 保留（再定式化が必要、現行形は却下）: 案(b)（composition 確定を待ってから flush）

「recovery 中は `pending_deferred` に触らない」という不変条件（INV-F）は
既に実装済みで、本件でも `finish_probe_stage` は正しく発火した（`01:944`
`[stage-end] ProbeDone: deferred の解放は raw recovery 側に委ねる`）。
穴は「recovery 中に触った」ことではなく、**recovery のラウンド間
（give-up→reinit confirmed の間、本件では約43ms）で `is_probe_or_recovery_blocking`
の全項が false になる隙間**にある。

本件のログでは「確定」を示す2つの候補シグナルがどちらも反証される:

- `GjiFsm::OnWarm` 復帰は ESC の**3ms前**に既に成立していた（`01:944`）。
  「OnWarm を待つ」では防げない。
- per-VK `CompositionConfirmed`（`01:877`、'n' について発火）はそれ自体が
  手順9の false-confirm（drain された「まえ」の候補ウィンドウを自分の
  証拠と誤読）であり、「安全に確定した」ことを意味しない。

案(b)を採るなら「新しい確定シグナルを探す」ではなく「recovery のラウンド
間の隙間そのものを埋める」——つまり採用案（drain を `Enforced` 限定に
する）とほぼ同じ結論に収束する。よって案(b)は独立の対策としては保留し、
採用案で十分かを実機ソークで確認してから要否を判断する。

### 格下げ: 案(c)（まず計測してから決める）→ 修正の代替ではなく独立の計装

ADR-100 決定4-a への引用は不正確だった（決定4-a は「give-up の実機発生
頻度」が対象で、本件の「escape と drain の近接」とは別軸）。加えて
ADR-100 本文（決定4-aの経緯節）は「計測期間中もユーザーは文字を失い
続ける」ことを**失敗モードとして名指し**しており、「まず計測」を正当化
する根拠にならない。`tuning-constants.md` の実測義務は `_MS` 定数変更が
対象で、本件のような gate 条件追加には適用されない。

一方、drain-before-send の flush は現状 journal に一切記録されていない
（`DeferredRecoveryFlush` は `trigger: "raw_recovery"` のみで、drain 経路
専用の記録が無い）ため、次に同種の報告が来たときの診断能力として
`journal.rs::JournalEntry::DeferredRecoveryFlush` に
`trigger: "drain_before_send"` を追加することは、採用案の実装と**独立に**
価値がある。同じ PR に含めてよい。

## テスト

`output/vk_send.rs` の既存テスト群
（`drain_pending_deferred_before_send_if_queue_only_preserves_queue_while_blocking`
等、`:770-`）に、`gate == Exempt` では
`drain_pending_deferred_before_send_if_queue_only` 相当の呼び出しが
キューを保持すること（blocking していなくても drain しないこと）を検証
する回帰テストを追加する。`#[cfg(windows)]` 配下のため Linux の
`cargo test` には出現しない——`cargo check --target x86_64-pc-windows-msvc
-p awase-windows --tests --lib` で確認し、実行は `windows-build` CI に
委ねる（`fix-requires-evidence.md` の (a) を満たす）。

**あわせて修正する stale doc comment:** `tsf_warmup_coord.rs:408-410` の
`take_pending_deferred_if_probe_idle` doc は「`dispatch_probe_actions` の
`TransmitTsf`/`TransmitChrome`/`TransmitSingleVk` は自身の送信直後に
無条件で `take_pending_deferred` を呼ぶ」と書いているが、これは ADR-103
決定4-b（deferred 解放を `finish_probe_stage` へ一元化）以前の記述が
取り残されたもので、現状の実装と食い違う（round3 で本 ADR 自身がこの
stale な記述を引用して一度誤った）。実装 PR で一緒に訂正すること。

## 未決定事項

1. **`WarmupAborted` 経路での shadow/実体の乖離（Non-blocker、本 ADR に
   含めるか別 BUG にするか）:** `cancel_probe`（`output/mod.rs:1636-1647`）
   の doc は「片方だけ残すと shadow と実体がずれ、残った VK は誰にも
   所有されないまま、はるか後の無関係な回収でまとめて送られる」と
   明記しているが、`WarmupAborted` は `cancel_probe` を通らず
   `finish_probe_stage` 経由のため、GjiFsm の shadow pending は破棄され
   ても `pending_deferred` は温存される非対称が本件ログでも観測された
   （`01:670` `DiscardPending count=3 reason=WarmupAborted`）。**採用案
   （drain を Enforced 限定にする）は `pending_deferred` の滞留時間を
   延ばす方向に働くため、この乖離が露出する窓はむしろ広がる**（round2
   指摘）。実機ソーク時の観測ポイントに含め、別症状として再発すれば別
   BUG として起票する。
2. **`WrongOrderDetected` 相当の drain⇔recovery-resend 間順序検出を
   `journal_policy::order_violation` に拡張するか。** 現状はバッチ内
   トークンしか見ておらず、本件のような「キュー外の再送との相対順序」の
   反転は検出できない。採用案で本件は解消するため優先度は下げるが、
   将来別の drain 系欠陥の早期検知に使える可能性がある。
3. **per-VK confirm の「候補可視ショートカット」自体の構造的弱点
   （Non-blocker、本 ADR のスコープ外）:** 手順9の false-confirm が成立
   したのは、per-VK 経路が候補ウィンドウの可視をそのまま confirm の証拠に
   使い、候補可視 veto（`probe_io.rs:659-670` `veto_eligible`）がこの
   経路に配線されていないため。本 ADR の採用案は汚染源の1つ（recovery
   resend 前の drain）を除くが、`Enforced` drain が流した本物の新規
   モーラや他アプリ由来の SHOW でも同型の false-confirm は起こりうる。
   veto 配線は [ADR-122](122-cold-start-per-vk-confirm-race-recovery.md)
   決定案のスコープ（未実装）。

## 関連

[BUG-109](../known-bugs.md)（本 ADR の起票根拠、訂正版タイムライン・
一次証拠）、[ADR-123](123-focus-resync-and-probe-defer-queue-composition-race.md)
（決定4-3・drain-before-send を導入した当該 ADR。本 ADR はその実装漏れの
修正）、BUG-38（give-up 後の `pending_deferred` flush 漏れ、その同期点は
本件で一度も flush していない点で無関係）、BUG-75（`StaleConfirm` 回収の
`escape_composition` 判定そのものの由来）、BUG-33 追補3・4（`VK_BACK` を
避け `VK_ESCAPE` を使う設計の由来、案(a)却下の根拠）。
