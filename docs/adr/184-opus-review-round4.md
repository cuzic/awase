# ADR-184 opus-adversarial-consult round4

対象: `docs/adr/184-gji-atok-muhenkan-toggle-awase-owned-eisu-hiragana.md`（v5）
前回: `docs/adr/184-opus-review-round3.md`（Blocker 3 / Must-fix 10 / Nits 4）
レビュー日: 2026-09-19 / ブランチ `feat/adr178-mode-key-actuation-and-tsfnative-rescue-teardown`（HEAD `51245736`）
方式: 読み取りのみ。v5 の主張・行番号・ADR-182 のマージ後の実装をすべて実コードと git 履歴で裏取りした。

---

## 総評

**S1 の全面採用は正しく、v5 は設計として大きく前進した。**
「新しい機構を足す」から「既存の到達不能な決定点を正しい軸で到達可能にする」への
スコープ縮小は、ADR-178 の撤去方針とも `fix-requires-evidence.md` の
「合流点を増やさない」要請とも整合する。行番号 `2246-2253` は**実コードと
完全に一致している**（`src/engine/nicola_fsm.rs` の `Some(_)` 腕内 Toggle 分岐、
ADR-182 マージ後の現在の位置）。ADR-182 がマージ済みという事実の発見も正しい
（`740bf8be`、PR #225、BUG-145）。

**しかし round4 で Blocker 2件を検出した。両方とも「v5 が新たに置いた前提」
そのものが実リポジトリの状態と食い違っている。**

1. **T1**: v5 の緊急性の根拠（R4:「Passthrough は広く使われている設定で、
   実害が今出ている」）は、**同じ日にユーザー自身が ADR-179 へ記録した指示
   （`51245736`:「Passthrough 実験は当面残すが、develop へのマージ前に
   Suppress へ戻す」）と正面から矛盾する**。しかも v5 はその ADR-179 を
   「Passthrough 運用が広く行われている前提」の**根拠として引用している**
   （L159-161）——ADR-179 が書いていることの逆である。
2. **T2**: v5 の S2/S3 への回答（「actuation は conv 軸専用なので Toggle 判定が
   外れても実害は無い」）は成立しない。(a) 送る `VK_DBE_ALPHANUMERIC` は
   **awase 自身の静的マップが open 軸 `TurnOff` と分類している**キーであり、
   (b) `ImeToggleKind::Toggle` は **ATOK プリセット由来**と **CUSTOM literal
   トークン由来**（こちらは本物の open 軸 `IMEOn`/`IMEOff`）を区別せず1つの値に
   畳んでいるため、後者のユーザーの IME ON/OFF キーが機能停止する。

重大度の内訳: Blocker 2件（新規）/ Must-fix 8件（新規4・継続4）/ Nits 5件。

---

## round3 指摘の対応状況

