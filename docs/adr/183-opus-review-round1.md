---
id: ADR-183-review-round1
title: |-
  ADR-183（VK_KANA を ModeKeyActuationOwner::PhysicalDelivery に合流させる案）
  への opus-adversarial-consult round1 レビュー
status: |-
  **Blocker 4件 / Must-fix 7件。現状の設計案のままでの実装着手は不可。**
  中核仮説（KeyUp 無条件 Suppress が症状の直接原因）がリポジトリ自身の
  既存記述と正面から矛盾するため、まず実機一次証拠の取得から戻ること。
related_adr:
  - "ADR-183"
  - "ADR-179"
  - "ADR-137"
  - "ADR-100"
  - "ADR-119"
---

# ADR-183 opus-adversarial-consult round1

## 総括

ADR-183 の**コード経路の追跡は正しい**。`VK_KANA` (0x15) が
`transport.rs::plan()` の `is_kanji_event` 分岐まで実際に到達すること
（InputRelay / `VK_DBE_HIRAGANA` / `injected` / `VK_CONVERT|VK_NONCONVERT`
のどの先行分岐にも捕まらないこと）、および `ime_actuation_owned` 下で
KeyDown が `shadow_toggled` 依存・KeyUp が無条件 Suppress になるという
非対称性の記述は、実コードと一致している。引用行番号もほぼすべて正確
（末尾「引用行番号の検証結果」参照）。

しかし**「その非対称性が報告された症状の原因である」という診断は、
このリポジトリ自身が既に持っている記述と矛盾する**。加えて、提案されている
「VK_KANA を無条件 Allow にする」という変更は、ADR-183 が安全性の根拠に
挙げている「単一方向・冪等」という前提が VK_KANA については**成立しない**
ため、BUG-08 / BUG-61 が「復旧不能」と確定させた破損を awase 自身が
作り出す経路を新設する。

---

## Blocker

### B1: 「KeyUp が Suppress されるからモード切替が完了しない」は、リポジトリ自身の記述と矛盾する

**根拠**: `src/config.rs:1149-1160`（`validate_thumb_keys`）

```rust
fn validate_thumb_keys(g: &GeneralConfig, w: &mut Vec<String>) {
    if g.left_thumb_key == "Kana" || g.left_thumb_key == "VK_KANA" ... {
        w.push(
            "Kana キーはロック型キーで KeyUp イベントが発生しません。\
             親指キーとしての使用は推奨しません."
        );
    }
}
```

この警告文は `src/config.rs:2410-2460` に4本のユニットテスト
（`test_validate_thumb_keys_warns_on_left_vk_kana` 等）で固定されている、
**このリポジトリの一次記述**である。

ADR-183 の中核仮説は「KeyDown は届くが KeyUp だけ消えるので、IME モードキーの
押下が『held down』のまま完結せずモード切替が発火しない」（ADR 本文 90-96行目）
だが、上記が正しければ:

- そもそも物理 VK_KANA の KeyUp は**発生しない**（ロック型キー）。
  `transport.rs::plan()` の KeyUp 無条件 Suppress 分岐は、VK_KANA については
  **到達自体が稀または皆無**であり、Suppress されていないものを Suppress の
  せいにしている。
- IME 側も、来ないことが前提の KeyUp に依存してモード遷移を確定させる実装には
  していないはず（そのような実装だと物理かなキー単体が永久に効かないことになる）。

**このどちらかが誤りである**。すなわち (a) `config.rs` の警告が古い/誤りで
VK_KANA にも KeyUp が来る、(b) ADR-183 の仮説が誤り、のどちらか。
**どちらであるかは実機ログ1行（`[hook] VK_KANA up 到達`、`hook.rs:1281-1287` の
INFO）で確定できる**。このログは既に常時出力されているため、取得コストは
「かなキーを1回押してログを grep する」だけである。

これを確認せずに `transport.rs` の条件式を変える設計を確定させるのは、
**存在しないイベントの Suppress を解除する変更**になりかねない。

### B2: 「Allow＝物理キーがそのまま OS に届く」という前提が誤り（`executor.rs` 自身が訂正済み）

