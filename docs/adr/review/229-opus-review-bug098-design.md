---
id: ADR-229-companion-bug098-design-opus-review
title: |-
  BUG-098 修正の設計の opus-adversarial-consult(案 D を推奨)
type: companion-doc
related_adr:
  - "ADR-229"
  - "ADR-108"
---

# Opus レビュー: BUG-098 修正の設計(bug098-design.md)

レビュー日: 2026-10-05。読み取り専用。基準は `origin/develop`(fb81a030。設計文書の基準 efe5248e から docs だけ進んでおり、対象のコードは同じ)。行番号は `git show origin/develop:<path>` のもの。

## 結論(先に)

- 設計文書が挙げた事実のうち、**M-3 の照合が既にあること・G1・G2 は正しい**。案 A は G1・G2 を閉じ、**再入時に正しい完了を誤って捨てることにもならない**(§1-2)。
- ただし、**リスク分析に事実の誤りが 2 つある**。(i) R1「`ime_mode_focus_gen` は同一プロセス内の hwnd 変更でも進む」は誤り。進むのはプロセスが変わったときだけ(§1-3)。(ii) G3 は `applied` の記録については**害がない**。実害は「OFF の書き込み先が捕獲されていない」こと(書き込みの問題)で、完了の記録の問題ではない(§1-4)。
- **推奨は案 D**(コードは変えず、BUG-098 の記録を現状に合わせて訂正し、実害を測る手順を書き残す)。案 A は「journal・ログで実害が 1 件でも見つかったら入れる」準備済みの案として残す(§4)。

---

## 1. 設計文書の事実の確認

### 1-1. M-3 の照合が既にあること — 正しい

`runtime/key_pipeline.rs:1351-1359`(develop)に、`run_open_chain_async(...).await` の後で `crate::with_app(|app| app.platform.output.ime_mode_focus_gen.get()).is_some_and(|g| g != focus_gen)` が真なら `UnsafeToToggle` に落とす処理がある。`focus_gen` を捕獲するのは spawn の前の `:1304`。完了を送るのは `:1362` の `post_async_ime_apply_complete(open, outcome, None, reason)`。設計文書の行番号は正しい。BUG-098 の本文(`docs/known-bugs/BUG-098.md`)は「~L1389 が `on_ime_apply_complete(open, outcome, None, ...)` を呼ぶ」と書いており、**呼び方も行番号も古い**(今は WM 経由)。M-3 にも触れていない。

### 1-2. G1・G2 の特定、案 A が閉じるもの — 正しい。再入時に誤って捨てることもない

**G1**: future の中の照合と投函(`post_to_main_thread_with`)は、同じ poll の中で続けて起きる。そのため窓は「投函から、WM がハンドラで取り出されるまで」。その間に、キューの先にある focus 系のメッセージが処理され、`ir_post_focus_change_snapshot` → `platform.rs:632 on_ime_mode_focus_changed` で世代が +1 され、続いて WM が処理される。この順序は起こりうる。補足: `WM_TIMER` は「キューが空のときだけ」合成されるので、タイマー起動の refresh は投函済みの WM を追い越せない。追い越せるのは、既に投函済みのメッセージと、GetMessage の中で投函済みメッセージより先に配送されるもの(out-of-context の WinEvent コールバック・sent message)だけ。WinEvent コールバックが同期でフォーカス検出を走らせるかは**未確認**で、これが G1 の窓の実際の大きさを決める。

**G2**: `with_app`(`lib.rs:237`)は `RUNTIME.try_borrow_mut()` に失敗すると `None` を返す。M-3 はこれを「一致扱い」にしている(fail-open)。winmsg-executor 0.3.2 の `spawn_local` は初回の poll も `PostMessageA(MSG_ID_WAKE)` で遅らせるので(`spawn_unchecked_lifetime`、`runnable.schedule()`)、`kp_shadow_actuate` が `with_app` を握っている間に同期で poll されることは**ない**。future の poll が `with_app` の内側で起きるのは、`with_app` の中でメッセージポンプが入れ子になる場合(モーダルループ: MessageBox・TrackPopupMenu・DialogBox 等)だけ。頻度は低い。

