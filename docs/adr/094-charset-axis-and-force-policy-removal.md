# ADR-094: charset 軸の追跡撤去と `conv_mode_policy`（force ポリシー）の全撤去

## ステータス

実装済み（2026-08-17）。Windows 実機での動作確認は未実施。

## コンテキスト

ユーザーから以下の要望があった:

1. トレイの「状態をリセット」から「ローマ字」表記を削除する。
2. 設定画面の `conv_mode_policy`（observe/force、IME 変換モードを能動的に
   追跡・強制するかどうかの設定）を撤去する。
3. ひらがな/カタカナ等の**かな形状軸（charset 軸）自体を撤去する**。
4. 英数かどうか（eisu vs かな）の二値境界は、親指シフトエンジンの活性判断
   （`state/eisu_recovery.rs`、BUG-57 対応の `ObservedEisu` 救済）に必要な
   ため残す。

[ADR-091](091-idempotent-charset-axis-gji-recommended-msime-self-responsibility.md)
決定3 §D3.1 は既に「charset 軸について awase が特定の状態を予測して belief
化することはしない」という原則を確立していた。しかし §D3.3 は
`state/conv_classify.rs::has_katakana`（`ConvModeMgr` が保持する**既存の
観測**を読むだけの判定）についてはこの原則と衝突しないとして明示的に容認して
おり、`ConvModeMgr` 自体は raw conv 値からの `Charset`（5値: Hiragana/
ZenkakuKatakana/HankakuKatakana/ZenkakuAlpha/HankakuAlpha）×`romaji`（2値）
の組を追跡し続けていた。加えて `conv_mode_policy = force` は「乖離を許容
しつつ定期的に正す」ための、charset 軸そのものを対象とする能動的な強制
書き込み機構（[ADR-085](085-conv-mode-force-policy.md)/
[ADR-086](086-force-write-trigger-and-target-identity.md) Phase 2/3）だった。

本 ADR は、ユーザーの要望3を「ADR-091 §D3.3 が容認していた残存 charset 観測
（`ConvModeMgr`/`has_katakana`/BUG-50 のカタカナ復旧判定）も含めて完全に撤去
する」ものとして実施する。これは ADR-091 決定3 の**延長・徹底**であり、
ADR-091 の他の決定（romaji/JIS かな非サポート方針、専用Fnキー方式等）を
覆すものではない。

### BUG-50 との関係（撤去前の事前検証）

`state/conv_mode.rs::should_reset_katakana_on_ime_on_combo`（BUG-50 対応、
2026-08-05）は「belief が誤って Off でも、実際に観測されたカタカナがあれば
IME-ON コンボでリセットする」という katakana 観測の消費先の一つだった。この
判定を撤去してよいかどうかは、BUG-50 の未解決だった「原因2」（そもそもなぜ
カタカナに入ったか）と無関係に切り離せるかを検討する必要があった。

`docs/known-bugs.md` BUG-50 の追補（2026-08-17）で記録した通り、コミット
時系列の検証により、BUG-50 の元インシデント（2026-08-05）は BUG-52
（物理 IME-ON キーが IME 起動済み状態で `VK_DBE_KATAKANA` を生成する OS 側の
既知の挙動、`21a6b6b6`→`bdf4a139`→`9a02ce6b`→`7fcb89aa` の順に対応）が真因
だった可能性が高いと判明した。BUG-52 の根本原因は既に修正済みであり、
`should_reset_katakana_on_ime_on_combo` は対症療法だったと結論づけられる。
この検証を経て、katakana 観測消費先を全撤去する決定に至った。

## 決定

以下を撤去する。

### 1. `Charset` 型と `ConvMode` の簡略化（`src/engine/conv.rs`）

`ConvMode { charset: Charset, romaji: bool }` を `ConvMode { eisu: bool,
romaji: bool }` に置き換える。`Charset` enum・`is_katakana()`・
`imm_conv_target()`（カタカナ復元用の conv 値算出）・`to_conv_bits()`
（charset→conv 変換）を削除する。`is_eisu()`/`is_eisu_evidence()`
（BUG-57 対応）は `eisu` フィールドを直接読むだけになり、シグネチャ・
呼び出し元は変更しない。