ADR-183 は VK_KANA を「素のままパススルー」する設計意図を述べ、ADR-179 の
`PhysicalDelivery`（awase の送信がゼロになる）に合流させようとしている。
しかし `runtime/executor.rs:417-435` のコメントは、まさにこの mental model が
誤りであることを**過去の失敗として明記している**:

> **訂正（2026-09-05、BUG-116 調査で発覚）**: 以前このコメントは「通常 hook 経路では
> PassThrough は `CallNextHookEx` で OS に直接届く」と書いていたが、これは誤り。
> （中略）**通常 hook 経路でも PassThrough は必ず `enqueue_reinject` 経由で SendInput に
> より再送出される**。`CallNextHookEx` で OS に直接届く経路は存在しない。
> この誤った mental model が、`docs/adr/137-...md`（BUG-116）で
> 「`PhysicalKeyDisposition::plan` が Allow を返せば OS に届く」という前提の一因になっていた
> （実際には reinject が `wScan: 0` を使うため、scan 依存のモードキー処理に影響しうる）。

実際の再送出は `crates/awase-windows/src/lib.rs:407-432`:

```rust
ki: KEYBDINPUT {
    wVk: VIRTUAL_KEY(self.vk_code.0),
    wScan: 0,
    ...
    dwExtraInfo: INJECTED_MARKER,
}
```

したがって VK_KANA における "PhysicalDelivery" は、**物理キーの配送ではなく
awase 自身による合成 VK_KANA の SendInput** である。帰結:

1. **ADR-179 の `PhysicalDelivery` 定義（「awase からの送信はゼロになる」、
   ADR-179 328-330行目）が VK_KANA については成立しない。** 列挙値の意味が
   VK ごとに変わることになり、ADR-179 が最大の成果として挙げた
   「exhaustive enum で配線漏れをコンパイラが検出する」という性質が崩れる。
2. `wScan: 0` の合成キーに対して GJI/MS-IME が物理キーと同じモード切替を
   行う保証はない。ADR-137 は `VK_DBE_KATAKANA` について scan なし注入で
   GJI がカタカナへ切り替わることを**実機確認した上で**採用しており
   （`transport.rs:90-94`）、VK_KANA について同等の確認は存在しない。
3. **awase 自身の再注入は `INJECTED_MARKER` 付きのため、`hook.rs` 先頭の
   `if self_injected { return CallNextHookEx(...) }`（`hook.rs:1165-1168`）で
   早期 return し、`hook.rs:1254` の BUG-08 swallow ブロックに到達しない。**
   すなわち B3 の保護を構造的に迂回する。

### B3: 「単一方向・冪等だから安全」は VK_KANA については事実として誤り（BUG-08 / BUG-61）

ADR-183 127-140行目は「VK_KANA は `TurnOn` 専用・idempotent なので二重に
届いても実害が無いはず」と論証している。`vk.rs:140-151` の `shadow_effect()`
が `Kana => TurnOn` を返すのは事実だが、**これは awase の belief モデル上の
分類であって、OS 側での VK_KANA の効果ではない**。

OS 側では VK_KANA は**ロック型（トグル）キー**である:

- `crates/awase-windows/src/observer/kana_lock.rs:14` —
  `crate::ime::is_toggle_key_on(VK_KANA)`、すなわち `GetKeyState(vk) & 1`。
  CapsLock と同じトグルビットを持つ。
- `hook.rs:1225-1231` —
  > VK_KANA down/up は OS のかなロックをトグルし、GJI/MS-IME がローマ字入力→JISかな
  > 入力に反転して NICOLA の romaji VK 出力が壊滅する（2026-07-06 実機: down→up
  > 135µs〜1ms の合成 VK_KANA ペアが 2 回到達し Windows Terminal が JISかな化）
- BUG-61（`hook.rs:1232-1238`）— 一度 JIS かな側へ切り替わると
  `ImmSetConversionStatus`・`VK_DBE_ROMAN` 注入の**どちらでも復旧不能**と実機で確定。

**具体的な失敗シナリオ**（Windows Terminal / WezTerm × MS-IME または GJI）:

