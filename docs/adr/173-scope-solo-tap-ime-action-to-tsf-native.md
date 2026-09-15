---
id: ADR-173
title: |-
  `muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`(ケース2/3改)をプロセス名指定のアプリ限定にする
status: |-
  round1でBlocker4件・Should-fix6件（`AppImeProfile::TsfNative`はWindows
  Terminalを構造的に取りこぼす、KeyDown/KeyUpペアリング不変条件の破壊、
  ケース1(コア側)の扱い未定、「IME ON固着」の因果関係の取り違え）を検出。
  round1指摘を反映し、決定を`app_overrides`方式のプロセス名リストへ変更。
  「IME ON固着」はBUG-142として本ADRの成否根拠から切り離した。round2待ち。
related_adr:
  - "ADR-153"
  - "ADR-121"
  - "ADR-149"
  - "ADR-172"
---

# ADR-173: `muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`(ケース2/3改)をプロセス名指定のアプリ限定にする

## 背景

2026-09-15、Windows Terminal + PowerShell + GJI で、半角モード（直接入力）中に無変換/
変換キーを単独タップすると2つの症状が実機で確認された。

1. **「@」の混入**（BUG-113 と同一症状）。
2. **物理半角/全角キーの「IME ONへの固着」**: 上記操作の直後から、物理半角/全角
   キーを押しても IME が OFF に戻らなくなる（[BUG-142](../known-bugs/BUG-142.md)
   として新規記録）。

いずれも **awase.exe を完全に停止した状態でも再現する**ことをユーザーが実機で確認
済み（2026-09-15）。したがって「@」の根本原因は awase の送信ではなく、GJI 自身が
無変換/変換キーの物理押下をネイティブに処理する際の TSF キー横取り
（`ITfKeyEventSink`）にあると考えられる——BUG-113 が 2026-09-07 に「無変換キー
単独タップの『@』は awase 側のactuation・送信回数とは完全に無関係」と確定した
結論と一致する。

**「IME ON固着」については、本 ADR では因果関係を主張しない**
（下記「非スコープ」参照、round1 で本 ADR の根拠から切り離した）。

## 既存の関連機構: `muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`

ADR-153 決定1（2026-09-08、BUG-122/123/124 を経て確定）は、この GJI 自身の
TSF キー横取りを避けるため、無変換/変換の生キーを awase が抑止し GJI に一切
渡さないケース3改（`config.toml` の `muhenkan_solo_tap_ime_action = "off"`
等）を既に実装・実機検証済みである
（`crates/awase-windows/src/runtime/key_pipeline.rs::explicit_ime_action_target`、
belief OFF 側、以下「ケース2/3改」）。`explicit_ime_action_target` の
doc comment 曰く「この抑止だけなら実機A/B実験（`docs/experiments.md`
エントリ25 Phase3）で『@』が完全に消えることを確認済み」。

**問題**: この設定は**グローバル**であり、有効にすると全アプリで無変換/変換
単独タップの挙動が変わる。ユーザーは「PowerShell（＝Windows Terminal 上の
TSF ネイティブ経路）でだけこの回避策を効かせたい」と明言しており、他アプリ
（例: メモ帳、Chrome）では GJI 自身のネイティブなかな切替に委ねる既存動作を
維持したい。

## 却下した代替案: `AppImeProfile::TsfNative` ガード

起草時点（round1 前）では、`explicit_ime_action_target` の先頭に
`self.platform.current_app_profile() == AppImeProfile::TsfNative` という
ガードを足す案を提示していた。**opus-adversarial-consult round1 でこの案は
2つの Blocker により却下された。**

