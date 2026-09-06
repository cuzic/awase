# ADR-143: かな系キーの役割代入（Kana Key Role Substitution）

## ステータス

**r1（Opus 2体の敵対的レビューをr1で1ラウンド実施。Blocker 3件・Major多数を
反映。r2レビュー未実施）。**

本ADRは、ADR-141（物理キー役割代入・Phase A、変換/無変換/スペースの3キー
間の入れ替え）決定0が「安全な3キー」へスコープを縮小した際に切り出した、
「ひらがな/かな系キー」を役割代入の対象に含めるための後続ADRである
（ADR-141決定0が暫定的に「ADR-14x: かな系キーの役割代入」と呼んでいたもの）。

**起草前に判明した、要望自体の再定義が必要な事実**: ユーザー要望は当初
「かな系キーを相互に入れ替える」という字義だったが、調査の結果、JIS配列で
「かな/カタカナ/ひらがな」に対応する物理キーは**scan 0x70の1個しか存在しない**
ことが判明した（詳細は背景節）。したがって「かな系キー**同士**の入れ替え」は
物理的に成立しない。ユーザーに確認した結果、実際に求められているのは
**この1個の物理かなキーと、ADR-141の「安全な3キー」（変換/無変換/スペース）
のいずれかとのクロスファミリー入れ替え**（例:「変換キーと物理かなキーを
入れ替える」）であることを確認済み（2026-09-06、本ADR起草者との対話）。
本ADRはこの再定義されたスコープで設計する。

**r0→r1で判明した、r0が見落としていた最重要事実**: r0の決定2は「to側を
`VK_DBE_HIRAGANA`固定にすれば安全」と結論していたが、r1レビューで
**Alt/Win押下中に合成`VK_DBE_HIRAGANA`をSendInputすると、MS-IME・GJI
問わずAlt+かなの入力方式切替ショートカットと同様に解釈され、BUG-61
（復旧不能）を新規に誘発しうる**という実機診断記録（`key_pipeline.rs:1983-1989`、
2026-08-17実機診断）が既にリポジトリ内に存在することが判明した。加えて、
hook.rs上流の`CACHED_SWALLOW_ALT_KANA_MODE_SWITCH`/`VK_KANA`+Alt早期return
（挿入点より前段）がかなスロットのKeyUpを消費してしまい、r0決定5が
「変更不要」としていたのは**既定設定でも実際は不十分**（`swallow_alt_kana_
input_method_switch=false`のオプトアウト設定では素通しされ、非対称な
Down/Upの実機ログが既存known-bugs.mdに記録されている）と確定した。r1は
これらを踏まえ決定2・決定4・決定5・決定6・決定7を全面的に書き直した。

### r1レビュー指摘との対応表

architect役: B1→決定5、B2→決定2、B3→決定4、M1→行番号全体、M2→決定1、
M3→決定6、M6→決定3、M7→未解決の疑問1。
premortem役: B1(Alt+合成0xF2でBUG-61)→決定2、B2(上流swallowのKeyUp消費)
→決定5、B3(オプトアウト設定での非対称Down/Up)→決定5、M1(MS-IME
scan=0無反応)→決定2/未解決の疑問4、M2(Shift併用で逆にカタカナへ飛ぶ)
→決定2/決定7、M3(THUMB_KEY_OPTIONS事実誤認)→決定6、M5(IME OFF方向の
喪失)→決定2、M6(conv_mutation・既定ホットキー)→決定9(新設)。

両エージェントが独立に到達した一致点（信頼度が高いと判断）: (a)
`THUMB_KEY_OPTIONS`の実際の3値は`VK_KANA`/`VK_DBE_KATAKANA`/
`VK_DBE_HIRAGANA`であり0xF0は含まれない、(b) 上流のAlt+かなswallow
ガードがかなスロットの役割代入と衝突しうる、という2点。

## 背景

### ADR-141決定0がかな系キーを除外した理由（再掲）

ADR-141決定0（`docs/adr/141-physical-key-role-substitution.md:111-149`）は
以下を確定させている:

- JIS配列の物理「ひらがな」キー（scan 0x70）は、`VK_KANA`(0x15)としては
  ほぼ届かない。実際にはOSのキーボードレイアウト変換層とIMEの現在状態に
  応じて`VK_DBE_HIRAGANA`(0xF2)/`VK_DBE_KATAKANA`(0xF1)/
  `VK_DBE_ALPHANUMERIC`(0xF0)のいずれかとして届く。
- `VK_DBE_*`一族（0xF0-0xF6）は`runtime/transport.rs::plan`のSuppress/Allow
  判定（BUG-52/116）、`hook.rs`の`CACHED_SWALLOW_ALT_KANA_MODE_SWITCH`
  （BUG-08/61/62対策）など、既存のIME actuationロジックに深く組み込まれて
  いる。
- 特に`to`が`VK_DBE_ROMAN`(0xF5)/`VK_DBE_NOROMAN`(0xF6)になる代入は、
  BUG-61（Windows Terminal + MS-IMEでJISかな入力方式に一度切り替わると、
  `ImmSetConversionStatus`・SendInput・IMC write のいずれを試しても実機4段階
  検証で一切復旧しないと確定済み）を誘発しうる。OS側の構造的制約であり、
  awase側の実装改善では回避不可能（`docs/known-bugs.md:8489-8510`）。

### 物理配線の実態（本ADR起草にあたっての追加調査で確定）

