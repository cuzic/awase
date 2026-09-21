# ADR-184 opus-adversarial-consult round5

対象: `docs/adr/184-gji-atok-muhenkan-toggle-awase-owned-eisu-hiragana.md`（v6）
前回: `docs/adr/184-opus-review-round4.md`（Blocker 2 / Must-fix 8 / Nits 5）
レビュー日: 2026-09-19 / ブランチ `feat/adr178-mode-key-actuation-and-tsfnative-rescue-teardown`（HEAD `51245736`）
方式: 読み取りのみ。v6 の主張・行番号・情報源区別の実現可能性を実コードで裏取りした。

---

## 総評

**T1 は解消した。** 「Passthrough は公式に非推奨だが実際には多くのユーザーが
選択しており不具合報告もある／ADR-179 の TODO は既定値の話」という整理は
筋が通っており、ADR-179 との衝突は無い。**T5・T6・T7・T8・T9・T10 も決定へ
格上げされ、設計としての骨格はほぼ固まった。**

**T2 への対応（情報源の区別）は方向として正しいが、実現方法が実コードと
噛み合っていない。** v6 は「`classify_mode_key_ime_action` の戻り値に情報源を
保持させる（例: `ImeToggleKind::Toggle { origin: .. }`）」としているが:

1. **情報源はコア境界で必ず消える。** 判定点（`resolve_delegate_to_open_axis`）
   はコアにあり、コアへ渡る唯一の経路は
   `Engine::set_muhenkan_delegate_to_open_axis(Option<ShadowImeAction>)`。
   `ShadowImeAction` は3値の open 軸 enum で origin を運べない。
2. **同じコアフィールドには MS-IME レジストリ由来の書き手がもう1つある。**
   Toggle の意味を conv 軸に変えると、本ADRが検討していない MS-IME 経路も
   巻き込む。
3. **`ImeToggleKind` は ADR-176（較正ウィザード）の結果型でもあり、
   config.toml に永続化される。** payload を足すと、origin の存在しない
   経路（較正結果・MS-IME レジストリ）に origin を要求することになり、
   永続化スキーマにも触れる。

**より安い代替がある**（U1 で具体案を示す）: 軸の決定を windows 層の
書き込み時に済ませ、コアには**別フィールド**で渡す。これなら
`ImeToggleKind` も ADR-176 も MS-IME 経路も BUG-115 の約20本の回帰テストも
一切触らずに済み、T5 の `ModeKeyRequest` とも素直に合成できる。

もう1点、**v6 の「情報源は2つ」という前提が実コードでは4つある**（U2）。
特に **ADR-174 が追加した第3の経路**（`session_keymap` が非CUSTOMでも
`custom_keymap_table` に行があればそちらを優先）は、**本ADRの動機になった
実機そのものが該当しうる**構成であり、無視できない。

重大度の内訳: Blocker 2件 / Must-fix 7件 / Nits 4件。

---

## round4 指摘の対応状況