1. **Windows Terminal を構造的に取りこぼす。** Windows Terminal のホスト
   ウィンドウクラス `CASCADIA_HOSTING_WINDOW_CLASS` は
   `focus/class_names.rs::IMM32_UNAVAILABLE_CLASSES` に含まれており、
   `from_class_name` は IMM32 リストを TSF ネイティブリストより**先に**評価
   するため、`AppImeProfile::TsfNative` には**決してならず**
   `Imm32Unavailable` になる（`class_names.rs:399-406` の名前付き回帰テスト
   `cascadia_profile_is_masked_to_imm32_unavailable` がこれを固定している）。
   本 ADR が対象と名指しした Windows Terminal 自身が、素朴な
   `AppImeProfile` 比較では取りこぼされるという自己矛盾があった。
   `is_effectively_tsf_native()`（`class_names.rs:234-247`、この種の誤判定を
   避けるための既存ヘルパー、doc がこの落とし穴を名指しで警告している）に
   差し替えれば個別の誤判定は直るが、次の問題が残る。
2. **KeyDown/KeyUp ペアリング不変条件を壊す。** 実機 journal
   （`crates/awase-windows/tests/journals/actuation_decision/
   bug-131-report-01m29kdnz.json`）では、Windows Terminal + GJI の同一
   セッション内でフォーカス先のウィンドウクラスが `TsfNative ⇔
   Imm32Unavailable` を**往復**している（GJI 候補ウィンドウの表示/非表示、
   Cascadia/InputSite 子ウィンドウ間のフォーカス遷移等が原因）。
   `explicit_ime_action_target` は KeyUp 側早期分岐（`key_pipeline.rs:
   1274-1281`）と KeyDown 側本体（`:1353`）の2箇所から呼ばれ、
   `:1197-1203` の doc は「KeyDown と KeyUp の間で belief が変化しない限り
   同じ結論になる」というステートレスな不変条件に意図的に依拠している
   （ステートフルなラッチは取り出し漏れリスクがあるため避けた、ADR-153
   決定1）。ここに**フォーカスプロファイル**という新しい入力を足すと、
   この不変条件が「belief 不変」から「belief かつプロファイル両方が不変」
   に暗黙に拡張されるが、プロファイルは class 単位で往復しうる（process は
   変わらない）ため、KeyDown 時と KeyUp 時で判定が食い違い、ADR-153 が
   M19 で塞いだ「孤立 KeyUp が GJI へ漏れる」非対称を作り直しうる。

**結論**: ウィンドウクラスベースの判定は、Windows Terminal のような
「1プロセス内で複数の子ウィンドウクラスを往復するアプリ」とは原理的に
相性が悪い。プロセス名ベースの判定に切り替える（下記決定）。

## 決定

### 決定1: `app_overrides.solo_tap_ime_action_apps`（プロセス名リスト）を新設し、ケース2/3改をこのリストに限定する

`src/config.rs::AppOverrides` に、既存の `disable_apps`/`input_relay_apps`
と同じ形（`Vec<String>`、大文字小文字無視・`.exe` 有無どちらでも一致、
`state/app_suppression::matches_disabled_app` を再利用）で
`solo_tap_ime_action_apps: Vec<String>`（既定値: 空 = 無効）を追加する。

`focus/tracker.rs::FocusTracker` に、`input_relay_apps`
（`:80-87`、`overrides.input_relay_apps()` 経由でプロセスグローバルに
キャッシュされる既存パターン）と同じ配線で `solo_tap_ime_action_apps` を
保持させ、`solo_tap_ime_action_in_scope() -> bool`
（`matches_disabled_app(&self.solo_tap_ime_action_apps,
self.process_name())`）を追加する。

`key_pipeline.rs::explicit_ime_action_target`（ケース2/3改、belief OFF 側）
の先頭で、`self.platform.focus.solo_tap_ime_action_in_scope()`
（`self.platform.focus.process_name()` は `key_pipeline.rs:3095` で
既に使われている既存アクセサ）が `false` なら
`ExplicitImeActionOutcome::Inactive` を返す。

