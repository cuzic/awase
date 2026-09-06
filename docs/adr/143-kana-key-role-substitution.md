# ADR-143: かな系キーの役割代入（Kana Key Role Substitution）

## ステータス

**r4（Opus 2体の敵対的レビューを4ラウンド実施。r3でBlockerは
premortem役ゼロ・architect役1件〈BUG-10食い逃げ救済経路の消失〉に
収束、`ime_will_be_turned_on_elsewhere`を新設して対応。r5レビュー
未実施）。**

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

**r1→r2で判明した、r1の改訂自体が混入させた新Blocker**: r1決定2は
「Alt/Win/Shift押下中は代入を不発火にする」というhook.rs側のmodifier
ゲートでBUG-61リスクに対処したが、r2レビューで両エージェントが独立に
2つのBlockerを発見した。(1) このゲートは`to`方向（安全な3キー→かな）
にしか掛かっておらず、`from`方向（かな→安全な3キー）には掛からない。
`swallow_alt_kana_input_method_switch=false`のオプトアウト構成でAlt
押下中に物理変換キー（ゲートで代入不発火→`VK_CONVERT`のまま）と物理
かなキー（0xF5/0xF6が素通しされ、fromゲートが無いため代入発火→
`VK_CONVERT`）が**同時に同一vkを生成し**、ADR-141決定1-2/決定8の
「代入後の1vkを生成しうる物理キーは常に1つ」という不変条件を破壊する。
(2) r1決定4が新設した`KANA_DOWN_WAS_ALLOWED`は、書き込み元
（`PhysicalKeyDisposition::plan`、メインスレッド）と読み出し元
（decision5のswallow分岐、フックスレッド）が別スレッドであり、Downの
`plan`結果が確定する前にKeyUpがフックへ到達しうるため、原理的に
安全に実装できない。加えてこの値は本来「かなスロット自身」ではなく
「安全な3キー側の`confirmed_target`」に必要な情報であり、決定4は
発見したハザードを塞げていなかった（間違ったスロットへの実装）。

premortem役はこれを受けて、根本原因（`to`=かなの代入を生の`SendInput`
でOSへ届けようとしていること自体）に対する構造的な代替案を提示した。
r2はこれを採用し、決定2をアーキテクチャレベルで再設計した（詳細は
決定2参照）。この再設計により、上記2件のBlockerに加え、r1で個別に
対処しようとしていた複数のMajor（modifier_snapshotが挿入点に存在しない
／Altセンチネル親指キー構成でゲートが常時閉じる／ImmCrossへの漏洩／
MS-IMEでscan=0が効かない懸念）が、ゲート機構そのものの削除によって
まとめて解消された。

### レビュー指摘との対応表

r0→r1（1ラウンド目）: architect役 B1→決定5、B2→決定2、B3→決定4、
M1→行番号全体、M2→決定1、M3→決定6、M6→決定3、M7→未解決の疑問1。
premortem役 B1(Alt+合成0xF2でBUG-61)→決定2、B2(上流swallowのKeyUp消費)
→決定5、B3(オプトアウト設定での非対称Down/Up)→決定5、M1(MS-IME
scan=0無反応)→決定2、M2(Shift併用で逆にカタカナへ飛ぶ)→決定2/決定7、
M3(THUMB_KEY_OPTIONS事実誤認)→決定6、M5(IME OFF方向の喪失)→決定2、
M6(conv_mutation・既定ホットキー)→決定9(新設)。

r1→r2（2ラウンド目）: architect役 NB1(KANA_DOWN_WAS_ALLOWEDのクロス
スレッド問題)→決定2/決定4の再設計で解消、NB2(modifierゲートの非対称が
全単射を破る)→決定2の再設計で解消、NM1(modifier_snapshotが挿入点に
存在しない)→決定2の再設計でゲート自体を削除し解消、NM3(決定9-2が
ADR-141決定6の必須要件を弱めた)→決定9で復元、NM4(ADR-141決定7の3箇所
への波及)→決定4/決定5に明示、NM5(ImmCross漏洩は既存挙動でスコープ
クリープ)→決定9で整理。
premortem役 R1-B1(決定2/決定7の方向不一致)→決定2の再設計で解消、
R1-B2(KANA_DOWN_WAS_ALLOWEDが取得不能)→決定2の再設計で解消、R1-B3
(ガードが間違ったスロット)→決定2の再設計で解消、R1-M1(Altセンチネル
親指キーでゲートが常時閉じる)→決定2の再設計で解消、R1-M3(決定5の
クリーンアップ条件が広すぎる)→決定5で修正、R1-M4(ADR-141決定7の3箇所
への波及)→決定4/決定5に明示、R1-M5(ホットキーのShift分裂)→決定2の
再設計で解消。

両エージェントが独立に到達した一致点（信頼度が高いと判断）: (a)
`THUMB_KEY_OPTIONS`の実際の3値は`VK_KANA`/`VK_DBE_KATAKANA`/
`VK_DBE_HIRAGANA`であり0xF0は含まれない、(b) 上流のAlt+かなswallow
ガードがかなスロットの役割代入と衝突しうる、(c) r1のmodifierゲートは
全単射を破る、(d) `KANA_DOWN_WAS_ALLOWED`は原理的に実装不能、という
4点。特に(c)(d)は独立に同一の結論へ到達しており、根本原因（生の
`SendInput`でかな相当を装う設計）そのものを疑う根拠になった。

**r2→r3で判明した、r2のアーキテクチャ転換の安全性論拠の誤り**: r2は
「Suppressされたイベントを既存のIME actuation経路へ渡す」という設計に
転換したが、その安全性論拠自体に事実誤認があった。architect役が独立に
発見: (1) NB3——「actuation経路が`ime_mode_key_injection_blocked_by_
modifier()`を経由するので安全」は誤りで、この関数を呼ぶのは半角英数
トグル復元の2箇所のみ、actuation本経路には存在しない。(2)
NB4——「Suppressされた結果としてactuation経路へ渡す」は因果順序が逆で、
実際にはactuationは`plan()`の判定より**前**に既に発火している（詳細は
決定2参照）。加えて両エージェントが独立に、`transport.rs::plan`の新
条件そのものに実装可能な状態への修正が必要な欠陥（NB5/R2-B1: 他プロセス
relayの0xF2を巻き込む、R2-B2: 機能未使用ユーザーへの影響）を発見し、
premortem役は`kp_restore_hiragana_for_suppressed_mode_key`の必要十分
条件が壊れる問題（R2-B3）も発見した。r3はこれらを修正し、決定2の安全性
論拠を「送出VKがDBE系でないから安全」という検証可能な形へ訂正した。

### レビュー指摘との対応表（r2→r3、3ラウンド目）

architect役: NB3(actuation安全性論拠の事実誤認)→決定2で訂正、
NB4(因果順序が逆・二重actuationの懸念)→決定2で訂正・未解決の疑問9を
削除、NB5(injected/機能未使用ユーザーへの影響)→決定2の条件に
`!event.injected`/`kana_role_active`追加、NM6(InputRelayでDown/Up非対称)
→決定4に既知の狭い制限として記録、NM7(オプトアウト無効化範囲が全単射を
壊す)→決定5でルール集合全体の無効化に訂正、NM8(actuation合流点が未確定)
→決定2で解決（NB4と同時に解消）、NM9(BUG-10食い逃げへの言及なし)→決定2
に補償の論証を追加。
premortem役: R2-B1(injected relay巻き込み)→決定2で`!event.injected`
追加、R2-B2(機能未設定ユーザーへの影響)→決定2で`kana_role_active`追加、
R2-B3(`kp_restore_hiragana_for_suppressed_mode_key`の必要十分条件破壊)
→決定2でscan_code除外条件を追加、R2-M1(Alt安全性論拠の訂正)→決定2で
送出VK根拠へ訂正、R2-M2(charset軸復帰も再現しない)→決定2の既知の制限に
追加、R2-M3(ADR-141本体への申し送り)→決定4に明記、R2-M4(押下中の設定
変更)→決定5に規律を追加、R2-M5(確定キー反転の実害)→決定9で既に必須
要件化済み。

両エージェントが3ラウンド共通で到達した一致点: r2のアーキテクチャ転換
（生のSendInputをやめる）の方向性自体は正しいが、その安全性論拠は
「既存の仕組みを経由するから安全」という**経路の共有**ではなく、
「送出される値そのものが危険な組み合わせに該当しないから安全」という
**値の性質**に基づかせるべきである、という教訓。前者は経路の実装が
変わると静かに壊れる（NB3がまさにその例）。

### レビュー指摘との対応表（r3→r4、4ラウンド目）

premortem役はr3にBlockerゼロと判定（r2の3件はすべて解消）。architect役
は新規Blocker1件（NB6）を発見した——2体のレビュアーの判定が分かれた
唯一のラウンドであり、NB6は具体的なコード引用（`kp_stage_shadow_ime_
toggle`の3ゲート）を伴う検証可能な指摘だったため採用した。

architect役: NB6(actuationのno-op分岐でBUG-10救済経路が消失)→決定2に
`ime_will_be_turned_on_elsewhere`を新設、NM10(決定6のthumb_key=ひらがな
構成でdelegate_ownedと重なる)→上記フラグのdelegate分岐でカバー、
Minor1(判別子の健全性証明)→決定2に証明を追記、Minor2(kana_role_activeと
決定5オプトアウトの連動)→決定2にSSOT要件を追記、Minor3(suppress_reason
の二重管理)→共有ヘルパへ一本化、Minor4(InputRelayのKeyUpがinertである
保証なし)→決定4に明記、Nit1(DbeModeKeyContextの2ガードが無意味化)→
決定7に追記。
premortem役: R3-M1(kana_role_activeの押下中反転)→決定2でKeyUp側から
`kana_role_active`を除外、R3-M2(scan==SCAN_KANAが未検証の前提)→決定5で
`!is_injected`のみに簡素化、R3-M3(kana_role_activeのSSOT未指定)→決定2に
SSOT要件を追記（architect役Minor2と同一結論、独立到達）、R3-m1〜m4→
決定2/決定8に反映。

