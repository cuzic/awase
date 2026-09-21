# ADR-184 opus-adversarial-consult round3

対象: `docs/adr/184-gji-atok-muhenkan-toggle-awase-owned-eisu-hiragana.md`（v4）
前回: `docs/adr/184-opus-review-round2.md`（Blocker 4 / Must-fix 9 / Nits 4）
レビュー日: 2026-09-19 / ブランチ `feat/adr178-mode-key-actuation-and-tsfnative-rescue-teardown`（HEAD `c8bc1adc`）
方式: 読み取りのみ。v4 の主張はすべて実コード（`src/types.rs`・`src/engine/fsm_types.rs`・`src/engine/nicola_fsm.rs`）と兄弟ADRで裏取りした。

---

## 総評

**R1（撤回主張の残存）は解消された。R4（Passthrough の位置づけ）はユーザー
訂正により明快になり、本ADRの動機は正当である。**

**しかし round3 で、設計の骨格そのものに関わる発見があった。**
v4 が「R3 の議論で新たに明確になった一般原則」として新設した
「設計原則: Toggle か冪等キーかで扱いを変える」節は、**ADR-179 決定2 が
既に確立し、`src/types.rs::ModeKeyActuationOwner` として現ブランチの
コードに実装済みの分類そのもの**である。しかも `nicola_fsm.rs::
resolve_delegate_to_open_axis`（2135-2187行）には、その原則に従った
`Toggle → 明示actuate` / `TurnOn|TurnOff → FollowOnly＋生キー配送` の
分岐が**既に存在する**。

これは本ADRにとって**悪い知らせではなく、良い知らせ**である。原則が
独立に2度導かれたのだから確度は高い。だが帰結は大きく変わる:

> **ADR-184 の真に新しい主張は「Toggle か否か」ではなく、
> 「既存の Toggle 分岐が `ImeOpenRequest::Explicit(ShadowImeAction::Toggle)`
> ＝ open 軸を反転させるが、GJI ATOK の無変換の実挙動は conv 軸であり、
> open 軸に写像した時点で誤りである」という一点に絞られる。**

この形に書き直せば、本ADRは「新しい機構を1つ足す」ではなく
「既存の決定点1箇所の写像先を直す」になり、ADR-178（領域A撤去）の
方向とも、`fix-requires-evidence.md`「IME actuation 合流点」の
「合流点を増やさない」という要請とも衝突しなくなる。
現状の「提案する設計」4（優先順位3の**手前**に新しい no-op 分岐を足す）は、
既存の決定点を迂回して合流点を1つ増やす形になっており、逆方向である。

重大度の内訳: Blocker 3件（すべて新規）/ Must-fix 10件（継続7・新規3）/ Nits 4件。

---

## round2 指摘の対応状況

| # | round2指摘 | v4での対応 | 判定 |
|---|---|---|---|
| R1 | 撤回主張がsummary/本文に残存 | summary・手順3/5を書き換え、ダングリング参照解消 | **対応済** |
| R2 | ATOK前提とatok.tsvの矛盾 | 「Mozc本家とGJIの乖離」で説明、判定を「ヒント」扱いに | **不十分**（S2・S3） |
| R3 | ADR-182対照テストの非互換 | 「設計原則」節を新設しcarve-outで両立と整理 | **部分**（S1・S4） |
| R4 | 既定設定では発生しない | ユーザー訂正を反映、Passthroughは広く使われる設定と明記 | **対応済** |
| M11 | `AssumedReason`の新variant | 未反映 | **未対応** |
| M12 | コア→windows層の受け渡し経路 | 未反映 | **未対応**（S1により論点が明確化） |
| M13 | 優先順位1・2との関係 | 未反映 | **未対応** |
| M14 | `HalfWidthAlnumState`転用可否 | 未反映（L295が依然流用を示唆） | **未対応** |
| M15 | open軸依存・commit規律 | 未反映（L191が依然「実機検証済み」） | **未対応** |
| M16 | ATOK下で注入経路未検証 | 未反映 | **未対応** |
| M17 | 無変換3連打 `engine_off_solo_repeat_vk` | 未反映 | **未対応** |
| M18 | InputRelay / `conv_mutation_allowed` | 未反映 | **未対応** |
| M19 | 証拠のビルド特定 | 手順3に「未記録」と明記、疑問1へ昇格 | **対応済**（実測は未実施） |
| N1 | wikilink | L7・L76 とも平文「ADR-183参照」に修正 | **対応済** |
| N2 | `resolve_pending_ thumb_as_single` の折り返し | L131-132 に残存 | **未対応** |
| N3 | BUG-015 同型の根拠 | 該当段落ごと削除された | **対応済** |
| N4 | statusのB1表現 | 「診断の根拠に使わない」と正しく表現 | **対応済** |