| # | round4指摘 | v6での対応 | 判定 |
|---|---|---|---|
| T1 | 緊急性の根拠がADR-179のTODOと矛盾／引用の向きが逆 | summary・statusを訂正、Passthroughの位置づけを整理 | **対応済**（ただし本文L165-176は未修正 → U5） |
| T2 | conv軸専用だから安全は不成立（VK_DBE_ALPHANUMERIC／Toggleの情報源混在／生キー停止で回復不能） | 情報源区別の設計へ変更、較正ウィザード統合を将来課題に | **方向は正しいが実現方法が不成立**（U1・U2） |
| T3 | ADR-182統合の記述訂正／引数名の3義化 | 機序の記述は正確になった | **部分**（L412-416にTODOが残存 → U6。引数名は未対応 → U7） |
| T4 | 行番号 | 2211-2262 / 2246-2253 / 2240-2243 / 2250-2252 へ統一 | **対応済**（実コードと一致を確認） |
| T5 | `ModeKeyRequest`型 | 決定へ格上げ（設計2） | **対応済**（U1と合わせて再設計の余地） |
| T6 | composing | 「composing中も発火させる」と決定（設計5） | **対応済** |
| T7 | commit規律 | Exit側を明記（設計3） | **部分**（Enter側のラッチ規律が未記載 → U9） |
| T8 | InputRelayのゲート位置 | コア`special`にフラグを足す決定（設計7） | **部分**（動的値の供給経路が未設計 → U8） |
| T9 | 配線方法の3択 | Passthrough限定の自動配線に決定（設計1） | **対応済** |
| T10 | `f0e36b0e`依存 | 設計1に明記 | **対応済** |
| Nit-1 | atok.tsv矛盾がsummary/背景/statusで不整合 | 未対応 | **未対応**（U4、3ラウンド連続） |
| Nit-3 | related_adr | ADR-107/119/153/154を追加済み | **対応済** |
| Nit-4 | 手順1〜7がADR-182マージ前の記述 | 未対応 | **未対応**（Nit-b） |
| Nit-5 | 証拠のビルド特定 | 疑問6のまま | **未対応**（Nit-c） |

---

## Blocker

### U1. 「`ImeToggleKind` に origin を足す」では判定点に届かず、3つの既存消費者と衝突する

**該当**: 「設計原則」L236-254（情報源の区別）、status L59-71

#### (1) origin はコア境界で必ず消える

判定点はコアの `resolve_delegate_to_open_axis`（`src/engine/nicola_fsm.rs:2211-2262`）。
そこへ値が届く経路は次の1本しかない:

```
gji_charset_autodetect::classify_mode_key_ime_action  → Option<ImeToggleKind>
  → gate_thumb_key_ime_actions                        → ThumbKeyImeWiring{henkan, muhenkan: Option<ImeToggleKind>}
  → ime_toggle_kind_to_shadow_action                  → Option<ShadowImeAction>      ← ここで origin が消える
  → Runtime::set_gji_thumb_key_delegate_to_open_axis  (runtime/mod.rs:1235-1241)
  → Engine::set_muhenkan_delegate_to_open_axis(Option<ShadowImeAction>)  (engine.rs:164)
  → NicolaFsm の muhenkan_delegate_to_open_axis
  → ThumbSoloSpecialHandling.delegate_to_open_axis
  → resolve_delegate_to_open_axis の open_axis_action
```

`ShadowImeAction`（`src/types.rs:143-147`）は `TurnOn`/`TurnOff`/`Toggle` の
3値で payload を持たない**コア側の open 軸語彙**である。
**v6 は「`classify_mode_key_ime_action` の戻り値に origin を保持させる」と
書くだけで、その origin が上記6段をどう渡るかを一切書いていない。**
`ImeToggleKind` に origin を足しても、`ime_toggle_kind_to_shadow_action`
（`gji_charset_autodetect.rs:385-395`）が `ShadowImeAction` へ潰す時点で
情報は失われ、判定点には届かない。

#### (2) 同じコアフィールドには MS-IME レジストリ由来の第2の書き手がいる

`crates/awase-windows/src/runtime/mod.rs:1227-1234`（`set_gji_thumb_key_
delegate_to_open_axis` の doc）:

> MS-IME側の`sync_ime_toggle_auto_detect`（レジストリ由来）と
> **同じ`set_muhenkan/henkan_delegate_to_open_axis`APIを共有する**ため、
> GJI→MS-IME遷移時はMS-IME側の値が必ず後から上書きする
> （`message_handlers.rs`の呼び出し順序参照）。

実際の第2の書き手は `message_handlers.rs:947-948`。
**`ShadowImeAction::Toggle` の意味を「conv 軸トグル」に変えると、
MS-IME レジストリが `Toggle` を返す構成でも conv 軸に倒れる。**
本ADRは MS-IME 経路を一度も検討していない（`related_adr` にも
`msime_key_assignment.rs` への言及が無い）。これは
`.claude/rules/fix-requires-evidence.md`「IME actuation 合流点」の
「新しい gate を1箇所に置いて満足しない、実際の呼び出し経路をすべて
洗い出す」に正面から該当する。

