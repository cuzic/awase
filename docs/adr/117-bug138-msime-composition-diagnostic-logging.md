# ADR-117: MS-IME「直接入力モード許可」時の英数キー文字消失（issue #138）切り分け用ログ

## ステータス

採用・実装済み（2026-09-02）。Opus 2体（architect/premortem_reviewer）による
敵対的レビュー。r1で両者から「主経路（非同期ImmCross）を対象から外していた」
「journal記録が composition tear-down 後の値になり一次証拠として機能しない」
という致命的指摘を受けr2へ修正、architectはr2で収束。premortemはr2で追加3点
（送信成否のinfo!可視性不足、fallback_write経由の値の陳腐化、`false`値の両義性）
を指摘しr3で反映、r3でさらに「送っていない」無音分岐3箇所とログ書式の不整合を
指摘されr4で反映し、premortemもr4で収束した。r4の設計どおり実装完了、
`cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows`・
`cargo clippy`（同ターゲット）・`cargo fmt --check`・
`cargo nextest run -p awase-windows --test architecture_guard --test golden_scenarios --test layer_boundary_guard`
（94件）・`cargo test --lib`（921件）すべて green。Windows実機ソークは未実施
（本ADRの性質上、ログの見た目確認以上の実機検証は次の実ユーザー報告を待つ）。

## r1からの主な変更（レビュー指摘の反映）

r1は `ime_controller.rs` の `ImmCrossProcessStrategy::apply` にログを足す案
だったが、両レビュアーが同一の致命的欠陥を指摘した:

1. **報告環境（Standard プロファイル×MS-IME）の実書き込みは
   `ImmCrossProcessStrategy::apply` を通らない**（architect MA-1、premortem 1）。
   `runtime/executor.rs:787`/`runtime/key_pipeline.rs:1029` の
   `imm_cross_is_first_applicable` が真のとき、`win32_async::spawn_local` で
   `runtime/open_chain.rs::imm_cross_write` へ直接分岐し、`ime_controller.rs` の
   戦略層自体を経由しない。r1のログはissue #138の主経路では一度も出力されない。
2. **`JournalEntry::ImeOpenApplied` への記録（`on_ime_apply_complete`）は
   composition tear-down 後の値になる**（architect MA-2、premortem 2）。async
   経路は `post_async_ime_apply_complete`（`PostMessage`）を挟むため、
   `EVENT_OBJECT_IME_HIDE`（`WINEVENT_OUTOFCONTEXT`で同じメッセージキューに
   載る）が先に処理され得る。結果、「composition が元々無かった」ケースと
   「composition が破棄された」ケースが journal 上で同じ `false` になり、
   本ADRが区別したい2ケースを区別できない。

加えて premortem からの指摘で以下も反映した:

3. **`KanjiToggleStrategy` は対象外にできない**（premortem 6）。Standard×MS-IME
   で ImmCross が `Failed` を返すと `open_chain.rs::fallback_write` 経由で
   `KanjiToggleStrategy`（`VK_KANJI` トグル送信）に到達しうる。r1は「GJI側の
   非対称性」を理由にGJI系戦略と一括りで対象外にしていたが、KanjiToggleは
   GJI専用ではなく、この一括りの根拠は薄い。
4. **`ime_composition_active` の信号品質そのものに解釈上の注意点がある**
   （premortem 3）。`EVENT_OBJECT_IME_SHOW/HIDE` ハンドラ（`win_event_obs.rs:200-213`）
   はPID/フォーカスで一切フィルタしておらず、フォーカス変更でもリセットされない。
   さらにMS-IMEのTSFインライン未確定文字列はIMEウィンドウを生成しないことが
   多く、`IME_SHOW` が一度も発火せず常時 `false` の可能性がある。ログを見る側は
   「`composition_active=false` ＝ composition 無し」と早合点してはいけない。