- `crates/awase-windows/src/vk.rs:196-226`（`is_synthetic_dbe_ime_hotkey`）:
  0xF0-0xF4は「通常の物理キーボードには存在しない、IME専用の合成VKコード」
  であり、OS/IMEのレイアウト変換層が現在の状態に応じて動的に生成する。
- JIS配列でこれらのVKを生成しうる物理キーは**scan 0x70の1個だけ**。この
  1個の物理キーは、単独押下で通常`VK_DBE_HIRAGANA`(0xF2)だがIME状態に
  よって`VK_DBE_KATAKANA`(0xF1)/`VK_DBE_ALPHANUMERIC`(0xF0)になることがある
  （BUG-52）。Shift併用時は`VK_DBE_KATAKANA`(0xF1)（Windows標準仕様、
  BUG-116/ADR-137実機確認済み）。Alt併用時は`VK_DBE_ROMAN`(0xF5)/
  `VK_DBE_NOROMAN`(0xF6)（BUG-61/62）。
- `crates/awase-settings/src/main.rs:3920-3928`の`THUMB_KEY_OPTIONS`は
  実際には次の3項目である（**r1訂正**: r0は`VK_DBE_ALPHANUMERIC`を含む
  3値と誤記していたが、実装は`VK_KANA`/`VK_DBE_KATAKANA`/`VK_DBE_HIRAGANA`
  であり0xF0は選択肢に存在しない）:
  ```
  ("かな",     "VK_KANA"),          // 0x15
  ("カタカナ", "VK_DBE_KATAKANA"),  // 0xF1
  ("ひらがな", "VK_DBE_HIRAGANA"),  // 0xF2
  ```
  この3項目が別々に提示されているのは、複数の物理キーがあるという意味では
  なく、同一の物理キー(scan 0x70)がIME状態に応じてどのVKとして届くかを
  ユーザーに選ばせているワークアラウンドである（この関係が非決定的である
  こと自体がBUG-52として過去にバグ化した）。ただし「かな」=`VK_KANA`
  だけは例外で、ADR-141決定0が確定したとおり実機ではほぼ届かない値であり、
  この選択肢自体が有効に機能する場面は薄い。
- `VK_KANA`(0x15)はADR-141決定0が確定したとおり実機ではほぼ届かないが、
  本ADRの決定1（scan値ベースのfrom判定）はvk値を見ないため、**稀に
  `VK_KANA`として届いた場合であってもscanが0x70である限り代入対象に含まれる**
  （意図した挙動、詳細は決定1参照。r0は「VK_KANAは対象外」と書いていたが
  scanベース判定と矛盾していたため訂正）。

この事実により、「かな系キーを相互に入れ替える」という要望は、ADR-141の
「複数の独立した物理キー間の入れ替え」とは意味論が異なる。本ADRが実際に
設計するのは、**この1個の物理キー（以下「かなスロット」）を、ADR-141の
全単射モデルにおける第4のオブジェクトとして追加し、安全な3キーの
いずれかと相互に入れ替え可能にする**ことである。

### ADR-141の再注入機構（本ADRが前提とする既存アーキテクチャ）

本ADR起草にあたり、ADR-141が依拠する`hook_callback`の実際の配送機構を
再確認した（`crates/awase-windows/src/hook.rs`。**r1訂正**: r0は行番号を
複数箇所で誤って引用していた。以下は実測値）:

- `hook_callback`は通常時、`ncode<0`・自己注入・overflowラッチ・
  `FOCUS_APP_DISABLED`の早期return以外では**常に`LRESULT(1)`を返して
  元の物理イベントを問答無用でSuppressする**（`hook.rs:1199`）。OSへの生
  イベント配送という経路自体が構造的に存在しない。
- `vk`はローカル変数（`hook.rs:900`、`let mut vk = VkCode(kb.vkCode as
  u16);`）としてAlt impersonation適用時に書き換えられ（`vk =
  rewritten_vk;`、`hook.rs:1135`）、`classify_key`（`:1171`付近）と
  `build_raw_key_event`（`:1184-1193`）へ渡る。`scan`（`hook.rs:901`、
  `let scan = ScanCode(kb.scanCode);`）は`mut`が付いておらず、挿入点まで
  一切書き換えられない。
- `build_raw_key_event`が構築する`RawKeyEvent`は、`vk_code`に**書き換え後
  のvk**、`scan_code`に**書き換えなしの元の物理scan値**を格納する。この
  非対称な構造が既に存在する。
- `PhysicalKeyDisposition::plan`が`Allow`を返した場合、実際にOSへ届くのは
  `RawKeyEventExt::reinject()`（`crates/awase-windows/src/lib.rs:348-376`、
  `wScan: 0`固定は363行目）が`SendInput`する、`vk_code`基準・`scan: 0`
  固定の合成イベントである。「Allow」という名前だが、元の物理イベント
  そのままではなく常に`vk_code`から再構成された合成イベント。

この事実（`RawKeyEvent.scan_code`が書き換えられずに保持される）は、
「かなスロットの物理キーをvkに関わらず識別する」という本ADRの要件に
極めて好都合であり、決定1の設計根拠になる。

## スコープ

- **対象**: 物理かなキー（scan 0x70、以下「かなスロット」）と、ADR-141の
  安全な3キー（`VK_CONVERT`/`VK_NONCONVERT`/`VK_SPACE`）の間の役割代入。
  かなスロットを含めた4オブジェクトの全単射（決定3）として、ADR-141の
  モデルを拡張する。