#### (3) `ImeToggleKind` は ADR-176 の結果型でもあり config.toml に永続化される

- `crates/awase-windows/src/state/calibrated_mode_key.rs:43-55`:
  ```rust
  pub(crate) struct CalibratedModeKey {
      pub(crate) vk: VkCode,
      pub(crate) result: ImeToggleKind,     // ← 較正結果
      ...
  }
  ```
  `to_config_entry()` で `awase::config::CalibrationEntry` へ変換され、
  **config.toml に永続化される**（ADR-176 T11、`dae84745`）。
- `gji_charset_autodetect.rs:398-410` の `shadow_action_to_ime_toggle_kind`
  は「ADR-176（較正結果の適用、176-T4）はどちらの経路でも同じ
  `CalibratedModeKey::result: ImeToggleKind` を使うため」に追加された
  **逆変換**であり、`ShadowImeAction::Toggle → ImeToggleKind::Toggle` を
  返す。ここで origin を要求されても**返せる値が無い**。
- `gji_charset_autodetect.rs:907-909` は較正結果として
  `result: ImeToggleKind::On` を直接構築する。

`ImeToggleKind` に payload を足すと、**origin の存在しない2経路
（較正・MS-IMEレジストリ）に origin を捏造させ、かつ永続化スキーマに
触れる**ことになる。ユーザー実機（dragonflyg4）の config.toml は
ADR-176 の本番設定として残置されているため、スキーマ変更は実害を伴う。

#### 推奨する代替設計（コストが1桁小さい）

**軸の決定を windows 層の書き込み時に済ませ、コアには別フィールドで渡す。**

```
sync_gji_charset_autodetect（windows層）
  ├─ origin == AtokPreset かつ Toggle  → set_gji_thumb_key_conv_toggle(Some(vk))  [新設]
  │                                      （delegate_to_open_axis は None のまま）
  └─ それ以外                          → 既存どおり set_gji_thumb_key_delegate_to_open_axis(...)
```

コア側は `ThumbSoloSpecialHandling` に `conv_toggle: bool` を1つ増やし、
`resolve_delegate_to_open_axis` の冒頭で:

```rust
if special.conv_toggle {            // ← 新しい腕（AtokPreset由来のToggle専用）
    if /* InputRelay等 */ { return DelegateResolution::Fallthrough(None); }
    return DelegateResolution::Resolve((no_op_actions, Some(ModeKeyRequest::ConvToggle)));
}
```

**この形の利点**:

| 論点 | `ImeToggleKind` に origin を足す案 | 別フィールド案 |
|---|---|---|
| origin がコアに届くか | 届かない（上記(1)） | 不要（windows層で決着） |
| ADR-176 / config.toml | 永続化スキーマに影響 | **無影響** |
| MS-IME レジストリ経路 | `Toggle` の意味が変わり巻き込む | **無影響**（別フィールドを書かない） |
| BUG-115 の約20本の回帰テスト | `ImeToggleKind` の型変更で全面改修 | **無影響** |
| `shadow_action_to_ime_toggle_kind` | 返す origin が無い | **無影響** |
| 既存 `Toggle` 分岐（2246-2253） | 写像先を差し替える＝既存挙動も変わる | **手を触れない**（CustomLiteral由来はそのまま open 軸） |
| T5 の `ModeKeyRequest` | 同上 | 新しい腕が素直に `ConvToggle` を返す |

**副次的な利点**: v6 の設計2（「既存 Toggle 分岐の写像先を差し替える」）は、
CustomLiteral 由来の Toggle も同じ分岐を通るため「origin で分岐を分ける」
コードを分岐**内部**に書くことになる。別フィールド案なら
**既存の Toggle 分岐には一切手を触れない**ので、round3 S1 が評価した
「既存決定点を壊さない」という性質がより強く保たれる。