**案 A は G2 を正しく閉じる**: `handle_wm_async_ime_apply_complete(app: &mut Runtime, ..)` は呼ばれた時点で既に `&mut Runtime` を持っているので、照合は必ず実行される。再入している間はハンドラ自体が呼ばれない(呼び出し側が `with_app_or_repost_with` で投函し直す流儀。**未確認**: WM_ASYNC_IME_APPLY_COMPLETE の dispatch が `with_app_or_repost_with` 経由かどうか。`run_message_loop` の該当行を実装前に確認すること)。

**「正しい完了を誤って捨てる」(BUG-141・ADR-208 型)にはならない**: 捨てるのは「spawn の後に、ハンドラの時点までにフォーカス世代が進んだ」完了だけ。その世代の bump と同じ同期の呼び出しの中で(下の 1-3)、`ImeModel` の `FocusChanged` が `applied = Unknown` にリセットしている(`ime_model.rs:~1017`)。つまり捨てる完了は、すでに「別プロセスの記録」になったもので、新しいプロセスの `applied` に書いてはいけないもの。押下そのものは握りつぶさない: 書き込みは完了しており、`on_ime_apply_complete` は `UnsafeToToggle` でも `post_ime_refresh` を必ず呼ぶ(`runtime/mod.rs` の E)。press 台帳は async では元から解かない(`:1378` は sync 分岐だけ)。BUG-141 の型は「省略が押下を握りつぶす」で、向きが逆。
M-3 の fail-open(`:1354`)の意味は「再入でわからないなら、記録を進める側に倒す」。ハンドラに移せば「わからない」状態が起きないので、fail-open の判断そのものが要らなくなる。fail-closed に変えるわけではない。

### 1-3. U1 への回答: 世代の bump と `applied` のリセットは同じ同期の呼び出しの中

- `ime_mode_focus_gen` を +1 するのは `output/mod.rs:361 on_ime_mode_focus_changed` だけで、その呼び元は `platform.rs:632`(`gji_on_focus_change` の中)だけ。これを呼ぶのは `ir_post_focus_change_snapshot`(`ime_refresh.rs:588`)で、条件は `focus.focus_changed`(`:252`)。
- `focus_changed` は `ir_stage_focus`(`ime_refresh.rs:96`)の `apply_focus_probe_result` / `detect_and_update_focus` の戻り値。どちらも **`process_changed`** を返す(`focus_tracking.rs:~100`・`:902-904`)。同じ関数の中で `on_focus_process_changed` → `ImeEvent::FocusChanged`(`focus_tracking.rs:640`)が dispatch され、`applied = Unknown` になる。
- つまり「`applied` のリセット(Stage 1)」と「世代の bump(Stage 3.7)」は**同じ `run_ime_refresh` の呼び出しの中で、`&mut Runtime` を握ったまま起きる**。ハンドラ(`&mut Runtime`)はこの 2 つの間に割り込めない。案 A で `ime_mode_focus_gen` を使っても、「`applied` をリセットしたフォーカス変更の後の完了を捨てる」と同じ意味になる。
- **設計文書の誤り(Should-1)**: §2-4 の U1 補足と §4 R1 の「`ime_mode_focus_gen` は同一プロセス内の hwnd 変更でも進む(epoch より厳しい)」は誤り。bump はプロセス変更のときだけで、`ImeModel` の FocusChanged と同じ粒度。したがって R1(同一プロセス内の移動で完了を捨てる)は**起きない**。§6 の取りやめ条件の 3 つ目(「同一プロセス内の hwnd 変更で頻繁に進むと判明したら epoch に切り替える」)も前提が無い。直し方: R1 と取りやめ条件の 3 つ目を削除し、「bump はプロセス変更時だけ、`applied` のリセットと同じ呼び出しの中」と書き換える。案 B の却下理由 (4)「epoch は `focus_gen` より緩い(後退)」も同じ理由で根拠を失う(却下の結論は (1)〜(3) だけで成り立つ)。

### 1-4. G3 の位置づけ — 記録の問題ではない(Should-2)