| # | round3指摘 | v5での対応 | 判定 |
|---|---|---|---|
| S1 | 設計原則はADR-179決定2で既に確立済み／挿入位置が合流点を増やす | 全面採用。「設計原則」節を書き換え、「提案する設計」を(a)配線有効化+(b)写像先差し替えの2点に縮約、旧設計を撤回 | **対応済（良い変更）** |
| S2 | ゲートかヒントかが4箇所で矛盾 | 「ヒント」に統一したと宣言（status/設計原則） | **不十分**（Nit-1: 背景L118とsummaryL11-16がまだ第3・第4の立場） |
| S3 | `Toggle`判定を信用できる根拠がない／open軸語彙でconv軸を分類できない | 「conv軸専用actuationなので判定が外れても実害なし」で回避 | **不成立**（T2） |
| S4 | ADR-182対照テストは機序をassert／決定1b・1cとの関係 | マージ済みと判明、テスト追随修正を本ADRスコープに含めると明記、1b/1cにも言及 | **対応済**（ただしT3で記述精度に誤り） |
| S5 | ADR-178撤去方針との整合 | 「その他の相互作用」節に1項目追加 | **対応済** |
| S6 | composing中の扱い | 専用節を新設、「実装時に明記」と保留 | **部分**（T6） |
| M11 | `AssumedReason`新variant | 「提案する設計」3に明記 | **対応済** |
| M12 | コア→windows層の受け渡し型 | 疑問1として残置、`ConvToggle` variant案を提示 | **未解決**（T5で具体的な問題点を指摘） |
| M13 | 優先順位1・2との関係 | S1採用により構造的に解消 | **対応済** |
| M14 | `HalfWidthAlnumState`転用範囲 | 「提案する設計」5で「型ごと転用せず`toggle_held`相当のみ・ラッチは分離」と明記 | **対応済** |
| M15 | `send_gji_half_width_alnum_toggle`のcommit規律 | 未反映 | **未対応**（T7） |
| M16 | ATOK下での注入経路未検証 | 未反映（L253が依然無条件に使う前提） | **未対応**（T2に統合） |
| M17 | 無変換3連打 | 「その他の相互作用」節に反映 | **対応済**（方針のみ） |
| M18 | InputRelay | 同上 | **部分**（T8） |
| M20 | 変換側も自動的に効く | 専用節を新設 | **対応済** |
| N2 | 行折り返し | L146-147 で解消 | **対応済** |
| N5 | 見出しの版数 | 一部更新、一部残存 | **部分**（Nit-2） |
| N6 | BUG-115節と疑問3の重複 | 未対応（該当節は削除された） | **解消** |
| N7 | `related_adr`にADR-119/153/154 | 未対応 | **未対応**（Nit-3） |

---

## Blocker

### T1. v5 の緊急性の根拠（R4）が、同日ユーザー自身が ADR-179 に記録した指示と正面から矛盾する。しかも ADR-179 を逆向きに引用している

**該当**: 「現状の機序」L150-161（前提条件の節）、status L39-42（保留解除の理由）

**実リポジトリの状態**: 本ブランチの HEAD コミット `51245736`
（**2026-09-19、本ADR改訂と同日**）:

```
docs(adr-179): 実装状況（決定1・2は実装済み）と実験コミット4件、
               マージ前TODO（実験を撤去し既定のSuppressへ戻す）を追記

status欄が「実装未着手」のままだった。ユーザー指示（2026-09-19）:
Passthrough実験は当面残すが、developへのマージ前にSuppressへ戻す。
```

`docs/adr/179-...md` に追記された本文:

> **マージ前TODO（ユーザー指示、2026-09-19）**: Passthrough設定を前提とした
> 上記の実験は、develop へマージする前に撤去し、既定（Suppress）の挙動へ戻す。
> 手順の案:
> 1. 実験コミット4件のうち、既定（Suppress）の挙動を変えているもの
>    （`f0e36b0e` の Suppress 完全無視、`c0814776` の owner 計算）を洗い出し、
>    「マージするもの」と「revert するもの」に分ける。……
> 2. **Windows実機の `config.toml` から実験設定
>    （`muhenkan_solo_tap_always_suppress = false`、
>    `henkan_solo_tap_always_suppress = false` 等）を削除し、既定値へ戻す。**

一方 ADR-184 v5 L156-161:

> **これは稀な実験設定ではなく、GJI+NICOLA ユーザーの間で広く使われている
> 設定であり、本 ADR はその既存ユーザー層に実際に発生している不具合の修正で
> ある**（新機能の追加ではない）。
> `docs/adr/179-mode-key-actuation-follow-only-vs-toggle-ownership.md` が
> 無変換/変換の非親指キー配置時に actuation-auto を撤去した経緯も、
> **Passthrough 運用が実運用で広く行われていることが前提になっている**。

**2つの問題がある**:

1. **事実関係の矛盾**: ADR-179 は `muhenkan_solo_tap_always_suppress = false` を
   「実験設定」と呼び、実機 `config.toml` からも削除して既定へ戻すよう
   指示している。ADR-184 は同じ設定を「広く使われている設定」と呼ぶ。
   **同じ日の、同じユーザーの、2つの指示が食い違っている。**