**プロセス名を選ぶ理由**: `WindowsTerminal.exe` というプロセス名は、内部の
子ウィンドウクラスが `CASCADIA_HOSTING_WINDOW_CLASS ⇔
Windows.UI.Input.InputSite.WindowClass` を往復しても変わらない。プロセス名
ベースにすることで、上記「却下した代替案」の Blocker 1・2 を両方とも
構造的に回避できる（フォーカスが Windows Terminal プロセス内に留まる限り、
KeyDown/KeyUp 間で判定が変わらない）。

### 決定2: ケース1（コア側、belief ON）は意図的にスコープ対象外のまま残す

`muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action` の設定値は、
`crates/awase-windows` 側のケース2/3改だけでなく、**コア側**
（`src/engine/nicola_fsm.rs::resolve_explicit_ime_action`、
`resolve_pending_thumb_as_single` の優先順位2、belief ON 側、以下
「ケース1」）からも読まれる。コアクレートは ADR-019 により OS 依存型
（`AppImeProfile`、プロセス名）を持てないため、決定1のスコープ判定を
ケース1に同じ形で適用することはできない。

**決定: ケース1はスコープ対象外のまま、グローバルに残す。** 理由:

- ケース1が発火するのは belief **ON** のとき（IME が開いている状態で
  無変換/変換を単独タップし、GJI 自身のひらがな/カタカナ/半角カナ切替に
  介入する場面）。今回報告された症状（「@」・「IME ON固着」）は、いずれも
  **belief OFF**（半角モード/直接入力）のときに無変換/変換を押した場合に
  限定されている。ケース1とケース2/3改は排他的な belief 条件で分岐して
  おり（`explicit_ime_action_target` 冒頭、`current` が true なら早期
  `Inactive`）、**ケース1は今回の症状と挙動が重ならない**。
- ケース1をアプリ限定するには、プラットフォーム層が事前分類した
  真偽値をコアエンジンの入力インタフェース（`RawKeyEvent`/
  `ImeRelevance` 相当の事前分類フィールド）へ新規に追加する必要があり、
  ADR-019 の層境界を守ったまま実装するには構造的な変更が要る。今回の
  症状を解消するために必要な範囲を超えるため、本 ADR のスコープ外とする。

**この判断により、ユーザー要望と部分的に矛盾する余地が残る**: 他アプリ
（例: メモ帳、Chrome）で belief **ON** のまま無変換/変換を単独タップした
場合、ケース1の明示 config が引き続きグローバルに効く。ただしこれは
「無変換/変換の生キーが GJI に渡り TSF キー横取りが起きる」経路とは無関係
（ケース1は明示的に IME を操作する設計であり、GJI の生キー横取りを
経由しない）ため、今回の「@」/固着とは別の懸念であり、ユーザーが
明示的に設定した挙動（`muhenkan_solo_tap_ime_action` を意図的に設定した
場合のみ発生）である点も踏まえ、許容する。

## 「IME ON固着」を本ADRの根拠から切り離す（round1 B4 対応）

**round1 opus-adversarial-consult の指摘**: 「awase.exe を完全に停止した
状態でも再現する」という観測は、「GJI が最終的な加害者である」ことの証拠には
なるが、「awase 側の生キー抑止で連鎖が切れるか」の証拠にはならない。さらに
**awase 稼働時、物理半角/全角キー自体は `transport.rs::
PhysicalKeyDisposition::plan`（既定 `dbe_mode_key_policy = Suppress`）
により無条件で Suppress され、そもそも GJI へ届いていない**——したがって
「半角/全角を押しても戻らない」症状の少なくとも一部は、GJI が半角/全角
キーをどう扱うかとは独立に、awase 自身の actuation/belief 側の問題である
可能性が高い。本 ADR（無変換/変換の生キー抑止）がこの経路に触れる保証は
無い。

ADR-172 が整理した TsfNative の ON 方向救済4系統（force-on/drift
correction/warmup/reassert）のいずれかが、ユーザーの OFF 操作を打ち消す
形で再表明している対立仮説も未検討のまま残っている。

