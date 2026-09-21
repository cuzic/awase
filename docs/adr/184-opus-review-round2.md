# ADR-184 opus-adversarial-consult round2

対象: `docs/adr/184-gji-atok-muhenkan-toggle-awase-owned-eisu-hiragana.md`（v2）
前回: `docs/adr/184-opus-review-round1.md`（Blocker 5 / Must-fix 10 / Nits 4）
レビュー日: 2026-09-19 / ブランチ `feat/adr178-mode-key-actuation-and-tsfnative-rescue-teardown`（HEAD `c8bc1adc`）
方式: 読み取りのみ。v2 の主張・引用はすべて実コードと兄弟ADRで裏取りした。

---

## 総評

**設計の骨格（B4の3点セット、B5の挿入位置訂正）は正しい方向へ直った。**
特に「`transport.rs` では効かない、正しい位置は `resolve_pending_thumb_as_single`」
への訂正と、`note_explicit_ime_action` を落とすと自分の書き込みで
DirectInput に舞い戻る、という因果の明記は round1 の核心に正しく応えている。

**一方で、v2 は「撤回した」と status に書いた主張を本文と frontmatter から
削除していない**ため、ADR 単体を読むと round1 で誤りと確定した診断が
そのまま生きているように読める。本リポジトリの
`.claude/rules/docs-frontmatter-convention.md` は「索引だけ読めば概要が
分かる」ことを前提に frontmatter を正本としているので、これは体裁の問題
ではなく**次セッションを誤った診断に誘導する実害**である。

さらに round2 で新たに 3 件の Blocker を検出した。いずれも v2 の変更
（B2 の解消ロジック、ADR-182 との関係節）**そのものから派生した**もので、
v1 には無かった論点である:

- **R2**: 「ATOKプリセットのテーブルは Mozc ソースに静的」という B2 解消の
  論拠を採用すると、本ADRが「別問題」として棚上げした atok.tsv との矛盾が
  棚上げできなくなる（ゲートの正当性が、本ADRが誤りと示唆した表に依存する）。
- **R3**: ADR-182 との衝突は「実装順序」ではなく**正常系の定義そのものの
  非互換**であり、ADR-182 の対照テストが本ADR実装後に落ちる。
- **R4**: 本ADRが対象とする症状は、既定設定では**構造的に発生しない**
  （`muhenkan_solo_tap_always_suppress` の既定は `true`）。前提となる
  設定が本文に書かれていない。

重大度の内訳: Blocker 4件（うち新規3件）/ Must-fix 9件（うち継続6件）/ Nits 4件。

---

## round1 指摘の対応状況（一覧）

| # | round1指摘 | v2での対応 | 判定 |
|---|---|---|---|
| B1 | 600ms後の物理キー押下がADR-181現象と未区別 | 診断の根拠としては撤回、「関係」節・疑問9を新設 | **部分**（本文・summaryに旧主張が残存 → R1） |
| B2 | ATOK前提がconfig1.db記録と矛盾 | ユーザー確認で大幅解消、実装時確認は残す | **部分**（解消ロジックが新たな矛盾を生む → R2） |
| B3 | doc/実装不一致は誤読 | 疑問1を取り消し線で撤回 | **部分**（本文手順3・summaryに旧主張が残存 → R1） |
| B4 | 決定4の自己矛盾 | 3点セットを「提案する設計」3に明記 | **対応済**（細部にM11/M12） |
| B5 | Suppress位置がtransport.rsでは効かない | `resolve_pending_thumb_as_single` へ訂正 | **対応済**（衝突整理は R3） |
| M1 | ADR-181/182 未参照 | related_adr追加＋関係節新設 | **対応済**（内容の妥当性は R3） |
| M2 | 優先順位表での位置が未定義 | 「優先順位3/4に入る前」とだけ記述 | **不十分**（M13） |
| M3 | `HalfWidthAlnumState` の転用が型上成立しない | 未反映（L211 が依然 `toggle_held` 流用を示唆） | **未対応**（M14） |
| M4 | open軸依存・commit規律 | 未反映（申告どおり） | **未対応**（M15） |
| M5 | ATOK下で注入経路が未検証／`VK_DBE_ALPHANUMERIC`の前科 | 未反映、L141 が依然「実機検証済み」 | **未対応**（R2に統合、M16） |
| M6 | 無変換3連打engine_off | 未反映（申告どおり） | **未対応**（M17） |
| M7 | InputRelay / `conv_mutation_allowed` | 未反映（申告どおり） | **未対応**（M18） |
| M8 | 証拠の出所・ビルドハッシュ | 未反映 | **未対応**（M19） |
| M9 | `InputModeApplyStrategy` 新variant | 「提案する設計」3・疑問4に明記 | **対応済**（M12に追補） |
| M10 | 回帰防止の選択 | 疑問7にADR-182統合の観点を追記 | **対応済**（方針は未決のまま） |
| N1 | ADR本文のwikilink | **未修正**（L7・L67の2箇所に残存） | **未対応**（N1再掲） |
| N2 | タイプミス | 未修正 | **未対応** |
| N3 | BUG-015同型の根拠 | 未修正（L122） | **未対応** |
| N4 | statusに実装前提条件 | ADR-182順序調整を前提条件として明記 | **対応済** |

