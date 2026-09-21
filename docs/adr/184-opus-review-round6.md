# ADR-184 opus-adversarial-consult round6（収束判定つき）

対象: `docs/adr/184-gji-atok-muhenkan-toggle-awase-owned-eisu-hiragana.md`（v7）
前回: `docs/adr/184-opus-review-round5.md`（Blocker 2 / Must-fix 7 / Nits 4）
レビュー日: 2026-09-19 / ブランチ `feat/adr178-mode-key-actuation-and-tsfnative-rescue-teardown`（HEAD `51245736`）
方式: 読み取りのみ。v7 の設計を実コードの構造（`resolve_delegate_to_open_axis` の
文の順序、`ThumbSoloSpecialHandling` の構築、既存の書き込み規律）と突き合わせた。

---

## 収束判定（結論を先に）

**設計の技術的骨格は収束したと判断する。** U1 の別フィールド案の採用は
実コードの構造と整合しており（後述の V3〜V5 は「採用した案の詰め」であって
案そのものの否定ではない）、U2〜U9 も実質的に解消している。
**本ADRは、もう1ラウンドのフル敵対レビューを要しない。**

ただし**このまま実装に入るのは推奨しない**。残件は2種類ある:

- **(A) 文書の同期漏れ（Blocker 1件）**: `status` ブロックが v6 のまま
  更新されておらず、`summary` と本文2節に **v7 が撤回したはずの旧設計
  （「既存 `Toggle` 分岐の写像先を差し替える」「`ImeToggleKind` に情報源を
  保持する」）がそのまま残っている**。frontmatter は本リポジトリの規約上
  「本文を開かずに gist を得る正本」であり、ここが本文と逆のことを言って
  いる状態で確定すると、次のセッションが撤回済みの設計を実装する。
  **4ラウンド連続で frontmatter が本文に追随していない**（round2 R1、
  round4 Nit-1、round5 U4、今回 V1）ので、ここは機械的に潰しきること。
- **(B) 採用した案の配置詳細（Must-fix 3件）**: 新しい腕を置く位置が
  実コードの早期 return より後ろだと**デッドコードになる**（V3）、
  BUG-14 ガードとの前後関係が未指定（V4）、新フィールドの
  ペア書き規律と GJI 離脱時クリーンアップが未記載（V5）。いずれも
  「設計判断」ではなく「1〜2行で書き足せる仕様の穴」であり、
  実装時に気づけない類（V3 はテストが無ければ静かに不発、V5 は
  BUG-115 が実際に踏んだ再発）なので ADR に明記しておく価値がある。

**推奨する進め方**: (A)(B) を反映した v8 を作り、**フル敵対レビューではなく
差分確認（軽量な round7）**で確定する。以後は実装フェーズ
（実機確認項目は V2・V7 にまとめた）。

重大度の内訳: Blocker 1件 / Must-fix 5件 / Nits 4件。

---

## round5 指摘の対応状況

| # | round5指摘 | v7での対応 | 判定 |
|---|---|---|---|
| U1 | `ImeToggleKind`にorigin追加は判定点に届かず3消費者と衝突 | 別フィールド案を採用。既存Toggle分岐・MS-IME経路・ADR-176較正・`ImeToggleKind`型・BUG-115テストを一切変更しないと明記 | **対応済（良い変更）** |
| U2 | 情報源は4段／段3が実機に該当しうる | 4段の表に修正、段3を明記、実機検証結果（`session_keymap=1`、`custom_keymap_table`176行に該当行なし）を引用 | **対応済**（証拠の出所は未記載 → V2） |
| U3 | ヒント/ゲートの二枚舌・回復機構の有無 | 「ゲートである」「回復機構は持たない（較正ウィザード待ち）」と明記 | **対応済** |
| U4 | summary と背景で atok.tsv 矛盾の扱いが逆 | 両方「棚上げする」に統一 | **対応済** |
| U5 | 本文L174-176の誤ったADR-179引用 | 削除、summary/statusと同じ表現に統一 | **対応済** |
| U6 | ADR-182節に訂正済みのTODOが残存 | 「追加配線ではなく既存`:2409`ガードで自動的に成立」に訂正 | **対応済** |
| U7 | 引数名の3義化 | 本ADRの実装スコープに含めると明記（改名案つき） | **対応済** |
| U8 | InputRelayが動的値でコア静的structに載せられない | 毎イベント評価し`ClassifiedEvent`/`ImeRelevance`に載せる方式へ訂正 | **対応済** |
| U9 | Enter側のラッチ規律 | 設計8として明記（INV-D、commit-on-success） | **対応済** |
| Nit-a | 見出しのラウンド番号／疑問3が決着済み | 未対応 | **未対応**（Nit-i） |
| Nit-b | 手順1〜7がADR-182マージ前のログ | 未対応 | **未対応**（Nit-ii） |
| Nit-c | 疑問6（証拠のビルド特定） | 疑問のまま | **未対応**（Nit-iii） |
| Nit-d | composing節の空スタブ | 削除済み | **対応済** |

