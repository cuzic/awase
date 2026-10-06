---
id: ADR-229-companion-229-opus-fcis-round2
title: |-
  ADR-229 FCIS 設計 opus-adversarial-consult round2
type: companion-doc
related_adr:
  - "ADR-229"
  - "ADR-224"
---

# FCIS 設計ドラフト r2 の確認(round2)

- 対象: `fcis-design-draft-r2.md`。
- 実測の基準: `origin/develop` = `d0d42be6`(#492〜#495 は未マージ)。PR マージ後の想定は、各 PR の差分から導いた。
- 読み取りのみ。

## 0. 結論

- **round1 の B1・M1〜M4・S1〜S5・N1・N2 は、意図どおりに直っている。設計の骨格(2 段の定義、HubClock を標準形に、handler の例外、核と殻の分割)は収束した。**
- ただし、実コードを読み直して**新たに Must が 3 つ**見つかった。どれも骨格を変える話ではなく、r3 で直せば収束と判断する(round3 は差分の確認だけでよい)。
  - **M5**: P2 の「殻は 1 行、呼び出し元の変更 0」が `note_awase_write_for_mode_key_pass` で成り立たない。このメソッドは runtime からではなく、**core のメソッド(`record_optimistic`/`record_confirmed`)の中から**呼ばれている。殻にすると、連鎖が `record_ime_apply_result` まで伸び、`architecture_guard.rs:1620` の件数固定にも触れる。
  - **M6**: P3/P4 で `ImeStateHub::new()` を `#[cfg(windows)]` にすると、`platform_state` の中のテストで `PlatformState::new()` を呼ぶ **37 か所**が Linux で構築できなくなる。さらに、`effective_open()`(壁時計を読む)を使うテストが **19 か所**あり、Linux 用の時計の選び方しだいで結果が変わりうる。r2 には、この扱いが無い。
  - **M7**: F-D5-1 と F-D1 の整合の文言(「handler 例外も 1 turn の中で完結する」)は、`run_chain_async` と矛盾する。`AsyncMechanismWriter` は `.await` をまたぎ、複数の turn に分かれる。守るべき不変条件は「1 turn で完結」ではなく、「await の後は必ず観測し直す」(`fallback_write` が機構ごとに view を作り直し、3 関数が独立に gate を再検出する。INV-45)。
- §8 の 6 点への答えは §2 にある。

---

## 1. round1 の指摘の反映状況

| 指摘 | 状況 | コメント |
|---|---|---|
| B1(2 段の定義) | 済 | §1 の Tier-1/Tier-2 は正確。ただし Tier-2 の違反の実測が r2 の許可リストより多い(§2-3、S6) |
| M1(HubClock) | 済 | 2 つの時間軸と「4 つ目を作らない」も入った |
| M2・M3(handler の例外) | 済(文言は要調整) | 濫用を防ぐ条件が足りない(S7)。`romaji_pre_write` の分類が誤り(S8)。F-D5 との整合の文言は M7 |
| M4(核と殻) | 済(実現性は要修正) | M5 |
| S1(実例の訂正) | 済 | |
| S2(殻を別ファイル) | 済 | 置き場所は子モジュールを推奨(S9) |
| S3(既存の世代の対応表) | 済 | 正確 |
| S4(HIMC) | 済 | |
| S5(共通の後処理) | 済 | 「着手前に、テストが呼ぶ識別子が gated 側にないか確認する」も入った。良い |
| N1・N2 | 済 | |

---

## 2. §8 の 6 点への回答

### (1) 直し方に新しい誤りが無いか

M5・M6・M7 と、S6〜S9 を参照。それ以外の記述(用語、層の図、F-D2〜F-D4、レシピ R1〜R7、指標)は事実と合っている。

### (2) handler の例外の文言と F-D5-1

- **濫用の恐れ(S7)**: 「途中の結果で次の手を決める」は、F の大半(読む → 判断 → 書く)が緩く当てはまってしまう。F の分割の実装者が「これも途中の結果で決めている」として何でも trait にできる。
  - 閉じた条件にする。(a) **同じアルゴリズムの中で、前の効果の結果が次の効果の選択を変える**(単に「読んでから書く」はサンドイッチで書ける)。(b) handler の trait のメソッドは**その効果だけ**を行い、他の環境を読まない。(c) trait は core に、本番の実装は shell に、**偽物の実装がテストに必ずある**。(d) **例外は ADR の列挙(閉じたリスト)にある関数に限り、追加には ADR の改訂が要る**。
  - (d) が無いと、F-D6 のテキスト走査は handler 経由の呼び出しを検出できない(trait 呼び出しは普通のメソッド呼び出しに見える)ので、事実上、無制限になる。
- **F-D5-1 との整合(M7)**: 同期の handler(`MechanismWriter`、将来の `ImeProbe`)は 1 turn の中で完結するので矛盾しない。非同期の handler(`AsyncMechanismWriter`、`open_chain.rs:132-148` の `write` は ImmCross で `.await`)は turn をまたぐ。
  - 直し方: F-D5-1 を「同期の処理は 1 turn で完結」に限定し、F-D5 に 4 番目の不変条件を足す。「**非同期の handler は、各 `.await` の後に、環境(view・gate・フォーカスの世代)を観測し直してから次の効果を選ぶ。await をまたいで借用も推測値も保持しない**」。根拠は `open_chain.rs:35-59`(INV-45、async だけ `WriteMechanism::ALL`)、`:459-555`(`fallback_write` が機構ごとに view を作り直す)、ADR-180 決定1(3 関数が独立に gate を再検出する)。
  - r2 の 63 行目「その場合も借用を保持しない」と 101 行目「handler trait 例外も 1 turn の中で完結する」は、この形に直す。

### (3) F-D6 の許可リストの再実測と `CORE_MODULES` の候補

再実測(`origin/develop`、`#[cfg(test)] mod tests` より前の本番コード、コメント行を除く):

| ファイル | 行 | 内容 | r2 の許可リスト |
|---|---|---|---|
| `state/ime_model.rs` | 435 | `self.effective_open_at(Instant::now())` | ○ 一致 |
| `state/ime_model.rs` | 504 | `monotonic: Instant::now(),` | ○ 一致 |
| `state/hub_clock.rs` | 41 | `base: Instant::now(),`(`manual()`) | ○(2 件のうち 1) |
| `state/hub_clock.rs` | 51 | `Self::Wall { .. } => Instant::now(),` | ○(2 件のうち 1) |
| `state/ime_event_log.rs` | 41 | `self.record_at(event, tick_ms, Instant::now())`(develop では gated、#492 マージ後に ungated) | ○ |
| `state/probe_admission.rs` | 71 | `static REJECTION_COUNTERS: RejectionCounters`(可変) | ○ |
| `state/ime_profile_driver.rs` | 186-188 | `IMM_CROSS_DRIVER`・**`IMM32_UNAVAILABLE_DRIVER`**・`TSF_NATIVE_DRIVER`(不変) | r2 は 2 つしか挙げていない(**3 つ**。Nit) |

**r2 の許可リストに無い違反**(「core のファイルに `#[cfg(windows)]` 項目が無い」「FS を持たない」の規則で数えた。`mod.rs` の gate 宣言 8 件は別扱い):

- `state/ime_event.rs:36, 46`: `#[cfg(windows)] impl HwndId { fn to_hwnd() }` / `impl From<HWND> for HwndId`。HWND との変換で、殻の役割。
- `state/key_effect_predictor.rs:570, 580`: `get_gji`/`get_native`(`gji_charset_autodetect`・`msime_key_assignment` の FS/レジストリを読む)。
- `state/key_effect_runtime.rs:377, 385, 393, 408, 440, 816`: 学習済み表のパス解決・`fs::metadata`・読み込み(**FS**)。
- `state/probe_admission.rs:325`: `#[cfg(windows)]` の関数。

→ **`#[cfg(windows)]` の項目が 11 件・4 ファイル**ある。r2 の「初期の許可リスト 5 件」のままでガードを入れると落ちる。

- #492〜#495 をマージした後の想定
  - #492 で `ime_event_log.rs` が ungated になる(許可リストの 1 件として既に勘定済み)。
  - #493 で `physical_disposition.rs` に `#[cfg(any(windows, test))]` が 1 件入る(規則で許可)。
  - #494 で `focus/hwnd_cache.rs`(違反 0。ただし `focus/mod.rs` の `allow(dead_code)` は S4 で外す見込み)。
  - #495 で `journal.rs` の `#[cfg(windows)]` が 2 件(dump 関数)と、`cfg_attr` が 1 件(S5 で `any(windows, test)` へ)。journal を core に入れるなら違反 2 件。ただし journal は記録係で、core に入れる必要は薄い。

**推奨(S6): 許可リスト方式をやめ、「違反 0 のファイルだけを `CORE_MODULES` に載せる」方式にする。**

- ungated な `state/` の 53 ファイルのうち、上の 4 つの規則(壁時計・可変 static/`thread_local!`・`#[cfg(windows)]`・FS/env)の違反が 0 のファイルは **45**。
  - `actuation_chain`、`actuation_decision_record`、`alt_impersonation`、`app_ime_policy`、`app_suppression`、`belief`、`conv_after_open`、`conv_classify`、`conv_mode`、`drift_correction`、`eisu_recovery`、`event_origin`、`evidence`、`explicit_press`、`external_change_watch`、`focus_probe_plan`、`focus_resync_policy`、`force_guard`、`generation`、`gji_direct_mechanism`、`half_width_alnum`、`hook_state`、`hook_watchdog`、`ime_actuation`、`ime_actuation_decision`、`ime_kind`、`imm_evidence`、`injection_mode`、`input_barrier`、`intent_store`、`key_effect_table`、`key_sequence_policy`、`keymap_initial_hypothesis`、`keymap_latch`、`layout_language`、`mode_key_pass`、`observation_store`、`open_warrant`、`physical_disposition`、`post_bypass`、`press_ledger`、`scoped_latch`、`state_dependent_key_warning`、`transition`、`win_key_guard`
- 違反のある 8 ファイル(`hub_clock`・`ime_event`・`ime_model`・`ime_profile_driver`・`key_effect_predictor`・`key_effect_runtime`・`probe_admission`・`mod.rs`)は、**最初は載せない**。直したら載せる。
- 指標は「`CORE_MODULES` の件数」(増やす)と「違反のある ungated ファイルの件数」(8 → 減らす)。
- 例外を書き並べる許可リストより単純で、ガード 1 本で済む。「ime_model.rs の 435 行だけ許す」のような行番号依存が無く、壊れにくい。`hub_clock.rs` は時計の実装そのものなので、恒久的に載せない(Tier-2 の外)と明記する。
- `#[cfg(windows)] mod shell;` のような**モジュール宣言**は、規則の対象外にする(S9 の殻の置き場所のため)。

### (4) P1〜P3 と ReplayWriter の並行可否、P2 の殻、P3 の `with_clock`

**ファイルの重なり**

| タスク | 主に触るファイル |
|---|---|
| P1 | `win32.rs`、`observer/ime_observer.rs`、新しい `state/foreground_scope.rs`・`state/ime_update.rs`、`state/mod.rs` |
| P2 | `state/platform_state.rs`、新しい殻のファイル、(M5 により)`architecture_guard.rs:1620` |
| P3 | `state/platform_state.rs`(`new`/`Default`/`PlatformState::new`)、(M6 により)そのテスト |
| ReplayWriter | `state/actuation_decision_record.rs`(テスト)。ImmCross の command 統合を含めるなら `state/ime_actuation_decision.rs`・`runtime/open_chain.rs` |

- **P2 と P3 は同じファイルを触るので、並行させずに P2 → P3 の順にする(またはまとめる)**。
- **P1 は #495 と `win32.rs` で重なる**(#495 は `SentKeyEvent` を消し、P1 は `ForegroundScope` を移す)。#495 のマージ後に着手する。
- ReplayWriter は P1〜P3 と重ならない(並行可)。

**P2: 11 メソッドの殻は 1 行で書けるか(全部読んだ)**

- 10 メソッドは 1 行で書ける: `arm_mode_key_pass_mark`(`:304-307`)、`mode_key_pass_expiry_wait_ms`(`:309-315`)、`mode_key_pass_window_remaining_ms`(`:338-344`)、`mode_key_pass_mark_live`(`:346-348`)、`expire_mode_key_pass_mark`(`:358-365`)、`arm_external_change_watch`(`:415-421`)、`external_change_watch_remaining_ms`(`:427-433`)、`follow_external_change`(`:439-462`。`foreground_scope()` を読むのは 1 回で、`scope` を 2 回使う。`_in_scope(read, now_ms, tick_ms, accepted, scope)` へ本体を移せば殻は 1 行)、`align_after_expired_mode_key_pass`(`:524-533`)、`invalidate_intents_if_mode_key_pass_live`(`:556-566`)。
- **M5: `note_awase_write_for_mode_key_pass`(`:322-324`、private)だけは書けない。** 呼び出し元は runtime ではなく、core のメソッド `record_optimistic`(`:588-592`)と `record_confirmed`(`:599-602`)。さらに `record_confirmed` は `record_ime_apply_result`(`:1071-`、`:1086`)の中から呼ばれる。
  - これを殻に出すと、`record_optimistic`(本番の外部呼び出し 1: ime_refresh)、`record_confirmed`(4: focus_tracking・ime_refresh・key_pipeline・runtime/mod)、`record_ime_apply_result`(1: runtime/mod)の 3 つも、`_in_scope` 版と殻に分ける必要がある。
  - 殻のメソッド名を保てば、runtime の呼び出し元の変更は 0 のまま。ただし core の中の `record_ime_apply_result` → `record_confirmed` は `record_confirmed_in_scope(.., scope)` になる。そのため、**`architecture_guard.rs:1620` の `RECORDERS`(`.record_confirmed(` = 5、`.record_optimistic(` = 1。INV-A97-1/BUG-69 のガード)の数え方を変える必要がある**。IME belief ファミリーのガードなので、件数の意味を保つ書き換え(`_in_scope` 版も数える)を同じ PR でする。
  - 殻は合計 13(10 + 3)。r2 の「11 メソッド、殻は 1 行、呼び出し元の差分 0」は、「13 個の殻、runtime の差分 0、ガード 1 本の数え方を変える」に直す。
- **テスト側の費用(P4 に響く)**: `platform_state.rs` のテスト(`:1814-`)は、殻の名前のメソッドを 16 か所(`follow_external_change` 9、`arm_external_change_watch` 3、`record_ime_apply_result` 3、`record_confirmed` 1)で呼び、**`crate::win32::foreground_scope()` を直接 9 か所で呼んでいる**。Linux では殻も `foreground_scope()` も無いので、P4 ではこれらを `_in_scope` + `test_foreground_scope()`(`:2561`、既存)に書き換える必要がある。r2 の P4「gate を外すだけ」は過小。
- **殻の置き場所(S9)**: 別ファイルの sibling モジュール(`state/platform_state_shell.rs`)にすると、private な `_in_scope`(例: `note_awase_write_for_mode_key_pass_in_scope` は `fn`)を `pub(super)` に広げる必要がある。**`platform_state.rs` の中に `#[cfg(windows)] mod shell;`(ファイルは `state/platform_state/shell.rs`)の子モジュールにすれば、親の private な項目をそのまま呼べて、可視性を広げずに済む**。ガードの `read_crate_file("src/state/platform_state.rs")` も影響を受けない。

**P3: `with_clock` と ungated な `new()` の使い手(M6)**

- `ImeStateHub::new()`(`:133`)は `PlatformState::new()`(`:1798-1806`)と `impl Default for PlatformState`(`:1808-1810`)から呼ばれる。`PlatformState::new()` の呼び出しは、本番では `app/bootstrap.rs:706`(gated)の 1 か所、**`platform_state.rs` のテストでは 37 か所**。`tests/support/harness.rs` は `ImeStateHub` を使っていない(gated なので使えない)ので、P3 で壊れるものは無い。
- **問題は P4 で表に出る**。`new()` を `#[cfg(windows)]` にすると、Linux のテストは `PlatformState` を構築できない。その上、テストのうち **19 か所が `effective_open()`(内部で `self.clock.now_tick()` = 壁時計)を使い、`TickMs(100)` などの小さな値で意図を記録している**。Windows では `GetTickCount64`(起動からの経過、通常は 30 秒の TTL より十分大きい)と比較している。Linux 用の時計を「プロセス開始からの ms」にすると、同じテストで TTL の判定が変わり、結果が反転しうる(**未確認**だが、起こりうる)。
- **直し方(推奨)**
  - (a) P3 で `with_clock(clock)` を足す。テストのヘルパーとして `PlatformState::for_test(clock: HubClock)` を用意し、テストの 37 か所を**時間が意味を持つものは `HubClock::manual(<Windows と同じ大きさの基準 tick>)`、それ以外は同じヘルパー**に置き換える。これは P3 で(まだ gated のまま)行い、windows-build で全テストが従来どおり pass することを確かめる。
  - (b) P4 で gate を外し、Linux で同じテストが pass することを確かめる。
  - (c) 本番の `new()` は `#[cfg(windows)]` のまま `HubClock::wall(crate::hook::current_tick_ms)`。これで `architecture_guard.rs:1382` は通る。
  - 「Linux 用の壁時計を `cfg` で切り替える」案は、時間軸の意味が変わるので採らない。

### (5) 5.3 の F の分割の順序と、Tier-2 のガードを入れる時期

- 順序(`ir_decide_read_strategy` → `plan_set_open` → `execute_relay`/`drain_deferred` → `DriftPlan` → focus → Output)は妥当。純粋な判断の割合が高く、ファミリーへの影響が小さい順になっている。
  - 注意 1: `execute_relay`/`drain_deferred` は defer/replay キューのファミリー(ADR-156)。defer 側と drain 側の 2 窓口を、同じ PR で必ず対にする旨をタスク表に書く。
  - 注意 2: `ir_apply_drift_correction` は、閉ループのハーネスが写している 2 系統のうちの 1 つ。`DriftPlan` を core に出すと、写しを本物に置き換えられる(写し 7 → 1 への道)。指標に結び付けておく。
- **Tier-2 のガードを入れる時期**: S6 の「違反 0 のファイルだけを載せる」方式なら、既に 45 ファイルが条件を満たしている。**最初の分割と同じ PR を待たず、P0 として単独で先に入れる**のを勧める。費用はテスト 1 本と定数 1 つで、CI だけで成否が決まる。後続の分割 PR は「新しい core のファイルを `CORE_MODULES` に足す」だけになり、PR ごとの論点が減る。同じ PR に混ぜると、ガードの書き方の議論と分割の議論が 1 つの PR で絡む。

### (6) 収束したか

**骨格は収束した。** M5・M6・M7 は、文言とタスクの粒度の修正で直る(設計の方向を変えない)。r3 でこれらと S6〜S9 を直せば、round3 は差分の確認だけで、ADR-229 の改訂文とタスク表に進んでよい。

---

## 3. 新しい指摘

| ID | 重大度 | 指摘 | 直し方 |
|---|---|---|---|
| M5 | **Must** | P2 の殻が `note_awase_write_for_mode_key_pass` で 1 行にならない(core の `record_optimistic`/`record_confirmed` → `record_ime_apply_result` の連鎖)。`architecture_guard.rs:1620` の `RECORDERS` の件数固定に触れる | §2-(4) のとおり、殻を 13 個にし、ガードの数え方を同じ PR で直す。P2 の成否の判定に「`RECORDERS` のガードが同じ意味で green」を足す |
| M6 | **Must** | P3/P4 で、テストの `PlatformState::new()` 37 か所が Linux で構築できない。`effective_open()` を使う 19 か所は、時計の選び方で結果が変わりうる。P4 のテストの書き換え(殻の名前 16 か所、`foreground_scope()` の直接呼び出し 9 か所)も見積もりに無い | §2-(4) の (a)〜(c)。テストの構築を P3 で `for_test(HubClock)` に寄せ(gated のまま windows-build で確認)、P4 では gate を外すだけにする。P4 の取りやめ条件に「Linux で結果が変わるテストが出たら止めて、時間軸の違いか本物のずれかを切り分ける」を足す |
| M7 | **Must** | F-D5-1 と F-D1 の整合の文言が `run_chain_async`(await をまたぐ)と矛盾する | §2-(2) のとおり、F-D5-1 を同期処理に限定し、「非同期の handler は各 await の後に観測し直す」を F-D5 の 4 番目に足す |
| S6 | Should | F-D6 の許可リスト 5 件は不完全(`#[cfg(windows)]` 11 件・4 ファイル、FS、不変の static は 3 つ) | 「違反 0 のファイルだけを載せる」方式(初期 45 ファイル)にする。指標は `CORE_MODULES` の件数と、違反のある ungated ファイルの件数(8) |
| S7 | Should | handler の例外の条件が開いている(濫用の恐れ) | §2-(2) の (a)〜(d)。特に (d)「ADR の閉じた列挙にある関数だけ、追加は ADR の改訂」 |
| S8 | Should | 例外 2 の「条件付きの同期読み取り」の例に `romaji_pre_write` が入っているが、これは読み取りではなく**条件付きの書き込み**(`SendMessageTimeoutW` でローマ字モードを設定する、BUG-34 E-prep)。「focus の段階的な分類」の UIA は非同期で、同期読み取りではない | 例外 2 は「条件付きの同期の効果(読み取り・書き込み)」と言い直すか、`romaji_pre_write` を例外 1(chain)側の前処理として扱う。focus は「MSAA までの同期の段階」に限定し、UIA は A 種の Cmd/Event とする |
| S9 | Should | 殻を sibling の別ファイルに置くと、private な `_in_scope` の可視性を広げる必要がある | `platform_state.rs` の中の `#[cfg(windows)] mod shell;`(子モジュール、`state/platform_state/shell.rs`)にする。F-D6 の規則で「`mod` 宣言の `#[cfg(windows)]` は可」とする |
| N3 | Nit | F-D6 の不変 static の例が 2 つ | `IMM32_UNAVAILABLE_DRIVER` を足して 3 つ(`ime_profile_driver.rs:186-188`) |
| N4 | Nit | §6 の指標 2「`platform_state` の ungate で 5」 | P4 の後に P5 を 1 系統ずつ進めて初めて減る(ungate 自体では減らない)と明記する |

---

## 4. ADR-229 の改訂文とタスク表に落とすときに失われやすい点

1. **Tier-1 と Tier-2 は別の作業で、別のガード**: ungate(コンパイラ)と `CORE_MODULES`(テキスト走査)。改訂文で「core」という語をどちらか一方の意味だけで使う(混ぜると B1 に戻る)。
2. **handler の例外は閉じた列挙**: `run_chain(_async)` と、条件付きの同期の効果(個別に列挙)。「途中の結果で次の手を決めるもの」という一般則だけを書くと、実装者に濫用される(S7)。
3. **F-D5 の 4 つ目の不変条件**(非同期の handler は await の後に観測し直す)を、open_chain を書き換えない理由とともに残す(M7)。
4. **P2 の殻は 13 個**で、`architecture_guard.rs:1620` の `RECORDERS` の数え方を同じ PR で直す(M5)。「11 個・差分 0」と書き写さないこと。
5. **P3 は時計の分割だけでなく、テストの構築の付け替え(37 か所、時間が意味を持つものは `Manual`)を含む**。P4 は gate を外すことと、殻の名前・`foreground_scope()` の直接呼び出しのテストの書き換え(16 + 9 か所)(M6)。
6. **順序の制約**: P1 は #495 のマージ後(`win32.rs` が重なる)。P2 → P3 は直列(同じファイル)。ReplayWriter は並行できる。
7. **`HubClock` は Tier-2 の外**(時計の実装そのもの)。「時計を持つ core は `HubClock` 経由」と「`hub_clock.rs` 自身は `CORE_MODULES` に入らない」の両方を書く。
8. **`CORE_MODULES` は「違反 0 のファイルだけ」**で、許可リストは持たない。初期 45 ファイル、違反あり 8。違反の内訳(`ime_event.rs` の HWND 変換、`key_effect_runtime.rs` の FS など)は、それぞれ「殻へ出す候補」としてタスク表に載せる。
9. **ADR-224 の改訂は所有者の判断**で、ADR-229 の改訂とは別に記録する。段階 2 を「核と殻の分割」で進めることが、ADR-224 の案A の懸念 (b) を解消する理由(ハーネスが `_in_scope` に任意のスコープを渡せる)を 1 行で残す。
10. **全レシピ共通の後処理**(表・pre-push・mutants・instrument の一覧・「Linux で実行されないから」のコメント)と、**着手前の「テストが呼ぶ識別子は gated 側にないか」の確認**を、タスク表の各行の成否の判定に入れる(#493 の M1 と S2 の再発防止)。
11. **`allow(dead_code)` を増やさない方針**: Linux で未使用の `pub(crate)` 項目には `#[cfg(any(windows, test))]` か `#[cfg(windows)]`。外から到達できる `pub` 項目には何も付けない(#494 の S4)。
12. **ローカルでビルドしない**ので、各タスクの成否は CI のどのジョブのどのログで見るか(Linux `test` のテスト名の列挙、`never used` が 0 件、windows-build の lib テスト)を、タスク表に具体的に書く。
