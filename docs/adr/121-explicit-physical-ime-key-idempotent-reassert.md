# ADR-121: 物理 IME 訂正キーの no-op 時に、冪等な再送を追加で試みる（BUG-37 部分対策）

## ステータス

**設計完了（未実装）。Opus 2体による敵対的レビュー round 1〜3 完了、両者
承認。** premortem_reviewer は round 3 で「承認」、architect は round 3 で
残っていた 3 点（(3) 単独省略の実装可能性・適用範囲の明記・機構的主張の
訂正）を指摘し、それらを反映した本版で収束。**BUG-37 の「解決」ではなく
「欠落経路の補填＋診断能力の追加」として位置づけること**（round 2 architect
総合判定、round 3 でも維持）。実装に進んでよいが、`docs/known-bugs.md`
BUG-37 は本 ADR の実装だけでは「解決」にせず、実機ソークで観測状態の
改善を確認するまで「実装済み・効果検証中」に留めること。**round 1 で
当初案の重大な欠陥が複数判明し、本稿は全面改訂版。** 改訂差分は
文末「round 1 レビューでの主な訂正」参照。

## 背景

### BUG-37（`docs/known-bugs.md`）で確定していたこと

2026-07-06、Ctrl+T 等の同一プロセス内フォーカス移動で IME belief が実状態と乖離した際、
訂正手段の一つである物理 IME キー（かな/変換等）押下が
`kp_stage_shadow_ime_toggle`（`runtime/key_pipeline.rs`）の無条件 no-op ガードに
遭遇し、**awase 自身の合成的な訂正書き込み（apply-ime）を発火させない**ことが
判明した。該当コード（現行 `develop`、行番号は本 ADR 起票時点）:

```rust
// runtime/key_pipeline.rs:935 付近
if self.platform_state.ime.effective_open() == current {
    log::debug!(
        "[shadow-toggle] no-op: vk=0x{:02X} action={:?} source={:?} \
         effective_open は既に {} → apply-ime 見送り",
        event.vk_code, action, kind, current,
    );
    // ...ObservedEisu 救済のみ行い、実 OS への書き込みは一切しない
    return false;
}
```

belief（`effective_open()`）が既に押されたキーの意図する方向と一致していれば、
「変化なし」とみなして awase 自身の apply-ime をスキップする。**これ自体は
「物理キーが OS に届くかどうか」とは独立**（後述「物理キーは既に OS に
届いていた」節）。当時の修正（`b90a7c31`）は Stage 1 のみで、「同一プロセス内の
軽量フォーカス移動でも belief=ON なら次入力前に再プライムする」という間接的な
緩和策に留まり、この no-op ガード自体は今も無条件のまま残っている
（`docs/known-bugs.md` BUG-37「未解決（Stage 1 の限定的な修正）」節）。

### 2026-09-02、実機で再現（不具合報告 `01M1GVNR840NZ3XWRX0JPDSQR7`）

タスクトレイの「不具合を報告」機能経由で journal/app.log 付きの報告が届き、
本 ADR 起票の直接のきっかけになった。症状は「よしっ」と入力したのに
「yosiltu」とリテラル出力（Windows Terminal、MS-IME、`Windows.UI.Input.
InputSite.WindowClass`、`app_kind=Uwp`/`ImePolicyProfile::TsfNative`、
`gji_state=MsImeStrategy`）。

journal と app.log の突き合わせで判明した事実（詳細は `docs/bug-reports-triage.md`
該当行）:

1. フォーカスが Windows Terminal → Chrome（別ウィンドウへ一瞬）→ Windows Terminal
   と往復した直後から、実 OS の IME 状態が沈黙裏に OFF へ落ちていたと見られる
   （TsfNative は observe 不能のため誰も気づけない）。
2. ユーザーが違和感に気づき、物理「かな」キー（`VK_DBE_HIRAGANA`, `0xF2`）で
   **3 回**訂正を試みた。3 回とも `[shadow-toggle] no-op: vk=0xF2 action=TurnOn
   source=PhysicalImeKey effective_open は既に true → apply-ime 見送り` が発火した。
3. 結果、ローマ字 `"yo"`/`"si"`/`"ltu"` が TSF 経由の生 VK として送信され、
   実 IME が OFF のまま合成されずリテラル出力になった。同じ語を 2 回タイプし、
   どちらも失敗している。
4. セッション末尾でフォーカスが観測可能なプロファイル（awase 自身のトレイ
   ウィンドウ、`OsPoll`）に移った瞬間、`IME snapshot: ime_on=Some(false)` →
   `[drift] correction: observed=false ≠ desired=true for 810ms` が発火し、
   実 IME がずっと OFF だったことが直接確認できた。

### 訂正: 物理キーは既に OS に届いていた——no-op 説は原因の半分でしかない

**当初の триage（`docs/bug-reports-triage.md`）は「no-op がユーザーの物理キー
操作自体を実 OS へ届かせていない」と記述したが、これは誤りだった。** round 1
レビュー（premortem_reviewer）の指摘を受けて 3 回の押下それぞれの直後のログを
再確認したところ、**3 回とも直後に
`[relay-passthrough] PassThrough idle: direct OS pass-through (vk=0xf2 down)`
が出ており、物理キー自体は 3 回ともネイティブに OS（MS-IME）へ届いていた**:

```
[hook] IME-mode vk=0xF2 down self_injected=false injected=false scan=0x70 extra=0x0
[shadow-toggle] no-op: vk=0xF2 action=TurnOn source=PhysicalImeKey effective_open は既に true → apply-ime 見送り
...
[relay-guard] vk=0xf2 down in_flight_ms=... has_pending=false output_in_flight=false
[relay-passthrough] PassThrough idle: direct OS pass-through (vk=0xf2 down)
```

これは `runtime/transport.rs::PhysicalKeyDisposition::plan` の設計どおりの
挙動である。`VK_DBE_HIRAGANA` は専用分岐を持ち、`is_tsf_mode && f2_warmup_owned`
のときのみ `Suppress`、それ以外は `Allow`（`transport.rs:182-187`）。
`f2_warmup_owned` は `warmup_coord.needs_f2_probe()` に等しく、
**`MsImeStrategy` では常に `false`**（`tsf/warmup/warmup_strategy.rs` の
GJI 専用 warmup 判定、MS-IME は F2 warmup を自前で必要としない）。したがって
今回のような「Windows Terminal + MS-IME」の組合せでは 0xF2 は無条件で
`Allow`（素通し）される。

**つまり実態は「MS-IME が正真正銘のネイティブ物理かなキー押下を 3 回受け取り
ながら、それでも自ら IME を開かなかった」ということである。**
`kp_stage_shadow_ime_toggle` の no-op ガードが握り潰していたのは
「awase が念のため送る合成的な追加の訂正」だけであり、これは症状の**一因
ではあるが唯一の原因ではない**——なぜ MS-IME がネイティブキーに応答しなかった
のかという、より深い謎が未解決のまま残る。

この訂正は 2026-08-18 の別調査（`docs/known-bugs.md` BUG-37 関連メモ、
このリポジトリの過去のセッション記録）が残していた「Ctrl+変換（`engine.rs::
ime_set_open_effects` 経由、no-op ガードを通らず無条件に `SetOpen` を発行
する別経路）を押しても復旧しなかった」という反例とも整合する——**両方とも
「awase 側は何らかの形で IME ON の意図を OS へ送っているのに、MS-IME 側が
反応しない」という同型の未解決症状**である可能性が高い。

### 本 ADR が実際に約束すること

上記の訂正を踏まえ、本 ADR の位置づけを以下のように改める（当初案の「ユーザーの
明示訂正操作だけは必ず実 OS に届く」という言い切りは撤回する——「範囲外」節・
「未解決のまま残る問題」節参照）。