**要求**: 設計原則節と設計2を、この別フィールド案（または同等に
origin をコアへ運ばずに済む案）へ書き直すこと。`ImeToggleKind` を
変更する案を採るなら、上記 (1)(2)(3) すべてに答えること。

---

### U2. 情報源は2つではなく4つある（特に ADR-174 の第3経路が、本ADRの動機になった実機に該当しうる）

**該当**: 「設計原則」L241-244 の2行の表

`classify_mode_key_ime_action`（`gji_charset_autodetect.rs:252-320`）の
実際の分岐は**4段**である:

| 段 | 条件 | 行 | Muhenkan が `Toggle` になるか | v6の表 |
|---|---|---|---|---|
| 1 | `overlay_keymaps` に 100 を含む | :256-267 | ならない（`Off` 固定） | 記載なし（無害） |
| 2 | `session_keymap == CUSTOM` の `custom_keymap_table` literal | :268-274 | なりうる（本物の open 軸） | **記載あり** |
| 3 | **`session_keymap` が非CUSTOMでも `custom_keymap_table` に該当行があればそちらを優先**（ADR-174、2026-09-15 実機検証で追加） | :290-295 | なりうる（本物の open 軸） | **記載なし** |
| 4 | プリセット静的知識（ATOK → `Toggle`） | :296-300 | なる（v6 の主張では conv 軸） | **記載あり（「優先順位4」と書いてある）** |

**段3が抜けているのが重大**な理由:

- 段3は **ADR-174 が実機で発見して追加した経路**である。コード内コメント
  （`:275-289`）はその実測をこう記録している:

  > ADR-174実機検証（2026-09-15）: `session_keymap`がCUSTOM以外の値
  > （**実機でMSIME=2を確認**）でも、`custom_keymap_table`にこのキーの
  > 明示的な行（**実機で`DirectInput\tHenkan\tIMEOn`を確認**）が実在する
  > ことがある

  つまり**プロジェクトオーナー自身の実機が、まさにこの構成だった**。
  もし現在も `custom_keymap_table` に Muhenkan の行が残っていれば、
  `session_keymap` を ATOK に変えても **段3が段4より先に勝つ** ので、
  origin は `CustomLiteral` になり、**本ADRの新機能は一度も発火しない**。
- v6 の表は ATOK 段を「優先順位4」と書いているのに、表の行は2つしかない。
  番号と行数が合っていないこと自体が、段3の脱落を示唆している。

さらに**第4の情報源**として、U1(3) で挙げた
`shadow_action_to_ime_toggle_kind`（MS-IME レジストリ／ADR-176 較正由来）が
ある。これも origin を持たない。

**要求**:
1. 表を4段（+ 較正/MS-IME 由来）に書き直す。
2. 「段3が勝つ構成では本設計は発火しない」ことを明記する。
3. **実装前の実機確認項目に `custom_keymap_table` の Muhenkan/Henkan 行の
   有無を加える**（round2 R2 から4ラウンド未実施の
   「`session_keymap` 実値確認」と同時にできる）。これを確認しないと、
   実装しても「効かない」という結末になりうる。

---

## Must-fix

### U3. 「ヒント扱い＋idle-conv-check が事後補正」（背景）と「AtokPreset origin のみ発火」（設計原則）と「`note_explicit_ime_action` 必須」（設計3）が三つ巴で噛み合っていない

**該当**: 背景 L142-150、設計原則 L236-254、設計3 L320-324

| 箇所 | 主張 |
|---|---|
| 背景 L142-150 | 判定は「絶対に正しいゲートとしては使わない……**ヒント**」。誤りは「`idle-conv-check` の**受動観測で事後的に補正**する」 |
| 設計原則 L241-254 | `AtokPreset` origin の場合に**のみ**発火（＝**ゲート**） |
| 設計3 L320-324 | `note_explicit_ime_action` を**必須**にする。これは `EXPLICIT_IME_SUPPRESS_MS` の間 **idle-conv-check をスキップさせる**（`platform_state.rs:441-451`） |