OS のフォーカスが動いてから awase が検出するまでの間に完了が処理された場合、`applied` には旧プロセスの文脈のまま `Confirmed` が書かれる。その後、検出のときの `FocusChanged` が `applied = Unknown` にリセットする。新しいプロセスに古い記録は残らない。composition の cold-mark も、検出時の `mark_composition_cold_focus_change`(`ir_post_focus_change_snapshot`)で上書きされる。**G3 は `applied` の記録については無害**。
G3 で残る本当の問題は、**OFF の書き込み自体が `ImmCrossOp::Untargeted`(`:1337`)なので、その時点の OS のフォアグラウンド(新しい窓)に着弾しうる**こと。これは「完了の記録」ではなく「書き込みの宛先」の問題(ADR-086 INV-14 の Phase C)で、BUG-098 とは別件。
副次: 完了の記録の `note_awase_write_for_mode_key_pass_in_scope(scope)` は、完了の処理時の `foreground_scope()` を使う(`platform_state.rs:593`)。G3 の窓では新しい窓の scope に「awase が書いた」印が付く。影響は ADR-187 の通過マークの判定 1 回分で、**未確認**だが小さい。
直し方: §2-4 の G3 を「完了の記録としては無害(検出時の FocusChanged がリセットする)。残るのは OFF の書き込み先の未捕獲(Phase C、別件)」に書き換える。BUG-098 の記録にもそのように書く。

### 1-5. その他の事実

- 既存の `post_async_ime_apply_complete` の呼び元は `executor.rs:825/855`(Engine。generation 付き。ADR-108 の epoch ゲートが効く)と `key_pipeline.rs:1322/1362`。設計文書のとおり。
- `wparam` は `generation << 3 | reason << 1 | open`(`message_handlers.rs` の `encode_apply_wparam`)。`lparam` は `encode_outcome` だけ。案 A で lparam に載せる設計は成り立つ(x86_64 のみ)。
- U3 への補足(Should-3): 設計文書は「M-3 は `None` を黙って一致扱い、ログに該当行は無い」と書くが、`with_app` 自身が再入時に `tracing::warn!("with_app re-entry detected — returning None ...")` を出す(`lib.rs:240`)。**G2 の頻度は既存のログ・不具合報告から測れる**(ただし、どの呼び出し元の再入かは区別できない)。直し方: U3 を「測れる。報告の中の該当 warn の件数を数え、近くに `[shadow-toggle]` の行や ShadowToggle の ImeOpenApplied があるかを見る」に直す。

## 2. 案 A の挙動の変化の分析

### 2-1. 十分な点

- 「捨てられるようになる完了」は G1・G2 の 2 種類だけで、どちらも「spawn の後に `applied` がリセットされた」完了。記録として捨てるのが正しい。
- 「捨てられていたものが適用される」は無い。M-3 で不一致なら、ハンドラでも不一致になる(世代は wrapping_add なので、例外は 2^32 回の bump で一周したときだけ)。
- R2(press 台帳)・R3(`OutputActiveGuard`)・R4(defer/replay)・R5(actuation 合流点の件数ガード)は、設計文書のとおり影響なし。補足: R5 について。これは actuation の gate ではなく完了の記録の gate なので、`fix-requires-evidence.md` の「IME actuation 合流点」行の 3 点(`claim_press_write`・`with_press`・`explicit_press_applied_pair`)の配線義務には当たらない。

### 2-2. 足りない点

**Should-4: journal の忠実度が落ちる**
落とした完了は `on_ime_apply_complete` の冒頭で `JournalEntry::ImeOpenApplied { outcome: UnsafeToToggle }` として記録される。実際には書き込みは `Applied` だったのに、journal 上は「送らなかった」に見える(M-3 も同じ問題を持つ)。後から journal で調べるとき、「書き込みが届いていない」と誤読される(memory にある `ActuationDecision.outcome` の誤読と同型)。
直し方: `outcome` を書き換えずに「stale なので記録を動かさない」を別の引数(または `on_ime_apply_complete` の前の早期分岐と、別の journal 項目)で表す。最低でも、設計文書 §6 の `[async-apply-stale]` のログ行に「元の outcome」を含める。

**Should-5: 照合する世代の選び方の根拠を書く**
案 A が `ime_mode_focus_gen` で正しく動くのは、1-3 のとおり「bump と `applied` のリセットが同じ同期の呼び出しの中にある」から。将来、bump の位置(`ir_post_focus_change_snapshot`)を別の時点に動かせば、この前提は黙って崩れる。BUG-098 本文の follow-up 方針は `probe_admission::admit_epoch_in_app` と同型、つまり `focus_fence().epoch`(`platform_state.focus.focus_epoch`)の照合を想定している。
直し方: どちらでもよいが、(a) `ime_mode_focus_gen` を使うなら「同じ呼び出しの中」という前提をコメントとガードで固定する、または (b) `applied` のリセットと同じ単位の `focus_fence()`(epoch + hwnd)を載せる。(b) は hwnd も比べる分、同一プロセス内の hwnd 変更でも捨てることになる(1-3 の誤った R1 が、ここでは本当に起きる)ので、(a) を勧める。