---

## Blocker

### S1. 「設計原則」は ADR-179 決定2（`ModeKeyActuationOwner`）として既に確立・実装済みであり、本ADRの新規性と挿入位置はそれに合わせて絞り直す必要がある

**該当**: 「設計原則: Toggle か冪等キーかで扱いを変える（ユーザー確認により確定）」節（L229-249）、「提案する設計」4（L213-222）

**実コードの確認**:

`src/types.rs:175-202`（ADR-179 決定2、現ブランチに実装済み）:

```rust
pub enum ModeKeyActuationOwner {
    NotAModeKey,
    FsmDelegate,
    /// awase自身がbelief書き込み・実actuationの両方を行う。静的
    /// `shadow_action`（Hiragana/Katakana/Alphanumeric/DBE系）、および
    /// 無変換/変換のToggle分類（非冪等、awaseが唯一の変更主体で
    /// あるべき）はここに属する。
    AwaseExplicit,
    /// beliefはawaseが書くが、実IME状態の変更は物理キー配送により
    /// GJI/MS-IME自身が行う。awaseは明示actuateも`ActivationSync`の
    /// 自動echoも一切発行しない。無変換/変換のOn/Off分類（非親指キー
    /// 設定時）専用。
    PhysicalDelivery,
}
```

`src/engine/fsm_types.rs:689-700`:

```rust
pub enum ImeOpenRequest {
    Explicit(ShadowImeAction),
    /// ……`delegate_to_open_axis`がユーザー設定によりパススルーへ辞退した
    /// 場合（TurnOn/TurnOff 方向のみ、**Toggleは非冪等なため常に
    /// `Explicit`のまま**）に使う。
    FollowOnly(ShadowImeAction),
}
```

`src/engine/nicola_fsm.rs:2168-2185`（`resolve_delegate_to_open_axis`、
Henkan/Muhenkan の Passthrough 設定時）:

```rust
Some(_) => {
    // Henkan/Muhenkan、Passthrough設定。
    if matches!(open_axis_action, ShadowImeAction::Toggle) {
        if composing { return DelegateResolution::Fallthrough(None); }
        explicit_resolve()                       // ← awase が明示 actuate
    } else if composing {
        DelegateResolution::Fallthrough(None)
    } else {
        // TurnOn/TurnOff: 辞退してModeKeyConfig側の生キー送出に
        // 委ねるが、belief追随だけは今ここで確定させる。
        DelegateResolution::Fallthrough(Some(open_axis_action))   // ← FollowOnly
    }
}
```

v4 L233-243 の原則（「Toggle は awase が能動的に actuate する必要がある /
On・Off は follow するだけでよい」）は、**この3つと一字一句同じ内容**である。
v4 はこれを「R3 の議論で新たに明確になった」と書き、ADR-179 決定2 にも
`ModeKeyActuationOwner` にも `resolve_delegate_to_open_axis` にも触れていない。

**なぜ Blocker か（2点）**:

1. **本ADRの新規性の所在が誤認されている。** 原則が既にあるなら、
   本ADRが主張すべき新規性は「原則」ではなく次の一点に絞られる:

   > 既存の Toggle 分岐は `ImeOpenRequest::Explicit(ShadowImeAction::Toggle)`
   > を返す。`ShadowImeAction::resolve(current_open) -> bool`
   > （`types.rs:156-162`）が示すとおりこれは **open 軸の反転**である。
   > しかし GJI ATOK の無変換の実挙動は **conv 軸**（ユーザー実測: IME OFF
   > では不変、IME ON で ひらがな⇔半角英数）。**既存の Toggle 分岐は、
   > この環境では軸を取り違えた actuation を発行する。**

   これは「新機能」でも「新しい原則」でもなく、**既存の1分岐のバグ**である。
   しかも `gji_thumb_key_ime_toggle` が既定 `false` でこの分岐が到達不能
   （ADR-182 が実機で `Fallthrough(None)` を確認）なため、**まだ誰も踏んで
   いない**。フラグを有効化した瞬間に「無変換を叩くと IME が閉じる」という
   実害が出る、という形で記述できる。