背景が「事後補正の担い手」と呼んでいる idle-conv-check を、設計3 が
意図的に黙らせている。**同じ文書の中で、誤判定時の回復機構が
「ある」と「止める」の両方に書かれている。**

なお `note_explicit_ime_action` は**時間窓**の抑止なので
（`EXPLICIT_IME_SUPPRESS_MS` 経過後は idle-conv-check が再開する）、
「事後補正」が原理的に不可能というわけではない。ただし:

- awase の書き込みが**成功**した場合、conv と belief が一致するので
  `classify_idle` は `None` を返し、補正は起きない（起きる必要もない）。
- awase の書き込みが**失敗**した場合にのみ、窓が明けた後の
  idle-conv-check が乖離を拾う。そのとき何が起きるか
  （`classify_conv_transition` のどの分岐に落ちるか、
  `EngineSync::SetOpen(RomajiRecovered)` か `None` か）は
  **ADR に追跡が書かれていない**。

**要求**: 「ヒント」という表現を撤回し（設計原則の origin ゲートが
採用された以上、これはゲートである）、誤判定・書き込み失敗時の回復は
(i) `EXPLICIT_IME_SUPPRESS_MS` 経過後の idle-conv-check がどの分岐で
拾うのかを具体的に書くか、(ii) 「回復機構は持たない、較正ウィザード統合
まではベストエフォート」と正直に書くか、どちらかに決めること。

### U4（3ラウンド連続未対応）. `summary` と本文で atok.tsv 矛盾の扱いが逆

- `summary` L11-16: 「この矛盾が**まだ解けていない**（……のいずれかが実機未確認）」
- 背景 L133: 「**この矛盾はユーザーの説明で解消した**」

v6 では summary の L28-42 は T1/T2 対応で更新されたが、**L11-16 は v1 から
一度も触られていない**。`.claude/rules/docs-frontmatter-convention.md` は
frontmatter を「本文を開かずに gist を得る正本」と定めているので、
ここが本文と逆のことを言っているのは実害がある。

なお round4 Nit-1 の指摘どおり、**最も正確なのは「未解明のまま棚上げする」**
（背景 L133 の「解消した」は、続く L137-140 が「〜可能性が高い」と
推測に留めていることと自己矛盾している）。3箇所をその立場に揃えること。

### U5（T1 の本文未反映）. 「現状の機序」L165-176 に、round4 T1 が削除を求めた誤った ADR-179 引用がそのまま残っている

status と summary は訂正されたが、本文は v5 のままである:

> L171-173: **これは稀な実験設定ではなく、GJI+NICOLA ユーザーの間で広く
> 使われている設定であり**、……
> L174-176: `docs/adr/179-...md` が無変換/変換の非親指キー配置時に
> actuation-auto を撤去した経緯も、**Passthrough 運用が実運用で広く
> 行われていることが前提になっている**。

L174-176 は round4 T1 で「ADR-179 が書いていることの逆なので削除すること」
と指摘した箇所そのもの。ADR-179 は Passthrough を「実験」と呼び、
実機 `config.toml` からの削除まで指示している（`51245736`）。
**この1文は削除し、L171-173 を summary/status と同じ表現
（「公式には非推奨だが実際には多くのユーザーが選択しており不具合報告もある」）
に揃えること。**

### U6（T3 の残り）. ADR-182 節に、訂正済みのはずの TODO が残っている

L412-416:
> また ADR-182 決定1bの `candidate.is_some()`……と同じ理由……から、
> **本ADRの新トグルも同じ条件に従う**（決定1bが立てるフラグの対象に
> 本ADRの分岐も含める）。