両エージェントが4ラウンド共通で到達した教訓: 新しく依拠する既存経路の
安全性を論じる際は、その経路の「入口が同じであること」ではなく「経路の
途中に分岐点が無いこと」まで確認する必要がある。NB3（r2、存在しない
ガードを根拠にした）とNB6（r3、存在するが見落とした分岐〈no-op〉を
根拠から漏らした）は同型の失敗である。

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
    `VK_DBE_DBCSCHAR`(0xF0/0xF1/0xF3/0xF4)を割り当てること（決定2の
    r2再設計により、`to`=かなの意味論は「`VK_DBE_HIRAGANA`固定・常に
    Suppress・awase自身がIME ON相当をactuateする」の1通りに一本化された
    ため、他の`VK_DBE_*`亜種をGUIの選択肢に出す余地自体が無い）。
  - IME OFF方向の代入（決定2の既知の制限1、「かなスロット＝IME ON指示」
    という意味論に限定するため再現しない）。
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

### 決定2（r3で機構を訂正、r2の見出しは実態と異なっていたため訂正）: toがかなスロットになる場合、OSへの二重配送だけを止める——actuationは既に既存経路が行っている

**r2→r3で判明した最重要事実（architect役NB4指摘）**: r2は「Suppress
されたイベントをIME ON要求としてactuation経路へ**新たに渡す**」という
書き方をしていたが、これは因果順序が逆だった。実際の実行順序
（`runtime/key_pipeline.rs:270`の`kp_stage_shadow_ime_toggle`呼び出しが
`:392`の`PhysicalKeyDisposition::plan`呼び出しより**前**——
`shadow_toggled`が`plan()`の引数であることからも明らか）では、
役割代入で書き換えられたvk（`VK_DBE_HIRAGANA`）は`build_raw_key_event`
の`classify_ime_relevance(vk)`（代入後vk基準）によって**既に**
`ImeKeyKind::Activate`→`ShadowImeAction::TurnOn`として分類され、
`kp_stage_shadow_ime_toggle`→`ime_controller::apply`のactuationが
`plan()`の判定を待たずに**既に発火している**。これはADR-141の
「vkを書き換えれば以後の全パイプラインがそれだけを見る」という設計
そのものの帰結であり、決定1がこれを一般化した時点で既に成立していた
（本ADRが新規に組み込む必要は無かった）。

したがって決定2が実際に決めるべきことは1点のみである: **役割代入由来の
`VK_DBE_HIRAGANA`が、既に発火済みのactuationに加えて、`reinject()`
経由でOSへも重複して送出されるのを止める**こと。r2の「actuation経路へ
渡す」という表現は、r1のBlocker解消の説明として不正確であり、未解決の
疑問9として残していた「実装方式」も的自体が存在しなかった（削除）。

**新しい設計（r4でさらに条件を訂正——architect役NB6・NM10、premortem役
R3-M1/R3-M2/R3-M3指摘への対応）**:

`transport.rs::plan`のF2専用分岐を拡張し、以下の条件を満たす場合にのみ、
`is_tsf_mode`/`f2_warmup_owned`の値に関わらず`Suppress`を返す。**KeyDown
とKeyUpで条件が異なる**（r4で分離、後述）:

```
// KeyDown（新規押下時点でのみ判定、ADR-141の規律と同一）
event.vk_code == VK_DBE_HIRAGANA
  && !event.injected                  // 他プロセスrelayの0xF2を巻き込まない
  && kana_role_active                 // かなスロットが関与する役割代入が
                                       // 現在有効な場合のみ適用する
  && event.scan_code != SCAN_KANA     // 役割代入由来であることの判定
                                       // （決定1のscan不変性を再利用）
  && ime_will_be_turned_on_elsewhere  // r4で追加、NB6/NM10対応（後述）

// KeyUp（r4でkana_role_activeを除外、premortem役R3-M1対応）
event.vk_code == VK_DBE_HIRAGANA
  && !event.injected
  && event.scan_code != SCAN_KANA
  && ime_will_be_turned_on_elsewhere
```

- **`!event.injected`が無いと**（r2の欠陥）: `transport.rs`の評価順序は
  InputRelay早期return（`:260`）→F2分岐（`:276`）→injected早期return
  （`:302`以降）であり、F2分岐はinjectedチェックより**前**にある。
  他プロセス（Mouse Without Borders等）がSendInputでrelayした0xF2は
  `wScan: 0`で届くため`scan_code != SCAN_KANA`が真になり、無条件で
  Suppressされてしまう。injectedイベントはBUG-14ガード
  （`kp_stage_shadow_ime_toggle`）により`shadow_toggled`へ昇格しない
  ため awase自身もactuateしない——結果、ADR-119（issue #136）決定1が
  防いだ「解釈しない入力を消費する」＝リモート側のかなキーが完全に
  無反応になる「二重の空振り」が再導入される。
- **`kana_role_active`が無いと**（r2の欠陥）: この条件は`plan()`の
  引数からのみ決まる必要がある純粋関数の制約に従い、`DbeModeKeyContext`
  （`transport.rs:63-69`の`is_configured_thumb_key`と同型の追加）の
  フィールドとして追加する。「かなスロットが関与する役割代入ルールが
  現在有効か」を示す`AtomicBool`で、`CACHED_SWALLOW_ALT_KANA_MODE_
  SWITCH`（`hook.rs:457`、setterは`:567`）と同型に、config ロード時と
  `app/mod.rs::reload_config()`の両方で更新する（ADR-142決定B1が定めた
  「起動時とreload時の両方から呼ぶ」検証と同じ配線に相乗りする）。
  `From<DbeModeKeyPolicy>`実装の既定値を`false`にすれば既存の回帰
  テストは無改修で通る。この条件が無いと、`[[key_role]]`を1つも設定
  していないユーザーにもこのSuppressが適用され、スコープ節の
  「代入なしの場合は既存挙動を一切変更しない」という宣言に反し、
  BUG-10（食い逃げ、後述）の回帰面を機能未使用のユーザーにまで広げて
  しまう。

  **SSOTの要件（r4追加、premortem役R3-M3・architect役Minor2指摘への
  対応）**: `kana_role_active`は独立した経路で計算してはならない。
  hook.rs側の役割代入テーブル（決定1のfrom判定・決定3の全単射検証済み
  ルール集合）と`kana_role_active`が別々に計算されると、両者が乖離
  した場合（hookは代入する側・`kana_role_active`はfalse側、という
  最悪の組み合わせ）に「代入後の0xF2が`Allow`されて`reinject()`で
  OSへ出る」という、本ADRが解消したはずの生の0xF2直接送出が復活する
  （`awase-settings::is_muhenkan_thumb_key`のdocが警告するissue #99型
  の二重管理と同型のリスク）。`kana_role_active`は、ADR-142決定B1が
  新設する`state/key_role.rs`の解決済みテーブル（hook.rs側が代入判定に
  使うのと同一のSSOT）から**導出する**ことを要件とし、独立したbool値
  として別経路で計算しない。決定5のオプトアウト無効化（`swallow_alt_
  kana_input_method_switch=false`時のルール集合全体無効化）も、この
  同じSSOTを通じて`kana_role_active`に反映される——両者が異なる
  タイミングで更新される二重管理を作らない。
- **KeyUpで`kana_role_active`を要求しない理由（r4追加、premortem役
  R3-M1指摘への対応）**: `kana_role_active`はGUI「詳細設定」からの
  reloadで変更されうる（`hook.rs:567`）。物理キーを押している最中に
  reloadが走ると、Down時点では真だった`kana_role_active`がUp時点では
  偽になりうる。この場合、Up側が偽でフォールスルーすると
  `is_tsf_mode && f2_warmup_owned`次第で`Allow`となり、対応するDownを
  持たない0xF2のKeyUpがOSへ送出される（決定4のInputRelayケースと同型の
  問題が、フォーカス遷移なしで起こる）。この経路は、hook.rs側の代入
  テーブル自体はADR-141の`confirmed_target`規律（Down時点の確定値を
  KeyUpまで保持する）に従っているため、Up時点でも`event.vk_code ==
  VK_DBE_HIRAGANA`かつ`event.scan_code != SCAN_KANA`は Down と一貫して
  成立する——`kana_role_active`だけを再評価対象から外せば、この一貫性
  だけでUpの判定が閉じる。
- **`ime_will_be_turned_on_elsewhere`（r4で新設、NB6/NM10対応）**:
  詳細は下記「BUG-10の補償論証」参照。
- 上記条件が偽の場合は、既存の`is_tsf_mode && f2_warmup_owned`による
  判定へフォールスルーする（現行と完全に同一）。この機能を使わない
  ユーザーの`plan()`の挙動は、この変更前後でビット単位で同一になる。

**Alt押下時の安全性の論拠を訂正（r3、architect役NB3・premortem役R2-M1
指摘への対応）**: r2は「actuation経路が`ime_mode_key_injection_
blocked_by_modifier()`を経由するため安全」と書いたが、これは事実誤認
だった。同関数の呼び出し箇所はリポジトリ全体で3つ（`hook.rs:296`定義、
`output/mod.rs:1201`のGJI半角英数トグル復元、`key_pipeline.rs:2004`の
MS-IME半角英数トグル復元）のみで、いずれも**半角英数トグルの復元経路**
であり、`ime_controller::apply`のactuation本経路（`ime.rs::send_ime_
mode_key`）には存在しない。actuation本経路が実際に持つガードは
`win_key_held()`のみで、Alt押下は意図的に対象外（`ime.rs:290`
「ALTを解放するとALT+TABスイッチャーが確定してしまうため、ALTは
解放しない」）。