**本 ADR が提案する再送（`VK_IME_ON`, `send_ime_mode_key` 経由）は、既に失敗が
確認された経路（ネイティブ `VK_DBE_HIRAGANA` の hook 再注入）とは、送信 VK・
送信 API（`SendInput` 直接）が異なる**——「ネイティブキーで 3 回失敗した」
という事実は「この別経路でも失敗する」ことを直接には意味しない。**しかし
保証もしない。** また round 2 レビュー（N-1）で判明したとおり、`VK_IME_ON`
自体は conv-mode に触れないが、送信直前に走る `romaji_pre_write()`
（`ime_controller.rs:336-340`）は MS-IME × ON 方向 × `belief_input_mode
!= ObservedKana` の条件で ROMAN ビットの IMC write を伴う（D1/D3 参照）
——「conv-mode に一切触れない」という当初の主張は不正確だったため訂正する。
本 ADR が実際に約束するのは:

1. no-op で終わっていた箇所に、機構的に別の追加試行を 1 回挟む（「試みる」
   であって「必ず直る」ではない）。
2. **初期実装のスコープは `VK_DBE_HIRAGANA`(0xF2) の `TurnOn` 方向のみ**
   （D1/D4 参照）。`VK_DBE_KATAKANA`(0xF1)/`VK_DBE_DBCSCHAR`(0xF4) は
   BUG-52 対策により no-op の有無に関わらず常に `Suppress` されるため
   （後述「D1」参照）、reassert が実装されれば理論上は「初めて何かが OS に
   届く経路」になるが、round 2 レビュー（R2-2）で「これらのキーがユーザーの
   『かなキー押下の別表現』なのか『独立したカタカナ/全角プレーン選択の
   意図的操作』なのかは実機で確認できていない」と指摘された——後者だとすると
   `VK_IME_ON`（open のみ、カタカナ化はしない）を送っても要求は満たされず、
   下手をすると ROMAN 補完がユーザーの意図的なカタカナ入力の妨げになる
   おそれもある。この解釈が実機で確定するまで、**0xF1/0xF4 は本 ADR の
   初期実装スコープに含めず、別途フォローアップとして扱う**。
3. **効果の検証は送信ログではなく、実際に観測された IME 状態で行う**
   （`docs/known-bugs.md` BUG-15 追補3: 送信ログは成功と出たが実モードは
   変わらなかった前例）。「テスト計画」節で明記する。

MS-IME がなぜネイティブキーに応答しないのかという根本原因の解明は本 ADR の
スコープ外（「未解決のまま残る問題」節）。

## 現在の実装から見つかった、活用できる既存資産

本 ADR は「現在の実装を元に」という指示のもと、2026-08-18 時点で設計されていた
（未実装のまま破棄された）`explicit_key_reassert.rs` 案をそのまま復活させるの
ではなく、その後に developed された以下の既存資産を土台にする。

### 1. `ActuationOrder` + `issue_actuation_order()` — ADR-087/ADR-090（`state/open_warrant.rs`, `state/actuation_chain.rs`）

ADR-087 で導入された「OS への実書き込みを許可してよいか」の授権判定
（`issue_open_warrant()`）と、ADR-090 で導入されたその起案ラッパー
`ActuationOrder` を使う。**`issue_open_warrant()` を直接呼ぶことはできない**
——`WarrantContext` の構築は `ImeStateHub::warrant_context()` 1 箇所に
固定されテストで守られており（`tests/architecture_guard.rs::
warrant_context_is_built_in_one_place`、INV-48）、`intent_store` フィールド
自体が private。正しい呼び出し方は、既存の全 actuation 経路（
`force_on_and_correct_romaji` 含む）が既に使っている
`self.issue_actuation_order(open, "explicit_key_reassert")`
（`runtime/mod.rs:375-385`）であり、これが内部で `warrant_context()` →
`ActuationOrder::issue()` → `issue_open_warrant()` を呼ぶ。

`kp_stage_shadow_ime_toggle` は物理キー押下のたびに
`write_physical_key()` → `record_explicit_intent()` →
`IntentStore::record(hwnd, target, source, tick_ms)` を**既に**呼んでいる
（`state/platform_state.rs:1230-1245`、no-op 分岐に入るかどうかに関わらず、
`effective_open()` のチェックより**前**に実行される）。つまり **no-op 分岐に
到達した時点で、`IntentStore` にはこの押下自身に由来する新鮮な
`ExplicitUserIntent` エントリが既に存在する**ため、`issue_actuation_order()`
は Step 1 で即座に warrant を発行するはずである。

`issue_actuation_order()`／`ActuationOrder` は現在、全ての actuation 経路
から呼ばれてはいるが ADR-090 §2.A の「A-1 shadow」フェーズに留まっている
——warrant を計算・ログ（journal の `[warrant-shadow] ... warranted`/
`would_have_blocked`）はするが、`into_actuation_shadow()` を通す限り
`None`（warrant 不成立）でも書き込みを止めない（「A-2」で実ゲート化する
予定、まだどの入口でも着手されていない）。**本 ADR はこの 1 箇所（物理
キー起点の明示的再送）に限って、`order.would_have_blocked()` を実際に
チェックし、`true`（授権不成立）なら書き込みを行わない——A-2 の最初の
限定的インスタンスになる**（D3 参照）。既存 2 箇所（`apply_force_on_
for_imm_broken`/`try_force_on_bootstrap`）の shadow モードや、
`is_eligible_for_ime_force_on()` の全面置換（ADR-087 §5 Phase 3 item15）
には一切手を触れない。

### 2. 冪等な OS 書き込みプリミティブ — `MsImeDirectStrategy`/`GjiDirectStrategy`（`ime_controller.rs`）

`force_on_and_correct_romaji()`（`runtime/mod.rs:877`、`apply_force_on_for_imm_broken`
から呼ばれる既存の周期的自己修復本体）が使っている書き込み経路は、**現在の
実装で既に「常に安全に再送してよい」プリミティブになっている**:

- `MsImeDirectStrategy`: ON = `VK_IME_ON`(0x16)、OFF = `VK_IME_OFF`(0x1A)。
  いずれも「開閉そのもの」を表す非トグルキーで、conv-mode に触れない
  （`ime_controller.rs:144-239` のコメント、2026-08-06 に旧
  `VK_DBE_HIRAGANA` モードキー方式から移行済み——BUG-50 の教訓で、
  「開く」と「かなに強制する」を1つの副作用に束ねていたことが原因だった）。
- `GjiDirectStrategy`: ON = `VK_IME_ON`、OFF = `VK_IME_OFF`。ON 方向のみ
  `view.control.shadow_on` を見て skip するガードがあるが、
  `force_on_and_correct_romaji()` は意図的に `applied=None` の view を渡す
  ことでこのガードを無効化している（`runtime/mod.rs:898-904` のコメント）。
- `KanjiToggleStrategy`（`VK_KANJI` トグル、非冪等）への到達不能性は
  下記「D1 の安全性の根拠」で独立に導出する。

### 3. VK レベルで「トグルか方向指定か」が既に型で分かれている（`vk.rs`）

```rust
pub enum ShadowImeEffect { TurnOn, TurnOff, Toggle }
```

`ImeKeyKind::shadow_effect()` の対応表:

| VK | 効果 | `PhysicalKeyDisposition::plan` の扱い（no-op 時、Blacklist プロファイル） |
|---|---|---|
| `VK_KANA`(0x15), `VK_IME_ON`(0x16), `VK_JUNJA`(0x17) | `TurnOn` | `Allow`（素通し） |
| `VK_DBE_HIRAGANA`(0xF2) | `TurnOn` | `is_tsf_mode && f2_warmup_owned` のときのみ `Suppress`。MS-IME(`f2_warmup_owned=false`)では常に `Allow` |
| `VK_DBE_KATAKANA`(0xF1), `VK_DBE_DBCSCHAR`(0xF4) | `TurnOn` | 既定設定では常に `Suppress`（BUG-52 対策） |
| `VK_IME_OFF`(0x1A) | `TurnOff` | `Allow`（素通し） |
| `VK_DBE_ALPHANUMERIC`(0xF0), `VK_DBE_SBCSCHAR`(0xF3) | `TurnOff` | 既定設定では常に `Suppress`（BUG-52 対策） |
| `VK_KANJI`(0x19) | `Toggle`（**非冪等**） | `Allow`（素通し） |

