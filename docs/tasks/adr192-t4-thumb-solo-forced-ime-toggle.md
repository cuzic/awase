# ADR-192 T4: 親指キー単体への強制ON/OFF新経路（決定3b）を実装する

状態: 完了（2026-09-22起票・実装完了、PR #249でdevelopマージ済み、2026-09-23。
/code-review指摘2件〈親指キー×UserOverrideの警告消失、Windows clippy pedantic〉を
修正済み）
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md)（rev8、
opus-adversarial-consult 7ラウンドで収束済み）決定3bは、`keys.ime_on`/`ime_off`/
`ime_toggle`に無変換/変換キー単体（bare、Shift等の修飾無し）を設定したユーザーのために、
`resolve_pending_thumb_as_single`（単独打鍵の確定点）に専用の新しい入力を追加する。

**このタスクはADR-192の中で最も再発リスクの高い領域を触る**
（[fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md)の
「キー選択」「defer/replayキュー」「IME belief」の複数の再発ファミリーに関係する）。
着手前に[ADR192-T0](adr192-t0-nicola-fsm-doc-fix.md)（`nicola_fsm.rs:858-867`のdoc矛盾
解消）を完了させること。

## 実装対象（設計はADR-192決定3bで確定済み、opus-adversarial-consult 7ラウンドで実コード
照合を経ている。以下は要点のみ、詳細・根拠は必ずADR本文を読むこと）

1. **新しい入力の型と配線**: `ThumbSoloSpecialHandling`に2つ目の`Option<ShadowImeAction>`
   フィールド（例: `forced_open_action`）を追加する。「このVKが`keys.ime_on`/`ime_off`/
   `ime_toggle`にbareで設定されているか」をPlatform側（config読み込み）でこのフィールドへ
   配線する。対象VKは`muhenkan_vk`/`henkan_vk`（無変換/変換限定）のみ。
2. **優先順位1.5**: `resolve_pending_thumb_as_single`の優先順位表で、新しい入力を
   `dedicated_fn_key`の**直後**・`*_solo_tap_ime_action`の**直前**に置く。既存チェック
   （`modifier_key`・`explicit_action_consumed`・`suppress_solo_output`）の**位置は
   動かさない**。
3. **`*_solo_tap_ime_action`との排他は「優先を逆にする」で実現する**（config検証でエラーに
   する案は`AppConfig::validate()`の設計上実装不能なため不採用——ADR本文参照）:
   新入力は、新入力自身の分岐**内で**`*_solo_tap_ime_action().is_some()`を自前確認し、
   `Some`なら発火しない（自己無効化）。`explicit_action_consumed`・`suppress_solo_output`
   も同様に新入力の分岐内で自前確認する（既存コードの並び順は変えない）。
   両方設定されている場合は、既存の警告メッセージの仕組み
   （`validate_thumb_key_in_ime_combos`と同様の形）で
   「`*_solo_tap_ime_action`が優先され、新しい強制ON/OFFの設定は無視される」ことを伝える
   （拒否ではなく警告）。
4. **解決タイミングはKeyUp**: `defers_solo_until_release`の対象にこの新しい入力を含め、
   通常のタップ（100ms超）でもKeyUp解決の`kp_stage_post_decision`経路を通す
   （`execute_from_loop`のタイマー解決に**落とさない**——ADR-186が実測した
   「タイマー解決はUnwarrantedで握り潰される」の再発防止、選択肢ではなく必須）。
   `defers_solo_until_release`は既存のADR-182決定1c由来の理由に加え、本決定の
   「belief書き込み経路〈KeyUp解決〉を通すため」という別の理由も持つことをdocに明記する。
5. **composing中も発火させる**: 「発火の有無」だけが選択事項（結果がDiscardedになるか
   Committedになるかはプリセット依存で選べない）。
6. **物理キー配送はDecision::Consumeに委ねる**: `transport.rs`への変更は不要
   （M19のような専用マーカーは立てない）。エンジン活性時のKeyDownは`PendingThumb`として
   FSMが`Decision::Consume`するため、これで配送は止まる（KeyDown/KeyUp双方）。