**正しい安全性の論拠**: `GjiDirectStrategy`/`MsImeDirectStrategy`が
IME ON操作として実際に送出するVKは`VK_IME_ON`(0x16)、
`KanjiToggleStrategy`は`VK_KANJI`(0x19)であり、**いずれも`VK_DBE_*`
一族ではない**（MS-IMEのON操作は2026-08-06・BUG-50対応で
`VK_DBE_HIRAGANA`から`VK_IME_ON`単発へ変更済み、`ime_controller.rs:
190-203`）。BUG-61が問題にしているのはAlt+`VK_DBE_*`一族という特定の
組み合わせがOSレベルの入力方式切替ショートカットとして解釈されることで
あり、`VK_IME_ON`/`VK_KANJI`はこのショートカットの対象ではないため、
Alt押下中にactuationが発火してもBUG-61のリスクには該当しない。

**この論拠が成立しなくなる条件（重要な制約として明記）**: 将来
`key_sequence_policy`の送出VK選択がIME ON操作用に`VK_DBE_*`一族へ
戻される変更が入った場合、この安全性の論拠は崩れる。そのような変更を
行う際は本ADRの前提が崩れることを明記し、Alt押下時のガードを別途
追加する必要がある。

**このアーキテクチャ転換で解消される指摘**（r1・r2で個別に対処しようと
していずれもBlocker/Majorを生んだもの）:

| 指摘 | 解消理由 |
| --- | --- |
| NB2/R1-B1（modifierゲートの非対称が全単射を破る） | hook.rs側にゲートを置かないため、この方向の代入は常にvk書き換えのみ・OSへは何も送らない。修飾キー押下中に2つの物理キーが同一vkを生成する状態は構造的に起こらない |
| NB1/R1-B2/R1-B3（`KANA_DOWN_WAS_ALLOWED`のクロススレッド不整合・誤スロット） | 「安全な3キー→かな」方向のDownは`plan()`の判定で（`kana_role_active`が真の間）常にSuppressされる。「Downが実際にAllowされたか」を動的に記録・スレッド越しに参照する必要が無い（決定4参照） |
| NM1/R1-M1（`alt_key_held()`がAltセンチネル親指キー構成で常時true）・NM1/R1-M2（`modifier_snapshot.shift`が挿入点に存在しない） | hook.rs側のゲートが無いため発生しない |
| NB3/R2-M1（Alt押下時のactuation安全性の論拠が誤り） | 「経路を共有するから安全」ではなく「送出VKがDBE系でないから安全」という、検証可能な論拠へ訂正した（上記参照） |
| NB4/NM8（actuationへの新規ディスパッチが未確定・二重actuationの懸念） | actuationは既存の`kp_stage_shadow_ime_toggle`が`plan()`より前に既に発火させている。新規ディスパッチは不要（未解決の疑問9を削除） |
| NB5/R2-B1（他プロセスrelayの0xF2を巻き込みADR-119を回帰） | `!event.injected`を追加 |
| R2-B2（機能未使用ユーザーへの回帰面） | `kana_role_active`を追加 |
| NM5/R1-M6・決定9-3（ImmCrossへの0xF2漏洩） | 役割代入由来の0xF2は常にSuppressされるため、profileに関わらずOSへ届かない |
| r0-M2（Shift併用で意図せずカタカナへ切り替わる） | OSへ0xF2を送らないため、Shift+0xF2の解釈という事象自体が起きない（決定7参照） |
| R1-M5（既定ホットキーのShift分裂） | hook.rs側のゲートが無いため発生しない |

**代償・確認が必要な点（決定として明記が必要な残課題）**:
1. **IME OFF方向・charset軸（カタカナ/半角英数→ひらがな）の復帰は
   再現しない**（**r3で範囲を拡大、premortem役R2-M2/R2-B3指摘**）:
   物理かなキーは、IMEが既にひらがな状態にある時に押されると実機的に
   「かな入力を終了する」効果（内部的には0xF0生成）を持つことがある。
   加えて、決定2の既知の制限3（`kp_restore_hiragana_for_suppressed_
   mode_key`の除外）により、代入先キーは**open軸（IME ON）のみ**を
   操作し、`VK_IME_ON`はcharset軸（カタカナ/半角英数からひらがなへの
   復帰）には一切触れない（`ime_controller.rs:224-228`）。これは
   ADR-100決定2以降の「eager warmupがopen軸のみ」という片肺化
   （ADR-137 M-6の真因）と同型の制限であり、意図的に選択したトレード
   オフである。将来charset軸まで再現したい場合は、GJI/MS-IMEでexit
   実装を共有しない（BUG-25/ADR-107）という制約を満たす専用経路を
   新たに設計する必要がある（`ShadowImeEffect::Toggle`相当への拡張は
   この専用経路の設計を待つ）。
2. **BUG-10（食い逃げ）への言及と補償の論証（r3で追加、r4で全面訂正
   ——architect役NM9・NB6・NM10指摘への対応）**: `transport.rs`のF2分岐
   docは、Allowの2つの分岐（MsImeStrategy・非TSF mode）の理由として
   「ここで消すと物理ひらがなキーが食い逃げされ、intent/EngineだけON
   で実IMEがOFFのまま乖離する（BUG-10、2026-07-06実機）」と明記して
   いる。

   **r3の誤り（NB6）**: r3は「`kp_stage_shadow_ime_toggle`によるactuation
   が`plan()`より前に既に発火しているため、乖離の前提自体が存在しない」
   と結論していたが、これは`kp_stage_shadow_ime_toggle`
   （`key_pipeline.rs:1008-1160`）が実際には3段のゲートを持つことを
   見落としていた:
   - **ゲート(1) `is_japanese_ime()`**（`:1078-1084`）: これは自己解決
     する——`should_upgrade_is_japanese_ime`（`:1039`）が、非注入の
     0xF0-0xF4受信で即座にこのbeliefをtrueへ昇格させるため、役割代入
     由来の0xF2自身がこのゲートを開ける。問題なし。
   - **ゲート(2) `delegate_owned`**（`:1074-1075`、`:1121-1128`）:
     「親指キー×delegate armed×belief ON」の場合、belief書き込み/
     actuationそのものをスキップする（delegateが別途open軸を担う設計
     のため）。
   - **ゲート(3) no-op分岐**（`:1158-`）: `effective_open() == new_val`
     （すなわちbeliefが既に目的の状態と一致している）場合、
     apply-ime/dispatch-imeに到達しない。**BUG-10が問題にしている
     「intent/EngineだけONで実IMEがOFFのまま乖離した」状態は、まさに
     `effective_open()`が既にtrueを指している状態であり、この乖離が
     存在する場合にこそゲート(3)がactuationを止める**。r3はここに
     物理配送のSuppressを重ねたため、実IMEをONへ戻す経路（従来の
     Allowアームが担っていた「MS-IMEはVK_DBE_HIRAGANAをネイティブ
     処理してIME ONにするため素通しが正しい」という救済）が塞がれて
     いた。

   **決定（r4）**: `plan()`の条件に`ime_will_be_turned_on_elsewhere`
   （上記条件ブロック参照）を追加する。これは「かなスロットへの代入を
   Suppressしても、既存の仕組みのどれかが実際にIMEをONにする」ことを
   表す値で、以下のように定義する:
   ```
   ime_will_be_turned_on_elsewhere =
       if delegate_owned {
           hiragana_delegate_to_open_axis_armed  // NM10対応、後述
       } else {
           !effective_open()  // ゲート(3)の条件そのもの
                               // ＝beliefが「まだ閉じている」場合のみtrue
       }
   ```
   `delegate_owned`・`effective_open()`はいずれも`kp_stage_shadow_ime_
   toggle`と同じ呼び出し元（`key_pipeline.rs:270`付近、`plan()`より前）
   で評価可能な値であり、`shadow_toggled`と同様に`plan()`への追加の
   引数として（同一スレッド・同一イベント処理内で）渡せる——決定4の
   `KANA_DOWN_WAS_ALLOWED`がクロススレッドで破綻したのとは異なり、
   これは同一呼び出し内の値の受け渡しであり新たなスレッド安全性の
   懸念を持ち込まない。この値が偽（＝どの既存機構もIMEをONにしない）
   の場合、Suppressの4条件目を満たさずフォールスルーし、既存の
   `is_tsf_mode && f2_warmup_owned`判定（Allowを含みうる）に委ねる
   ——BUG-10が守っていた「食い逃げ」防止の救済経路をそのまま保持する。

   **narrow edge（既知の限定的な制限として受容）**: `ime_will_be_
   turned_on_elsewhere`は物理キーのDown/Up双方で個別に（新鮮に）評価
   されるため、この値が同一の物理キー押下の最中にbelief変化（他の
   要因によるIME状態変化）で反転する可能性は理論上残る。この窓は
   「1回の打鍵の最中に別要因でIME状態が変わる」という極めて狭い条件を
   要するため、決定4のInputRelay既知の制限と同型に、解決を試みない
   既知の限定的なリスクとして記録するに留める。