---

## Blocker

### R1. 撤回した主張が frontmatter `summary` と本文「現状の機序」に残っており、ADR 単体では誤った診断が正本として読める

**該当**: frontmatter `summary`（L6-30）、本文「現状の機序」手順3・5〜7（L104-124）、L110 の参照

`status`（L31-52）には「(2) B3『docコメントと実装の不一致』は……撤回」
「旧版が……断定した事象は……この因果関係を診断の根拠に使うのは撤回する」
と正しく書かれている。**しかし本文と summary は v1 のまま一字も変わって
いない**:

- `summary` L17-20: 「この経路は（**コード内docコメントの記述に反して**）
  実際には `apply_ime_open(false)`＝実IMEを強制的に閉じる SendInput
  (VK_IME_OFF) を送信していることが実機ログで判明した」
  → round1 B3 で誤読と確定（`conv_classify.rs:44` の doc 第一文が
  「engine OFF + DirectInput」と明記している）。
- `summary` L20-24: 「その約600ms後、物理「IME ON」キー(VK_DBE_HIRAGANA)の
  **押下が**OFF→ON遷移を検出し……」
  → round1 B1 で ADR-181 の現象と未区別と確定、v2 で根拠から撤回。
- 本文 L107-111 は doc 不一致の主張を丸ごと維持したうえで
  **「本 ADR のスコープでは深掘りしない、未解決の疑問参照」**と書いているが、
  参照先の「未解決の疑問」1（L219-221）は取り消し線で撤回済み。
  **ダングリング参照**になっている。

**なぜ Blocker か**: `.claude/rules/docs-frontmatter-convention.md` は
「frontmatter に完全な情報を retain したまま index を機械的に短縮」
「全文が欲しい場合は index.md を編集せず、対象ADRファイルの frontmatter か
本文を開く」と定めている。つまり **frontmatter `summary` は「本文を開かずに
gist を得る」正本**である。そこに撤回済みの断定が残っていると、次の
セッションが `docs/adr/index.md` 経由で本ADRに当たったとき、
「DirectInput の doc コメントと実装は食い違っている」「600ms後に物理キーが
押された」を確定事実として再導入する。これは `experiment-logging.md` が
防ごうとしている「なぜ前回それを捨てたのかが辿れず、同じ結論を再発見する」
パターンそのものである（同ルールは `VK_DBE_ALPHANUMERIC` で実際に複数回
起きたと記録している）。

**要求**:
1. `summary` の L17-24 を書き換える。「idle-conv-check が `ObservedEisu` を
   検出して `EngineSync::DirectInput` を発火し、open 軸へ `false` を書く
   （ADR-182 が別ADR/BUGとして分離済みの論点）。その後 belief が
   `UserImeOnEisuReset` で巻き戻る経路が観測されたが、その巻き戻しの
   起点が物理キーか ADR-181 の GJI 自己主張かは未確定」程度に温度を下げる。
2. 本文「現状の機序」手順3 の doc 不一致の段落を削除し、手順5 に
   「この F2 の由来（物理 / ADR-181 の自己主張）は未確定」と明記する。
3. L110 のダングリング参照を解消する。

---

### R2. B2 の解消論拠（「ATOKテーブルはMozcソースに静的」）を採ると、L79-82 で「別問題」として棚上げした atok.tsv との矛盾が棚上げできなくなる