- **対象外（本ADRでは扱わない）**:
  - `to`に`VK_DBE_ROMAN`/`VK_DBE_NOROMAN`(0xF5/0xF6)を割り当てること
    （決定2、BUG-61により構造的に禁止）。
  - `to`に`VK_DBE_ALPHANUMERIC`/`VK_DBE_KATAKANA`/`VK_DBE_SBCSCHAR`/
    `VK_DBE_DBCSCHAR`(0xF0/0xF1/0xF3/0xF4)を割り当てること（決定2、
    `transport.rs`の既存無条件Suppressガードにより機能しないため）。
  - Alt/Win/Shift押下中に発生する役割代入（決定2、実機診断で危険と確定
    したため代入自体を新規押下時点で不発火にする）。
  - config形式・設定GUIの具体的な表現（Phase C、ADR-142と同様に後続ADRへ
    切り出す。決定8参照）。
- かなスロットが自分自身へ写る（＝代入なし）場合、既存の挙動
  （BUG-52/116のガードを含む）は一切変更しない。本ADRの分岐は、かなスロット
  と安全な3キーのいずれかとの間に**非自明な**入れ替えが設定された場合にのみ
  発火する。

## 決定

### 決定1: from判定をかなスロット専用にscan値ベースへ拡張する

ADR-141決定1の挿入点（`hook.rs:1135`の`vk = rewritten_vk;`直後、`if
!is_injected`ブロック内側先頭）はそのまま維持する。安全な3キーの`from`
判定は引き続きvk値の一致で行う（ADR-141と同一）。かなスロットの`from`
判定のみ、以下の条件に変更する:

```
is_kana_slot_press = (scan == SCAN_KANA) && event_eligible
```

`event_eligible`はADR-142決定B7が安全な3キー用に確立した
`!alt_impersonated && !is_injected`と**同一のパラメータをそのまま流用する**
（**r1修正**: r0は独自に`!is_alt_impersonated_output`という項を立てていたが、
これは`is_alt_impersonated_output`が判定対象にできる状況が存在しない空文
であり〈`apply_alt_impersonation`が書き換えるのはvk∈{0x12,0xA4,0xA5}の
scanが0x70になることはない〉、本来ここに必要だったのは`!is_injected`
だった。他プロセスがLLKHF_INJECTEDでリレーした入力〈PowerToys Mouse
Without Borders等〉は`hook.rs:935`の`is_self_injected`早期returnを
通過して挿入点まで到達するため、`!is_injected`を落とすとADR-119
〈issue #136〉が確立した「解釈しない入力は消費しない」方針に違反する）。

`SCAN_KANA`は`0x70`のマジックナンバーを直接書かず、名前付き定数として
追加する。置き場所は`crate::vk`ではなく`crates/awase-windows/src/
scanmap.rs`（scan値のSSOT、`scan_to_pos_jis`/`pos_to_scan_jis`を既に
所有する）とする（**r1修正**: r0は`vk.rs`を提案していたが、`vk.rs`には
既に`VkCode(0x70)`＝`VK_F1`という別のVK空間の値が定義されており
〈`vk.rs:467`〉、同じ数値の別意味の定数を同じファイルに置くと混同を招く）。

`scan`は`hook.rs:901`で一度読み取られた後、`mut`が付いていないため
（Alt impersonationを含め）挿入点まで一切変更されない。したがって
`kb.scanCode`をそのまま使ってよい。

安全な3キーとの決定的な違い: 安全な3キーは「vk↔物理キー」が1:1で安定して
いるためvkだけで`from`を識別できる（ADR-141決定1-2の全単射前提）。かな
スロットは同一物理キーが状態依存で複数のvk（0xF0/0xF1/0xF2、Shift併用で
0xF1、Alt併用で0xF5/0xF6）を生成しうるため、vkベースの判定では取りこぼしが
生じる。scanベースの判定に変えることで、この非決定性を役割代入の判定から
構造的に排除できる（副次効果として、かなスロットをどこか別のキーへ代入した
場合、BUG-52型の「意図せず0xF0/0xF1が生成されて素通しされる」という問題
自体がそのキーには発生しなくなる——scanが一致すれば生成されたvkが何であれ
代入判定に乗るため）。

**副作用として明記が必要な点**（r1レビュー指摘）: 物理かなキーが役割代入で
別のキーへ変換されると、その物理キーは`classify_ime_relevance(vk)`
（代入後vk基準で計算）経由の`shadow_action`をもはや生成しなくなる。つまり
shadow IME beliefは物理かなキー押下を一切観測しなくなる。これは
`state/ime_model.rs`が属する再発ファミリー（IME belief、
`.claude/rules/ime-belief-architecture.md`参照）に触れる帰結であり、
実装時にこの観測経路の消失が既存のbelief更新ロジックに悪影響を与えないか
個別に確認が必要（未解決の疑問5）。

### 決定2: toがかなスロットになる場合の合成vkを`VK_DBE_HIRAGANA`に限定し、Alt/Win/Shift押下中は代入自体を不発火にする

安全な3キーのいずれかがかなスロットへ代入される場合（例:
「変換→かな」）、生成する合成イベントの`vk_code`は常に`VK_DBE_HIRAGANA`
(0xF2)に固定する。他の`VK_DBE_*`亜種を`to`として選択させない（config
読み込み時にバリデーションエラーとする、ADR-142決定と同様「実行時無効化
のみ、config.tomlの書き換えはしない」方針を踏襲）。