3. **`kp_restore_hiragana_for_suppressed_mode_key`（BUG-116決定2、
   `key_pipeline.rs:81-201`）を役割代入由来のイベントから除外する**
   （**r3で方針転換、premortem役R2-B3指摘への対応**）: r2は「この関数が
   役割代入由来でも発火するようになる」ことを既知の制限として記載する
   に留めていたが、これは同関数のコメント（`:104-108`）が明記する
   「`physical == Suppress`かつ0xF2は`plan()`のF2分岐でのみ成立し、
   その条件は`is_tsf_mode && f2_warmup_owned`そのもの。したがって
   `physical == Suppress`であることがGJI戦略としてSuppressされたことの
   **必要十分条件**である」という同値性を、決定2のr2設計（Suppress条件
   が増える）が壊す。GJI戦略でなくとも役割代入由来のSuppressで
   `physical == Suppress`が成立するため、MS-IME環境でもこの関数が発火し
   `send_gji_half_width_alnum_toggle(Exit, ..)`という**GJI専用の
   scan付きVK_DBE_HIRAGANA注入**がMS-IMEセッションへ誤発火する
   （BUG-25/ADR-107が明記する「GJI/MS-IMEでexit実装を共有しない」原則
   への違反、かつBUG-15追補7のかなロックトグルハザードの当事者）。

   **決定**: 同関数の発火条件に`event.scan_code == SCAN_KANA`を追加し、
   役割代入由来のイベント（`scan_code != SCAN_KANA`）をこの関数の対象外
   にする。これにより「`physical == Suppress`はGJI戦略としてSuppress
   されたことの必要十分条件」という同値性は、物理かなキーについてのみ
   の主張として回復する。同関数のコメント（`:104-108`）に、この前提が
   決定2の追加により変わったこと・役割代入由来のイベントは
   `scan_code != SCAN_KANA`で除外済みであることを追記する（外した
   `active_ime_kind`チェックの根拠として使われている同値性が変わった
   ことを、次にこのコードを触る人のために残す）。

   **置き場所の指定（r4追加、premortem役R3-m2指摘）**: この除外条件
   （`event.scan_code == SCAN_KANA`）は、同関数の**最初の早期return**
   として、`:90`の`if event.vk_code != VK_DBE_HIRAGANA { return; }`と
   同じ位置に置く。`:93-103`のKeyUp早期return（`kana_mode_restore_
   key_down`ラッチを解除する分岐）より**前**に置く必要がある——後段に
   置くと、代入先キー（scan 0x79等）のKeyUpが物理かなキーの立てた
   このラッチを誤って解除し、次のauto-repeat KeyDownでscan付き0xF2
   注入が重複発火しうる。全単射により「変換→かな」が有効な間は物理
   かなキー自身が0xF2を出さない（両者が同時に存在しない）ため実害は
   ほぼ無い見込みだが、位置を明記しておく。

   **auto-repeat時の扱い（r4追加、premortem役R3-m3指摘）**: 代入先
   キーを押しっぱなしにするとauto-repeat KeyDownのたびにshadow-toggle
   →actuation判定が走る（物理かなキーでも同じベースラインであり新規の
   劣化ではないが、物理かなキー側には`kana_mode_restore_key_down`
   ラッチによる重複防止があるのに対し、代入先キーは本項の除外により
   この関数の対象外になるため重複防止の有無が非対称になる）。
   `kp_stage_shadow_ime_toggle`側の`already_matched`判定により実質
   no-opになる想定であり、Phase A実装時の実機確認observationの1つと
   する。
4. **`kp_restore_hiragana_for_suppressed_mode_key`の各ゲートの監査
   優先度が上がる**（未解決の疑問6）: 決定2により「安全な3キー→かな」
   方向のDownは（`kana_role_active`が真の間）常にSuppressされる。上記3
   の除外により当該イベント自体はこの関数の対象外になるが、
   `is_configured_thumb_key`・`half_width_alnum_toggle_before`・
   `kana_mode_restore_key_down`（単一グローバルラッチ）・
   `conv_mutation_allowed`・`is_composition_warm()`複合条件・
   `read_kana_lock()`によるABORT（いずれも`key_pipeline.rs:81-201`の
   範囲内）が、除外条件自体（`scan_code`比較）を正しく実装できているかの
   監査は引き続き必要。
5. **`to`に`VK_DBE_ROMAN`/`NOROMAN`(0xF5/0xF6)・その他の`VK_DBE_*`亜種を
   割り当てることは引き続き禁止する**: `to`=かなスロットの意味は
   「`VK_DBE_HIRAGANA`固定・条件を満たせば常にSuppress・OSへの二重配送を
   止めるだけで実際のactuationは既存経路が行う」に一本化されており、
   他の`VK_DBE_*`値を`to`として選択させる余地はそもそも存在しない。
6. **Phase Aの受け入れ条件（r3追加、premortem役R2-m4指摘）**: r2で
   残していた「実装方式」の疑問（旧・未解決の疑問9）は前提の誤りにより
   削除したが（決定2冒頭参照）、「actuationが既に発火している」という
   主張はコードの静的な読解に基づく。実装完了後、代入先キーの押下で
   実際にGJI・MS-IME双方の実IMEがONになることを実機で確認することを、
   Phase A実装の受け入れ条件とする（**r4訂正、premortem役R3-n2指摘**:
   削除済みの疑問番号への参照だったため表現を修正）。
7. **`suppress_reason`のラベル訂正が必要（r3追加、architect役Minor1
   指摘。r4で判定方法を訂正、architect役Minor3・premortem役R3-m1指摘
   への対応）**: `PhysicalKeyDisposition::suppress_reason`
   （`transport.rs:32-45`）は`event.vk_code == VK_DBE_HIRAGANA`の場合に
   理由ラベルを常に`"tsf-f2"`とする。役割代入由来のSuppress（本決定が
   新設した条件）もこのラベルで記録されると、BUG-90調査が前提とする
   「journalの`KeyInput.decision`（意味論的判断）と`suppress_reason`
   （実際の配送判断）を突き合わせる」という目的にとって、役割代入起因
   の配送問題と物理かなキー起因のGJI warmup契約を区別できなくなる。

   **r4での訂正**: r3は`event.scan_code != SCAN_KANA`のみで判定を分岐
   させるとしていたが、`suppress_reason`は`event`と`profile`のみを
   引数に取り`injected`・`kana_role_active`・`ime_will_be_turned_on_
   elsewhere`を受け取れないため、この1条件だけで再判定すると
   `transport.rs:22-27`のdocが警告する「2箇所が別々に判定ロジックを
   持って乖離する」状態そのものを作る（機能OFFのユーザーでscanが
   0x70以外の0xF2が既存条件でSuppressされた場合に`"kana-role"`と
   誤ラベルされる等）。決定として、Suppress判定の全条件を評価する
   小さなヘルパ関数（例:`fn is_kana_role_suppress(event, dbe) -> bool`）
   を新設し、`plan()`と`suppress_reason`の両方がこのヘルパを呼ぶ形に
   一本化する（`DbeModeKeyContext`を`suppress_reason`にも渡せるよう
   シグネチャを拡張する）。
8. **判別子`event.scan_code != SCAN_KANA`の健全性の証明（r4追加、
   architect役Minor1指摘）**: 本決定は「`scan_code != SCAN_KANA`ならば
   役割代入由来である」ことを前提にしているが、この健全性を明示的に
   証明する。挿入点でvkを書き換える機構はAlt impersonationと役割代入の
   2つのみである。Alt impersonationの発動フラグは`resolve_thumb_key`
   （`alt_impersonation.rs:38-45`）が`"Left Alt"`/`"Right Alt"`にのみ
   `true`を返し、その2つはそれぞれ`VK_NONCONVERT`/`VK_CONVERT`に解決
   される（`src/config.rs:214-220`が、これを独立したチェックボックス
   ではなく`left_thumb_key`/`right_thumb_key`の選択肢に統合すること
   で、値が一箇所だけに存在し設定GUIの表示条件と実際の有効状態がズレる
   余地を無くす設計だと明記している。`bootstrap.rs:200-210`→
   `hook::set_alt_impersonation_enabled`〈`:536`〉→
   `CACHED_LEFT/RIGHT_ALT_IMPERSONATION_ENABLED`〈`hook.rs:436-437`〉
   という単一経路で導出される）。したがってAlt impersonationが0xF2を
   生成することは構造的に不可能であり、`scan_code != SCAN_KANA`は
   役割代入由来であることの健全な判別子である。将来Alt センチネルの
   解決先を変える変更が入った場合、この前提が崩れることに注意
   （「送出VKがDBE系でない」という別の前提と同様に扱う）。

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

### 決定4（r2で簡素化）: hold-state・KeymapLatchをかなスロット専用ペアとして追加する（`from`=かな方向のみ）

ADR-141決定2は、安全な3キーそれぞれに固定長・名前付きの
`{KEY}_WAS_DOWN: bool`/`{KEY}_CONFIRMED_TARGET: Option<VkCode>`を持つ
（3キー×2=6変数、インデックスは代入前の物理vk）。かなスロット用に、同型の
`SCAN_KANA_WAS_DOWN`/`SCAN_KANA_CONFIRMED_TARGET`を専用に追加する
（**r2で改名**: Nit1指摘によりscan基準で識別することを変数名に反映した。
r1の`KANA_WAS_DOWN`/`KANA_CONFIRMED_TARGET`から改称）。

安全な3キー用のペアとの決定的な違い: 安全な3キーの`{KEY}_WAS_DOWN`は
「代入前の物理vk」でインデックスされる（vkが物理キーと1:1で安定して
いるため）。`SCAN_KANA_WAS_DOWN`は、vkではなく**scan値が0x70であること**
をトリガーにする（決定1と同じ理由）。同じ`decide_role_substitution`
関数（ADR-141決定2で新設予定の判定関数）は共通で流用できるが、呼び出し
側の「fromをどう識別するか」の判定ロジックだけがキー種別（安全な3キー
vs かなスロット）で分岐する非対称設計になることを明記する。