**該当**: `status` L35-40（B2解消）、本文 L79-82（棚上げ）、「提案する設計」1（ゲート）

**v2 の新しい論拠**（status L35-38）:
> ATOKプリセットのキーマップテーブル自体が config1.db ではなく
> Mozc/GJI のソースコードに静的に埋め込まれている

これは技術的に正しい（`awase-gji-config` が `session_keymap` の値だけを
読み、テーブル本体は `gji_charset_autodetect.rs:296-300` に
「ATOK → Henkan/Muhenkan は `Toggle`」という**結論だけ**をハードコード
している。元データは `google/mozc` の `src/data/keymap/atok.tsv`）。

**しかしこの論拠を採ると、次の3つが同時には成り立たない**:

| # | 主張 | 出典 |
|---|---|---|
| (a) | ATOKプリセットのテーブルは Mozc ソースに静的＝可搬で検証可能 | v2 status（ユーザー確認） |
| (b) | atok.tsv の Muhenkan 行は `DirectInput→IMEOn` / `Precomposition→CancelAndIMEOff`（**open軸**） | BUG-115 L112-114、ADR-135「調査の展開」3・「スコープ確定」節（2026-09-05に実データ取得） |
| (c) | 実機の無変換は「IME OFF → **不変**」「IME ON ひらがな ⇔ IME ON 半角英数」（**conv軸**、open軸に一切触れない） | 本ADR L73-76（ユーザー実測 2026-09-19） |

(b) の `DirectInput Muhenkan → IMEOn` は「IME OFF から押すと IME が開く」
を意味し、(c) の「IME OFF → 不変」と**正面から矛盾する**。
(a) が正しい（テーブルは静的で環境差が無い）なら、矛盾を環境差では
説明できない。残る説明は次のいずれか:

1. **実際に有効なキーマップが ATOK ではない**（`session_keymap` が
   MSIME/CUSTOM、または `custom_keymap_table` に該当行があり
   `gji_charset_autodetect.rs:290-295` のフォールスルーが先に効く）。
   → ゲート `ImeToggleKind::Toggle` は成立せず、**本設計は一度も発火しない**。
2. **awase が埋め込んでいる ATOK の静的知識（`Toggle`）が誤っている**。
   → `classify_mode_key_ime_action` の ATOK 分岐をゲートに使うこと自体が
   誤った表に依存することになる。
3. **キーマップ表がこの挙動を支配していない**（IMEモードキー/TSFの
   別レイヤが先に処理している）。
   → やはりキーマップ判定をゲートに使う根拠が失われる。

**v2 は L79-82 で「その前提は本 ADR の対象とは別問題として扱う
（BUG-115 側の記述訂正は別途検討）」と棚上げしている**が、上記1〜3の
どれであっても**ゲートの正当性が直撃される**。「別問題」ではなく
**本設計の前提条件そのもの**である。

**さらに派生する問題（旧M5の再掲・強化）**: (a) を採るなら atok.tsv は
今すぐ再取得して確認できる。ADR-135「スコープ確定」節は同ファイルの
`DirectInput` セクションに **Hiragana/Katakana 行が存在しない**ことを
記録している。本設計の actuation は `VK_DBE_ALPHANUMERIC`（Eisu）と
`VK_DBE_HIRAGANA`（Hiragana）を送るので、ATOK プリセット下でこの2キーが
キーマップ上どう扱われるかは **atok.tsv の `Precomposition` セクションを
見れば静的に判定できる**はずである。L141 の「GJI 向けに実機検証済み」は
BUG-25追補5（2026-08-27、別キーマップ環境）の検証であり、ATOK 下の
検証ではない。

**要求**: L79-82 の棚上げを撤回し、次を実装前提条件に格上げする。
1. 現在の実機 `session_keymap` / `overlay_keymaps` / `custom_keymap_table`
   の実値と、`classify_mode_key_ime_action(Muhenkan, raw)` の実戻り値をログで確認。
2. `atok.tsv` を再取得し、`Muhenkan` 全行・`Hiragana`/`Eisu` 全行
   （`DirectInput` / `Precomposition` / `Composition` / `Conversion`）を
   ADR に転記。(b) と (c) の矛盾がどれで説明されるかを確定する。