2. **引用の向きが逆**: v5 は ADR-179 を自説の**根拠**として引用しているが、
   ADR-179 が実際に書いているのは逆の方針である。この引用は撤回するか、
   ADR-179 側の記述を直すかのどちらかが必要。

**なぜ Blocker か**: 本ADRは「較正ウィザード完成待ちの保留を解除する」
判断を「実害が今出ている」という根拠で行った（status L39-42）。もし
ADR-179 の TODO どおり実験が撤去され既定 Suppress に戻るなら:

- 既定 Suppress では優先順位4が無条件 `SoloTapAction::Suppress` になり、
  生キーは合成されず GJI は無変換を受け取らない（v5 L150-154 が自ら認めている）。
- さらに `f0e36b0e` で入った `Some(cfg) if !cfg.is_passthrough() => no_op_resolution()`
  の腕（`nicola_fsm.rs:2240-2243`）が revert 候補に挙がっており、
  **v5 の設計が隣接して立っている構造そのものが変わる**。
- つまり **develop 上では本ADRが対象とする症状は発生せず**、緊急性の根拠が消える。

**要求**（どれか1つに決めること）:
- (A) Passthrough を正式サポートとして残す決定を ADR-179 側で先に行い、
  マージ前TODO を撤回する。その上で本ADRを進める。
- (B) Passthrough は実験のまま撤去する。その場合、本ADRは「Suppress 既定でも
  無変換単独タップに conv トグルを与える新機能」になり、緊急性の根拠と
  `gji_thumb_key_ime_toggle` の既定値（疑問2）の議論が全部変わる。
- (C) 「Passthrough は広く使われている」の根拠（ユーザー数・報告）を
  ADR に明記し、ADR-179 の TODO を本ADRの決定で上書きすると宣言する。

いずれにせよ **L159-161 の ADR-179 引用は誤りなので削除すること。**

---

### T2. 「actuation は conv 軸専用だから Toggle 判定が外れても実害は無い」は成立しない（S2/S3 への回答が不成立）

**該当**: status L44-54、「設計原則」L220-226

v5 の主張:
> 本設計の actuation は左Shift版と同じ `send_gji_half_width_alnum_toggle`
> （**conv軸専用**、open軸には一切書き込まない）のみを使い、無変換の生キーは
> GJI に一切渡さない。したがって「GJI の内部ロジック（Mozc の宣言）が本当は
> open 軸か conv 軸か」は actuation の正しさに影響しない……判定が外れていても
> 実害（open 軸を誤って動かす等）は起きない設計にする。

**この主張は3つの独立した理由で成立しない。**

#### (a) 送信する VK 自体を、awase 自身が open 軸のキーとして分類している

`crates/awase-windows/src/vk.rs`:

```rust
/// VK_DBE_ALPHANUMERIC / VK_OEM_ATTN (0xF0) — 英数モード（IME OFF 扱い）   // :99
Alphanumeric,
...
Self::ImeOff | Self::Alphanumeric | Self::Deactivate => ShadowImeEffect::TurnOff,  // :148
```

`send_gji_half_width_alnum_toggle` の Enter は `VK_DBE_ALPHANUMERIC`
（`output/mod.rs:1238`）。**awase の静的マップはこれを「IME OFF 扱い」＝
open 軸 `TurnOff` と定義している。** 自己注入マーカー（`IME_KANJI_MARKER`）が
あるので *awase 自身の* shadow-toggle は発火しないが、それは awase の belief を
守るだけで、**受け取った IME が open 軸として解釈しないことは何も保証しない**。

さらに ADR-135「スコープ確定」節は、Mozc 本家 `ms-ime.tsv`/`mobile.tsv` に
`DirectInput Eisu IMEOn` という行が**実在する**ことを記録している——
Mozc 自身がこのキーを open 軸のキーとして扱うプリセットが現に存在する。
同節はこの緊張関係を理由に `VK_DBE_ALPHANUMERIC` を**明示的にスコープ外**に
している:

> `VK_DBE_ALPHANUMERIC` は本プロジェクトで**複数回** IME OFF キーとして
> 採用・撤回されており、その都度「これは半角英数（IME ON）であって直接入力では
> ない」という同じ事実が再発見されている……Eisu を含めると、過去に複数回
> 振り出しに戻った論点を検証不十分なまま作り込むことになる

`.claude/rules/experiment-logging.md` は同じVKについて「5日間に6回、採用と撤回が
反転した」と記録している。**「conv 軸専用だから安全」という主張は、
このリポジトリが最も高い代償を払って学んだ論点を、検証なしに再び仮定している。**

左Shift版（BUG-25）がこのVKで動いている実績はあるが、その検証は
**CUSTOM または MSIME キーマップ環境**（BUG-115 が 2026-09-05 に
`session_keymap = CUSTOM(0)`、ADR-174 が 2026-09-15 に `MSIME(2)` を実測）で
行われたものであり、**ATOK プリセット下は未検証**（round2 M16 / round3 M16、
v5 でも未反映）。「conv 軸専用」は ATOK 下では実測に裏付けられていない。

#### (b) `ImeToggleKind::Toggle` は「ATOK プリセット由来」と「CUSTOM literal 由来」を区別せず畳んでいる

`gji_charset_autodetect.rs::classify_mode_key_ime_action` は3つの情報源を
**優先順位付きで1つの `Option<ImeToggleKind>` に畳む**（BUG-115「設計」節）:

| 情報源 | Muhenkan が `Toggle` になる条件 | 実際の軸 |
|---|---|---|
| `overlay_keymaps`（最優先） | ならない（`Off` 固定） | — |
| `session_keymap == CUSTOM` の `custom_keymap_table` literal | `keys.toggle` に含まれる＝`extract_ime_keys` が `IMEOn` 行と `IMEOff` 行の対から導出（`classify_vk_in_ime_keys`、`gji_charset_autodetect.rs:324-337`） | **本物の open 軸** |
| ATOK プリセット静的知識 | 無条件（`:297-300`） | v5 の主張では conv 軸 |

**戻り値は `ImeToggleKind` だけで、どの情報源から来たかの情報は失われる。**

v5 の設計1 は「`Toggle` 判定を `muhenkan_delegate_to_open_axis` へ実際に書き込む
ようにする」（＝配線を有効化する）としているので、**CUSTOM で Muhenkan に
`IMEOn`/`IMEOff` を明示的に割り当てているユーザーの `Toggle` も同じ扱いになる**。
そのユーザーに何が起きるか:

1. 生キーは `explicit_resolve()` により**合成されない**（v5 設計4）。
2. awase は open 軸ではなく **conv 軸**（`VK_DBE_ALPHANUMERIC`）を送る。
3. 結果: **ユーザーが自分で設定した「無変換で IME を ON/OFF する」機能が
   完全に消え、代わりに半角英数トグルになる。**

これは「判定が外れても実害は無い」どころか、**判定が当たっている
（本当に `Toggle` である）ユーザーにこそ起きる機能破壊**である。
BUG-115 が記録するとおり、**プロジェクトオーナー自身の実機が過去に
`session_keymap = CUSTOM` だった**ので、机上の空論ではない。

#### (c) 「生キーを一切渡さない」ことが、判定ミス時の被害を回復不能にしている

v5 の設計は「Toggle と判定 → awase が conv を書く、生キーは出さない」である。
判定が外れていた場合、ユーザーは:

- 無変換を押しても期待する動作（IME OFF なり何なり）が起きない
- **かつ、生キーも届かないので GJI 側の本来の機能も働かない**

round3 以前の設計（生キーが届く）なら「awase が余計なことをするが、
GJI 本来の動作は残る」だったが、v5 は生キーを止めるので
**awase の判定が唯一の経路**になる。「最悪でも conv トグルが期待どおりに
機能しないだけ」（L225-226）という自己評価は、この点を見落としている。

**要求**:
1. **情報源を保持する**: `classify_mode_key_ime_action` の戻り値に
   「どの情報源から来たか」を持たせ、**ATOK プリセット由来の `Toggle` のみ**を
   本設計の対象にする（CUSTOM literal 由来の `Toggle` は従来どおり
   open 軸 `explicit_resolve()` のまま）。これが (b) への最小の対処。