`kp_stage_shadow_ime_toggle` が使う `ShadowImeAction::{TurnOn,TurnOff,Toggle}`
はこの分類をそのまま引き継ぐ。

## 決定

### D1: `kp_stage_shadow_ime_toggle` の no-op 分岐で、条件を満たせば冪等再送を発火する

**round 2 レビュー（R2-2）を受け、初期スコープを `VK_DBE_HIRAGANA`(0xF2) の
`TurnOn` 方向のみに縮小する。** `action != ShadowImeAction::Toggle` という
より広い条件（`VK_KANJI` 以外の全物理 IME キー）は当初案だったが、
0xF1(`VK_DBE_KATAKANA`)/0xF4(`VK_DBE_DBCSCHAR`) は「かなキー押下の別表現」
なのか「独立したカタカナ/全角プレーン選択の意図的操作」なのか実機で未確認
であり（上記「本 ADR が実際に約束すること」節参照）、後者だった場合
`VK_IME_ON` の送信は要求を満たさない上に ROMAN 補完（D3 参照）が意図的な
カタカナ入力の妨げになりうる。実機で解釈が確定するまで対象から外す。
0x15/0x16/0x17/0x1A（`VK_KANA`/`VK_IME_ON`/`VK_JUNJA`/`VK_IME_OFF`）も、
今回の実機証拠（`vk=0xF2` のみ）が無いため対象に含めない——D4 で述べる
とおり本 ADR は TurnOff 方向自体を対象外にしているため 0x1A は元々対象外
だが、0x15/0x16/0x17 についても「対象を広げる強い理由が見つかるまでは
実証されたキーのみ」という同じ原則で対象外とする。

no-op 分岐（`effective_open() == current`）に到達し、かつ以下を全て満たす
場合、実 OS への冪等書き込みを 1 回発火する:

1. `event.vk_code == VK_DBE_HIRAGANA` かつ `action == ShadowImeAction::TurnOn`
   （初期スコープ。他の VK/方向への拡張は別途 ADR 追補で扱う——「D1 の
   安全性の根拠」節・「本 ADR が実際に約束すること」節参照）。
2. `!self.can_use_imm32_cross_process()`（Blacklist プロファイルのみ。Standard/
   ImmCross は既存の observation-based drift correction が既に対応しており、
   本 ADR の対象外——「範囲外」節参照）。
3. focus-settle 中でも実行する（BUG-16 が守ろうとしたのは IMC 宛先 hwnd が
   不定な書き込みであり、本 ADR の reassert は宛先を持たない `SendInput`
   ベースの `send_ime_mode_key` のみを使うため——`state/actuation_chain.rs`
   の `Captured`/`FocusImplicit` 区分参照、後者に該当）。ただし
   `self.ime_apply_should_defer()` が真の間は「settle 明けに 1 回だけ
   reassert する」pending フラグ（`pending_explicit_reassert: Option<TickMs>`
   相当、focus/hwnd 単位ではなくグローバルな単発フラグでよい）を立て、
   settle 明けの既存リフレッシュ tick（`schedule_settle_retry` が使うのと
   同じ 20ms/150ms タイマー）で消費する（round 2 premortem レビュー R2-4:
   「defer するだけで再試行の担い手が無いと、明示訂正が黙って消えるという
   本 ADR が解消しようとした症状がそのまま再現する」という指摘への対応。
   settle 中に送ってよいという判断自体に確証が持てない場合は、この
   pending フラグ方式を確実な代替として採用する）。
4. 直近の半角英数トグル復元処理（`kp_restore_kana_from_half_width`）が同一
   打鍵で発火していない（D6 参照。二重注入を避ける）。
5. D2 のデバウンスを通過する。

満たさない場合は現行どおり no-op のまま何もしない（ObservedEisu 救済のみ
既存どおり実行する）。

#### D1 の安全性の根拠（`Toggle` 除外・KanjiToggle 到達不能性の独立導出）

初期スコープを 0xF2/`TurnOn` 単独に絞ったため `VK_KANJI`（`Toggle`）は
条件1で自動的に除外され、以下の議論は**将来 0x15/0x16/0x17/0xF1/0xF4 等へ
対象を広げる際に同じ枠組みが使えることを示す**ために残す（現時点の
安全性そのものはこれに依らない）。

`!can_use_imm32_cross_process()`（Blacklist）スコープでは、以下 3 点から
`KanjiToggleStrategy` に到達しないことが導ける（`apply_force_on_for_imm_broken`
が Standard を除外しているから、という循環論法には依らない）:

1. `ActiveImeKind` は `GoogleJapaneseInput`/`MicrosoftIme` の 2 値のみで
   「IME 種別不明」は存在しない（`tsf/observer.rs:604-609`）。
2. `!can_use_imm32_cross_process()` では、`gji_direct_applicable(kind)` と
   `ms_ime_direct_applicable(kind, profile)` のどちらか一方が必ず真になる
   （`state/key_sequence_policy.rs:45-53`）。
3. チェーンが `KanjiToggle` へフォールスルーするのは `Failed` を返した
   ときだけであり（`state/actuation_chain.rs:211-213`）、`Failed` を
   返しうる機構は `ImmCross` のみ（同 `:196-198`）。`UnsafeToToggle`
   （Win キー押下中等）はフォールスルーに含まれない——非冪等な `VK_KANJI`
   を「Win キー押下中に安全に送れない」状態でさらに送る新経路を作らない
   ための明示的な設計（同 `:206-208` のコメント）。

したがって GjiDirect/MsImeDirect が走った時点でチェーンは止まり、
`KanjiToggleStrategy` には到達しない。**この不変条件は
`state/app_ime_policy.rs::caps_chains_have_no_unreachable_trailing_element`
と `ime_controller.rs::caps_chain_matches_legacy_all_scan` がテストで
固定している**——これらが崩れたら D1 の安全性の前提も崩れる。実装時は
この 2 テストへの依存をコメントで明記すること。

#### `PhysicalKeyDisposition::plan` との相互作用（BUG-46 の教訓との照合）

初期スコープの 0xF2（MS-IME）は、上表のとおり no-op 時にも `Allow`
（素通し）される——ネイティブキー自体が既に OS に届いている上に、本 ADR
の reassert が**追加で**冪等書き込みを送ることになる。これは意図的な
冗長送信であり、`docs/known-bugs.md` BUG-46（GJI の `SendInput(VK_IME_ON/
OFF)` と、素通しされた元の物理 KANJI 系キーの reinject が二重に actuate
し、**両者が競合する conv-mode 副作用を持っていた**ため最終状態が壊れた
事故）とは性質が異なると判断する: BUG-46 で実際に壊れたのは
`VK_DBE_DBCSCHAR`(0xF4) のような「開閉」と「プレーン選択」を1つの副作用に
束ねたキーであり、この副作用の競合が根本原因だった。0xF2 自身も歴史的には
「開く」と「ひらがなに強制する」を束ねたモードキーだったが（BUG-50）、
**本 ADR の reassert が送るのは `VK_IME_ON` であり、`VK_DBE_HIRAGANA`
そのものを再送するわけではない**——ネイティブ 0xF2 が持つ「ひらがな強制」
の副作用と、reassert の `VK_IME_ON`（open のみ、ただし D3 の
`romaji_pre_write` により ROMAN 補完を伴う）は、同じ「開く」方向を指す限り
競合しない。ただし 0xF1/0xF4 を将来スコープに含める際は、この判断を
個別に再検証すること（それらは BUG-52 対策で常時 `Suppress` されるため
「ネイティブキーとの冗長送信」という前提自体が成立しないが、意味論上の
不一致——「かなキーの別表現」か「独立したカタカナ要求」か——は別問題として
残る）。