2. **「提案する設計」4 の挿入位置が、既存の決定点を迂回して合流点を増やす。**
   v4 は「優先順位3/4に入る前に no-op 相当の分岐を追加」としているが、
   Toggle/冪等の判別は**優先順位3の中（`resolve_delegate_to_open_axis`）で
   既に行われている**。手前に別の分岐を置くと、同じ「Toggle かどうか」の
   判定が2箇所に分かれる。これは `.claude/rules/fix-requires-evidence.md`
   の「IME actuation 合流点（新しい gate/precondition を足す場所、ADR-119）」
   行が列挙する失敗（issue #136: gate を1箇所に置いて他を素通しさせた）の
   **鏡像**であり、ADR-135 が Phase 2 v1 を撤回した理由（「`shadow_action`
   を既に持つキーに対し2経路が同一打鍵で二重に発火する」）とも同型である。

**要求**: 「設計原則」節を「ADR-179 決定2 の `ModeKeyActuationOwner` が
既に確立した分類を再確認した」と書き換え、本ADRの決定を
**`resolve_delegate_to_open_axis` の `Toggle` 分岐の写像先を open 軸から
conv 軸へ変える**（＝ `explicit_resolve()` が返す `ImeOpenRequest::Explicit`
を、conv 軸の要求に差し替える）という形に絞り直すこと。
挿入位置を「優先順位3の手前」ではなく「優先順位3の中」にすることで、
判定が1箇所に留まり、M13（優先順位1・2との勝敗）も既存の構造がそのまま
答えになる（1・2 は既に 3 より前で return する）。

---

### S2. 「ATOK判定はゲートか、ヒントか」が ADR 内の4箇所で相互に矛盾している

**該当**: L113-121（背景・経緯）、L186-189（提案する設計1）、L245-249（設計原則）、L321-323（疑問5）

同一文書の4箇所が別のことを言っている:

| 箇所 | 主張 |
|---|---|
| L113-121 背景・経緯（**R2対応で新設**） | 「`classify_mode_key_ime_action` の ATOK 判定は……**絶対に正しいゲートとしては使わない**——あくまで『Toggle 系のモードキーである可能性が高い』という**ヒント**として使い」 |
| L186-189 提案する設計1 | 「GJI キーマップが ATOK プリセット……**であることを前提条件に含める**」 |
| L245-249 設計原則（**R3対応で新設**） | 「本設計（awase 能動トグル）が適用されるのは**`classify_mode_key_ime_action` が `Toggle` を返す場合に限る**」＝絶対のゲート |
| L321-323 疑問5 | 「本設計は `ImeToggleKind::Toggle` 判定を**前提条件に含めるべきか、それとも常時有効にすべきか**」＝未決 |

**なぜ Blocker か**: この判定がゲートかヒントかで、設計の帰結が正反対になる。

- **ゲート**なら: ATOK 以外（MSIME/CUSTOM/判定不能）では本設計は発火せず、
  従来どおり生キーが GJI へ行く。ADR-182 の対照テストとの carve-out
  （L261-284）はこれを前提に成立する。
- **ヒント**なら: 判定が外れていても本設計が発火しうる。すると
  ADR-182 の対照テストとの carve-out の境界が定まらず、S4 の問題が
  解けなくなる。また「ヒントに従って conv を書いたが、実 GJI は
  open 軸トグルだった」場合、awase が conv=Eisu を書き、GJI が open を
  閉じる、という**両軸が同時に動く**最悪ケースになる。

R2 対応（ヒント化）と R3 対応（ゲート化）が、互いを知らずに別の節へ
追記された結果である。**どちらか一方に決めること。** S1 の整理に従えば
「ゲート」が自然である（`resolve_delegate_to_open_axis` は
`special.delegate_to_open_axis` が `Some` でなければ即 `Fallthrough(None)`
であり、構造上すでにゲートになっている）。

---

### S3. R2 の解消論拠（「Mozc 本家と GJI の実装に乖離がある」）は、R3 の carve-out が依存する `Toggle` 判定そのものを掘り崩す

**該当**: L104-121（R2の解消）と L245-249（Toggle をゲートに使う）

**論理の問題**: awase が `Muhenkan → Toggle` と判定する根拠は、
`atok.tsv` の**同じ2行**である:

```
DirectInput    Muhenkan  IMEOn                 ← IME OFF なら開く
Precomposition Muhenkan  CancelAndIMEOff       ← IME ON なら閉じる
```

`awase-gji-config::keymap::extract_ime_keys` は
`STATUSES_WHEN_IME_OFF`/`STATUSES_WHEN_IME_ON` の対から
「OFF時にOn、ON時にOff ⇒ `Toggle`」を導く（BUG-115「設計」節2）。

v4 は「実測は IME OFF → **不変**」と述べ、その食い違いを
「Mozc 本家由来の静的知識が GJI の実際の挙動とは異なる」で説明した。
**それは1行目（`DirectInput Muhenkan IMEOn`）が誤りだということ**である。
そして `Toggle` という結論は、**その誤った1行目を含む対から導かれている**。

つまり: 1行目が誤りなら、`Toggle` という結論は正しい導出ではない。
実挙動がたまたま（conv 軸の）トグルであることは、この導出の正しさを
支持しない。**誤った前提から偶然正しい形の結論が出ただけ**である。
この状態で `Toggle` 判定を carve-out の唯一のゲート（S2 で「ゲート」に
決めるなら、なおさら）に使うのは、「なぜ信用できるのか」に答えていない。

**さらに型レベルの問題（ADR-182 選択肢F と衝突）**:

- `ImeToggleKind`（`On`/`Off`/`Toggle`）と `ShadowImeAction`
  （`TurnOn`/`TurnOff`/`Toggle`、`resolve(current_open) -> bool`）は
  **どちらも open 軸の語彙**である。
- ADR-182「選択肢F」は、まさにこの点を根拠に
  「GJI(ATOKプリセット)の無変換単独タップは、IME ONのままひらがな→半角英数へ
  遷移する。**open状態が変わらない動作なので、open軸の `ShadowImeAction`
  では表現できない**」として FollowOnly 付与を却下している。
- つまり **conv 軸の挙動を `ImeToggleKind` で分類することは、
  既に ADR-182 が「できない」と結論した操作**である。

v4 は ADR-182 を related_adr に入れ関係節も設けているが、**選択肢F の
この結論には一言も触れていない**。carve-out のゲートが、兄弟ADRが
「表現できない」と明記した写像に依存している。

**要求**:
1. `Toggle` 判定をゲートに使い続けるなら、「なぜ 1行目が誤りでも
   `Toggle` という結論は使えるのか」を明示的に論じること。
   （例: 「`IMEOn`/`CancelAndIMEOff` という**対で割り当てられている**
   こと自体が『状態依存の反転キーである』ことの証拠であり、反転する軸が
   open か conv かは別問題」——この筋なら通る。ADR に書くこと。）
2. ADR-182 選択肢F を引用し、「open 軸語彙では表現できない」という結論を
   本ADRがどう乗り越えるのか（新しい conv 軸の要求型を作る、S1/M12）を
   明記すること。
3. R2 の最短の解決は依然として **atok.tsv の Muhenkan 全行と
   現在の `session_keymap` 実値を1回実機で確認すること**である
   （round2 R2 の要求は未実施のまま）。これを実装前提条件として
   status に残すこと。

---

## Must-fix

### S4. ADR-182 の対照テストは carve-out では直らない（機序を assert しているため）

**該当**: L261-284（ADR-182との関係）、疑問8（L332-339、自認済み）

v4 は「ADR-182『検証計画』2 (a)(b) に『ただし `ImeToggleKind::Toggle` と
判定された無変換/変換は対象外』を追加する形で両立できる」としている。
**方向は正しいが、次の3点が未解決である。**

1. **ADR-182 の対照テストは「効果」ではなく「機序」を assert している。**
   > (a) Idleから親指↓→148ms保持→親指↑……で**生の親指VKが出続けること**。

   本ADR実装後、この環境（ATOK Toggle）では生の親指VKは出ず、awase が
   conv を書く。**ユーザーから見た効果（半角英数になる）は同じ**なので、
   ADR-182 の意図（「Idle起点の単独タップによる半角英数化は意図した動作
   なので抑止しない」）は守られる。しかしテストの**書き方**が機序を
   固定しているため、carve-out を足すだけでは済まず、
   **ADR-182 の検証計画そのものを「効果」ベースに書き直す**必要がある。
   ADR-182 は v8・round7 でほぼ収束済みであり、この書き直しは
   ADR-182 側の再レビューを要する。**「両ADRは独立に実装可能」
   （L280-284）という結論は、この点で楽観的すぎる。**