**r2で`KANA_DOWN_WAS_ALLOWED`を撤回した理由**: r1はここに第3の
フィールド`KANA_DOWN_WAS_ALLOWED: bool`を追加していたが、r2レビューで
これが(a)書き込み元（`plan()`、メインスレッド）と読み出し元（決定5の
swallow分岐、フックスレッド）が異なりクロススレッドで安全に実装できない
こと、(b)そもそも必要な情報が「かなスロット自身」ではなく「安全な
3キー側の`{KEY}_CONFIRMED_TARGET`」に付くべきものだったこと、の2点で
根本的に破綻していると判明した。決定2のアーキテクチャ転換
（「安全な3キー→かな」方向のDownは`transport.rs::plan`で**常に**
Suppressされる、実行時に変わらない静的な事実になった）により、この
フィールドが解決しようとしていた問題自体が消滅したため撤回する。

**「安全な3キー→かな」方向にはKeyUp注入が原則不要になった**: 決定2に
より、この方向のDownは（`kana_role_active`が真である間）常に
Suppressされる（OSは一度もDownを見ない）。`transport.rs::plan`のF2分岐
は静的な条件のみで判定されるため、対応するUpも同じ条件で必ずSuppress
される。したがってADR-141決定7のKeyUp注入パターンは、
`{KEY}_CONFIRMED_TARGET == Some(VK_DBE_HIRAGANA)`の場合には**原則
発火させない**（対象vkが`VK_DBE_HIRAGANA`の場合のみの例外、他の安全な
3キー同士の代入では従来どおりKeyUp注入が必要）。**この例外はADR-141
決定7自身への申し送りとしてADR-141本体にも参照ポインタを追加する**
（**r3追加、premortem役R2-M3指摘**: ADR-142がr5で行ったのと同じ手当て
——ADR-143だけに書くと、ADR-141を単体で読む実装者が無条件でKeyUp注入を
実装してしまう）。

**例外の例外（r3追加、architect役NM6指摘）**: 上記の「静的に必ず同じ
結果になる」という主張は、`profile == AppImeProfile::InputRelay`の
場合に成立しない。`transport.rs:260`のInputRelay早期returnはF2分岐
より**前**にあり、無条件で`Allow`を返す。`profile`は
`key_pipeline.rs:385`でイベントごとに読み直されるため、Down押下時は
InputRelay以外のプロファイルで（F2分岐によりSuppress）、Up時に
フォーカスがInputRelayウィンドウ（Mouse Without Borders等の中継窓）へ
移っていれば（`Allow`）、対応するDownを持たない0xF2のKeyUpが`reinject()`
でOSへ送出されうる。この経路は「押下中にフォーカスがInputRelayウィンドウ
へ移る」という狭い条件を要するため、ADR-141決定4末尾がAltセンチネル
構成の既存衝突に対して取った立場（悪化させないが解消もしない）と同型に
整理し、本ADRでは解決を試みない既知の狭い制限として記録するに留める
（実害が確認された場合は別途対応する）。**r4追加（architect役Minor4
指摘）**: この制限の内容として、送出される0xF2のKeyUpが無害
（inert）である保証はADR本文にも実機記録にも無い点を明記する——この
vk種別は`transport.rs`の無条件Suppressガードが「素通しすると実IME
（MS-IME）がWindows標準仕様どおり能動的にネイティブ効果（英数/
カタカナ/半角/全角への切替）を適用してしまう」として遮断している当の
キー種別であり、KeyUp単独であっても同様の実害が無いとは断定できない。

**`from`=かな方向（物理かなキー→安全な3キー）には引き続きhold-stateが
必要**: この方向は代入後vkが通常の安全な3キーのいずれかになり、
`transport.rs::plan`は通常どおり評価される（F2分岐を通らない）ため、
ADR-141決定7のKeyUp注入パターンがそのまま必要になる。これが
`SCAN_KANA_WAS_DOWN`/`SCAN_KANA_CONFIRMED_TARGET`の役割であり、決定5で
扱う失敗シナリオもこの方向のものである。

**r2で追加（architect役NM4・premortem役R1-M4指摘）**: ADR-141決定7が
KeyUp注入の対象として列挙する3箇所——`reset_physical_key_state()`
（`hook.rs:334`）、`clear_hook_latches_for_app_disable()`（`hook.rs:374`、
`disable_apps`のEnter/Leave両方から呼ばれる）、
`passthrough_or_swallow_for_impersonation`（`hook.rs:599-610`、overflow
ラッチ経路。`hook.rs:1100`付近の早期returnと`hook.rs:1203`の
`ProduceResult::Overflow`アームの両方から到達）——のいずれにも、
`SCAN_KANA_CONFIRMED_TARGET`が`Some(vk)`を保持している場合のKeyUp注入
処理を追加する必要がある。これはADR-141決定7の3箇所へのkana分の追加
であり、決定5が新設する2つのswallow分岐（`hook.rs:1013-1046`・
`:1066-1087`）とは別の合流点である（合計5箇所）。

### 決定5: 上流のAlt+かなswallowガードに、かなスロットの保留KeyUpを注入してからクリアする分岐を追加する（`from`=かな方向専用）

**r0からの方針転換**: r0は「`CACHED_SWALLOW_ALT_KANA_MODE_SWITCH`は挿入点
より前段にあるため構造的に独立、変更不要」と結論していたが、これは
**誤りだった**（architect役B1・premortem役B2/B3で独立に指摘）。「前段に
ある」ことこそが、かなスロットの保留中KeyUpを取りこぼす原因になる。
本決定は決定2のアーキテクチャ転換後も**変更なく必要**——`from`=かな
方向（物理かなキー→安全な3キー）は決定2の再設計と無関係に、通常どおり
`reinject()`でOSへ届く経路のままだからである。

**問題の実機的根拠**: `hook.rs:1066-1087`（`VK_DBE_ROMAN`/`NOROMAN`
swallow）と`hook.rs:1013-1046`（`VK_KANA`+Alt押下時のswallow）は、
いずれもKeyDown/KeyUpを区別せず無条件に`return LRESULT(1)`する早期return
である。BUG-62追補4の実機ログ（`docs/known-bugs.md:8746-8752`）は、
物理かなキーをAlt併用でリリースした際の実際のイベント列が
「`vk=0xF5 KeyUp`→`vk=0xF6 KeyDown`」という、Down/Upが対で来ない
非対称な列であることを記録している。

**失敗シナリオ**（config: かな↔スペースの2-cycle、既定設定）: (1) 物理
かなキーを単独押下（Alt無し、vk=0xF2）→ 決定1の条件を満たし
`SCAN_KANA_WAS_DOWN=true`・`SCAN_KANA_CONFIRMED_TARGET=Some(VK_SPACE)`、
OSへ`VK_SPACE`のKeyDownが送出される（この方向は常にAllowされる、
決定4参照）。(2) かなキーを押したままAltを押す。(3) かなキーを離す→
OSのレイアウト変換層がこのKeyUpを`VK_DBE_ROMAN`/`NOROMAN`として渡す→
`hook.rs:1066`のswallowガードに掛かり`return LRESULT(1)`→本ADRの挿入点
（`hook.rs:1135`以降）に到達しない→`SCAN_KANA_WAS_DOWN`はtrueのまま、
`VK_SPACE`のKeyUpは永久に送出されない→OS側でスペースキーが押しっぱなし
になる（BUG-100型のstuck key）。

**決定**: `hook.rs:1013-1046`と`hook.rs:1066-1087`の両swallow分岐で、
`return LRESULT(1)`する**前**に、以下の**両方**を満たす場合にのみ、
`SCAN_KANA_CONFIRMED_TARGET`が保持する`vk`のKeyUpを
`make_key_input_ex(vk, keyup=true, INJECTED_MARKER)` +
`win32::send_input_safe`直接呼び出し（ADR-141決定7-3と同じ方式、
フックスレッド内。`reinject()`はメインスレッド専用契約のため使えない）
で注入してから`SCAN_KANA_WAS_DOWN`/`SCAN_KANA_CONFIRMED_TARGET`を
クリアする:

1. `SCAN_KANA_CONFIRMED_TARGET`が`Some(vk)`である（保留中の代入がある）。
2. **この swallow対象イベント自身**が`!is_injected`である（**r2で追加、
   premortem役R1-M3指摘への対応。r4で条件を簡素化、premortem役R3-M2
   指摘への対応**: BUG-62追補4の実機ログは、他プロセス由来の
   foreign-injected `VK_KANA`が「物理キー操作と無関係に、Alt押下有無に
   関わらず常時〈打鍵ごとに数msおきに〉」到達し、その際`scan=0x0`・
   `extra=0x0`であることを記録している。この条件を付けないと、物理
   かなキーを押している最中にforeign-injected `VK_KANA`が届くたびに
   クリーンアップが誤発火し、代入先vkのKeyUpが早期に注入されて
   hold-stateがクリアされ、実際に指を離した際の本物のKeyUpが
   `confirmed_target=None`で処理される——OS視点でDown/Upの対応が壊れる
   新たな経路になる。

   **r4での訂正**: r2〜r3は条件2に`kb.scanCode == SCAN_KANA`も要求して
   いたが、これは「物理Alt+かなキーが生成する0xF5/0xF6のscanCodeも
   0x70である」という**未検証の前提**に依存しており、この前提が外れると
   本決定が防ごうとしているstuck keyそのものが復活する（scanが0で
   届けばクリーンアップが発火せず、代入先vkが押しっぱなしのまま残る）。
   一方、この条件が対処したかったforeign-injected `VK_KANA`ストームは
   `!is_injected`**単独で完全に排除できる**（ストームは定義上
   `LLKHF_INJECTED`付きであり、`is_injected`で確実に弾ける。scan一致は
   冗長な安全策のつもりが、未検証の前提に賭けるリスクだけを持ち込んで
   いた）。scan一致の条件を落とし、`!is_injected`のみを必須とする。
   物理Alt+かなキーのscan値の実機確認は、確認できればより厳しく絞れる
   改善項目に格下げする（Phase A実装時の宿題として残すが、本決定の
   正しさはこれに依存しない）。