**r1で追加した制約（最重要）**: かなスロットへの代入は、新規KeyDown時点
（`was_down`が false から true になる遷移、ADR-141決定2の「新規押下時点
でのみ参照する」規律と同一）で、以下の**両方**が満たされる場合にのみ発火
する。満たされない場合、その物理キーは代入なしで元の挙動のまま扱う
（`confirmed_target = None`のまま、そのDown/Upのライフサイクル全体で
固定——ADR-141の`engine_enabled`と同じ、押下中の状態変化で発火有無が
入れ替わることを防ぐ規律）。

1. `!ime_mode_key_injection_blocked_by_modifier()`（`hook.rs:296-298`、
   `win_key_held() || alt_key_held()`）。この関数は既に
   `kp_restore_kana_from_half_width`のMS-IME分岐と
   `Output::send_gji_half_width_alnum_toggle`のGJI分岐の両方が共有する
   既存の判定点であり、IME種別を問わず「Alt/Win押下中にDBEモードキーを
   送るとOSレベルの入力方式切替ショートカットと解釈される」危険を防ぐ。
2. `!event.modifier_snapshot.shift`（Shift押下中ではない）。

**なぜこの制約が必要か（r0からの変更理由）**: r1レビューで、Alt押下中に
合成`VK_DBE_HIRAGANA`をSendInputすると、実機診断（2026-08-17、専用
プローブツールでの検証、`crates/awase-windows/src/runtime/
key_pipeline.rs:1983-1989`）により「MS-IMEのAlt+かなローマ字⇔JISかな
直接入力切替ショートカットと同様に解釈され、実際にJISかな直接入力へ
切り替わる」ことが既に確認済みであると判明した。これはBUG-61
（復旧不能）そのものである。既存の2つのDBEキー送出経路
（`key_pipeline.rs:2004`のMS-IME側、`output/mod.rs:1201`の
`send_gji_half_width_alnum_toggle`）はいずれも`ime_mode_key_injection_
blocked_by_modifier()`を経由済みだが、本ADRが新設する`reinject()`
（`lib.rs:348-376`）は単発`INPUT`を素で送るだけでこのガードを経由しない。
決定2はこのガードを明示的に呼び出す3つ目の経路として位置づける。

Shiftを併用条件から除外する理由は決定7を参照。

**根拠（0xF2固定・0xF5/0xF6構造的禁止について）**:
- `transport.rs::plan`のF2専用分岐（`:276-282`）は`event.vk_code ==
  VK_DBE_HIRAGANA`のみを条件にしており、Suppress/Allowの判定
  （`is_tsf_mode && f2_warmup_owned`）は代入元が物理かなキーか役割代入かを
  区別しないグローバル状態で決まる。したがって代入で生成した0xF2イベントも、
  既存のGJI/MS-IME warmup契約にそのまま安全に乗る。
- 0xF0/0xF1/0xF3/0xF4を`to`に許すと、`transport.rs:340-351`の
  「`ime_actuation_owned`時は`shadow_toggled`に関わらず常にSuppress」という
  無条件ガード（**r1訂正**: r0は`:213-222`という無関係な関数を誤って
  引用していた）に毎回引っかかり、素通しされず機能しない（BUG-52対策の
  ガードが役割代入の出力も無差別に殺してしまう）。
- 0xF5/0xF6（`VK_DBE_ROMAN`/`NOROMAN`）はBUG-61（復旧不能）に直結するため
  `to`として構造的に禁止する。

**既知の制限（r1で修正・追加）**:
1. **IME OFF方向の代入は再現しない**: 物理かなキーは、IMEが既にひらがな
   状態にある時に押されると実機的に「かな入力を終了する」効果（内部的
   には0xF0=`VK_DBE_ALPHANUMERIC`生成）を持つことがあるが、決定2の合成
   vkは常に0xF2固定であり、この「IME OFF方向」の効果は代入後の位置では
   再現できない（**r1追加**、r0はこの制限を明記していなかった）。
2. **`kp_restore_hiragana_for_suppressed_mode_key`（BUG-116決定2、
   `key_pipeline.rs:81-141`）が代入後イベントでも発火する**: この関数は
   `event.vk_code == VK_DBE_HIRAGANA`かつ`event.injected == false`
   （代入対象は実物理キー押下由来のため常にfalse）かつ非Shift
   （決定2の制約により常に非Shiftで到達）かつ`physical ==
   PhysicalKeyDisposition::Suppress`の場合に、GJI半角英数トグルの
   復元処理（`send_gji_half_width_alnum_toggle(Exit, ..)`）を実行する。
   この関数は物理かなキーの専用ゲート（`is_configured_thumb_key`・
   `half_width_alnum_toggle_before`・`kana_mode_restore_key_down`という
   単一グローバルラッチ）を前提に設計されており、これらが「代入後は
   別の物理キーに適用される」ことを想定していない。実装時にこれらの
   ゲート全てを役割代入の観点で再監査する必要がある（未解決の疑問6）。
3. **MS-IME環境での有効性は未検証**: `reinject()`が送る合成イベントは
   `wScan: 0`固定である。`key_pipeline.rs:1955-1961`のコメント
   （2026-07-07実機）は「scan=0の`send_ime_mode_key`ではMS-IME(TSF)が
   モードキーとして処理しない」と記録している一方、GJIは`wScan: 0`
   のままカタカナへ切り替わることをADR-137（BUG-116、2026-09-05実機）
   で確認済みである。したがって決定2はGJIでは機能しMS-IMEでは無反応、
   というIME依存の分裂を起こす可能性が高い（未解決の疑問4）。この
   「自然な修正」（scan 0x70を付与する）はBUG-15追補7（scan付きDBEキー
   注入によるJISかな固着ハザード）に直行するため単純には採用できない。
   実装前に両IMEでの実機確認が必須（`.claude/rules/tuning-constants.md`
   と同水準の実測義務に準じる）。