7. **T-16警告文の3分岐**（`src/config.rs::validate_thumb_key_in_ime_combos`）:
   (1)無変換/変換への設定→「単独タップ確定時に強制ON/OFFが発火します」に書き換え、
   (2)無変換/変換以外の親指キー→既存文面のまま、
   (3)無変換/変換だが`*_solo_tap_ime_action`も設定済み→「`*_solo_tap_ime_action`の設定が
   優先され、この設定は無視されます」の専用文面を追加。
8. **eisu救済との対称配線（必須、見落としやすい）**: この新経路は「user IME-ON経路」に
   当たるため、[ime-belief-architecture.md](../../.claude/rules/ime-belief-architecture.md)
   が定める対称性に従い、`state/eisu_recovery.rs::eisu_reset_on_ime_on`と対で配線する。
   `tests/architecture_guard.rs::user_ime_on_paths_are_paired_with_eisu_reset`に新経路を
   追加すること。
9. **`.claude/rules/fix-requires-evidence.md`表への追記**: 「IME actuation合流点」表では
   なく、「**キー選択（IME ON/OFFに送るVK）**」行が列挙する`resolve_pending_thumb_as_single`
   の優先順位（`dedicated_fn_key`/`*_solo_tap_ime_action`/`ModeKeyConfig`）に**4つ目として
   本決定の新しい入力を追記する**（「合流点表の7つ目」ではない——同表は既に3項目に整理
   済みで、本決定の新経路はそこを通らない。ADR本文参照）。

## 完了条件・テスト（[fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md)
の「キー選択」再発ファミリー対象、テストが必須）

`cargo test --lib`（ルート`awase`クレート、engineロジック）と`cargo test -p awase-windows`
（Linux実行可能なもの）に、少なくとも次を固定する単体テスト/journal replayテストを追加:

- (a) エンジン活性中の単独打鍵（KeyUp解決）でOFF/Toggleが発火すること
- (b) 通常のタップ（100ms超）でも`execute_from_loop`のタイマー解決に落ちずactuateされる
  こと（ADR-186 3/3 FAILの再発防止の直接確認）
- (c) チョード成立時は発火しないこと
- (d) composing中も発火すること（結果の`Disposition`がプリセット依存であることは実機検証で
  確認し単体テストでは求めない）
- (e) `modifier_key`／`explicit_action_consumed`／`suppress_solo_output`が立っている
  ときは発火しないこと
- (f) 専用Fnキーと同一VKに設定した場合は専用Fnキーが勝つこと
- (g) `tests/architecture_guard.rs::user_ime_on_paths_are_paired_with_eisu_reset`に
  新経路を追加し、`eisu_reset_on_ime_on`との対称配線を固定すること
- (h) 同一VKに`keys.ime_toggle`（bare）と`*_solo_tap_ime_action`の両方を設定した場合、
  `*_solo_tap_ime_action`が優先され新入力は発火しないこと（既存の警告メッセージで
  案内されること）
- (i) エンジン非活性時に無変換/変換の孤立したKeyUpがGJIへ漏れないこと（既存の「@」対策
  〈`key_pipeline.rs:1220-1226`〉が本決定の追加で壊れていないことの直接確認——
  **最重要**、これが壊れるとBUG-113/124の「@」ファミリーが再発する）
- (j) 新入力が発火する打鍵でKeyDown/KeyUpとも物理配送が`Decision::Consume`で止まり、
  追加のマーカーを立てなくてもGJIへ生キーが漏れないこと

実機検証（可能であれば）: 無変換単体を`keys.ime_off`に設定し、エンジン活性中の単独タップで
実際にIMEがOFFになること、composing中に押した場合の未確定文字列の行方（GJI/MS-IMEプリセット
での破棄/確定の違い）を確認する。

## 完了後にやること

- `.claude/rules/fix-requires-evidence.md`「キー選択」表の該当行を更新する（上記9）。
- 変更が[fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md)の対象
  ファイル（`nicola_fsm.rs`、`key_pipeline.rs`、`transport.rs`等）に触れるため、pre-push
  フックの自動チェックに引っかかった場合はテスト追加漏れがないか再確認する。

## 関連

- [ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md) 決定3b
- [ADR192-T0](adr192-t0-nicola-fsm-doc-fix.md)（着手前提）
- [ADR-186](../adr/186-gji-atok-mode-key-measured-matrix-and-belief-follow.md)
  （KeyUp解決必須の実測根拠）
- BUG-113/124（「@」ファミリー、`key_pipeline.rs:1220-1226`の対策）