```rust
// イメージ（実装時に確定させる詳細は D2〜D6 参照。初期スコープは 0xF2/TurnOn のみ）
if event.vk_code == crate::vk::VK_DBE_HIRAGANA
    && action == ShadowImeAction::TurnOn
    && !self.can_use_imm32_cross_process()
    && !half_width_restore_fired_this_keystroke
    && self.explicit_key_reassert_debounce_ok(tick_ms)
{
    if self.ime_apply_should_defer() {
        self.schedule_pending_explicit_reassert(new_val, tick_ms); // R2-4
    } else {
        self.reassert_explicit_physical_key(new_val, tick_ms);
    }
}
```

### D2: デバウンスは新しい短いクールダウン定数で行う（既存の `physical_key_held_ms` は使えない）

**当初案は誤りだった**（round 1 premortem レビューで発覚）。当初案は
`hook.rs::physical_key_held_ms()`（`PHYSICAL_KEY_DOWN_AT_MS`）で「新規押下か
auto-repeat か」を判定しようとしたが、2 つの理由で成立しない:

1. `PHYSICAL_KEY_DOWN_AT_MS` はフック内で `kp_stage_shadow_ime_toggle` が
   実行される**前**に更新される（`hook.rs:863-876`）ため、パイプライン段階に
   到達した時点では新規押下でも「既に押されている」状態にしか見えない。
   `ALT_L_WAS_DOWN` パターンはフック内部でスナップショットを取っている
   ため転用できない。
2. **`VK_DBE_HIRAGANA`（かなキー）が物理的に KeyUp を生成しない疑いがある。**
   典拠として引いていた `hook.rs` の BUG-14 コメントは「**外部注入された**
   `VK_DBE_HIRAGANA` down+up が hook 上では `0xF0 up` + `0xF2 down` に
   翻訳される」という注入イベントの話であり、物理押下自体が KeyUp を
   出さないことの一次証拠ではなかった（round 2 architect レビュー N-4 で
   指摘）。実際、今回の不具合報告の journal/app.log を確認したところ、
   3 回の物理 `0xF2 down` に対応する `0xF2 up`（KeyUp）は**一度も
   出現しなかった**（`grep "vk=0xF2.*up"` 0 件、journal の `KeyInput`
   エントリも `is_down: true` のみ）——これは KeyUp が本当に生成されて
   いないことの直接証拠ではなく「たまたま KeyUp 前に報告された」可能性も
   残るが、当初案（物理キー状態ベースのデバウンス）を採用しない理由と
   しては理由 1（フック更新順序）だけで既に十分であり、この点は
   参考情報として扱う。

**改訂案**: **実装の最初の一歩として、まず「かなキーを押しっぱなしにした
とき `VK_DBE_HIRAGANA` の `KeyDown` が OS の auto-repeat で繰り返し届くか」
を実機で確認する**（round 2 premortem レビュー R2-3）。届かないなら
——0xF2 が Windows 側でロッキングキー的に扱われ repeat しないなら——
デバウンス自体が不要になり、D2 は「新設しない」で完結する。これが
最も安全な選択である（前述の当初案・改訂案がいずれも repeat との区別を
誤って壊した経緯を踏まえ、まず repeat の有無という前提事実を確認する）。

repeat する場合のみ、以下のクールダウンを新設する: `last_reassert_ms:
Option<TickMs>` を 1 つ保持し（初期スコープは 0xF2/`TurnOn` 単独のため
VK 単位/方向単位で分岐する必要はない——round 2 architect レビュー N-3 が
指摘した「複数 VK が同じクールダウンバケツを共有し、常時 Suppress される
VK の唯一の到達経路を巻き添えにする」リスクは、スコープを単一 VK に絞った
ことで構造的に消える）、`now_ms.saturating_sub(last) >=
EXPLICIT_KEY_REASSERT_COOLDOWN_MS` のときのみ発火する。**これは時間長の
しきい値判定であり、`.claude/rules/tuning-constants.md` の実測義務の
対象になる**。実機測定が必要だが、参考として:

- 今回の実機ログでの 3 回の押下間隔は 25.1 秒・2.4 秒——いずれも数百 ms
  オーダーの妥当なクールダウンで十分間に収まる（ただし連打の下限を示す
  ものではない、上記 R2-3 参照）。
- **フォーカス変更でクールダウンをリセットする**（round 2 architect
  レビュー N-3）。belief ドリフトが生まれるのはフォーカス往復の直後で
  あり、直前のフォーカス窓で reassert がクールダウンを消費していると、
  新しい窓での最初の訂正が落ちる——今回の実機インシデント（Windows
  Terminal → Chrome → Windows Terminal の往復直後）がまさにこの状況。
- 実装時に実機で「かなキーを意図的に連打したときの最短間隔」を測り、
  その半分程度を下限としてクールダウン値を決定すること
  （`.claude/rules/tuning-constants.md` 準拠）。本 ADR は具体的な ms 値を
  確定させない——確定は実装 PR の本文に実測根拠を書くこと。

`force_on_retry`（ゲート B の状態）とは共有しない、独立した状態にする
（D1 冒頭で述べた「ゲート B を経由しない」という決定の維持）。

### D3: `issue_actuation_order()` + `would_have_blocked()` で授権する（A-2 の最初のインスタンス）

`reassert_explicit_physical_key(open, tick_ms)` は以下の手順で実装する:

1. `let order = self.issue_actuation_order(open, "explicit_key_reassert");`
   （`runtime/mod.rs` の既存ヘルパー。内部で `ImeStateHub::warrant_context()`
   → `ActuationOrder::issue()` → `issue_open_warrant()` を呼ぶ。target hwnd
   は `current_focus().unwrap_or(HwndId::NULL)` で自動的に解決される——
   `write_physical_key()` が `record_explicit_intent()` に渡したのと同じ
   `current_focus()` を、同一の同期処理シーケンス内で再度読むだけなので、
   フォーカスが割り込む窓は無い）。
2. `if order.would_have_blocked() { /* 何もせず終了。ログのみ */ return; }`
   ——**これが「A-2 実ゲート化」の本体**。`IntentStore` に Step 1 が
   参照する新鮮な `ExplicitUserIntent` エントリが既にあるため、通常は
   ここを通過するが、Step 0（`SafetyValve`、`PanicReset` 等）が先に評価
   されるため、まれに block されることがある——それは既存の安全弁の
   仕様どおりであり、本 ADR が悪化させる余地はない。
3. 通過したら `order.into_actuation_shadow()`（または実装時に妥当な
   `Actuation` 状態遷移）を経由し、`ImeController::apply` 相当のチェーンで
   実際に `SendInput` する。`force_on_and_correct_romaji()` と同じ書き込み
   経路（`platform.build_ime_control_view(None)`）を、方向 `open` を
   パラメータ化して使う（ON 固定だった `force_on_and_correct_romaji()`
   自体は変更せず、並行する新関数として実装する——D4 参照）。**このとき
   `view.belief_input_mode = self.platform_state.ime.input_mode();` を
   必ず明示的に設定すること**（round 2 architect レビュー N-1）。
   `apply_mechanism`（`ime_controller.rs:336-340`）は `strategy.apply()`
   の前に `romaji_pre_write()` を実行し、`open && mechanism ∈ {ImmCross,
   MsImeDirect} && kind == MsIme && belief_input_mode != ObservedKana`
   の条件で ROMAN ビットの IMC write を行う（`state/actuation_chain.rs:
   243-257`）——これは今回のスコープ（MS-IME × TsfNative × ON 方向）に
   正面から該当する。`build_ime_control_view(None)` は `belief_input_mode`
   を `Unknown` 固定で作るため、この 1 行を足さないと「ユーザーが意図的に
   かな入力を選んでいれば ROMAN 補完で上書きしない」という `ObservedKana`
   保護が効かない——`force_on_and_correct_romaji()` がこの行を持つのは
   まさに同じ理由（2026-08-08 Opus レビュー指摘 N2 への対応）であり、
   省略するとその回帰を再導入することになる。「`VK_IME_ON` は conv-mode
   に一切触れない」という当初の安全性論証はこの 1 行を前提にしてのみ
   成立する——訂正済み（「本 ADR が実際に約束すること」節参照）。