3. 矛盾が「awase の静的知識が誤り」で説明される場合、BUG-115/ADR-135 の
   記述訂正は「別途検討」ではなく本ADRの必須成果物になる。

---

### R3. ADR-182 との衝突は「実装順序」ではなく「正常系の定義の非互換」であり、ADR-182 の対照テストが本ADR実装後に落ちる

**該当**: 「ADR-181/ADR-182 との関係」節 L188-200、「未解決の疑問」8

**v2 の整理**（L191-194）:
> ADR-182 は「GJIのATOKプリセットでの無変換単独タップ→半角英数化」を
> **仕様（正常系）として扱い**、チョード誤判定による漏出（異常系）のみを
> 抑止する設計。本ADRは正常系の actuation 主体そのものを……置き換える

ここまでは正確である。**しかし帰結の書き方が弱い。** v2 は
「両ADRは独立に実装できず、順序を決めて統合する必要がある」（疑問8）と
順序問題に丸めているが、実際には ADR-182 が**明示的に「抑止してはならない」
と固定しようとしている対照テストが、本ADR実装後に必ず失敗する**。

ADR-182「検証計画」2（対照＝決定1が抑止してはならないもの）:

> (a) Idleから親指↓→148ms保持→親指↑（07:37:24型、`handle_key_up_pending`経路）で
>     **生の親指VKが出続けること**。
> (b) 同じくタイムアウト（>100ms保持）経路で**出続けること**。
> (d) Suppress設定（既定）では従来どおり何も出ない。

本ADR「提案する設計」4 は、まさにこの「Idle 起点の正常な単独タップ」で
**生キー `Key(vk_code)` の合成そのものを止める**設計である。つまり
ADR-182 (a)(b) は本ADR実装後に定義上 fail する。

さらに ADR-182 側には、本ADRが触れていない波及がある:

- **ADR-182 決定1c**（ユーザー判断で採用済み）は、`ModeKeyConfig` を持つ
  親指キー（＝無変換/変換）について **`on_timeout` の `PendingThumb` 腕で
  単独タップを解決しない**よう変える。本ADRの新トグルは
  `resolve_pending_thumb_as_single` に乗るので、**発火タイミングが
  「100ms タイムアウト時」から「親指 KeyUp 時」へ変わる**（＝長押し中は
  conv が切り替わらない）。本ADRの UX 前提（「単独タップしたら即座に
  半角英数」）に影響する。
- **ADR-182 決定1b** は `candidate.is_some()`（親指面のかながある文字）の
  場合に限り抑止する。本ADRの新トグルもこの条件に従うのか、
  それとも conv トグルは常に抑止対象なのかが未定義。
- **ADR-182 決定3** は `muhenkan_solo_tap_always_suppress = true` と
  `config.toml` の閾値調整を「決定1が入るまでの暫定措置」と位置づけている。
  本ADRは `always_suppress` の意味論を変える（下記 R4）ので、この
  暫定措置の位置づけも変わる。

**要求**: 疑問8 を「実装順序」から次の3点に組み替える。
1. ADR-182「検証計画」2 の (a)(b) を、本ADR実装後の期待値へどう書き換えるか
   （＝両ADRのどちらが正常系の SSOT か）をADR間で1つに決める。
   決めずに両方実装すると、テストが互いを壊す。
2. ADR-182 決定1c（タイマー経路の解決延期）による本ADRの発火タイミング変化を
   受け入れるか、例外を設けるか。
3. ADR-182 決定1b の `candidate.is_some()` 条件と本ADRのガード条件の関係。

なお、**v2 が「ADR-182 のフラグは本ADRの新トグル分岐にも同じ条件で適用する
必要がある」と書いた点自体は正しい**（誤判定された単独タップは新トグルからも
正当な単独タップに見える）。この不変条件は維持したうえで、上記の非互換を
別途解く必要がある。

---

### R4. 本ADRが対象とする症状は既定設定では構造的に発生しない（前提となる設定が本文に無い）

**該当**: 本文全体（「現状の機序」L86-95 が設定を `left_thumb_key` しか書いていない）

**実コードの確認**: `src/config.rs:508`
```rust
muhenkan_solo_tap_always_suppress: true,   // 既定
```
`src/config.rs:514` で `henkan_solo_tap_always_suppress: true` も同様。