**Nit-1: M-3 を削除するか(設計文書の判断を仰ぎたい点 2)**
削除してよい。ハンドラの照合は M-3 の上位互換で(2-1)、残しても挙動の差は「G2 のとき M-3 は通し、ハンドラが捨てる」だけ。同じ判定が 2 か所にあると、片方だけ直す事故の元になる。U2 の確認結果: `tests/architecture_guard.rs` に `ime_mode_focus_gen`・`post_async_ime_apply_complete` を固定する行は無い(`git grep` で 0 件。複数行のチェーンは対象の文字列が 1 行に収まっているので、見落としの心配は小さい)。

## 3. 回帰テストの層(書けるか)

- **T1(純関数の host テスト)**: 書ける。ただし `if spawn != now { UnsafeToToggle } else { outcome }` の表は、ほぼ自明なテストになる(テストの価値は低い)。価値があるのは結線のほう。
- **T2(encode/decode の往復、gated)**: 書ける。`message_handlers.rs` は `runtime/` 以下なので windows-build だけで走る。走ったことは nextest のログの件数で確認する(CLAUDE.md の注意どおり)。
- **T3(ハンドラ統合)**: **書けない見込み**(U6 への回答)。`Runtime` を作るのは `app/bootstrap.rs:740`(`RUNTIME.set(Runtime::new(..))`)だけで、テスト用のフィクスチャは `src/` に見当たらない(`git grep "Runtime::new("` が 1 件)。作るなら `Platform`・`Output` 等のテスト用の構築口が新たに要り、BUG-098 の修正には過大。
- **T4(source-scan)**: 書ける。`handle_wm_async_ime_apply_complete` の本体(`extract_all_balanced_blocks`)が判定関数を呼ぶこと、`:1362` の呼び元が focus_gen を渡すこと、を固定する。結線を守る主役はこれになる。
- **fix-requires-evidence の (a)(b)**: (a) は T1+T2+T4 で「回帰テストを足した」の形式は満たす。ただし再発ファミリーの趣旨(実際に壊れた経路を再現する)から見ると、どれも実際の到着順を再現していない。(b) の BUG-098 の更新はいずれの案でも要る。

## 4. 案 A・B・D の優先度 — **案 D を推奨**

判断材料:

1. **実害の記録が無い**。BUG-098 は ADR-108 のレビューから出た理論上の残存で、報告も journal も無い。主な窓は M-3(`cf48bc67`)で既に閉じている。
2. **残る窓は狭く、害が出る経路も狭い**。G1 は「キューの中で WM の前に focus 系のメッセージがある」とき、G2 は「`with_app` の中でモーダルループが入れ子になっている」ときに限られる。古い `Confirmed` が新しいプロセスの `applied` に入った場合でも、害が出るのは「`applied` を根拠に送信を省く」判断だけ。押下由来の書き込みは D1(`explicit_press_applied_pair`)で `applied` を省略の根拠にしない。drift correction は `build_ime_control_view(None)` で迂回する。残るのは、press の無い Engine 経路の SetOpen が already-matched で省かれる場合で、それも次のフォーカス変更か PanicReset・KeyEffect の不一致で `Unknown` に戻る。
3. **設計文書のリスク分析に誤りがあった**(R1・G3)。「推測で直さない」(設計文書 §1 自身の方針)と、過去の判断(ADR-218〜220 は実害の記録が弱く見送り、ADR-180 決定 2 は費用対効果が負で見送り、memory の「設計の前に症状の存在を実機で確認する」)に照らすと、実害を測らずに WM の wire 形式を変える根拠が弱い。
4. 一方で、案 A は「新しい抽象」ではなく既存の照合を移すだけで、M-3 を消せば正味の行数はほぼ増えない。WM の wire 形式の変更と T2・T4 の追加が主なコスト。**実害が 1 件でも見つかれば、ためらわず入れてよい**大きさ。
5. 案 B は不採用(設計文書の (1)〜(3) のとおり。(4) は 1-3 の理由で根拠を失うが、結論は変わらない)。