**`force_on_attempt_allowed()`（ゲート B、`FORCE_ON_RETRY_COOLDOWN_MS`=3000ms
のクールダウンと「`applied` が既に一致していたら送らない」ルール）は経由
しない**——ゲート B は「20ms/500ms 周期リフレッシュへの相乗り」を対象にした
設計であり、人間の指の反応速度でしか発火しない明示的な 1 回のキー押下に
同じ抑制をかけると、今回の事故そのもの（周期リフレッシュも明示キーも両方
沈黙する）を再現する。

**書き込み後に `on_ime_apply_complete()` を呼ぶかどうか（round 1 レビュー
指摘）は、round 2 で architect/premortem 両レビューから踏み込んだ検証を
受け、次のように決定する。** `on_ime_apply_complete()`（`runtime/mod.rs:
504-552`）は 4 つのことを行う: (1) journal へ `ImeOpenApplied` 記録、
(2) `post_ime_refresh()`（20ms 無条件タイマー、`UnsafeToToggle`/
`Aborted(GenStale)` でも必ず張る——2026-08-08 修正、これを落とすと
「無関係な次のイベントまで無期限に取りこぼす」旧回帰が新経路で再発する）、
(3) `record_ime_apply_result()`（`applied` → `Confirmed{open:true}` 更新、
**ゲート B のルール(1) を再ラッチするのはここ**）、(4) `on_ime_applied()`
（composition warm/cold の簿記）。

- **architect の round 2 指摘（N-2）**: (1)(2)(4) を落とすと既知の回帰を
  個別に再導入するため、分割せず丸ごと呼ぶべき。ゲート B の再ラッチは
  ADR 自身が「新しく壊すものではない」と認めている既存仕様であり、
  `docs/known-bugs.md` への記録で足りる。
- **premortem の round 2 指摘（R2-1）**: (3) が原因である以上、(3) を
  呼ばないことこそが唯一の実効的な緩和であり、それをしないと「reassert
  が空振りするたびに `applied=Confirmed{true}` が上書きされ、ゲート B の
  周期自己修復の沈黙期間が**訂正を試みるたびに延長される**」という、
  現状（53 秒の無反応）より悪化しうる具体的な退行シナリオがある。

**決定: premortem の指摘を採用し、(3)（`record_ime_apply_result`）だけを
呼ばない形で切り出す。** (1)(2) はそのまま呼ぶ——architect が指摘した
個別の回帰（設定漏れの取りこぼし、journal 記録の欠落）を避けるため。
(3) を呼ばない理由: 本 ADR は「効果不明の best-effort 追加試行」である
ことを自ら明記しており（「本 ADR が実際に約束すること」節）、効果が
確認できていない書き込みに対して `applied=Confirmed{true}` という
**確定した観測であるかのような値**を記録するのは、BUG-69（TsfNative
force-on の belief 偽装）と同型の危険を持ち込む。

**round 3 architect レビューで判明した 3 点の訂正・追記:**

- **R3-1（実装可能性）**: 「(3) だけ飛ばす」は文字通りには書けない——
  (4) `on_ime_applied()` の呼び出し可否は (3) の戻り値
  `ImeApplyAcceptance`（`acceptance.drives_composition_side_effects()`、
  `Accepted` のみ true、`ime_model.rs:43-51`）で決まるため、(3) を丸ごと
  飛ばすと (4) の発火条件を決める材料が無くなる。(3) 自身のロジック
  （`platform_state.rs:848-869`）から同値な条件を直接導出する:
  **`outcome ∉ {UnsafeToToggle, NotOwned}` のときのみ (4) を呼ぶ**
  （これらの outcome は「実際には送っていない」ケースであり、composition
  簿記を走らせると BUG-31 族の「送っていないのに warm/cold が動く」実害を
  持ち込む）。
- **R3-2（適用範囲の明記）**: この「(3) を飛ばしてよい」という判断が
  安全なのは、reassert が同期経路（`generation: None`）だからである。
  `record_ime_apply_result` の `generation: Some` 側は `record_confirmed`
  に加えて `dispatch_event(ImeEvent::from_apply_outcome(..))` と
  `pending_generation` の解放も行う（`platform_state.rs:872-884`）——
  ここを飛ばすと event dispatch の欠落と pending 固着という別種の重大
  バグになる。**「(3) を飛ばしてよいのは `generation == None` の同期
  reassert 経路に限る」を不変条件とし、`tests/architecture_guard.rs` の
  テキスト走査（既存の「`force_on_attempt_allowed()` を経由しない」
  固定と同じ枠）で固定する。**
- **R3-3（機構的主張の訂正）**: 当初「(3) を飛ばすとゲート B の周期
  自己修復が沈黙し続けず、次のセカンドチャンスが生き残る」と書いていたが
  誤りだった。ゲート B のルール(1)（`state/ime_actuation.rs:251-258`）は
  `matches!(applied, Optimistic(true) | Confirmed{open:true,..})` という
  値だけの判定でタイムスタンプを見ておらず、クールダウン（ルール(3)）が
  読む `retry.last_attempt_ms` を更新するのは `note_force_on_attempt()`
  のみで reassert 経路はこれを呼ばない。したがって `record_confirmed`
  を実行しても実行しなくても、既に `Confirmed{true}` だった `applied`
  の値そのものは変わらず、ゲート B の沈黙が「訂正のたびに延長される」
  という機構は成立しない（今回のインシデントのように `applied` が既に
  `Confirmed{true}` の状態では、(3) を飛ばしてもゲート B は依然閉じた
  ままで「セカンドチャンスが生き残る」効果もこのケースでは発生しない）。
  **(3) を飛ばす決定を支える理由は上記の BUG-69 型 belief 偽装回避のみ**
  であり、これのみを根拠として残す。

**実機ソークの追加観点**: 「reassert が効いたか」だけでなく、「reassert
実装後に `force-ON (ImmBrokenForceOn)` ログの発火間隔が延びていないか」を
記録すること——R3-3 の訂正により理論上は延びないと分かったが、実装が
設計どおりかを実機で裏取りする意味は残る（「テスト計画」節に追記）。

**実装時の補足事項（round 3 premortem レビュー、非ブロッカー）**:

1. `record_ime_apply_result` を呼ばない結果、`pending`（`shadow_model`）は
   立てないままにすること（`generation=None` 経路なのでそもそも立たない
   はずだが、実装で誤って generation を割り当てると `try_force_on_
   bootstrap` の doc が警告する「pending が永久残留する」形になる）。
2. `view.belief_input_mode = input_mode()` は、`force_on_and_correct_
   romaji()` と同じ順序（`build_ime_control_view(None)` の直後）で書く
   こと。
3. `ActuationOrder::into_actuation()` の doc コメント「現時点で本番呼び
   出し元は無い」を、本 ADR がその最初の呼び出し元になった時点で更新する
   こと。

### D4: OFF 方向は本 ADR に含めない（別課題として起票する）

**当初案（ON/OFF 対称化）は round 1 レビューで撤回**。理由:

- `issue_open_warrant()` の Step 0 は `finalize(requested, true, ..)` と
  resolved を `true` に固定しており（`open_warrant.rs:147`）、override
  guard（`SafetyValve`）が有効な間は OFF 方向の warrant が構造的に発行
  されない。「ON/OFF 対称」という前提が授権層で既に崩れている。