`always_suppress = true` のとき `ThumbKeySoloTapGuard::from_legacy_bools`
（`fsm_types.rs:605-606`）は無条件 Suppress を返し、
`resolve_pending_thumb_as_single` 優先順位4 は
`SoloTapAction::Suppress`（`nicola_fsm.rs:2346-2349`）になる。
**つまり既定設定では、無変換の単独タップで生キーは一切合成されず、
GJI は無変換を受け取らない。** ADR-181「なぜ今まで顕在化しなかったか」節が
同じことを明記している:

> 既定の Suppress 設定では無変換/変換単独タップが awase に握りつぶされ
> ……今回初めて Passthrough 設定を実機で試したことで……初めて可視化された。

現ブランチの実験コミット `c0814776` / `f0e36b0e`（ADR-179 の
Passthrough 実験）が、この既定を外した状態で実機検証が行われている。

**帰結（本ADRが明示すべきこと）**:
1. 「現状の機序」（GJI が conv を倒す → idle-conv-check → DirectInput）は
   **`muhenkan_solo_tap_always_suppress = false` でのみ起きる**。
   これを書かないと、次のセッションが「既定設定でも起きる不具合」と誤認する。
2. 既定（Suppress）のユーザーにとって、本ADRは「不具合修正」ではなく
   **新機能**（無変換単独タップに半角英数トグルを付与する）である。
   `gji_thumb_key_ime_toggle` の既定値（疑問3）はこの文脈で判断すべき——
   BUG-115 が既定OFFにした4理由のうち「非opt-in全ユーザー適用」
   （理由3: 「キーマップにATOKを選んだだけの全ユーザーに自動適用される
   ——親指シフト利用者と重なりが大きい層」）が直接効く。
3. ADR-179 の Passthrough 実験の最終形として本ADRを位置づけるのか、
   実験とは独立の新機能とするのかを status に書く。

---

## Must-fix

### M11. 3点セットの「exit 方向」の belief と `AssumedReason` の扱いが未定義

「提案する設計」3 は `apply_input_mode_correction(ObservedEisu/AssumedRomaji相当, ...)`
と書くが、実コードの exit 側（`key_pipeline.rs:2701-2707`）は:

```rust
self.apply_input_mode_correction(
    InputModeState::AssumedRomaji {
        reason: awase::engine::AssumedReason::UserHalfWidthAlnumToggleOff,
    },
    crate::state::ime_event::InputModeApplyStrategy::UserHalfWidthAlnumToggle,
    now_tick,
);
```

`AssumedRomaji` は `reason: AssumedReason` を要求する。M9 で
`InputModeApplyStrategy` の新 variant を決めたのと**同じ理由**
（journal 上で左Shift由来と無変換由来を区別する）で、`AssumedReason` にも
新 variant が要る。疑問4 を「`InputModeApplyStrategy` **と `AssumedReason`** の
新 variant 名」に広げること。

### M12. 3点セットを入れる場所（`runtime/` か `state/` か）と、ADR-090 warrant／`architecture_guard` への影響が未記載

- `apply_input_mode_correction` は `Runtime` のメソッド（`runtime/mod.rs:852`）。
  一方、新トグルのトリガー判定は **コア `nicola_fsm.rs`**（B5訂正後の位置）。
  コアは `Effect`/`ImeOpenRequest` を返すだけで belief を書けない（ADR-019）。
  **「コアが判定 → windows 層が3点セットを実行」の受け渡し経路**
  （新しい `Effect` variant か、既存 `ImeOpenRequest::FollowOnly` 相当の
  conv 版か）が設計に無い。ここは本ADRで最も実装コストが読めない部分。
- `.claude/rules/ime-belief-architecture.md` 段3 が要求する
  `crates/awase-windows/tests/architecture_guard.rs` の件数固定テスト
  （`InputModeObserved` 構築点数、`user_ime_on_paths_are_paired_with_eisu_reset`）
  への影響を設計時に確認すること。
- ADR-090 の warrant（`c8bc1adc` で強制化）は open 軸の actuation を対象と
  するため conv 書き込みには効かないはずだが、**effect として「効かない」
  ことを明記**しておかないと、次に warrant を conv へ広げる際に本経路が
  漏れる（`fix-requires-evidence.md`「IME actuation 合流点」の洗い出し漏れ）。

