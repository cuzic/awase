---
id: FEATURE-115
title: |-
  打鍵列機能（ADR-115）実装状況・既知の限界
type: feature-status
---

# FEATURE-115: 打鍵列機能（ADR-115）実装状況・既知の限界

新機能のため「不具合」ではなく実装状況・既知の限界の記録（ADR-115決定9(c)の
証拠義務）。関連: [ADR-115](adr/115-yab-keystroke-sequence.md)、
[GitHub Issue #118](https://github.com/cuzic/awase/issues/118)。

**実装状況（2026-08-31時点）**: core（`awase`クレート、Linux上でテスト可能）は
実装完了。`CtrlChord`（`C`+`V`+hex）・セル内`+`区切り（`InlineSequence`）・
名前付きマクロ（`KeystrokeMacro`）・`resolve_keystroke_syntax`・
`flatten_actions`統一・`release_only`網羅match化・投機出力ガード（決定7）・
`LayoutEntry::scan_all`への配線まで完了。既定`keystroke_sequence = "off"`の
ためマージ後もデフォルト挙動は無変化。

**未実施**:
- Windows実機ソーク（`CtrlChord`がMS-IME/GJI双方で実際に確定を引き起こすか、
  TSF-nativeアプリでの順序・タイミング、押下中の物理修飾キーとの衝突）
- `awase-macos`/`awase-linux`の`send_keys`は`CtrlChord`/`Sequence`をログ
  出力のみのスタブとして実装（決定10、実送信は本ADRのスコープ外）

**実装後Opusレビュー1周目で発見・修正（2026-08-31、修正コミット `866f38657`）**:
- **M1**: `crates/awase-windows/src/platform.rs`の`send_keys`が
  Unicode injection mode + GJI long-cold時に`unicode_cold_defer`で
  `Char`のみ遅延バッファへ積み、`CtrlChord`/`Key`/`SpecialKey`は
  即時送信していたため、`'（'+CV4D+'）'+CV4D+左`のような混在バッチで
  実行順序が入れ替わる欠陥があった。1周目の修正はバッチ内が`Char`/`Romaji`
  のみの場合に限り defer を有効化する条件（`all_defer_safe`）を追加したが、
  これは過剰に広く、後述の2周目レビューで既存パターンを巻き込む新規回帰と
  判明したため2周目で訂正した。
- **M2**: `awase-settings`（GUIプレビュー、`load_yab_layout`）が
  `resolve_keystroke_syntax`を呼んでおらず、キルスイッチ`off`でも
  GUIが`Ctrl+4D`/`@name`等の新構文表示のまま（実エンジンは`Literal`に
  復元済み）というプレビューと実挙動の食い違いがあった。1周目の修正は
  `load_yab_layout`の戻り値（`self.layout`として保存され`layout_write_to_path`
  でそのまま`.yab`へ書き戻される対象）に`resolve_keystroke_syntax`を通したが、
  これが致命的なデータ破壊回帰を生んだため2周目で全面的に差し戻した
  （詳細は次項）。
- **M3**: 決定7の投機ガード拒否経路2箇所（`confirm_policy.rs::idle_speculative`、
  `nicola_fsm.rs::on_timeout_speculative`）と決定5の`flatten_actions`の
  end-to-end確認（`Sequence`セルが実際に複数`KeyAction`として出力される
  こと）にテストが無かった。3件追加（`src/engine/confirm_policy.rs`・
  `src/engine/nicola_fsm.rs`・`src/engine/tests.rs`）。この項目は2周目でも
  訂正なし。

**実装後Opusレビュー2周目（1周目の修正自体を対象、3観点が独立に同一の
2件を検出、2026-08-31）**:
- **M1再訂正**: 1周目の`all_defer_safe`（バッチ全体がChar/Romajiのみか）は
  条件が広すぎた。実際に安全性を左右するのは「Char/Romajiより後に
  非defer対象（CtrlChord/Key/KeyUp/SpecialKey）が続くかどうか」のみ——
  `retract_and_replace`が出す既存パターン`[Backspace, Char]`
  （ADR-115以前から存在、非defer対象が先）は本来 defer 安全なのに、
  1周目の条件はこれも一律で defer 無効化していた。GJI long-cold + Unicode
  injection modeという条件が重なると、この既存パターンで warmup 保護
  （BUG-02系の`bあ`部分リテラル化再発防止）が効かなくなる新規回帰で、
  `keystroke_sequence = off`（既定）でも発生しうる。修正: 「Char/Romajiを
  見た後に非defer対象が続かない」ことだけを判定する走査に置き換えた
  （`crates/awase-windows/src/platform.rs::send_keys`）。
- **M2revert**: `load_yab_layout`が返す`YabLayout`は GUI 編集の作業コピーで
  あると同時に保存時の書き戻し対象そのもの。1周目の`resolve_keystroke_syntax`
  呼び出しはこれを解決済みの値で上書きしてしまい、`InlineSequence`/
  `MacroRef`が`Literal`/`Sequence`に変わった状態で`.yab`へ`serialize()`
  される（`Sequence`は`serialize()`が`無`を返す設計のため、キルスイッチ
  `on`でレイアウトを開いて保存すると打鍵列セルが丸ごと`無`に、`off`でも
  `raw`の一部だけが残る形で破壊される）。「未編集セルの無損失往復は
  保証される」という決定9(a)の主張と直接矛盾する回帰だった。修正:
  `load_yab_layout`から`resolve_keystroke_syntax`呼び出しを削除し、
  `self.layout`は常に未解決の生パース結果を保持するよう2周目で全面的に
  差し戻した。GUIプレビューが`off`時の実際の送信内容と食い違う点
  （元のM2指摘）は非破壊的な既知の制約として残し、下記へ記載する。
- **新規発見（Linux/macOS配線漏れ）**: `crates/awase-linux/src/main.rs`・
  `crates/awase-macos/src/main.rs`の起動時レイアウト読み込みが
  `resolve_kana()`のみで`resolve_keystroke_syntax`を呼んでおらず、
  これら2プラットフォームではキルスイッチ`off`（既定）でも新構文セルが
  未解決のままエンジンへ到達していた（`InlineSequence`/`MacroRef`は
  `Suppress`化、`+`を含む既存セルもパース時点で無条件に`InlineSequence`
  化されるため巻き込まれうる）。両ファイルに`resolve_keystroke_syntax`
  呼び出しを追加して解消。

**既知の限界（決定として受け入れ済み、バグではない）**:
- GUIでの新構文authoringは非対象。`awase-settings`のテキスト欄で
  `CtrlChord`/`InlineSequence`/`MacroRef`セルを編集すると、保存時に
  打鍵列としての意味を失う（決定9(a)）。未編集セルの無損失往復は保証される。
- セル内`+`区切りで`Romaji`要素を含む列（例: `ｋａ+CV4D`）は、単体`ｋａ`
  セルと違いn-gram文脈・同時打鍵のkana依存閾値調整から外れる
  （`lookup_face`のkana抽出が`Sequence`に対して常に`None`を返すため、
  決定7）。実害は次のキーのタイブレーク精度が僅かに落ちる程度。
- `KeyAction::romaji()`（`src/types.rs`）の非網羅性は対応しない方針を
  確定（決定6、`OutputEntry.romaji`はproductionコードで一度も読まれない
  dead codeのため実害なし。将来的には[ADR-045](adr/045-dead-field-detection-policy.md)
  の対象として整理しうる）。
- 通常面が`CtrlChord`単体セルの場合、投機出力ガード（決定7）は`Sequence`
  のみ拒否するため投機出力される。閾値内に親指キーが来ると
  `retract_and_replace`がBACKSPACE 1発で取り消そうとするが、IME確定は
  BSで戻らないため確定済み文字を1つ余分に消す（既存の`Special(Enter)`
  セルと同型のリスクで退行ではないが、`CtrlChord`にも当てはまることを
  実装後レビューで確認）。
- `KeystrokeMacro.steps`の各要素は`parse_cell`ではなく`YabValue::parse`
  を通るため、ステップ文字列に`+`を書くと無警告で`Literal`（先頭1文字
  のみ）になる。マクロステップ内で打鍵列合成をしたい場合は`@`で別マクロ
  を参照する形にする必要がある（現状は無警告、将来的に警告追加を検討）。
- `awase-settings`のGUIプレビュー（レイアウトタブのグリッド表示）は
  `resolve_keystroke_syntax`を通さない生の`YabLayout`をそのまま表示する
  ため、キルスイッチ`off`時でも`CtrlChord`/`InlineSequence`/`MacroRef`
  セルは新構文のまま（紫色）表示され、実際に送信される`Literal`内容とは
  一致しない（M2の元指摘）。GUIから保存する対象がまさにこの生レイアウト
  であるため、プレビューを解決済みにすると保存が壊れる（M2revert参照）。
  非破壊を優先しこの表示上の不一致は許容する——正確なOff時挙動を
  確認したい場合は`resolve_keystroke_syntax`のロジック
  （`src/yab/mod.rs`）を読むか、実機で`keystroke_sequence = "off"`の
  ままテキストエディタで.yabを確認すること。

**関連する既存の再発ファミリーとの接点**: `OutputHistory`のKeyUp整合性索引
（BUG-101/ADR-112）と同じ危険領域に触れるため、打鍵列のマクロステップ/
セル内`+`区切り双方から生の`KeyAction::Key`/`KeyUp`を構造的に排除している
（決定6・決定2c、r5レビューCritical C1の回帰テスト`resolve_on_rejects_vk_inside_inline_sequence_with_warning`で固定）。
---