2. **ATOK 下での実測**（round2 M16 から3ラウンド未実施）: ATOK プリセットの
   GJI に `VK_DBE_ALPHANUMERIC`/`VK_DBE_HIRAGANA` を送ったとき、conv だけが
   変わり open が変わらないことを実機で確認する。これが (a) への対処であり、
   **本設計の中心的前提の唯一の検証手段**である。ADR-135 が Eisu を
   スコープ外にした理由に正面から答える段落も要る。
3. (c) について、判定ミス時のフォールバック（例: conv 書き込み後に
   `idle-conv-check` が conv 不変を観測したら生キー経路へ戻す）を検討するか、
   「フォールバックは持たない」と明記する。

---

## Must-fix

### T3. ADR-182 の実装に関する記述が不正確（ただし結論は v5 に有利な方向へ訂正される）

**該当**: L298-316

v5 の記述:
> `PendingThumbData::suppresses_open_axis_actuation()`（`auto_delegate_open_axis_
> consumed || after_char_flush` を合成、`nicola_fsm.rs`）が `resolve_pending_
> thumb_as_single` の7呼び出し元すべてで**7番目の引数として渡され**、これが
> 立っていれば優先順位3（`resolve_delegate_to_open_axis`）に到達する前に
> `no_op_resolution()` で打ち切る

**実コード（`src/engine/nicola_fsm.rs:2317-2325`）**:

```rust
fn resolve_pending_thumb_as_single(
    &self,
    scan_code: ScanCode,
    vk_code: VkCode,
    modifier_key: Option<crate::types::ModifierKey>,
    injected: bool,
    composing: bool,
    explicit_action_consumed: bool,
    auto_delegate_open_axis_consumed: bool,      // ← 7番目、名前は従来のまま
) -> (ResolvedAction, Option<ImeOpenRequest>) {
```

**新しい引数は追加されていない。** ADR-182 は `suppresses_open_axis_actuation()`
の戻り値を**既存の `auto_delegate_open_axis_consumed` 引数の実引数として渡す**
形で着地した（呼び出し元7箇所: `:631`・`:1657`・`:1870`・`:1899`・`:2996`・
`:3028`・`:3155`。うち `:1870` は決定1b のため
`thumb.suppresses_open_axis_actuation() || char_has_thumb_face` とさらに OR）。
ガードは既存の `if auto_delegate_open_axis_consumed { return no_op }`（`:2409`）
がそのまま担っている。ADR-182 ドラフトが計画した「シグネチャを
`(&self, thumb: &PendingThumbData, composing: bool)` へ束ね直す」は**実施されて
いない**。

**この訂正は v5 にとって良い知らせである**: `:2409` の early return は
`resolve_delegate_to_open_axis` の呼び出し（`:2415`）より前なので、
**v5 が変更する Toggle 分岐は ADR-182 のフラグによって自動的に覆われる**。
したがって L337-341 の TODO:

> ADR-182 決定1bの `candidate.is_some()`……と同じ理由から、**本ADRの新トグルも
> 同じ条件に従う**（決定1bが立てるフラグの対象に本ADRの分岐も含める）

は **追加作業ではなく、既に無条件に成立している**。「含める」ではなく
「既存の `:2409` ガードが `resolve_delegate_to_open_axis` の手前にあるため
自動的に成立する（新規配線不要）」と書き直すこと。

**ただし負債が1つ増える**: `auto_delegate_open_axis_consumed` という引数名は
いま「ADR-154 の consumed マーカー **または** ADR-182 の char-flush フラグ」の
2義であり、v5 が conv 軸トグルを足すと「conv トグルの抑止」という3つ目の
意味も背負う。改名（例: `suppress_mode_key_actuation`）を本ADRのスコープに
含めるか、少なくとも doc コメントで3義を明記すること。

### T4. 行番号の参照が混在している（正しいもの1箇所、古いもの3箇所）