- `force_on_and_correct_romaji()` は `OpenBelief{effective_open:true}`・
  `correction_for_imm_broken()` 呼び出しまで含め全て ON 固定
  （`runtime/mod.rs:877-936`）であり、「同じコードパスで自然に対応できる」
  という当初の前提も成立しない——OFF 対応には独立した新関数が要り、
  「追加コストはほぼ無い」という当初の見積りは誤りだった。

対称性の原則自体（ON 方向だけ厚くすると OFF 方向の同型欠陥が後日「別バグ」
として再発見される、`.claude/rules/ime-belief-architecture.md` 参照）は
支持するが、それは「本 ADR で両方直す」ではなく「OFF 方向の穴を明示的に
`docs/known-bugs.md` へ起票して残す」ことで満たす。本 ADR は **ON 方向
（`TurnOn` の物理キー）のみ**を対象とする。OFF 方向は D1〜D3 と同じ設計
（`issue_actuation_order` ベース）を素直に流用できるはずだが、
`force_on_and_correct_romaji()` に相当する OFF 専用の実装・テストが
無いため、別 ADR（または軽量な追補）で扱う。

### D5: 新しい `OpenApplyReason` variant を追加し、`ShadowToggle`/`ImmBrokenForceOn` と区別する

```rust
pub enum OpenApplyReason {
    EngineDecision,
    ImmBrokenForceOn,
    Bootstrap,
    DriftCorrection,
    ShadowToggle,
    ExplicitKeyReassert, // 新設
}
```

理由: 今回の実機診断は `force-ON (ImmBrokenForceOn): ...` のログと
`[shadow-toggle] no-op: ...` のログが別々に存在したことで初めて全体像が
組み立てられた。既存の reason と混ぜてしまうと「no-op のはずが実は
reassert を送信していた」ケースと「周期リフレッシュが送信した」ケースが
journal 上で区別できなくなり、次に同型の不具合報告が来たときの再現性の
ある一次情報を失う（`.claude/rules/fix-requires-evidence.md`/
`experiment-logging.md` が要求する「観測された失敗条件」の記録可能性を
維持する）。

### D6: 既存の ObservedEisu 救済・半角英数トグル復元とは排他にする

no-op 分岐には既に「TurnOn 系キーが押されたとき、IME が既に open でも
stale `ObservedEisu` を `AssumedRomaji` に戻す」救済ロジックがある
（`key_pipeline.rs:950-978`、2026-07-09 MS Edge/MS-IME 由来）。これは
belief（input_mode）側の訂正であり、D1 が対象にする「実 OS への open/close
書き込み」とは直交するため、両方を実行してよい。

一方、**半角英数トグル復元（`kp_restore_kana_from_half_width`）が同一打鍵で
発火した場合は、D1 の reassert を明示的にスキップする**（round 1 premortem
レビュー指摘）。この復元は MS-IME 分岐で scan=0x70 付き `VK_DBE_HIRAGANA`
ペアの注入と IMC write/verify を行い、GJI 分岐では非冪等なトグル送信を
行う（`key_pipeline.rs:1698-1775` 付近）——いずれも既に「意図した方向への
OS 書き込み」を実行済みであり、D1 が追加で `VK_IME_ON` を送ると、GJI 側は
特に「二重送信防止」の設計意図（`mem::replace` による `was_toggle_active`
ガード）と衝突しうる。

## 未解決のまま残る問題

- **MS-IME がなぜネイティブ物理キーに応答しなかったのか。** 本 ADR は
  この根本原因を解明しない。3 回のネイティブ `VK_DBE_HIRAGANA` 押下が
  いずれも MS-IME を開かなかった、という事実そのものが未説明であり、
  2026-08-18 の Ctrl+変換 反例（`engine.rs` 経由の無条件 `SetOpen` も
  失敗した）と合わせて考えると、**「awase がもう一度書き込みを試みる」
  という方向のアプローチ全般に効果の上限があるかもしれない**。本 ADR の
  reassert が実際に効くかどうかは、実機ソークで観測された IME 状態に
  基づいて確認する必要がある（「テスト計画」節）。効かないと判明した
  場合、次に調べるべきは「MS-IME 側が IME ON 系キーを無視する状態に
  陥る条件」（フォーカス往復・スレッド attach 状態・TSF マネージャの
  状態等）であり、それは本 ADR とは別の調査になる。

  **有力な候補仮説（2026-09-02、ユーザー指摘）**: このセッションで GJI 起動
  → 途中で MS-IME へ切り替え、という操作があった場合、切替そのものが
  今回の症状の引き金になった可能性がある。`awase` は `WM_IME_KIND_CHANGED`
  （CLSID ベース判定、最大 ~4 秒の検出遅延、`docs/known-bugs.md:979`）で
  ライブに IME 製品切替を検出する仕組みを持っており、実際に不具合報告の
  可視ログ内でも「`[stage-observe] GJI observe skipped (active IME is not
  GJI)`」と MS-IME を正しく認識している——**この時点での `active_ime_kind`
  自体は追従できていたことは確認できた**。ただし報告に含まれる app.log は
  直近 ~65 秒分のみで、実際に切替が起きた瞬間（3 時間の稼働時間のどこか）は
  ログに残っておらず、**切替の瞬間に別のサブシステムが古い IME 製品を前提
  にした状態を持ち越していないかは今回のデータからは確認できない**。
  `docs/known-bugs.md`（半角英数トグル機能、BUG-25 系）には**まさにこの
  形の既知の未対応の限界**が記録されている——「entry 時点の IME 種別を
  記憶せず、exit 時に `active_ime_kind` をその場で再取得するため、
  entry〜exit の間に言語バー等で IME 製品自体を切り替えると、実際に
  entry した側の IME には復元キーが送られないまま belief だけ先に進む」
  （同ファイル「未対応の既知の限界」節）。今回の症状がこれと同一機構とは
  確認できていないが、**「IME 製品切替時に、切替前提を持ち越したまま
  残るサブシステムがある」という失敗クラス自体はこのリポジトリで前例が
  ある**。ユーザーからは「awase 自身が製品切替を検知して、自主的に
  再起動する、あるいはユーザーに再起動を促すべきではないか」という設計
  提案があった——これは本 ADR の reassert（実 OS への追加送信）とは
  **別レイヤーの対策**（検出 → 既知安全状態への復帰、というアプローチ）
  であり、有効な場合は reassert より根本的な解決になりうる。本 ADR の
  スコープには含めないが、次の調査候補として明記する: (a) 今回のユーザー
  が実際にセッション中 GJI→MS-IME の切替を行ったか確認する、(b) 行って
  いた場合、`WM_IME_KIND_CHANGED` ハンドラ（`app/mod.rs:394-395`、
  `message_handlers.rs::handle_wm_ime_kind_changed`）が更新する状態と、
  更新しない（＝古い IME 製品の前提を持ち越しうる）状態を洗い出す、
  (c) 「切替を検出したらユーザーに通知する／再起動を促す」機能の要否を
  判断する。
- **ゲート B（`force_on_attempt_allowed()` の周期的自己修復クールダウン）。**
  今回の実機ログは、フォーカス往復後に `applied` がキャッシュ復元で
  `Confirmed{true}` に固定され、以後 53 秒間まったく再試行されなかった
  ことも示している。D3 で触れたとおり、本 ADR の reassert 自体がこの
  固定化を助長しうる。別 ADR（または `docs/known-bugs.md` BUG-37 追補）
  で扱う。

## 範囲外（本 ADR では扱わない）

- **ゲート B の再設計**（上記「未解決のまま残る問題」参照）。
- **`is_eligible_for_ime_force_on()` の `issue_open_warrant()` への全面置換**
  （ADR-087 §5 Phase 3 item15）。既存 2 箇所（`apply_force_on_for_imm_broken`/
  `try_force_on_bootstrap`）のゲートには一切手を触れない。
- **Standard/ImmCross プロファイルへの拡張。** これらは `ImmGetOpenStatus`
  等で実状態を読めるため、既存の observation-based drift correction が
  既に対応している。D1 条件2（`!can_use_imm32_cross_process()`）で明示的に
  除外する。