1. ユーザーが IME ON・Eisu 状態でかなキーを押す。
2. ADR-183 の変更により `plan()` が Down/Up 共に `Allow` を返す。
3. `enqueue_reinject` が `wVk=VK_KANA, wScan=0, INJECTED_MARKER` の
   **down + up ペア**を SendInput する。
4. これは `hook.rs` の `self_injected` 早期 return で BUG-08 swallow を素通りし、
   OS のかなロックビットをトグルする。
5. GJI/MS-IME がローマ字入力→JIS かな直接入力に反転。以後 NICOLA が出力する
   romaji VK 列（例: `ko` → `[4B,4F]`）が JIS かな解釈され、入力が壊滅する。
6. **BUG-61 により awase からは復旧不能。** ユーザーは Alt+かな を自分で
   押し直すしかない（しかもその Alt+かな は `hook.rs:1266-1278` で swallow される）。

現状（KeyUp のみ Suppress）は down 単独の再注入になっており、ペアを作らない。
ADR-183 はこのペアを作る変更である。「報告された症状（ひらがなに戻らない）」より
**桁違いに重い、復旧不能な破損**をトレードに出している。

なお、down 単独でもロックビットが動く可能性はある（ロックキーのトグルは
通常 KeyDown で起きる）。もしそうなら「今日すでに起きている」ことになり、
**これ自体が報告症状の第一容疑になる**（M1 参照）。いずれにせよ
**かなロックの実挙動を実機で確認せずにこの分岐を触ってはならない**。

### B4: OFF→ON 経路の実 actuate 主体を特定した結果、無条件 Allow は BUG-46 型の二重 actuation を新設する

ADR-183 の「未解決の疑問2」への回答（コードを辿って特定した）:

- `kp_stage_shadow_ime_toggle` の **ON→OFF 方向にだけ**明示 actuate ブロックが
  ある（`key_pipeline.rs:1755` の `if !self.platform_state.ime.effective_open()
  && owner_permits_explicit_off_actuate`）。
- **OFF→ON 方向には明示 actuate が無い。** belief を書くだけで、実 IME を ON に
  しているのは `Engine::transition_activation` が発行する
  `Effect::Ime(SetOpen { origin: ActivationSync })` →
  `kp_stage_execute` → `executor.rs::dispatch_ime_set_open` →
  `GjiDirectStrategy`/`MsImeDirectStrategy` の `SendInput(VK_IME_ON)` である
  （ADR-154 の確定事項。ADR-179 の「レビュー経緯 round4」が、この事実を
  4ラウンド誤り続けた末に確定させている）。

したがって:

- **案A（`transport.rs` だけ無条件 Allow にする、ADR-183 の「疑問5」で
  『十分かもしれない』とされている方）**: VK_KANA が belief を OFF→ON に
  動かすと、awase は `ActivationSync` 経由で `VK_IME_ON` を SendInput し、
  **同時に**再注入された VK_KANA も GJI/MS-IME に届く。これは BUG-46 が
  修正した「awase 自身の SendInput と素通しされた物理 KANJI 系キーが二重に
  actuate する」の**そのままの再現**であり、`transport.rs:229-236` の
  `ime_actuation_owned` 導出コメントが名指しで警告しているケースである。
  加えて VK_KANA は `vk_may_mutate_conv == true`（`vk.rs:195-214`）なので、
  conv ワードも二重に動く。
- **案B（`ModeKeyActuationOwner::PhysicalDelivery` に合流させる）**: 消費点2
  （`key_pipeline.rs:457-462`）が `ActivationSync` 由来の `SetOpen` を strip
  するため二重 actuation は消えるが、今度は M3 の問題（救済経路の全喪失）が
  主要 IME ON キーに適用される。

**どちらの案も、現状のまま実装すると新規バグを作る。** ADR-183 が「疑問5」で
「`transport.rs` だけで十分か」を open にしているのは、この二者択一が
トレードオフの本体であることを見落としているためである。

---

## Must-fix

### M1: conv 軸という別の原因候補を一切検討していない（症状に対する説明力はこちらが高い）

報告症状は「IME ON・**conv mode が半角英数（Eisu）**・Engine ON でかなキーを
押してもひらがなに戻らない」である。これは **open 軸ではなく conv 軸（charset 軸）の
問題**として記述されている。