2. **ADR-182 決定1c（採用済み）によるタイミング変化が未整理。**
   決定1c は「`ModeKeyConfig` を持つ親指キー（＝無変換/変換）は
   `on_timeout` の `PendingThumb` 腕で単独タップを**解決しない**」と
   変える。本ADRの新トグルは `resolve_pending_thumb_as_single` に乗るので、
   **発火タイミングが「100ms タイムアウト時」から「親指 KeyUp 時」へ
   後ろ倒しになる**。左Shift版が KeyUp 起点なので整合的ではあるが、
   「無変換を長押ししている間は conv が切り替わらない」という UX 変化を
   ADR に書くこと。

3. **ADR-182 決定1b の `candidate.is_some()` 条件との関係が未定義。**
   決定1b は「到着文字に親指面のかながある場合のみ」抑止する。
   本ADRの新トグルもこの条件に従うのか（従うべき——親指が shift として
   消費された打鍵を conv トグルにも使うのは決定1b が指摘する
   「1打鍵内の自己矛盾」と同型）を明記すること。

なお **v4 が書いた評価順序（ADR-182 のフラグが先 → 立っていなければ
Toggle/冪等で分岐）は正しい**。`resolve_pending_thumb_as_single` の
既存構造（`explicit_action_consumed` → `auto_delegate_open_axis_consumed`
→ 優先順位3）に、ADR-182 のフラグを同じ並びで足し、その内側で
S1 のとおり優先順位3の中を直す、という形で素直に実現できる。

### S5. ADR-178（領域A撤去）の方向との整合が本文にない

現ブランチは `feat/adr178-mode-key-actuation-and-tsfnative-rescue-teardown`
で、直近3コミットは `reassert` 機構の撤去（`f83084b3`）、force-ON 機構の
撤去（`621bf93c`）、warrant 強制（`c8bc1adc`）である。
ADR-182 は**自分から**この点に言及している:

> また現ブランチ（ADR-178領域Aの撤去作業中）に新しい対症療法を足すことに
> なる点は認識している。決定1は `resolve_pending_thumb_as_single` に条件を
> 1つ足すもので、撤去済みの `reassert`/force-on とは別の領域である。

ADR-184 には同等の記述が無い。S1 の整理（既存分岐の写像先を直す）を
採れば「機構の追加ではなく既存決定点の修正」と正直に書けるので、
**S1 とセットで1段落足すこと**。auto-memory
`feedback_design_change_means_teardown_not_addition`（「設計思想を転換したい
＝場当たり的パッチの撤去が主目的、成功基準は削除量で測る」）に直接該当する。

### S6. composing 中の扱いが未定義（新規）

`resolve_delegate_to_open_axis` の Toggle 分岐は composing 中を
**fail-closed** にしている（`nicola_fsm.rs:2172-2176`）:

> composing 中は fail-closed に倒す。誤って true でも suppress に落ちる
> だけだが、誤って false で Toggle(→OFF) すると composition を復旧不能に
> 破棄する。

一方、左Shift版の conv トグルは **composition 中でも発火する**
（ADR-107 決定5 の緩和、`output/mod.rs:1255-1268`、BUG-25追補5 で
preedit 非破壊を実機確認）。**軸が違えば正しい判断も違う**——
open 軸なら composition 破棄のリスクがあるが、conv 軸なら（左Shift版の
実績どおり）非破壊でありうる。

S1 のとおり Toggle 分岐の写像先を conv へ変えるなら、
**この `if composing { Fallthrough(None) }` も同時に見直し対象になる**。
現状の ADR には composing の記述が1行も無い。変えるのか変えないのかを
明記すること（変えないなら「composing 中は無変換タップで conv が
切り替わらない」という仕様になる）。

### M11（継続）. `AssumedReason` の新 variant も必要

exit 側の実コード（`key_pipeline.rs:2701-2707`）は
`InputModeState::AssumedRomaji { reason: AssumedReason::UserHalfWidthAlnumToggleOff }`
を使う。`InputModeApplyStrategy`（疑問4）だけでなく **`AssumedReason` にも
新 variant が要る**（journal 上で左Shift由来と無変換由来を区別する、
という M9 と同じ理由）。疑問4 を両方に広げること。

### M12（継続、S1で論点が明確化）. コア → windows 層の受け渡し型が無い