- **OFF 方向**（D4 参照、別課題として起票）。
- **SyncKey（config の `keys.ime_on`/`keys.ime_off` 等）を本 ADR の対象に
  含めるか。** `write_sync_key()` も `write_physical_key()` と同様に
  `IntentStore` へ記録するため、技術的には D1〜D3 と同じ仕組みで
  `IntentKind::SyncKey` にも適用できる。「手動 Ctrl+変換 = strategy chain
  経由の apply は毎回効いていた」という既存の実機観察（`runtime/mod.rs:853`
  のコメント）を踏まえると含める動機はあるが、本 ADR の実機証拠
  （report `01M1GVNR840NZ3XWRX0JPDSQR7`）は `PhysicalImeKey` のみのため、
  **本 ADR は `IntentKind::PhysicalImeKey` に限定する**。SyncKey への拡大は
  別途検討する（round 2 レビューで、含めるべきという強い理由が見つかれば
  この決定を覆す）。
- **ADR-028（フォーカスイベント処理の再設計、承認済み・未実装）。**

## 明示的に決めなかった／保証を弱めた点（round 1 レビューで発覚）

- **`current_focus()` が `None`（フォーカス追跡が確立していない）のとき、
  reassert は静かに発火しない。** `record_explicit_intent()` は
  `current_focus()` が `Some` のときしか記録しないため（`platform_state.rs:1220`）、
  この場合 `IntentStore` にエントリが無く、Step 1 が外れ warrant が
  `None` になる。「ユーザーの明示訂正は必ず実 OS に届く」という当初の
  言い切りはこの理由で撤回した——正しい表現は「フォーカス追跡が確立
  している通常のケースでは、追加の試行を行う」。
- **`InputRelay` プロファイル**（Mouse Without Borders 等の中継ウィンドウ、
  issue #136/BUG-90）は `!can_use_imm32_cross_process()` を満たすため
  D1 条件2 を通過するが、`ImeController::apply` は冒頭で `NotOwned` を
  返す（`ime_controller.rs:514-517`）ため実害は無い。物理キー自体も
  `transport.rs:186` の InputRelay 早期 `Allow` により素通しされる。
  実装時、この経路も `would_have_blocked` 相当の早期リターンで
  無害に終わることをテストで確認すること。
- **`UnsafeToToggle`（Win キー押下中）時の再試行は無い。** `MsImeDirectStrategy`/
  `GjiDirectStrategy` はいずれも Win キー押下中に `UnsafeToToggle` を返し
  送信しない（`ime_controller.rs:132-137`, `:205-212`）。
  `apply_force_on_for_imm_broken` はこの場合クールダウンの起点にせず
  20ms リフレッシュに再試行を委ねるが、本 ADR の reassert 経路には
  そのような担い手がいない——この場合ユーザーがもう一度キーを押すまで
  何も起きない。人間が「効かなかったら押し直す」という自然な行動を
  取ることを期待し、専用のリトライ機構は本 ADR のスコープに含めない。

## 代替案として検討したが採用しなかったもの

### A1: 2026-08-18 案（`explicit_key_reassert.rs` 新設、`WriteMechanism::is_idempotent()`）をそのまま復活させる

当時案は ADR-087（`issue_open_warrant`）/ADR-090（`ActuationOrder`）が存在
する前に設計されたため、授権判定・冪等性判定のいずれも独自に新設する
前提だった。現在は両方とも既存資産（`issue_actuation_order()`、
`ShadowImeAction != Toggle` の型的区別）で表現できるため、新しいモジュール・
新しい型を追加する必要がない。

### A2: 周期リフレッシュの間隔を短縮する、または `force_on_attempt_allowed()` のクールダウンを短くする

`.claude/rules/tuning-constants.md` が警告する「同じ役割の定数の盲目的な
釣り上げ」の典型パターン。周期を短くしても「なぜ今回 53 秒も再試行が
起きなかったか」という根本（`applied` がキャッシュ復元で固定されたこと）
は直らず、別のクールダウン値を探る対症療法になる。ゲート B は「範囲外」
として別途扱う。

### A3: `kp_stage_shadow_ime_toggle` の no-op ガード自体を撤去し、常に apply を試みる

最も単純だが、`effective_open()` が実際に変化しない大多数のケースで
毎回 OS へ書き込みが飛ぶことになり、TSF composition への余計な副作用
（`mark_composition_cold` の連発等）を増やす。D1 の条件（`Toggle` 除外・
Blacklist 限定・settle-defer・デバウンス）は、この単純な撤去がもたらす
副作用を避けつつ同じ目的を達成するための絞り込みである。

### A4: `is_japanese_ime()` を注入イベントにも適用して upgrade を広げる

無関係な軸（ADR-093 で既に検討・却下済み、BUG-14 の再発リスク）。

## リスクと軽減

| リスク | 軽減策 |
|---|---|
| デバウンス実装ミスで連続送信し、TSF composition を壊す | まず auto-repeat の有無を実機確認し（R2-3）、必要な場合のみ専用クールダウン定数を実機測定で決定、境界値の単体テストを固定する |
| スコープ判定漏れで `VK_KANJI` 等の非冪等キーを再送してしまう | D1 の発火条件を `vk_code == VK_DBE_HIRAGANA && action == TurnOn` という単一の具体的な等値比較にし、`action != Toggle` のような広い型条件に頼らない。golden テストで他の VK が reassert を発火しないことを固定する |
| 0xF2 への冗長送信が BUG-46 型の副作用競合を再現する | D1「`PhysicalKeyDisposition::plan` との相互作用」節・D3 の `belief_input_mode` 明示的設定（N-1 対応）により、ROMAN 補完込みで安全側に倒れることを確認済み。golden テストで `ObservedKana` 時の非上書きを固定する |
| reassert が毎回 `mark_composition_cold`+eager warmup を誘発し、健全なセッションにレイテンシを持ち込む | 既存の周期的 force-ON 機構が既に受け入れているコストと同種であり新規のリスクではないが、D2 のクールダウン（必要な場合）で頻度を抑える |
| `record_ime_apply_result()` を呼ばないことで診断能力が落ちる、あるいは他の消費者が `applied` の更新を期待している | D5 の journal 記録（呼ばないのは (3) のみ、(1)(2)(4) は呼ぶ）で診断能力は維持される。`applied` を更新しないことの影響範囲は実装時に呼び出し元を洗い出して確認する |
| 新しい `OpenApplyReason::ExplicitKeyReassert` の追加が `tests/architecture_guard.rs` 等の出現数固定テストを壊す | 実装時に該当テストを確認し、期待値を更新する |
| 本 ADR の reassert が実際には症状を直さない（MS-IME 側の根本原因が別、GJI→MS-IME 製品切替等） | 「未解決のまま残る問題」節で明示。実機ソークで観測状態を確認するまで「解決」とは扱わない |

## テスト計画

`.claude/rules/fix-requires-evidence.md` の「キー選択（IME ON/OFF に送る VK）」
「force-write / actuation ターゲット」再発ファミリーに該当するため、回帰
テストを必須とする。

- **実機確認（コーディング前、round 2 premortem レビュー R2-3）**: 「かな
  キーを押しっぱなしにしたとき `VK_DBE_HIRAGANA` の `KeyDown` が OS の
  auto-repeat で繰り返し届くか」を先に確認する。届かないなら D2 は
  「デバウンスなし」で確定する。
- **純粋関数テスト（Linux で実行可能）**:
  - D1 の発火条件（`vk_code == VK_DBE_HIRAGANA && action == TurnOn` かつ
    D1 条件2〜5）の単体テスト。
  - D2 のデバウンス判定（auto-repeat 確認の結果次第で「常に true」または
    クールダウン判定のいずれか）の単体テスト。
  - `issue_actuation_order()`/`would_have_blocked()` 呼び出し自体は既存の
    網羅テストを再利用し、新規テストは呼び出し元が正しい `open`/`origin`
    を渡しているかに絞る。
  - `ObservedKana` のとき reassert が ROMAN ビットを書かないことの単体
    テスト（D3、N-1 対応）。