このチェックはscan値（0x70）に紐づく既存の状態を参照するだけであり、
swallowガード自体が既存で防いでいるOSへの生イベント配送（0xF5/0xF6の
素通し・`VK_KANA`+Alt素通し）は一切変更しない——BUG-08/61/62対策
そのものには手を加えず、その手前で「かなスロットの役割代入が残した
未解決のhold-stateだけ」を掃除する。

**`swallow_alt_kana_input_method_switch=false`の場合**（premortem役B3・
R1-m3指摘）: このオプトアウト設定では`CACHED_SWALLOW_ALT_KANA_MODE_
SWITCH`が`false`のため、上記のswallow分岐自体が発火せず、0xF5/0xF6
イベントが素通しされる。BUG-62追補4の実機ログが示す
「`vk=0xF5 KeyUp`→`vk=0xF6 KeyDown`」という非対称な列がそのまま挿入点へ
届く場合、`0xF6 KeyDown`がfresh pressとして誤って新規の代入を確定させ、
対応するUpが来ない別種のstuck keyを生みうる（**r2訂正**: r1は「決定1の
scanベースmatchがそのまま機能するため追加対応不要」としていたが、
premortem役の指摘のとおりこれはr0-B3の孤児Down/Up問題そのものへの回答に
なっていなかった）。

**決定（r2で追加、r3で無効化範囲を訂正）**: `swallow_alt_kana_input_
method_switch=false`が設定されている間は、config読み込み時に**役割
代入ルール集合全体**を無効化する（**r3訂正、architect役NM7指摘**:
r2は「かなスロットが関与するルールだけ」を無効化するとしていたが、
これは決定3の全単射制約に反する。例えば3-cycle「変換→かな、かな→
スペース、スペース→変換」から「かなが関与する2本」だけを落とすと、
残る「スペース→変換」の1本により、恒等に戻った変換キーと元々の
スペース→変換ルールの両方が`VK_CONVERT`を生成し、NB2と同型の
「2つの物理キーが同一vkを生成する」状態を作る。ADR-141決定1-2が確立
した是正方法——個別ルールのskipではなく**ルール集合全体を無効化**する
——にここでも揃える）。この設定は「JISかな直接入力を意図的に使いたい
ユーザー向けのオプトアウト」（BUG-62追補5）であり、かなスロットの役割
代入とは利用者層が重なるが、非対称なDown/Upイベント列に対して安全な
hold-state管理を行う設計は未検証であり、実装コストに見合わない。
ユーザーがどうしても両方を使いたい場合は別途の設計検討が必要
（Phase C以降の課題とし、本ADRのスコープ外とする）。

**押下中の設定変更（r3追加、premortem役R2-M4指摘）**: `swallow_alt_
kana_input_method_switch`はGUI「詳細設定」から変更可能で、
`CACHED_SWALLOW_ALT_KANA_MODE_SWITCH`はreload時に更新される
（`hook.rs:567`）。物理かなキーを押している最中（`SCAN_KANA_
CONFIRMED_TARGET`が`Some`のまま）にこの設定が変更されても、
ADR-141決定3の「`confirmed_target`はKeyDown時点の確定値をKeyUpまで
保持する」規律にそのまま従う——設定変更はconfig再読み込みの完了時点
以降の**新規押下**にのみ適用され、既に確定している`confirmed_target`
を遡って無効化しない。この規律を守らないと、押しっぱなし中の設定変更
でKeyUpが失われBUG-100型のstuck keyになる。

`is_self_injected`（`hook.rs:935`、`is_injected`とは別物——`LLKHF_INJECTED`
一般ではなくawase自身の`INJECTED_MARKER`のみを見る）はこの2つのswallow
分岐より前段にあるため、決定5が新設するKeyUp注入自体が二重評価される
ことはない。

### 決定6: 親指キー設定（`THUMB_KEY_OPTIONS`）との追加バリデーションは不要と確認した

r0はここで「かなスロットの役割代入と親指キー設定は同時に有効化できない」
という全面禁止を新設していたが、r1レビュー（architect役M3）でこれが
**ADR-141決定4（r2）が撤回した失敗パターンの再導入**であり、かつ
実コードの動作を誤認していたと判明したため撤回する。

`hook.rs`の`update_thumb`（`:1137-1154`、**r2訂正**: architect役Minor3
指摘により行範囲を実測値へ修正）は`vk == config.left_thumb_vk`という
**代入後vk**基準で親指キー押下時刻を更新する。この判定は決定1の役割
代入（`hook.rs:1135`）より**後**に実行される。したがって:

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
- **r2追加（premortem役R1-m4指摘）**: `left_thumb_vk`/`right_thumb_vk`が
  `VK_KANA`(0x15)に設定されている場合、代入後vkが0x15になることは
  構造的に無い（決定2は`to`=かなの合成vkを`VK_DBE_HIRAGANA`に一本化して
  おり、決定1のscanベースfrom判定もvk=0x15を生成先として使わない）ため、
  この親指キー設定は恒久的に無反応になる。ただしこれは役割代入導入前
  から変わらない性質（ADR-141決定0が確定したとおり`VK_KANA`は実機では
  ほぼ届かない）であり、本ADRが新たに悪化させるものではない。Phase Cの
  GUI説明で一言触れる。

**決定**: 追加のconfig検証は行わない。ADR-141決定4のr2の教訓（全面禁止は
既定configで機能を丸ごと無効化しうる致命的な欠陥になりやすい）を踏まえ、
「代入後vk基準で全ての下流ロジック（親指キー判定・既定ホットキー判定
〈決定9参照〉）が一貫して動く」という全単射モデルの性質そのものに委ねる。

**r4追加（architect役NM10指摘）**: `right_thumb_vk = VK_DBE_HIRAGANA`
かつ「変換↔かな」の役割代入という、本決定が明示的に許容する構成では、
物理変換キーを押した際に`kp_stage_shadow_ime_toggle`の`delegate_owned`
ゲート（`key_pipeline.rs:1074-1075`、`:1121-1128`）が真になり、
shadow-toggle actuationがスキップされる（親指キーとして扱われ、
open軸はdelegateに委譲される設計のため）。この構成は決定2が新設した
`ime_will_be_turned_on_elsewhere`が`delegate_owned`の場合の分岐
（`hiragana_delegate_to_open_axis_armed`を見る）でカバーしており、
delegateがarmedでなければSuppressせずAllowへフォールスルーする。
`hiragana_delegate_to_open_axis`がarmedでない場合に「誰も何もしない」
状態（`key_pipeline.rs:1065-1072`がBUG-115の元症状として警告する状態）
を再現しないための決定2側の対応は、この分岐によって既に閉じている。

### 決定7（r2でほぼ解消）: Shift併用時の懸念は決定2の再設計により大部分が消滅した

r0は「Shift併用時にカタカナへ切り替わらない」という（誤った）既知の
制限を記載し、r1はこれをhook.rs側のmodifierゲート（Shift押下中は代入
不発火）で対処しようとしていた。r2の決定2再設計により、`to`=かな方向は
OSへ生のvkを一切送らなくなったため、「Shift併用でOS/IMEが0xF2を
カタカナ切替と誤解釈する」という問題自体（r0-M2、`shift_katakana_
passthrough`の分岐可能性）が構造的に発生しなくなった。したがって`to`
方向についてはhook.rs側のShiftゲートは不要であり、決定2から削除した。

**`from`=かな方向（物理かなキー→安全な3キー）のShift併用**: この方向は
通常の安全な3キーのいずれかへの代入であり、ADR-141の安全な3キー同士の
代入と同様、Shiftの有無で特別な分岐は発生しない（Shift+物理かなキーが
生成する`VK_DBE_KATAKANA`(0xF1)は決定1のscanベース判定に乗り、代入後の
安全な3キーのvkとして扱われる。これはADR-141の対象外の`VK_DBE_KATAKANA`
というvk値が、決定1の設計により初めて代入対象に含まれることを意味する
——決定1の背景節が既に述べたとおり、scanベース判定はvkの値を問わない
ため意図した挙動である）。

**既知の制限**: Shift併用時、代入後の位置では物理かなキー固有の
Shift併用効果（`shift_katakana_passthrough`経由のカタカナ入力）は
再現されない——`from`方向で物理かなキーが安全な3キーへ代入されている
限り、Shift+その物理キーは代入後の安全なキーのShift併用挙動になる
（例:「かな→スペース」でShift+物理かなキーは代入後vk=`VK_SPACE`の
Shift併用、すなわちMS-IMEの半角/全角スペース切替相当になる）。これは
「かな入力ロックがそのままの位置に残らない」という意味論上自然な帰結
であり、ユーザーには設定GUI（Phase C）で明示する。

**r3追加（premortem役R2-m2指摘）**: かなスロットの役割代入を有効にした
ユーザーでは、`shift_katakana_passthrough`（`transport.rs:103-109`）
——BUG-116/ADR-137が実機検証のうえ「BUG-52由来の無条件Suppressに対する
唯一の例外」として意図的に開けた経路——が到達不能になる。物理かなキー
自身が常にscanベース判定（決定1）で代入対象になるため、このキーが
`VK_DBE_KATAKANA`のまま`transport.rs::plan`へ到達する経路（代入なしの
場合）以外では、この分岐に乗る機会が無くなる。既知の制限として明記する。

