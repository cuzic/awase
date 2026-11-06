---
id: ADR-288
title: |-
  Mozc の compose-check(ADR-270/284/285)が Chrome を除外する前提を、実機 TIP + 実際の Mozc サーバーで反証する
summary: |-
  ADR-285 は Chrome 系窓(`Chrome_WidgetWin_1`・`Chrome_RenderWidgetHostHWND`)を compose-check の対象から除外している。
  根拠は「Chrome の TSF composition context 再初期化が確認の期限(`MOZC_COMPOSE_CHECK_MS`、300ms)を越えうるため、
  期限超過を Literal と誤判定して Backspace を送ると、もうすぐ正しく入る文字を壊しうる」(PR #809 レビュー M3)。
  所有者の提案(2026-11-06)を受け、Chrome 自体のタイミングを再現する代わりに、mozc-readable の TIP 自身に
  CI 専用の一回限りのフックを足して「awase から見える症状(key_seq/phase が期限を越えて凍結する)」を意図的に
  作り出し、本物の TIP + 本物の Mozc サーバーに対して Backspace(1 回、ESCAPE なし)+ 全体再送の回収が正しく
  終わるかを確かめた。結果: 期限超過後も Backspace+再送は常に正しい文字列に終わった(素通りは一度も起きず
  〈`passed` カウンタが 0 のまま〉、直前の文字も消えなかった)。除外の前提は成立しないと判断する。
  **決定は実装済み**(`is_mozc_compose_check_window` から Chrome 除外を外す本番コード変更、2026-11-06)。
  実機 Chrome 自体での確認も実施した(dragonflyg4、2026-11-06): 除外を外したビルドで実 Chrome に対し
  `[mozc-compose]` が実際に張られ、6 回連続で正しく confirmed することを確認した(下記「実機 Chrome での
  確認」参照)。
status: |-
  提案・検証済み(実機 TIP + 実際の Mozc サーバーでの CI 確認、2026-11-06)。本番コード(`is_mozc_compose_check_window`
  の Chrome 除外を外す変更)も実装済み(2026-11-06、同ブランチ)。実機 Chrome 自体での確認も実施済み(2026-11-06、
  dragonflyg4、下記「実機 Chrome での確認」参照)。
related_adr:
  - "ADR-270"
  - "ADR-284"
  - "ADR-285"
---

# ADR-288: Mozc の compose-check が Chrome を除外する前提を、実機 TIP + 実際の Mozc サーバーで反証する

## 背景

`is_mozc_compose_check_window`(`crates/awase-windows-base/src/focus/class_names.rs:83`)は、ADR-285 で
「Chrome 系と空のクラス以外のすべての窓」に反転されたが、Chrome 自体は今も除外されている。除外の根拠は:

> Chrome の TSF composition context の再初期化に ~326ms かかる実測(`79134f5`)があり、確認の期限
> (`MOZC_COMPOSE_CHECK_MS`、300ms)を越えて遅れてできた入力中の文字を、誤判定の Backspace が消しうる
> (PR #809 レビュー M3)。

この ADR の調査で、引用されている実測 `79134f5` は **Mozc 固有の計測ではなく、GJI(Google 日本語入力)側の別の
バグ(F2NonTsf プローブのタイミング)を類推で使っていた**ことが分かった(直接 Mozc で測った値ではない)。

## 調査(cross-session research、2026-11-06)

所有者の依頼で、別セッション(研究エージェント、Opus)が mozc-readable の実際の TIP 実装
(`tip_keyevent_handler.cc`・`tip_edit_session.cc`・`tip_edit_session_impl.cc`・`session/session.cc`)を読み、
以下を確認した:

1. **`key_seq` は `OnTestKeyDown`/`OnPreservedKey` の入口で必ず進む**(ADR-267 D1)。鍵が TIP に届いた後の処理
   (サーバー往復・編集セッション・組成の書き込み)は 1 本の同期呼び出しで、通常経路で「数えたが公開が遅れる」
   という形(仮説 B)は構造上起きない。Chrome の遅れがあるとすれば、`key_seq` が進む**前**(A1: キューに
   積まれているだけ・A2: key sink に届かない本物の素通り)のどちらかしかない。
2. **BUG-225 の実際の漏れ方は A2**(key_seq がログで 330→331 のまま進まない=TIP に一度も届いていない)。
3. **Backspace の挙動は `session/session.cc` の `Session::Backspace`(入力中の文字がある状態、`:2562-2568`)と
   `Session::Revert`(無い状態、`:1137-1141`)で決まる**: 入力中の文字があれば**サーバーが消費**し文書には
   届かない。無ければアプリへ素通りする(素通りが正解になるのは、素通りした分を正しく消す場合だけ=A2)。
   どちらの場合も、Backspace ×1 + 全体の再送は最終的に正しい文字列になる。
4. **ESCAPE を足す案(検討したが却下)**: A2 では入力中の文字が無いので ESCAPE は Mozc に食われずアプリへ
   素通りする。Chrome ではオムニボックスの編集取り消しや検索バー・候補ポップアップを閉じるなど、意図しない
   ブラウザ操作になりうる。per-VK 化(ADR-285)後は漏れが常に 1 キーに決まるので、ESCAPE で得るものがない。
   既存コードが `escape_composition: false` のままなのは正しい選択だった。

この時点では**ソースコードの読解のみ**で、実機の TIP・実際の Mozc サーバーでは確認していなかった。

## 実機検証(CI、2026-11-06)

Chrome 自体の cold-start タイミングを CI で再現するのは信頼できない(CI ランナーの Chrome が実デスクトップと
同じ遅延を示す保証がない)。そこで、Chrome の実際の遅延を再現する代わりに、**mozc-readable の TIP 自身を
改修して、awase から見える症状(`key_seq`/`phase` が確認の期限を越えて凍結する)を意図的に作り出した**
(所有者案)。実際の処理(サーバー往復・編集セッション・組成)は普段どおり即座に進み、読み手に見える値だけを
人為的に止める。

- 実装: mozc-readable `exp/mozc-chrome-compose-queue` ブランチ(CI 専用、既に撤去済み——下記)。
  `MOZC_CI_DELAY_AT_KEY_SEQ=<N>` を設定すると、スレッドの `key_seq` が N に達した時点から、次のキーが
  数えられるまで `Flush()`(共有メモリへの公開)を差し止める。
- プローブ(`delayed_key_seq_probe.py`、撤去済み): 素の EDIT コントロール(Chrome 自体は使わない。理由は
  下記「Chrome 自体では確認していない」参照)で、F2 で IME を ON にし、次に送る `k` を遅延対象に指定してから
  `k` を送信。300ms+20ms の間、共有メモリが凍結していることを確認したあと、実際の `MozcComposeCheckFsm` の
  Literal 判定と同じ回収(`Backspace ×1`、ESCAPE なし、全体再送 `ko`)を送り、結果を確認した。
- **結果(CI run `42195405660`、mozc-readable `exp/mozc-chrome-compose-queue` ブランチ、全チェック green)**:
  - `key_seq`/`phase` は 320ms の間、送信前の値(`key_seq=1`, `phase=0`)のまま凍結した(フックが意図どおり
    動いたことの確認)。
  - Backspace+再送のあと、`key_seq` は 1→5、`phase=1` まで正しく進んだ(組成が正しく再開した)。
  - **`passed` カウンタ(素通りした打鍵の数、version 5 診断フィールド)は最初から最後まで 0 のまま**(`k`・
    Backspace・再送の `k`,`o` のどれも文書へ素通りしなかった)。`eaten` は 1→5(すべてサーバーが処理した)。
  - 最終テキストは `PREこ`(事前に置いた `PRE` という文字列は消えず、`k` の漏れも残らなかった)。
  - 4 回の実行のうち、最初の 3 回は検証スクリプト側の不具合(F2 自体が「最初のキー」として誤って遅延対象に
    なった、`cp1252` が「こ」を表示できない、`EM_SETSEL` のカーソル位置指定の誤り)で失敗し、修正して green に
    した(詳細はコミット `96c623d36`〜`34c5470ab`、同ブランチ)。

結論: **PR #809 レビュー M3 の懸念(期限超過後の Backspace が確定済み文字を壊す)は、実機の TIP + 実際の
Mozc サーバーに対しても再現しなかった。** Chrome を除外する前提は成立しない。

## 決定(実装済み、2026-11-06)

1. `is_mozc_compose_check_window`(`crates/awase-windows-base/src/focus/class_names.rs:83-85`)から Chrome 系
   (`Chrome_WidgetWin_1`・`Chrome_RenderWidgetHostHWND`)の除外を外し、空のクラスだけを除く形にした。
2. これに伴い、旧 `chrome_is_left_alone_so_the_leak_is_not_recovered`(`crates/awase-windows/tests/pseudo_mozc_repro.rs`)
   のような「Chrome は確認を張らない」ことを固定していたテストは、`chrome_is_now_recovered_like_any_other_window_after_adr288`
   として「Chrome も確認を張り、正しく回収する」方向に反転した(fix-requires-evidence.md の「キー選択」再発ファミリー
   対象。`class_names.rs` の単体テスト2件も同様に反転)。
3. タイミング定数(`MOZC_COMPOSE_CHECK_MS`)は変えていない。この ADR の主張は「待つ時間を伸ばす」ではなく
   「待った後の回収そのものは安全」という点にあるため。

全テスト(`cargo test -p awase-windows-base --lib`・`cargo test -p awase-windows --test pseudo_mozc_repro
--test pseudo_mozc_chrome_queue --test pseudo_mozc_sweep --test architecture_guard`)・
`cargo check --target x86_64-pc-windows-msvc`・`cargo fmt -- --check` はいずれも green(2026-11-06)。
実機 Chrome 自体での確認はまだ行っていない(下記「残っている懸念」参照、マージ前に確認を推奨)。

## 実機 Chrome での確認(dragonflyg4、2026-11-06)

Chrome 除外を外したビルド(`exp/mozc-chrome-compose-queue`)を実機にデプロイし、実 Chrome(`googlechrome`、
`scoop` 版)に対して BUG-225/226 と同型の repro(別窓〈タスクバー〉へ退避 → Chrome へ戻る
= `ColdReason::FocusChange` → ひらがなキー(F2) → NICOLA Engine 経由の romaji 送信)を 6 回連続で行った
(`chrome_probe --mozc-cold-chrome --repeat=6`、`crates/awase-windows/examples/chrome_probe.rs` に追加)。

結果: **6 回すべてで `[mozc-compose]` が実 Chrome 上で張られ、"composition confirmed" で正しく終わった**
(`cold=4`〜`cold=9`、いずれも confirmed after 15〜16ms、`key_seq` の進みは送った文字数と一致、Backspace は
発生しなかった=素通りなし)。除外を外す前はこの窓クラス(`Chrome_RenderWidgetHostHWND` 等)で
`is_mozc_compose_check_window` が常に偽を返し、`[mozc-compose]` 自体が一度も現れなかった(BUG-226 の LINE と
同じ構造の「確認が張られない」状態)。これで、**窓クラス名の一致判定が実際の Chrome の前景窓に対して正しく
働くこと**(this ADR の唯一の未検証部分だった、CI は素の EDIT コントロールだったため)を確認した。

**この実機確認で捕まえられなかったもの**: 6 回とも `sent 1 chars` で健全に confirmed しており、BUG-225/226 型の
本物の素通り(先頭文字だけ TIP に届かず Backspace 回収が走る場面)は起きなかった(NICOLA Engine が単キーを
直接仮名に訳す経路のため、2 文字以上の romaji バーストを安定して作れなかった)。この「期限超過後の回収」側の
安全性は、実際の Mozc TIP + サーバーに対する CI の人為的遅延実験(run `42195405660`、上記「実機検証(CI)」節)
で既に確認済みであり、本節はそれとは別の懸念(**Chrome の前景窓判定がそもそも効くか**)を補う。

残っている懸念:

- Chrome で本物の先頭文字素通り(A1/A2)自体を実機で意図的に起こす実験は、CI の人為的遅延フック
  (`MOZC_CI_DELAY_AT_KEY_SEQ`)と同じ仕掛けを実機 Mozc Readable に入れないと安定して再現できない。今回は
  未実施(CI 側の検証で代替)。
- Chrome の実際の cold-start 遅延が、本当に A1(キューに積まれるだけ、いずれ届く)なのか、確認していない。
  この ADR の安全性の主張は A1・A2 のどちらでも成り立つので、除外を外す決定自体はこの区別に依存しないが、
  「Chrome でどのくらいの頻度で compose-check が必要になるか」はまだ分からない。

## 撤去した一回限りの検証(one-off-verification-workflows.md)

CI 専用のフック(`MOZC_CI_DELAY_AT_KEY_SEQ`、`tip_state_publisher.cc`)とプローブ(`delayed_key_seq_probe.py`)、
`verify.ps1`/`readable-build.yaml` の配線は、結果をここに記録したので撤去した(mozc-readable
`exp/mozc-chrome-compose-queue` ブランチ、撤去コミット `562eb841f7c47586a8f4af1abbf4768f78d0d7ac`)。
戻す場合は、その 1 つ前のコミット(`34c5470ab`、CI run `42195405660` で green を確認済み)を参照する。