### 決定3: 全単射モデルを4要素（安全な3キー＋かなスロット）へ拡張する

ADR-141決定1-2の「役割代入ルールの集合は対象キー上の全単射（置換）で
なければならない」という制約を、対象オブジェクトを{変換, 無変換, スペース,
かな}の4要素に拡張して適用する。明示的にルールが無いオブジェクトは恒等
として補完し、補完後の写像が単射（異なる2つの入力が同じ出力を持たない）
であることをconfig読み込み時に検証する。違反する設定はADR-141と同様
ルール集合全体を無効化する。

4要素に拡張しても、ADR-141決定8（`KeymapLatch`）が前提とする「代入後の
1つのvkを生成しうる物理キーは常にちょうど1つに定まる」という不変条件は
全単射性により構造的に保証され続ける。

**r1追加（architect役M6指摘）**: ADR-141決定4はAlt impersonationとの
不動点制約（左右両方をAltセンチネルにすると`VK_NONCONVERT`/`VK_CONVERT`
双方が不動点になり、残る`VK_SPACE`も自動的に不動点にならざるを得ず、
本機能が構造的に使用不可になる）を安全な3キーの3要素上で検証していた。
かなスロットを加えた4要素上でこの不動点制約を再検証する必要がある。
むしろ4要素化は**この制約を緩和する**: 2つのAltセンチネルが2つの不動点を
固定しても、残る{`VK_SPACE`, かなスロット}の2要素で2-cycleを組めるため、
ADR-141 r3が「両Altセンチネル構成では本機能が構造的に使用不可」と結論
していた制約が本ADRのスコープでは発生しない。この点をADR-141側にも
参照ポインタとして残すこと。

### 決定4: hold-state・KeymapLatchをかなスロット専用ペアとして追加し、Down時の実際のOS配送結果を記録する

ADR-141決定2は、安全な3キーそれぞれに固定長・名前付きの
`{KEY}_WAS_DOWN: bool`/`{KEY}_CONFIRMED_TARGET: Option<VkCode>`を持つ
（3キー×2=6変数、インデックスは代入前の物理vk）。かなスロット用に、同型の
`KANA_WAS_DOWN`/`KANA_CONFIRMED_TARGET`を専用に追加する。加えて、**r1で
新設**する第3のフィールド`KANA_DOWN_WAS_ALLOWED: bool`を持つ。

安全な3キー用のペアとの決定的な違い: 安全な3キーの`{KEY}_WAS_DOWN`は
「代入前の物理vk」でインデックスされる（vkが物理キーと1:1で安定して
いるため）。かなスロット用の`KANA_WAS_DOWN`は、vkではなく**scan値が
0x70であること**をトリガーにする（決定1と同じ理由）。同じ
`decide_role_substitution`関数（ADR-141決定2で新設予定の判定関数）は
共通で流用できるが、呼び出し側の「fromをどう識別するか」の判定ロジック
だけがキー種別（安全な3キー vs かなスロット）で分岐する非対称設計になる
ことを明記する。

**r1で修正したKeyUp注入の前提条件（architect役B3指摘への対応）**:
ADR-141決定7のKeyUp注入パターン（確定した代入先のKeyUpを先に送出してから
状態をクリアする）を無条件に流用してはならない。かなスロットが`to`側
（安全な3キー→かな）の場合、`transport.rs::plan`のF2分岐は
`is_tsf_mode && f2_warmup_owned`という単一条件でKeyDown/KeyUpの**両方**
を同じ結果（Suppress または Allow）に振り分ける。GJI既定戦略
（`f2_warmup_owned=true`）ではDown自体がSuppressされ、OSは一度も0xF2の
Downを見ていない。この状態でADR-141決定7のパターンをそのまま適用すると、
**対応するDownを持たない新規のDBEモードキーKeyUpをOSへ送ってしまう**
（BUG-52の無条件Suppressガードが防いでいる当のもの）。

**決定**: `KANA_DOWN_WAS_ALLOWED`にDown処理時点の`PhysicalKeyDisposition::
plan`の結果（`Allow`なら`true`、`Suppress`なら`false`）を記録する。
KeyUp注入（決定5のswallow分岐からの注入を含む）は`KANA_DOWN_WAS_ALLOWED
== true`の場合にのみ行う。`false`の場合はOSに何も送出されていないため、
内部hold-stateのクリアのみを行う。

### 決定5: 上流のAlt+かなswallowガードに、かなスロットの保留KeyUpを注入してからクリアする分岐を追加する

**r0からの方針転換**: r0は「`CACHED_SWALLOW_ALT_KANA_MODE_SWITCH`は挿入点
より前段にあるため構造的に独立、変更不要」と結論していたが、これは
**誤りだった**（architect役B1・premortem役B2/B3で独立に指摘）。「前段に
ある」ことこそが、かなスロットの保留中KeyUpを取りこぼす原因になる。