---

## Blocker

### V1. `status` が v6 のまま、`summary` と本文2節に v7 が撤回した旧設計が残っている

v7 は「別フィールド案を採用し、既存の `Toggle` 分岐には**一切手を触れない**」
（設計2 L322、設計原則 L263-273）に変わった。しかし以下は v5/v6 の
「既存 `Toggle` 分岐の写像先を差し替える」記述のままである。

| 箇所 | 現在の記述 | v7 の決定 |
|---|---|---|
| `status` L42-85 | 見出しが「**ドラフトv6**（round4指摘を…、round5前）」。T2の説明が「`classify_mode_key_ime_action`の戻り値に**情報源を保持**し…」（U1で撤回した案）。T3「（**未反映**）」・T4「（未反映箇所あり、round5で統一する）」・T5〜T10「round4時点で**未反映**——round5で反映する」 | すべて反映済み。別フィールド案。 |
| `summary` L34-36 | 「本設計はADR-179決定2が既に確立した`resolve_delegate_to_open_axis`の`Toggle`分岐の**写像先（open軸→conv軸）を差し替えるだけ**」 | 既存 `Toggle` 分岐は**変更しない** |
| `summary` L37-41 | 「本設計は**情報源を保持し**ATOKプリセット由来のToggleのみを対象にする」 | `ImeToggleKind` に origin は持たせない（windows 層で判定し別フィールドで渡す） |
| 本文 L417-423（ADR-182節） | 「**v5 の設計変更**（…既存の `Toggle` 腕〈2246-2253行〉の**写像先を差し替えるだけ**）」「本ADRの変更点（`explicit_resolve()` が返す型を open 軸から conv 軸へ差し替える）」 | 同上 |
| 本文 L472-478（変換の扱い） | 「`resolve_delegate_to_open_axis` の `Toggle` 分岐は Henkan/Muhenkan を区別しない（`special.delegate_to_open_axis` は VK ごとに解決済みの値が入るだけ）。**つまり実装すると自動的に変換側にも効く**」 | 新フィールドは別物。Henkan/Muhenkan に**個別に**書くか否かは V5 の論点 |

**なぜ Blocker か**: `.claude/rules/docs-frontmatter-convention.md` は
frontmatter（`summary`/`status`）を「索引だけ読めば概要が分かる」ための
**正本**と定めている。ここが「既存 `Toggle` 分岐の写像先を差し替える」と
言い続けている限り、`docs/adr/index.md` 経由で本ADRに当たった次の
セッションは、**U1 が3つの理由で否定した設計**（origin がコア境界で消える／
MS-IME レジストリ経路を巻き込む／ADR-176 の永続化スキーマに触れる）を
そのまま実装しうる。これは `.claude/rules/experiment-logging.md` が
「なぜ前回それを捨てたのかが辿れないと同じ失敗を踏む」として防ごうとしている
パターンそのものである。

**要求**: `status` を v7 の内容（採用した案・各 T/U の解消状況・残件）へ
全面的に書き直し、`summary` L34-41 と本文 L417-423・L472-478 を
別フィールド案の記述に揃える。

---

## Must-fix

### V2. U2 の実機検証結果に出所（取得日時・方法・PID/ログ）が無い

設計原則 L240-245:

> **この会話の実機検証（round3で実施）で `session_keymap=1`（ATOK確定）
> かつ `custom_keymap_table`（176行）に素の無変換（Shift無し）への該当行が
> 無いことを確認済み**

この確認結果は本設計が発火するかどうかを直接決める最重要データだが、
ADR には**いつ・どうやって取得したかが書かれていない**。しかも同じ
フィールドの既存の記録2件と値が異なる:

| 記録 | 日付 | `session_keymap` |
|---|---|---|
| `docs/known-bugs/BUG-115.md`（config1.db を汎用protobufスキャナで検証） | 2026-09-05 | **0 (CUSTOM)** |
| `gji_charset_autodetect.rs:275-281` の ADR-174 実機検証コメント | 2026-09-15 | **2 (MSIME)** |
| 本ADR（round3 の会話） | 2026-09-19 | **1 (ATOK)** |

2週間で3回値が変わっている（ユーザーがプリセットを切り替えたのであれば
自然だが、記録が無いと後から検証できない）。BUG-115 と ADR-174 は
どちらも**取得方法つきで**コード/文書に残しているので、本ADRも同水準に
揃えること。auto-memory
`feedback_confirm_physical_key_and_config_source_before_remote_test`
（押したキー・configパス・PID/コミットを毎回確認してから結果解釈）にも
直接該当する。

**要求**: 取得日時・取得方法（clipwire 経由の config1.db ダンプ等）・
`custom_keymap_table` の該当行検索に使った条件（「素の無変換」＝
`Muhenkan` トークンで修飾なしの行、という定義）を ADR に1段落で残すこと。

### V3. 新しい腕の挿入位置が「冒頭」では曖昧で、実コードの早期 return より後ろに置くと**デッドコードになる**

設計2 L313-314:
> `resolve_delegate_to_open_axis` の**冒頭**（既存の `Toggle` 分岐より前）に
> この専用の腕を置く。

実コード（`src/engine/nicola_fsm.rs:2211-2222`）:

```rust
fn resolve_delegate_to_open_axis(
    special: &ThumbSoloSpecialHandling,
    injected: bool,
    composing: bool,
) -> DelegateResolution {
    let Some(open_axis_action) = special.delegate_to_open_axis else {   // :2216
        return DelegateResolution::Fallthrough(None);                    // :2217
    };
    let is_fake_injected_solo_tap = special.injected_guarded_delegate && injected;  // :2219
    if is_fake_injected_solo_tap {
        return DelegateResolution::Fallthrough(None);
    }
    ...
```

**本設計では段4のとき `delegate_to_open_axis` は `None` のままにする**
（設計1 L298-301）。したがって `:2216-2217` の早期 return が**必ず先に
発火する**。「既存の `Toggle` 分岐より前」という条件だけでは、
`:2219` 以降に置いても満たされてしまい、**新しい腕には永久に到達しない**。

しかも失敗の出方が悪い: コンパイルは通り、テストを書かなければ
「実装したのに何も起きない」という形でしか現れない。

**要求**: 次のどちらかを明記すること。
- (i) 新しい腕を **`:2216` の `let Some(...) else` より前**（関数の
  文字どおりの先頭）に置く。ただしその場合、関数名
  `resolve_delegate_to_open_axis` が実態と合わなくなる（open 軸専用の
  名前で conv 軸も解決する）ので、改名または doc コメントでの明記が要る。
- (ii) **`resolve_pending_thumb_as_single` 側**（`:2415` の
  `resolve_delegate_to_open_axis` 呼び出しの直前）に置く。
  ADR-182 のガード（`:2409`）より後ろなので U6 の「既存ガードで自動的に
  覆われる」性質はそのまま保たれ、`resolve_delegate_to_open_axis` には
  一切手を触れずに済む。**構造的にはこちらの方が素直**で、
  round3 S1 が評価した「既存決定点を壊さない」性質とも整合する。

### V4. 新しい腕と BUG-14 ガード（`injected_guarded_delegate`）の前後関係が未指定。しかも無変換/変換は現状このガードが無効

`ThumbSoloSpecialHandling` の構築（`nicola_fsm.rs:1000-1030`）:

| キー | `injected_guarded_delegate` | 行 |
|---|---|---|
| Muhenkan | **`false`** | :1005 |
| Henkan | **`false`** | :1013 |
| Hiragana | `true` | :1021 |
| Katakana | `true` | :1029 |

無変換/変換が `false` なのは、ADR-135 が
「対象VKが `VK_CONVERT`/`VK_NONCONVERT`——MS-IME/CTF が注入しないキー——
だから安全」と判断したため。**しかしこの判断が下された時点では、
無変換の delegate は `gji_thumb_key_ime_toggle` 既定 `false` により
到達不能で、実質 no-op だった**（本ADR自身が L219-223 で指摘している）。

