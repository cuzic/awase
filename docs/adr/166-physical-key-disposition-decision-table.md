---
id: ADR-166
title: |-
  PhysicalKeyDisposition::plan() の全数決定表化、DBEモードキーDown/Up vk非対称ハザードの明文化 (BUG-131)
summary: |-
  不具合報告01M29KDNZ22KNY1FPXSKBGMW7V（「カタカナで状態が固着」）の調査から、
  transport.rs::PhysicalKeyDisposition::plan（物理キーのOSへの配送判断、fix-requires-evidence.md
  記載のreincidence family）の36件の個別サンプルテストだけでは検出できていなかった2つの
  空白セルを発見。(1) kana_mode_restore_key_downラッチの解除条件がDBEキーのDown/Up vk
  非対称（実機でDown=0xF1/0xF2, Up=0xF0を確認）で成立しない構造的欠陥（BUG-131、
  scan_code一致+フォーカス遷移クリア方式に修正）。(2) plan()自体にも同一物理押下内で
  Down=Allow/Up=Suppressとなる組が実在し（Shift+VK_DBE_KATAKANA経路）、GJI/OS側が
  対応するKeyUpを一度も受け取らないという、(1)とは独立の未解決candidate機序が判明
  （opus-adversarial-consult指摘M-2、plan()自体は変更せずpinテストのみ追加）。
  opus-adversarial-consult3ラウンドで、当初「BUG-131が本報告の直接原因」としていた
  確定的な記述は、決定的な反証も確証も無い「有力仮説の一つ」へ格下げした
status: |-
  実装済み・PR #206でwindows-build CI実行済み。初回のwindows-build CIで、決定表
  テストが`plan()`自身の`debug_assert!`（injected==trueならshadow_toggledは必ず
  false、BUG-14ガード）に違反する無効な組み合わせ（injected&&shadow_toggled）を
  生成していたことが実際に検出され、Linuxコンパイル確認・Python独立シミュレーション
  だけでは`debug_assert!`を再現できず見逃していたと判明（CLAUDE.md記載のとおり
  runtime/配下はLinux上での実テスト実行が構造的に不可なため、windows-build CIが
  唯一の実行環境だったことの実例）。生成時に`continue`で該当組み合わせを除外して
  修正、再度CIへ投入。実機でのJISキーボード検証、および本報告の実際の原因特定
  （BUG-131 vs M-2 vs 他候補）は未実施のまま。M-1（同型のDown/Up非対称でhook.rsの
  親指キー押下タイムスタンプも影響を受けうる件）はBUG-132として別途起票、本PRの
  スコープ外
related_adr:
  - "ADR-137"
  - "ADR-119"
  - "ADR-141"
  - "ADR-153"
---

# ADR-166: `PhysicalKeyDisposition::plan()` の全数決定表化、DBEモードキーDown/Up vk非対称ハザードの明文化 (BUG-131)

## 背景

不具合報告 `01M29KDNZ22KNY1FPXSKBGMW7V`（「なぜか、カタカナで状態が固着してしまった」）を
調査した結果、`key_pipeline.rs::kp_restore_hiragana_for_suppressed_mode_key`（ADR-137 決定2）
が持つ M-2 リピート防止ラッチの解除条件に、「KeyDown と対になる KeyUp は同じ vk_code で届く」
という誤った前提があり、実機ではこの前提が成立しないことを発見した（BUG-131、詳細は下記節）。
この調査を`crates/awase-windows/src/runtime/transport.rs::PhysicalKeyDisposition::plan`
（物理キーイベントを実際に OS へ届けるか消費するかの配送判断）自体の決定表化に発展させたが、
opus-adversarial-consult のレビューで **`plan()` 自体にも同一物理押下内で Down=Allow/
Up=Suppress となる組が実在する**（M-2 節参照）ことが判明し、「`plan()` 側は最初から正しく、
利用側の誤読だけが原因」という当初の整理は不正確だったと訂正した。BUG-131（ラッチの欠陥）は
確認済みの独立した設計欠陥として修正したが、それが本報告の症状の**直接原因であるという確証は
得られていない**（詳細は「BUG-131」節末尾参照）。