### M13. 優先順位1・2 との関係が依然未定義（round1 M2 の残り）

v2 は「優先順位3/4に入る前に no-op 相当の分岐を追加」とだけ書き、
`nicola_fsm.rs:2257-2262` の表の**優先順位1・2 との勝敗**が未定義のまま:

| 順位 | 内容 | 行 | 本ADRとの関係（未定義） |
|---|---|---|---|
| 1 | `muhenkan_solo_tap_dedicated_fn_key` | 2273 | BUG-115 F5 が記録する「専用Fnキーが delegate を**黙って**無効化する」非対称が、新トグルにも同じ形で起きる。`warn_thumb_key_toggle_if_needed` の F5 診断（`wiring.muhenkan.is_some()` ゲート）を新トグルにも広げるか。 |
| 2 | `muhenkan_solo_tap_ime_action`（ADR-153決定1、ユーザー明示config） | 2286 | ユーザーが**明示的に**設定した意味論 vs. GJI設定からの**自動検出**。明示設定が勝つべきに見えるが、明示設定は open 軸の `ImeOpenRequest` を返す（軸が違う）ので、単純な優先順位で表現できるか要検証。 |
| — | `explicit_action_consumed` → no-op | 2314 | ケース3改（ADR-153/BUG-124）が立てるマーカー。新トグルはこの早期returnの前か後か。 |
| — | `auto_delegate_open_axis_consumed` → no-op | 2333 | ADR-154。同上。 |

優先順位表に本ADRの行を挿入した完成形を ADR に転記すること
（ADR-182 決定1 が同じ表に行を足す前提なので、表は1つに統合する）。

### M14. `HalfWidthAlnumState` 転用の可否が未決定（round1 M3 が未反映）

L211 は依然「本設計は awase 自身が状態を持つ（`HalfWidthAlnumState.toggle_held`）」
と書いており、既存型の流用を示唆している。round1 M3 で挙げた3点は未解決:

1. **ラッチ共有か分離か**。共有すると、無変換で入った半角英数が左Shift 1回
   タップ／右Shift緊急解除で抜ける（`plan_half_width_alnum_action`、
   `half_width_alnum.rs:51-69`）。分離すると **2つのラッチが同一の conv を
   独立に所有**し、本ADR L213-215 が懸念する「awase側 `toggle_held` と
   GJI 側 conv のズレ」を awase 内部で再生産する。
2. **`entry_policy`（`HalfWidthAlnumTogglePolicy`、`half_width_alnum.rs:124`）**
   の kill switch が無変換経路も殺すのか。`MsImeOnly` のときの挙動。
3. 型の主要な複雑さ（`ShiftKeyUpKind` 4値判定、`arm_tap`/`note_physical_key_down`
   の `VK_LSHIFT`/`VK_RSHIFT` 直接比較、`take_shift_up_kind_disarming_both` の
   BUG-25追補11 対策）は**無変換側では一切使わない**（単独タップ確定は
   FSM が済ませている）。共通化に値するのは `toggle_held` 1 フィールドと
   commit-on-success 規律だけであり、**型ごと転用しないほうがよい**可能性が高い。

### M15. `send_gji_half_width_alnum_toggle` の open 軸依存と commit 規律（round1 M4 が未反映）

`output/mod.rs:1230-1273` の実装上の事実:

| 行 | 内容 | 設計への影響 |
|---|---|---|
| 1241-1247 | `ime_mode_key_injection_blocked_by_modifier()` なら**無送信で `false`** | Win/Alt 押下中はトグルが不発。 |
| 1248-1254 | `if !ime_open { 無送信で false }` | **open 軸を読む**。L131 の「open軸には一切触れない」は「変更しない」の意味であって「依存しない」ではない。ユーザー観測「IME OFF → 不変」とは整合するので、これは**設計の弱点ではなく明記すべき仕様**。 |
| 1255-1268 | Enter は composition/候補表示中も発火（ADR-107決定5の緩和） | 無変換は左Shift単独タップより打鍵頻度が高くなりうる＝preedit 破壊リスクの露出が増える。コメントが「preedit破壊の兆候が実機で出た場合はここにガードを復活させること」と明記。 |