round4 T3 で確認したとおり、`resolve_pending_thumb_as_single` の
`if auto_delegate_open_axis_consumed { return no_op }`（`nicola_fsm.rs:2409`）は
`resolve_delegate_to_open_axis` の呼び出し（`:2415`）**より前**にあり、
7呼び出し元すべてが `thumb.suppresses_open_axis_actuation()`
（`:631`/`:1657`/`:1870`/`:1899`/`:2996`/`:3028`/`:3155`、うち `:1870` は
決定1b のため `|| char_has_thumb_face`）を渡している。
**したがって本ADRの分岐は追加配線なしで既に覆われる。**
「含める」ではなく「既存ガードにより自動的に成立する（新規配線不要）」に
書き直すこと。節見出しの「（v5で更新）」も「（v6で更新）」へ。

### U7（T3 の(b)、未対応）. `auto_delegate_open_axis_consumed` の3義化は本PRで解消すべき

現在この引数名が意味するもの:

1. ADR-154 の consumed マーカー（`kp_stage_shadow_ime_toggle` が belief を
   OFF→ON へ動かした）
2. ADR-182 決定1/1b の `after_char_flush`（チョード誤判定）
3. （本ADR実装後）conv 軸トグルの抑止

**このリポジトリが繰り返し記録している失敗は、まさに「同じ合流点に
別の意味が乗り、洗い出しが漏れる」こと**である（issue #136 / ADR-119、
`fix-requires-evidence.md`「IME actuation 合流点」行）。

改名コストは小さい:
- 引数名 1 箇所（`nicola_fsm.rs:2325`）
- メソッド名 1 箇所（`fsm_types.rs:554` `suppresses_open_axis_actuation`
  → `suppresses_mode_key_actuation`）——呼び出し元7箇所は既にこのメソッドを
  渡しているので、呼び出し側は機械的置換
- doc コメント（`fsm_types.rs:550-553`、`nicola_fsm.rs:2310-2311`）

**本ADRの実装スコープに含めること**（別PRに切り出すと、3義目が乗った
状態で develop に残る）。

### U8（T8 の残り）. InputRelay をコア `ThumbSoloSpecialHandling` に載せる案は、値の供給経路が未設計。しかも InputRelay は**フォーカスごとに変わる動的値**

設計7 の方向（windows 層で握り潰すのではなくコアで `Fallthrough(None)` に
倒す）は正しい。しかし:

- `ThumbSoloSpecialHandling` はコア内で `self.thumb_solo_special_handling(vk_code)`
  が毎回組み立てる（`nicola_fsm.rs:977-1040`）。ソースは `NicolaFsm` の
  フィールド（`mode_key_muhenkan` 等）＝**config reload 起点で設定される
  静的な値**である。
- 一方 `AppImeProfile::InputRelay` は **フォーカス中のアプリごとに変わる**
  （`focus/classifier.rs::INPUT_RELAY_APPS`、`ADR-119`）。config reload 起点の
  setter では追随できない。
- **これは ADR-135 Phase 2 が実際に踏んだ罠と同型**である。同ADRは当初
  「書き込み時に親指キー判定をする」設計だったが、Opus レビュー2ラウンド目の
  R1 で「判定タイミングと書き込みタイミングが独立しているため古い判定が
  残る窓ができる」と指摘され、**消費時判定**（`Runtime::enrich_ime_relevance`
  で毎イベント評価）へ訂正している。

**要求**: InputRelay の判定を、(i) 毎イベント windows 層で評価して
`ClassifiedEvent`/`ImeRelevance` に載せてコアへ渡す（`enrich_ime_relevance`
と同じ流儀、ADR-135 の前例あり）か、(ii) コアではなく windows 層で
`ModeKeyRequest::ConvToggle` を受けた後に握り潰し、**かつ生キーを復活させる**
（＝「二重の空振り」を作らない）か、どちらかに決めること。
(ii) は ADR-119 の不変条件を守るために「生キーを出す」側の配線が要るので、
コア側で `Fallthrough(None)` に倒す (i) の方が筋がよい。

### U9（T7 の残り）. Enter 側のラッチ規律（INV-D）が明記されていない