S1 のとおり `resolve_delegate_to_open_axis` の Toggle 分岐を直すなら、
返すべきものは `ImeOpenRequest::Explicit(ShadowImeAction::Toggle)` では
なく「conv 軸のトグル要求」である。しかし:

- `ImeOpenRequest` は `Explicit`/`FollowOnly` の2 variant で、
  どちらも `ShadowImeAction`（open 軸）を持つ（`fsm_types.rs:689-710`）。
- `ShadowImeAction::resolve(current_open) -> bool` は open 軸専用
  （`types.rs:156-162`）。
- コア `awase` クレートは ADR-019 により windows 層の
  `ImeToggleKind`/`InputModeApplyStrategy` を参照できない。

したがって **新しい型（例: `ImeOpenRequest` に `ConvToggle` variant を
足す、または `ResolvedAction` と並ぶ第3の戻り値を作る）が必須**であり、
これが本ADR最大の実装コスト不確定要素である。設計に書くこと。
また「conv 軸の要求を core が持つこと」自体が ADR-019 の
「engine は事前分類済みイベントのみ受け取る」と整合するかも要検討
（conv 軸は既に `ConvMode`/`InputModeState` としてコアにあるので
整合しうるが、明記が要る）。

### M13（継続、S1で大半が解消見込み）. 優先順位1・2との関係

S1 のとおり優先順位3の**中**を直すなら、1（`dedicated_fn_key`）と
2（`muhenkan_solo_tap_ime_action`）は既に3より前で return するため、
勝敗は自動的に「1・2 が勝つ」で確定する。**ただし BUG-115 F5 が記録する
「専用Fnキーが delegate を黙って無効化する」非対称**は本ADRにも波及する
（`warn_thumb_key_toggle_if_needed` の F5 診断のメッセージが
「open 軸の delegate が無効化される」前提の文面のままになる）。
診断メッセージの更新要否を書くこと。

### M14（継続）. `HalfWidthAlnumState` 転用可否

L295 は依然「本設計は awase 自身が状態を持つ（`HalfWidthAlnumState.toggle_held`）」
と書いている。round1 M3 / round2 M14 の3点（ラッチ共有か分離か、
`entry_policy` kill switch の適用範囲、`ShiftKeyUpKind` 4値判定は
無変換側では不要）が未回答。

補足: S1 の整理を採ると、状態の所在はさらに問題になる。
`resolve_delegate_to_open_axis` は**コア側の `&self` 純粋判定**であり、
`HalfWidthAlnumState` は **windows 層（`platform_state.gate`）**にある。
「今トグルが ON か」をコアが知る手段が無いので、
`ShadowImeAction::Toggle` と同じく「windows 層が現在状態を見て反転させる」
形（コアは `ConvToggle` を要求するだけ）にするのが自然。
その場合 `HalfWidthAlnumState.toggle_held` を共有するか別ラッチにするかの
判断は windows 層側の問題として切り分けられる。

### M15（継続）. `send_gji_half_width_alnum_toggle` の open 軸依存と commit 規律

`output/mod.rs:1230-1273` の3つのゲート（modifier / `!ime_open` / composition）と、
`kp_send_gji_restore_exit`（`key_pipeline.rs:2381-2410`）の
`prepend_synthetic_shift_up == false` で**未送信でも `true` を返し
belief を進めてしまう**問題（＝BUG-25追補3 と同型の実害）が未反映。
round2 M15 の表をそのまま本文へ取り込むこと。

### M16（継続）. ATOK 下での注入経路が未検証／`VK_DBE_ALPHANUMERIC` の前科

L191 の「GJI 向けに実機検証済み」は BUG-25追補5（2026-08-27、
CUSTOM または MSIME 環境）の検証。ATOK 下は未検証。
`VK_DBE_ALPHANUMERIC` は ADR-135「スコープ確定」節が
「過去に複数回振り出しに戻った論点」として明示的にスコープ外にした
キーである。S3-3 の atok.tsv 再取得の際に `Eisu`/`Hiragana` 行も
同時に確認すること。

### M17（継続）. 無変換3連打 `engine_off_solo_repeat_vk`

3連打で conv が3回トグル（奇数＝半角英数で終端）した上でエンジンOFF。
`conv_mutation_allowed`（`ConvModeAuthority::UserOwned`）は3打目自身が
authority を奪うため順序依存の穴になる。
ADR-182 決定1c が同キーを明示的に carve-out しているのと同じ除外が要る。