**最も危険なのは exit の commit 規律**。`kp_send_gji_restore_exit`
（`key_pipeline.rs:2381-2410`）は `prepend_synthetic_shift_up == false` のとき
**SendInput が見送られても `true` を返し、呼び出し元に belief 補正を進めさせる**
（:2401-2409 のコメントが理由を明記）。無変換タップ起点は物理 Shift を
伴わないので `prepend=false` が自然だが、そのまま流用すると
「実GJIは半角英数のまま、awase の belief だけひらがな」＝
**BUG-25追補3 と同型の実害**（engine が pass-through を抜けて生ローマ字を送る）
になる。無変換経路はユーザーが即座に再試行できる文脈なので、
`rearm_after_failed_gji_exit` 側の扱いを設計すること。

Enter 側は `commit_enter_gji()` が commit-on-success（`key_pipeline.rs:2313-2318`）
なのでそのまま流用できる。

### M16. 「GJI 向けに実機検証済み」という評価の根拠が ATOK 下では成立しない（round1 M5 が未反映、R2 に統合）

L140-141 の「GJI 向けに実機検証済み」は BUG-25追補5（2026-08-27）の検証を
指すが、その実機の `session_keymap` は CUSTOM（BUG-115、2026-09-05）
または MSIME（`gji_charset_autodetect.rs:275-281` の ADR-174 コメント、
2026-09-15）であり、ATOK ではない。R2 の atok.tsv 再取得で
`Eisu`/`Hiragana` 行を確認するまで、この評価は保留すべき。

加えて、`VK_DBE_ALPHANUMERIC` は ADR-135「スコープ確定」節が
**明示的にスコープ外にした**キーである:

> `VK_DBE_ALPHANUMERIC` は本プロジェクトで**複数回** IME OFF キーとして
> 採用・撤回されており、その都度「これは半角英数（IME ON）であって直接
> 入力ではない」という同じ事実が再発見されている……Eisu を含めると、
> 過去に複数回振り出しに戻った論点を検証不十分なまま作り込むことになる

本ADRはこのVKを無変換という新しい起点に結び付けるので、ADR-135 が
スコープ外とした理由に正面から答える必要がある（「左Shift版が既に
使っているから既決」は答えになっていない——左Shift版は BUG-25 という
別の文脈で個別に検証された例外である）。

### M17. 無変換3連打エンジンOFF（`engine_off_solo_repeat_vk`）との相互作用（round1 M6 が未反映）

`src/engine/engine.rs:100` / `nicola_fsm.rs:221`。3連打すると conv が
3回トグル（奇数回＝半角英数で終端）した上でエンジンが OFF になる。
「エンジンを止めたい」だけのつもりの操作が IME を半角英数に置いて終わる。

さらに `output.conv_mutation_allowed`（`output/mod.rs:306,695`）は
`ConvModeAuthority::UserOwned`（engine が user-disabled）の間 false になり、
その文脈では conv に一切触れてはならない（`key_pipeline.rs:219-225` の
M-4 コメント）。**3連打の3打目はその打鍵自身が authority を奪う**ので
順序依存の穴になる。

参考: ADR-182 決定1c は `engine_off_solo_repeat_vk` に一致する親指を
明示的に対象外にしている。本ADRも同じ carve-out が要る。

### M18. `AppImeProfile::InputRelay` と `conv_mutation_allowed` のゲート（round1 M7 が未反映）

- **InputRelay**（MWB/RDP等、ADR-119決定4 / issue #136）: awase 自身が
  actuation を所有しないプロファイル。本ADRが生キー合成を止めた上で
  awase も conv を書かないと、「OS側にも awase 側にも誰も切り替えない
  二重の空振り」＝ADR-119 が明文で禁じた不変条件に**新規に**該当する。
  BUG-115「Phase 3設計上の既知の限界」節が、`Decision::Consume` 経路には
  `transport.rs::plan` の Allow が構造的に効かないことを既に記録している。
- **`conv_mutation_allowed`**: M17 のほか、`AwaseOwned` でない文脈全般で
  新トグルを止める必要がある。

`.claude/rules/fix-requires-evidence.md` の「IME actuation 合流点」行が
列挙する合流点すべてについて、新ゲートの要否を洗い出すこと
（issue #136 の自己回帰の再演を避ける）。

### M19. 証拠の出所・ビルド（コミットハッシュ）が依然未記載（round1 M8 が未反映）