`kp_stage_shadow_ime_toggle` の no-op 分岐（`key_pipeline.rs:1612-1690`）は、
まさにこの状況を既に扱っている:

```rust
if self.platform_state.ime.effective_open() == current {
    ...
    // TurnOn 系キー（ひらがな/かな 等）は IME が既に open でも「英数から
    // ひらがなへ戻す」ユーザー操作として意味を持つ。
    if let Some(new_mode) = eisu_recovery::eisu_reset_on_turn_on_while_open(...) {
        ... self.apply_input_mode_correction(new_mode, UserTurnOnEisuReset, tick_ms);
    }
    return false;
}
```

**この救済は awase の belief（`input_mode`）を `AssumedRomaji` に書き戻すだけで、
実 IME の conv モードには一切触れない。** つまり awase 側は「ひらがなに戻った
ことにする」が、実 IME は Eisu のまま——ユーザーから見た症状と完全に一致する。

さらにこれは既知問題として記録済みである。`transport.rs:283-291`:

> ADR-100 決定2（2026-08-22）で eager warmup の送信キーは `VK_DBE_HIRAGANA` から
> `VK_IME_ON` 単発（open 軸のみ）へ変更済み。つまり物理 F2 の代替として実際に
> 送られるのは open 軸のみで、**charset 軸（カタカナ→ひらがな）を戻す効果は無い**。
> この「埋め合わせの片肺化」が、**GJI 環境で物理かなキー単独ではひらがなに戻せない
> 副問題（ADR-137 M-6）の真因**であり、`kp_restore_hiragana_for_suppressed_mode_key`
> （BUG-116 決定2）がこの埋め合わせを別経路で補っている。

**ADR-137 M-6 / BUG-116 は、ADR-183 が調査している症状と同じ症状を、
同じ「かなキー」について、既に真因まで特定している。** ADR-183 は
`VK_DBE_HIRAGANA` を「論点が混ざる」としてスコープ外に置いた（147-151行目）が、
**スコープ外にしたその領域にこそ、症状の既知の説明が存在する**。

対応: ADR-183 は「この症状が ADR-137 M-6 と同一か別か」を先に切り分けること。
同一なら ADR-183 は不要（BUG-116 決定2 の `kp_restore_hiragana_for_suppressed_mode_key`
が VK_KANA 経路をカバーしていないだけ、という遥かに小さい修正になる）。

### M2: そもそも押されたキーが VK_KANA である保証が無く、0xF0/0xF1 の可能性が高い

`transport.rs:875-888` の実機記録（BUG-52、2026-08-05）:

> NICOLA の物理「IME ON」キー（scan 0x70）は **IME が既に目的の状態にある時に押されると**、
> `VK_DBE_HIRAGANA` (0xF2) ではなく `VK_DBE_ALPHANUMERIC` (0xF0) や
> `VK_DBE_KATAKANA` (0xF1) が生成されることがある（実機ログで両方確認）。

報告症状の前提条件は「**IME が既に ON**」であり、この実機記録が適用される条件
そのものである。もし実際に生成されているのが 0xF0/0xF1 なら:

- `plan()` は `is_dbe_mode_key_down`（`transport.rs:405-414`）により
  **KeyDown を無条件 Suppress** する。ADR-183 が提案する VK_KANA 専用分岐は
  この経路に一切影響しない＝**修正しても症状は変わらない**。
- そして 0xF0 は `TurnOff` 分類なので、belief 側でも「ひらがなへ戻す」意図として
  解釈されない。

ADR 本文40行目の「このユーザーの環境では VK_KANA (0x15) が生成される」に
根拠が示されていない。`hook.rs:1128-1162` の
`[hook] IME-mode vk=0x.. {dir} self_injected=.. injected=.. scan=0x..` は
0x15/0xF0/0xF1/0xF2 のすべてで debug 出力されるため、**ログ1行で確定できる**。

### M3: `PhysicalDelivery` 合流は「再試行も自己修復も無い」性質を主要 IME ON キーへ拡大する

ADR-179「残るトレードオフ」（398-423行目）が正直に記録している性質:

> `PhysicalDelivery` には再試行も自己修復も無い。（中略）beliefは目標値に書かれるが
> 実IMEは変わらないままになる。しかも次に同じキーを押しても `action.resolve(current)` が
> belief（既に目標値）と一致するため no-op 早期 return に落ち、**回復しない**。
> 観測可能な環境ではdrift correctionが救うが、**Blind 環境（TsfNative/UWP）では
> 救済経路が無く恒久固着する**。

ADR-179 はこの危険を受け入れる範囲を「**無変換/変換が非親指キー配置のとき**」
という極めて狭い条件に絞り、かつ **{GJI, MS-IME} × {ImmCross, TsfNative} の
4象限の実機A/Bを実装の受け入れ条件**として課した（ADR-179 475-479行目）。

ADR-183 は同じ性質を、**ユーザーが日本語入力を開始する主経路であるかなキー**に
拡大しようとしている。しかも B2 により、VK_KANA では「物理配送」の実体が
awase 自身の scan なし合成キーであるため、**「GJI/MS-IME 自身が必ず反応する」
という PhysicalDelivery の大前提そのものが未検証**である。

最低でも ADR-179 と同等（4象限の実機A/B）を受け入れ条件に課すこと。
それを課すなら、その実機確認を**設計確定の前**に行えば B1/M1/M2 も同時に
解決するため、順序を入れ替えるべきである（Q7 参照）。

### M4: `PassthroughQueue` の「inert である」という不変条件を壊す

`transport.rs:118-129`（`PassthroughQueue` の doc）:

> `deferred_vks` は（中略）0xF3/0xF4 のような「ペア表現」の KANJI 系キー
> （対応する KeyUp が原理的に来ない場合がある）ではエントリが残留し得るが、
> **BUG-46 の修正で KANJI 系 KeyUp は常に Suppress されるようになり
> `check_output_guard_defer` に到達しなくなったため inert。**
> 「leak しているように見える」からと TTL/クリア機構を追加する前に、まずこの残留が
> 実際に `check_keyup_symmetry` の誤発火につながる経路があるか確認すること。

ADR-183 が VK_KANA の KeyUp を Allow にすると、この「inert」という前提が
VK_KANA について崩れる:

1. VK_KANA KeyDown が Allow → `check_output_guard_defer` が output in-flight 中に
   defer → `deferred_vks.insert(VK_KANA)`（`transport.rs:186`）。
2. B1 が正しければ（ロック型キーで KeyUp が来ない）、このエントリは**永久に残留**。
3. 次に何らかの理由で VK_KANA の非 KeyDown イベントが来たとき、
   `check_keyup_symmetry`（`transport.rs:144-154`）が誤って発火し、
   `ReinjectKey` を積んで `Consumed` を返す。

VK_KANA は「対応する KeyUp が原理的に来ない場合がある」という、この doc が
名指しで懸念している 0xF3/0xF4 と**同じ性質のキー**である。ADR-183 はこの
doc の警告に答えていない。

### M5: ImmCross での扱いを未確定のまま残すのは「未確定」ではなく「危険側に倒れている」

ADR-183 175-180行目は ImmCross を open にしているが、これは設計の欠落である。

**具体的な失敗シナリオ（ImmCross プロファイル × MS-IME、例: 一般 Win32 アプリ）**:

ImmCross では awase が `ImmSetOpenStatus`/`ImmSetConversionStatus` を子 hwnd に
対してクロスプロセスで直接書く。`feedback_immcross_owns_kanji` の設計原則
（`transport.rs:308-317` にも記載）は「ImmCross アプリには物理 IME キーを
見せない」——理由は spurious 連鎖の**構造的**遮断である。

VK_KANA だけを ImmCross でも Allow にすると:

1. ユーザーがかなキーを押す → awase が belief を更新 → ImmCross で
   `ImmSetOpenStatus(TRUE)` を書く。
2. 同時に再注入された VK_KANA が MS-IME に届き、MS-IME が自分でモードを変える
   （かつ OS のかなロックもトグルしうる、B3）。
3. 次の poll（`classify_fetched_snapshot`）が、awase の書き込みと MS-IME の
   自律変更の**どちらの結果か区別できない** conv 値を High confidence で観測し、
   `input_mode` belief に反映する。