**問題の実機的根拠**: `hook.rs:1066-1087`（`VK_DBE_ROMAN`/`NOROMAN`
swallow）と`hook.rs:1013-1046`（`VK_KANA`+Alt押下時のswallow）は、
いずれもKeyDown/KeyUpを区別せず無条件に`return LRESULT(1)`する早期return
である。BUG-62追補4の実機ログ（`docs/known-bugs.md:8746-8752`）は、
物理かなキーをAlt併用でリリースした際の実際のイベント列が
「`vk=0xF5 KeyUp`→`vk=0xF6 KeyDown`」という、Down/Upが対で来ない
非対称な列であることを記録している。

**失敗シナリオ**（config: かな↔スペースの2-cycle、既定設定）: (1) 物理
かなキーを単独押下（Alt無し、vk=0xF2）→ 決定1/2の条件を満たし
`KANA_WAS_DOWN=true`・`KANA_CONFIRMED_TARGET=Some(VK_SPACE)`・
`KANA_DOWN_WAS_ALLOWED=true`、OSへ`VK_SPACE`のKeyDownが送出される。
(2) かなキーを押したままAltを押す。(3) かなキーを離す→OSのレイアウト
変換層がこのKeyUpを`VK_DBE_ROMAN`/`NOROMAN`として渡す→`hook.rs:1066`の
swallowガードに掛かり`return LRESULT(1)`→本ADRの挿入点（`hook.rs:1135`
以降）に到達しない→`KANA_WAS_DOWN`はtrueのまま、`VK_SPACE`のKeyUpは
永久に送出されない→OS側でスペースキーが押しっぱなしになる（BUG-100型の
stuck key）。

**決定**: `hook.rs:1013-1046`と`hook.rs:1066-1087`の両swallow分岐で、
`return LRESULT(1)`する**前**に、`KANA_CONFIRMED_TARGET`が`Some(vk)`か
つ`KANA_DOWN_WAS_ALLOWED == true`であれば、その`vk`のKeyUpを注入してから
`KANA_WAS_DOWN`/`KANA_CONFIRMED_TARGET`をクリアする（決定7-3
〈ADR-141〉と同じ`send_input_safe`直接呼び出し、フックスレッド内、
`reinject()`はメインスレッド専用契約のため使えない）。このチェックは
scan値（0x70）に紐づく既存の状態を参照するだけであり、swallowガード
自体が既存で防いでいるOSへの生イベント配送（0xF5/0xF6の素通し・
`VK_KANA`+Alt素通し）は一切変更しない——BUG-08/61/62対策そのものには
手を加えず、その手前で「かなスロットの役割代入が残した未解決の
hold-stateだけ」を掃除する。

**`swallow_alt_kana_input_method_switch=false`の場合**（premortem役B3
指摘）: このオプトアウト設定では`CACHED_SWALLOW_ALT_KANA_MODE_SWITCH`
が`false`のため、上記のswallow分岐自体が発火せず、0xF5/0xF6イベントは
（scanが0x70のまま）本ADRの挿入点まで到達する。この場合は決定1の
scanベースmatch・決定4のhold-state・決定5の（swallow分岐を経由しない）
通常のKeyUp処理パスがそのまま機能するため、追加の対応は不要——ただし
これは「素通しされた0xF5/0xF6がOSへ渡り、入力方式切替が発生しうる」
という、このオプトアウト自体が抱える既存のリスク（BUG-62追補5の
ドキュメント済みトレードオフ）とは独立である。本ADRはこのリスクを
悪化させも改善させもしない。

`is_self_injected`（`hook.rs:935`、`is_injected`とは別物——`LLKHF_INJECTED`
一般ではなくawase自身の`INJECTED_MARKER`のみを見る）はこの2つのswallow
分岐より前段にあるため、決定5が新設するKeyUp注入自体が二重評価される
ことはない。

### 決定6: 親指キー設定（`THUMB_KEY_OPTIONS`）との追加バリデーションは不要と確認した

r0はここで「かなスロットの役割代入と親指キー設定は同時に有効化できない」
という全面禁止を新設していたが、r1レビュー（architect役M3）でこれが
**ADR-141決定4（r2）が撤回した失敗パターンの再導入**であり、かつ
実コードの動作を誤認していたと判明したため撤回する。

`hook.rs`の`update_thumb`（`:1137-1156`）は`vk == config.left_thumb_vk`
という**代入後vk**基準で親指キー押下時刻を更新する。この判定は決定1の
役割代入（`hook.rs:1135`）より**後**に実行される。したがって:

- `right_thumb_vk = VK_DBE_HIRAGANA`（GUIで「ひらがな」を選択）かつ
  「変換↔かな」の役割代入を設定した場合、物理変換キーを押すと代入で
  vk=0xF2に書き換わり、`update_thumb`が正しくこれを親指キー押下として
  認識する。逆に物理かなキーを押すと代入でvk=VK_CONVERTに書き換わり、
  親指キーとしては認識されなくなる。これは「秀Caps的な入れ替え」が
  意図する挙動そのものであり、バグではない（ADR-141決定4がr2で
  「全単射により親指キーの役割は逆写像で正しく再配置される、これは
  要望そのものが意図する挙動」と結論したのと同型）。
- したがって親指キー設定と役割代入設定を同時に有効化することを禁止する
  理由はなく、追加のバリデーションは不要と判断する。

**決定**: 追加のconfig検証は行わない。ADR-141決定4のr2の教訓（全面禁止は
既定configで機能を丸ごと無効化しうる致命的な欠陥になりやすい）を踏まえ、
「代入後vk基準で全ての下流ロジック（親指キー判定・既定ホットキー判定
〈決定9参照〉）が一貫して動く」という全単射モデルの性質そのものに委ねる。