`classify_idle()` の「ROMAN=0 かつ KATAKANA」を romaji-capable 扱いする
特殊分岐を削除し、charset に関わらず ROMAN=0・NATIVE=1 を一様に
「JIS かな観測」として扱う（is_roman_reliable の既存分岐へ統合）。これは
挙動変更であり、ROMAN=0 カタカナは以後 JIS かな入力と同じ扱いになる
（ADR-091 決定2 の「JIS かな非サポート」方針と整合）。

### 2. `ConvModeMgr` の簡略化（`state/conv_mode.rs`）

`katakana_candidate`（BUG-19 のカタカナ一発誤読デバウンス）・
`suppress_zenkata_until_ms`（HanKata→ZenKata ダウングレード抑制）・
`desired_mode`/`policy`/`set_desired_mode`/`set_policy`/`on_focus_changed`
を削除する。`update_from_conv(conv: u32) -> bool` は raw conv 値の変化を
検出して `mode: Cell<Option<ConvMode>>` を更新するだけの薄いラッパーになる
（`now_ms` 引数も不要になり削除）。

`should_reset_katakana_on_ime_on_combo`（BUG-50）を削除する。
`key_pipeline.rs::kp_reset_to_hiragana_romaji_capsoff` の起動条件から
`was_open_before`/`observed_katakana` の判定を外し、IME-ON コンボ
（既定 `Ctrl+変換`）押下時は常にひらがな＋ローマ字＋CapsLock OFF へ
リセットする（既に ADR-050 時点で `was_open_before=true` 時は無条件の
破壊的リセットだったため、新しい破壊的動作のクラスは増えない）。

`kp_shift_conv_guard_key_up` のかな入力復元（半角英数トグル解除時）は、
切替前の charset に関わらず常にローマ字ひらがなへ復元する
（旧: カタカナだった場合は KATAKANA ビット込みで復元）。

### 3. `conv_mode_policy`（force ポリシー）と付随する force-write 機構の全撤去

`config.rs::ConvModePolicy`（Observe/Force）と `GeneralConfig::conv_mode_policy`
を削除する。設定画面（`awase-settings`）の ComboBox を削除する。

これに伴い、以下の force-write 機構が**巻き添えで**全撤去される
（`conv_mode_policy` という名前だけを見ると conv 軸専用に見えるが、実際には
open/close 軸の force-write 機構もこのポリシー1個で分岐していたため）:

- conv 軸（[ADR-086](086-force-write-trigger-and-target-identity.md)
  Phase 2）: `Output::force_pending`（武装フラグ）・
  `consume_force_pending_and_actuate`・`ConvModeTarget::Desired`・
  `ConvMutationReason::ForcePolicy`。
- open/close 軸（ADR-086 Phase 3）: `Runtime::force_open_pending`・
  `arm_force_open_pending`・`consume_force_open_pending`・
  `should_attempt_force_open`/`next_force_open_pending_after_outcome`
  （純粋関数とそのテスト）・`OpenApplyReason::ForcePolicyResend`。
- `Output::is_force_policy()`（両軸共通の唯一の判定点）と、それに依存して
  いた `Runtime::reschedule_ime_refresh`/`apply_force_on_for_imm_broken`
  の force policy 分岐。

**`apply_force_on_for_imm_broken`（IMM ブリッジ非対応アプリ向けの周期
force-ON）は撤去しない。** これは `conv_mode_policy = observe`（デフォルト、
ほぼ全ユーザー）でも動いていた既存機構であり、charset/force-policy 撤去とは
独立している。ただし、force policy 分岐の削除により、この関数は今後
**常に**周期リフレッシュ経路を使う（分岐が無くなったため条件分岐ではなく
唯一の経路になる）。これは [ADR-086](086-force-write-trigger-and-target-identity.md)
INV-15（「生の周期タイマートリガー禁止」）に対する**既知の逸脱として元々
存在していた**ものであり、本 ADR で新たに生んだものではない（ADR-086
Phase 3 は「force policy 時のみ INV-15 準拠の入力意図駆動トリガーへ移行する」
という限定的な対応であり、observe policy 側は最初から周期トリガーのままだった）。

### 4. トレイの「IME 状態」サブメニュー撤去