4. `conv_mutation_seq`（`conv_mutation.rs:39` の「SendInput 経由、
   VK_DBE_*/VK_KANA/VK_CONVERT」というゲート）は awase 自身の送信しか
   数えないため、MS-IME 側の自律変更分を照合できない。

これは BUG-107（`ImmCapabilityStore` のプロセス間汚染）と同種の、
「2つの主体が同じ状態を書き、観測がどちらのものか区別できない」構造である。

**推奨**: ImmCross は現状維持（Suppress のまま）。VK_KANA だけを
`feedback_immcross_owns_kanji` の例外にする積極的な理由は見当たらない。
ただしそうすると「VK_KANA は常に PhysicalDelivery」という ADR-183 の
単純な語りは成立せず、プロファイル依存の条件付き分類になる
（＝ADR-179 が避けようとした「特殊条件の追加」そのものになる、ADR-179 round4 R4-3）。

### M6: `ime_actuation_owned` が false の環境では既に Down/Up 共 Allow であり、診断の切り分けに使える

`key_sequence_policy.rs:51-62`:

```rust
pub(crate) const fn gji_direct_applicable(kind: ImeKindId) -> bool {
    matches!(kind, ImeKindId::Gji)
}
pub(crate) fn ms_ime_direct_applicable(kind: ImeKindId, profile: AppImeProfile) -> bool {
    matches!(kind, ImeKindId::MsIme) && !profile.can_use_imm32_cross_process()
}
```

すなわち **ATOK 等の第三の IME では `ime_actuation_owned == false` となり、
VK_KANA は KeyDown も KeyUp も既に Allow されている**。

ADR-181（ATOK/GJI のキーマップ問題、同時に起票中のドラフト）が示唆するように、
このユーザーが ATOK を使っている可能性がある場合、ADR-183 の仮説は
**そもそも成立しない**（既に Down/Up 共通っている環境で「KeyUp が消えるから」
という説明はできない）。

これは M2 と並ぶ、**ログを見れば即座に排除できる診断分岐**である。
`[hook] IME-mode ... since_actuation_us=` の周辺および
`tsf::observer::active_ime_kind()` の値で確定する。

### M7: VK_KANA 専用分岐の配置位置が、既存の3つのガードのどれかを必ず弱める

ADR-183「未解決の疑問4」はこれを `event.injected` チェックとの前後関係だけの
問題として扱っているが、実際には**3つのガードすべてとの前後関係**を決める必要がある。
提案されている「常時パススルー」の `return Self::Allow;` をどこに置いても
副作用がある:

| 配置位置 | 失われるもの |
|---|---|
| `InputRelay` 判定（`:276`）より前 | 影響なし（InputRelay も Allow なので同値）。ただし意味の重複 |
| `injected` 判定（`:318`）より前 | `debug_assert!(!shadow_toggled, ...)`（BUG-14 ガードの検証）を VK_KANA についてスキップする。BUG-14 の「注入 IME キーを shadow_toggled に昇格させない」不変条件が VK_KANA について**検証されなくなる**（hook.rs が foreign-injected VK_KANA を swallow しているので実害は小さいが、`hook.rs:1173-1180` が記録するとおり**この swallow は過去に一度撤回されている**——再撤回されたときに気付けなくなる） |
| `injected` 判定より後・ImmCross 分岐より前 | M5 の ImmCross 問題が確定的に発生する |
| ImmCross 分岐の中で VK_KANA だけ除外 | `feedback_immcross_owns_kanji` の「構造的遮断」が「VK ごとの例外リスト」に退化する。ADR-179 round4 R4-3 の「簡素化ではなく特殊条件の追加」判定に該当 |

**どの位置も無害ではない。** ADR-183 は「対称な VK_KANA 専用分岐を追加する」
（109-118行目）と書いているが、`VK_CONVERT`/`VK_NONCONVERT` 分岐が
`injected` チェックの**後**に置かれているのは偶然ではなく、上表の帰結である。

---

## Nits

### N1: ADR-179 のステータス記述が実態と食い違っている

ADR-179 の frontmatter は `status: **収束済み（...）実装未着手。**` だが、
`ModeKeyActuationOwner` は既に実装済みである:

- 計算点: `key_pipeline.rs:1455`（`event.ime_relevance.actuation_owner = if delegate_owned {`）
- ガード: `tests/architecture_guard.rs:809`
  （`actuation_owner_is_computed_in_exactly_one_place`、`NEEDLE = "ime_relevance.actuation_owner ="`）

ADR-183 が ADR-179 を「型を導入した先行 ADR」として参照している以上、
ADR-179 のステータス更新も併せて行うこと
（記憶 `feedback_verify_adr_status_via_git_log_not_status_section` の再発）。

### N2: 実装済みコードに ADR-179 に無い実験的変更が入っている

`key_pipeline.rs:1440-1448` のコメント:

> 2026-09-18（ユーザー指示、実験的）: 主条件から `!is_configured_thumb_key` を撤廃した。

ADR-179 の「owner の定義」（203-237行目）は主条件を
「対象VK**かつ**On/Off分類**かつ非親指キー設定**」と規定しているため、
現在のコードは ADR-179 の記述と乖離している。ADR-183 がこの機構を拡張する
以上、拡張前に ADR-179 側の記述を現状へ同期させること
（`.claude/rules/experiment-logging.md` の対象範囲）。

### N3: `ModeKeyActuationOwner` の実消費点の洗い出し結果（「疑問5」への回答）

実際に `grep -rn actuation_owner` で確認した消費点は以下の3箇所のみ。
ADR-183 が懸念した「他の消費点が古い `AwaseExplicit` 前提のまま残る」
というリスクは、少なくとも現時点では**小さい**（ただし B4 の結論は変わらない）:

| 箇所 | 用途 |
|---|---|
| `runtime/key_pipeline.rs:458` | `ActivationSync` 由来 `SetOpen` の strip（消費点2） |
| `runtime/key_pipeline.rs:1751-1754` | `owner_permits_explicit_off_actuate`（ON→OFF 明示 actuate の抑止） |
| `runtime/key_pipeline.rs:1751`（tracing 引数） | 診断ログのみ |

デフォルト値の設定点は `hook.rs:314` / `state/evidence.rs:553` /
`state/platform_state.rs:2412` の3箇所（いずれも `default()`）。

### N4: 提案テスト（`plan_tests` への「VK_KANA Down/Up 対称性」テスト）は B1 未確定のまま書くと誤った仕様を固定する

ADR-183「疑問6」の提案どおり `plan_tests` に Down/Up 対称性テストを足すと、
**「VK_KANA には KeyUp が存在する」という（B1 と矛盾する）仕様を
テストとして固定してしまう**。B1 の確認後に、実際に観測されたイベント形状に
合わせて書くこと。

---

## 診断プロセスそのものへの評価（「疑問1」「疑問7」への回答）

**「実機ログを取らずにコードリーディングだけで設計を確定させるのは、
この領域では明確に不適切である」と判定する。** 理由は3つ:

1. **必要なログが既に全部ある。** 追加実装ゼロで、かなキーを1回押すだけで
   B1・M2・M6 の3つの診断分岐が同時に潰せる:
   - `[hook] IME-mode vk=0x.. {dir} self_injected=.. injected=.. scan=0x..`（`hook.rs:1148`）
     → **押されたキーの VK が 0x15 か 0xF0/0xF1/0xF2 か**（M2）、
       **up が来るか**（B1）
   - `[hook] VK_KANA {dir} 到達 (injected=false, scan=0x..)`（`hook.rs:1281`、INFO）
     → 同上、VK_KANA 専用の確証
   - `[shadow-toggle] no-op: vk=0x.. action=.. effective_open は既に ..`（`key_pipeline.rs:1594`）
     → belief 側が no-op 分岐に落ちているか（M1 の conv 軸仮説の確認）
   - journal の `KeyInput.physical` + `suppress_reason`（`transport.rs:30-45`、BUG-90 対応）
     → **実際に Suppress されたか、その理由ラベルは何か**（仮説の直接検証）

   最後の項目は、まさに「decision（意味論）と physical（実配送）を突き合わせる」
   ために BUG-90 調査で追加された機構である。**ADR-183 が立てた仮説を
   検証するための計器が、既にこのリポジトリに実装されている。**