**決定: 「IME ON固着」は [BUG-142](../known-bugs/BUG-142.md) として独立に
記録し、本 ADR の受け入れ基準（下記「次のアクション」）から外す。** 本 ADR
の実装後に固着が再現しなくなったとしても、それを「無変換/変換の生キー抑止が
効いた」証拠として扱わない——タイミング依存でマスクされた可能性を排除
できないため。

## 未確定・opus-adversarial-consult round2 で検証すべき点

1. **決定1の process_name アクセサのフェンシング**: `FocusTracker` の
   `process_name()`/`solo_tap_ime_action_in_scope()` は他の `current_app_
   profile()` 系アクセサと同じくフォーカス変更時にキャッシュされる値で
   あり、フォーカス着地直後の stale window（`focus_tracking.rs:139-144`
   が記録する BUG-114 根本原因1と同型）を持つ可能性がある。決定1はこの
   プロセス名ベースの切り替えにより B1/B2（クラス往復由来の誤判定）は
   解消するが、S1 型の stale window（フォーカス変更直後の一時的な
   誤判定、fail-open で生キーが GJI へ漏れる方向）が残るかどうかは
   round2 で確認する。
2. ログ: ガードが弾いたとき（スコープ外で `Inactive` を返したとき）に
   `tracing::info!` 等でプロファイル/プロセス名を記録するか
   （`kp_stage_shadow_ime_toggle` の他の分岐は全てログを出している）。
3. 回帰テストの置き場所: `crates/awase-windows/src/runtime/mod.rs` 配下は
   `#[cfg(windows)]` が掛かっており、`#[cfg(test)]` ユニットテストは
   Linux の `cargo nextest run --workspace --lib` では一切検証されない
   （CLAUDE.md 既知の制約）。`tests/architecture_guard.rs`
   （Linux で走るソーススキャン型）に、決定1のガード呼び出しが
   `explicit_ime_action_target` に存在することを固定する回帰を追加する
   か検討する。
4. `matches_disabled_app` の純粋関数テスト自体（`state/app_suppression.rs`）
   は Linux で走る——`solo_tap_ime_action_apps` の照合ロジック自体は
   ここに追加のユニットテストを足せる。

## 次のアクション

1. 本 ADR を opus-adversarial-consult round2 にかけ、上記未確定点を解消する。
2. 実装後、dragonflyg4 実機で以下を確認する（本 ADR の受け入れ基準）:
   - Windows Terminal + PowerShell + GJI: `config.toml` の
     `[app_overrides] solo_tap_ime_action_apps = ["WindowsTerminal.exe"]` +
     `muhenkan_solo_tap_ime_action = "off"` 設定時、半角モードで無変換/変換
     → 「@」が再現しないこと。
   - 他アプリ（例: メモ帳 = Standard、Chrome = Imm32Unavailable）:
     `solo_tap_ime_action_apps` にリストされていない状態で、無変換/変換
     単独タップの挙動が本 ADR 適用前と変わらないこと（回帰確認）。
   - **「IME ON固着」の再現有無は記録するが、本 ADR の成否判定には使わない**
     （BUG-142 側で別途検証する）。
3. `.claude/rules/fix-requires-evidence.md` の要求に従い、回帰テスト
   （`state/app_suppression.rs` の `solo_tap_ime_action_apps` 照合テスト、
   可能なら `architecture_guard.rs` のガード存在確認）を追加する。
4. BUG-142 の調査は本 ADR とは独立に進める（journal `ActuationDecision`
   レコードによる ADR-172 4系統の内訳確認が最初の手順）。

## 関連

ADR-153（`muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action` の
設計・ケース2/3改の実機検証、本 ADR が再利用する既存機構）、BUG-113
（GJI 自身の TSF キー横取りによる「@」の根本原因）、ADR-121/149（TsfNative
プロファイルの物理 IME キー処理の設計変遷）、ADR-172（TsfNative の ON
方向救済4系統の整理、BUG-142 の調査手順の前提となる journal 記録基盤）、
BUG-142（本 ADR から切り離した「IME ON固着」の独立記録）。