**r4追加（architect役Nit1指摘）**: 同様に、`DbeModeKeyContext`の2つの
否定ガード`is_configured_thumb_key`・`half_width_alnum_toggle_active`
（ADR-137 M-1/M-3で追加）も、かなスロットの役割代入を有効にした
ユーザーでは物理かなキーが常に代入対象になるため意味を持たなくなる。
BUG-116/ADR-137は2026-09-06実装の直近機能であり、後で「なぜ効かないのか」
を追う人のために一言記録しておく。

### 決定8: config形式・設定GUIはPhase Cへ切り出す

ADR-141/142と同様、本ADR（Phase A相当）は挿入点・判定ロジック・既存機構
との相互作用のみを決定する。具体的なconfig表現（`[[key_role]]`の`from`/
`to`にかなスロットをどう指定させるか——`VK_DBE_HIRAGANA`という名前を
そのまま使うか、`"kana"`のような物理キーエイリアスを新設するか）・
設定GUIのプリセット表現は、ADR-142の拡張として後続ADR（Phase C、本ADRの
後続として次の空き番号を採番）で決定する。

**テスト網羅性（r2追加、architect役Minor4指摘）**: ADR-142決定B7が
安全な3キーについて確立した`event_eligible`パラメータへの畳み込みは、
かなスロットにも適用する。`from`=かな方向（決定1・決定4・決定5）は
`event_eligible`（`!alt_impersonated && !is_injected`）に加えて
`scan == SCAN_KANA`が判定に加わるだけであり、r1で懸念された次元爆発
（modifierゲート追加による32→128通り）は決定2の再設計でゲート自体が
無くなったため発生しない。`decide_role_substitution`の網羅テーブル
テストは、安全な3キー用の32通りに加え、かなスロット用の同型テストを
追加すること。また6変数（3キー×2）+ 2変数（`SCAN_KANA_WAS_DOWN`/
`SCAN_KANA_CONFIRMED_TARGET`）がスロットとして混線しないことを確認する
テスト（ADR-141決定2が要求する「変換を押しっぱなしのまま無変換を押して
離しても変換側のconfirmed_targetが保持される」の4要素版）も必要。

**確認済み事項（r2追加、architect役Minor5/r0-Nit2指摘への対応）**:
`classify_key`（`hook.rs:42-63`）は挿入点直後で`(vk, scan)`の両方を
消費するが、かなスロットについては安全であることを確認済み。
`scan_to_pos_jis`（現行JISテーブル）にscan 0x70のエントリが無いため
`Char`には落ちず`Passthrough`になり、0xF0-0xF2は`is_passthrough`のVK
範囲（0x70..=0x87、VK_F1〜VK_F24の意味）の外にある。ただしこれは
現行JISテーブルの内容に依存した安全性であり、US配列テーブルが将来
scan 0x70を持つようになった場合は再検証が必要。**r3追加（premortem役
R2-m3指摘）**: `classify_key`は`vk == left_thumb/right_thumb`を最初に
評価するため、親指キーが`VK_DBE_HIRAGANA`に設定された構成（決定6が
「意図した挙動」とするケース）では、代入後の0xF2は`LeftThumb`/
`RightThumb`に分類される。この分類経路も確認範囲に含めて安全である
ことを確認済み（決定6の論拠そのもの）。

**テスト・実装計画への追加項目（r3追加、premortem役R2-m1・R2-B3指摘への
対応）**: `transport.rs::plan`の既存ユニット/goldenテスト（Linux実行
可能な純粋関数テスト）に`scan_code`の設定・`kana_role_active`フラグの
既定値（`false`）テストを追加する。`DbeModeKeyContext`への4つ目の
フィールド追加に伴う`From<DbeModeKeyPolicy>`の既定値（`kana_role_active
= false`）テストも必要（ADR-142決定B7のテスト計画の拡張として書き足す）。
`kp_restore_hiragana_for_suppressed_mode_key`（決定2既知の制限3）の
`event.scan_code == SCAN_KANA`除外条件のテストも追加する。**r4追加
（premortem役R3-m4指摘）**: 回帰防止の要として、`injected=true`の
ケース（R2-B1の回帰防止、Suppressされないことを確認）と
`kana_role_active=false`のケース（R2-B2の回帰防止、既存の
`is_tsf_mode && f2_warmup_owned`判定にフォールスルーすることを確認）
を名指しで要求する。`plan`はLinux実行可能な純粋関数なので、いずれも
`cargo test -p awase-windows --lib`で自動テスト化できる。

### 決定9: 代入後vkを基準に評価される下流の合流点を棚卸しする

premortem役M6指摘を受け、決定1〜8がカバーしない、代入後vkを見る下流
ロジックを以下のとおり棚卸しする。

1. **`vk_may_mutate_conv`（`vk.rs:187-194`、ADR-084/086 conv mode force
   policyの再発ファミリー）**: `VK_CONVERT`(0x1C)・`VK_KANA`(0x15)・
   0xF0-0xF6は`true`、`VK_NONCONVERT`(0x1D)・`VK_SPACE`は`false`。
   「変換→かな」の代入では変換キーは代入前後どちらもconv-mutating
   （0x1Cも0xF2も`true`）のため実質的な変化は無い。一方、全単射
   （決定3）により2-cycleでは必ず逆方向（かな→無変換、かな→スペース）
   も同時に設定されるため、**両方向の反転を確認する必要がある**
   （**r2追加、architect役Minor2指摘**）: 「無変換→かな」または
   「スペース→かな」では、その物理キーが代入前は非conv-mutating
   だったのに代入後はconv-mutatingになる。同時に、対になる「かな→
   無変換」または「かな→スペース」では、物理かなキーがconv-mutating
   から非conv-mutatingへ反転する。実際の露出点は`win32.rs:169`の
   `send_input_safe`が参照するconv_mutationゲートで、`reinject()`が
   渡す代入後vkを見る。conv mode policy=force（ADR-086）がこの双方向の
   反転を正しく扱えるかは未検証であり、実装前に`state/conv_mode.rs`・
   `runtime/conv_actuation.rs`との相互作用を専用に確認する必要がある
   （`.claude/rules/fix-requires-evidence.md`の「conv mode」ファミリー
   に該当、未解決の疑問7）。
2. **既定ホットキー（`Ctrl+変換`=IME ON、`Ctrl+Shift+変換`=engine ON等）・
   `runtime/focus_tracker.rs::enrich_ime_relevance`のper-app sync key
   （ADR-141決定6-2）・`vk::is_composition_confirm_key`（`vk.rs:319`、
   対象は`VK_SPACE`/`VK_RETURN`/`VK_ESCAPE`）**: これらはいずれもVK値で
   判定されるため、決定3の全単射制約が保たれる限り、判定対象は
   「代入後にそのVKを生成する物理キー」へ自動的に追従する（決定6と
   同型の論拠——2-cycleなら入れ替わった相手キーで、3-cycle以上でも
   巡回先のキーで、必ずどこかの物理キーがそのVKを生成し続ける）。
   したがって**機能そのものの喪失は起こらない**（全単射性による構造的
   保証）。

   ただし ADR-141決定6が確立した要件は「喪失しないことの保証」では
   なく「**意図しない反転が起きることをconfig検証時に警告する**」こと
   だった（**r2訂正、architect役NM3指摘**: r1決定9-2は「喪失は起こら
   ない」の一点のみで済ませ、この必須要件を弱めてしまっていた）。
   例えば`VK_SPACE`が4オブジェクトの1つである以上、「かな↔スペース」
   の2-cycleでは**物理かなキーがcomposition確定キーになり、物理スペース
   キーが確定キーでなくなる**（`executor.rs`の`handle_confirm_key_
   passthrough`/`handle_reinject`の発火対象が入れ替わる）。GJI/MS-IMEの
   変換確定操作に直結する挙動変化であり、ADR-141決定6が安全な3キー内の
   反転について課した「Phase B（本ADRではPhase C）のconfig検証で衝突を
   警告する機能を必須要件とする」を、かなスロットを含む4要素の組み合わせ
   へそのまま拡張する。GUI文言の検討は未解決の疑問3と統合。
3. **`AppImeProfile::ImmCross`への0xF2漏洩は、本ADRが新規に持ち込む
   ハザードではない**（**r2訂正、architect役NM5・premortem役R1-M6
   指摘**: r1決定9-3はこれを本ADR固有の新規ハザードとして扱い、
   `RawKeyEvent`への出自フィールド追加まで検討課題に挙げていたが、
   これはスコープクリープだった）。`transport.rs::plan`の評価順序
   （InputRelay早期return→F2分岐→injected早期return→ImmCrossアーム）
   は、ImmCrossプロファイル×`is_tsf_mode=false`の場合に`VK_DBE_HIRAGANA`
   がF2分岐で`Allow`されてImmCrossアームに到達しないという性質を既に
   持っている——**これは物理かなキー自身の押下についても現状で真**
   であり、役割代入が変えるのは「どの物理キーがそれを生むか」だけで、
   配送判断そのものは同一である。加えて決定2の再設計により、役割代入
   由来の0xF2（`scan_code != SCAN_KANA`）は常にSuppressされるため、
   本ADRが新たにこの経路でImmCrossへ漏洩を増やすことは無い（物理かな
   キー自身の押下〈`scan_code == SCAN_KANA`〉についての既存の性質は
   本ADRのスコープ外、ADR-141決定4末尾がAltセンチネル構成の既存衝突に
   対して取った立場——悪化させないが解消もしない——と同型に整理する）。

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
4. **（r2で部分的に解消、r3で範囲を訂正）MS-IME環境での決定2の有効性**:
   r1時点では`reinject()`の`wScan: 0`がMS-IME環境で実際にモードキーとして
   処理されるかが未検証だったが、決定2のr2再設計（OSへ役割代入由来の
   `reinject()`を送らない設計への転換）により、**reinject経路について
   この懸念は解消した**（architect役Minor4指摘）。ただし、actuation経路
   自体（`ime.rs::send_ime_mode_key_with_shift_release_prefix`が
   `make_scan_key_input`をimportしており、scan付きで送る経路を持つ）に
   ついては、送出VKがDBE系でないという決定2の安全性論拠により本ADRの
   対象外だが、scanの有無に依存する問題が形を変えて残っていないかは
   Phase A実装時に再評価する。
