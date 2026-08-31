# ADR-115: `.yab` 打鍵列機能（1キーに複数の `KeyAction` を定義する）

## ステータス

**採用（未実装、2026-08-31、r6でOpus 2体レビュー収束——両エージェントとも
Critical 0件・Major 0件、「問題なし、実装着手可」）。**
[GitHub Issue #118](https://github.com/cuzic/awase/issues/118)
のコメントで、リポジトリ所有者自身が「句読点自動確定（やまぶき `CV4D` 相当）を専用実装せず、
より汎用的な『1つのキーに対して複数のキーアクションの列を定義できる打鍵列機能』の一特殊ケース
として位置づける」と方針表明したことを受けて起票する。

**r1 → r2 での変更点（要約）。** r1 は Opus 2体（提案役/批判役）の独立レビューで
Critical 4件・Major 9件・Minor 15件超の指摘を受け、「現状のままでは実装に進められない」
という総合判定で一致した。両者が独立に到達した最重要の指摘は次の3点:

1. **r1 決定6（打鍵列セルを同時打鍵の組合せ解決から除外する）は、ADR 自身が根拠として
   引用した Issue #118 の実例（`'『'CV4D'』'CV4D左` 等）を到達不能にする自己矛盾だった。**
   これらは全て親指シフト面のセルで、親指シフト面は同時打鍵解決経由でしか読まれない。
2. **r1 決定2・決定9 の「セル内トークナイザ」設計は、既存レイアウト資産を壊すか
   実装不能な曖昧さを残すかのどちらかだった。** 無クォート全角トークンの1文字分割案は
   同梱 `layout/nicola.yab` の `ｋａ` 等を全壊させ、`V`+hex/`機`+数値/エスケープ付き
   クォートの前方一致化には既存コードに拠り所が無かった。
3. **r1 決定3（`Response::emit_one` の差し替え）は、対象となるコードがそもそも
   存在しなかった。** 実際の平坦化ポイントは別の場所にあり、かつ `OutputHistory`
   （KeyUp 整合性の索引）への記録経路が一切考慮されていなかったため、打鍵列に VK
   トークンが混じると stuck key（BUG-101 / ADR-112 と同じ再発ファミリー）を招く
   設計だった。

さらにユーザーから、本機能の優先順位について明示的な方針転換があった:
**やまぶきR との構文互換性は最優先事項ではない。** `.yab` の1セル内に無理に
詰め込む必要はなく、`.toml` 等の新しい設定表現を使ってよい。優先すべきは
**(a) エッジケースを含めた安定性、(b) 表現力、(c) 将来 RPA 相当（マウス操作・
待機等）まで発展させても破綻しない拡張性**である。

r2 はこれらを踏まえて設計を全面的に作り直した（`CtrlChord`/名前付きマクロレジストリの
導入）。

**r2 → r3 での変更点（要約）。** r2 も同じ2体に再レビューさせ、両者とも「条件付きで
進められる」（設計の作り直しは不要、局所修正で足りる）という判定で一致したが、
次の指摘は設計判断そのものの修正を要した:

1. **決定2 の「3要素以上は名前付きマクロ」という切り分けが、Issue #118 の実データと
   噛み合っていなかった。** 実レイアウトの `CV4D` 使用箇所33セルのうち29セルは
   「literal + `CV4D`」という**2要素**（句読点確定という主用途そのもの）で、
   5要素は4セルのみ。しかも29セルは互いに literal が異なるため**再利用が効かない**
   ——「名前付きマクロで再利用できる」という決定2 の採用理由が実データで反証された。
   r3 は**セル内 `+` 区切り（各セグメントは既存 `YabValue::parse` をそのまま再利用）と
   名前付きマクロの併用**に変更する（決定2）。
2. **`CtrlChord` がキルスイッチの対象外だった。** 既存のやまぶき派生ファイルに
   偶然含まれる `CV41`（今日は無害な `Literal→Char('C')`）が、`keystroke_sequence`
   を有効化していないユーザーでも無条件に Ctrl+A（全選択）へ変わってしまう
   設計になっていた。安定性最優先という決定0 に反するため、r3 は全ての新構文
   （`CtrlChord`/セル内 `+`/`@マクロ`）を同じゲートで統一する（決定3・決定8）。
3. **`MacroStep` が `VkCode`/`SpecialKey` を直接持ち、コンパイル不能だった**
   （どちらも serde derive を持たない core 型）。加えて `AppConfig::save()` が
   `toml::to_string_pretty(self)` で全体を書き戻すため、ユーザーの手書き TOML が
   GUI の無関係な操作で書き換わる経路もあった。r3 は `MacroStep` の全フィールドを
   `String` にし、`.yab` と同じ語彙（`CV4D`/`左` 等）で書けるようにする（決定2）。
4. **平坦化ポイントの列挙が2回連続で不完全だった。** r1 は存在しない API を指し、
   r2 は `flush_pending`（フォーカス変更・IME OFF 等の異常系パス、3箇所）を
   列挙し忘れていた。r3 は全 `into_vec()` 呼び出しを1つのヘルパー関数に統一する
   ことで「個別に列挙する」というアプローチ自体をやめる（決定5）。
5. **投機出力ガードが `SpeculativeChar` への入口を1つ（`on_timeout_speculative`）
   しか塞いでいなかった。** `confirm_policy.rs` の `idle_speculative`
   （`ConfirmMode::Speculative`/`NgramPredictive`）というもう1つの入口が素通り
   だった。r3 はガードを共通の入口関数 `enter_speculative_char` 自身に移す
   （決定7）。
6. **決定11（将来の RPA 拡張）が自己矛盾していた**（「`MacroStep` に variant を
   足すだけで済む」と「別の実行系にすべき」を同時に主張）。r3 は型レベルで明確に
   分離する（決定11）。

**r3 → r4 での変更点（要約）。** r3 も同じ2体に再レビューさせ、両者とも「条件付きで
進められる」（設計の骨格は健全、局所修正で足りる）という判定で一致したが、
r3 で新設した機構そのものに固有の欠陥が3件見つかった:

1. **投機出力ガード（決定7）が `false` を返したときのフォールバック指示が、
   入口2箇所の**既存の**「レイアウト未定義キー」用 else 分岐を流用するという
   誤りだった。** 指示どおり実装すると、`ConfirmMode::Speculative`/`TwoPhase`
   等（既定の `Wait` 以外）で打鍵列セルを押すと**打鍵列が消失するか、生の
   VK が OS へ漏れる**——両エージェントが独立に発見。r4 は入口ごとに正しい
   フォールバックを規定する。
2. **セル内 `+` 区切り（決定2a）の分割関数 `split_unquoted_plus` を、
   `YabValue::parse` の内側で呼ぶのか外側で呼ぶのかを決めていなかった。**
   これは `lint_raw_cell` の不変条件・`lint` の走査単位・再帰停止条件の
   全てに影響する。r4 は呼び出し位置を確定させる。
3. **キルスイッチ Off 時の復元に `YabValue::serialize()` を使う設計が、
   `serialize()` が `parse()` の厳密な逆写像ではないために破綻していた**
   （クォート種別の非保持、16進のゼロ詰め落ち、空白の正規化等）。
   「`Off` なら挙動は1ミリも変わらない」というキルスイッチ自身の目的を
   裏切っていた。r4 は `CtrlChord`/`InlineSequence`/`MacroRef` に元のセル
   生テキストをそのまま保持させ、Off 復元を「保持した生テキストで
   `Literal` を作る」という非可逆変換を伴わない形に変更する。

詳細な差分は各決定の本文に記す。旧 r1/r2/r3 の内容は git 履歴で参照できる。

**r4 → r5 での変更点（要約）。** r4 も同じ2体に再レビューさせた。提案役は
「条件付きで進められる（Critical 0件）」と判定したが、批判役は
**決定2（セル内 `+` 区切り）が第2の `Sequence` 生成経路であるにもかかわらず、
決定4（非ネスト）・決定6（生 `Key` 禁止）の不変条件が「マクロ経由」だけを
想定して書かれており、`InlineSequence` 経由には追随していなかった**という、
実データで裏付けられる2件の Critical を発見した。両エージェントの判定が
割れたが、批判役の指摘は Issue #118 の実レイアウト（`[拡張親指シフト1]` に
`V1D` 等の `Vk` トークンが実在する）と ADR 自身の記述（決定2 の「(a)/(b) の
合成」が「追加コード無しで成立する」と明記していた箇所が実際には成立しない）
に基づいており妥当と判断し、両方を反映する:

1. **`InlineSequence` の要素に `Vk`/`None` を許可するフィルタが無かった。**
   `'あ'+V1D` のようなセルが `Sequence([Char('あ'), Key(0x1D)])` になり、
   決定6 が防いだはずの stuck key（BUG-101 / ADR-112 再発ファミリー）を
   別経路から再導入していた。決定2b の許可リストを `InlineSequence` にも
   適用する（決定2c）。
2. **`InlineSequence` の要素が `MacroRef` のとき、マクロの steps を
   `Sequence` で包んだまま埋め込んでいたため `Sequence` がネストし、
   決定4 の非ネスト不変条件が壊れていた。** マクロの steps を平坦に
   `extend` する形に修正する（決定2c）。
3. **決定7 の `on_timeout_speculative` フォールバック（「その場で確定出力」）
   が、打鍵列セルの同時打鍵受付窓を `simultaneous_threshold_ms`（Wait
   モード相当）から `speculative_delay_ms`（既定30ms）に縮めていた。**
   `PendingChar` を維持したまま残り時間で `TimerCommand` を再設定する形に
   変更し、確定ロジックを手書きで複製しない設計にする（決定7）。
4. **`YabFace::resolve_kana` が `InlineSequence` の中まで降りず、
   `ｋａ+CV4D` のような `Romaji` 要素の kana が解決されないまま残っていた。**
   1階層だけ降ろすよう修正する（決定2a）。
5. 決定8 に r3 時点の記述（`Literal(value.serialize())`）が残存し、
   決定3・決定9(a) の `raw` 直接参照という r4 の方針と食い違っていた。
   決定8 を決定3 と同じ表現に揃える。
6. `is_valid_macro_name` が空文字列を弾かず `@` 単独セルの扱いが未定義
   だった点、`lint` の分割規則が `parse_cell` の空セグメントガードを
   共有していなかった点、GUI（`awase-settings`）が新構文を authoring
   できずテキスト欄経由だと破壊する限界が未記載だった点を修正・明記する。

**r5 → r6 での変更点（要約）。** r5 を再レビューさせた結果、両エージェント
とも**Critical 0件**——提案役は「問題なし」、批判役は「条件付きで進められる
（Major 3件は設計変更を伴わない記述追加のみ）」と判定した。r4 で新設した
`InlineSequence`（セル内 `+` 区切り）に決定2b/決定6 の許可リストが追随して
いなかった2件の Critical（`Vk` フィルタ未適用によるstuck key再発、`MacroRef`
展開時の `Sequence` ネスト）は決定2c で構造的に解消し、以後の指摘は記述の
精度・未確定事項の明記に収束した。r6 は残った3件の Major と主要な Minor を
反映する:

1. **全ステップ/全セグメントが許可リストで拒否されたとき**（`V1D+V1C` や
   `steps = []` 等）に空の `Sequence(vec![])` が生まれ、扱いが未定義
   だった。既存の「明示的な無出力」を表す `YabValue::None` に統一する。
2. **`InlineSequence` に `Romaji` を許可したことで、そのセルが n-gram
   文脈・同時打鍵の kana 依存閾値調整から外れる**（`lookup_face` の
   kana 抽出が `Sequence` を素通りするため）という副作用が未記載
   だった。既知の制約として決定2c に明記する。
3. `resolve_keystroke_syntax`/`resolve_macro_steps` の設置モジュールを
   `src/yab/`（`crate::config` を import する一方向依存）に確定する。
4. Minor 群: `resolve_macro_steps` 呼び出しの引数数の記述誤り、`Romaji`
   禁止理由の2箇所での食い違いの統一、決定2c の match への
   `YabValue::Sequence(_)` アーム追加（コンパイルを通すため）、決定7
   `idle_speculative` の後段機構の記述訂正（`Phase2Transition` ではなく
   満額 `TimerIntent::Pending` を経由する）、`KeyAction::romaji()` の
   非網羅性（5ラウンド連続の指摘）を既知の制約として確定、マクロの
   `Romaji` 拒否警告文に代替手段（セル内 `+` 区切り）を明記。

[ADR-109](109-yab-cv4d-punctuation-auto-confirm.md)（保留中、句読点確定専用ラッパー案
の調査ログ）とは以下の関係に整理する: ADR-109 が検討した「composing 観測に基づく
条件付き確定（`ConfirmThenSend`/`ConfirmIfComposing`）」は、**composing 観測の
実装がまだ存在しない**（後述コンテキスト・レビュー指摘 M1）ことと、Issue #118 の
実レイアウトが実際には無条件の `Ctrl+M` 送信で長期運用されている実績（後述コンテキスト）
があることから、**本 ADR の v1 スコープでは採用しない**。ADR-109 はそのまま保留とし、
将来 composing 条件付き確定が本当に必要になった時点の参考資料として残す。

## コンテキスト

### 要望と実例（Issue #118）

Issue #118 は当初「句読点入力時に直前の変換候補を自動確定する」（やまぶき `CV4D` 相当）
機能要望として報告された。所有者の調査により、専用の2要素 variant を先に作ると汎用機能設計
時に型を二重管理することになるため、汎用「打鍵列機能」の一特殊ケースとして実装する方針にした。

報告者が実際に使っている（ライフラボ社製・やまぶき派生の）レイアウトファイルには、次の
2つの重要な事実がある。

**事実1: `CV4D` は「確定サフィックス」ではなく `CVxx` = `Ctrl+<英字>` 語彙の一員である。**
報告者が貼ったレイアウトの末尾コメント:

```
; CV41(Ctrl+A)、CV58(Ctrl+X)、CV43(Ctrl+C)、CV56(Ctrl+V)、CV4D(Ctrl+M)
```

`0x41`=`A`、`0x58`=`X`、`0x43`=`C`、`0x56`=`V`、`0x4D`=`M`——これは偶然ではなく、
Win32 の VK コードは英大文字キーに関して ASCII コードと一致する
（`VK_A`=`0x41`…`VK_Z`=`0x5A`）という事実そのものである。つまり `CVxx` は
「`C`（Ctrl修飾）+ `V`+16進数（`.yab` に既存の直接VK指定構文、`parse_direct_vk`、
`src/yab/mod.rs:484-490`）」という**極めて素直な合成表記**であり、`CV4D` は
「Ctrl を押しながら VK 0x4D（=`M`）を送る」という意味の**独立した単一トークン**である。
報告者本人も「この『句読点入力時に **Ctrl+M を送って** 即時確定する』機能は…以前、
私からライフラボ社に…お願いし、それを受けて同社が実装してくださった」機能だと明記して
おり、**これは無条件の Ctrl+M 送信であり、composing 観測に基づく条件分岐は行っていない**。
かつ「標準的に広く使われている仕様ではなく個人的な要望から生まれた機能」としながらも、
長期間実運用されている（実害の記録は無い）。

**事実2: 実際の打鍵列は2要素の組ではなく、任意長の列である。**

```
[ローマ字左親指シフト]
...,'［'CV4D'］'CV4D左,...,'（'CV4D'）'CV4D左,...,'『'CV4D'』'CV4D左,...
```

`'『'CV4D'』'CV4D左` は1セルにつき5アクション（「『」出力→Ctrl+M→「』」出力→
Ctrl+M→カーソル左1つ、`左` は `.yab` の既存トークンで `SpecialKey::Left`、
`mod.rs:410`）を意味する。**これらは全て親指シフト面（`[ローマ字左親指シフト]`/
`[ローマ字右親指シフト]`）のセルである**——この事実が r2 の決定4（同時打鍵除外の撤回）
の直接の根拠になる。

### 現状のコード構造（r1 レビューで裏取りされた事実、修正版）

1. **`.yab` の1セルは単一の `YabValue` にしか対応しない。** `YabFace`
   （`src/yab/mod.rs:19-34,48`）は `PhysicalPos` → `Option<YabValue>` の固定配列。
   `YabValue::parse`（mod.rs:84-112）はセルの生テキスト全体を1回だけ判定し、単一の
   variant（`Romaji`/`Literal`/`KeySequence`/`Special`/`Vk`/`None`）を返す。
2. **`YabValue → KeyAction` 変換は 1:1 のコンテキストフリー関数**
   （`impl From<&YabValue> for KeyAction`、`src/engine/nicola_fsm.rs:57-73`）。
3. **`lookup_face` の呼び出し元は 14 箇所**（r1 の「11箇所」は誤り、レビュー指摘 m2）。
   `nicola_fsm.rs`: 725, 768, 943, 1027, 1159, 1196, 1238, 1305, 1581, 2011。
   `confirm_policy.rs`: 61, 119, 122, 125。うち `confirm_policy.rs` の3箇所と
   `nicola_fsm.rs:725`（`lookup_kana_at`）・`nicola_fsm.rs:1196`
   （`step_pending_char_thumb`、`candidate.as_ref().and_then(|(_, kana)|
   *kana)`）の**計5箇所**は **action を捨てて kana しか使わない**（レビュー指摘
   Minor6、r3 まで4箇所のままだった誤りを訂正）。残り9箇所は
   `ParseAction::Reduce{ actions: smallvec![action], record:
   OutputUpdate::record(sc, &action, kana) }` または `ResolvedAction{ actions, output }`
   を組み、**action は送信列（`actions`）と出力履歴（`record`）の2系統に同時に流れる**
   （レビュー指摘 C3、後述決定6の前提）。
4. **`timed_fsm::Response<A, T>` の `actions` は最初から `Vec<A>`。**
   `Response::emit(actions: Vec<A>)`（`crates/timed-fsm/src/response.rs:94-101`）も
   既存。実際に `SmallVec<[KeyAction;2]> → Vec<KeyAction>` へ変換している箇所
   （＝平坦化ポイント）は `NicolaFsm::decide()`（`nicola_fsm.rs:854-882`）内の
   **2箇所**（`Reduce`/`ReduceAndContinue`）、`build_response()`（`:826-841`）の
   1箇所、および `flush_pending`（`:375-457`、フォーカス変更・IME OFF・
   エンジン無効化という異常系パスで `resolve_pending_char_as_single`/
   `resolve_char_thumb_as_simultaneous` の結果をそのまま emit する）の3箇所、
   **計6箇所**である（レビュー指摘 C1/C2——r1 決定3 が指定した
   `Response::emit_one` はエンジンに1箇所も存在せず、r2 はこの6箇所のうち
   `flush_pending` の3箇所を挙げ忘れていた。両者とも決定5 で訂正済み）。
5. **`Output::send_keys(&self, actions: &[KeyAction])`**
   （`crates/awase-windows/src/output/mod.rs:1139`）は `&[KeyAction]` を受け取り
   `for action in actions { match action { .. } }`（`:1160-1211`）で逐次送信する。
   この `match` は網羅的。
6. **出力履歴（`OutputHistory`）の KeyUp 整合性索引は、`KeyAction::Key(vk)` の
   リテラルな1階層 match/if-let に依存しており、コンパイラの網羅性検査が効かない**
   （レビュー指摘 C3/C4/Minor10）。
   - `append_key_up_for`（`nicola_fsm.rs:819-823`）: `if let Some(KeyAction::Key(vk))`
   - `release_only`（`nicola_fsm.rs:1880-1894`）: `Char(_)|Romaji(_) => Suppress`、
     `Key(vk) => KeyUp(vk)`、**`_ => Response::pass_through()`**——このため未知の
     variant は物理 KeyUp を OS へ素通しする
   - `drain_pending_releases_as_keyups`（`output_history.rs:196-204`）:
     `.filter_map(|(_, action)| match action { KeyAction::Key(vk) => Some(..),
     _ => None })`
   - `remove_by_scan`（`output_history.rs:145-150`）は `.position()` で**最初の1件のみ**
     除去する。1 scan_code に複数の `Key(vk)` を積む設計は成立しない。
   - これは `output_history.rs:189-195` のコメントが名指しで警告している
     ADR-112 / BUG-101 の再発ファミリーそのもの。**打鍵列に生の `KeyAction::Key`/
     `KeyUp` を含めてはならない**という制約は、この事実から直接導かれる
     （決定7で確定させる）。
7. **`is_composing()` の Windows 実装・composing 観測経路は、現状 `send_keys` から
   到達可能な形では存在しない**（レビュー指摘 M1。r1 は「既に存在する」と誤記していた）。
   `ImeDetector` トレイトの Windows 実装は無く（`grep` で0件）、
   `ime_composition_active_now() || gji_candidate_visible_now()` という OR 合成も
   コードベースに存在しない。ADR-109 決定3 は「ADR-107 決定5 が採った OR 合成を
   **踏襲する方が**よい」という将来形の提案であり、既成事実ではなかった。
   composing 条件付き確定を実装するなら、この観測経路を**新規に敷設する**必要がある
   ——本 ADR がこれを v1 スコープから外す（決定1）理由の一つ。
8. **`AppConfig`（`src/config.rs:589-601`）には、名前付きルールのリストを
   トップレベルに持つ既存の前例が複数ある。** `keymaps: Vec<KeymapRule>`
   （`KeymapRule{ app: Option<String>, from: String, to: Option<String> }`、
   `config.rs:502-511`）、`post_bypass: Vec<PostBypassRule>`（`:573-582`）。
   いずれも `#[serde(default)]` で任意の TOML テーブルとして追加されており、
   本 ADR の「名前付きマクロのリスト」（決定2）はこの並びにそのまま追加できる。
9. **`crates/awase-windows/src/app/mod.rs:221` の `parse_key_combos` と
   `crates/awase-windows/src/vk.rs:531-551` の `parse_key_combo(s: &str) ->
   Option<awase::config::ParsedKeyCombo>`** が、`"Ctrl+I"` のような文字列を
   `ParsedKeyCombo{ ctrl, shift, alt, vk: VkCode }`（`config.rs:988-993`、**core**
   に定義）へパースする経路として既に存在する。ただし `VkCode::from_name`
   （名前→VkCode のテーブル）は `crates/awase-windows/src/vk.rs` にしかない
   （プラットフォーム固有）。本 ADR は Ctrl+英字の表現に、この経路（プラットフォーム
   固有の名前解決）ではなく、**既存の `V`+hex 直接指定（core、プラットフォーム
   非依存）に `C` プレフィクスを足すだけの新ブランチ**を使う（決定1）——理由は
   コンテキスト事実1 で確認した「`CVxx` は元々 `C`+`V`+hex の合成だった」という
   実態と正確に一致するため。

### 制約

- [layer-boundaries](../layer-boundaries.md) A-1: コア `awase` クレートは OS 非依存を保つ。
  `VkCode` 型自体は core（`src/types.rs`）にあり、`V`+hex 直接指定は既に core の
  `YabValue::parse` で完結している——Ctrl+VK 合成もこの延長として core に置ける。
- `KeystrokeMacro`（`steps: Vec<String>`）の TOML 定義・解決ロジックは
  `AppConfig`（`src/config.rs`）と同じく core に置く。プラットフォーム固有
  なのは「解決済みの `KeyAction::Sequence` をどう `SendInput` するか」
  （`output/`）だけ。
- [fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md):
  「キー選択（IME ON/OFF に送る VK）」ファミリーへの該当は無い（Ctrl+M は
  IME ON/OFF キーではなく、確定用に汎用的に使われる修飾キーコンボであり、
  ADR-107/109 が扱う IME 制御キー選択とは別軸）。ただし出力送信・
  `OutputHistory` に触れる以上、決定7・決定10 の証拠義務は課す。

---

## 決定0（スコープと優先順位）

**優先順位（ユーザー指示、2026-08-31）**: (a) エッジケースを含めた安定性 >
(b) 表現力 > (c) 将来の RPA 的拡張を潰さない拡張性 >>> (d) やまぶきR との構文互換性。
(d) は「あれば嬉しい」程度であり、(a)(b)(c) と衝突する場合は (d) を捨てる。

**対象**: `.yab` の1つのキーに、1回の物理キー押下（KeyDown）に対して**順序付きで
実行される複数の出力アクション**（打鍵列）を定義できるようにすること。

**r1 からの最大の設計転換**: r1 はセル内の生テキストを「区切り文字なしで連結された
複数トークン」として左から走査する独自トークナイザを作ろうとしたが、これは
やまぶき互換性を最優先した結果であり、レビューで最も危険な指摘（既存レイアウト破壊・
実装不能な曖昧さ・誤字検出機構の無効化）を集中して受けた領域だった。ユーザーの優先順位
転換を受け、r2 は**セル内トークナイザを廃止**し、代わりに:

- Ctrl+英字1個だけの合成（`CV4D` 等）は、`.yab` の**既存の `V`+hex 構文に `C`
  プレフィクスを足す新しい単一トークン**として扱う（決定1）。既存パーサの分岐を
  1つ増やすだけで済む。
- **単発・局所的な列（句読点確定等）は、セルの生テキストをクォート外の `+` で
  区切り、各セグメントを既存 `YabValue::parse` にそのまま渡す**（決定2a）。
  「トークンの種類判定」と「区切りの検出」を分離することで、r1 のトークナイザが
  抱えていた曖昧さ（前方一致の要・エスケープ境界の要）を回避する——分割器の
  責務は「クォート外の `+` を見つける」ことだけであり、各トークンの意味判定は
  一切行わない。
- **複数キーで再利用する列、または将来ステップ種別が増える列は、`.yab` の外
  ——`config.toml` に名前付き「打鍵列マクロ」として定義し、`.yab` セルからは
  `@マクロ名` という1トークンで参照する**（決定2b）。両者は排他ではなく、
  `+` 区切りの1セグメントとして `@name` を書くこともできる（決定2c）。

**非対象（明示的に扱わない）**:

- **composing 観測に基づく条件付き確定**（ADR-109 の `ConfirmThenSend`/本 ADR r1 の
  `ConfirmIfComposing`）。理由はコンテキスト事実1・事実7（観測経路が存在せず、かつ
  無条件送信の実運用実績がある）。将来必要になれば別 ADR とする。
- **マウス移動・クリック・sleep/wait 等の RPA アクション**。決定11 で「将来追加しても
  破綻しない」ことだけを設計上保証し、実装はしない。
- **打鍵列マクロの編集 GUI**（`awase-settings`）。当面は `config.toml` 手動編集のみ。
  ただし `.yab` レイアウトファイル側で `@マクロ名` セルを**無損失に読み書きできる**
  ことは必須（決定9）。
- **同時打鍵（マクロ）同士の組合せ解決**。決定4 参照。

---

## 決定1: `CV4D` 系トークンは `C`+`V`+hex（core、`.yab` 既存構文の延長）として実装する

`YabValue::parse`（`src/yab/mod.rs:84-112`）に、既存の `parse_direct_vk`
（`:484-490`、`V`+16進数）と対になる新しい前方一致関数を追加する:

```rust
/// `C`+`V`+16進数（半角）の Ctrl 修飾VK直接指定をパースする。
/// 例: `CV4D` → Ctrl+VK(0x4D) = Ctrl+M。
fn parse_ctrl_vk(s: &str) -> Option<VkCode> {
    let hex = s.strip_prefix("CV")?;
    if hex.is_empty() || !hex.is_ascii() {
        return None;
    }
    u16::from_str_radix(hex, 16).ok().map(VkCode)
}
```

`YabValue::parse` の判定順に、`parse_direct_vk` の**前**に挿入する
（`CV4D` は `parse_direct_vk` にとって `V` プレフィクスに一致しないため、
本来どちらを先にしても衝突しないが、`C`+`V`+hex の方がより具体的な形なので
先に置く）:

```rust
if let Some(vk) = parse_ctrl_vk(trimmed) {
    return Self::CtrlChord { vk, raw: trimmed.to_string() };
}
```

**受理範囲は `parse_direct_vk` と同じ性質**——`CV` に続く16進数なら
何でも受理する（例: `CVE` → Ctrl+VK(0x0E)）。新種の危険ではなく、既存 `V`+hex と
同じ「タイプミスをそのまま受理する」設計を踏襲するだけである。`CV`+非16進
（例: `CVXY`）は `None` を返し、従来どおり `Literal` にフォールバックする。

```rust
pub enum YabValue {
    Romaji { romaji: String, kana: Option<char> },
    Literal(String),
    KeySequence(String),
    Special(SpecialKey),
    Vk(VkCode),
    /// 新設。Ctrl+VK の単一チョード送信（コンテキスト事実1 の CVxx 語彙）。
    /// `raw` は元のセルテキスト（トリム済み）をそのまま保持する
    /// ——キルスイッチ Off 時の復元に使う（決定3、レビュー指摘 C3）。
    /// `YabValue::serialize()` から `parse` の逆写像を再構成しようとすると
    /// 16進のゼロ詰めが落ちる（`CV0D` → `"CVD"`）等の非可逆性があるため、
    /// 逆写像を作る代わりに生テキストをそのまま持たせる。
    CtrlChord { vk: VkCode, raw: String },
    /// 新設（決定2）。セル内 `+` 区切りによる打鍵列（非ネスト）。
    /// `raw` は元のセルテキスト全体（トリム済み）——理由は `CtrlChord` と同じ
    /// （クォート種別の非保持・空白の正規化等、`serialize()` は逆写像にならない）。
    InlineSequence { items: Vec<YabValue>, raw: String },
    /// 新設（決定2）。名前付き打鍵列マクロへの参照。
    /// `name` は `strip_prefix('@')` の結果をそのまま保持するため、
    /// `format!("@{name}")` は常に元のセルテキストと厳密に一致する
    /// （`CtrlChord`/`InlineSequence` と異なり非可逆性が無いので `raw` は不要）。
    MacroRef(String),
    None,
}
```

`KeyAction` にも対応する `CtrlChord(VkCode)`（`raw` を持たない——`KeyAction` は
OS への送信専用で `.yab` への書き戻しに使われないため）を追加する:

```rust
YabValue::CtrlChord { vk, .. } => Self::CtrlChord(*vk),
```

**送信は自己完結にする**（決定6 の `OutputHistory` 制約に抵触しないため）。
Ctrl 押下 → 対象VK押下 → 対象VK解放 → Ctrl解放の4イベントを**1回の `SendInput`
呼び出しにバッチして**送る（レビュー指摘 M1）。**低レベルの注入は既存の責務分担
（`send_keys` の他の全アームが `self.injector.*` を経由する）に揃え、
`KeyInjector`（`crates/awase-windows/src/output/key_injector.rs`）に新しい
バッチ送信メソッドを1つ足す**（レビュー指摘 M6——r3 のスケッチは
`crate::win32::send_input_safe` を `output/mod.rs` から直接呼んでおり、
`KeyInjector` の責務を跨いでいた。既存 `send_key` が `unicode_cold_defer` 等の
状態を見ている（`key_injector.rs:118-122`）ため、injector をバイパスすると
将来の deferral 機構と齟齬を起こしうる）:

```rust
// crates/awase-windows/src/output/key_injector.rs
impl KeyInjector {
    /// Ctrl+VK の1チョードを、Ctrl↓/VK↓/VK↑/Ctrl↑ の4イベントとして
    /// 1回の SendInput にバッチして送る（決定1）。
    pub(super) fn send_ctrl_chord(&self, vk: VkCode) {
        let inputs = [
            make_key_input(crate::vk::VK_CONTROL, false),
            make_key_input(vk, false),
            make_key_input(vk, true),
            make_key_input(crate::vk::VK_CONTROL, true),
        ];
        let _ = crate::win32::send_input_safe(&inputs);
    }
}
```

`send_keys` の `CtrlChord` アームは:

```rust
KeyAction::CtrlChord(vk) => {
    log::debug!("  → CtrlChord(Ctrl+{vk:#06X})");
    self.injector.send_ctrl_chord(*vk);
}
```

**`VkCode(VK_CONTROL)` ではなく `crate::vk::VK_CONTROL` を使う**（レビュー指摘
M6）: `vk.rs:22` の `pub const VK_CONTROL: VkCode = VkCode(0x11);` は既に
`VkCode` 型であり、`VkCode(VK_CONTROL)` は型エラーになる（r3 のスケッチの誤り）。

**単一バッチにする理由**: `self.injector.send_key(..)` を4回呼ぶ素朴な実装は
4回の独立した `SendInput` になり、その間に実ハードウェア入力が OS の入力キューへ
割り込みうる（割り込んだキーは「Ctrl 押下中」として対象アプリに届く）。この
リポジトリの既存の修飾キー付き注入（`crates/awase-windows/src/hook.rs:305-310`
`inject_alt_menu_mask`、`crates/awase-windows/src/ime.rs:126-170`
`push_release`/`push_restore`）はいずれも `Vec<INPUT>` を組んで1回の `SendInput`
で送る前例に揃えている。

**自己注入マーカーにより物理修飾キー状態は汚染されない**: `make_key_input` は
常に `INJECTED_MARKER` を付与し（`key_injector.rs:50-52`）、
`hook.rs:773-776 is_self_injected` がこれを検出して `CallNextHookEx` で
素通しするため、注入した Ctrl はエンジンの `phys.modifiers.ctrl` を汚染せず
`OsModifierHeld` バイパスを誘発しない。`read_os_modifiers()`
（`observer/focus_observer.rs:26-38`）も `PHYSICAL_KEY_STATE` を読むため
同様に無汚染である。

**押下中の物理修飾キーとの衝突は既知の限界として明示する**: ユーザーが物理的に
Shift/Alt を押している最中に `CtrlChord` が発火すると `Ctrl+Shift+M`/`Ctrl+Alt+M`
になりうる。`ime.rs:126-170` の `push_release`/`push_restore`（IME モードキー用に
押下中の修飾キーを解放→復元する仕組み）を流用することも検討したが、v1 では
「打鍵列セルは通常、修飾キーを押しながら打つ位置には置かれない」という運用上の
前提に留め、実機ソークでこの限界が実害を出した場合にのみ流用を検討する
（決定9(c) の実機確認項目に追加）。

**`OutputHistory` に解放対象は残らない**（`OutputUpdate::record` は呼ばれるが、
`CtrlChord` は「1回の呼び出しで完結し、後から解放すべき片割れが無い」ため——
決定6の制約は「片方だけ送って後から解放を期待する」`Key`/`KeyUp` にのみ適用され、
`CtrlChord` はそもそもその形を取らない）。

**旧 `V`+hex（`Vk` variant）との衝突は起きない**: `parse_direct_vk` は `V` の直後を
即座に16進として読むため `CV4D` に対しては `strip_prefix('V')` が失敗し（先頭が `C`）、
`None` を返す。逆に `parse_ctrl_vk` は `CV` の2文字プレフィクスを要求するため、
純粋な `V4D`（Ctrl無しの VK 0x4D 直接指定）とは衝突しない。

**キルスイッチは決定3 で `InlineSequence`/`MacroRef` と統一的に扱う**（r2 では
`CtrlChord` だけがゲート対象外で、既存のやまぶき派生ファイルに偶然含まれる
`CV41` が `keystroke_sequence` を有効化していないユーザーでも無条件に Ctrl+A
（全選択）へ変わってしまう非対称な設計だった。レビュー指摘 Major1。r3 で是正
する、後述決定3）。

---

## 決定2: セル内 `+` 区切り（単発・局所的な列）と名前付きマクロ（再利用・将来拡張）を併用する

**r2 からの変更（レビュー指摘 C2/M5、両エージェント一致）**: r2 は「3要素以上は
名前付きマクロ」と決め打ったが、Issue #118 の実レイアウトを数えると `CV4D` を
含むセルは合計33あり、**うち29セルが「literal + `CV4D`」という2要素**（句読点
`．CV4D`/`，CV4D` を含む、Issue の主用途そのもの）、5要素は4セルのみだった。
しかも29セルは literal が1つずつ異なるため、名前付きマクロにしても**再利用は
一切効かない**——r2 決定2 の採用理由(a)「同じ打鍵列を複数キーで再利用できる」が
実データで反証された。1レイアウトの移植に ~33 個の `[[keystroke_macro]]` を
手書きするのは「(b) 表現力」の優先順位に反する。

r3 は両方を採る。**排他ではない**——セル内トークンの1つとして `@name` を許せば
両立する。

### (a) セル内 `+` 区切り（単発・局所的な列。config.toml 不要）

セルの生テキストを、**クォートの外側にある** `+`（半角、U+002B。全角 `＋`
U+FF0B とは別物——後述）で分割する。各セグメントは既存の `YabValue::parse`
へ**そのまま**渡す（新しいトークン分類ロジックは一切書かない）。分割そのもの
だけを担う、単一責務の小さな状態機械:

```rust
/// セル生テキストを、クォート外の `+` で分割する。
/// クォート（'/"）の対応関係だけを追跡し、トークンの意味は一切判定しない
/// ——各セグメントの解釈は既存 `YabValue::parse` に完全に委譲する。
/// クォート**内**のバックスラッシュのみをエスケープとして扱う
/// （`unescape_literal` がクォート内でしかエスケープを解決しないのと同じ前提）。
fn split_unquoted_plus(raw: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, ch) in raw.char_indices() {
        if escaped { escaped = false; continue; }
        match (quote, ch) {
            (None, '\'' | '"') => quote = Some(ch),
            (Some(q), c) if c == q => quote = None,
            (Some(_), '\\') => escaped = true,
            (None, '+') => { segments.push(&raw[start..i]); start = i + 1; }
            _ => {}
        }
    }
    segments.push(&raw[start..]);
    segments
}
```

**呼び出し位置は `YabValue::parse` の外——`parse_face`（`yab/mod.rs:559`
付近）から呼ぶ新しい入口関数 `parse_cell` に置く**（レビュー指摘 Critical2/C2
——r3 はこの呼び出し位置を未決定のまま残していた）。`YabValue::parse` 自身は
**一切変更しない**（decision1 の `CtrlChord`/後述 `MacroRef` の2ブランチ追加を
除く。どちらも既存6分岐と同じ全文一致で、前方一致や分割を伴わないため
`YabValue::parse` の「1トークン→1値」という契約は壊れない）:

**「分割するかどうか」の判定を `parse_cell` と `lint` の共有関数に切り出す**
（レビュー指摘 M2——r4 は `parse_cell` と `lint` それぞれが独立に
`split_unquoted_plus` を呼ぶ形を想定していたが、`lint` が `parse_cell` の
空セグメントガードを共有しなければ、`+'a` のようなセルで「実際のパース
結果」と「lint が検査する単位」がズレる）:

```rust
/// セルを分割すべきか判定し、分割するならセグメント列を返す。
/// `parse_cell` と `lint` の両方がこれを呼ぶ——分割規則を1箇所に集約する。
fn cell_segments(trimmed: &str) -> Option<Vec<&str>> {
    let segments = split_unquoted_plus(trimmed);
    // 空セグメントが1つでもあれば分割しない(先頭/末尾/連続する `+`、
    // レビュー指摘 Major1/M2)。`YabValue::parse("")` は `YabValue::None`
    // を返すが、`None` は「明示的な無出力」という特別な意味を持つ値
    // （`resolve_thumb_face` の chord フォールバック遮断に使われる、
    // yab/mod.rs:742-743）なので、分割の副作用として紛れ込ませてはならない。
    if segments.len() < 2 || segments.iter().any(|s| s.trim().is_empty()) {
        None
    } else {
        Some(segments)
    }
}

/// `.yab` セルの解釈における唯一の入口。`parse_face` はここを呼ぶ
/// （`YabValue::parse` を直接呼ばない）。`YabValue::parse` 自体は
/// 62箇所の既存呼び出し元（本番2箇所＋`src/yab/tests.rs`、レビュー指摘
/// m1——r1〜r4 が「168箇所超」と書いていたのは根拠不明な数字だったため
/// 実測値に訂正）に影響を与えないため無改修。
pub fn parse_cell(raw: &str) -> YabValue {
    let trimmed = raw.trim();
    match cell_segments(trimmed) {
        None => YabValue::parse(trimmed),   // 今日と完全に同じ結果
        Some(segments) => YabValue::InlineSequence {
            items: segments.iter().map(|s| YabValue::parse(s)).collect(),
            raw: trimmed.to_string(),
        },
    }
}
```

再帰は発生しない——`parse_cell` が `YabValue::parse` を呼ぶだけで、
`YabValue::parse` は `+` 分割を一切行わないため、無限再帰の懸念がない。

**`lint_raw_cell`/`lint` も `cell_segments` に揃える**（レビュー指摘
Critical2/C2/M2）: `lint_raw_cell`（`yab/mod.rs:134-150`）の不変条件
（`parse(x) == Literal(x)` で「フォールバック経路に落ちたか」を判定する）
は**セグメント単位でそのまま成立する**——`lint_raw_cell` 自体は変更せず、
`lint`（`:194-231`、セルを `split(',')` してセル単位で `lint_raw_cell`
を呼ぶ）を「セルをまず `cell_segments` で判定し、`Some` ならセグメントごと
に、`None` ならセル全体に対して `lint_raw_cell` を呼ぶ」形に変更する。
1セグメントのセルは今日と同じ（`lint_raw_cell(cell)` を1回だけ呼ぶ）。
これにより、`ｂ'ｕ` のようなクォート不整合の誤字は、`+` 区切りのどの
セグメントに現れても検出され、かつ `parse_cell` が実際に生成する
`YabValue` と `lint` が検査する単位が常に一致する。

例（Issue の実例、句読点確定＝主用途）:

```
'．'+CV4D                → InlineSequence{items: [Literal("．"), CtrlChord{vk:VK(0x4D),..}], raw: "'．'+CV4D"}
'『'+CV4D+'』'+CV4D+左     → InlineSequence{items: [Literal("『"), CtrlChord{..}, Literal("』"), CtrlChord{..}, Special(Left)], raw: "..."}
```

**安全性の根拠**: この分割器の仕事は「クォート外の `+` の位置を見つける」ことだけで、
トークンの種類判定（`V`+hex か `機`+数値か等）には一切関与しない。したがって
r1 決定2 が抱えていた曖昧さ（無クォート全角の境界、`V`+hex と `CV`+hex の衝突、
エスケープ付きクォートの前方一致化）は**構造的に発生しない**——各セグメントは
既存 `YabValue::parse` の6+2分岐（全文一致、変更なし）にそのまま渡る。

**エスケープ規則は2系統が並存する点に注意**（レビュー指摘 m3）: `split_unquoted_plus`
はクォート種別を問わず `\` の直後の1文字をエスケープ扱いするが、
`unescape_literal`（`mod.rs:440-481`）はクォート種別が一致する場合のみ
エスケープと解釈する（例: ダブルクォート内の `\'` は `\` と `'` の2文字の
まま残る）。実レイアウトの `[小指拡張親指シフト1]` に実在する `"\""` `"\'"`
`"\\"` の3形で分割結果が正しいことを決定9(b) のテストで固定する。

**半角 `+`（U+002B）と全角 `＋`（U+FF0B）は別物である**（レビュー指摘 m7）:
全角 `＋` は `is_all_fullwidth_ascii` を満たし `classify_fullwidth` 経由で
`KeySequence("+")` になる（区切り文字としては機能しない）。Issue の実レイアウト
には `'＋'CV4D`（全角）が実在し、日本語 IME ユーザーが全角 `＋` を区切りの
つもりで打つ誤用が起きやすい。`lint_raw_cell` に「全角 `＋` を含み半角 `+` を
含まないセル」への注意喚起を追加することを実装時に検討する（決定9(b)）。

**新規構文であることの確認**: 半角 `+`（U+002B）は今日 `YabValue::parse` の
どの分岐にも該当せず最終フォールバック `Literal("+")` になる（同梱
`layout/*.yab` に半角 `+` を含むセルは無いことを確認済み）。したがって
「今日 `+` を含むセルがどう解釈されていたか」との衝突は無い。

### (b) 名前付きマクロ（再利用・将来拡張、`config.toml`）

`AppConfig`（`src/config.rs:588-601`）に、既存の `keymaps: Vec<KeymapRule>` と
同じ並びで追加する（実物の derive に合わせ `Default` は付けない、レビュー指摘 m1）:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    // ...(既存フィールド)
    #[serde(default)]
    pub keystroke_macro: Vec<KeystrokeMacro>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct KeystrokeMacro {
    /// マクロ名（`.yab` セルから `@name` で参照する）
    pub name: String,
    /// 順序付きの出力ステップ列。非ネスト（`@` 参照はマクロ内で禁止、
    /// 決定2c）。各要素は `.yab` の1トークンと**全く同じ文字列表記**
    /// （"'（'"/"CV4D"/"左" 等）。空リスト、または全要素が許可リストで
    /// 拒否された場合はバリデーションエラーにせず、そのマクロは
    /// `YabValue::None`（明示的な無出力）に解決される（レビュー指摘
    /// M1/m6、決定2c/決定3 参照）。
    pub steps: Vec<String>,
}
```

**各ステップは `YabValue::parse` へそのまま渡す**（レビュー指摘 M1/Critical2/C3
を受けた変更——r2/r3 は `MacroStep` という別 enum を用意していたが、
`.yab` の語彙にそのまま揃えた時点でタグ付き enum は不要になっている。
`YabValue::parse` は既に `pub fn`（`yab/mod.rs:84`）なので、`SPECIAL_KEYWORDS`
や `parse_ctrl_vk` 等のモジュール私有ヘルパーを `pub(crate)` に昇格させる必要も、
新しい `resolve_macro_step` 関数を `src/yab/` 側に置く必要も無い——**既存の
公開関数を再利用するだけ**で解決できる）:

```toml
[[keystroke_macro]]
name = "bracket_paren"
steps = ["'（'", "CV4D", "'）'", "CV4D", "左"]
```

**マクロステップとして許可する `YabValue` variant は
`Literal`/`KeySequence`/`Special`/`CtrlChord` の4種のみ**とする。
`YabValue::parse(step)` の結果が `Vk`/`Romaji`/`None`/`InlineSequence`/
`MacroRef` のいずれかであれば、そのステップは**警告付きでスキップ**する
（決定3 の警告経路にそのまま載る）:

- `Vk` を禁止する理由は決定6 と同じ（生の `KeyAction::Key` は
  `OutputHistory` の KeyUp 整合性索引と衝突する、コンテキスト事実6）。
- `MacroRef` を禁止する理由はマクロの入れ子・循環参照を構造的に防ぐため
  （決定4 の非ネスト不変条件をマクロ経由で破らせない）。
- `None` は `.yab` の `無` をマクロの1ステップに書く実用上の需要が無く、
  v1 では単純に対象外とする。
- **`Romaji` を禁止する理由は機械的なタイミング問題である**（レビュー
  指摘 m2——旧稿は「実用上の需要が無い」という弱い表現だったが、これは
  誤解を招く。決定2c の `InlineSequence` は同じ `Romaji` を許可している
  ため「需要が無い」わけではない）: `KeystrokeMacro.steps` は
  `resolve_keystroke_syntax` の中で初めて `YabValue::parse` に通される
  ため、`resolve_kana`（`.yab` 読み込み直後に1回だけ走る）より後になる。
  `KanaTable` をマクロ展開時に渡す設計にしない限り、マクロ由来の
  `Romaji` は `kana` が永久に `None` のまま残り、`KeyAction::Romaji`
  （VK バッチ送信）に落ちて単体セルと注入経路が変わる。決定2c の
  `InlineSequence` は `resolve_kana` の時点で既に構築済みのためこの
  問題が起きず、`Romaji` を許可できる（詳細は決定2c 参照）。

利点: (a) `String` のみなので `AppConfig::save()`（`config.rs:668`
`toml::to_string_pretty(self)`）の往復でユーザーの手書き TOML が変質しない、
(b) 表記が `.yab` セルと**完全に同じ**語彙になり学習コストが下がる、
(c) 不正表記の検出が決定3 の警告経路（`Vec<String>` 戻り値）に自然に載る、
(d) core の基本型（`VkCode`/`SpecialKey`）に serde 依存を持ち込む判断を
避けられる、(e) タグ付き enum を廃したことで実装コード自体が既存の
`YabValue::parse` を呼ぶだけになり、新規ロジックの余地がほぼ無い。

`.yab` 側は `YabValue::parse` に新しい分岐を1つ足すだけでよい:

```rust
if let Some(name) = trimmed.strip_prefix('@') {
    if is_valid_macro_name(name) {          // Unicode識別子(英数字/かな/漢字/_/-)
        return Self::MacroRef(name.to_string());
    }
}
```

マクロ名は `!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() ||
matches!(c, '_' | '-'))` ベースとし、日本語名（`@括弧ペア` 等）も許す
——「(b) 表現力」の優先順位に照らして禁止する理由が無い（レビュー指摘
Minor6、未解決の疑問2 を決定へ格上げ）。**`!name.is_empty()` を明記する**
（レビュー指摘 M4——`.all()` は空文字列に対し vacuously true を返すため、
このガードが無いと `@` 単独セルが `MacroRef("")` になり、`On` のとき
「マクロ @ が見つかりません」という警告と `None` を生む。`parse_direct_vk`
が `hex.is_empty()` を、`parse_function_key` が `digits.is_empty()` を
弾いているのと同じ配慮）。

### (a)/(b) の合成、および `InlineSequence`/マクロ steps 双方への許可リスト適用

**r4 からの変更（レビュー指摘 Critical C1/C2、批判役）**: r4 は
「`InlineSequence` の1セグメントが `@name` であってもよい…この合成は
追加コード無しで成立する」と書いていたが、これは2点で誤っていた。

**(1) `InlineSequence` の要素に決定2b の許可リストが適用されていなかった**
（C1）。決定2b は `KeystrokeMacro.steps` に対してのみ「`Literal`/
`KeySequence`/`Special`/`CtrlChord` の4種以外は警告付きスキップ」という
制約を書いていたが、`resolve_keystroke_syntax`（決定3）の `InlineSequence`
解決は要素を「それ以外はそのまま」通していた。`.yab` の `+` 区切りは
`YabValue::parse` の全既存分岐（`Vk`/`機`+数値含む）をそのまま通すため、
`'あ'+V1D` のようなセルが `InlineSequence([Literal("あ"), Vk(0x1D)])` →
`Sequence([Char('あ'), Key(0x1D)])` になりうる。これは決定6 が名指しで
禁じた「後から `KeyUp` を期待される片方だけの `Key` 送信」そのものであり、
`OutputHistory` の KeyUp 整合性索引と衝突して stuck key を招く（BUG-101 /
ADR-112 再発ファミリー）。`Vk` は架空ではない——Issue #118 の実レイアウト
`[拡張親指シフト1]` に `VF2,VF0,V1D,V1C` が実在する。

**(2) `InlineSequence` の要素が `MacroRef` のとき、マクロの steps を
`Sequence` に包んだまま埋め込むと `Sequence` がネストする**（C2）。
`'。'+@confirm` を素直に解決すると `Sequence([Literal("。"),
Sequence([KeySequence("."), CtrlChord{..}])])` のように内側の `Sequence`
が残り、決定4 の非ネスト不変条件（「内側の要素に `Sequence` は現れない」）
と、決定5 の `flatten_actions`（1階層しか展開しない設計）の両方が破れる。

**修正**: `resolve_keystroke_syntax` の `InlineSequence` 解決を次のように
確定させる（`InlineSequence` は decision2b の許可リストに `Romaji`
（後述の理由により許可）を加えた5種のみを最終的な要素として許す）。

```rust
// InlineSequence { items, .. } の各要素 item を1つずつ処理し、
// Vec<YabValue> へ「平坦に」積む（ネストを作らない）。
match item {
    YabValue::Literal(_) | YabValue::KeySequence(_)
    | YabValue::Special(_) | YabValue::CtrlChord { .. }
    | YabValue::Romaji { .. } => resolved.push(item.clone()),
    YabValue::MacroRef(name) => match macros.iter().find(|m| m.name == *name) {
        Some(m) => resolved.extend(resolve_macro_steps(&m.steps, &mut warnings)),
        // 見つからなければ、このステップだけを無かったことにする（単体
        // MacroRef セルの「セル全体が YabValue::None になる」動作、決定3
        // 参照、とは非対称——`InlineSequence` の一要素が未定義マクロを
        // 指すだけで列全体を捨てるのは過剰と判断した。どちらも決定0 の
        // 寛容フォールバック方針の表れであることを明記する、レビュー
        // 指摘 m3）。
        None => warnings.push(format!("マクロ @{name} が見つかりません")),
    },
    // Vk/None は決定6 と同じ理由で禁止。InlineSequence（ネスト）は
    // parse_cell が単一階層しか作らないため構造的に到達しないが、
    // &YabValue の網羅 match を満たすため防御的に同じ扱いにする
    // （レビュー指摘 m3）。Sequence も同様に到達しない
    // （YabValue::parse は Sequence を返さない、レビュー指摘 Minor2）。
    YabValue::Vk(_) | YabValue::None
    | YabValue::InlineSequence { .. } | YabValue::Sequence(_) => {
        warnings.push(format!("打鍵列の要素として使えない値です: {item:?}"));
        // このステップを無かったことにする（列全体は失敗させない、
        // 決定0 の寛容フォールバック方針を踏襲）
    }
}
// 決定後の要素数で結果の形を決める（レビュー指摘 M1/Minor2/Minor3）:
//   0要素 → YabValue::None（「明示的な無出力」を表す既存の値に合わせる。
//     MacroRef 未定義時と挙動が揃う）。
//   1要素 → Sequence で包まず、その要素をそのまま返す（`'あ'+V1D` の
//     ように Vk だけが拒否されて Literal("あ") だけが残るケースが、
//     単体 'あ' セルと完全に同じ挙動——kana Some('あ') を含む——になる。
//     `Sequence` に包んだままだと lookup_face の kana 抽出が `_ => None`
//     に落ちてしまい、誤字セルの副作用が本来のセルより悪化する）。
//   2要素以上 → YabValue::Sequence(resolved)。
let result = match resolved.len() {
    0 => YabValue::None,
    1 => resolved.into_iter().next().expect("checked len == 1"),
    _ => YabValue::Sequence(resolved),
};
```

`InlineSequence` が `Romaji` を許すのは `KeystrokeMacro.steps` と異なる
判断である。**理由は両者とも同じ機械的な事実——「`resolve_kana` より後に
展開されると kana が永久に `None` のまま残る」——だが、その事実が
`InlineSequence` には当てはまらない**（レビュー指摘 m2、旧稿は
`KeystrokeMacro.steps` 側の理由を「実用上の需要が無い」という別の弱い
表現で書いており、将来「需要が出たから許可しよう」と誤って緩められる
余地があった。表現を統一する）。`InlineSequence` は決定2a の
`resolve_kana` 修正（後述）で `resolve_kana` の**時点で既に構築済み**
であり `Romaji.kana` が正しく解決されるため許可できる。
`KeystrokeMacro.steps` は `resolve_keystroke_syntax` の中で初めて
`YabValue::parse` に通されるため `resolve_kana` より後であり、
`KanaTable` を渡す設計にしない限り解消できない——マクロ展開順序を
入れ替えるか展開時に `KanaTable` を渡さない限り `Romaji` を許可しては
ならない、という歯止めとして機能する。

**`Romaji` を許可したことで、そのセルは n-gram 文脈・同時打鍵の kana
依存閾値調整から外れるという副作用が残る**（レビュー指摘 M2、既知の
制約として明記する）: `lookup_face`（`nicola_fsm.rs:715-719`）の kana
抽出は `Sequence` に対して常に `None` を返す（決定7参照）。したがって
`ｋａ` 単体は kana `Some('か')` だが `ｋａ+CV4D` は kana `None` になり、
`recent_kana`（n-gram 文脈、`nicola_fsm.rs:652`）と
`is_simultaneous(.., candidate_kana)`（`:1200-1203`）の両方から除外
される。決定7 の「打鍵列は主に句読点・記号キーに付くので実用上の
劣化は軽微」という評価は、`Romaji` を対象に含めたことで前提が変わって
いる——実際に kana を生む `か` 等のキーが対象になりうる。`lookup_face`
の kana 抽出を `Sequence` の第1要素まで降ろす拡張（決定4 の非ネスト
不変条件により1階層で完結するため実装は容易）は v1 では見送り、
既知の制約として記録するに留める（決定0 の優先順位 (a) 安定性を
優先し、実装範囲を広げない）。

決定9(b) のテストに `'あ'+V1D`（`Vk` 拒否、警告付きスキップ）・
`'。'+@confirm`（`Sequence` が非ネストで展開されること）・
`'V1D'+'V1C'`/空の `steps`（全要素拒否 → `YabValue::None` になること）
を追加する。

**`YabFace::resolve_kana` を `InlineSequence.items` に1階層だけ降ろす**
（レビュー指摘 Major2）: `YabFace::resolve_kana`（`yab/mod.rs:307-317`）は
トップレベルの `Romaji` しか見ておらず、`ｋａ+CV4D` のような `InlineSequence`
内の `Romaji` は `kana: None` のまま残っていた。呼び出し順序
（`bootstrap.rs:693-695` の `YabLayout::parse` → `resolve_kana()` の直後に
`resolve_keystroke_syntax` を置く、決定3）により `InlineSequence` は
`resolve_kana` の時点で既に構築済みなので、この取りこぼしは確実に発生する:

```rust
pub fn resolve_kana(&mut self, table: &KanaTable) {
    for value in self.values_mut() {
        match value {
            YabValue::Romaji { romaji, kana } => *kana = table.kana_for_romaji(romaji),
            // InlineSequence の要素も解決する（決定4 の非ネスト不変条件により
            // 1階層で完結する。Sequence は resolve_kana より後に作られるため対象外）
            YabValue::InlineSequence { items, .. } => {
                for it in items {
                    if let YabValue::Romaji { romaji, kana } = it {
                        *kana = table.kana_for_romaji(romaji);
                    }
                }
            }
            _ => {}
        }
    }
}
```

これを怠ると、`ｋａ+CV4D` の第1要素が `Romaji{kana:None}` のまま
`impl From<&YabValue> for KeyAction` の `kana:None` 分岐（`nicola_fsm.rs:65`、
`Self::Romaji(romaji.clone())`、VK バッチ送信）に落ち、単体の `ｋａ` セル
（`Char` による Unicode 直接注入）と注入経路が変わってしまう。決定9(b) に
「`ｋａ+CV4D` の第1要素が `Romaji{kana:Some('か')}` になり
`KeyAction::Char('か')` に変換されること」をテストとして追加する。

**使い分けの指針**（ドキュメント/README 相当、実装には影響しない）: 単発・
局所的な列（Issue の主用途である句読点確定）はセル内 `+` 区切りで書く。
複数キーで再利用する列、または将来 `Wait`/マウス操作を持つ列（決定11）は
名前付きマクロで書く。

---

## 決定3: 新構文（`CtrlChord`/`InlineSequence`/`MacroRef`）は読み込み時に1回だけ解決し、キルスイッチを統一的にゲートする

**r2 からの変更（レビュー指摘 Major1/Major5、両者一致）**: r2 は `MacroRef` だけを
解決パスの対象にし、`CtrlChord` はゲート対象外（常に有効）としていた。その結果
既存のやまぶき派生ファイルに含まれる `CV41`（今日は無害な `Literal→Char('C')`）が、
`keystroke_sequence` を有効化していないユーザーでも無条件に Ctrl+A（全選択、
破壊的操作）へ変わってしまう非対称な設計だった。また r2 が挙げた呼び出し箇所
（`runtime/mod.rs`）は実際には解決を挟める場所ではなく（後述）、`.yab` を
パースしている本番経路は `LayoutEntry::scan_all` の1箇所に閉じている
（レビュー指摘 Major5）。r3 はこの2点を修正する。

`YabLayout::parse` の**後**、`AppConfig` が確定した**後**に、新しい解決パス関数を
1つ追加する。対象は `CtrlChord`/`InlineSequence`/`MacroRef` の**3種すべて**:

```rust
/// `.yab` レイアウト中の新構文（CtrlChord/InlineSequence/MacroRef）を、
/// キルスイッチとマクロ定義に基づいて確定させる。呼び出しは
/// `LayoutEntry::scan_all` 内・awase-settings のプレビュー生成時の
/// 各1箇所のみ。
pub fn resolve_keystroke_syntax(
    layout: YabLayout,
    macros: &[KeystrokeMacro],
    policy: KeystrokeSequencePolicy,
) -> (YabLayout, Vec<String>) {  // 戻り値2要素目は警告メッセージ一覧
    // 全6面を YabFace::values_mut()（既存、yab/mod.rs:265-267）で走査し、
    // セルごとに:
    //
    //  policy == Off:
    //    CtrlChord{raw,..}/InlineSequence{raw,..} → YabValue::Literal(raw)
    //    （保持しておいた元のセル生テキストをそのまま使う——レビュー指摘
    //    Critical3/C3。r3 は YabValue::serialize() で「逆写像」を作ろうと
    //    したが、serialize() は parse() の厳密な逆写像ではない
    //    （クォート種別の非保持・16進のゼロ詰め落ち・空白の正規化等）ため
    //    「Off なら無変化」という保証そのものが壊れていた。raw を素通しに
    //    すれば非可逆変換を経由しないため厳密に無変化になる）。
    //    MacroRef(name) → YabValue::Literal(format!("@{name}"))
    //    （name は strip_prefix の結果そのままなので、これは元のセル
    //    テキストと厳密に一致する——raw を別途持つ必要が無い）。
    //    警告は出さない（静かに「機能を使わなかったことにする」）。
    //    例: CtrlChord{vk:VK(0x41), raw:"CV41"} → Literal("CV41")
    //    → 今日どおり Char('C')。
    //
    //  policy == On:
    //    - CtrlChord{vk,..} はそのまま(解決済み)。
    //    - InlineSequence{items,..} は決定2c の許可リスト
    //      （Literal/KeySequence/Special/CtrlChord/Romaji）で各要素を
    //      フィルタし、MacroRef 要素は resolve_macro_steps(&m.steps,
    //      &mut warnings) の結果を「平坦に」extend する（Sequence で
    //      包まない——決定2c、非ネスト不変条件を守るため）。Vk/None/
    //      InlineSequence（ネスト）は警告付きでそのステップだけ
    //      スキップする。決定2c と同じ「0要素→None／1要素→そのまま
    //      返す／2要素以上→Sequence」規則で差し替える（レビュー指摘
    //      M1/Minor2/Minor3）。
    //    - MacroRef(name) は macros から name を検索し、見つかれば
    //      resolve_macro_steps(&m.steps, &mut warnings) の結果を同じ
    //      規則（0要素→None／1要素→そのまま／2要素以上→Sequence）で
    //      差し替える。見つからなければ YabValue::None + 警告
    //      "マクロ @name が見つかりません"。
}

// resolve_keystroke_syntax / resolve_macro_steps は src/yab/mod.rs
// （または同モジュール配下の新規ファイル）に置く。src/config.rs は
// crate::yab を一切参照していないため（レビュー指摘 M3、grep で確認済み
// ——`config` → `yab` の依存が存在しない）、resolve_keystroke_syntax の
// シグネチャが KeystrokeMacro/KeystrokeSequencePolicy（crate::config）を
// 引数に取る形にして `yab` → `config` の一方向依存に留める。逆方向
// （`config` 側に置いて `yab` を import する）は循環にはならないが、
// YabLayout/YabValue の内部構造を最も詳しく知っている `yab` 側に置く
// 方が自然。

/// KeystrokeMacro.steps（Vec<String>、決定2b）を YabValue の列へ変換する。
/// InlineSequence 解決（決定2c）と MacroRef 単体解決の両方から呼ばれる
/// 共通ヘルパー——許可リストの判定をここ1箇所に集約する。
fn resolve_macro_steps(steps: &[String], warnings: &mut Vec<String>) -> Vec<YabValue> {
    steps.iter().filter_map(|s| match YabValue::parse(s) {
        v @ (YabValue::Literal(_) | YabValue::KeySequence(_)
            | YabValue::Special(_) | YabValue::CtrlChord { .. }) => Some(v),
        // Romaji はここでは常に拒否する（決定2c 参照——マクロ展開は
        // resolve_kana より後に走るため kana が永久に None のまま残る）。
        // 代替手段を警告文に含める（レビュー指摘 Minor4——動いていた
        // セル内 + 区切りをマクロに移した瞬間に静かに1ステップ落ちる、
        // という使い勝手の崖を警告文だけで埋める）。
        YabValue::Romaji { .. } => {
            warnings.push(format!(
                "マクロのステップにローマ字は書けません: {s:?}。\
                 セル内 `+` 区切り（例: `ｋａ+CV4D`）を使ってください。"
            ));
            None
        }
        other => {
            warnings.push(format!("マクロステップとして使えない値です: {s:?} ({other:?})"));
            None
        }
    }).collect()
}
```

**`YabValue::parse` のシグネチャは変えない**（r1 決定8/レビュー指摘
M5/Minor13 が指摘した「純粋関数へのポリシー引数の波及」を回避する）。config を
必要とするのは新しい解決パスのみで、既存の `YabValue::parse` 呼び出し
（62箇所、`src/yab/tests.rs` 含む。レビュー指摘 m1——旧稿の「168箇所超」は
根拠不明な数字だったため実測値に訂正）は一切変更しない。

**呼び出し箇所は本番経路1つ＋テスト1箇所**（レビュー指摘 Major5/M5——r3 は
「呼び出し元2箇所」としていたが、実際は3箇所ある）: `.yab` をパースしている
本番経路は `LayoutEntry::scan_all`（`crates/awase-windows/src/app/bootstrap.rs:693`
の `YabLayout::parse` 呼び出し、直後に `yab.resolve_kana()`）だけであり、
呼び出し元は起動時（`bootstrap.rs:170`）とホットリロード
（`crates/awase-windows/src/app/mod.rs:667`、r3 が「`runtime/mod.rs`」と
誤記していた箇所の訂正）の2箇所——どちらも `config` を手元に持つため
`scan_all` へ `&[KeystrokeMacro]`/`KeystrokeSequencePolicy` を足す変更は
機械的に通る。**加えて `bootstrap.rs:1099` のテストが `config` を持たずに
`scan_all` を呼んでいる**ため、実装時にテスト側の呼び出しへ `&[]`/`Off`
を渡す対応が必要（fan-out に含める）。`resolve_kana()` の直後に
`resolve_keystroke_syntax` を適用する。警告は新しい表示機構を作らず、
既存の `StartupDiagnostics::warn`（`bootstrap.rs:702, 707` で「レイアウト
読込失敗」に既に使われている）に載せる。

`crates/awase-settings/src/main.rs` はプレビュー表示用の一時コピーに対しての
み `resolve_keystroke_syntax` を適用する（決定9(a) 参照）。**編集対象のデータ
は `CtrlChord`/`InlineSequence`/`MacroRef` のまま保持する**（未解決のまま
`.yab` へ書き戻せば無損失往復になる、決定9(a)）。

**エンジンのホットパス（`NicolaFsm::lookup_face` 等）は `YabValue::Sequence`/
`CtrlChord` だけを見る。`InlineSequence`/`MacroRef` は理論上到達しない**
——防御的に、`impl From<&YabValue> for KeyAction` のそれぞれのアームは
`unreachable!()` を使わず、`log::error!` + 安全な既定（`Suppress`）とする:

```rust
YabValue::InlineSequence { .. } | YabValue::MacroRef(_) => {
    log::error!("[yab] 未解決の新構文がエンジンに到達した — resolve_keystroke_syntax の呼び出し漏れ: {value:?}");
    Self::Suppress
}
```

（[ADR-104](104-observation-freshness-and-hardening.md) の
「型で保証されない `unreachable!` の除去」に整合。**決定9(a) の
`serialize()` にも同じ方針を適用する**——`panic!`/`unreachable!()` は
どちらも実装しない、後述）。

---

## 決定4: `YabValue::Sequence(Vec<YabValue>)` / `KeyAction::Sequence(Vec<KeyAction>)` は r1 のまま維持する

```rust
pub enum YabValue {
    // ...(決定1・決定2 の新 variant含む)
    /// 打鍵列。**不変条件: 内側の要素に `Sequence` は現れない**
    /// （`resolve_keystroke_syntax` が `resolve_macro_steps()` の結果を
    /// 常に `extend`（平坦化）で積み、`Sequence` で包んで埋め込むことを
    /// しないため——r4 は `InlineSequence` 内の `MacroRef` 要素を
    /// `Sequence` のまま埋め込んでおりこの不変条件を破っていた、
    /// レビュー指摘 Critical C2、決定2c で是正）。
    Sequence(Vec<YabValue>),
}
```

r1 の提案役レビューが「設計の骨格——`Sequence(Vec<Self>)` という一般化は Issue の
実例（5要素）に照らして正しい」と評価した部分はそのまま維持する。r1→r2→r3 で
変わるのは**この `Sequence` がどこから来るか**（r1: セル内トークナイザ／r2:
`MacroRef` のみ／r3: `InlineSequence`+`MacroRef` の合成、決定2・決定3）だけで
あり、`Sequence` そのものの型・非ネスト不変条件・`KeyAction` への変換
（`impl From<&YabValue> for KeyAction` に1行追加）は変更しない。

---

## 決定5: `SmallVec → Vec` 変換を1つのヘルパーに集約し、全ての変換点で `Sequence` を展開する

**r2 からの変更（レビュー指摘 C1、両者一致）**: r2 は平坦化ポイントを
「`decide()` の3箇所 + `build_response()` の1箇所 = 4箇所」と個別列挙したが、
これは2重に誤っていた——`decide()`（`nicola_fsm.rs:854-882`）の `into_vec()` は
実際には**2箇所**（`Reduce`/`ReduceAndContinue`、`Shift`/`PassThrough` は
actions を持たない）であり、かつ **`flush_pending`（`:375-457`）内の3箇所
（`:388, 412, 432`）が完全に欠落していた**。`flush_pending` は
`ContextChange::{ImeOff, InputLanguageChanged, EngineDisabled, LayoutSwapped,
FocusChanged, BypassKey}` ——**フォーカス変更・IME OFF・エンジン無効化という
異常系パス**——で呼ばれ、`resolve_pending_char_as_single`（`:1305`）や
`resolve_char_thumb_as_simultaneous`（`:768`）の結果をそのまま
`Response::emit` する経路であり、**`Sequence` が確実に通る**。「個別に
列挙する」というアプローチ自体が、r1→r2 と2回連続で見落としを生んだ
（存在しない API → 不完全な列挙）ため、r3 はアプローチを変える。

**正しい平坦化ポイントは `SmallVec<[KeyAction; 2]> → Vec<KeyAction>` へ変換して
いる箇所**であり、これを1つのヘルパー関数に**集約**し、`.into_vec()` の
呼び出しを**全てこのヘルパー経由に置き換える**:

```rust
/// `SmallVec` 中の各 `KeyAction` を、`Sequence` なら中身を展開し、
/// それ以外はそのまま1要素として並べた `Vec` に変換する。
/// 決定4 の非ネスト不変条件により、この展開は1階層で完結する。
/// `.into_vec()` を直接呼ぶ箇所を作らないこと——この関数が唯一の変換点。
fn flatten_actions(actions: SmallVec<[KeyAction; 2]>) -> Vec<KeyAction> {
    let mut out = Vec::with_capacity(actions.len());
    for action in actions {
        match action {
            KeyAction::Sequence(items) => out.extend(items),
            other => out.push(other),
        }
    }
    out
}
```

置き換え対象は `grep -n "actions.into_vec()\|resolved.actions.into_vec()"
src/engine/nicola_fsm.rs` で機械的に列挙できる（`decide()` の2箇所、
`build_response()` の1箇所、`flush_pending` の3箇所、計6箇所。この grep
コマンド自体を決定9(b) のテスト/CI チェックに含め、将来 `.into_vec()` の
直接呼び出しが新設されたら気付けるようにする）。

**この変更で、`lookup_face` の呼び出し元14箇所（うち10箇所がアクションを積む側）は
1つも触らない。** `smallvec![action]`/`ResolvedAction{ actions, .. }` は今日と同じ
`SmallVec<[KeyAction; 2]>`（要素数1、`Sequence` はそのまま1要素として入る）を組み
続け、`flatten_actions` を呼ぶ出口だけが `Sequence` を知っている。

---

## 決定6: `OutputHistory`（KeyUp 整合性索引）は `Sequence`/`CtrlChord` を安全に扱えるようにする

コンテキスト事実6・r1 レビュー Critical3/C3/C4 の核心。`OutputUpdate::record(scan,
&action, kana)`（`fsm_types.rs:301-308`）は決定5 とは**別経路**で、9箇所の呼び出し元
それぞれから直接呼ばれ、`action`（`Sequence` を含みうる）をそのまま
`OutputHistory.pending_releases`（`output_history.rs:71-75`）に格納する。

**対策は2段構え:**

1. **生の `Key(vk)`/`KeyUp(vk)` を許可しない——`KeystrokeMacro.steps`
   だけでなく `InlineSequence` の要素にも同じ制約を課す**（レビュー指摘
   Critical C1、r4 は `KeystrokeMacro.steps` にしかこの制約を適用しておらず、
   `'あ'+V1D` のようなセル内 `+` 区切り経由で `Vk` が `Sequence` に混入
   しうる穴が残っていた）。各ステップ文字列・各セグメントを `YabValue::
   parse` した結果が `Literal`/`KeySequence`/`Special`/`CtrlChord`（＋
   `InlineSequence` に限り `Romaji`、決定2c）以外（`Vk`/`None`/
   `InlineSequence`（ネスト）/`MacroRef`（マクロ steps の場合のみ禁止、
   `InlineSequence` の場合は平坦展開を許す、決定2c）」であれば警告付きで
   スキップする（決定2b/決定2c、決定3）。`CtrlChord` は決定1で自己完結
   （1回の `SendInput` 呼び出し内で press/release が閉じる）と決めた。
   したがって **`Sequence` の要素が「後から `KeyUp` を期待される片方だけの
   `Key` 送信」を含むことは、マクロ経由・セル内 `+` 区切り経由のどちらでも
   構造的に無い**。
2. **それでも `pending_releases` に `Sequence`/`CtrlChord` を持つエントリが積まれた
   場合の3つの消費点に、明示的な match アームを追加する**（r1 が「網羅性で守られる」
   と誤って安心していた箇所、コンテキスト事実6参照）:

   - `append_key_up_for`（`nicola_fsm.rs:819-823`）: `Some(KeyAction::Key(vk))` 以外
     （`Sequence`/`CtrlChord` 含む）は何もしない、を**明示的コメント付きで**維持。
   - `release_only`（`nicola_fsm.rs:1880-1894`）: 現状の危険な既定
     `_ => Response::pass_through()`（`Char`/`Romaji` は明示的に `Suppress` される
     のに未知 variant は物理 KeyUp が OS へ素通しされる非対称、r1 レビュー C4 が
     指摘）を撤去し、**網羅 match** に書き換える（レビュー指摘 Major4——
     r2 の `Response::consume_no_actions()` は実在しない API であり、単純な
     `Response::consume()` への置換は既存アームが持つ `TimerIntent::CancelAll`
     を落として保留中タイマーを残す事故を招くため、既存アームへの合流という
     形にする）:

     ```rust
     KeyAction::Char(_) | KeyAction::Romaji(_)
     | KeyAction::Sequence(_) | KeyAction::CtrlChord(_)
     | KeyAction::SpecialKey(_) | KeyAction::KeySequence(_) | KeyAction::Suppress =>
         self.build_response(smallvec![KeyAction::Suppress], true, TimerIntent::CancelAll),
     KeyAction::Key(vk) =>
         self.build_response(smallvec![KeyAction::KeyUp(vk)], true, TimerIntent::CancelAll),
     KeyAction::KeyUp(_) => Response::pass_through(),
     ```

     **これは打鍵列とは独立に存在した既存のバグであり、本 ADR の実装と同じ
     コミットで修正する。** 網羅 match にすることで、将来 `KeyAction` に
     variant が増えた際にコンパイラが更新漏れを検出するようになる（r1/r2
     レビューが繰り返し指摘した「網羅性で守られない箇所」を1つ塞ぐ）。
   - `drain_pending_releases_as_keyups`（`output_history.rs:196-204`）: 現状の
     `_ => None`（黙って捨てる）のままでよい——`Sequence`/`CtrlChord` は解放すべき
     片割れを持たないため、これは正しい既定（コメントで理由を明記する）。

**なぜ `pending_releases` の型自体（`Vec<(ScanCode, KeyAction)>`）を拡張しないのか**
（却下案参照）: `Vec<(ScanCode, SmallVec<[KeyAction;2]>)>` への拡張は ADR-112 が
固めたばかりの不変条件を触ることになり、コストに見合わない。対策1（生 `Key` を
マクロに入れさせない）で実害を根本から断てるなら、型を変える必要は無い。

**`KeyAction::romaji()`（`src/types.rs:278-284`）の非網羅性は許容し、
既知の制約として記録するに留める**（レビュー指摘 m5——r1〜r5 で5ラウンド
連続の指摘のため、本 r6 で対応の要否を確定させる）:

```rust
pub fn romaji(&self) -> &str {
    if let Self::Romaji(s) = self { s } else { "" }
}
```

網羅 `match` ではないため、`Sequence`/`CtrlChord` を足してもコンパイラが
検出しない。`OutputUpdate::record`（`fsm_types.rs:304`）が `romaji:
action.romaji().to_owned()` として `OutputEntry` に入れるため、
`Sequence([Romaji("kya"), ..])` の `romaji` フィールドは空文字列になる。

**対応しない**。理由: `OutputEntry.romaji` は `pending_releases` 側では
参照されず（`output_history.rs:44-47` のコメント）、`committed` 側でも
n-gram は `kana`（別フィールド、こちらは `Sequence` に対して常に `None`
になることを決定7 で扱い済み）だけを見て `romaji` は見ない。**この関数を
網羅 match に書き換えても、動作上の差分がどこにも生まれない**——決定6
が `release_only` を網羅 match 化したのは KeyUp 漏れ・タイマー保留という
実害があったからだが、`romaji()` には対応する実害が無い。対応すると
不要な変更を増やすだけなので、決定0 の「安定性優先＝挙動を変えない
変更を増やさない」という精神からも見送るのが正しい。今後同じ指摘が
出た場合は本段落を参照して打ち切ってよい。

**根拠の追記（レビュー指摘 m4）**: `OutputEntry.romaji`
（`output_history.rs:14`）→ `KeyAction::romaji()` の連鎖は、production
コードでは書かれるだけで一度も読まれない（`pending_releases` は
`(ScanCode, KeyAction)` のみを持ち KeyUp 整合性側は `romaji`/`kana` を
参照しない、`output_history.rs:44-47` のコメントが明記。`committed` 側の
読み出しは `recent_kana`/`display_text` のどちらも `kana` のみを見る）。
この dead code 自体は本 ADR の対象外だが、[ADR-045](045-dead-field-detection-policy.md)
（dead-field detection policy）の対象として別途整理すれば、`KeyAction`
に variant を足すたびにこの関数を気にする必要そのものが消える
（未解決の疑問に追記）。

---

## 決定7: 投機出力（`SpeculativeChar`）への入口を単一の関門に集約し、`Sequence` を発火させない

r1 決定6（同時打鍵組合せからの除外）は撤回する（コンテキスト事実2・r1 レビュー
Critical1 が指摘した通り、Issue の実例が全て親指シフト面にあり、除外すると
到達不能になるため）。**`resolve_thumb_face`/`thumb_shift_face_defines`/
`resolve_char_thumb_as_simultaneous` には一切手を入れない。**

**r2 からの変更（レビュー指摘 Critical1/M2、両者一致）**: r2 は
`on_timeout_speculative`（`TwoPhase`/`AdaptiveTiming` 経由）だけにガードを
入れたが、`SpeculativeChar` への入口は**もう1つ**ある——
`confirm_policy.rs:50-74` の `idle_speculative`（`ConfirmMode::Speculative`
および `NgramPredictive` 経由、`idle_ngram` の `should_speculate` 分岐から
呼ばれる）が、`face = Face::Normal` で `lookup_face` した結果をその場で
投機送信し `enter_speculative_char` を呼ぶ、全く同型の処理を行う。r2 の
ガードはここを素通しにしていた。

**修正**: ガードを2箇所に個別に置くのではなく、**両方の入口が呼ぶ共通関数
`enter_speculative_char` 自身に移す**（1関門への集約。将来3つ目の入口が
増えても構造的に守られる）。

**r3 からの変更（レビュー指摘 Critical1、両エージェント一致）**: r3 は
「呼び出し元は戻り値 `false` の場合は投機を諦め通常経路（`go_idle()` +
`pass_through()` 等、既存の非投機フォールバックと同じ形）にする」とだけ
書いていたが、**両呼び出し元の既存 `else` 分岐は「Normal 面にそもそも
定義が無い」ケース専用であり、「打鍵列だから投機しない」ケースに流用すると
入力が失われる**:

- `on_timeout_speculative`（`nicola_fsm.rs:2007-2024`）の既存 `else` は
  `self.go_idle(); Response::pass_through().with_kill_timer(TIMER_SPECULATIVE)`
  ——これは**タイマー満了イベントへの応答**なので `pass_through()` は
  「何も出力しない」を意味する。`Sequence` に流用すると、既に消費済みの
  KeyDown に対応する出力が一切生成されず、**打鍵列がそのまま消える**。
- `idle_speculative`（`confirm_policy.rs:59-73`）の既存 `else` は
  `ParseAction::PassThrough { timer: TimerIntent::Keep }`
  ——これは**KeyDown イベントへの応答**なので `pass_through()` は
  「リマップせず生の物理キーを OS へ渡す」を意味する。`Sequence` に流用すると
  **生の VK がそのままアプリに届く**（打鍵列の代わりに素の文字が出る）。

**正しいフォールバックは入口ごとに異なる**:

```rust
// nicola_fsm.rs:812 付近。`const` を外し、戻り値を bool にする
// （既に引いた action を受け取ることで二重 lookup_face を避ける——
// 呼び出し元の `face` 変数と本関数がハードコードする面がズレる余地も消える）。
/// 投機出力の開始を試みる。`Sequence`（複数出力・複数 composition unit）は
/// retract_bs_count が前提とする「BACKSPACE 1発で取り消せる」性質を持たない
/// ため拒否する（決定7）。呼び出し元は戻り値 false を「投機せず、
/// PendingChar のまま Wait モード相当の確定を待つ」として扱うこと
/// （`go_idle()`+`pass_through()` は使わない——打鍵列の消失・生VK漏洩を招く、
/// 後述）。
pub(crate) fn enter_speculative_char(&mut self, key: PendingKey, action: &KeyAction) -> bool {
    if matches!(action, KeyAction::Sequence(_)) {
        return false;
    }
    self.state = EngineState::SpeculativeChar(key);
    true
}
```

- **`on_timeout_speculative`**（呼び出し元は既に `(action, kana)` を
  `lookup_face` 済み）: `false` のとき、**`PendingChar` を維持したまま
  actions 無しで `TimerIntent::Phase2Transition { remaining_us }` を返す**
  （`build_response(SmallVec::new(), false, Phase2Transition{..})` ——
  `consumed=false` とし、既存の「レイアウト未定義キー」用 `else` 分岐
  （`pass_through()`）と同じ `consumed` 値に揃える。`fsm_adapter::
  response_to_decision`（`:239-250`）は `(false, false) =>
  Decision::pass_through_with(effects)` を持つため、`consumed` の値に
  関わらずタイマー effect（`Kill{TIMER_SPECULATIVE}`+`Set{TIMER_PENDING,
  remaining_us}`）は失われない、レビュー指摘 m4）（r4 からの変更、
  レビュー指摘 M1——r4 は「その場で確定出力する」として
  いたが、これは `TimerIntent::CancelAll` を使うため**打鍵列セルの同時打鍵
  受付窓を `simultaneous_threshold_ms`（Wait モード相当）から
  `speculative_delay_ms`（既定30ms、`config.rs:358`）に縮めてしまう**。
  投機**成功**時（`:2019`）が同じ `Phase2Transition { remaining_us }` で
  TIMER_PENDING を残り時間ぶん張り直しているのと対称に、投機**を諦める**
  ときも同じ機構で `PendingChar` の残り時間をそのまま活かす。これにより:
  - TIMER_PENDING が本来の満了時刻に達すれば、既存の `on_timeout`
    （`:2235`）→ `mem::replace` → `timeout_pending_char`（`:1916-1920`）が
    確定を行う——**確定ロジックを決定7 側で手書き複製する必要が無くなる**
    （Wait モードと確定経路が構造的に1本に揃う）。
  - 満了前に親指キーが来れば、`PendingChar` のままなので
    `step_pending_char_thumb` が chord 判定を行う——r4 で失われていた
    同時打鍵受付窓がそのまま保たれる。
- **`idle_speculative`**（`ConfirmMode::Speculative`/`NgramPredictive`）:
  `false` のとき、`self.idle_wait(ev)`（同関数が Shift 押下時・親指キー時に
  既に使っている既存の退避先、`confirm_policy.rs:38-47,51-57`）へ落とす。
  **`idle_wait` は `ParseAction::Shift { timer: TimerIntent::Pending }` を
  返し、TIMER_PENDING を満額 `threshold_us` で張る**（レビュー指摘
  Minor1——r5 は「`on_timeout_speculative` と同じ経路（`Phase2Transition`
  再設定）を通る」と書いていたが誤り。`TimerIntent::SpeculativeWait`
  （TIMER_SPECULATIVE を張る唯一の Intent）を設定するのは `TwoPhase`
  経路（`confirm_policy.rs:95`）だけであり、`idle_speculative` はそもそも
  TIMER_SPECULATIVE を経由しないため `on_timeout_speculative`/
  `Phase2Transition` を通らない）。後段は `on_timeout`（`:2231`）→
  `timeout_pending_char`（`:1916`）へ直行して確定する——**満額の
  `threshold_us` を張るため、chord 受付窓は `on_timeout_speculative`
  経由の場合よりもむしろ広い**。結論（打鍵列が欠落しない、受付窓が
  `simultaneous_threshold_ms` である）自体は変わらないため、機構の
  記述だけを訂正する。

決定9(b) のテスト表に「`ConfirmMode::Speculative`/`TwoPhase` の両方で、
Normal 面が `Sequence` のセルを押すと打鍵列が欠落せず出力されること」に加え、
「chord 受付窓が非 `Sequence` セルと同じ `simultaneous_threshold_ms` である
こと（`speculative_delay_ms` に縮んでいないこと）」を追加する（既定の
`ConfirmMode::Wait` ではこの経路自体を通らないため、オプトイン設定に限定
される実害だが、決定0 の優先順位(a)「安定性」に照らして省略しない）。

理由（背景）: `on_timeout_speculative`（`nicola_fsm.rs:2006-2032`）・
`idle_speculative` はいずれも Normal 面の `lookup_face` が成功すれば
無条件に投機送信して `SpeculativeChar` 状態に入るが、その後の取消は
`retract_and_replace`（`:1120-1139`）が**常に BACKSPACE 1発**を仮定して
いる（`retract_bs_count`、`output_history.rs:93-99`、「完結したローマ字は
1 composition unit になるため常に1」という前提）。打鍵列（複数出力・複数
composition unit）にはこの前提が成立しない。

これが本 ADR に必要な**唯一**の「組合せ判定への介入」であり、r1 決定6 が
試みた「chord 解決からの除外」より遥かに狭い（1関数、Normal 面限定）。

決定5 の帰結として `Sequence` の kana 先読み値は常に `None` になる（`lookup_face`
の既存の仮名抽出は `Romaji`/`Literal` のみ `Some` を返し、`Sequence`/`MacroRef`/
`CtrlChord`/`InlineSequence` はどれにもマッチしないため、コード変更なしで自然に
`None` になる）。これにより n-gram 文脈と同時打鍵の kana 依存閾値調整
（`is_simultaneous` 等）から打鍵列セルは除外される——実害は「その次のキーの
タイブレーク精度が僅かに落ちる」程度で、r1 決定6 のように機能そのものを
潰すものではない。

---

## 決定8: `KeystrokeSequencePolicy` は既定 Off、`CtrlChord`/`InlineSequence`/`MacroRef` を統一的にゲートする

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum KeystrokeSequencePolicy {
    #[default]
    Off,
    On,
}
```

`GeneralConfig::keystroke_sequence`（`src/config.rs`）として追加する。

**r2 からの変更（レビュー指摘 Major1）**: r2 は `CtrlChord` をゲート対象外に
していたが、これは既存のやまぶき派生ファイルに偶然含まれる `CV41`
（今日は無害な `Literal→Char('C')`）を、`keystroke_sequence` を有効化して
いないユーザーでも無条件に Ctrl+A（全選択、破壊的操作）へ変えてしまう
設計であり、決定0 の優先順位(a)「安定性」に反していた。r3 は**3種の新構文
（`CtrlChord`/`InlineSequence`/`MacroRef`）全てを同じポリシーでゲートする**。

**`.yab` パーサ自体（決定1・決定2 の `CV`/`+`/`@` 認識）はポリシーを見ない**
——常に新 variant を認識する（`YabValue::parse` はコンテキストフリーの
純粋関数のまま）。ポリシーがゲートするのは決定3 の**解決パス
（`resolve_keystroke_syntax`）のみ**: `Off` のとき、`CtrlChord`/
`InlineSequence` は保持している `raw`（決定1・決定2）から、`MacroRef` は
`format!("@{name}")` から `YabValue::Literal` に差し替えられ、**元の
セル生テキストと厳密に一致する状態に復元される**（`serialize()` は経由
しない——レビュー指摘 M5、r4 の決定3・決定9(a) は既に `raw` 直接参照に
修正済みだったが、本決定8 の説明文だけ r3 時点の `Literal(value.
serialize())` という表現が残っていた。挙動としては結果が一致するため
矛盾ではなかったが、実装者が本決定だけを読んで非可逆な `serialize()`
経由のコードを書く事故を避けるため、決定3 と同じ表現に揃える）。
例: `CV41` は `CtrlChord{vk:VK(0x41), raw:"CV41"}` → `Literal("CV41")` →
今日どおり `Char('C')`。

**既存 `YabValue::parse` を素通しにする**（config を一切スレッドしない）ことで、
決定8 が抱えていた「純粋関数へのポリシー波及」問題（レビュー指摘 M5/Minor13）が
構造的に発生しない——config を必要とする分岐は決定3 の新規関数1つに閉じている。

---

## 決定9: 証拠義務・fan-out・GUI/serialize

### (a) `YabValue::serialize()` と awase-settings の往復保全

`YabValue::serialize()`（`src/yab/mod.rs:154-176`）に4行追加する。
**`panic!`/`unreachable!()` は使わない**（決定3 が `MacroRef`/`InlineSequence`
の未解決到達に採った方針と同じ理由——`serialize()` は `pub fn` であり、
到達しないことは型で保証されていない。`awase-settings` のプレビュー生成
（決定3、解決済み `YabLayout` を表示専用コピーに対して作る）で
`YabLayout::serialize`（`yab/mod.rs:287-304`）が呼ばれれば実際に
到達しうる。レビュー指摘 Major3/M4 を反映し、安全側に倒す）。
**`CtrlChord`/`InlineSequence` は決定1/決定2 で保持している `raw`
フィールドをそのまま返すだけでよい**（`format!` で逆写像を再構成しない
——`CV0D` のゼロ詰め落ち等、非可逆になる箇所が無くなる、レビュー指摘 C3）:

```rust
Self::CtrlChord { raw, .. } | Self::InlineSequence { raw, .. } => raw.clone(),
Self::MacroRef(name) => format!("@{name}"),
Self::Sequence(_) => {
    log::error!("[yab] 解決済み Sequence を .yab へ serialize しようとした（プレビュー専用コピーのはず）");
    "無".to_string()
}
```

`awase-settings` は `.yab` の読み込み・保存の**両方で `resolve_keystroke_syntax`
を呼ばない**——GUI が扱う `YabLayout` は常に未解決（`CtrlChord`/
`InlineSequence`/`MacroRef` のまま）を維持する。プレビュー表示（実際に
何が起きるかを見せる機能）が必要な場合のみ、表示専用の一時コピーに対して
`resolve_keystroke_syntax` を適用する。これにより:

- `.yab` の読み込み→保存の往復で `@name`/`CV4D`/`+` トークンが1バイトも
  変化しない（`CtrlChord`/`InlineSequence` は保持した `raw` をそのまま
  返し、`MacroRef` は `name` から厳密に再構成できるため、`Sequence` を
  経由しない限り往復の劣化が構造的に起こらない）。
- `crates/awase-settings/src/main.rs` の `YabValue::` 網羅 match（`718-740`,
  `3503-3529`, `3535-3540`, `3547-3562` 等、64箇所参照）には
  **`CtrlChord`/`InlineSequence`/`MacroRef`/`Sequence` の4アーム**を
  追加する（レビュー指摘 Major2——r2 は「`Sequence` は GUI 層に届かないため
  対応不要」としたが、これらの match はワイルドカード無しの網羅形であり、
  到達可能性に関わらず**コンパイルを通すために4アーム全てが必須**）。
  表示ロジックが実質必要なのは `CtrlChord`（例: `Ctrl+M`。`raw` からではなく
  `vk` から都度整形する）と `MacroRef`（例: `@bracket_paren`）と
  `InlineSequence`（保持している `raw` をそのまま表示する）の3つで、
  `Sequence` は `None` と同じ表示（`—`）にする。

**GUI での新構文 authoring は非対象であり、無損失往復は「未編集セル限定」**
（レビュー指摘 M3）: `awase-settings` がセル編集時に `ValueKind` から
`YabValue` を直接構築する経路（`main.rs:786-824`）には `CtrlChord`/
`InlineSequence`/`MacroRef` を作る分岐が無い。ユーザーが
`ValueKind::Literal` のテキスト欄に `'．'+CV4D` と直接入力すると
`YabValue::Literal("'．'+CV4D")` になり、保存時に `serialize` が
`format!("'{s}'")` で二重クォートするため内容が変わる。決定0 が GUI
編集を非対象としているため設計判断としては許容するが、「1バイトも
変化しない」という無損失往復の保証は**そのセルを GUI 上のテキスト欄で
編集しない限りにおいて**成立する、という限界を明記する。

**`raw` の不変条件・等値比較・メモリ影響**（レビュー指摘 m3/m4）:
`raw` は `parse_cell`/`YabValue::parse` の時点で確定し、以後変更しない
（`vk`/`items` と `raw` の二重管理を避けるため）。`YabValue` は
`#[derive(Debug, Clone, PartialEq, Eq)]`（`yab/mod.rs:18`）なので `raw`
は等値比較に参加する——意味的に同じ2セル（`'．'+CV4D` と `'．' + CV4D`）
は `raw` が異なるため `!=` になる。本番ロジックが `YabValue` を `==` で
比較する箇所は無い（`nicola_fsm.rs:2072` は `!matches!(v,
YabValue::None)` のみ）ため実害は無いが、決定9(b) のテストで
`assert_eq!` する際は `raw` まで厳密に一致させる必要がある。メモリ影響は
無視できる（`InlineSequence`/`CtrlChord` 1値あたり `String` 1個分の
増加、レイアウト1つ（6面・最大52セル×6）あたり数KB程度）。

### (b) 自動テスト

| 対象 | 置き場所 | 固定する内容 |
|---|---|---|
| `parse_ctrl_vk`（決定1） | `src/yab/tests.rs` | `CV4D` → `CtrlChord{vk:VkCode(0x4D), raw:"CV4D"}`。`V4D`（Ctrl無し）とは区別されること。`CV`単独・16進不正は非該当（フォールバック） |
| `split_unquoted_plus`/`parse_cell`（決定2a） | `src/yab/tests.rs` | `'．'+CV4D` → 2セグメント。クォート内の `+` は分割されないこと。`+` を含まないセルは今日と同じ結果を返すこと。先頭/末尾/連続 `+`（空セグメント）は分割されず単一トークン扱いになること。エスケープ実例3形（`"\""` `"\'"` `"\\"`）で分割が正しいこと |
| `cell_segments` 経由の `lint`（決定2a） | `src/yab/tests.rs` | `lint` が `cell_segments` の判定に揃えてからセグメント単位（または `None` ならセル全体）で `lint_raw_cell` を呼ぶこと。`+` を含むセルでもクォート不整合の警告が検出されること。`parse_cell` の分割結果と `lint` の検査単位が常に一致すること |
| `resolve_kana` の `InlineSequence` 対応（決定2c） | `src/yab/tests.rs` | `ｋａ+CV4D` の第1要素が `Romaji{kana:Some('か')}` になり `KeyAction::Char('か')` に変換されること（単体 `ｋａ` セルと同じ注入経路になることの回帰） |
| `InlineSequence`/マクロへの許可リスト適用（決定2c） | `src/yab/tests.rs` | `'あ'+V1D` が `Vk` 要素を警告付きスキップすること（stuck key の回帰）。`'。'+@confirm` の解決結果が非ネストの `Sequence`（`Sequence` の要素に `Sequence` が現れない）になること。`'V1D'+'V1C'`（全要素拒否）と `steps=[]` のマクロが `YabValue::Sequence(vec![])` ではなく `YabValue::None` に解決されること（M1/Minor3） |
| `@name` 認識（決定2b） | `src/yab/tests.rs` | `@bracket_paren`/`@括弧ペア` → `MacroRef(..)`。空文字列（`@` 単独）はフォールバックすること（レビュー指摘 M4）。不正な識別子もフォールバック |
| `resolve_keystroke_syntax`（決定3） | `src/yab/tests.rs`（関数本体は `src/yab/mod.rs`、決定3 参照。レビュー指摘 m1——旧稿はここで「新規 `src/yab/macro_resolve.rs`」と書いており決定3 の記述と食い違っていた） | `Off`→3種の新variantが全て `raw`（`CtrlChord`/`InlineSequence`）または `format!("@{name}")`（`MacroRef`）から `Literal` に復元され、**元のセル生テキストと完全一致**すること（`CVXY+CV4D`/`あ+い`/`CV0D` 等の非全角・ゼロ詰め崩れケースを含む）。マクロ未定義→`None`+警告。定義あり→`Sequence`に展開、非ネスト（`Vk`/`Romaji`/`None`/`InlineSequence`/`MacroRef` を含むマクロステップは警告付きスキップ）。`InlineSequence`内の`MacroRef`要素も**平坦に**解決されること（`Sequence`のネスト無し） |
| `YabValue → KeyAction` 変換 | `src/engine/nicola_fsm.rs` 既存テスト群 | `Sequence`/`CtrlChord` の変換。到達済み`InlineSequence`/`MacroRef`の防御アーム |
| 平坦化（決定5） | `src/engine/nicola_fsm.rs` または `fsm_types.rs` | `flatten_actions` を通した `Sequence` が N要素になること。`flush_pending`（3箇所）経由でも同様に展開されること |
| `OutputHistory`（決定6） | `src/engine/output_history.rs` | `release_only` が網羅 match になり `Sequence`/`CtrlChord` が `Suppress`・`TimerIntent::CancelAll` になること（物理KeyUp素通し修正・タイマー保留の回帰） |
| 投機出力ガード（決定7） | エンジン既存テスト | `enter_speculative_char(key, action)` が `Sequence` で `false` を返すこと。`idle_speculative`（`ConfirmMode::Speculative`/`NgramPredictive`）は `false` で `idle_wait(ev)` に落ち `TimerIntent::Pending`（満額 `threshold_us`）が発行されること（生VK漏洩の回帰）。`on_timeout_speculative`（`TwoPhase`/`AdaptiveTiming`）は `false` で `PendingChar` を維持し `TimerIntent::Phase2Transition{remaining_us}` を返すこと（打鍵列消失の回帰）。どちらのモードも満了時に既存 `timeout_pending_char` 経由で確定すること、満了前に親指キーが来ると `step_pending_char_thumb` で chord 判定されること（`simultaneous_threshold_ms` の受付窓が非`Sequence`セルと同じであること、`speculative_delay_ms` に縮んでいないことの回帰） |
| serialize往復（決定9a） | `src/yab/tests.rs` | `@name`/`CV4D`/`'．'+CV4D`/`CV0D`（ゼロ詰め）/`"．"+CV4D`（ダブルクォート）セルの parse→serialize→parse が**元のセル生テキストと完全一致**すること |
| `lint_raw_cell` | `src/yab/tests.rs` | 決定1・決定2 追加後も `ｂ'ｕ` 等クォート不整合の警告が変化しないこと |
| `AppConfig` 往復 | `src/config.rs` 既存テスト群 | `keystroke_macro` を含む TOML の `save()`→`load()` 往復でマクロ定義（`steps: Vec<String>`）が1バイトも変化しないこと |
| 平坦化点の網羅性 | CI スクリプトまたは `#[test]` | `nicola_fsm.rs` 内で `flatten_actions` の定義行を除いて `.into_vec()` の直接呼び出しが0件であること（決定5 は全呼び出しをこのヘルパー経由に統一する方針のため、期待値は6ではなく0——r3 の記述誤りを訂正） |

### (c) 自動テストで代替できないもの

- `CtrlChord`（Ctrl+M 等）が MS-IME / GJI 双方で実際に意図通り動作すること
  （報告者の実運用実績はあるが、awase 自身の送信経路での確認は必要）。
- `"ctrl_chord"` ステップを含むマクロが Chrome/Windows Terminal 等の
  TSF-native アプリで正しい順序・タイミングで届くこと。
- 打鍵列セル押下時にユーザーが物理的に Shift/Alt を押している場合の
  `CtrlChord` の実際の挙動（決定1、既知の限界として記録済み。実害が出た場合は
  `ime.rs` の `push_release`/`push_restore` パターンの流用を検討する）。

`docs/known-bugs.md` に本機能専用のエントリを立て、実機確認結果を記録する
（着手前に `docs/experiments.md` へ事前登録エントリも立てる、
[experiment-logging](../../.claude/rules/experiment-logging.md)）。

---

## 決定10: `awase-macos`/`awase-linux` の `send_keys` にも新 variant のアームを追加する（CI 必須）

`crates/awase-macos/src/output.rs:100-123` と `crates/awase-linux/src/output.rs:117-156`
は `KeyAction` を網羅 match する `send_keys` を持つ（CI は
`cargo nextest run --workspace --lib` で両クレートをコンパイルする、
`.github/workflows/ci.yml:46`）。`CtrlChord`/`Sequence` を追加した時点で
**両クレートがコンパイルエラーになる**——本 ADR の実装 PR に必ず含める。

両プラットフォームともスタブ実装で構わない（`log::debug!` + no-op、または
可能なら Windows と同型の press/release シーケンスを実装）。この修正自体は
機能追加ではなく「新 variant を足したことによるコンパイル通過の必須対応」として
扱う。

---

## 決定11: 将来の RPA 的拡張（マウス操作・待機）に向けた設計制約（本 ADR では実装しない）

ユーザーからの指示: 「将来的には簡単な RPA レベルまで発展させることもありえるので、
その点も考慮して破綻しないような設計を」。以下は**設計上の制約の明文化のみ**で、
実装は本 ADR のスコープ外（決定0）。

**核心の制約: 待機（sleep）はブロッキングであってはならない。** このコードベースは
シングルスレッド・メッセージループ駆動（`winmsg-executor`）で、キーフック・タイマー・
フォーカス検出は全て同一スレッドの `spawn_local` タスクとして動く。ブロッキング
API は `run_with_timeout`（`crates/win32-async/src/thread_timeout.rs`）で
ワーカースレッドへ隔離するのが本 repo の一貫した設計原則であり、
`send_keys`/`decide()` のようなホットパス内でブロッキング `sleep` を呼ぶことは
メッセージループ全体を止める——この repo の根本的なアーキテクチャ制約に違反する。

**したがって、将来 `Wait(Duration)` のようなステップを追加するとしても、それは
JavaScript の `setTimeout` と同じ「継続渡し」モデルで実装しなければならない。**
幸い、この repo の `timed_fsm::Response<A, T>` は最初から `actions: Vec<A>` と
`timers: Vec<TimerCommand<T>>` を同じ `Response` の中に持ち（`TimerCommand::Set`
でタイマーを仕掛け、期限が来たら `on_timeout` で再入する、という継続モデルを
フレームワークレベルで既にサポートしている、`crates/timed-fsm/src/response.rs:47-84`）、
warmup FSM 等で既に実例がある。

**r2 からの変更（レビュー指摘 M7）**: r2 は「`MacroStep`（別 enum）に `Wait` を
足すだけで済む」（設計指針1）と「`Wait` を含むマクロは別実行系にすべき」
（設計指針2）を同時に主張していたが、これは自己矛盾である——`MacroStep` が
単一 enum である限り、`Wait` を足した瞬間に同じ `KeystrokeMacro` 型の中に
「同期バッチで実行できるマクロ」と「継続モデルでしか実行できないマクロ」が
混在し、`resolve_keystroke_syntax`（決定3）が `Wait` を含むマクロをどう
`YabValue` へ写すかが型として表現できなくなる。r3 は型レベルで明確に分離する。
（r4 で `MacroStep` という別 enum 自体を廃止し `KeystrokeMacro.steps:
Vec<String>` に一本化したことで、この懸念はさらに強化される——`.yab` の
既存語彙をそのまま再利用する文字列表現に `Wait` のような新語彙を混ぜることは
`.yab` パーサ自体の文法を汚染することになり、なおさら避けるべきだと分かる。）

**設計上の帰結（本 ADR が保証すること）**:

1. **`KeystrokeMacro.steps`（決定2b、`Vec<String>`、`.yab` と同じ語彙）に
   `Wait`/マウス操作系の表記は追加しない。** 本 ADR が実装する同期ステップ
   （`Literal`/`KeySequence`/`Special`/`CtrlChord` に対応する4種の文字列
   表記）専用の語彙として固定し、将来にわたって「瞬時に完了する」という
   不変条件を保つ。
2. **将来 `Wait`/マウス操作が必要になったら、`KeystrokeMacro` とは
   別の新しい設定概念（仮称 `KeystrokeRoutine`/`RoutineStep`）を新設する。**
   これは本 ADR の `Sequence`/`send_keys` 一括送信モデルとは異なる実行系になる
   ——`Sequence` は「1回の `send_keys` 呼び出しの中で即座に全ステップを実行する」
   という**同期バッチ**モデルであり、`Wait` を含まない（=瞬時に完了する）
   ステップ列にしか安全に使えない。`RoutineStep::Wait` を含む列は、
   **残りステップを状態として保持し、`TimerCommand::Set` でタイマーを仕掛け、
   `on_timeout` で残りを再開する、専用の小さな FSM**（既存の `tsf/warmup/` 系
   FSM 群と同型のパターン）として別途設計する。`KeyAction::Sequence` を拡張して
   無理に表現しないのは、同期モデルと非同期継続モデルを1つの型に混在させると
   「今この `Sequence` は同期的に完結するか、それとも待機を含むか」を呼び出し側が
   事前に判定できなくなり、決定6/決定7 が前提にしている「`Sequence` は常に
   1回の `send_keys` で完結する」という不変条件が壊れるためである。
3. 本 ADR は上記1・2を「型レベルの設計指針」として記録するに留め、
   `RoutineStep`/`Wait`/マウス操作の実装そのものは着手しない。着手する際は
   別 ADR を起票し、本決定11 を前提として引用すること。

---

## 却下した代替案

- **r1 決定2 のセル内トークナイザ（区切り文字なし連結）をそのまま維持する**:
  やまぶき互換性を最優先すればこの形になるが、ユーザーの優先順位転換（決定0）と
  r1 レビューが指摘した実装不能な曖昧さ・既存レイアウト破壊リスクにより採用しない。
- **r2 の「名前付きマクロのみ、区切り文字は使わない」を維持する**: 採用理由
  「同じ打鍵列を複数キーで再利用できる」が Issue #118 の実データ（33セル中29が
  単発の2要素で再利用が効かない）で反証されたため、r3 で決定2 をセル内 `+` 区切り
  との併用に変更した。
- **`ConfirmIfComposing`（composing 観測に基づく条件付き確定）を v1 でも実装する**:
  コンテキスト事実1（実際の実運用は無条件 Ctrl+M）・事実7（観測経路が存在しない）
  により、v1 では実装コストに見合わない。ADR-109 を保留のまま残し、将来必要になれば
  参照する。
- **`pending_releases` の型を `Vec<(ScanCode, SmallVec<[KeyAction;2]>)>` に拡張する**
  （決定6 の対案）: ADR-112 の不変条件を触るコストに対し、「マクロに生の `Key` を
  含めない」という制約で同じ安全性を達成できるため不要。
- **`YabValue::parse` にポリシー引数を追加してキルスイッチを実現する**（r1 決定8 の
  ままの案）: 62箇所の呼び出し元（レビュー指摘 m1、旧稿の「168箇所超」は根拠不明）
  に波及し、かつグローバル/thread-local 化は
  BUG-65 の既往（`probe_bridge.rs:68-99`）と衝突する。決定3 の「解決パスだけを
  ゲートする」設計の方が影響範囲が小さい。
- **`CtrlChord` をキルスイッチの対象外にする**（r2 決定8 のままの案）: 既存
  ファイルの `CV41` 等が無言で破壊的操作（Ctrl+A 等）に変わる非対称を許容する
  ことになり、決定0 の優先順位(a)「安定性」に反するため r3 で撤回した。
- **`release_only` の `Sequence`/`CtrlChord` アームに専用の `Response` コンストラクタ
  （`consume_no_actions()` 等）を新設する**: 実在しない API を仮定した r2 の設計
  ミスであり、既存の `Char`/`Romaji` アーム（`TimerIntent::CancelAll` を伴う）へ
  合流させる方が正確かつタイマー処理の一貫性を保てる。
- **`MacroStep` を（`VkCode`/`SpecialKey` を `String` に変えただけの）タグ付き
  enum のまま維持する**（r3 決定2b のままの案）: `.yab` の語彙にフィールドを
  揃えた時点で、各バリアントは実質「`YabValue::parse` に通す1文字列」に
  収束しており、タグ（`type = "literal"` 等）は情報を増やしていなかった。
  r4 は `KeystrokeMacro.steps` を `Vec<String>` に単純化し、既存の
  `YabValue::parse`（既に `pub fn`）をそのまま再利用することで、
  `SPECIAL_KEYWORDS`/`parse_ctrl_vk` 等のモジュール私有ヘルパーを
  `pub(crate)` に昇格させる必要も、新しい解決関数を `src/yab/` に置く
  必要も無くした（レビュー指摘 M1）。
- **キルスイッチ Off 時の復元に `YabValue::serialize()` を使う**（r3 決定3 の
  ままの案）: `serialize()` は `parse()` の厳密な逆写像ではない（クォート
  種別の非保持・16進のゼロ詰め落ち・`InlineSequence` の空白正規化等）ため、
  `CVXY+CV4D`/`あ+い` 等のケースで「`Off` なら無変化」という保証そのものが
  崩れていた（レビュー指摘 Critical3/C3、両エージェント一致）。r4 は
  `CtrlChord`/`InlineSequence` に元のセル生テキストを `raw` として直接
  保持させ、非可逆変換を経由しない形に変更した。
- **`InlineSequence` にはマクロと同じ許可リストを課さない**（r4 決定3 の
  ままの案）: `KeystrokeMacro.steps` にだけ `Vk`/`None` 禁止を課し、
  `InlineSequence` の要素は「それ以外はそのまま」通していた。`+` 区切りは
  `.yab` の既存全分岐（`Vk`/`機`+数値含む）をそのまま通す設計（決定2a）で
  ある以上、この非対称は理論上の懸念ではなく `'あ'+V1D` のような実在
  可能なセルで stuck key を招く（レビュー指摘 Critical C1）。r5 は許可
  リストを `InlineSequence` にも適用する（決定2c）。
- **`InlineSequence` 内の `MacroRef` を `Sequence` のまま埋め込む**（r4
  決定2 の「(a)/(b) の合成」のままの案、「追加コード無しで成立する」と
  記していた）: これは決定4 の非ネスト不変条件を破る（レビュー指摘
  Critical C2）。r5 は `resolve_macro_steps()` の結果を `extend` で平坦に
  積む形に修正した（決定2c）。
- **`on_timeout_speculative` の guard-false 時に「その場で確定出力する」
  （`TimerIntent::CancelAll`）**（r4 決定7 のままの案）: 出力内容・タイマー
  kill は既存の Wait モード確定（`timeout_pending_char`）と等価だが、
  **確定タイミングが早まる**——投機**成功**時が `TimerIntent::
  Phase2Transition { remaining_us }` で残り時間を活かすのと非対称に、
  投機を諦める側だけ `speculative_delay_ms`（既定30ms）時点で確定して
  しまい、打鍵列セルの同時打鍵受付窓が `simultaneous_threshold_ms` から
  縮む（レビュー指摘 Major M1）。r5 は `PendingChar` を維持したまま
  `Phase2Transition` で残り時間を再設定する形に変更し、確定は既存
  `on_timeout` → `timeout_pending_char` の経路にそのまま委ねる（決定7）。

## 未解決の疑問（実装着手前後で確認すること）

実機確認が必要な項目（`CtrlChord` の MS-IME/GJI 動作、TSF-native アプリでの
タイミング、押下中の物理修飾キーとの衝突）は決定9(c) にまとめてある
（レビュー指摘 m8、`未解決の疑問` との重複を解消）。ここには実機確認以外の
疑問のみ残す:

1. **`resolve_keystroke_syntax` の警告（未定義マクロ参照等）を、どこに集約して
   表示するか** — 決定3 で `StartupDiagnostics::warn`（`bootstrap.rs:702, 707`）
   を使う方針にしたが、`awase-settings` 側のプレビュー生成時の警告表示先は
   実装時に既存の警告表示機構と整合させる。
2. **`KeyAction::romaji()`/`OutputEntry.romaji` が dead code かどうかの
   確認と、[ADR-045](045-dead-field-detection-policy.md) の対象として
   別途整理するかどうか**（決定6 参照、本 ADR のスコープ外）。