### M18（継続）. `AppImeProfile::InputRelay` と `conv_mutation_allowed`

生キー合成を止めた上で awase も conv を書かないと、ADR-119 が明文で
禁じた「二重の空振り」に新規に該当する。BUG-115「Phase 3設計上の
既知の限界」節が、`Decision::Consume` 経路には `transport.rs::plan` の
Allow が構造的に効かないことを既に記録している。

### M20（新規）. 変換（Henkan）側の扱いが「未検証」のまま設計に組み込まれていない

疑問2 は「変換側も同じ conv 軸トグルになっている可能性が高いが未検証」
と書くが、S1 の整理では `resolve_delegate_to_open_axis` の Toggle 分岐は
**Henkan/Muhenkan を区別しない**（`special.delegate_to_open_axis` は
VK ごとに解決済みの値が入るだけ）。つまり**実装すると自動的に変換側にも
効く**。BUG-115 が既定 OFF にした理由2（「ATOKでは変換・無変換の両方が
Toggle になり、NICOLA親指キー2本ともIME切替を持つことになり露出が2倍」）が
そのまま効く。「変換側は未検証だが同じ挙動になる」ではなく、
**「実装すると必然的に両方に効く。変換側も実測するか、VK で明示的に
限定するか」**を決定として書くこと。

---

## Nits

### N2（再掲・未修正）. `resolve_pending_ thumb_as_single` の行折り返し

L131-132。grep できなくなる。`resolve_pending_thumb_as_single`（改行位置を
ハイフンやバッククォート外へ）にすること。

### N5. 見出しの版数が本文と合っていない

- L177「## 提案する設計（決定案v2、round1指摘を反映）」← 実体は v4
- L251「## ADR-181/ADR-182 との関係（round1 B1/M1/B5 対応で新設）」← v4 で全面改訂済み
- L301「## 未解決の疑問（opus-adversarial-consult **round2** で検証してほしい点）」← round3 向け

### N6. 「なぜ BUG-115 の『Toggle 既定 opt-in OFF』の理由がそのまま適用できない可能性があるか」節が疑問3 と重複している

L286-299 と L311-319（疑問3）が同じ議論をしている。疑問3 の側で
結論（「`muhenkan_solo_tap_always_suppress=false` を選んだユーザーに限り
既定で有効、新しい別フラグは不要」）まで出ているので、L286-299 は
その結論へのリンクに縮めてよい。BUG-115 の理由2（親指キー2本への露出倍増）は
M20 と直結するので、そちらへ移すのが自然。

### N7. `related_adr` に ADR-119 / ADR-153 / ADR-154 が無い

本ADRが触る `resolve_pending_thumb_as_single` の既存ガードは
ADR-153（`explicit_action_consumed`）・ADR-154
（`auto_delegate_open_axis_consumed`）由来であり、M18 は ADR-119 を参照する。
3つとも `related_adr` に加えること。

---

## 次のラウンドへの要求（優先順）

1. **S1 の整理を本文へ反映する。** 「設計原則」を ADR-179 決定2 の
   再確認と位置づけ、本ADRの決定を
   「`resolve_delegate_to_open_axis` の `Toggle` 分岐の写像先を
   open 軸から conv 軸へ変える」に絞る。これが決まると M13 が大半解消し、
   S5（ADR-178 との整合）も素直に書ける。
2. **S2 を決める**（ATOK 判定はゲートかヒントか）。S1 の構造に従えば
   ゲートが自然。4箇所の記述を1つに揃える。
3. **S3 に答える**（誤った1行目から導かれた `Toggle` をなぜ信用できるのか、
   ADR-182 選択肢F の「open 軸語彙では表現できない」をどう乗り越えるか）。
   併せて **atok.tsv の Muhenkan/Eisu/Hiragana 全行と現在の
   `session_keymap` 実値の実機確認**（round2 R2 から未実施）を
   実装前提条件として status に固定する。
4. **S4 を ADR-182 側と調整する**（対照テストを「機序」から「効果」へ
   書き直す必要がある点、決定1b/1c との関係）。ADR-182 は round7 まで
   進んでいるので、先に ADR-182 側へフィードバックする方が安い。
5. **M12（conv 軸の要求型）を設計に入れる。** 実装コストの最大の不確定要素。
6. S6・M20・M11・M14〜M18 を本文へ反映。
7. N2・N5〜N7。