`plan()` は `.claude/rules/fix-requires-evidence.md` が明記する reincidence family
（「IME actuation 合流点」表の物理キー配送判断枠）であり、36件の個別サンプルテスト
（`crates/awase-windows/src/runtime/transport.rs::plan_tests`）を既に持っていたが、いずれも
「ある1つのシナリオが期待どおりか」を確認するもので、「入力空間全体のうち検証されていない
組み合わせがどこか」を横断的に確認する仕組みはなかった。今回のバグは `plan()` 自体の
空白セルではなく `plan()` の**利用側**の誤読だったが、同種の「決定関数は正しいが利用側が
暗黙の前提を誤る」事故は今後も起こりうる。`src/engine/nicola_fsm.rs::run_flush_matrix`
（BUG-129 発見の全数決定表）と同じ「全数決定表 + 不変条件テスト」パターンを `plan()` にも
適用し、決定表自体を可読なドキュメント（本ADR）として固定する。

## `plan()` の決定表（as-built、2026-09-11時点の実装）

`plan(event, profile, shadow_toggled, is_tsf_mode, f2_warmup_owned, active_ime_kind, dbe_mode_key)`
は以下の優先順位で早期 return する（`transport.rs:253-425`）。各段の条件に一致した時点で
以降の段は評価されない。

| 優先順 | 条件 | 結果 | 備考 |
|---|---|---|---|
| 1 | `profile == InputRelay` | **Allow** | awase はこの窓の actuation を所有しない（issue #136/BUG-90決定4）。他の全軸に関わらず常にAllow |
| 2 | `vk == VK_DBE_HIRAGANA (0xF2)` | `is_tsf_mode && f2_warmup_owned` なら **Suppress**、他は **Allow** | `injected` を含む他の全軸を一切参照しない専用分岐。TSFモード外・MsImeStrategy(`f2_warmup_owned=false`)では素通し必須（BUG-10） |
| 3 | `event.injected == true` | **Allow** | 他プロセスSendInput由来。`shadow_toggled`は設計上ここでは常にfalse（BUG-14ガード、`debug_assert!`で固定） |
| 4 | `vk in {VK_CONVERT, VK_NONCONVERT}` | `ime_relevance.explicit_ime_action_consumed` なら **Suppress**、他は **Allow** | ADR-141: follow-only原則、ADR-153決定1ケース3改のみが例外的にSuppress |
| 5 | `ime_relevance.shadow_action.is_none()`（非KANJI系VK） | **Allow** | 上記以外の一般キーは常に素通し |
| 6a | 上記全て非該当 かつ `profile.can_use_imm32_cross_process()`（Standard） | **Suppress**（Down/Up共） | ImmCross: KANJI系VKは無条件Suppress（`feedback_immcross_owns_kanji`原則） |
| 6b | 上記全て非該当 かつ ImmCross以外（Imm32Unavailable/TsfNative） | `ime_actuation_owned && (shadow_toggled \|\| is_dbe_mode_key_down \|\| event_type==Up)` なら **Suppress** | `ime_actuation_owned = gji_direct_applicable(active_ime_kind) \|\| ms_ime_direct_applicable(active_ime_kind, profile)`（BUG-46）。**KeyUpは`is_kanji_event`なvkなら常にSuppress対象**（vkの種類を問わない、下記「決定表テストが固定する性質」参照） |

段6bの `is_dbe_mode_key_down` は以下の**すべて**を満たす場合のみ true:
`dbe.policy == Suppress` かつ `vk in {0xF0,0xF1,0xF3,0xF4}` かつ `event_type == Down` かつ
`!shift_katakana_passthrough(event, dbe)`。

`shift_katakana_passthrough`（ADR-137 決定1、BUG-116 唯一の抜け道）は
`vk == VK_DBE_KATAKANA(0xF1) && event_type==Down && shift && !half_width_alnum_toggle_active
&& !is_configured_thumb_key` の場合のみ true——**0xF1 の Down 単体にしか作用しない**。
0xF0/0xF3/0xF4 や、0xF1 の Up には一切作用しない。

## 決定表テストが固定する性質

`crates/awase-windows/src/runtime/transport.rs::plan_tests` に、36件の既存サンプルテストに
加えて次を追加した（`nicola_fsm.rs::run_flush_matrix` と同型、削除せず維持）:

- `run_plan_matrix()`: VK種別を6分類（`VK_DBE_HIRAGANA` / `VK_DBE_KATAKANA` /
  その他DBEモードキー3種 / `VK_CONVERT`・`VK_NONCONVERT` / 一般KANJI系 / 非KANJI系）し、
  各分類ごとに意味のある軸だけを総当たりして `PlanRow` を生成する（計1296行。
  `injected && shadow_toggled` という `plan()` 自身の `debug_assert!` が禁じる
  無効な組み合わせは生成時に `continue` で除外する——実機windows-build CIで
  この除外漏れ2件が実際に検出された、Python独立シミュレーションだけでは
  `debug_assert!` を再現できず見逃していた）。
