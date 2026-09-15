---
id: ADR-173
title: |-
  `muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action` を `AppImeProfile::TsfNative` に限定する
status: |-
  起草中（opus-adversarial-consult 未実施）
related_adr:
  - "ADR-153"
  - "ADR-121"
  - "ADR-149"
---

# ADR-173: `muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action` を `AppImeProfile::TsfNative` に限定する

## 背景

2026-09-15、Windows Terminal + PowerShell + GJI で、半角モード（直接入力）中に無変換/
変換キーを単独タップすると2つの症状が実機で確認された。

1. **「@」の混入**（BUG-113 と同一症状）。
2. **物理半角/全角キーの「IME ONへの固着」**: 上記操作の直後から、物理半角/全角
   キーを押しても IME が OFF に戻らなくなる（新規観測、本 ADR 執筆時点では
   `docs/known-bugs/` に未記録）。

いずれも **awase.exe を完全に停止した状態でも再現する**ことをユーザーが実機で確認
済み（2026-09-15）。したがって根本原因は awase の送信ではなく、GJI 自身が無変換/
変換キーの物理押下をネイティブに処理する際の TSF キー横取り（`ITfKeyEventSink`）に
あると考えられる——BUG-113 が 2026-09-07 に「無変換キー単独タップの『@』は
awase 側のactuation・送信回数とは完全に無関係」と確定した結論と一致する。

## 既存の関連機構: `muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`

ADR-153 決定1（2026-09-08、BUG-122/123/124 を経て確定）は、この GJI 自身の
TSF キー横取りを避けるため、無変換/変換の生キーを awase が抑止し GJI に一切
渡さないケース3改（`config.toml` の `muhenkan_solo_tap_ime_action = "off"`
等）を既に実装・実機検証済みである
（`crates/awase-windows/src/runtime/key_pipeline.rs::explicit_ime_action_target`）。
`explicit_ime_action_target` の doc comment 曰く「この抑止だけなら実機A/B実験
（`docs/experiments.md` エントリ25 Phase3）で『@』が完全に消えることを確認済み」。

**問題**: この設定は**グローバル**であり、有効にすると全アプリで無変換/変換
単独タップの挙動が変わる。ユーザーは「PowerShell（＝Windows Terminal 上の
TSF ネイティブ経路）でだけこの回避策を効かせたい」と明言しており、他アプリ
（Standard/Imm32Unavailable プロファイル）では GJI 自身のネイティブなかな
切替に委ねる既存動作を維持したい。

## 決定（案、opus-adversarial-consult 未実施）

`explicit_ime_action_target`（`key_pipeline.rs`）の先頭に、現在のフォーカス
アプリが `AppImeProfile::TsfNative` であることを要求するガードを追加する。
`self.platform.current_app_profile()`（既に `key_pipeline.rs` の複数箇所
——`:455`, `:659`, `:2926`, `:2994`, `:3060`——で参照されている既存アクセサ）
を使う。

対象外（`Standard`/`Imm32Unavailable`/`InputRelay`）のプロファイルでは、
現状どおり `ExplicitImeActionOutcome::Inactive` を返し、この設定自体は
一切発火しない。

### なぜ `AppImeProfile::TsfNative` で区切るのが妥当か

GJI 自身の TSF キー横取り（`ITfKeyEventSink`）は TSF text store を持つ
ホストアプリ内でのみ意味を持つ機構であり、`AppImeProfile::TsfNative`
（Windows Terminal/WezTerm 等、`focus/class_names.rs::is_tsf_native_window`）
はまさにこの区別のために既に存在する分類軸である。ユーザーの要望
「PowerShell でだけ」は実際には「PowerShell というシェル」ではなく
「PowerShell を動かしている Windows Terminal というホストアプリの TSF
経路」が条件であり、新しいアプリ許可リストを config に追加するのではなく、
既存の `AppImeProfile` 分類を再利用するのが最小の変更になる。

## 未確定・opus-adversarial-consult で検証すべき点

1. **「IME ON固着」自体の機序は未確定。** 今回確認できたのは「無変換/変換の
   生キー抑止で『@』が消える」という ADR-153 の既存知見のみであり、「IME ON
   固着」が同じ生キー抑止で解消するかどうかは**未検証**（実機再検証が必要）。
   固着が「物理半角/全角キーの OS/ドライバ側トグル記憶」の破壊であり
   「無変換/変換の生キーが GJI の TSF 処理へ渡ったこと」自体が引き金なら、
   本 ADR の抑止で解消するはずだが、別の独立した機序（例: GJI 側の内部状態
   破壊がキー抑止の有無に関わらず起きる）である可能性も残る。
2. `AppImeProfile::TsfNative` への限定が、ADR-153 が既に検証した
   「ケース2（OFF→ON昇格）」「ケース3改（抑止のみ）」の実機確認済み挙動
   （`Imm32Unavailable`/`Standard` プロファイルでも一部有効だった可能性）
   を意図せず後退させないか。ADR-153 の実機検証がどのプロファイルの
   アプリで行われたか（Windows Terminal = TsfNative）を再確認する必要が
   ある。
3. `current_app_profile()` はフォーカス変更時にキャッシュされる値であり、
   `explicit_ime_action_target` 呼び出し時点のフォーカスと一致するかの
   フェンシングが必要かどうか（同種の懸念は ADR-140/163 で繰り返し登場して
   いる）。

## 次のアクション

1. 本 ADR を opus-adversarial-consult にかけ、上記未確定点を解消する。
2. 実装後、dragonflyg4 実機で以下を確認する:
   - Windows Terminal + PowerShell + GJI: 半角モードで無変換/変換 → 「@」・
     「IME ON固着」とも再現しないこと。
   - 他アプリ（例: メモ帳 = Standard、Chrome = Imm32Unavailable）: 無変換/
     変換単独タップの挙動が本 ADR 適用前と変わらないこと（回帰確認）。
3. `.claude/rules/fix-requires-evidence.md` の要求に従い、回帰テスト
   （`explicit_ime_action_target` の単体テスト、プロファイル別の期待値）
   と `docs/known-bugs/BUG-142.md`（新規、「IME ON固着」の記録）を追加する。

## 関連

ADR-153（`muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action` の
設計・ケース2/3改の実機検証、本 ADR が再利用する既存機構）、BUG-113
（GJI 自身の TSF キー横取りによる「@」の根本原因）、ADR-121/149（TsfNative
プロファイルの物理 IME キー処理の設計変遷）。