**推奨: 案 D**。中身は次のとおり。
- BUG-098.md を訂正する: 状態を「M-3(`cf48bc67`)で主な窓は閉じた。残りは G1(投函から取り出しまで)・G2(`with_app` 再入時の fail-open)。G3 は記録としては無害で、OFF の書き込み先の未捕獲(Phase C)は別件」にする。行番号と呼び方(WM 経由)を直す。30 行以内のルールに合わせて短くする。`fix_commits` に `cf48bc67` を足す。
- **実害を測る手順**を BUG-098.md に書く: (i) 不具合報告の journal で、`ActuationDecision`(caller が shadow toggle)の後に `FocusTransition`(プロセス変更)があり、その後に `ImeOpenApplied { reason: ShadowToggle }` が outcome ≠ UnsafeToToggle で来る並びを数える(= G1 の実例)。(ii) `with_app re-entry detected` の warn の件数(= G2 の上限)。
- **再開の条件**: (i) が 1 件以上、または G2 の warn が shadow toggle の直後に出ていたら、案 A(本レビューの Should-4・5 を反映したもの)を実装する。
- 所有者の決定「F-D5-2 で直す」は、M-3 が記録に無かったという前提に立っていた。この前提が崩れたことを所有者に伝え、D で止めてよいかの確認を取る(team-lead から)。

## 5. 設計文書の「判断を仰ぎたい点」への回答

1. **優先度**: 案 D(§4)。
2. **M-3 の削除**: 案 A を入れるなら削除してよい(Nit-1)。D なら触らない。
3. **OFF の `Untargeted`(G3)**: BUG-098 からは切り離す。記録としては無害なので、BUG-098 は「G3 は残る」ではなく「G3 は記録の問題ではない」と書いて閉じてよい。書き込み先の捕獲(Phase C)を進めるかは別のタスクで判断する。こちらも実害の記録は無い(**未確認**: 報告を検索していない)。
4. **載せ方**: 案 A をやるなら lparam への詰め込みでよい。`RuntimeOutbox` で構造体のまま渡す案は、既存の 3 呼び元と経路が分かれ、「完了の入口は 1 つ」(`on_ime_apply_complete` の doc)を崩す。64bit 前提は `const _: () = assert!(size_of::<isize>() == 8);` で固定する(U5)。16bit に畳む案は、衝突 = 誤って一致扱い、なので採らない。
5. **R1 の許容**: R1 は起きない(1-3)。判断は不要。

## 6. 指摘の一覧

| 重さ | 内容 | 直し方 |
| --- | --- | --- |
| Should-1 | R1・U1 補足・取りやめ条件 3・案 B 却下理由 (4) の「同一プロセス内の hwnd 変更でも gen が進む」は誤り | bump はプロセス変更時だけ、`applied` のリセットと同じ呼び出しの中、に書き換えて該当項目を削除 |
| Should-2 | G3 を「記録の残余リスク」としているが、記録には無害。本当の残余は OFF の書き込み先 | §2-4 と BUG-098 の記述を書き換え、Phase C は別件に |
| Should-3 | U3「ログに該当行は無い」は誤り(`with_app` は再入時に warn を出す) | 実害を測る手順に組み込む |
| Should-4 | 案 A(と M-3)は `outcome` を `UnsafeToToggle` に書き換えるので、journal で「送っていない」と誤読される | 元の outcome を残す(別の引数・別の journal 項目、最低でもログに) |
| Should-5 | `ime_mode_focus_gen` が正しく働く前提(bump と `applied` のリセットが同じ呼び出しの中)が暗黙 | コメントとガードで固定 |
| Nit-1 | M-3 の扱い | A なら削除、D なら触らない |
| Nit-2 | T3 は `Runtime` のフィクスチャが無く書けない見込み | 取りやめ条件から外し、T4 を結線の主な証拠にする |
| 未確認 | WM_ASYNC_IME_APPLY_COMPLETE の dispatch が `with_app_or_repost_with` 経由か。WinEvent コールバックが同期でフォーカス検出を走らせるか(G1 の窓の大きさ) | 案 A に進む場合は実装前に確認 |

Blocker・Must なし(設計文書であり、コードの変更は無い)。