ADR-182 マージ（`740bf8be`）で `nicola_fsm.rs` の行番号が約80行ずれた。

| 箇所 | 記載 | 実際 | 判定 |
|---|---|---|---|
| L311 | `Toggle` 腕〈**2246-2253行**〉 | `if matches!(.., Toggle)` = 2246、`explicit_resolve()` = 2253 | **正しい** |
| L203 | `resolve_delegate_to_open_axis`（**2168-2185行**） | 現在は 2244-2261（`Some(_)` 腕） | 古い |
| L242 | `Toggle` 分岐（**2172行付近**） | 2246 | 古い |
| L356 | `nicola_fsm.rs:2172-2176` | 2246-2252 | 古い |

すべて `2246-2253`（および関数全体なら `2211-2262`）へ統一すること。

### T5. M12（conv 軸の要求型）は `ImeOpenRequest` への variant 追加では収まらない

v5 疑問1 は「`ConvToggle` variant を足すか、第3の戻り値にするか」を未決に
しているが、**`ImeOpenRequest` への追加は次の理由で筋が悪い**:

```rust
// src/engine/fsm_types.rs:702-710
impl ImeOpenRequest {
    /// この要求が要求する`ShadowImeAction`（責務の種別を問わない）。
    pub const fn action(self) -> crate::types::ShadowImeAction {
        match self {
            Self::Explicit(a) | Self::FollowOnly(a) => a,
        }
    }
}
```

- `action()` は**全 variant が `ShadowImeAction` を返す**ことを前提にした
  total function。`ConvToggle` は `ShadowImeAction` を持てない（open 軸語彙）ので、
  `Option` 化するか panic するかになり、呼び出し元全部に影響する。
- 型名自体が `ImeOpen`Request であり、conv 軸の要求を入れると名前が嘘になる。
- 消費側 `Engine::apply_ime_open_request` は open 軸の `resolve(current_open)`
  を呼ぶ。

**推奨**: `resolve_pending_thumb_as_single` の戻り値タプルの第2要素を
`Option<ImeOpenRequest>` から `Option<ModeKeyRequest>`（仮称、
`OpenAxis(ImeOpenRequest)` / `ConvToggle` の2 variant）へ広げる。
呼び出し元7箇所とその先の `Effect` 生成が影響範囲。
ADR-019 との整合は取れる（コアは既に `ConvMode`/`InputModeState` を持つ）。
**この型を決めないと実装に着手できないので、疑問のままではなく
決定として書くこと。**

### T6. composing の扱いを「実装時に明記」で先送りすると、決定の一部が抜け落ちる

v5「composing 中の扱い」節は正しい問題提起をしているが、結論を実装時へ
送っている。**ここは設計判断であり、ADR で決めるべき**:

- 現状の fail-closed（`:2250-2252`）の理由は「誤って `Toggle(→OFF)` すると
  composition を復旧不能に破棄する」＝**open 軸固有のリスク**。conv 軸に
  差し替えれば、この理由はそのまま消える。
- 左Shift版は ADR-107 決定5 を BUG-25追補5 の実機検証（preedit 非破壊）に
  基づいて緩和し、**composition 中も発火する**（`output/mod.rs:1255-1268`）。
  そのコメントは「preedit 破壊の兆候が実機で出た場合はここにガードを復活
  させること」と明記している。
- したがって整合的な選択肢は2つ:
  - (i) 左Shift版に揃えて composing 中も発火させる（根拠: BUG-25追補5）。
  - (ii) 保守的に fail-closed のまま残す（仕様: 「composing 中は無変換タップで
    conv が切り替わらない」）。

どちらかを決定として書くこと。(i) を採るなら、BUG-25追補5 の検証が
**ATOK プリセット下では未実施**である点（T2-a）と合わせて扱うこと。

### T7（継続、round2 M15 / round3 M15）. `send_gji_half_width_alnum_toggle` の commit 規律が未反映

v5 L253-254 は「`send_gji_half_width_alnum_toggle`（Enter=…/Exit=…）で conv を
書き込む」とだけ書き、**失敗時の扱いが無い**。実コードの事実:

| 箇所 | 事実 | 設計への影響 |
|---|---|---|
| `output/mod.rs:1241-1247` | Win/Alt 押下中は**無送信で `false`** | トグル不発 |
| `output/mod.rs:1248-1254` | `if !ime_open { 無送信で false }` | open 軸を**読む**（「open 軸に一切書き込まない」は正しいが「依存しない」ではない） |
| `key_pipeline.rs:2313-2318` | Enter は `commit_enter_gji()` が **commit-on-success** | そのまま流用可 |
| `key_pipeline.rs:2381-2410` | Exit は `prepend_synthetic_shift_up == false` のとき**未送信でも `true` を返し、呼び出し元に belief 補正を進めさせる** | 無変換起点は `prepend=false` が自然。そのまま流用すると「実GJIは半角英数のまま、awase の belief だけひらがな」＝**BUG-25追補3 と同型の実害**（engine が pass-through を抜けて生ローマ字を送る） |

無変換タップはユーザーが即座に再試行できる文脈なので、
`rearm_after_failed_gji_exit` 相当の扱いが要る。**「提案する設計」3 に
1段落足すこと。**

### T8（継続、round3 M18）. InputRelay のゲート位置が具体化されていない

v5「その他の相互作用」節の InputRelay の記述は問題の提示までで、
**どこにゲートを置くか**が無い。v5 の設計では判定がコア
（`resolve_delegate_to_open_axis`）にあり、`AppImeProfile` は windows 層に
しかない。したがって:

- コア側で止める（`special` に「actuation を所有してよいか」のフラグを足す）
- windows 層で `ConvToggle` 要求を受けた後に握り潰す（＝コアは要求を出すが
  windows が実行しない）

のどちらかになる。**後者は ADR-119 が禁じる「二重の空振り」そのもの**
（コアが生キーを止め、windows も何もしない）なので、前者が必要。
`.claude/rules/fix-requires-evidence.md`「IME actuation 合流点」行の
洗い出しと合わせて書くこと。

### T9（新規）. 設計1（配線の有効化）が疑問2・疑問3 と噛み合っていない

「提案する設計」1 は3つの選択肢を並べたまま:
- `gji_thumb_key_ime_toggle` の既定値を変える
- Passthrough 選択時に限定した自動有効化
- （疑問3）`Toggle` 判定を前提条件にするか常時有効にするか

T1 の決着（Passthrough を残すか撤去するか）と T2-1（情報源を保持して
ATOK 由来のみ対象にするか）が決まれば自動的に絞れるので、
**T1/T2 の後に1つへ決めること**。

また `gate_thumb_key_ime_actions` は Henkan/Muhenkan を**独立に**ゲートする
（`gji_charset_autodetect.rs:348-354` の doc「一方だけ `Toggle` の場合もある」）。
「変換の扱い」節（M20）の「VK で明示的に限定する」案は、このゲートに
手を入れずとも実現できる——その旨を書くと実装が軽くなる。

### T10（新規）. ADR-179 の revert 候補 `f0e36b0e` が、v5 の設計が隣接して立つ腕そのものを作っている

`nicola_fsm.rs:2240-2243`:

```rust
Some(cfg) if !cfg.is_passthrough() => {
    // Henkan/Muhenkan、Suppress設定: 方向を問わず完全に無視する。
    DelegateResolution::Resolve(Self::no_op_resolution())
}
```

この腕は実験コミット `f0e36b0e`（「Henkan/Muhenkan×Suppress設定は方向を問わず
完全に無視」）が作ったもので、ADR-179 のマージ前TODO は
**この commit を revert 候補として名指ししている**。

revert されると、Suppress 設定時に `Toggle` 分岐へ到達しうる経路が復活し、
v5 の「Passthrough 設定時のみ Toggle 分岐に来る」という暗黙の前提が崩れる。
**v5 の設計が依存している構造は、同じブランチ上で撤去が検討されている
実験コミットの産物である**ことを本文に明記し、T1 の決着と紐付けること。

---

## Nits