本設計はこの経路を初めて実効化し、しかも **conv を能動的に書き換える**
actuation にする。一方 ADR-181 は、GJI 自身が `VK_DBE_HIRAGANA` を
`injected=false` で周期的に送ってくることを実機で記録している——
「このIMEはこのVKを注入しない」という種類の前提が、この環境では
実際に破れた前例がある。

**要求**:
1. 新しい腕を V3 の (i)(ii) どちらに置くにせよ、**`injected` ガードの
   後段に置く**（(i) を採るなら `:2219-2222` を新しい腕にも適用する、
   (ii) を採るなら `resolve_pending_thumb_as_single` が受け取っている
   `injected` 引数で同じ判定を行う）ことを明記する。
2. 無変換/変換の `injected_guarded_delegate` を `true` に変えるか、
   `false` のままでよい理由（ADR-135 の判断が本設計の文脈でも
   有効である根拠）を明記する。**BUG-14 は本リポジトリで最も高い代償を
   払った再発ファミリーの1つ**であり、「前は no-op だったから未検証で
   通っていた」ガードを実効化するときは、判断を書き直す価値がある。

### V5. 新フィールドの API 形状・ペア書き規律・GJI 離脱時クリーンアップが未記載

設計1 L296-298 は新しい setter を
`Runtime::set_gji_thumb_key_conv_toggle(Option<VkCode>)`（**単数**）と
書いているが、次の3点と噛み合っていない。

1. **Henkan/Muhenkan の両方が対象になる。** `ThumbSoloSpecialHandling` は
   VK ごとに組み立てられ（`:1000-1014`）、既存の setter は
   `set_gji_thumb_key_delegate_to_open_axis(henkan, muhenkan)` という
   **ペア**である（`runtime/mod.rs:1235-1241`）。新フィールドも
   `muhenkan_conv_toggle` / `henkan_conv_toggle` の2つ（またはペアを取る
   setter）にしないと、「変換の扱い」節（L470-478）の議論と接続できない。
2. **ペア全書きの規律。** ADR-135 Phase 2 は
   「**毎回2フィールドとも無条件に（`Some`か`None`を必ず代入する形で）書く**
   （Phase 1 の `set_gji_thumb_key_delegate_to_open_axis` と同じ
   『ペア全書き』規律）」と明記している。新フィールドも同じ規律に
   従わせること（片方だけ書くと、前回の値が残る）。
3. **GJI 離脱時のクリーンアップが必須。** BUG-115 の `/code-review` 指摘2
   （2026-09-05、実際に発生した欠陥）:

   > `sync_gji_charset_autodetect` の `!is_gji` クリーンアップは
   > Hiragana/Katakana 側の delegate/shadow override は解除するが、
   > 無変換/変換側の `gji_thumb_key_delegate_to_open_axis` は……解除して
   > いなかった。……**stale な GJI 由来 delegate が無期限に残留し、
   > 無関係なアプリでの単独タップが IME 状態を静かに反転させうる。**
   > 修正: クリーンアップブロックで
   > `set_gji_thumb_key_delegate_to_open_axis(None, None)` も明示的に呼ぶ。

   **新フィールドは同じ穴を新規に作る。** GJI から離脱した（あるいは
   IME 種別が未検出になった）後も `conv_toggle` が残っていると、
   MS-IME や非対応アプリでの無変換単独タップが
   `VK_DBE_ALPHANUMERIC` を送る。`!is_gji` クリーンアップブロックで
   新フィールドも `(None, None)` へ戻すことを ADR に明記すること。

### V6. 「変換（Henkan）の扱い」節が旧設計の前提のまま残っている（V1 の一部だが、決定が変わるので独立に挙げる）

L472-475:
> `resolve_delegate_to_open_axis` の `Toggle` 分岐は Henkan/Muhenkan を
> 区別しない（`special.delegate_to_open_axis` は VK ごとに解決済みの値が
> 入るだけ）。**つまり実装すると自動的に変換側にも効く**

別フィールド案では、**新フィールドに何を書くかは windows 層
（`sync_gji_charset_autodetect`）が決める**ので、「自動的に効く」のではなく
**明示的に書くかどうかの選択**になった。V5-1 の API 形状と合わせて、
「Henkan にも書く（＝両方に効かせる）」のか「Muhenkan だけに書く
（＝VK で限定する）」のかを**決定として書くこと**。
BUG-115 の理由2（「無変換/変換の両方が Toggle になり親指キー2本とも
IME 切替を持つことになり露出が2倍」）は、この選択で直接効く。