ログ引用に (a) ファイルパスと保全状況、(b) 取得時刻、(c) 走っていた
awase のビルド（コミット / PID）が無い。R1 とも絡むが、独立した問題として:

現ブランチ HEAD `c8bc1adc`（`feat(adr090): warrant強制(A-2)を実装`）は
`ImeOpenOutcome::Unwarranted` を新設し warrant 無しの書き込みを
**実際に止める**。ADR-182 は同じ実機ログで
「`apply(open=false)` は `outcome=Unwarranted`（`c8bc1adc` の warrant）で
止まっている」と記録している。本ADR L104 は
`[apply-ime] GJI direct: send 0x001A (open=false)` ＝**実送信**のログを
引いている。両者が同じビルドなら矛盾、違うビルドなら本ADRのログは
`c8bc1adc` 以前のもの——**つまり現在の HEAD では実IMEはもう閉じられておらず、
残るのは belief 書き込み（`handle_engine_set_open(false)`）だけ**の可能性がある。
これは「現状の機序」の実害評価を直接左右する。

ADR-182 は参考になる書き方をしている（「解析対象の `awase.log` は……
現存しない。本ADRの数値は `gaps.log` 等に基づく」）。同水準を求める。
auto-memory `feedback_confirm_physical_key_and_config_source_before_remote_test`
にも直接該当する。

---

## Nits

### N1（再掲・未修正）. ADR 本文の wikilink（ユーザールール違反・4回目）

L7（frontmatter `summary` 内）と L67（本文冒頭）の2箇所:
```
(教訓: 通称のキー名からVKを仮定せず、実機ログで実際のVKを確認してから設計する)
```
auto-memory `feedback_no_memory_wikilinks_in_source_code`
（「記憶システムの wikilink `[[...]]` 構文をソースコード/リポジトリ内 doc に
書かない、2026-09-13で3回目の再発」）に違反。**「ADR-183参照」の平文**に
置き換えること。round1 で指摘済みだが未修正。
`docs/adr/183-vk-kana-physical-delivery-passthrough.md` の status 節にも
同じ違反があるので同時修正を推奨。

### N2（再掲・未修正）. タイプミス

- L92-93: `resolve_pending_ thumb_as_single`（行折り返し位置にスペース）——
  grep できなくなる。
- 旧「トグン」は疑問1 の撤回に伴い消滅（解消）。

### N3（再掲・未修正）. BUG-015 同型という自己評価の根拠が本文に無い

L122-124。`docs/known-bugs/BUG-015.md` は実在するが、どの機序が共通なのかが
1行も書かれていない。R1 で手順5〜7 の断定を緩める際に、この段落も
併せて書き換えるか削除すること。

### N4. `status` の「残るBlocker……は本版で設計を作り直して対応した」は B1 について過大

B1 は「設計で対応した」のではなく「**診断の根拠から外した**」が正確。
実際 v2 は疑問9 で「ADR-181 の GJI 自己主張が、awase の新トグルで書いた
belief を後から乱す経路として残る可能性は排除できていない」と正しく
認めている。status の表現を実態に合わせること。

---

## 次のラウンドへの要求（優先順）

1. **R1 を先に片付ける**（frontmatter `summary` と「現状の機序」から
   撤回済みの断定を削除、ダングリング参照の解消）。これは 10 分の作業で、
   かつ未処理のまま他の議論を進めると次セッションへの汚染が固定化する。
2. **R2 の atok.tsv 再取得と `session_keymap` 実値確認**。ゲートの
   正当性が確定しないと、以降の設計はすべて条件付きになる。
   M16 もここで同時に解決する。
3. **R3 のADR間 SSOT 決定**（ADR-182 検証計画2 (a)(b) の期待値をどちらに
   合わせるか）。順序ではなく正常系の定義を1つに決める。
4. **R4 を本文に明記**（`muhenkan_solo_tap_always_suppress` 既定 `true`、
   本ADRは既定ユーザーにとって新機能、ADR-179 Passthrough 実験との関係）。
   これが決まると疑問3（`gji_thumb_key_ime_toggle` の既定）も決まる。
5. M12（コア→windows 層の受け渡し経路）を設計に入れる。ここが実装コストの
   最大の不確定要素。
6. M13/M14/M15/M17/M18 を本文に反映（round1 から継続して未反映のもの）。
7. M19・N1〜N4。