5. **物理かなキーの役割代入がIME belief観測に与える影響**: 決定1の
   副作用として記載した、`classify_ime_relevance`経由のshadow belief
   観測が代入後は失われる点について、既存のbelief更新ロジック
   （`state/ime_model.rs`）への実害の有無は未検証。
6. **`kp_restore_hiragana_for_suppressed_mode_key`の各ゲートの役割代入対応
   監査（r2で優先度上昇）**: 決定2の既知の制限3に記載した
   `is_configured_thumb_key`・`half_width_alnum_toggle_before`・
   `kana_mode_restore_key_down`（単一グローバルラッチ）・
   `conv_mutation_allowed`・`is_composition_warm()`複合条件・
   `read_kana_lock()`によるABORT（いずれも`key_pipeline.rs:81-201`の
   範囲内）が、代入元が物理かなキー以外になった場合にも正しく動作
   するかの専用監査が必要。決定2のr2再設計により「安全な3キー→かな」
   方向のDownは常にSuppressされるようになったため、この経路の発火
   頻度がr1時点より増しており、監査の優先度が上がっている。
7. **conv mode force policy（ADR-086）との相互作用**: 決定9-1に記載した
   `vk_may_mutate_conv`の意味論反転（無変換/スペース→かな、および
   その逆方向、双方向の反転）が、ADR-086のforce policyの前提を壊さない
   かの専用検証が必要。
8. **（r2で解消）ImmCrossプロファイルへの代入後0xF2漏洩の防止方式**:
   決定9-3のr2訂正のとおり、これは本ADR固有の新規ハザードではなく
   既存の性質であり、かつ決定2の再設計により役割代入由来の0xF2は
   常にSuppressされるため、防止方式の検討自体が不要になった。
9. **（r3で解消、削除）「安全な3キー→かな」方向の実装方式**: r2は
   Suppressされたイベントをactuation経路へ「渡す」実装方式を未確定として
   いたが、r3で判明したとおりactuationは既存の`kp_stage_shadow_ime_
   toggle`が`plan()`より前に既に発火させており、新規の実装は不要
   （決定2のNB4参照）。この疑問自体が前提の誤りに基づいていたため削除。

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
  のみで独立した決定内容を持たなかった）を削除し、スコープ節へ統合。
- r2（2026-09-06）: Opus 2体の敵対的レビュー2ラウンド目で、r1の改訂
  自体が新規Blocker 2件を混入させていたと判明（ADR-141/142で繰り返された
  「改訂が新Blockerを生む」パターンの再現）。(1) r1決定2のmodifierゲート
  が`to`方向にしか掛からず、`swallow_alt_kana_input_method_switch=false`
  構成でAlt押下中に2つの物理キーが同一vkを生成し全単射を破る（NB2/
  R1-B1）、(2) r1決定4の`KANA_DOWN_WAS_ALLOWED`がフックスレッドと
  メインスレッドを跨ぎ原理的に実装不能かつ誤ったスロットに付いていた
  （NB1/R1-B2/R1-B3）——の2件。premortem役の構造的提案を採用し、決定2を
  「OSへ生のSendInputでかな相当を届ける」設計から「既存のIME actuation
  経路（`kp_stage_shadow_ime_toggle`→`ime_controller::apply`）に委ねる」
  設計へ全面的に再設計した（`transport.rs::plan`のF2分岐を
  `scan_code != SCAN_KANA`の場合は常にSuppressするよう拡張し、決定1が
  確立したscan不変性をそのまま流用）。これにより上記2件に加え、r1で
  個別対処しようとしていたMajor群（`alt_key_held()`のAltセンチネル
  常時true問題・`modifier_snapshot`が挿入点に存在しない問題・ImmCross
  漏洩・MS-IMEでscan=0が効かない懸念・既定ホットキーのShift分裂）も
  ゲート機構自体の削除により同時に解消した。決定4を`from`=かな方向専用に
  簡素化（`KANA_DOWN_WAS_ALLOWED`撤回）、決定5にBUG-62追補4のforeign-
  injected `VK_KANA`ストーム対策とオプトアウト設定時のかなスロット無効化
  を追加、決定7をほぼ解消扱いに縮小、決定9を修正（conv_mutationの双方向
  反転・ADR-141決定6-2/6-4の追加・ImmCross漏洩の「既存挙動」への再整理・
  既定ホットキー警告の必須要件復元）。変数名を`KANA_*`から
  `SCAN_KANA_*`へ改名（scan基準の識別であることを明示）。
- r3（2026-09-06）: Opus 2体の敵対的レビュー3ラウンド目で、r2のアーキ
  テクチャ転換の方向性は正しいと確認されたが、安全性論拠に事実誤認が
  あり新規Blocker 3件が見つかった。architect役が独立に発見:
  (1) NB3——「actuation経路が`ime_mode_key_injection_blocked_by_
  modifier()`を経由するので安全」という論拠が誤り（この関数の呼び出し
  箇所はリポジトリ全体で3つあり、いずれも半角英数トグル復元経路で
  actuation本経路には存在しない）、(2) NB4——「Suppressされた結果として
  actuation経路へ渡す」という因果順序が逆（`kp_stage_shadow_ime_toggle`
  は`plan()`より前に既に発火しており、新規のディスパッチ機構は不要）、
  (3) NB5/R2-B1・R2-B2——`transport.rs::plan`の新条件に`!event.injected`
  と機能有効時のみ適用する`kana_role_active`フラグが欠けており、他
  プロセスrelayの巻き込み・機能未使用ユーザーへの影響という2つの回帰面
  を作っていた。premortem役はR2-B3として、この新条件が`kp_restore_
  hiragana_for_suppressed_mode_key`の「`physical==Suppress`はGJI戦略
  Suppressの必要十分条件」という既存の同値性を壊し、MS-IME環境でGJI
  専用のscan付き0xF2注入を誤発火させることを発見した。決定2の安全性
  論拠を「経路を共有するから安全」から「送出VKがDBE系でないから安全
  （`VK_IME_ON`/`VK_KANJI`であって`VK_DBE_*`ではない）」という検証可能
  な形へ訂正し、`transport.rs::plan`の新条件に`!event.injected`・
  `kana_role_active`（`DbeModeKeyContext`の4つ目のフィールド）を追加、
  `kp_restore_hiragana_for_suppressed_mode_key`に`event.scan_code ==
  SCAN_KANA`の除外条件を追加。未解決の疑問9（実装方式）を、前提が誤り
  だったため削除。決定4にInputRelayでのDown/Up非対称という狭い既知の
  制限を追加、決定5のオプトアウト無効化範囲を「かな関与ルールのみ」から
  「ルール集合全体」へ訂正（全単射を壊す欠陥の是正）、決定7に
  `shift_katakana_passthrough`到達不能の既知の制限を追加。BUG-10
  （食い逃げ）への言及と補償の論証を決定2に追加。
- r4（2026-09-06）: Opus 2体の敵対的レビュー4ラウンド目で、premortem役
  はr3のBlockerをゼロと判定（r2の3件はすべて解消）したが、architect役
  が新規Blocker1件（NB6）を発見した。決定2の代償2（BUG-10の補償論証）
  が「`kp_stage_shadow_ime_toggle`は`plan()`より前に既に発火している」
  という主張のみに基づいていたが、同関数には実際にactuationへ到達する
  前に3つのゲートがあり、うち no-op分岐（belief状態が既に目的の状態と
  一致している場合、actuationしない）がBUG-10の「intent/EngineだけONで
  実IMEがOFFのまま乖離した」状態でまさに発火し、この分岐に加えて物理
  配送のSuppressも重なることで、実IMEをONへ戻す経路が完全に塞がれる
  ことが判明した。`ime_will_be_turned_on_elsewhere`を新設し、
  `delegate_owned`・no-op分岐（`effective_open()`）の状態に応じて
  既存の仕組みがIMEを実際にONにする場合にのみSuppressするよう決定2を
  修正。この修正は同時に、決定6が「意図した挙動」として許容する
  `right_thumb_vk=VK_DBE_HIRAGANA`×かな役割代入の組み合わせで
  `delegate_owned`ゲートと重なりBUG-115の元症状（誰も何もしない状態）
  を再現しうるというNM10も解消した。premortem役のMajor 3件
  （R3-M1: `kana_role_active`の押下中反転でKeyUpにDownを持たない0xF2が
  出る、R3-M2: 決定5条件2のscan一致が未検証の前提に依存し外れると
  stuck keyが復活する、R3-M3: `kana_role_active`のSSOT未指定）にも
  対応: KeyUp側の判定から`kana_role_active`を除外、決定5条件2から
  scan一致を外し`!is_injected`のみに簡素化、`kana_role_active`は
  ADR-142決定B1が新設する`state/key_role.rs`のSSOTから導出することを
  要件化。architect役独立発見のMinor1（判別子`scan_code != SCAN_KANA`
  の健全性の証明）・Minor3（`suppress_reason`の二重管理防止のための
  共有ヘルパ新設）・Minor4（InputRelay既知の制限にKeyUpがinertである
  保証が無い旨を追記）・Nit1（`DbeModeKeyContext`の2ガードが無意味化
  する旨を決定7に追記）も反映。