---

## Nits

### Nit-i（round5 Nit-a、未対応）. 見出しのラウンド番号と、決着済みの疑問

- L495「## 未解決の疑問（opus-adversarial-consult **round5** で検証してほしい点）」→ round6
- 疑問1「…が **round5** の検証対象」→ round6（または「実装時に確定」へ）
- 疑問3「`session_keymap` が ATOK 以外のとき……前提条件に含めるべきか、
  常時有効にすべきか」は、設計原則の4段の表＋「段4のみを対象とするゲート」
  で既に決着している。削除するか「段2・段3・その他は対象外」と結論だけ残すこと。

### Nit-ii（round5 Nit-b、未対応）. 「現状の機序」手順1〜7 が ADR-182 マージ前のログのまま

ADR-182 決定1/1b/1c が develop に入った現在、チョード誤判定経由の漏出は
再現しなくなっているはずで、残るのは「Idle 起点の正常な単独タップ」経由のみ。
1行足すと本ADRが何を直そうとしているかが明確になる。

### Nit-iii（round5 Nit-c、未対応）. 疑問6（証拠のビルド特定）

「記録として残すべきか」ではなく「手順3 の `send 0x001A` が現行コードでも
起きるのか」という実質的な問い。V2 の実機確認と同時に1回で済むので、
疑問から外して実装前タスク一覧へ移すこと。

### Nit-iv. 実装前の実機確認項目が本文中に散っている

現時点で ADR 各所に散らばっている「実装前/実装後に実機で確認すべきこと」は
次の5つ。1つの節にまとめると実装者が拾いやすい。

1. `session_keymap` 実値と `custom_keymap_table` の Muhenkan/Henkan 行の
   有無（設計原則 L244-245、V2）。
2. ATOK プリセット下で `VK_DBE_ALPHANUMERIC`/`VK_DBE_HIRAGANA` が
   conv 軸のみに効くこと（設計5 L361-362、round4 T2-a）。
3. 変換（Henkan）側の実挙動（「変換の扱い」節、V6）。
4. 手順3 の `send 0x001A` が現行ビルドでも起きるか（疑問6、Nit-iii）。
5. ADR-179 の `f0e36b0e` revert 判断との依存確認（設計1 L304-309、T10）。

---

## 収束判定の根拠（まとめ）

**収束したと判断する理由**:

- round1〜5 で指摘した**設計レベルの誤り**（診断の代替説明、軸の取り違え、
  挿入位置、型の置き場所、情報源の混在、動的値の扱い、commit 規律）は
  すべて解消または明示的な棚上げに至っている。
- v7 の別フィールド案は、**既存コードへの変更面積が最小**であり
  （既存 `Toggle` 分岐・MS-IME レジストリ経路・ADR-176 較正結果・
  `ImeToggleKind` 型・BUG-115 の約20本の回帰テストを一切触らない）、
  ADR-178 の撤去方針とも `fix-requires-evidence.md`「IME actuation 合流点」の
  「合流点を増やさない」要請とも整合する。
- 残る Blocker は**文書同期**であり、設計の是非ではない。
- 残る Must-fix（V3〜V6）は**採用した案の詰め**であり、いずれも
  「どちらを選ぶか」が1〜2行で書ける粒度に落ちている。

**それでも v8 を作ってから確定すべき理由**:

- V1（frontmatter が撤回済み設計を指し続ける）は、確定した瞬間に
  次セッションへの誤導が固定化する。4ラウンド連続の再発であり、
  ここで止めないと同じことが続く。
- V3 は「実装したのに何も起きない」という静かな失敗になる。
- V5-3（GJI 離脱時クリーンアップ）は BUG-115 が実際に踏んだ穴の
  完全な再演であり、ADR に書いておかないと同じ `/code-review` 指摘を
  もう一度受けることになる。

**次のアクション**: V1〜V6 と Nit-i〜iv を反映した v8 を作成 →
**差分確認のみの軽量 round7**（フル敵対レビュー不要）→ 確定 → 実装。
実装フェーズの実機確認は Nit-iv の5項目。