2. **この領域は「コードは正しいが前提が誤っている」失敗の再発ファミリーである。**
   ADR-179 は8ラウンドを要し、そのうち**4ラウンドは同型の前提誤り
   （「送信元が移動するだけ」）の繰り返し**だった。今回も B2 で、
   `executor.rs` 自身が「この mental model が BUG-116 の一因になった」と
   訂正を明記している前提を、ADR-183 が再び踏んでいる。
   記憶 `feedback_confirm_input_vs_output_layer_before_log_archaeology`
   （「実機検証で症状が出たら入力/出力どちらの層かを最初に確認する」）が
   まさにこのケースに該当する。

3. **誤診のコストが非対称。** 仮に ADR-183 の設計を実装した場合、
   B3 のシナリオ（かなロック反転）は**ユーザー環境で復旧不能な破損**を起こす。
   一方、実機ログを取るコストは数分である。

### round2 へ進む前の宿題（推奨）

1. **実機ログ1本**: IME ON・Eisu・Engine ON の状態でかなキーを1回押し、
   `[hook] IME-mode` / `[hook] VK_KANA` / `[shadow-toggle]` /
   journal の `KeyInput.physical`+`suppress_reason` を取得する。
   これで B1・M2・M6 と、仮説の正否そのものが確定する。
2. **かなロックの現状確認**: 同じ操作の前後で
   `observer::kana_lock::read_kana_lock()` の値が変わるか
   （`[kana-mode-restore] ABORT:` ログ、または診断出力）。
   変わるなら B3 が「将来のリスク」ではなく「現在のバグ」になり、
   症状の第一容疑に昇格する。
3. **ADR-137 M-6 / BUG-116 との同一性判定**（M1）。同一なら ADR-183 は
   撤回し、`kp_restore_hiragana_for_suppressed_mode_key` の VK_KANA 対応
   という遥かに小さい変更に置き換わる。

この3点が揃うまでは、`transport.rs::plan()` の条件式を変える設計は確定させない
ことを推奨する。

---

## 引用行番号の検証結果（ADR-183 本文の裏取り）

| ADR-183 の引用 | 検証結果 |
|---|---|
| `vk.rs:83-89,140-151`（`Kana` doc / `shadow_effect`） | ✅ 正確 |
| `hook.rs:1254-1288`（VK_KANA 専用ブロック） | ✅ 正確（`if vk == crate::vk::VK_KANA {` が 1254行目） |
| `key_pipeline.rs:1275`（`kp_stage_shadow_ime_toggle` 開始） | ✅ 正確 |
| `key_pipeline.rs:1612`（no-op 早期 return） | ✅ 正確 |
| `key_pipeline.rs:1449-1452`（`is_target_vk`） | ✅ 正確 |
| `key_pipeline.rs:457-463`（`ActivationSync` strip） | ✅ ほぼ正確（実際は 457-462） |
| `transport.rs:292-298`（VK_DBE_HIRAGANA 専用分岐） | ✅ 正確 |
| `transport.rs:318-325`（`injected` チェック） | ✅ 正確 |
| `transport.rs:363-372`（VK_CONVERT/NONCONVERT 分岐） | ✅ 正確 |
| `transport.rs:374-419`（KANJI 系分岐） | ✅ 正確 |
| `executor.rs:463-473`（`Decision::PassThrough` の Suppress アーム） | ✅ ほぼ正確（実際は 464-471） |
| `hook.rs::classify_ime_relevance` が `shadow_action` を埋める | ✅ 正確（`hook.rs:285-300`、`ImeKeyKind::from_vk(0x15) → Kana → TurnOn`） |
| VK_KANA が `is_kanji_event` 分岐に到達する | ✅ 正確。`is_ime_control(0x15)==true`（`vk.rs:292`）→ `nicola_fsm.rs:3243` の `BypassReason::ImeControl` → `Decision::PassThrough` → `execute_relay` の PassThrough アームが `physical` を参照する（`executor.rs:464`） |

**経路追跡は正確である。** 問題は経路ではなく、
「その経路の Suppress がこの症状を起こす」という因果の主張
（B1・M1・M2）と、「無条件 Allow が安全である」という主張（B2・B3・B4）にある。