`ImeHiragana`/`ImeFullKatakana`/`ImeFullAlpha`/`ImeHalfAlpha`/
`ImeHalfKatakana`/`ImeDirect` の6コマンドと対応する IDM 定数・サブメニュー
構築コードを削除する（`tray.rs`/`message_handlers.rs`）。

`ResetState`（"状態をリセット"）は残すが、ラベルから「ローマ字」を削除し
（"状態をリセット (Engine ON/Caps OFF/ひらがな)"）、**書き込みマスクからも
`IME_CMODE_ROMAN` を外す**（ユーザーの明示決定。romaji/JIS かなの区別自体は
決定3 §D3.4 により別軸として残るが、このリセット操作の対象からは外す）。

## 保持するもの（変更しない）

- **eisu/かな の二値境界**（`ConvMode::eisu`、`is_eisu()`/`is_eisu_evidence()`）
  は `state/eisu_recovery.rs`（BUG-57 対応）が引き続き使う。親指シフト
  エンジンの活性判断に必要なため、ADR-091 決定3 §D3.4 のとおり本 ADR の
  対象外。
- `apply_force_on_for_imm_broken`（前述）。
- `ConvModeAuthority`/`allows_conv_mutation()`（awase エンジン ON/OFF に
  よる conv mutation 許可判定、force policy とは無関係）。
- `ActuationTarget`/[ADR-086](086-force-write-trigger-and-target-identity.md)
  INV-14（書き込みターゲット同一性）— `actuate_conv_mode` が
  `ConvModeTarget::HalfWidthAlnum`（shift-conv-guard 用）を書き込む経路で
  今も使われている。

## 関連 ADR への影響

- **[ADR-085](085-conv-mode-force-policy.md)（`conv_mode_policy = force`
  の定義）**: 全撤去。ステータスを「廃止（本 ADR で撤去）」に更新する。
- **[ADR-086](086-force-write-trigger-and-target-identity.md)（force-write
  の単一規律）**: Phase 2（conv 軸）・Phase 3（open/close 軸）は全撤去。
  Phase 0〜1（INV-14 ターゲット同一性）は `actuate_conv_mode` で存続する
  ため完全な廃止ではない。ステータスに撤去範囲を追記する。
- **[ADR-087](087-open-belief-actuation-warrant-separation.md)**: 影響なし
  （open/close belief と actuation warrant の分離は charset 軸と独立）。
- **[ADR-088](088-ime-axis-capability-and-charset-owner.md)**: トラック A
  （`CharsetOwner` 設計）は「実装・実機ソークは未着手、コード変更は一切
  行っていない」ドラフトのまま。本 ADR により charset 軸自体を追跡しない
  方針が確定したため、トラック A の設計は**提案として撤回**（実装されて
  いないコードの削除は発生しない）。
- **[ADR-091](091-idempotent-charset-axis-gji-recommended-msime-self-responsibility.md)
  決定3**: §D3.1 の原則を実装面でも徹底する形で延長する。§D3.3 が明示的に
  容認していた `has_katakana`/`ConvModeMgr` の既存観測利用は、本 ADR で
  撤去対象に含めた（BUG-50 の原因2が BUG-52 と判明したことによる、
  §D3.3 時点からの状況変化）。§D3.4（eisu/かな二値境界は別軸）は変更しない。
- **[ADR-092](092-external-key-semantics-absorption-and-thumb-key-restructure.md)**:
  影響なし（無変換/変換キーの宣言吸収は charset 軸と独立）。

## 既知の限界・未検証事項

- Windows 実機での動作確認は未実施。特に以下を確認する必要がある:
  - IME-ON コンボが常にひらがなへリセットするようになったことで、
    意図的に半角英数/全角カタカナ等へ切り替えた直後の IME-ON コンボ操作が
    想定外のリセットを起こさないか。
  - トレイの「状態をリセット」が ROMAN ビットを含まなくなったことで、
    JIS かな入力に固定された状態からの復旧手段が失われていないか
    （awase 自身のローマ字入力は engine 側の romaji 出力とは独立に動作
    するため、理論上は問題ないはずだが実機未確認）。
  - `apply_force_on_for_imm_broken` が常時周期経路に乗るようになったことで、
    Chrome 等 Imm32Unavailable アプリでの force-ON 頻度・レイテンシに
    体感できる変化がないか。