### 決定7: Shift併用時は代入自体を不発火にする（r0の「既知の制限」記述は事実誤認だったため訂正）

決定2の制約により、かなスロットへの代入はShift押下中は新規発火しない
（Shift押下中に押されたキーは、代入なしの元の挙動のまま扱われる）。

**r0の記述の誤りとその訂正（premortem役M2・architect役M5指摘）**: r0は
「Shift併用してもカタカナには切り替わらない（ひらがな相当のまま）」を
既知の制限としていたが、これは以下の2点で事実と異なっていた:

1. r0の理由付け（「代入で生成した0xF1イベントであっても
   `shift_katakana_passthrough`の条件を満たせば同じ分岐に乗ってしまう
   可能性がある」）は、決定2が合成vkを0xF2に固定している以上0xF1は
   生成されないため、決定2自体と矛盾していた。`shift_katakana_
   passthrough`（`transport.rs:104-109`）は`event.vk_code ==
   VK_DBE_KATAKANA`（0xF1）を条件にしており、この分岐に乗ることは
   構造的に不可能である。
2. より重要な事実誤認: `key_pipeline.rs:2007-2016`は、既存の（役割代入
   とは無関係な）合成`VK_DBE_HIRAGANA`送出経路が、Shift併用時に
   「synthetic Shift up」を同一バッチの先頭に前置していることを示して
   いる。これは「Shift物理押下中に0xF2を送るとIME側がShift+かな＝
   カタカナ切替と解釈してしまう」問題に対する既存の対策である。
   `reinject()`にはこの前置がない。つまりr0が想定した制限
   （「ひらがな相当のまま」）とは逆に、**対策しなければShiftを押し
   ながら代入後キーを押すと意図せずカタカナへ切り替わりうる**。

r1がこの問題を「合成ロジックにShift-up前置を追加する」のではなく
「Shift押下中は代入自体を不発火にする」（決定2）で解決したのは、
前者が`reinject()`という新しい注入経路に既存の複雑な前置ロジックを
複製することになり、ADR-141/142で繰り返し観測された「改訂作業自体が
新しいBlockerを混入させる」パターンを誘発しやすいためである。後者は
ADR-141の`engine_enabled`規律（新規押下時点でのみ判定する）にそのまま
沿う、より単純で検証しやすい解である。

**既知の制限（r1訂正版）**: Shift押下中は役割代入が発火しないため、
Shift+代入後キーはそのキー本来の（代入前の）挙動になる。例えば
「かな→スペース」の設定で、Shift+スペース位置のキー（＝物理かなキー）
を押すと、通常のShift+スペース（MS-IMEの半角/全角スペース切替）ではなく、
**代入が発火しないため物理かなキーの通常のShift併用挙動**
（`VK_DBE_KATAKANA`、`shift_katakana_passthrough`経由でカタカナ入力）
になる。逆方向（安全な3キー→かな）で例えばShift+変換キーを押した場合も、
代入は発火せず、変換キー本来のShift+変換の挙動になる。ユーザーには
設定GUI（Phase C）で「Shift併用時は代入が効かない」ことを明示する。

### 決定8: config形式・設定GUIはPhase Cへ切り出す

ADR-141/142と同様、本ADR（Phase A相当）は挿入点・判定ロジック・既存機構
との相互作用のみを決定する。具体的なconfig表現（`[[key_role]]`の`from`/
`to`にかなスロットをどう指定させるか——`VK_DBE_HIRAGANA`という名前を
そのまま使うか、`"kana"`のような物理キーエイリアスを新設するか）・
設定GUIのプリセット表現は、ADR-142の拡張として後続ADR（Phase C、本ADRの
後続として次の空き番号を採番）で決定する。

### 決定9: 代入後vkを基準に評価される下流の合流点を棚卸しする（r1新設）

premortem役M6指摘を受け、決定1〜8がカバーしない、代入後vkを見る下流
ロジックを以下のとおり棚卸しする。

1. **`vk_may_mutate_conv`（`vk.rs:187-194`、ADR-084/086 conv mode force
   policyの再発ファミリー）**: `VK_CONVERT`(0x1C)・`VK_KANA`(0x15)・
   0xF0-0xF6は`true`、`VK_NONCONVERT`(0x1D)・`VK_SPACE`は`false`。
   「変換→かな」の代入では変換キーは代入前後どちらもconv-mutating
   （0x1Cも0xF2も`true`）のため実質的な変化は無いが、「無変換→かな」
   または「スペース→かな」の代入では、その物理キーが代入前は
   非conv-mutatingだったのに代入後はconv-mutatingになるという**意味論
   の反転**が起こる。conv mode policy=force（ADR-086）がこの反転を
   正しく扱えるかは未検証であり、実装前に`state/conv_mode.rs`・
   `runtime/conv_actuation.rs`との相互作用を専用に確認する必要がある
   （`.claude/rules/fix-requires-evidence.md`の「conv mode」ファミリー
   に該当、未解決の疑問7）。
2. **既定ホットキー（`Ctrl+変換`=IME ON、`Ctrl+Shift+変換`=engine ON
   等）**: これらはVK値で判定されるため、決定3の全単射制約が保たれる
   限り、ホットキーは「代入後にそのVKを生成する物理キー」へ自動的に
   追従する（決定6と同型の論拠——2-cycleなら入れ替わった相手キーで、
   3-cycle以上でも巡回先のキーで、必ずどこかの物理キーがそのVKを
   生成し続ける）。したがって既定ホットキーの喪失は起こらない
   （全単射性による構造的保証）。ただし、これがGUI上でユーザーに
   直感的に伝わるかは別問題であり、Phase Cで文言を検討する
   （未解決の疑問3と統合）。
