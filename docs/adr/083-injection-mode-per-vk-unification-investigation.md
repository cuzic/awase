# ADR-083: `InjectionMode`（文字送信経路）をGJI専用にper-VK確認方式へ統一する構想の検討記録

## ステータス

**検討フェーズ。統一自体はNO-GO。観測専用の診断配線のみ実施済み（2026-08-03）。**

対象: `crates/awase-windows`のIME文字送信経路（`InjectionMode::{Unicode,Vk,Tsf}`）。

## コンテキスト

ADR-081（プロファイル別ドライバ分離）Phase 1d検討の過程で「GjiFsm（TSF composition
状態機械）の同期義務がprofileごとに非対称」という問題を発見し、その調査を深める中で、
LINE（IMM32互換アプリ、`AppImeProfile::Standard`=ImmCross）での実際の入力フローを
解明する必要が生じた。この調査がさらに発展し、「文字送信経路（`InjectionMode`）自体を、
GJIがアクティブなときはprofileを問わずper-VK（1キーずつ確認しながら送る）方式に統一
したい」という、より大きな構想の検討に至った。MS-IMEにはGJIのような「プロセスの
cold-start（sleepからの復帰）」問題が無いため、この構想はGJI専用にスコープされている。

Opus・Fable・Codex（Codex CLI）の3系統に独立してこの構想の設計書をレビューさせた
結果、驚くほど収束した結論が得られた。本ADRはその結論と、判明した事実誤認の訂正を記録する。

## 決定

**現時点での`InjectionMode`統一（実際の送信方式の変更）はNO-GO。** 代わりに、統一の
実現可能性を左右する最重要要素（HIMC照合がIMM32互換アプリで実際に使えるか）を、
実挙動を一切変えない**観測専用の診断ログ**として先に実機で確認するフェーズに着手する。

### NO-GOの理由

1. **統一の中核である「per-VK confirm」機構自体が、直近（2026-07-29、BUG-45）発見
   された未解決の構造的欠陥を抱えている。** confirm判定は実際の出力の直接観測ではなく
   「候補ウィンドウSHOW/GJI I/O増加という代理証拠のタイムアウト判定」に過ぎない。
   Claude本体・Fable・Codexの3系統で検証してなお未解決。この状態で確認実績ゼロの
   アプリ群（LINE等のIMM32互換アプリ）へ適用範囲を広げるのは、失敗の分母を増やす
   行為になる。
2. **統一の「追い風」としていた前提（Unicode注入がIMM32アプリでGJIの実compositionに
   ライブに取り込まれる）は、証拠が支える強さを超えていた。** 実際に観測できているのは
   `gji_write_bytes()`（GJIプロセスのI/O）の増分のみで、「注入した文字が変換可能な
   preeditに入った」ことの直接証拠ではない。
3. **warm時のper-VK confirmレイテンシが、NICOLA高速打鍵の間隔に間に合うか、
   全く未実測。** 間に合わなければ構想自体が成立しない、最大の未知数。

### GO判定の範囲（観測専用フェーズ）

`capture_composition_snapshot`（`ime.rs:1124`、`GCS_COMPSTR`/`RESULTSTR`等を
クロスプロセスHIMC照合で読む機能）は既に実装済みで、`log_composition_probe`
（`ime_diagnostic.rs:275`）経由で`Vk`/`Tsf`モードのper-VK confirm判定点11箇所に
既に配線されている——ただし**診断ログ用途のみで、判定ロジックには一切使われていない**。
この呼び出しを`Unicode`モードの経路（`UnicodeLiteralObserverFsm::tick`）にも1箇所
追加し、LINEのようなIMM32互換アプリで`comp_str`/`himc_null`等が実際に何を返すかを
実機ログで測定する。**判定ロジック（`ProbeAction`の分岐）は一切変更しない。**

## 調査で訂正された事実誤認

検討の過程で作成した設計書ドラフトには、以下の誤りがあった（Opusのレビューで発見）。
将来この構想を再検討する際に同じ誤りを繰り返さないよう記録する。