- **`crates/awase-windows/tests/ime_key_sequence_golden.rs`**: 「TsfNative
  プロファイル・MS-IME で、belief=ON のまま物理 `VK_DBE_HIRAGANA` が再度
  押されたとき、`VK_IME_ON` を再送する」「`belief_input_mode ==
  ObservedKana` のときは ROMAN 補完を伴わない」の 2 ケースを golden に
  追加する。
- **`crates/awase-windows/tests/architecture_guard.rs`**: 新設する
  `OpenApplyReason::ExplicitKeyReassert` の構築箇所数、`reassert_explicit_
  physical_key` が `force_on_attempt_allowed()` を経由しないこと、
  `record_ime_apply_result()`（D3 の (3)）を呼ばないこと、および
  **この省略が `generation == None` の同期経路に限られること**
  （round 3 architect レビュー R3-2）をテキスト走査で固定する。
- **`docs/known-bugs.md`**: BUG-37 に本 ADR 実装を反映した追補を追記する
  （症状・実機ログの要約・修正内容・「未解決のまま残る問題」節の内容
  ——GJI→MS-IME 製品切替の候補仮説を含む——・関連コミット）。
- **実機ソーク（必須、送信ログではなく観測状態で確認）**: Windows Terminal
  （MS-IME）で意図的にフォーカス往復を発生させ、実 IME が OFF に落ちた
  状態を作った上で、物理「かな」キー押下で `VK_IME_ON` の**送信ログが
  出ること**と、**その後実際に IME 状態が ON に変わりローマ字が正しく
  合成されること**の両方を確認する（BUG-15 追補3 の前例: 送信ログだけ
  では不十分）。**加えて、reassert 実装前後で `force-ON (ImmBrokenForceOn)`
  ログの発火間隔が延びていないかを記録する**（D3、premortem R2-1 指摘:
  reassert が空振りしてもゲート B の周期自己修復が沈黙しないことを確認
  する直接の手段）。可能であれば「未解決のまま残る問題」節の GJI→MS-IME
  製品切替の有無をユーザーに確認する。**この検証で効果が確認できなければ、
  D1〜D6 は実装しても「未解決のまま残る問題」節がクローズしないことを
  known-bugs.md に明記する。**

## Round 3 レビューでの主な訂正（収束）

- D3 の `on_ime_apply_complete` 分割を「(1)(2) は呼ぶ、(4) は
  `outcome ∉ {UnsafeToToggle, NotOwned}` のときだけ呼ぶ、(3) は呼ばない」
  という実装可能な形に具体化（architect R3-1）。
- 「(3) を飛ばしてよいのは `generation == None` の同期経路に限る」という
  適用範囲の不変条件を明記し、architecture_guard での固定を追加
  （architect R3-2）。
- 「ゲート B の沈黙が訂正のたびに延長される／セカンドチャンスが残る」と
  いう当初の機構的主張を誤りとして削除し、(3) を飛ばす根拠を BUG-69 型
  belief 偽装の回避のみに一本化（architect R3-3）。
- premortem の round 3 実装時補足事項（`pending` 非発生の確認、
  `belief_input_mode` の記述順序、`into_actuation()` doc 更新）を追記。
- 両レビュアーとも承認、round 3 で収束。

## Round 2 レビューでの主な訂正

- 「本 ADR が実際に約束すること」: `VK_IME_ON` が「conv-mode に一切触れ
  ない」という主張を訂正し、`romaji_pre_write()` の ROMAN 補完が入る
  ことを明記。初期スコープを 0xF2/`TurnOn` 単独に縮小（0xF1/0xF4 は解釈
  未確定のため見送り）（architect N-1、premortem R2-2）。
- D1: 発火条件を「`action != Toggle`」から「`vk_code == VK_DBE_HIRAGANA
  && action == TurnOn`」へ縮小。settle 中も送るか、pending フラグで
  settle 明けに確実に再試行するかを明示（premortem R2-4）。BUG-46 との
  差異判定を 0xF2 単独のケースに絞って再構成。
- D2: 「まず auto-repeat の有無を実機確認する」を最初のステップに追加
  （premortem R2-3）。フォーカス変更でのクールダウンリセットを追加
  （architect N-3）。VK 単位のクールダウン競合問題はスコープ縮小により
  解消したことを明記。KeyUp 不在の典拠を実データ（自己検証）で補強し
  つつ断定を弱めた（architect N-4）。
- D3: `view.belief_input_mode = input_mode()` を必須手順として追加
  （architect N-1）。`on_ime_apply_complete()` の扱いを、architect と
  premortem の異なる推奨を検討した上で「`record_ime_apply_result()` の
  みを呼ばない」に決定し、理由を明記（architect N-2、premortem R2-1）。
  実機ソークに「ゲート B の沈黙期間が延びていないか」の観点を追加。
- 「未解決のまま残る問題」: ユーザー指摘の GJI→MS-IME 製品切替仮説を
  候補として追加し、次の調査手順を明記。

## Round 1 レビューでの主な訂正

- 背景: 「物理キーが OS に届かなかった」という当初の因果説明を撤回し、
  「3 回ともネイティブに OS へ届いていたが MS-IME が応答しなかった」に
  訂正（premortem_reviewer F2、自己検証済み）。
- D1: `PhysicalKeyDisposition::plan`/BUG-46 との相互作用の分析を追加
  （architect A-1）。settle-defer・半角英数トグル復元との排他を追加
  （architect (b)、premortem F3/F5）。
- D2: `physical_key_held_ms` ベースの当初案を撤回し、専用クールダウン
  定数（実測要）へ変更（architect A-7、premortem F1 — 0xF2 に KeyUp が
  無いため当初案は既存の 3 回連打の 2/3 回目を誤って抑制していた）。
- D3: `issue_open_warrant()` 直接呼び出し（実装不能、INV-47/48 違反）を
  撤回し、`issue_actuation_order()` + `would_have_blocked()` へ変更
  （architect A-2/A-3）。
- D4: ON/OFF 対称化を撤回し、ON 方向のみに縮小（architect A-4/A-5）。
- KanjiToggle 到達不能性の根拠を、循環論法から独立した 3 点の導出へ
  差し替え（architect A-6）。
- 「明示的に決めなかった／保証を弱めた点」節を新設し、`current_focus()==None`・
  `InputRelay`・`UnsafeToToggle` の扱いを明記（architect A-3、premortem F7）。
- 「未解決のまま残る問題」節を新設（premortem 総合判定）。

## 関連

- `docs/known-bugs.md` BUG-37（本 ADR の直接の対象）、BUG-46（D1 の
  安全性判断の比較対象）、BUG-50（`VK_DBE_HIRAGANA` モードキー副作用の
  前例）、BUG-15 追補3（送信ログと実効果の乖離の前例）、BUG-25（半角英数
  トグル機能の「entry/exit 間の IME 製品切替」既知の限界——「未解決のまま
  残る問題」節の GJI→MS-IME 切替仮説と同型の失敗クラスの前例）
- `docs/bug-reports-triage.md` report `01M1GVNR840NZ3XWRX0JPDSQR7`
- [ADR-087](087-open-belief-actuation-warrant-separation.md)
- [ADR-090](090-typestate-effectuation-and-adjacent-adr-closure.md)（A-1 shadow → A-2 実ゲート化の段階設計、本 ADR は A-2 の最初の限定的インスタンス）
- [ADR-098](098-tsfnative-applied-confirmed-laundering-and-force-on-removal.md)
- [ADR-093](093-dbe-hotkey-observation-upgrades-japanese-ime-belief.md)
- [ADR-028](028-focus-event-redesign.md)（承認済み・未実装、本 ADR とは独立）
- `.claude/rules/ime-belief-architecture.md`
