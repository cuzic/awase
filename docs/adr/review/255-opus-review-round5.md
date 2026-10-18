---
id: ADR-255-companion-255-opus-review-round5
title: |-
  ADR-255（IME OFF のときだけ無変換/変換を Space にする）Opus敵対的レビュー round5
type: companion-doc
related_adr:
  - "ADR-255"
---

# ADR-255 敵対的レビュー round5(Opus)

対象: `docs/adr/255-ime-off-thumb-key-space.md`(ワークツリー keymap-ime、HEAD `34b30c34`)。行番号はこの HEAD のもの。

## 総評

**新規の Blocker/Must なし。収束した。**

r4 の Must 3 件(R4-M1・M2・M3)はすべて意図どおり反映されている。残りは記述の正確さと docs 規約の Should だけ。

**判断**: 実装は決定3b のゲート(報告者の回答)で保留したまま、**この ADR を docs として develop にマージしてよい**。ただしマージ前に、ステータスの文言と review ファイルの frontmatter を直すこと(下の S4・S5)。

---

## r4 対応の確認

| ID | 判定 | 根拠 |
| --- | --- | --- |
| R4-M1 | 満たす | 本番の定数を足すこと、出典、突き合わせのテスト、判定表、「報告の構成に合う」の取り下げ。表はテストの定数(`role.rs:410-505`)と一致する。MS-IME の変換が `Reconvert`、ATOK の両キーが `IMEOn`、KOTOERI は行なし |
| R4-M2 | 満たす | InsertSpace 系を機能ありに数え、案内は「行を消す」。(b) を採らない理由も書かれている |
| R4-M3 | 満たす | 「不具合を報告」で集める。言語バーで手で確かめる代替もある |
| R4-S1 | 満たす | 未解決の疑問9 |
| R4-S2 | 満たす | 未知のオーバーレイ・未知の `session_keymap` は `Unknown` |
| R4-S3 | 満たす。「実装時に確かめる」は今閉じられる | S2 |
| R4-S4 | 満たす | `enrich_thumb_key_role` と同じ形 |
| R4-S5 | 満たす。前提の記述に事実の誤り | S1 |
| R4-S6 | 満たす | |

---

## Should(残り)

### S1. `check_active_transition` は元から `SetOpen` を出さない

- 決定4 は「前置するのは `EngineStateChanged` と保留出力の解放だけで、`SetOpen` は外す」と書き、`SetOpen` を取り除く操作が要るように読める。実際は `check_active_transition` が `transition_activation(new_state, false)` を呼び(`src/engine/engine.rs:393-395`、ADR-213 決定3 P2b「観測・RefreshState 由来の遷移は SetOpen を出さない」)、`SetOpen` は元から入らない。
- 前置するのは `check_active_transition` の戻り値をそのまま(flush・`release_pending_and_reinject`・`EngineStateChanged`)でよい。文言を「`check_active_transition` は `emit_set_open=false` で呼ばれるので `SetOpen` は含まれない(ADR-213 P2b)。テストで、前置した effects に `SetOpen` が無いことを固定する」に直すこと。結論は変わらない。

### S2. R4-S3 は既存のコードで満たされている

- `KeyStates::of`(`crates/awase-gji-config/src/role.rs:244-257`)の doc は「`vk_name` を生むキー名の**無修飾の行だけ**を、表の順に読む(後勝ち)」で、`mozc_key_vk_names(&row.key)` で照合している。`Shift Henkan` は変換の行に数えない。
- `Effect::of`(:229-237)は `DirectInput` の行を `Open`(IMEOn・絶対モード指定)か `Other`(Reconvert・InsertSpace 等)に分ける。決定2-4 の「行があれば機能あり」は `states[DirectInput].is_some()` でそのまま書ける。ADR の「実装時に確かめる」を、この 2 点の引用に置き換えて閉じてよい(テストで固定する、は残す)。

### S3. 三値の名前

- `KeyOpenEffect::Opens` は Reconvert・InsertSpace 等の「開かないが機能がある」行も含むので、名前と中身が合わない。`KeyDirectInputEffect = HasFunction | NoFunction | Unknown` のように、「直接入力でこのキーに IME の機能があるか」と読める名前にする。仮称なので実装時でもよいが、ADR の表と単体テストの名前を揃えること。

### S4. review ファイルの frontmatter(docs 規約)

- `docs/adr/review/` の既存の review(例 `178-opus-review-round1.md`)は先頭に `id: ADR-178-companion-178-opus-review-round1` 等の frontmatter を持つ(`docs-frontmatter-convention.md` の「補助資料は `type: companion-doc`」)。`255-opus-review-round{1..4}.md`(とこの r5)には frontmatter が無い。マージ前に 178 の形に揃えること。index.md の補助資料の行(:439)も round5 までに更新する。
- review ファイルの本文は、対象ファイル名を当時の名前(`255-keymap-ime-state-condition.md`)で書いている。記録としてはそのままでよいが、round1 の冒頭に「のちに `255-ime-off-thumb-key-space.md` へ改名」の一行があると辿りやすい。

### S5. マージ時のステータス

- frontmatter の `status` と本文のステータス節は「r5 待ち。実装は未着手」になっている。マージ時は「提案(Opus r1〜r5 で収束、2026-10-10)。実装は決定3b のゲートで保留(報告者の回答待ち)」に更新し、index.md の行(:264)も同じ短縮表記にすること。
- 報告者が回答しない場合の扱いを一行書いておく(例: 「回答が無い間は実装しない。保留のまま、別の報告で需要が出たら決定3b から再開する」)。これが無いと、後のセッションが「起草済み・未実装」を実装待ちと誤読しやすい(memory の「ADR 実装状況は git log で裏取り」型の誤読)。

### S6. ゲートの対象を分ける

- 検証計画0 のうち、(ii) 入力先の分類の確認・(iv) 仮説 a〜c の切り分けは、報告者の回答と独立に CI で進められる。そのうえ (iv) の結果は報告者への返答(決定3b(b) の「GJI の直接入力状態の制約」)の材料になる。決定3b に「ゲートが止めるのは実装(コード変更)だけ。計画0(ii)(iv) の CI スパイクは回答を待たずに進めてよい」と書くと、次のセッションが何から着手できるか迷わない。

---

## 確認に使ったコマンド(HEAD `34b30c34`)

- `git diff 01a98ccb 34b30c34 -- docs/adr/255-ime-off-thumb-key-space.md`
- `sed -n 339,398p src/engine/engine.rs`(`check_active_transition` → `transition_activation(new_state, false)`)
- `sed -n 229,258p crates/awase-gji-config/src/role.rs`(`Effect::of`・`KeyStates::of` の無修飾の照合)
- `head -3 docs/adr/review/178-opus-review-round1.md`(review ファイルの frontmatter の前例)
- `grep -n 255 docs/adr/index.md`(:264 本体の行、:439 補助資料の行)