これらを踏まえ、**journalへのフィールド追加（旧決定3）は撤回**し、
`imm_cross_write`（非同期ImmCross経路の実体）への新規ログ追加を主軸に据え、
`KanjiToggleStrategy` にも診断ログを追加する形に変更した（下記「決定」参照）。
なお、`ImeOpenApplied` の構築箇所は `runtime/mod.rs:514` の1箇所のみで
テスト/fixtureへの影響は無いこと、`ObservedState` の構造体リテラルは
`ime_controller.rs:591,633` の2箇所とも `..ObservedState::default()` で
無修正で通ることを、両レビュアーが実コードで確認済み（旧「検証」節が
懸念していた広範なコンパイル破壊は空振り）。

## 背景

[GitHub issue #138](https://github.com/cuzic/awase/issues/138): awase + MS-IME の組み合わせで、
MS-IME の設定「直接入力モードを使用しない」のチェックを外している（＝直接入力モードの使用を
許可している）状態にすると、かな入力中に「英数」キー（`VK_DBE_ALPHANUMERIC`）を押すと、
入力していた未確定文字列が全て消える、との報告がある。awase を入れていない状態では発生しない。

「不具合を報告」機能で取得した journal/ログを調査したが、以下が判明した。

1. 実際に文字が消える瞬間そのもの（まとまった Backspace 等の痕跡）は journal に記録されて
   いなかった。
2. `runtime/transport.rs` の `PhysicalKeyDisposition::plan()` により、ユーザーが押した物理
   `VK_DBE_ALPHANUMERIC` は awase 側で無条件 Suppress され、MS-IME には一度も届いていない
   （BUG-52 対策）。MS-IME が実際に受け取るのは、awase が能動的に発行する
   `ImmSetOpenStatus(FALSE)`（`ImmCrossProcessStrategy`、通常アプリ）または `VK_IME_OFF`
   （`MsImeDirectStrategy`、TSF-native アプリ）である。
3. `ime.rs:78-79` に実装者自身のコメントがある: 「IME OFF (open=false) は composition
   tear-down と IME UI 隠蔽が走るため 50ms では時々取りこぼす」。IME OFF 送信が composition
   tear-down を引き起こすこと自体は既知の事実として書き残されているが、これを検出・回避する
   ガードは実装されていない。
4. GJI には `gji_candidate_visible`（候補ウィンドウ SHOW/HIDE 監視）等、composition 状態を
   実観測する仕組みがあるが、MS-IME 側の唯一の composition シグナルである
   `TsfObservations::ime_composition_active`（`EVENT_OBJECT_IME_SHOW/HIDE` 由来）は、
   `build_input_context()` 経由でエンジンの `composing`（無変換/変換キーの solo-tap ガード
   専用）には渡っている（`runtime/mod.rs:313`、`key_pipeline.rs:110`、
   `message_handlers.rs:273`）ものの、**IME open/close を実際に決定・送信する層
   （`ime_controller.rs` の戦略、および `runtime/open_chain.rs` の非同期書き込み）
   のどちらにも届いていない**。`ImeControlView`（`ime_controller.rs` の戦略が受け取る
   唯一の観測ビュー）にも composition 関連のフィールドが存在せず、戦略層は構造的に
   条件分岐しようがない。
5. `ImmCrossProcessStrategy::apply`（`ime_controller.rs`。ただし後述の通り、報告環境
   （Standard×MS-IME）の実書き込みは大半がこの関数を経由せず、代わりに
   `runtime/open_chain.rs::imm_cross_write` が非同期に処理する）も、`imm_cross_write`
   自体も、送信の成功パスでは **一切ログを出していない**。`MsImeDirectStrategy::apply` の
   送信ログも `log::debug!` であり、通常起動（`--debug` なし）の `awase.log` は
   `default_filter_or("info")`（`app/bootstrap.rs:145`）のため、実際のユーザー報告
   （debug モードなし）では journal にも awase.log にも「いつ・どの経路で IME OFF を
   送ったか」「その瞬間 composition が有効だったか」が一切残らない。

現時点の仮説（[[project_bug138_msime_alnum_composition_wipe]] 参照）は、「MS-IME が
DirectInput へ実際に移行できる設定のときに限り、composition 中に IME OFF を受け取ると
未確定文字列を確定させずに破棄する」というものだが、上記 5 の欠落により**次回の実機報告
でもこの仮説を裏付けるログが取れない**。本 ADR は、コード変更で症状そのものを直そうと
せず、次に同じ報告が来たとき（または報告者に再現を依頼したとき）に仮説を検証できるだけの
診断ログを追加することを目的とする。

## 決定

### 決定1: `ObservedState` に `composition_active` と show/change シーケンスを追加する

`state/ime_decision_view.rs::ObservedState` に、既存の `candidate_visible`（GJI 専用）と
並ぶ形で `composition_active: bool` を追加する。供給元は
`TsfObservations::ime_composition_active()`（`tsf/observer.rs` に `gji_candidate_visible()`
と同型の `&self` メソッドとして新設、内部は既存の `ime_composition_active_now()` と同じ
`AtomicBool::load(Relaxed)`）。`ObservedState::from_snapshot()` で他フィールドと同様に
スナップショット化する。`Default` 実装にも `false` を追加する。

これにより `ime_controller.rs`（「観測値を自ら読んではいけない、`ImeControlView` 経由で
受け取ること」という同ファイル冒頭のアーキテクチャ制約を持つ）からも、既存の規律を破らず
composition 状態を参照できるようになる。

**（premortem r2レビュー指摘Q4を反映）** `composition_active=false` は「一度も
`IME_SHOW` が発火していない」のか「`IME_HIDE` で落ちた」のかを区別できない。
`TsfObservations` に保持済みの `ime_show_seq`/`ime_change_seq`（`ChangeCounter`、
`observer.rs:209,215` で `notify()` 済み）の現在値を読む `value()` アクセサを
`ChangeCounter` に追加し（既存の `notify()`/`baseline()`/`has_changed()`/`reset()`
と同型の `pub(super) fn value(&self) -> u32 { self.0.load(Ordering::Relaxed) }`）、
`TsfObservations::ime_show_seq()`/`ime_change_seq()`（`pub`、`u32` を返す）として
公開する。`ObservedState` にも `composition_active` と並べて
`ime_show_seq: u32`/`ime_change_seq: u32` を追加し、決定2の全ログ行に含める。
これにより `composition_active=false, show_seq=0` なら「一度も発火していない」、
`composition_active=false, show_seq>0` なら「発火した後 HIDE で落ちた」と
1行で判別できる。

### 決定2: 実際に送信が起きる4箇所すべてに、送信直前（await をまたぐ前）の
`composition_active`/`ime_show_seq`/`ime_change_seq` を埋め込んだ `info!` ログを
追加する

「送信直前」を各経路の**実際の raw write 呼び出し箇所**で捉えることが要点（r1の
「戦略の`apply()`にさえ足せばよい」という前提そのものが誤りだった）。

**（premortem r3レビュー指摘Q3、書式統一）** 以下の4箇所の送信直前ログは、
例示の長さを揃えるため一部のバレットでは `composition_active` のみを literal
例に示しているが、**実装では4箇所すべてに `composition_active`/`ime_show_seq`/
`ime_change_seq` の3値を含める**（決定1の狙いである「`false` の両義性解消」は
どれか1経路でも欠けると台無しになるため）。

- **`runtime/open_chain.rs::imm_cross_write`**（非同期 ImmCross、報告環境の主経路）:
  関数の先頭、`op` を match する前に `crate::tsf::observer::tsf_obs()` から
  `composition_active`/`ime_show_seq`/`ime_change_seq` を live 読み取りし、
  `log::info!("[apply-ime] ImmCross async: open={open} composition_active={composition_active} show_seq={show_seq} change_seq={change_seq} (issue #138診断)")`
  を追加する。この関数は `ImeControlView` を持たないため、`runtime/` 層の既存
  precedent（`key_pipeline.rs:110` 等）と同様に `tsf::observer` を直接読む
  （`ime_controller.rs` のような直接読み取り禁止の制約下にはない）。
  読み取り位置は `.await` の**前**であることが必須——`set_ime_open_then_conv_for_target`
  / `set_ime_open_cross_process_async` の呼び出しより前でなければ、決定3で撤回した
  旧設計と同じ「tear-down 後の値を掴む」問題を再現する。

  **（premortem r2レビュー指摘Q2を反映）** 同関数内の `ActuationOutcome::Aborted`
  分岐（`open_chain.rs:186` 付近）と `ActuationOutcome::Failed` の2分岐
  （`:196-211` 付近）にある既存の `log::debug!` を `log::info!` に格上げする。
  これらは「実際に送ったが検証失敗で中止した（1バイトも書いていない）」
  「送信自体が失敗した」を示す分岐であり、`Written`（成功）と同じ `info!` 可視性が
  無いと、実ユーザー報告で「送ったのに文字が消えた」と「そもそも送っていない
  （＝awase起因ではない）」を判別できない（`Written` 側は既存の
  `JournalEntry::ImeOpenApplied.outcome` で追跡できるため新規ログ不要）。

  （architect r2レビューでの補足）最も早い捕獲点は起案側
  （`executor.rs:785`/`key_pipeline.rs:1030`、`ImeControlView` 構築直後）であり、
  `spawn_local` から初回ポーリングまでの間に `EVENT_OBJECT_IME_HIDE` が処理される
  余地はまだ残る。決定1で追加する `ObservedState.composition_active` はまさにこの
  時点の値だが、非同期ImmCross経路では使われない（`ImmCrossOp` に渡っていない
  ため）。本ADRでは「起案時点の view」までは遡らず、関数冒頭の live 読み取りで
  実務上十分とする——後者は前者よりわずかに遅いだけで、どちらも「実際の Win32
  呼び出しより前」という要件は満たす。
- **`MsImeDirectStrategy::apply`**（`ime_controller.rs`）: ON/OFF 各分岐で
  `send_ime_mode_key(vk)` を呼ぶ直前に、`view.observed` の3値（決定1で追加）を
  埋め込んだ `log::info!` を追加する（OFF分岐の既存 `log::debug!` は撤去し
  これに統合、ON分岐は新規）。

  **（premortem r3レビュー指摘Q2-1、無音失敗の解消）** 送信直前ログとは別に、
  ON/OFF 両分岐の `if !unsafe { crate::ime::send_ime_mode_key(vk) } { return
  ImeOpenOutcome::UnsafeToToggle; }`（2箇所）に、`return` の直前で
  `log::info!("[apply-ime] MS-IME direct: send_ime_mode_key failed (Winキー押下中等) → UnsafeToToggle")`
  を追加する。現状この分岐は無ログのため、実際には1バイトも送っていないのに
  「送信直前ログだけが info に残る」状態になり、読む側が「送った直後に文字が
  消えた」と誤読する（premortem r3指摘）。

  （architect r2レビューでの訂正）この戦略が適用可能なのは
  `ms_ime_direct_applicable`（`key_sequence_policy.rs:51-53`）＝
  `MsIme && !profile.can_use_imm32_cross_process()` のときであり、これは
  TsfNative/Imm32Unavailable プロファイルに限られる。これらのプロファイルは
  `imm_cross_is_first_applicable` が常に偽（chain の先頭が `ImmCross` ではない）
  なので**非同期チェーンには最初から入らず、常に `ImeController::apply` →
  `SyncChainWriter` の完全同期経路のみを通る**。r1時点の「`fallback_write`
  からも呼ばれる」という記述は誤りだった（`fallback_write` に実際に到達する
  Standard×MS-IME×ImmCross-Failed の場合、chain 中で次に applicable なのは
  `KanjiToggle` のみ）。編集箇所自体（1箇所）は変わらないが、カバー対象は
  「完全同期経路のみ」に訂正する。
- **`KanjiToggleStrategy::apply`**（同上）: 既存の
  `log::debug!("[apply-ime] shadow=... candidate=... was_seen=... profile=... → desired=...")`
  に `view.observed` の3値（`composition_active`/`ime_show_seq`/`ime_change_seq`）を
  追加し、`log::info!` に格上げする。Standard×MS-IME で ImmCross が `Failed` を
  返した場合に `open_chain.rs::fallback_write` が実際に到達させる唯一の機構であり
  （premortem 6、architect r2レビューで確認）、GJI 専用ではないため対象から外さない。
  `post_kanji_toggle_to_focused()` は返り値を持たず常に `FallbackSent` を返すため、
  失敗分岐は存在しない（premortem r3で確認、対応不要）。

  **（premortem r2レビュー指摘Q3）** `fallback_write`（`open_chain.rs:252`）は
  `shadow_ime_control_view()` で view を**作り直す**ため、この経路で
  `KanjiToggleStrategy::apply` が見る `composition_active`/`ime_show_seq`/
  `ime_change_seq` は、直前に試みた ImmCross 送信の `.await` が完了した**後**の
  ライブ値である。つまりこの値は「これから送る KanjiToggle にとっての送信前」
  ではあるが、「直前の ImmCross 試行にとっては送信後（tear-down 済みかもしれ
  ない）」の値でもある——撤回した旧決定3と同型の罠がこの経路にだけ残る。
  挙動修正はせず、`fallback_write` の doc コメントにこの事実を明記するに留める
  （ログを見る側が「ImmCross と KanjiToggle、どちらの送信に対応する値か」を
  混同しないための注記）。
- **`ImmCrossProcessStrategy::apply`**（同上）: `_view` 引数を `view` に変更し、
  `view.observed` の3値を埋め込んだ `log::info!` を送信直前に追加する。報告環境の
  主経路ではない（`try_force_on_bootstrap` 等、稀な完全同期呼び出しのみ到達）が、
  現状ログが皆無であり追加コストが低いため揃える。

  **（premortem r3レビュー指摘Q2-2、無音失敗の解消）** `set_ime_open_cross_process(open)`
  が偽を返す `else { ImeOpenOutcome::Failed }` 分岐（`ime_controller.rs:77-81`）にも
  同様に `log::info!("[apply-ime] ImmCross sync: set_ime_open_cross_process failed → Failed")`
  を追加する。Q2-1（`MsImeDirectStrategy`）と同型の「送信直前ログだけが残り
  実際は送っていない」誤読を防ぐため。

`GjiDirectStrategy` のみ据え置く。GJI は候補ウィンドウ SHOW/HIDE が既に `info!` で
記録されており（`win_event_obs.rs`）、本 ADR が埋めたい欠落（MS-IME 系4経路に info
レベルの送信ログが無い非対称性）に該当しないため。

**（premortem r3レビュー指摘Q2-3、優先度低・任意）** `fallback_write`
（`open_chain.rs:252-260`）自体は、`mechanism_is_applicable` が偽のとき、および
`with_app` が `None` を返したとき（`.unwrap_or(ImeOpenOutcome::Failed)`）、
無音で `Failed` を返す——「ImmCross 失敗後 KanjiToggle へフォールスルーしたが
結局何も送られなかった」が観測できない。決定2の主眼（4送信経路の可視化）ほど
優先度は高くないが、この関数にも `log::info!` を1行追加して両ケースを記録する。

**対象外: `apply_mechanism` 内の ROMAN 補完（`romaji_pre_write`）。**
`needs_romaji_pre_write`（`state/actuation_chain.rs:243-257`）は `open &&
mechanism ∈ {ImmCross, MsImeDirect} && kind == MsIme && belief != ObservedKana`
の場合のみ真を返す——**`open=true`（IME ON 方向）専用**であり、issue #138 が
問題にしている `open=false`（英数キー押下による IME OFF 方向）では発火しない。
そのため今回は対象外とする（architect r2レビュー指摘4への回答）。ただし
`open=true` 側の composition 診断が将来必要になった場合は、既存の
`log::debug!("[apply-ime] ROMAN 補完結果: ...")`（`open_chain.rs:167`、
`ImmCross Targeted` 経路のみ）が同様の欠落を持つことに留意する。

### 決定3（撤回）: journal へのフィールド追加はしない

r1では `JournalEntry::ImeOpenApplied` に `composition_active` を追加し、
`on_ime_apply_complete` でのライブ読み取りで埋める設計だったが、両レビュアーの
指摘（architect MA-2、premortem 2）により撤回した。`on_ime_apply_complete` は
非同期経路では `post_async_ime_apply_complete`（`PostMessage`）を経由した**完了後**
にしか呼ばれず、その時点では `EVENT_OBJECT_IME_HIDE` によるリセットが既に
反映されている可能性があり、「送信時点で composition が有効だったか」という
知りたい情報を運べない。

決定2の4箇所は、いずれも実際の Win32 呼び出し（または `.await` に入る）の**直前**に
位置しており、この問題を構造的に持たない。追加のプラミング（`ImmCrossOp`への
フィールド追加、`post_async_ime_apply_complete`のwparam拡張等）をしてまで
journalに二重化する必要は無いと判断した。「送信後にcompositionが消えたか」を
知りたい場合は、既存の `[ime-obj] IME_HIDE`（info!、`win_event_obs.rs:213`）を
awase.logのタイムスタンプで決定2のログと突き合わせれば足りる。

（architect r2レビューでの補足、premortem r2レビューQ1で追認）「journalは
RUST_LOGに関係なく必ず残る／awase.logは残らない」というr1時点の撤回理由の
前提そのものが誤りだった：`bug_report.rs`（`attach_log`チェックボックス、
`awase-settings/src/bug_report.rs:140`「ログを添付する（journal + awase.log）」）は
journal JSONとawase.log末尾を**同一トグル1つ**で両方添付するため、`info!`ログも
journalと同条件で報告に載る。加えて、突き合わせ相手の`[ime-obj] IME_HIDE`は
journalには存在せずawase.logにしかないため、journal側だけ持っていても対比が
成立しない——journal撤回は正しい判断だったことがこの点からも裏付けられる。

一方で残る制約がある。`LOG_EXCERPT_MAX_BYTES = 200KB`（`bug_report.rs:27`）の
上限があり、`truncate_text_tail`（`bug_report.rs:361`）はawase.logの**末尾**を
残す。`EVENT_OBJECT_IME_CHANGE`は変換候補が動くたびに1発火＝1`info!`行
（`win_event_obs.rs:215`付近）を生むため、報告者が再現後も入力を続けると、
決定2のログと`IME_HIDE`がまとめて200KB圏外へ押し出される恐れがある。
運用上は「再現した直後に報告する」ことを次回の依頼時に明記する。

### 決定4: 挙動は変更しない

本 ADR はログ追加のみを行い、composition 中の IME OFF 送信を止める・遅延する・確認前に
別の処理を挟むといった**挙動変更は一切行わない**。理由は 2 点:

1. `ime_composition_active` 自体が MS-IME 環境で正しく発火するかがまだ実機で未検証
   （`win_event_obs.rs` 冒頭コメント: 「GJI TSF モードでは発火しないが Chrome ホスト側から
   発火するか検証用」）。検証前の信号を actuation のゲートに使うのは
   [[feedback_conv_mode_unreliable_dont_gate_actuation_on_it]] と同種の失敗パターンを
   繰り返すリスクが高い。
2. 症状の再現条件（DirectInput 許可時のみ）・発生タイミング（複数文字が一括で消える瞬間）
   のどちらも実機ログで未確認であり、対策を打つには早すぎる。

### ログ解釈上の注意（premortem 3、実装後もコード上に残す）

`EVENT_OBJECT_IME_SHOW/HIDE` ハンドラ（`win_event_obs.rs:200-213`）は PID/フォーカスで
一切フィルタしておらず、フォーカス変更でもリセットされない。さらに MS-IME の TSF
インライン未確定文字列は IME ウィンドウを生成しないことが多く、`IME_SHOW` が一度も
発火せず `composition_active` が常時 `false` になっている可能性がある。したがって
今回追加するログの `composition_active=false` は「その瞬間 composition が無かった」
ことの証明にはならない——「この信号がそもそも MS-IME で機能しているか」自体を
実機ログで確認するのが本 ADR の副次目的でもある。この注意点は決定2で追加する
ログの直上に doc コメントとして残す。

## 検証（実装時にレビューする影響範囲）

- `crates/awase-windows/src/tsf/observer.rs`:
  - `ChangeCounter::value(&self) -> u32` メソッド新設（`notify`/`baseline`/
    `has_changed`/`reset` と同型）
  - `TsfObservations::ime_composition_active()` / `ime_show_seq()` / `ime_change_seq()`
    の3メソッド新設（`gji_candidate_visible()` と同型の `&self` メソッド）
- `crates/awase-windows/src/state/ime_decision_view.rs`: `ObservedState` に
  `composition_active: bool` / `ime_show_seq: u32` / `ime_change_seq: u32` を追加、
  `Default`、`from_snapshot` 更新（構築箇所は `ime_controller.rs:591,633` の2箇所のみ、
  いずれも `..ObservedState::default()` 使用済みのため無修正で通る——レビューで確認済み）
- `crates/awase-windows/src/ime_controller.rs`:
  - `MsImeDirectStrategy::apply` / `KanjiToggleStrategy::apply` /
    `ImmCrossProcessStrategy::apply` の3箇所に `composition_active`/`ime_show_seq`/
    `ime_change_seq` 込みの `info!` ログ追加
- `crates/awase-windows/src/runtime/open_chain.rs`:
  - `imm_cross_write` 冒頭にログ追加（live 読み取り）
  - 同関数内 `ActuationOutcome::Aborted`/`Failed` 分岐の既存 `log::debug!` 2箇所を
    `log::info!` に格上げ
  - `fallback_write` に doc コメント追加（premortem Q3、composition 値の
    「どちらの送信に対応するか」の注記、挙動変更なし）
- `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows`
- `cargo nextest run -p awase-windows --test architecture_guard --test golden_scenarios --test layer_boundary_guard`
- `cargo test -p awase-windows --lib`

## 代替案として検討し不採用としたもの

- **composition tear-down を防ぐ挙動変更を今回一緒に入れる**: 仮説が実機で未検証のまま
  挙動を変えると、[[feedback_verify_review_fixes_not_just_original_code]] 同様「直した
  つもりの変更が新しい回帰を生む」リスクを診断目的のはずの変更に持ち込むことになる。
  今回はログのみに限定し、原因確定後に別 ADR で対策を検討する。
- **`ImeControlView` を経由せず `ime_controller.rs` から `tsf::observer` を直接呼ぶ**:
  同ファイル冒頭のアーキテクチャ制約に反する。`architecture_guard.rs` に該当する grep
  ガードが無くても、既存の設計意図（観測値の出所を型で強制する）を診断コードだからと
  破るべきではない。
- **`log::warn!` を使って強制的に目立たせる**: 実際には正常系（IME OFF 自体は日常的に
  発生する意図した操作）であり、`warn!` は誤解を招く。`info!` で十分。
- **`JournalEntry::ImeOpenApplied` への `composition_active` 追加（r1決定3）**:
  「決定3（撤回）」に記載の通り、非同期経路では tear-down 後の値しか取れず一次証拠に
  ならないため撤回。「送信時点」の値を journal まで正しく運ぶには `ImmCrossOp`・
  `post_async_ime_apply_complete` の wparam 拡張等、複数箇所への値の受け渡しが必要になり、
  診断ログという目的に対して変更範囲が見合わない。
- **送信直後にもう一度 `ime_composition_active_now()` を読んで before/after を両方ログに残す**:
  「後」の値は前述の通りタイミング次第で信頼できず、しかも既存の `[ime-obj] IME_HIDE`
  ログ（info!）が実質的に同じ役割を果たせるため、送信箇所ごとに二重にログを増やす
  価値がないと判断した。