3. **`AppImeProfile::ImmCross`への漏洩**（premortem役m3指摘）:
   `transport.rs::plan`のF2専用分岐（InputRelay判定の直後）は、
   ImmCross専用の無条件Suppressアーム（F2分岐より後段）より**先に**
   評価される。ImmCrossプロファイルのアプリで`is_tsf_mode=false`
   （典型的なWin32/IMM32アプリはTSFではない）の場合、代入で生成した
   0xF2はF2分岐で`Allow`され、ImmCross専用アームに到達する前にOSへ
   送出されてしまう可能性がある。これは`feedback_immcross_owns_kanji`
   の設計原則（ImmCrossアプリには物理IMEキーを見せない）に反する。
   この判定はhook.rsの挿入点ではなく`transport.rs::plan`呼び出し時点
   （focusプロファイルが判明した後）でしか行えないため、`RawKeyEvent`
   に「この事象は役割代入由来か」を示す情報を追加する必要があるか
   どうか、実装方式の検討が必要（未解決の疑問8、本ADRでは未解決の
   まま残す）。

## 未解決の疑問

1. **かなスロットの`from_name`表現**: ADR-142決定B0は「`from_name`は
   `[[keymaps]]`/`[[key_remap]]`と共有するSSOTであり、key_role専用の都合で
   拡張しない」という設計だった。かなスロットは既存のどのVK名とも1:1で
   対応しない（scan 0x70という物理キー概念）ため、既存の`from_name`解決
   （platform側、`state/key_role.rs`）にどう統合するかは未決定。決定3・
   決定6が「config読み込み時に検証する」と定めている以上、この表現が
   決まらないと実装に着手できない（architect役M7指摘）。
2. **`awase-settings`の`THUMB_KEY_OPTIONS`とkey_role設定GUIの表示上の整合**:
   決定6を撤回した結果、追加のバリデーションGUIは不要になったが、
   ユーザーが両機能を組み合わせて設定した場合の挙動（決定6の説明）を
   どうGUI文言で伝えるかはPhase Cの課題として残る。
3. **かなスロットを`to`にする代入・既定ホットキーの再配置についてのユーザー
   向け説明**（決定9-2と統合）: 「変換→かな」設定時、変換キー本来のIME
   変換機能は失われるが、既定ホットキー自体は全単射により別の物理キーへ
   自動的に付いていく。この「機能の移動」をユーザーにどう伝えるか
   （GUI文言、Phase C）は未決定。
4. **MS-IME環境での決定2の有効性**: 決定2の「既知の制限」節に記載のとおり、
   `reinject()`の`wScan: 0`がMS-IME環境で実際にモードキーとして処理される
   かは未検証（既存コードのコメントは否定的な実測を示唆）。実装前に
   Windows実機でGJI・MS-IME双方の確認が必須。
5. **物理かなキーの役割代入がIME belief観測に与える影響**: 決定1の
   副作用として記載した、`classify_ime_relevance`経由のshadow belief
   観測が代入後は失われる点について、既存のbelief更新ロジック
   （`state/ime_model.rs`）への実害の有無は未検証。
6. **`kp_restore_hiragana_for_suppressed_mode_key`の各ゲートの役割代入対応
   監査**: 決定2の既知の制限2に記載した`is_configured_thumb_key`・
   `half_width_alnum_toggle_before`・`kana_mode_restore_key_down`
   （単一グローバルラッチ）が、代入元が物理かなキー以外になった場合にも
   正しく動作するかの専用監査が必要。
7. **conv mode force policy（ADR-086）との相互作用**: 決定9-1に記載した
   `vk_may_mutate_conv`の意味論反転（無変換/スペース→かな代入時）が、
   ADR-086のforce policyの前提を壊さないかの専用検証が必要。
8. **ImmCrossプロファイルへの代入後0xF2漏洩の防止方式**: 決定9-3に記載。
   `RawKeyEvent`に役割代入の出自を示すフィールドを追加するか、別の
   防止方式を取るかは未決定。

## 変更履歴

- r0（2026-09-06）: 初版起草。ADR-141決定0の除外理由の再確認、物理配線の
  追加調査（scan 0x70が唯一の物理かなキーであること）、ユーザーとの対話で
  スコープを「かなスロット↔安全な3キーのクロスファミリー入れ替え」に
  再定義した上で決定0〜8を新設。
- r1（2026-09-06）: Opus 2体（architect役・premortem役）の敵対的レビュー
  1ラウンドを反映。Blocker 3件（Alt/Win押下中の合成0xF2がBUG-61型の入力
  方式切替を誘発／上流swallowガードがかなスロットのKeyUpを消費し
  stuck keyを起こす／オプトアウト設定で非対称なDown/Upイベント列が
  同種のstuck keyを起こす）を決定2・決定4・決定5の書き直しで解消。
  THUMB_KEY_OPTIONSの事実誤認（両エージェント独立発見）を訂正。決定6
  （親指キー全面禁止）を撤回・不要と確認。決定7を全面的に書き直し
  （Shift併用時の実際の挙動がr0の記述と逆だった）。決定9（下流合流点の
  棚卸し）を新設。行番号を全面的に実測値へ修正。決定0（スコープの要約
  のみで独立した決定内容を持たなかった）を削除し、決定1に統合。