- `plan_matrix_covers_all_branches_without_panicking`: 全1296行の構築自体が「任意の入力で
  panicしない」を実質的に検証する。
- `kanji_family_keyup_suppress_verdict_is_independent_of_specific_vk`: **段6bのKeyUp分岐は、
  `profile`/`shadow_toggled`/`active_ime_kind`/`injected`/`dbe_policy`が同じなら、vkが
  0xF0/0xF1/0xF3/0xF4のどれであってもSuppress判定が一致する**、という`plan()`自身が持つ
  性質を固定する（この`dbe_family`配列は`VK_DBE_HIRAGANA`を含まない——0xF2は段2の専用分岐
  のため対象外。段2を挟むDown/Upペアの整合性は下記`shift_katakana_down_allow_can_pair_
  with_keyup_suppress_pin`が別途扱う）。
- `input_relay_always_allows_regardless_of_other_axes`: 優先順1の性質を一般化して固定する。
- `shift_katakana_down_allow_can_pair_with_keyup_suppress_pin`（opus-adversarial-consult
  指摘M-2）: 同一物理押下内でDown=Allow・対応するUp=Suppressとなる組が実在することを
  現状の仕様として固定する（`plan()`の不具合ではなく、下記「M-2」節で述べる独立の
  未解決candidate機序があることの記録）。

## BUG-131: `kana_mode_restore_key_down` ラッチのDown/Up vk非対称ハザード（確認済みの設計欠陥）

**実機で確認した事実**: JIS配列「カタカナ ひらがな ローマ字」キー（scan=0x70）は、Windows の
キーボードレイヤーが押下時と離鍵時で別々に現在の IME モードを見て vk を合成するため、
**同一の物理押下内で KeyDown と KeyUp の vk_code が一致するとは限らない**。実機ログ
（report `01M29KDNZ22KNY1FPXSKBGMW7V`）で、KeyDown は切替先モードに応じ
`VK_DBE_KATAKANA`(0xF1)/`VK_DBE_HIRAGANA`(0xF2) だったのに対し、対応する KeyUp は
**5/5件すべて** `VK_DBE_ALPHANUMERIC`(0xF0) で届いた（`scan_code` は Down/Up 双方とも
`0x70` で一致していた——vk_code ではなく scan_code の方が物理キーの同一性を表す安定な軸）。

`key_pipeline.rs::kp_restore_hiragana_for_suppressed_mode_key`（ADR-137 決定2）の
M-2 リピート防止ラッチ (`kana_mode_restore_key_down`) は、旧実装では
`event.vk_code == VK_DBE_HIRAGANA` の KeyUp のみを離鍵と認めていた。実機ではこの KeyUp が
`0xF0` で届くため、解除条件が構造的に一度も成立せず、条件を満たした KeyDown でラッチが
セットされた場合はプロセス生存中ずっと解除されなくなる（コードを読めば直ちに確認できる
設計欠陥。修正・テストは `docs/known-bugs/BUG-131.md` 参照）。

**本報告の直接原因かどうかは未確定**: opus-adversarial-consultの検証で、ラッチをセットする
唯一の箇所（`send_gji_half_width_alnum_toggle`呼び出し直後の無条件`tracing::info!`）に対応する
成功ログが、報告に添付された app_log（`awase.log`の末尾200KB切り出し、`bug_report.rs::
truncate_text_tail`）中に1件も無いことが判明した。ログ切り出しがプロセス起動時点
（`"Keyboard Layout Emulator starting..."`）を含んでいれば「ラッチが一度も立たなかった」と
決定的に言えるが、本報告のexcerptはこの起動行を含んでおらず（末尾切り出しのため、より
早い時点での成功が窓の外に落ちた可能性を排除できない）、ラッチ説を確定的に排除することも
確証することもできなかった。加えて、他に4つの無ログ早期return候補（`is_configured_
thumb_key`/`half_width_alnum_toggle_before`/`conv_mutation_allowed`(M-4)/ラッチ自身）が
同じ「ログ0件」という観測と区別なく両立するため、根本原因の特定はできていない。

## M-2（opus-adversarial-consult指摘）: `plan()` 自体にもDown/Up非対称の未解決candidate機序がある