1. **`InjectionMode`は`AppImeProfile`ではなく`AppKind`から決まる。** 「GJIがアクティブ
   なときはprofileを問わず統一する」という構想の枠組み自体が、実装の分岐軸と
   ずれていた。`AppKind::TsfNative`→`Vk`、それ以外→`Unicode`という判定は、
   `AppImeProfile`（Standard/Imm32Unavailable/TsfNative）と一致しない（例:
   `Windows.UI.Input.InputSite.WindowClass`は`AppImeProfile`上TsfNative寄りだが
   `AppKind::Uwp`扱いでデフォルトはUnicode）。
2. **「Tsfモードはforce_tsf設定時の2件（WezTerm/Windows Terminal）のみ」は誤り。**
   ADR-062の実行時自己学習機構（`UnicodeLiteralObserverFsm`→`InjectionModeStore`→
   `cache.toml`の`[injection_mode]`）が、GJI write bytesが増えないと判明した任意の
   ウィンドウクラスを動的にTsfへ昇格させる。Tsfモードは実行時に増殖しうる。
3. **`platform.rs:296-298`のコメント（「Unicode injectionはGJI TSF contextを迂回する」）
   はアプリ種別を区別しない無条件の記述であり、「TSFネイティブアプリ限定」という
   解釈の根拠には使えない。**
4. **HIMC照合と同種の実験（`ImmGetCompositionStringW`によるcomposition検出）は、
   過去に一度試されて撤回されている**（`558c39f`→`b643bac`、2026-05-15）。ただし
   撤回理由は「WezTerm（TSFネイティブアプリ）では`ImmGetCompositionStringW`が
   常に0を返す」であり、**IMM32互換アプリ（LINE等）での検証ではない**。今回の
   観測フェーズは、この過去の失敗の再演ではなく未踏の組み合わせを対象にしている。
5. **ADR-006が導入したUnicode注入モードのフック干渉耐性の理解が誤っていた。**
   ADR-006の一次資料では「1文字ずつ個別SendInputする方式（`per_key`）がデフォルト・
   互換性重視」であり、干渉に弱かったのは「全文字をバッチ化する方式」の方だった。
   per-VK送信（1キーずつ）はこの軸ではむしろ改善方向であり、設計書の懸念は逆だった。

## 今回実施した内容（2026-08-03）

`crates/awase-windows/src/tsf/warmup/unicode_literal_observer.rs::UnicodeLiteralObserverFsm::tick`
の判定確定点（GJI write bytesのbaseline比較の直後）に、`log_composition_probe`への
呼び出しを1行追加した。既存の`ProbeAction`判定ロジック（`current == self.baseline_bytes`
での分岐）は一切変更していない。

## 次の一歩（実機セッション向け）

1. 実機で`LINE`等のImmCrossプロファイル×GJIアプリを使い、`[unicode-obs-himc-check]`
   ログ（`ime_diagnostic.rs::log_composition_probe`のフォーマット）を収集する。
2. `docs/experiments.md`に事前登録した合格基準（`comp_str`が実際に非空になる率、
   `capture_composition_snapshot`の所要時間、`comp_read_str`の一致率）と照合する。
3. 使えると判明した場合のみ、Standard/ImmCrossプロファイル専用の確認戦略として
   `LiteralDetector`に段階的に組み込む設計を再度検討する（本ADRのNO-GO判定を覆す
   には、この観測結果と、warm時のconfirmレイテンシ実測の両方が要る）。

## 明示的にやってはいけないこと（次のセッションへの申し送り）

- `InjectionMode::Unicode`の廃止・`send_romaji_as_unicode`の削除
- per-VK confirmの`gji_active`ゲート緩和（`probe_fsm.rs:582`, `:136`）
- `RAW_TSF_LITERAL_DETECT_MS`系・`COMPOSITION_BYTES_THRESHOLD`の値変更（実測なし）
- `UnicodeLiteralObserverFsm`/`UnicodeColdWarmupFsm`の撤去
- ADR-081 Phase 1eをこの統一構想の完成待ちにすること（両者は独立に解決可能）

## 関連

- ADR-081（プロファイル別ドライバ分離）— GjiFsm同期義務の非対称発見の発端
- ADR-062（injection mode自己学習昇格）
- ADR-006（output mode、Unicode注入導入の経緯）
- ADR-023（adaptive output、「全アプリVK方式」却下の過去記録）
- `docs/known-bugs.md` BUG-45（per-VK confirmの構造的欠陥、未解決）
- `.claude/rules/experiment-logging.md`（過去の失敗実験の記録規約）