設計3 は Exit 側（`rearm_after_failed_gji_exit` 相当）を明記したが、
Enter 側は「Win/Alt押下中または`!ime_open`のときは無送信で`false`が返る
——この場合belief補正を進めない」までしか書いていない。

左Shift版は **belief だけでなくラッチ（`toggle_held`）も** 成功時にのみ
立てる（`key_pipeline.rs:2313-2318` の `commit_enter_gji()` は
`send_gji_half_width_alnum_toggle` が `true` を返した場合のみ呼ばれる）。
`output/mod.rs:1226` の doc が INV-D としてこれを明文化している:

> 戻り値 `false`（未送信）の場合、呼び出し元は belief を進めてはならない
> （INV-D）。

設計3 に「**未送信ならラッチ（`toggle_held` 相当）も立てない**」を
1行足すこと。立ててしまうと、次のタップが Exit 方向に回り、
実 GJI がひらがなのままなのに awase は「半角英数から戻した」と
思い込む——`HalfWidthAlnumState` の commit-on-success 規律
（`half_width_alnum.rs:265-269`）が防いでいるのと同じ事故になる。

---

## Nits

### Nit-a. 「未解決の疑問」の見出しが round4 のまま／疑問3 が決着済みの論点を再掲

- L447「## 未解決の疑問（opus-adversarial-consult **round4** で検証してほしい点）」→ round5
- 疑問3「`session_keymap` が ATOK 以外のとき……前提条件に含めるべきか、
  常時有効にすべきか」は、設計原則の origin ゲート（`AtokPreset` のみ発火）で
  既に決着している。U2 の4段化と合わせて書き直すか削除すること。

### Nit-b（round4 Nit-4 未対応）. 「現状の機序」手順1〜7 が ADR-182 マージ前のログのまま

ADR-182 決定1/1b/1c が develop に入った現在、チョード誤判定経由の
漏出は再現しなくなっているはず。残るのは「Idle 起点の正常な単独タップ」
経由のみ。この1行を足すと、本ADRが何を直そうとしているかが明確になる。

### Nit-c（round4 Nit-5 未対応）. 疑問6（証拠のビルド特定）

`c8bc1adc`（warrant 強制）は 2026-09-18 23:21、ADR-182 マージ `740bf8be` は
その後。実機ログの取得時刻が分かればビルドは機械的に決まる。
「記録として残すべきか」ではなく「手順3 の `send 0x001A` が現行コードでも
起きるのか」という実質的な問いなので、疑問から外して実装前タスクにすること。

### Nit-d. 「composing 中の扱い」節（L428-430）が本文中の空スタブになっている

「提案する設計」5 で決定済みなら、この節は削除してよい（索引的な
リンクを残したいなら1行に縮める）。

---

## 次のラウンドへの要求（優先順）

1. **U1 を決める。** `ImeToggleKind` に origin を足す案を捨て、
   「windows 層で軸を決めてコアには別フィールドで渡す」案へ切り替えるか、
   足す案を採るなら (1) origin のコアへの運搬経路、(2) MS-IME レジストリ
   経路への影響、(3) ADR-176 `CalibratedModeKey`/config.toml への影響の
   3点すべてに答えること。
2. **U2 の4段化と、実装前の実機確認項目の追加。** 特に
   `custom_keymap_table` に Muhenkan/Henkan の行があるかは、
   **本設計が発火するかどうかを直接決める**。round2 R2 から4ラウンド
   持ち越している `session_keymap` 実値確認と同時に1回で済む。
3. **U3 を決める**（誤判定・書き込み失敗時の回復を「持つ」のか
   「持たない」のか）。ヒント／ゲートの二枚舌を解消する。
4. **U5（本文の誤った ADR-179 引用の削除）と U4（summary の整合）**。
   どちらも文言修正で、残したまま実装に入ると次セッションを誤導する。
5. U6・U7（引数名の3義化解消を本PRスコープへ）・U8（InputRelay の
   動的値問題）・U9（Enter 側ラッチ規律）。
6. Nit-a〜d。