### Nit-1. ATOK 矛盾の扱いが、同一文書の3箇所でまだ食い違っている（round3 S2 の再発）

| 箇所 | 立場 |
|---|---|
| `summary` L11-16 | 「この矛盾が**まだ解けていない**（……のいずれかが実機未確認）」 |
| 背景・経緯 L118 | 「**この矛盾はユーザーの説明で解消した**」 |
| status L44-54 | 「（矛盾は）**実装を妨げないことにした**……根本原因は未解明のままでよい」 |

status の立場（＝「未解明のまま棚上げする」）が最も正確なので、
summary と背景をそれに揃えること。特に背景 L118 の「解消した」は、
続く L120-125 が「〜可能性が高い」と推測に留めていることと自己矛盾している。

### Nit-2. 見出しの版数が残っている

- L382「## 未解決の疑問（opus-adversarial-consult **round4** で検証してほしい点）」← これは正しい
- L228「## 提案する設計（決定案v5、round3 S1 準拠で全面改訂）」← 正しい
- L191「## 設計原則: ……（ADR-179決定2で既に確立済み、**round3 S1**）」← 正しい

今回は概ね整合。L288「## ADR-181/ADR-182 との関係（**v5で更新**）」も可。
残る不整合は無し（round3 N5 は解消）。

### Nit-3（継続、round3 N7）. `related_adr` に ADR-119 / ADR-153 / ADR-154 / ADR-107 が無い

本ADRが触る `resolve_pending_thumb_as_single` の既存ガードは ADR-153
（`explicit_action_consumed`）・ADR-154（`auto_delegate_open_axis_consumed`）
由来、T8 は ADR-119、T6 は ADR-107 決定5 を参照する。4つとも追加すること。

### Nit-4. 「現状の機序」の手順1〜7 が、依然 ADR-182 マージ前の記述のまま

手順1〜7 は ADR-182 がマージされる前のログ由来である。ADR-182 決定1/1b/1c が
入った現在、**同じ操作をしても失敗4件相当（チョード誤判定経由）は
再現しなくなっている**はずで、残るのは「Idle 起点の正常な単独タップ」
経由のみ。この区別を1行足すと、本ADRが何を直そうとしているかが明確になる。

### Nit-5. 疑問6（証拠のビルド特定）が3ラウンド未実施

`git log` で `c8bc1adc`（warrant 強制）の日時は 2026-09-18 23:21、
ADR-182 のマージ `740bf8be` はその後。実機ログの取得時刻が分かれば
どのビルドかは機械的に決まる。「記録として残すべきか」ではなく、
**手順3 の `send 0x001A` が現行コードでも起きるのか**という実質的な問いなので、
疑問から外して「実装前に1回確認する」タスクにすること。

---

## 次のラウンドへの要求（優先順）

1. **T1 を決める。** Passthrough を正式サポートとして残すのか、ADR-179 の
   マージ前TODO どおり撤去するのか。これが本ADRの緊急性・スコープ・
   既定値（疑問2/3）・T10 のすべてを決める。**ADR-179 側との整合を
   取らずに本ADRだけ進めると、develop マージ時に衝突する。**
2. **T2 に答える。** (i) `classify_mode_key_ime_action` の戻り値に情報源を
   持たせて ATOK 由来の `Toggle` のみを対象にする、(ii) ATOK プリセット下で
   `VK_DBE_ALPHANUMERIC`/`VK_DBE_HIRAGANA` が conv 軸のみに効くことを実機で
   確認する（round2 M16 から3ラウンド未実施）、(iii) 判定ミス時に生キー経路へ
   戻すフォールバックの有無を決める。
3. **T5（`ModeKeyRequest` 型）を疑問から決定へ格上げする。** これが無いと
   実装に着手できない。
4. **T3 の記述を訂正する**（引数は増えていない／本ADRの分岐は既存ガードで
   自動的に覆われる／引数名の3義化）。
5. **T6（composing）を決定として書く。** T4（行番号）も同時に直す。
6. T7・T8・T9・T10 を本文へ反映。
7. Nit-1〜5。