決定表化の過程で、`plan()` 自体にも**同一物理押下内でDown=Allow・対応するUp=Suppressとなる組**
が実在することが判明した（`shift_katakana_down_allow_can_pair_with_keyup_suppress_pin`が
固定）。`Shift+VK_DBE_KATAKANA`のKeyDownは`shift_katakana_passthrough`（ADR-137決定1）に
よりAllowされ実IMEへ届くが、実機ではこの物理キーのKeyUpは`VK_DBE_ALPHANUMERIC`相当で届き、
段6b（KeyUpは`is_kanji_event`なvkなら常にSuppress、vkの種類を問わない）によって**GJI/OS側は
この物理押下に対応するKeyUpを一度も受け取らない**。

これはBUG-131（awase側の復元ラッチ）とは独立した、**GJI自身の内部状態が「このキーはまだ
押下中」のまま崩れうる**候補機序であり、報告された症状（「カタカナで状態が固着」）に対して
BUG-131より直接的な説明になっている可能性がある。ただし:

- `dbe_mode_key_policy = Passthrough`（ADR-091 §D3.6の隠し設定）は**この経路の回避策には
  ならない**。この設定が無効化するのは `is_dbe_mode_key_down`（Down側の判定）のみで、
  段6bのKeyUp無条件Suppressは`dbe.policy`に一切ゲートされていない。
- 段6bの「KeyUpは常にSuppress」という条件自体はBUG-52対策の中核であり、ADR-100/BUG-50が
  意図的に確定させてきた設計（`docs/experiments.md`参照）に踏み込む変更になる。`plan()`は
  既に複数系統の敵対的レビューを経た枯れた関数であり、本ADR/本PRでは**修正せず、現状を
  pinテストで固定するに留める**。

**今後の切り分け方針（未実施）**: ADR-137自身が`38ca9a04`で採った「診断スパイクで実機
A/Bを取ってから恒久修正を設計する」という方針を踏襲する。段6bの`event_type==Up`による
Suppressを一時的にDBE系vkだけ無効化した診断ビルドを作り、カタカナ固着が実際に解消するかを
実機で確認してから、恒久対応（例: 段6bに「Downが`shift_katakana_passthrough`でAllowされた
scan_codeのUpは除外する」条件を追加する等）を設計する。

## 一般ルール

DBE合成キー（`VK_DBE_ALPHANUMERIC`〜`VK_DBE_DBCSCHAR`、0xF0-0xF4）の Down/Up をペアリング
するコードを新たに書く場合、**vk_code の一致を前提にしてはならない**。`scan_code` の一致
（実機で確認済みの安定軸、BUG-131の修正が採用）を第一候補とすること。`crate::vk::
is_synthetic_dbe_ime_hotkey`（0xF0-0xF4を一括判定する既存ヘルパー）でDBE合成キー群全体を
1つの論理キーとして扱う方式は次善であり、`rewritten_vk`（ADR-140/143のキー役割代入）で
Up側のvkが書き換わる構成には脆弱なため、scan_codeが使える場面ではそちらを優先すること。

## 関連ADR・バグ

- [ADR-137](137-shift-katakana-dbe-mode-key-suppression-regression.md)（BUG-116）:
  本ADRが決定表化した `plan()` の段6b（`is_dbe_mode_key_down`）と `shift_katakana_
  passthrough` を導入した決定1、その埋め合わせとして `kp_restore_hiragana_for_
  suppressed_mode_key` を導入した決定2（BUG-131はこの決定2の実装欠陥）。
- BUG-52: 段6bの無条件Suppress自体の由来。
- BUG-46: `ime_actuation_owned` を`profile`単独ではなく`ActiveImeKind`からも導出する理由。
- [ADR-119](119-injected-and-relay-key-consumption-invariant.md)（issue #136）: 優先順1(InputRelay)。
- [ADR-141](141-henkan-muhenkan-delegate-inactive-recovery.md): 優先順4(CONVERT/NONCONVERT)。
- [ADR-153](153-gji-keymap-aware-safe-vk-substitution-for-mode-keys.md): GJIのTSFキー横取りと
  ケース3改（優先順4の例外条項）。
- `docs/known-bugs/BUG-131.md`: BUG-131（ラッチ設計欠陥）の症状・修正記録。
- `docs/known-bugs/BUG-132.md`: M-1（`hook.rs`の`LEFT_THUMB_DOWN_AT_US`が同型のDown/Up
  vk非対称で影響を受けうる件、本PRのスコープ外として別途起票）。
