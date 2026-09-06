# ADR-143: かな系キーの役割代入（Kana Key Role Substitution）

## ステータス

**収束（r11相当、両エージェント最終確認済み）。r10でOpus 2体の敵対的
レビューを10ラウンド実施しBlocker/Major/Minor/Nit全てゼロで承認された
後、持ち越し3点（実装順序・NM18・from_name表現）を実際に解消する過程で、
architect役・premortem役との追加協議（実質r11、途中でさらにBlocker
相当1件・Major相当1件を新規発見・修正）により以下を確定した。
2026-09-06、両エージェントとも最終差分を確認しBlocker/Major/Minor
（Nit3件は反映済み）ゼロで再承認:**

1. **実装順序**: ADR-141（Phase A基盤）→ADR-142（Phase B config/GUI）
   →本ADRの順で実装すること（変更なし）。
2. **【解決済み】NM18（ADR-141決定1とADR-142決定B7の`{KEY}_WAS_DOWN`
   更新規則の不一致）**: 「hold-stateへのstoreは`event_eligible`で
   ゲートし、vk変換自体は無条件に行う」という形で決着した（ADR-141
   決定1「r-later訂正」・ADR-142決定B7「訂正」参照）。当初「ADR-141
   決定1の字面どおり（適用とstoreをいずれも`!is_injected`ガード内で
   行う）を正とする」という r10時点の推奨は、ADR-142決定B7のr3→r4
   stuck-key対策と矛盾することが追加協議で判明し撤回された。この訂正
   に伴い、本ADR決定2のフォールバック（NM17）の正当化も再訂正した
   （injected由来のhold-state stuckは両方向とも構造的に消滅、残る
   正当化は`ProduceResult::Overflow`経路のみ）。
3. **【解決済み】かなスロットの`from_name`表現（決定8-1〜8-4）**:
   config記号は`"VK_DBE_HIRAGANA"`のみ受理（`"VK_KANA"`/`"かな"`/
   `"カナ"`/`"Kana"`と0xF0/0xF1/0xF3-0xF6は拒否）、rule table格納値は
   0xF2、拒否・正規化は単射チェックより前。GUI表示文言のみPhase C送り。
4. **【追加で発見・修正、Blocker相当】かなスロットの恒等補完の意味**
   （決定3参照）: 「恒等＝rule_targetをかなスロットの正規VkCodeとして
   テーブル引きする」実装だと、かなスロットを一切設定していないユーザー
   でもBUG-52/BUG-116（ADR-137）の既存挙動が壊れることが判明した。
   「恒等＝テーブル引き自体をスキップしvkを書き換えない」に訂正した。
5. **【追加で発見・修正、Major相当×3】injected KeyUpによるラッチ迂回**
   （決定2・決定4参照）: `to`=かな方向のDownがSuppressされた後、リレー
   ツール等のinjected KeyUpが決定2の静的3条件（`!event.injected`）を
   満たさずラッチを迂回し、対応するDownを持たない0xF2のKeyUpがOSへ
   送出されうる経路を発見。3段階で訂正した:
   (a) 当初`confirmed_target == Some(VK_DBE_HIRAGANA)`を条件にしたが、
   これは「Downは常にSuppressされる」というr3時点の前提に乗っており、
   決定2のr4以降（`actuation_will_fire`が偽のフォールスルー）では
   DownがAllowされOSへ実際に配送される場合があるため誤り（architect役
   指摘）→条件をラッチの値そのもの（`Some(true)`＝Suppressのときのみ）
   に訂正。
   (b) その訂正版も「hook.rs側のvk変換自体を止める」実装は不可能と
   判明（premortem役R11-M1指摘——変換はフックスレッド、ラッチは
   メインスレッドの値でクロススレッド参照になる）→例外の置き場所を
   `plan()`側のKeyUp専用Suppress分岐へ移し、hook.rs側の変換（ADR-141
   決定1「適用は無条件」）自体には触れない形に再訂正。
   (c) この移動によりinjected KeyUpがラッチを**読む**ようになった
   ため、ラッチの**クリア**規則も`!event.injected`でゲートしないと、
   injected Upが先にラッチをクリアし直後の物理Upがフォールスルーする
   形で同じ問題が別経路から復活すると判明（architect役指摘）→クリアは
   非注入のKeyUpのみ、読み取りはKeyUpに限りinjectedにも開く、という
   形に最終確定した。

Phase A実装時の受け入れ条件として以下の実機確認が必要（決定8のテスト
計画に加えて）: 代入先キー押下でGJI・MS-IME双方の実IMEが実際にONになる
こと、代入先キーを押しっぱなしにしてOSへ0xF2が繰り返し届かないこと
（auto-repeat時の二重送出防止の検証）。**

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

### レビュー指摘との対応表（r4→r5、5ラウンド目）

両エージェントが独立に同一の2つのBlockerへ到達した（NB7=R4-B1、
NB8=R4-B2）——r4で新設した`ime_will_be_turned_on_elsewhere`自体が、
まさにこの機構が対処しようとしていた「生の0xF2直接送出」を別の経路
から再導入していた。

architect役: NB7(`effective_open()`をライブ値で読むと機構全体が発火
しない)→事前スナップショット化・`_before`命名規約への統一、
NB8(KeyUpで再評価すると成功パスで毎打鍵非対称が生じる)→KeyUpの判定
から完全に除外、NM11(`delegate_owned`が呼び出し元から二重計算になる)
→`kp_stage_shadow_ime_toggle`の戻り値を構造体化、Minor1(決定5の
scan省略に根拠が無い)→背景節の事実を根拠として追記、Minor2(フラグ名が
実態とずれ誤読を誘発)→`ime_actuation_will_fire_before`へ改名、
Nit1(カタカナ側delegateへの言及漏れ)→0xF2固定により構造的に非該当と
明記。
premortem役: R4-B1(`effective_open()`のタイミング問題、architect役
NB7と同一発見)→同上で解消、R4-B2(KeyUp側の構造的常時false、architect
役NB8と同一発見)→同上で解消、R4-M1(近似であることの明記不足)→既知の
制限として明記、R4-M2(delegateの代入後構成での正しさ未検証)→実機確認
項目に追加、R4-m1〜m3→narrow edge記述の訂正・テスト計画のDown/Up分離・
`suppress_reason`ヘルパのDown/Up対応。

5ラウンド共通の教訓: 本ADRのようにActuation/belief/配送が絡む機構では、
「新しい条件が正しい値を指すか」だけでなく「その値がいつ読まれるか
（呼び出し前後でどちらのスナップショットか）」まで具体的なコード行で
検証しないと、r4のように機構全体が無効化される欠陥を見逃す。このリポ
ジトリには`half_width_alnum_toggle_before`という同型の前例が既に存在
しており、次に類似の値を扱う際はまずこの命名規約・配線を踏襲すること。

### レビュー指摘との対応表（r5→r6、6ラウンド目）

premortem役はr5にBlockerゼロと判定した（Major 3件のみ）。architect役は
新規Blocker1件（NB9、auto-repeat経路）を発見した——r3・r4に続き3度目の
「片方がゼロ・もう片方が新規Blocker発見」という分かれ方で、いずれも
具体的なコード引用を伴う検証可能な指摘だったため採用した。

architect役: NB9(auto-repeat KeyDownで毎回Allowへフォールスルーし
押しっぱなし中ずっと生の0xF2が漏れる)→fresh press時点でのラッチ化に
より解消、NM12(BUG-46前例の引用が不正確かつ0xF2に適用されたことが
無い)→ラッチ化によりDown/Up非対称自体が無くなり解消、Minor1〜3→
決定2の記述に反映（早期return時の値・健全性証明のKeyUp依存・delegate
分岐のauto-repeat反転、いずれもラッチ化により発生条件が無くなった）。
premortem役: R5-M1(`ime_actuation_will_fire_before`の非delegate分岐は
既に`plan()`へ渡っている`shadow_toggled`と同一情報で二重計算)→
`shadow_toggled`を再利用する設計へ簡素化、R5-M2(delegate分岐の述語が
配線先の方向を見ていない)→`Some(ShadowImeAction::TurnOn)`との一致判定
へ訂正、R5-M3(「機能未使用ユーザーには挙動がビット単位で同一」という
主張がKeyUp条件と矛盾)→ラッチ化によりKeyUp独自の条件式自体が無くなり
解消。

6ラウンド共通の総括: architect役が3ラウンド連続（r3のNB6・r4の
NB7/NB8・r5のNB9）で発見したBlockerは、いずれも「actuation/belief
関連の値を、いつ・どのイベント種別で読むか」という時間軸上の詰めの
甘さに起因していた。premortem役が提案したr6の簡素化（`shadow_toggled`
の再利用）は、新しい値を作るたびにこの種の欠陥を持ち込むリスクそのもの
を減らす方向の修正であり、fresh press時点のラッチ化と組み合わせること
で、Down/auto-repeat/KeyUpという3種類のイベントを個別に検討する必要
自体を無くした。

### レビュー指摘との対応表（r6→r7、7ラウンド目）

premortem役はr6にBlockerゼロと判定した（Major 3件、いずれもラッチ
仕様の記述漏れ）。architect役は新規Blocker1件（NB10）を発見した——
r3・r4・r6に続き4度目の「片方がゼロ・もう片方が新規Blocker発見」と
いう分かれ方で、今回も両エージェントが独立に「fresh pressの判定根拠
がメインスレッドから参照できないhook側の値を指している」という同一の
核心に到達した（architect役NB10・premortem役R6-M1）。

architect役: NB10(fresh press検出手段が未定義でラッチがstale `Some`に
なる経路が3つある)→hook.rsが`is_fresh_press`を`RawKeyEvent`の新
フィールドとして明示的に運ぶ設計へ訂正、NM13(ラッチ判定とInputRelay
早期returnの評価順序が未定義)→ADR-119の既存順序を優先し狭い既知の
制限として明記、NM14(`delegate_will_turn_on`が既存の`turn_on_
direction`の再実装)→既存計算を流用する形へ訂正、Minor1〜3・Nit1→
決定2/決定4の記述に反映。
premortem役: R6-M1(fresh press判定根拠がhook側`was_down`でメイン
スレッドから参照不能、architect役NB10と同一発見)→同上で解消、
R6-M2(ラッチを参照してよいイベントの限定が未記載)→静的3条件を満たす
イベントに限定する記述を追加、R6-M3(ラッチが取り残される経路が実在し
フックスレッドからクリアできない)→fresh press明示ビットによる自己
修復で解消、R6-m1〜m3→決定2/決定8に反映。

7ラウンド共通の総括: r6のラッチ化は「いつ再評価するか」の問題は解決
したが、「freshnessをどう検出するか」という新しい時間軸の問題を
持ち込んでいた——r1（クロススレッドで値を渡そうとして破綻）と対称的な
失敗である。r1は「メインスレッドの値をフック側で読もうとした」、
r7で見つかったのは「フック側の値をメインスレッドで読もうとした」で、
向きは逆だが構造は同じ「スレッド境界を跨ぐ値の受け渡しを暗黙に仮定
する」という誤り。解決策も対称的で、r1はメインスレッド側で完結する
設計に倒したのに対し、r7はhook.rsが計算した値を`RawKeyEvent`という
既存のデータフロー経由でメインスレッドへ**明示的に運ぶ**設計に倒した
（プラットフォーム層が事前分類した値をcoreへ渡す、というADR-019の
層境界原則にも合致する）。

### レビュー指摘との対応表（r7→r8、8ラウンド目）

両エージェントが独立に、r7の`is_fresh_press`実装位置に同一の欠陥を
発見した（architect役NB10、premortem役R7-M1）——`hook.rs:1135`以降の
挿入点では、汎用の押下状態（`PHYSICAL_KEY_STATE`/`PHYSICAL_KEY_DOWN_
AT_MS`）が既にこのKeyDown自身によって更新済みであり、`is_keydown &&
!was_down`は常にfalseになる。これはr4のNB7と同じ失敗モードの3度目の
再来だった。両エージェントとも同じ解決の方向（既存のfresh-press検出
イディオムをより早い位置——`PHYSICAL_KEY_DOWN_AT_MS`の更新ブロック
自体——で使う）を独立に提案した。

architect役: NB11(`is_fresh_press`の定義域・計算元が未定義で常にfalse
になる)→`PHYSICAL_KEY_DOWN_AT_MS`更新ブロック内の`prev == 0`イディオム
を流用する位置へ訂正、NM16(vkキーであるためかなスロット自身には
信頼できない)→フィールドdocへの制約明記、Minor1〜3・Nit1→決定8の
テスト計画・決定2のラッチクリア明記・NM13既知の制限への留保再併記・
layer-boundaries.mdカテゴリ明記に反映。
premortem役: R7-M1(計算順序が挿入点更新後を指す、architect役NB11と
同一発見)→同上で解消、R7-M2(フィールドが汎用core型なのに定義域が
4オブジェクトのみ)→計算位置の訂正により全VKで意味を持つ値になり
同時に解消、R7-M3(fresh press自身がoverflowで失われるケース)→
ラッチ`None`でのKeyDown評価をフォールバックとして併用、R7-m1〜m3・
R7-n1→決定8のテスト計画・決定4/決定2の相互参照・NM13重複整理・
フィールドdocに反映。

8ラウンド共通の総括: r7の`is_fresh_press`導入も、r4〜r6で繰り返し
見てきた「新しい値をいつ・どの位置で計算するか」という同じ種類の
落とし穴を踏んだ。今回の教訓は、**既存のコードベースに同種の問題への
対処イディオムが既にある場合はそれを流用すべきで、新しい計算位置を
一から選ぶと同じ罠を踏み直す**ということ——`PHYSICAL_KEY_DOWN_AT_MS`
の`prev == 0`判定は、まさに「auto-repeatと新規押下を区別する」という
本ADRが繰り返し必要としてきた判定そのものであり、最初からここを見て
いれば3ラウンド分（NB7・r6のラッチ化議論・NB10/NB11）の一部を短縮
できた可能性がある。

### レビュー指摘との対応表（r8→r9、9ラウンド目）

architect役が新規Blocker1件（NB12）を発見した——r8の教訓「既存の
イディオムを流用すべき」が、そのイディオムが属する**状態機械の
ライフサイクルごと**借りてきてしまうという新しい失敗を生んだ。

architect役: NB12(`is_fresh_press`と役割代入自身の`was_down`が二重の
情報源になりクリア箇所が食い違う)→情報源を役割代入自身の`{KEY}_
WAS_DOWN`/`SCAN_KANA_WAS_DOWN`へ一本化し`role_substitution_fresh_
press: Option<bool>`へ改名、Minor1〜3・Nit1→情報源統一により大部分が
解消（injectedは`None`で自然に表現、`reset_physical_key_state`の
挙動を既知の制限として明記）。
premortem役: R8-M1(`reset_physical_key_state`によるmid-holdの再確定)
→情報源統一後もADR-141が既に持つ性質として既知の制限に明記、
R8-m1〜m3→フィールド型の変更・テスト計画・将来の変更への耐性という
形で反映。

9ラウンド共通の総括: architect役自身がr8総括で述べた「イディオム
（判定式）は流用してよいが、その状態のライフサイクル（誰がいつ、
どの範囲をクリアするか）まで一緒に借りてはならない」という教訓が、
まさにその同じレビューが生んだ設計に対して次のラウンドで検証された
形になった。最終的な解は、新しい状態機械を作らず・既存の別の状態機械
から借りるのでもなく、**役割代入自身が既に持つhold-state（ADR-141
決定2/本ADR決定4）をそのまま再利用する**という、最も保守的な選択
だった——fresh press検出という概念自体が、実は役割代入のhold-state
管理と本質的に同じものだったことに、8ラウンドを経てようやく気づいた
形である。

### レビュー指摘との対応表（r9→r10、10ラウンド目）

**両エージェントとも新規Blockerゼロと判定した初めてのラウンド。**
r0〜r9の10ラウンドを通じて設計の骨格（決定2のアーキテクチャ転換・
fresh press時点でのラッチ化・情報源の一本化）はr9で安定し、r10で
指摘されたのはいずれも記述の訂正・補強のみだった。

architect役: NM17(フォールバックの正当化がr9で「不要」と誤って
宣言されていた)→対で来ないinjectedによる`was_down`stuckへの保険
として正当化を訂正、NM18(ADR-141決定1とADR-142決定B7がinjected
イベントでの`{KEY}_WAS_DOWN`更新有無について食い違う)→本ADRは
「injectedでは更新しない」に依存すると明記しADR-141側への申し送りと
する、Minor1〜3・Nit1→フィールド契約の明確化・フォールバック発火時の
正しさの論証追加・依存値一覧表への`event_eligible`行追加・「一致
させた」から「一致が構造的に従う」への言い換え。
premortem役: R9-M1(決定9-3が却下した出自フィールドとの性質の違いが
未記載)→キーライフサイクルの事実である旨を明記して区別、R9-m1
（`clear_hook_latches_for_app_disable`のLeave時にも同型の再確定が
起こりうる）→既知の制限に追記、R9-m2（`SCAN_KANA_WAS_DOWN`由来の値に
現時点で消費者が無い）→フィールドdocに明記、R9-m3（guardテスト・
journal serde互換への波及）→決定8のテスト計画に追記。

10ラウンド共通の総括: r9でBlockerがゼロになった後もr10で新たな
Major指摘（NM17・NM18）が出たことは、「Blockerが無いこと」と「記述が
実装者を正しく導けること」が別の基準であることを示している。特に
NM17（前ラウンドで入れた仕掛けの正当化が、別の変更のついでに書き
換えられて意図を失う）は、r0〜r9で繰り返し観測されたパターン
（「改訂が新Blockerを生む」）の記述版といえる——コードだけでなく
ADR自身の文章も、変更のたびに「なぜこの仕掛けが必要か」の一貫性を
検証する必要がある。

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

### 決定2（r6で全面整理——r2〜r5の変遷はステータス節・変更履歴を参照）: toがかなスロットになる場合、fresh press時点でSuppress/Allowを確定し、以後の同一押下ではその決定を踏襲する

**中核の設計判断**: 安全な3キーのいずれかがかなスロットへ代入される場合
（例:「変換→かな」）、実際のIME ON操作は`kp_stage_shadow_ime_toggle`
（`key_pipeline.rs:270`付近、`plan()`〈`:392`〉より**前**に呼ばれる）が
既に行っている——役割代入で書き換えられたvk（`VK_DBE_HIRAGANA`）は
`build_raw_key_event`の`classify_ime_relevance(vk)`（代入後vk基準）に
よって`ImeKeyKind::Activate`→`ShadowImeAction::TurnOn`として分類され、
`plan()`の判定を待たずにactuationが発火する。これはADR-141の「vkを
書き換えれば以後の全パイプラインがそれだけを見る」という設計の帰結で
あり、決定1がこれを一般化した時点で既に成立していた（NB4、r3で判明）。

したがって決定2が実際に決めるべきことは1点のみである: **役割代入由来の
`VK_DBE_HIRAGANA`が、既に発火済みのactuationに加えて、`reinject()`
経由でOSへも重複して送出されるのを止める**こと。

**actuation発火の判定（NB6/R5-M1/R5-M2、r6で確定）**: 「既存の仕組みが
実際にIMEをONにするか」は、新しいスナップショット機構を作らず、
`kp_stage_shadow_ime_toggle`が**既に**返している情報に寄せる。

```
actuation_will_fire = shadow_toggled || delegate_will_turn_on
```

- `shadow_toggled`は`kp_stage_shadow_ime_toggle`の既存の戻り値
  （「IME ON/OFFが変化したらtrueを返す」、`key_pipeline.rs:1001-1002`）
  であり、既に`plan()`の引数として渡っている（`:390-394`）。この値は
  同関数が持つ4つの早期return（KeyUp `:1009-1011`、`event.injected`
  `:1116-1123`、`intent_kind`なし`:1086-1088`、`IntentWitness`なし
  `:1143`/`:1151`）とno-op分岐（`:1180`）を**すべて内包**しており、
  r4〜r5が独自に持ち込んだ`ime_open_before`スナップショットより厳密
  （r5の`!ime_open_before`は`is_japanese_ime()`ゲート等の早期return
  経路を区別できず、actuationが起きていないのにSuppress判定してしまう
  ケースがあった）。同じ判定を`plan()`側で再計算しない、という
  `DbeModeKeyContext::is_configured_thumb_key`のdoc（`transport.rs:
  65-69`）の規律にも合致する。
- `delegate_will_turn_on`は`delegate_owned`（`:1074-1075`）が真の場合に
  限り、`turn_on_direction`（`key_pipeline.rs:1188-1200`、既存の
  no-op分岐内で`hiragana_delegate_to_open_axis()`/`katakana_delegate_
  to_open_axis()`の`Option<ShadowImeAction>`を引いて`.unwrap_or(action)`
  している既存計算）が`ShadowImeAction::TurnOn`と一致するかで決める。
  **単純な`is_some()`（armed判定）ではない**——同関数の既存コメント
  （`:1181-1187`、/code-reviewで既に指摘済み）が警告するとおり、
  delegateの配線先はTurnOff/Toggleでもありうるため、armed判定だけでは
  IMEをONにしないdelegateでもSuppressしてしまう（r5の欠陥）。

  **r7訂正（architect役NM14指摘）**: `delegate_will_turn_on`を新しい
  判定式として独自に書いてはならない。`delegate_owned`が真の場合、
  `turn_on_direction`は**既に**（no-op分岐の中で）計算済みであり
  （`delegate_owned`が真なら必ずno-op分岐に入るため、この計算は
  必ず走っている）、これを流用しないと`DbeModeKeyContext::
  is_configured_thumb_key`のdocが禁じる二重管理になる。`turn_on_
  direction`を関数の戻り値に載せ、`delegate_will_turn_on = (delegate_
  owned && turn_on_direction == ShadowImeAction::TurnOn)`という形で
  呼び出し元が導出する。

`kp_stage_shadow_ime_toggle`の戻り値は`shadow_toggled`と
`turn_on_direction`（`delegate_owned`が偽の場合は無視してよい）の2値
（r4〜r5が提案した3値の`ShadowToggleOutcome`構造体は不要——
`shadow_toggled`を再利用するため`ime_open_before`というフィールド自体
が要らなくなった）を運べる形に変更する。

**fresh press時点での確定とラッチ（NB8/NB9/NM12、r6で新設）**:
r4〜r5は、Down/Up/auto-repeatのそれぞれで動的条件を毎回再評価しようと
して、2種類の欠陥を生んだ——(a) KeyUpでの再評価は`kp_stage_shadow_ime_
toggle`がKeyUpで即returnするため`shadow_toggled`相当が常に「変化なし」
になり、Down/Upが常に非対称になる（NB8）。(b) auto-repeat KeyDownでの
再評価は、1打目のactuationでbeliefが既に開いているため2打目以降で
`actuation_will_fire`が常に偽になり、押しっぱなしの間ずっと生の0xF2が
OSへ流れ続ける（NB9、決定6が許容する親指キー構成では特に深刻）。

**決定**: 動的条件の評価は**fresh press**（新規押下の瞬間、ADR-141の
`engine_enabled`規律と同一）**でのみ**行い、判定結果（Suppress/Allow）
を`plan()`呼び出し元（メインスレッドの`kp_run_inner`）に閉じた単一の
`Option<bool>`ラッチへ格納する。全単射（決定3）により、ある瞬間に
「安全な3キー→かな」方向で0xF2を生成しうる物理キーは高々1つなので、
単一の値で足りる。auto-repeat KeyDownとそれに対応するKeyUpは、この
ラッチの値をそのまま踏襲し、動的条件を再評価しない。0xF2のKeyUp処理後
にラッチをクリアする（他vkのイベントはこのラッチを読みも書きも
クリアもしない）。

**fresh pressの検出方式（r7訂正、architect役NB10・premortem役R6-M1
指摘への対応）**: r6は「`was_down`がfalseからtrueへ遷移する瞬間」と
書いていたが、`was_down`はhook.rs（フックスレッド）側の状態であり、
メインスレッドの`kp_run_inner`からは参照できない（`RawKeyEvent`
にもrepeat/freshを示すフィールドは無い、`src/types.rs:190-215`）。
「ラッチの`None`/`Some`自体からfreshnessを推論する」という代替案も
検討したが、これは**KeyUpが`kp_run_inner`に到達しない3経路**——
`FOCUS_APP_DISABLED`早期return（`hook.rs:980-982`）、overflowラッチ
（`hook.rs:1100-1102`）、`ProduceResult::Overflow`（`hook.rs:1203-1205`）
——でラッチが`Some(...)`のまま取り残された場合、次のfresh pressを
auto-repeatと誤認して古い決定を踏襲してしまう（`Some(true)`残留なら
BUG-10の食い逃げが、`Some(false)`残留なら二重配送が再発する）。
これらの経路はいずれもフックスレッドの関数であり、メインスレッドの
ラッチに手が届かない（届かせようとするとr1の`KANA_DOWN_WAS_ALLOWED`
と同じクロススレッド問題に戻る）。

**決定**: fresh pressの判定は、ラッチの状態から推論せず、hook.rsが
明示的なビットとして運ぶ。

**計算位置の訂正（r8、architect役NB11・premortem役R7-M1指摘への対応）**:
r7は「決定1の挿入点（`hook.rs:1135`以降）で`is_keydown && !was_down`
を判定する」としていたが、これは常にfalseになる——挿入点で汎用的に
参照できる押下状態は`PHYSICAL_KEY_STATE`（定義`hook.rs:186`）と
`PHYSICAL_KEY_DOWN_AT_MS`のみで、いずれも**挿入点より前**（`:951-968`）
で**このKeyDown自身により既に更新済み**である。これはr4のNB7
（`effective_open()`のライブ値読み）と同じ失敗モードで、このリポジトリ
で同じ罠が3度目（`half_width_alnum_toggle_active`、NB7、今回）になる
ところだった。

**r8の解決策とそこに残った欠陥（NB12、r9で訂正）**: r8は`is_fresh_
press`を`PHYSICAL_KEY_DOWN_AT_MS`の更新ブロック内で計算する設計に
したが、これは新たに「fresh pressの情報源が2つになる」という問題を
生んでいた。`PHYSICAL_KEY_DOWN_AT_MS`は**長押し時間計測**のための
独自の状態機械で、そのクリア規律（`reset_physical_key_state`で全256
スロット、`clear_hook_latches_for_app_disable`のLeave時はCtrl/Shiftの
6スロットのみ、BUG-78対策）は、役割代入自身の`{KEY}_WAS_DOWN`/
`SCAN_KANA_WAS_DOWN`のクリア規律（ADR-141決定7の3箇所＋本ADR決定5の
2箇所）と**一致するのは`reset_physical_key_state`だけ**だった。
`disable_apps`のEnter遷移やoverflowラッチ経路では、役割代入側の
`{KEY}_WAS_DOWN`はクリアされる（fresh press再確定）のに
`PHYSICAL_KEY_DOWN_AT_MS`は維持される（auto-repeat判定のまま）という
食い違いが起こり、NB10が解決したはずの3経路のうち2つで自己修復性が
再び失われていた。

**決定（r9）**: fresh pressの情報源を1つに統一する。fresh press判定は
`PHYSICAL_KEY_DOWN_AT_MS`ではなく、**役割代入自身の`{KEY}_WAS_DOWN`
（安全な3キー、ADR-141決定2）/`SCAN_KANA_WAS_DOWN`（かなスロット、
本ADR決定4）**から計算する。`decide_role_substitution`（ADR-141決定2の
判定関数）は`was_down: bool`を引数に取り更新後の状態を返す設計なので、
決定1の挿入点（`hook.rs:1135`以降）でこの関数に**渡すのとまったく
同じ`was_down`**（更新前の値）を使えば、挿入点で計算してもr7のNB11の
ような順序汚染を受けない——r7がNB11で見落としていたのは、汎用の
`PHYSICAL_KEY_STATE`/`PHYSICAL_KEY_DOWN_AT_MS`だけを検討し、役割代入
専用の`{KEY}_WAS_DOWN`という第3の情報源を見ていなかったことだった。

この判定は`is_keydown && !was_down`という式そのもので、役割代入対象の
fromキーまたは`to`側で使われる安全な3キーの物理キーについてのみ、
この`was_down`を参照する。情報源をこの1つに統一することで、クリア箇所
は（人手で揃えるのではなく）**定義から自動的に**ADR-141決定7の3箇所
＋本ADR決定5の2箇所（計5箇所）に一致する（r10訂正、architect役Nit1
指摘: r8時点の失敗〈`PHYSICAL_KEY_DOWN_AT_MS`という独立した状態を
手で揃えようとして食い違った〉との本質的な違いは「一致させた」では
なく「一致が構造的に保証される」ことにある。将来の変更提案——例えば
性能のために専用キャッシュを別途持つ——に対する歯止めとして、この
違いを明記する）。

**injectedイベントでの`{KEY}_WAS_DOWN`更新有無への依存（r10追加、
architect役NM18指摘）**: `role_substitution_fresh_press`が`{KEY}_
WAS_DOWN`から導かれるようになったことで、この状態に新しい消費者が
できた。したがって「injectedイベントが`{KEY}_WAS_DOWN`を更新するか」
が本ADRの正しさにとって重要になったが、親ADR2本の記述が食い違って
いる——ADR-141決定1は「役割代入の適用とhold-stateの更新は、いずれも
`!is_injected`ガード内で行う」（injectedでは更新しない）とする一方、
ADR-142決定B7は「hook.rs側は`!is_injected`の早期returnゲートを持たず、
安全な3キーのイベントを無条件に`decide_role_substitution`へ渡す」
（素直に読むと`was_down`の更新も無条件になる）としている。B7の読み方
を採ると、ADR-141決定1が名指しで防いだstuck-`was_down`が復活する
（上記フォールバックが`None`の場合のみを救うため、完全な保険には
ならない）。**決定**: 本ADRは「injectedイベントは`{KEY}_WAS_DOWN`を
更新しない」という前提に依存すると明記し、ADR-141/142側のこの不一致
を解消対象として申し送る（ADR-141決定7の3箇所への波及をADR-141本体へ
参照ポインタとして追加したのと同じ手当て）。既存の`apply_alt_
impersonation`（`hook.rs:86-99`）が`ALT_L_WAS_DOWN.store(is_keydown,
...)`を無条件に行っている実装は、素直に真似るとB7の読み方になる
ため、実装者が誤りやすい箇所として注意を促す。

**フィールドの型（r9訂正、NB11(a)の再解決）**: 汎用の`is_fresh_press:
bool`ではなく、`role_substitution_fresh_press: Option<bool>`とする
（`None`＝役割代入の対象外〈安全な3キー・かなスロットのいずれでも
ない、またはinjected〉のイベント、`Some(true)`＝fresh press、
`Some(false)`＝auto-repeat）。安全な3キー・かなスロットの4オブジェクト
のうち、config上ルールが明示されていない恒等写像のキーについても
`{KEY}_WAS_DOWN`自体は存在するため`Some`になる（r10追加、architect役
Minor1指摘: フィールドの契約として明記する）。`bool`固定にすると
「fresh pressの'A'が非freshと記録される」という嘘のフィールドをcoreの
公開構造体に持ち込むことになる（`feedback_dont_provision_ahead_
without_consumer_logic`が警告する「消費ロジックの無い予備フィールド
の先回り」と同型のリスク、premortem役R7-M2・R8-m1が指摘した
「injectedイベントでは常にfalse」という不正確さも`None`で自然に表現
できる）。`key_classification`/`ime_relevance`/`physical_pos`/
`modifier_key`と同じ「プラットフォーム層が事前に決定する」フィールド
群の一員として自然に収まり、VKのマジックナンバーではないためADR-019
にも抵触しない（`docs/layer-boundaries.md`のカテゴリでいえば、これら
既存フィールドと同じ「プラットフォーム層が事前分類してcoreへ渡す情報」
に該当する）。

**決定9-3との区別（r10追加、premortem役R9-M1指摘）**: 決定9-3は
`RawKeyEvent`に「役割代入由来か」という**出自**（provenance）を示す
フィールドを追加する案を、スコープクリープ・層境界を理由に却下した。
`role_substitution_fresh_press`はこれとは性質が異なる——出自ではなく
「このKeyDownがauto-repeatかどうか」という**キーライフサイクルの
事実**であり、`key_classification`/`physical_pos`と同じ「platformが
事前に確定してcoreへ渡す情報」の系列に属する。両者は似て見えるが
別物であることを明記する。

**vkキーであることの限界が消えた（r9、NM16は解消）**: r8時点では
`PHYSICAL_KEY_DOWN_AT_MS`がvkインデックスであるため、物理かなキー
自身（vkが0xF0/0xF1/0xF2の間で揺れる）については信頼できないという
制約があった。r9の情報源統一により、かなスロットについては
`SCAN_KANA_WAS_DOWN`（scanベース、決定1の設計原則をそのまま継承）
から計算されるため、この制約は不要になる——`role_substitution_fresh_
press`は安全な3キー・かなスロットのどちらについても、それぞれの
専用hold-stateの規律にそのまま従う。

この設計により、**fresh pressでは必ずラッチを上書きする**という
自己修復性の論拠は、「情報源が単一であり、その単一の情報源が既に
5箇所で正しくクリアされている」という、より強い形で成立する
（r8時点の「取りこぼしたら次のfresh pressで上書きされる」という
自己修復の論証は、そもそも取りこぼし自体が起こらなくなったため
不要になった）。

**ラッチを参照・更新してよいイベントの限定（r7追加、premortem役R6-M2
指摘への対応。2026-09-06訂正、architect役Minor指摘）**: ラッチの
**書き込み・クリア**は、静的3条件（`event.vk_code == VK_DBE_HIRAGANA
&& !event.injected && event.scan_code != SCAN_KANA`）を満たすイベント
に限る。満たさないイベント（物理かなキー自身の押下〈`scan_code ==
SCAN_KANA`〉、他プロセスrelayのinjected 0xF2など）はラッチを書き込み・
クリアせず、既存判定へ進む。この限定が無いと、取り残されたstaleな
ラッチが無関係なイベントに適用され、物理かなキーでは本来
`is_tsf_mode && f2_warmup_owned`で判定すべきものが無条件Suppressに
なり（MS-IME環境でBUG-10の食い逃げ）、injected relayではADR-119回帰が
別経路で復活する。**訂正**: **読み取り**については、KeyUpに限り
injectedなイベントも許す（下記「injected KeyUpがラッチを迂回する
ケースへの対処」参照——役割代入で確定した押下の解放に対応するための
決定2のKeyUp専用Suppress分岐）。R6-M2が懸念したstaleラッチの誤適用は、
書き込み・クリアを従来どおり非注入に限定していれば維持される
（読み取りだけをKeyUpに限り開いても、ラッチの値自体は非注入イベント
でしか変化しないため、無関係なイベントに古い値が誤って「新規に
書き込まれる」ことは起きない）。

**既知の制限（architect役指摘、2026-09-06追加）**: 読み取りを
injectedへ開いた副作用として、ローカルで代入先キーを押している最中
（ラッチが`Some(true)`）に、リレーツールが（awaseの変換を経由しない）
生の0xF2 KeyUpを送ってきた場合も、この分岐でSuppressされる。awase
自身が変換したUp（scanが物理変換キーの値等）とリレーの生Up（scan=0）
は理屈上区別できるが、新たな判別子を追加するほどの実害ではないと
判断し、ADR-119「解釈しない入力は消費しない」に対する狭い例外として
ここに記録する（発生窓が「ローカルの押下中」に限られるため、
リモート側のキーが恒久的に無反応になるADR-119の被害像には至らない）。

**injected KeyUpがラッチを迂回するケースへの対処（2026-09-06追加、
architect役Major指摘・premortem役の対称な解法提案）**: 静的3条件は
`!event.injected`を含むため、次の経路でラッチが迂回されうる: (1)
物理変換キーのfresh press（eligible）→ラッチがSuppress（0xF2の
DownはOSへ届かない）を確定。(2) その後リレーツール等が同じ変換キーの
injected KeyUpを送る→ADR-141決定1の訂正（適用は無条件）により変換
自体は行われ`vk=0xF2・injected=true`のイベントになるが、静的3条件の
`!event.injected`を満たさないためラッチを読まず既存のF2分岐へフォール
スルーする→MS-IME・非TSF環境では`Allow`となり、**対応するDownを一度も
持たない0xF2のKeyUpがOSへ送出される**。

この経路は、ADR-141決定7が既に持つ「`{KEY}_CONFIRMED_TARGET ==
Some(VK_DBE_HIRAGANA)`の場合はKeyUp注入を発火させない（Downが一度も
配送されていないため注入する意味が無い）」という例外（本ADR決定4
参照）と類似の理由で解消できるが、**条件は`confirmed_target`ではなく
ラッチの値そのものにする**（2026-09-06、architect役Major指摘で訂正
——`confirmed_target == Some(VK_DBE_HIRAGANA)`は「Downが常にSuppress
される」というr3時点の前提に乗っていたが、決定2のr4以降の設計では
`actuation_will_fire`が偽の場合〈belief既にOPEN側でのBUG-10救済、
delegateがturn-on以外〉にラッチが`Some(false)`＝Allowへフォールスルー
し、**その場合はOSが実際に0xF2のDownを受け取っている**。この場合に
injected Upの変換を止めると、OS側の0xF2が解放されないまま残り、対応
する物理Upが上流ガードで失われると恒久的にstuckする）。

**置き場所の訂正（2026-09-06、premortem役R11-M1指摘）**: 当初「ラッチが
`Some(true)`の間はhook.rs挿入点でのvk変換自体を行わない」という案を
検討したが、**これは実装不可能だった**——vk変換はADR-141決定1の挿入点
（`hook.rs:1135`、フックスレッド）でのみ行われるのに対し、ラッチは
`plan()`の呼び出し元（メインスレッドの`kp_run_inner`）に閉じた値で
あり、フックスレッドから直接参照できない。「参照できない経路では
安全側デフォルト（変換する）を適用する」を機械的に当てはめると、
変換は常に行われることになり、この例外自体がデッドコードになって
orphan 0xF2 KeyUpの問題が未解決のまま残ってしまう。

**決定（訂正版）**: 例外はhook.rs側の変換を止める形ではなく、
**`plan()`側にKeyUp専用のSuppress分岐を追加する**形で実装する。
orphan 0xF2 KeyUpが実際にOSへ届くのは`plan()`が`Allow`を返し
`reinject()`が走る一点だけなので、そこをSuppressすればよい——変換
自体（ADR-141決定1の「適用は無条件」）はhook.rs側で変更せず、
変換後のvk=0xF2としてメインスレッドに届いた**後**、`plan()`が以下を
追加条件としてSuppressを返す:

```
（決定2の追加分岐、KeyUpのみ）
event.vk_code == VK_DBE_HIRAGANA
  && event.injected
  && event.scan_code != SCAN_KANA
  && ラッチ == Some(true)        // Downがこの押下でSuppressされていた
  → Suppress
```

ラッチが`Some(false)`（Downを配送済み）または`None`（不明）の場合は
この分岐に入らず、既存のF2判定（`Allow`）へフォールスルーし、Upを
通常どおり届ける——いずれも「stuck Downを作らない」安全側である。
この分岐は`plan()`（メインスレッド）で完結するためラッチを問題なく
参照でき、決定1の静的3条件（`!event.injected`）とは別の、KeyUp限定の
追加分岐として実装する。ADR-119との関係: ここでSuppressするのは
「Downを一度もOSへ配送していない押下のUp」に限られるため、ADR-119
（issue #136）が守ろうとした「リモートのかなキーが完全に無反応になる」
ケースには当たらない——リレーツール側から見ればDown自体が最初から
awase側でSuppressされていた押下であり、Upだけを機能させる意味が無い。

**危険度の優先順位（premortem役R11-m1指摘、2026-09-06追加）**:
本ADR・ADR-141を通じて、stuck Down（危険——OSにキーが押しっぱなしの
まま残る）とorphan/重複Up（無害側——BUG-46のKANJI系Up常時Suppress・
`deferred_vks`残留inertと同型の既に受容された性質）のどちらかを選ぶ
局面では、常に**「stuck Downを避けることを優先し、orphan/重複Upは
許容する」**という優先順位に従う。上記の`plan()`分岐・決定4のKeyUp
注入抑止・ADR-141決定1の重複KeyUp許容は、いずれもこの単一の優先順位
から導かれる（個別に矛盾して見える場合は、この優先順位に立ち返って
判断する）。

**ラッチ`None`の扱い（premortem役R11-m2指摘）**: 決定4のKeyUp注入
抑止・上記`plan()`分岐とも、ラッチが`Some(true)`である場合にのみ
抑止/Suppressし、それ以外（`Some(false)`・`None`）はすべて通常どおり
注入/Allowする、という単一の規則で統一する（「`Some(true)`以外は
すべて安全側」という形が最も誤読が少ない）。

**ラッチのスレッド安全性**: `plan()`の呼び出しはメインスレッドの
`kp_run_inner`に閉じており、このラッチもそこに閉じた値（`plan()`の
内部状態ではなく、呼び出し元`kp_run_inner`が保持し引数として渡す値）
として実装する。r1の`KANA_DOWN_WAS_ALLOWED`がフックスレッド（hook.rs
のhold-state）とメインスレッド（`plan()`）を跨いで破綻したのとは構造的
に別物——今回は書き込みと読み出しが同一スレッド・同一関数群内で完結
する。`role_substitution_fresh_press`フィールドはhook.rs側で計算
されるが、それを`RawKeyEvent`経由でメインスレッドへ**運ぶだけ**で
あり、hook.rs側の状態をメインスレッドから直接参照するわけではない
（既存の`key_classification`等と同じデータフロー）。

**stale latch取り残しへの残存対応（r7追加、premortem役R6-M3指摘への
対応。r9でNB12対応により論拠を強化）**: `role_substitution_fresh_
press`の情報源を`{KEY}_WAS_DOWN`/`SCAN_KANA_WAS_DOWN`に統一した
（NB12対応）ことにより、この値自体がADR-141決定7の3箇所＋決定5の2箇所
（計5箇所）で常に正しくクリアされる——r8時点で懸念していた「情報源が
別の状態機械のため経路によってクリア規律が食い違う」という問題は
構造的に存在しなくなった。`kp_run_inner`側のSuppress/Allowラッチ自体
はこれら5箇所には含まれない（含める必要が無い——そもそも情報源が
単一になったことで、いつ`Some`のまま取り残されても次の`Some(true)`な
`role_substitution_fresh_press`で確実に上書きされる）。取り残された
ラッチが与えうる実害は「次のfresh press一回分の待ち時間だけ、直前の
（古い）dispositionが誤って適用される」ことに限定される（R6-M2の
限定により影響範囲は0xF2のイベントのみ）。

**フォールバック（r8追加。r10で正当化を訂正、architect役NM17指摘への
対応）**: `role_substitution_fresh_press`を主たる判定に使いつつ、
ラッチが`None`のまま静的3条件を満たすKeyDownを観測した場合も評価点
として扱う（`role_substitution_fresh_press`が`Some(false)`でもラッチ
が`None`なら動的条件を評価し確定させる）。

**r10での訂正**: r9は情報源統一の効果を「r8時点の自己修復の論証は、
そもそも取りこぼし自体が起こらなくなったため不要になった」と書いて
いたが、これは不正確だった——**このフォールバックは依然として必須**
であり、正当化すべき危険が変わっただけである。

**r11での再訂正（2026-09-06、ADR-141決定1/ADR-142決定B7の訂正——
hold-storeのゲートを`event_eligible`にしたこと——を受けた再検証）**:
r10はフォールバックの主たる正当化として「MWB等が物理変換キーのinjected
KeyDownだけを送りKeyUpを送らない場合、`CONVERT_WAS_DOWN`がtrueのまま
残る」というシナリオを挙げていたが、**ADR-141決定1のr-later訂正
（hold-stateへのstoreをinjected/非eligibleでは行わない）により、この
シナリオは構造的に発生しなくなった**——injectedなKeyDownはstoreされない
ため`{KEY}_WAS_DOWN`をtrueにできない。逆方向（物理KeyDown→injectedな
KeyUp→物理KeyUpが後から上流ガードで失われる）というシナリオも検討したが、
安全な3キーのKeyUpが挿入点に届かない経路（`FOCUS_APP_DISABLED`・
overflowラッチ・`ProduceResult::Overflow`・セッションロック）はいずれも
ADR-141決定7-1〜3が既に`was_down`をクリアする（`confirmed_target.
is_some()`を条件にしており、injected Upがstoreをスキップしていても
`Some`のまま残るためクリアは正常に発火する）ため、こちらもstuckしない。
したがって**injected由来のhold-state stuckは、fresh press判定の観点では
両方向とも構造的に消滅した**。

**フォールバックが今なお必要な唯一の確定した理由は
`ProduceResult::Overflow`（`hook.rs:1203-1205`）である**: 役割代入の
挿入点（`hook.rs:1135`）は`produce()`（`hook.rs:1195`）より前にあるため、
fresh pressのKeyDownでhold-stateへのstoreが終わった**後に**`produce()`が
overflowでイベント自体を破棄することがありうる。この場合、状態
（`{KEY}_WAS_DOWN=true`）だけが進み、対応するイベントは`kp_run_inner`に
一度も届かずラッチは`None`のまま残る。以後のauto-repeat KeyDownは
`role_substitution_fresh_press=Some(false)`になり、ラッチが`None`である
ことを条件とする本フォールバックだけが評価点を作る。対照的に
`hook.rs:1100`（overflowラッチの早期return）はそもそも挿入点に到達
しないため`{KEY}_WAS_DOWN`が更新されず、次に到達したイベントは正しく
freshになる——こちらはフォールバックを必要としない。

この経路でフォールバックが正しい値を確定できる理由も明記する: fresh
pressのKeyDownが`:1203`のoverflowで`kp_run_inner`に届かなかった場合、
そのイベントは`kp_stage_shadow_ime_toggle`も走らせていないため
**actuationも同時に失われている**——beliefは変化しないままである。
したがって次のauto-repeat KeyDownが届いた時点で
`kp_stage_shadow_ime_toggle`が初めてbeliefを変え、`shadow_toggled`が
真になる。フォールバック発火時点の動的条件（`actuation_will_fire`）は、
本来のfresh press時点で評価していた場合と同じ値になる。

**正当化の書き方についての教訓（premortem役指摘）**: 上記のとおり
「特定シナリオの列挙」で正当化すると、そのシナリオが後の設計変更で
消えたときに再び書き直しが必要になり、最悪「不要」と誤判定され削除
される（r9→r10で一度、r10→r11で二度目に発生した）。そのためフォール
バック自体の存在理由は「ラッチが未確定のまま押下ライフサイクルが進行
している状態への防御的な受け皿」という一般形で理解し、`ProduceResult::
Overflow`は現時点で確認できている具体例の1つと位置づける。ADR-141
決定7-3の発火条件（overflowラッチ解除時、生の物理KeyUpの代わりに
代入後vkのKeyUpを注入する処理が、KeyUp到達時のみ発火するのか、pending
な`confirmed_target`がある限りKeyDownでも発火しうるのかが本文からは
確定しない）次第では、この経路以外にも評価点が必要になる可能性が残る
——フォールバックの条件自体（ラッチ`None`のKeyDownを評価点にする）は
このいずれの解釈でも安全側に働くため、変更しない。

**確定する条件（fresh pressビットが真、またはラッチが`None`のKeyDown
時点でのみ評価）**:
```
event.vk_code == VK_DBE_HIRAGANA
  && !event.injected
  && kana_role_active
  && event.scan_code != SCAN_KANA
  && actuation_will_fire
```
真ならばラッチに`Some(true)`（Suppress）を、偽ならば`Some(false)`
（Allow、既存の`is_tsf_mode && f2_warmup_owned`判定へフォールスルー）
を格納する。auto-repeat KeyDown（`role_substitution_fresh_press`が
`Some(false)`かつラッチが`Some`）とKeyUpは、この確定済みラッチを
そのまま踏襲し再評価しない。
**非注入の**0xF2のKeyUpを観測した時点で、`plan()`の戻り値や早期return
の分岐に関わらず`kp_run_inner`側でラッチをクリアする（r8追加、
architect役Minor2指摘: ラッチのライフサイクルを`plan()`内部の分岐
——InputRelay早期returnを含む——に依存させない。**2026-09-06訂正
（architect役Major指摘）**: クリアの条件に`!event.injected`を明記
する——後述のinjected KeyUp用Suppress分岐がラッチを**読む**ように
なったため、クリアまでinjectedイベントに開くと、injectedなUpが
先にラッチをクリアしてしまい、直後の物理Upが`None`を読んでフォール
スルーし、対応するDownを持たない0xF2のKeyUpがOSへ出る——今回の
修正で防ごうとした問題がクリア経路から別途復活する。読み取りのみ
KeyUpに限りinjectedへ開き、書き込み・クリアは従来どおり非注入に
限定する）。KeyUp到達時点でラッチが`None`
（異常系）の場合はSuppress側を安全側とする（r7追加、premortem役
R6-m1指摘: BUG-46の「KANJI系KeyUpは常にSuppress」という既存の規律、
`transport.rs:118-125`、に揃える。`None`のままAllowへフォールスルー
すると、対応するDownを持たない0xF2のKeyUpがOSへ送出されうる）。

**config reload時のラッチの扱い（r7追加、premortem役R6-m2指摘）**:
`kana_role_active`が押下中のreloadで反転しても、ラッチはfresh press
時点で確定した値を保持し続ける——reloadの瞬間にラッチをクリアしては
ならない（クリアすると押下中のDown/Upが非対称になる）。ADR-141決定3の
`confirmed_target`規律と同じ立場である。

**評価順序の明示（r7追加、architect役NM13指摘への対応）**: 本ラッチに
よるSuppress判定は、`transport.rs`のF2専用分岐（`:276`）を拡張する形で
実装し、`transport.rs:260`のInputRelay早期returnより**後**に評価する
（ADR-119/issue #136がこの順序自体をレビューで発見・修正した経緯
——`:246-259`のコメント「F2分岐より先に判定する」——を尊重し、
本ADRのために変更しない）。したがって、fresh press時点で非InputRelay
プロファイルだったためラッチがSuppress側に確定した押下でも、Up時点で
フォーカスがInputRelayウィンドウへ移っていれば、InputRelayの早期return
が先に評価され`Allow`が返る——対応するDownを持たない0xF2のKeyUpがOSへ
送出されうる。この経路は「押下中にフォーカスがInputRelayウィンドウへ
移る」という狭い条件を要するため、ADR-141決定4末尾がAltセンチネル
構成の既存衝突に対して取った立場（悪化させないが解消もしない）と同型
に整理し、本ADRでは解決を試みない既知の狭い制限として記録するに留める
（ADR-119の順序を優先する判断の代償として明示する）。**r8追加
（architect役Minor3指摘、r5 Minor4の留保を再併記）**: この経路で
OSへ送出されうる0xF2のKeyUpは、BUG-52の無条件Suppressガードが
「素通しすると実IME（MS-IME）がWindows標準仕様どおり能動的にネイティブ
効果（英数/カタカナ/半角/全角への切替）を適用してしまう」として遮断
している当のキー種別であり、**KeyUp単独が無害（inert）である保証は
ADR本文にも実機記録にも無い**。「狭い条件なので解決を試みない」という
判断自体は妥当だが、その代償に残る不確実性として記録する。

**`!event.injected`が無いと**（r2の欠陥）: `transport.rs`の評価順序は
InputRelay早期return（`:260`）→F2分岐（`:276`）→injected早期return
（`:302`以降）であり、F2分岐はinjectedチェックより**前**にある。他
プロセス（Mouse Without Borders等）がSendInputでrelayした0xF2は
`wScan: 0`で届くため`scan_code != SCAN_KANA`が真になり、無条件で
Suppressされてしまう。injectedイベントはBUG-14ガード（`kp_stage_
shadow_ime_toggle`）により`shadow_toggled`へ昇格しないためawase自身も
actuateしない——結果、ADR-119（issue #136）決定1が防いだ「解釈しない
入力を消費する」＝リモート側のかなキーが完全に無反応になる「二重の
空振り」が再導入される。

**`kana_role_active`が無いと**（r2の欠陥）: この条件は`plan()`の引数
からのみ決まる必要がある純粋関数の制約に従い、`DbeModeKeyContext`
（`transport.rs:63-69`の`is_configured_thumb_key`と同型の追加）の
フィールドとして追加する。「かなスロットが関与する役割代入ルールが
現在有効か」を示す`AtomicBool`で、`CACHED_SWALLOW_ALT_KANA_MODE_
SWITCH`（`hook.rs:457`、setterは`:567`）と同型に、configロード時と
`app/mod.rs::reload_config()`の両方で更新する（ADR-142決定B1が定めた
「起動時とreload時の両方から呼ぶ」検証と同じ配線に相乗りする）。
`From<DbeModeKeyPolicy>`実装の既定値を`false`にすれば既存の回帰
テストは無改修で通る。この条件が無いと、`[[key_role]]`を1つも設定
していないユーザーにもこのSuppressが適用され、スコープ節の「代入なし
の場合は既存挙動を一切変更しない」という宣言に反し、BUG-10（食い逃げ、
後述）の回帰面を機能未使用のユーザーにまで広げてしまう。

**SSOTの要件**: `kana_role_active`は独立した経路で計算してはならない。
hook.rs側の役割代入テーブル（決定1のfrom判定・決定3の全単射検証済み
ルール集合）と`kana_role_active`が別々に計算されると、両者が乖離した
場合（hookは代入する側・`kana_role_active`はfalse側、という最悪の
組み合わせ）に「代入後の0xF2が`Allow`されて`reinject()`でOSへ出る」
という、本ADRが解消したはずの生の0xF2直接送出が復活する
（`awase-settings::is_muhenkan_thumb_key`のdocが警告するissue #99型の
二重管理と同型のリスク）。`kana_role_active`は、ADR-142決定B1が新設
する`state/key_role.rs`の解決済みテーブル（hook.rs側が代入判定に使う
のと同一のSSOT）から**導出する**ことを要件とし、独立したbool値として
別経路で計算しない。決定5のオプトアウト無効化（`swallow_alt_kana_
input_method_switch=false`時のルール集合全体無効化）も、この同じSSOT
を通じて`kana_role_active`に反映される。

**Alt押下時の安全性の論拠**: `GjiDirectStrategy`/`MsImeDirectStrategy`
がIME ON操作として実際に送出するVKは`VK_IME_ON`(0x16)、
`KanjiToggleStrategy`は`VK_KANJI`(0x19)であり、**いずれも`VK_DBE_*`
一族ではない**（MS-IMEのON操作は2026-08-06・BUG-50対応で
`VK_DBE_HIRAGANA`から`VK_IME_ON`単発へ変更済み、`ime_controller.rs:
190-203`）。BUG-61が問題にしているのはAlt+`VK_DBE_*`一族という特定の
組み合わせがOSレベルの入力方式切替ショートカットとして解釈されること
であり、`VK_IME_ON`/`VK_KANJI`はこのショートカットの対象ではないため、
Alt押下中にactuationが発火してもBUG-61のリスクには該当しない
（`ime_mode_key_injection_blocked_by_modifier()`を経由するから安全、
という論拠は事実誤認だった——同関数の呼び出し箇所は半角英数トグル
復元の2箇所のみで、actuation本経路には存在しない）。

**この論拠が成立しなくなる条件**: 将来`key_sequence_policy`の送出VK
選択がIME ON操作用に`VK_DBE_*`一族へ戻される変更が入った場合、この
安全性の論拠は崩れる。そのような変更を行う際は本ADRの前提が崩れる
ことを明記し、Alt押下時のガードを別途追加する必要がある。

**既知の制限・残課題**:
1. **IME OFF方向・charset軸（カタカナ/半角英数→ひらがな）の復帰は
   再現しない**: 物理かなキーは、IMEが既にひらがな状態にある時に
   押されると実機的に「かな入力を終了する」効果を持つことがあるが、
   決定2の既知の制限2（`kp_restore_hiragana_for_suppressed_mode_key`
   の除外）により、代入先キーは**open軸（IME ON）のみ**を操作し、
   `VK_IME_ON`はcharset軸には一切触れない（`ime_controller.rs:
   224-228`）。ADR-100決定2以降の「eager warmupがopen軸のみ」という
   片肺化（ADR-137 M-6の真因）と同型の制限であり、意図的に選択した
   トレードオフである。将来charset軸まで再現したい場合は、GJI/MS-IME
   でexit実装を共有しない（BUG-25/ADR-107）という制約を満たす専用
   経路を新たに設計する必要がある。
2. **`kp_restore_hiragana_for_suppressed_mode_key`（BUG-116決定2、
   `key_pipeline.rs:81-201`）を役割代入由来のイベントから除外する**:
   同関数のコメント（`:104-108`）が明記する「`physical == Suppress`
   かつ0xF2は`plan()`のF2分岐でのみ成立し、その条件は`is_tsf_mode &&
   f2_warmup_owned`そのもの。したがって`physical == Suppress`である
   ことがGJI戦略としてSuppressされたことの**必要十分条件**である」
   という同値性を、決定2のSuppress条件拡張が壊す。GJI戦略でなくとも
   役割代入由来のSuppressで`physical == Suppress`が成立するため、
   MS-IME環境でもこの関数が発火し`send_gji_half_width_alnum_toggle
   (Exit, ..)`という**GJI専用のscan付きVK_DBE_HIRAGANA注入**が
   MS-IMEセッションへ誤発火する（BUG-25/ADR-107が明記する「GJI/MS-IME
   でexit実装を共有しない」原則への違反、かつBUG-15追補7のかなロック
   トグルハザードの当事者）。**決定**: 同関数の発火条件に
   `event.scan_code == SCAN_KANA`を追加し、役割代入由来のイベント
   （`scan_code != SCAN_KANA`）をこの関数の対象外にする。この除外は
   同関数の**最初の早期return**として、`:90`の`if event.vk_code !=
   VK_DBE_HIRAGANA { return; }`と同じ位置に置く（`:93-103`のKeyUp
   早期return・`kana_mode_restore_key_down`ラッチ解除より**前**——
   後段に置くと代入先キーのKeyUpが物理かなキーの立てたラッチを誤って
   解除しうる）。同関数のコメント（`:104-108`）にこの前提変更を追記
   する。監査対象（`is_configured_thumb_key`・
   `half_width_alnum_toggle_before`・`kana_mode_restore_key_down`・
   `conv_mutation_allowed`・`is_composition_warm()`複合条件・
   `read_kana_lock()`によるABORT）は未解決の疑問6として残す。
3. **近似であることの明記**: `actuation_will_fire`は「beliefが変わる
   （`shadow_toggled`/`delegate_will_turn_on`）」ことの近似であり、
   「実際にIMEがactuateされる」ことの保証ではない。belief変化後の
   実際のactuationは下流（`executor.rs::dispatch_ime_set_open`→
   `ime_controller::apply`）で行われ、ADR-119の`AppImeProfile::
   InputRelay`ゲートをはじめ複数のpreconditionがある
   （`.claude/rules/fix-requires-evidence.md`の「IME actuation合流点」
   参照）。InputRelayは`plan()`自身の早期return（`transport.rs:260`）
   が先に`Allow`を返すため整合するが、それ以外の下流ゲートで
   actuationが落ちる組み合わせが無いかは未検証——belief遷移は起きたが
   実際には誰もIMEをONにしない場合、BUG-10の食い逃げが残る。下流
   ゲートの棚卸しはPhase A実装時の確認項目とする（未解決の疑問6と
   統合）。**r7追加（architect役Minor3指摘）**: ラッチ化により、この
   近似の限界が及ぶ範囲が一段広がった——r5までは動的条件をイベントごと
   に再評価していたため、近似が外れても次の打鍵で状況が変わりうる
   余地があったが、r6のラッチ化により**fresh press時点の1回きりの
   判定が押下期間全体を支配する**。近似が外れた場合、その押下が終わる
   まで訂正されない。この点を踏まえ、下流ゲートの棚卸しの優先度を
   Phase A実装の前提条件（未解決の疑問6）として維持する。
4. **delegate分岐の代入後構成での正しさは未検証**: `hiragana_
   delegate_to_open_axis`は物理かなキーが親指キーである前提で設計・
   検証された機構である。決定6が「意図した挙動」とする構成（代入に
   より物理変換キーが親指キーとして扱われる）で、このdelegateが実際に
   open軸をONにするかは未検証。Phase A実装の実機確認項目に追加する
   （未解決の疑問6と統合）。`delegate_owned`の実体は`mode_key_
   delegate_owns_shadow_toggle(event.vk_code)`であり、親指キーが
   「カタカナ」(0xF1)の構成では`katakana_delegate_to_open_axis`が
   対象になりうるが、決定2が`to`側の合成vkを0xF2に固定しているため
   `delegate_owned`が真になるのは`thumb_vk == 0xF2`の場合のみで、
   カタカナ側は構造的に該当しない。
5. **`to`に`VK_DBE_ROMAN`/`NOROMAN`(0xF5/0xF6)・その他の`VK_DBE_*`
   亜種を割り当てることは引き続き禁止する**: `to`=かなスロットの意味
   は「`VK_DBE_HIRAGANA`固定・条件を満たせば常にSuppress・OSへの
   二重配送を止めるだけで実際のactuationは既存経路が行う」に一本化
   されており、他の`VK_DBE_*`値を`to`として選択させる余地はそもそも
   存在しない。
6. **`deferred_vks`への0xF2残留**: Down側がAllow（BUG-10救済で
   ネイティブ配送に委ねた場合）だった場合、`transport.rs:110-125`の
   `deferred_vks`に0xF2が残留しうる。既存の0xF1/0xF3/0xF4と同じ扱いで
   inertと見込まれるが、`check_keyup_symmetry`への影響が無いかは
   Phase A実装時に確認する。
7. **`suppress_reason`のラベル訂正が必要**: `PhysicalKeyDisposition::
   suppress_reason`（`transport.rs:32-45`）は`event.vk_code ==
   VK_DBE_HIRAGANA`の場合に理由ラベルを常に`"tsf-f2"`とする。役割
   代入由来のSuppressもこのラベルで記録されると、BUG-90調査が前提と
   する「journalの`KeyInput.decision`（意味論的判断）と
   `suppress_reason`（実際の配送判断）を突き合わせる」という目的に
   とって、役割代入起因の配送問題と物理かなキー起因のGJI warmup契約を
   区別できなくなる。Suppress判定の全条件（ラッチの値を含む）を評価
   する小さなヘルパ関数を新設し、`plan()`と`suppress_reason`の両方が
   このヘルパを呼ぶ形に一本化する（`DbeModeKeyContext`を
   `suppress_reason`にも渡せるようシグネチャを拡張する）。
8. **判別子`event.scan_code != SCAN_KANA`の健全性の証明**: 挿入点で
   vkを書き換える機構はAlt impersonationと役割代入の2つのみである。
   Alt impersonationの発動フラグは`resolve_thumb_key`
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
   役割代入由来であることの健全な判別子である。将来Altセンチネルの
   解決先を変える変更が入った場合、この前提が崩れることに注意
   （「送出VKがDBE系でない」という別の前提と同様に扱う）。
9. **Phase Aの受け入れ条件**: 実装完了後、代入先キーの押下で実際に
   GJI・MS-IME双方の実IMEがONになることを実機で確認することを、
   Phase A実装の受け入れ条件とする。
10. **`reset_physical_key_state`によるmid-holdの再確定（r9追加、
    premortem役R8-M1指摘）**: `reset_physical_key_state()`
    （`hook.rs:334-347`）は`{KEY}_WAS_DOWN`/`SCAN_KANA_WAS_DOWN`を含む
    hold-stateを全クリアする。役割代入対象キーを押しっぱなしのまま
    これが呼ばれると（呼び出し元は`WTS_SESSION_UNLOCK`と
    `panic_reset()`——前者はアンロック時点で物理キーはどれも離されて
    いると仮定してよいため実害が薄いが、後者は打鍵中に走りうる）、
    直後のauto-repeat KeyDownが`role_substitution_fresh_press=
    Some(true)`（fresh press）と再判定され、ラッチが再評価される。この
    時点では1打目のactuationで既にbeliefが開いているため
    `actuation_will_fire`は偽となり、ラッチは`Some(false)`（Allow）へ
    反転する——残りのauto-repeatとKeyUpで生の0xF2がOSへ流れる
    （Alt押下中ならBUG-61のリスクも伴う）。これは`reset_physical_key_
    state`がADR-141決定7以来「押下中の物理キー状態を全クリアする」と
    いう設計を既に持っていることの帰結であり、本ADRが新たに導入する
    ハザードではない（安全な3キー同士の役割代入でも同型の再確定が
    起こりうる）。既知の限定的な制限として記録するに留める——実害は
    当該押下の残り時間に限定され、指を離せば自己修復する。

    **r10追加（premortem役R9-m1指摘）**: 同型の再確定は
    `clear_hook_latches_for_app_disable`の`SuppressionEdge::Leave`
    （`disable_apps`対象ウィンドウから戻る瞬間）でも起こりうる——
    押下中に`disable_apps`対象へフォーカスが移り、戻ってきた時点で
    hold-stateがクリアされていれば、`reset_physical_key_state`と同型
    のmid-hold再確定が発生する。実害の範囲・自己修復の性質は上記と
    同一である。
11. **依存する値の一覧表（r8追加、architect役の総括での推奨）**:
    r0〜r7で発見されたBlockerは、いずれも「決定2が依存する既存の値を
    いつ・どこで読むか」という同一クラスの問題だった。以下に決定2が
    依存する値と、その計算地点・参照地点・間で更新されうるかを一覧
    する。この表はPhase A実装のチェックリスト、および実機受け入れ
    条件の導出元として機能する。

    | 値 | 計算される地点 | 参照される地点 | 間で更新されうるか |
    | --- | --- | --- | --- |
    | `shadow_toggled` | `key_pipeline.rs:270` | `:392`（`plan()`引数） | しない（同一呼び出しの戻り値） |
    | `delegate_owned`/`turn_on_direction` | `:1074-1075`/`:1188-1200` | `:392` | しない（同上、戻り値に載せる） |
    | `kana_role_active` | configロード/reload時 | `:392` | する（押下中のreload）→fresh pressでのみ評価し、確定後はラッチが優先される |
    | `role_substitution_fresh_press` | 決定1の挿入点（`hook.rs:1135`以降、`decide_role_substitution`に渡す`was_down`と同一） | `:392`（`RawKeyEvent`経由） | しない（`{KEY}_WAS_DOWN`/`SCAN_KANA_WAS_DOWN`という単一の情報源から計算、ADR-141決定7＋決定5の5箇所で正しくクリアされる） |
    | `profile` | `key_pipeline.rs:385` | `transport.rs:260` | する（フォーカス遷移）→NM13の既知の制限（決定4参照） |
    | `event_eligible` | 決定1の挿入点（`!alt_impersonated && !is_injected`） | 同挿入点（`decide_role_substitution`の引数） | しない（同一イベント内で計算・消費）。ただしこれが`{KEY}_WAS_DOWN`の更新を条件づけるか否かが未定（r10追加、architect役Minor3・NM18参照） |

    この表から、「押しっぱなしで0xF2が繰り返し届かないこと」
    （`role_substitution_fresh_press`の行）・「押下中のreloadでKeyUp
    が失われないこと」（`kana_role_active`の行）・「押下中のフォーカス
    遷移でorphan
    KeyUpが出ないこと」（`profile`の行）という3つの実機受け入れ条件が
    直接導ける。
12. **`SCAN_KANA_WAS_DOWN`由来の値には現時点で消費者が無い（r10追加、
    premortem役R9-m2指摘）**: `from`=かな方向のイベントは決定2の静的
    3条件（`scan_code != SCAN_KANA`）で除外されるため、かなスロット
    由来の`role_substitution_fresh_press`（`Some`/`None`）を読む消費者
    は現時点で存在しない（決定4/決定5のフック側判定はhook.rs内で
    完結しており、この値を参照しない）。`feedback_dont_provision_
    ahead_without_consumer_logic`の観点から、フィールドのdocに
    「かなスロットについては値を持つが、現時点の消費者は無い（決定2の
    ラッチは`to`=かな方向のみを対象とする）」と明記する——将来
    「使われていないから削除してよい」と誤判断されないようにする。

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

**重要（2026-09-06追加、premortem役指摘、Blocker相当——かなスロットを
設定していないユーザーにも波及する）**: かなスロットの「恒等」は
**「テーブル引きをスキップし、vkを一切書き換えない」ことであって、
「rule_target = Some(かなスロットの正規VkCode)としてテーブル引きする」
ことではない**。安全な3キーでは恒等（vk不変）とテーブル引き結果が
偶然一致するため区別が意識されないが、かなスロットではこの2つは
**別の結果になる**: 物理かなキーはIMEの状態に応じて`VK_DBE_
ALPHANUMERIC`(0xF0、`ShadowImeAction::TurnOff`)・`VK_DBE_KATAKANA`
(0xF1、`shift_katakana_passthrough`が依拠する値)・`VK_DBE_HIRAGANA`
(0xF2)など**複数の異なるvk**を状態依存で生成する（決定8のfrom_name
表現が正規化するのは設定ファイル上の識別子であり、実行時に届く生の
vkはこの限りではない）。恒等補完を素直に「テーブル引きしてかなスロット
の正規VkCode(決定8参照)を返す」実装にすると、0xF0や0xF1のイベントが
その正規VkCode（例: `VK_DBE_HIRAGANA`）に書き換わってしまい、BUG-52が
積み上げてきたIME OFF/ON判定や、BUG-116/ADR-137が実機検証のうえ開けた
`shift_katakana_passthrough`（`transport.rs:103-109`）の唯一の例外経路
が発火しなくなる。**しかもこれは、かなスロットを一切設定していない
ユーザー**（例:「変換⇔無変換」の2-cycleだけを設定した人）**にも起こる**
——決定3の恒等補完により、明示していなくてもテーブルに「かな→かな」
という写像が暗黙に入るため。**決定**: `decide_role_substitution`の
呼び出し側（hook.rs挿入点）は、`original_vk`が安全な3キーのいずれにも
一致せず、かつscan値が`SCAN_KANA`でもない場合と同様に、かなスロットの
`rule_target`が恒等（自分自身と等しい）と解決された場合は**関数の呼び
出し自体を行わず**、vkを一切変更しない。これは実装上の最適化ではなく、
正しさの要件として決定1・決定2の前段に明記する。

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

**store規律の統一（2026-09-06追加、premortem役指摘）**: ADR-141決定1の
r-later訂正（hold-stateへのstoreは`event_eligible`でゲートし、
`!is_injected`単独では行わない）は、`SCAN_KANA_WAS_DOWN`/
`SCAN_KANA_CONFIRMED_TARGET`にも同一に適用する。決定5が新設する
swallow分岐のクリーンアップは既に`!is_injected`を必須条件にしている
（決定5参照）が、storeの規律自体は安全な3キー側と同じ`event_eligible`
に揃え、かなスロットだけ異なる規律にしない。

**`KANA_DOWN_WAS_ALLOWED`は不要（r1で追加・r2で撤回）**: r1はここに
第3のフィールド`KANA_DOWN_WAS_ALLOWED: bool`を追加していたが、これは
(a)書き込み元（`plan()`、メインスレッド）と読み出し元（決定5のswallow
分岐、フックスレッド）が異なりクロススレッドで安全に実装できないこと、
(b)そもそも必要な情報が「かなスロット自身」ではなく「安全な3キー側の
`{KEY}_CONFIRMED_TARGET`」に付くべきものだったこと、の2点で根本的に
破綻していた。

**「安全な3キー→かな」方向にはKeyUp注入が不要な場合がある（r6で確定、
2026-09-06にarchitect役Major指摘で条件を訂正）**: 決定2のr6設計
（fresh press時点でSuppress/Allowをラッチし、以後の同一押下の
auto-repeat/KeyUpはそのラッチをそのまま踏襲する）により、この方向の
Down/Upは**ラッチが確定した値の範囲では常に一致する**——r2〜r5で
個別に検討していた「DownがSuppress・UpがAllowになりうる例外」
（InputRelayへのフォーカス遷移〈NM6〉、`kana_role_active`の押下中
反転〈R3-M1〉、auto-repeatでの反転〈NB9〉）は、いずれもラッチが
個々のイベントで動的条件を再評価しないことにより発生条件自体が
無くなった。

**訂正**: 当初「`{KEY}_CONFIRMED_TARGET == Some(VK_DBE_HIRAGANA)`の
場合には発火させない」としていたが、これは「この方向のDownは常に
Suppressされる」というr3時点の前提に乗っていた。決定2のr4以降、
`actuation_will_fire`が偽の場合（BUG-10救済のフォールスルー等）は
ラッチが`Some(false)`＝Allowになり、**OSは実際に0xF2のDownを受け
取っている**。この場合にKeyUp注入を止めると、ADR-141決定7が補償する
はずのstuck-Downが救済されなくなる。**正しい条件はラッチの値そのもの**
——ラッチが`Some(true)`（Suppress）を保持している場合にのみKeyUp注入を
発火させず、`Some(false)`（Allow）の場合はADR-141決定7のとおり通常
どおり発火させる。ラッチはメインスレッド（`kp_run_inner`）に閉じた値
のため、フックスレッド側の経路（決定7-3のoverflow経路等）でラッチを
直接参照できない場合は、安全側のデフォルトとして「注入する」を選ぶ
（未対のUpの方が、stuck Downより安全——ADR-141決定7-2の非対称に従う）。
**決定7-3（フックスレッド）ではラッチを参照できないため、本規則により
常に注入側になる——フックスレッドからラッチを参照する実装をしては
ならない**（premortem役R11-m1補足、2026-09-06。r1の
`KANA_DOWN_WAS_ALLOWED`と同じクロススレッド罠の再発防止）。なお決定5
が新設する2つのswallow分岐は`SCAN_KANA_*`（`from`=かな方向、targetは
安全な3キーのいずれかのvkであり0xF2にはならない）が対象であり、この
0xF2限定の規則とは無関係——この規則が実際に効くのはADR-141決定7の
5箇所のうち、かなスロット（`{KEY}_CONFIRMED_TARGET == Some(VK_DBE_
HIRAGANA)`）に対する`reset_physical_key_state`・
`clear_hook_latches_for_app_disable`（メインスレッド、ラッチ参照可）・
`passthrough_or_swallow_for_impersonation`（フックスレッド、常に注入
側）の3箇所のみである。
**この例外はADR-141決定7自身への申し送りとしてADR-141本体にも参照
ポインタを追加する**（ADR-142がr5で行ったのと同じ手当て——ADR-143だけ
に書くと、ADR-141を単体で読む実装者が無条件でKeyUp注入を実装して
しまう）。

かつてr2〜r5で個別に検討していた「InputRelayへのフォーカス遷移中の
Down/Up非対称」は、決定2のr6ラッチ設計（Up・auto-repeatはfresh press
時点の判定を再評価せずそのまま踏襲する）により発生条件自体が無くなった
——`profile`が押下中に変わっても、ラッチが確定させた disposition が
そのままUpにも適用されるため、対応するDownを持たない0xF2のKeyUpが
OSへ送出される事態は構造的に起こらない。

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
`:1066-1087`）とは別の合流点である（合計5箇所）。**r7追加
（architect役Minor2指摘）**: これら5箇所はいずれもフックスレッドの
hold-state（`SCAN_KANA_WAS_DOWN`/`SCAN_KANA_CONFIRMED_TARGET`）を扱う
ものであり、決定2が新設するメインスレッドのラッチとは別物——決定2の
`role_substitution_fresh_press`は（r9でNB12対応により）まさにこの
`SCAN_KANA_WAS_DOWN`を含む役割代入自身のhold-stateから計算されるため、
ここでのクリアがそのままfresh press判定の正しさにも反映される。
メインスレッドのラッチ自体をこの5箇所で個別にクリアする必要は無い。

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

   **なぜscanを見なくてよいか（r5追加、architect役Minor1指摘）**:
   背景節が確定した「JIS配列でこれらのVKを生成しうる物理キーはscan
   0x70の1個だけ」という事実により、非注入の0xF5/0xF6/`VK_KANA`は
   物理かなキー由来しかありえない（他の物理キーがこれらのvkを生成する
   経路が存在しない）。したがって`!is_injected`だけで「この swallow
   対象イベントは物理かなキー由来である」ことが導ける——scan一致の
   条件は元々冗長だった。BUG-08/61/62のガードに手を入れる箇所なので、
   この根拠を明記しておく。

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
`ime_actuation_will_fire_before`（r5改名）が`delegate_owned`の場合の
分岐（`hiragana_delegate_to_open_axis_armed`を見る）でカバーしており、
delegateがarmedでなければSuppressせずAllowへフォールスルーする。
`hiragana_delegate_to_open_axis`がarmedでない場合に「誰も何もしない」
状態（`key_pipeline.rs:1065-1072`がBUG-115の元症状として警告する状態）
を再現しないための決定2側の対応は、この分岐によって既に閉じている。

**r5追加（architect役Nit1指摘）**: `delegate_owned`の実体は
`mode_key_delegate_owns_shadow_toggle(event.vk_code)`であり、原理的には
親指キーが「カタカナ」(0xF1)の構成では`katakana_delegate_to_open_axis`
が対象になりうる。しかし決定2が`to`側の合成vkを`VK_DBE_HIRAGANA`
(0xF2)に一本化しているため、役割代入経由で`delegate_owned`が真になる
のは`thumb_vk == 0xF2`の場合のみであり、カタカナ側は構造的に該当
しない。

**r5追加（premortem役R4-M2指摘）**: `hiragana_delegate_to_open_axis`は
物理かなキーが親指キーである前提で設計・検証された機構である。本決定が
許容する構成（代入により物理変換キーが親指キーとして扱われる）で、
このdelegateが実際にopen軸をONにするかは未検証。決定6が「意図した
挙動」とする構成そのものなので、Phase A実装の実機確認項目に追加する
（未解決の疑問6と統合）。

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

### 決定8: config形式の記号は本ADRで確定し、設定GUIのプリセット表現のみPhase Cへ切り出す

ADR-141/142と同様、本ADR（Phase A相当）は挿入点・判定ロジック・既存機構
との相互作用のみを決定する。設定GUIのプリセット表現・文言は、ADR-142の
拡張として後続ADR（Phase C、本ADRの後続として次の空き番号を採番）で
決定する。

**ただし、`[[key_role]]`の`from`/`to`にかなスロットをどう指定させるかの
記号自体は、実装着手前に確定が必要なため本ADRで決定する**（旧「未解決の
疑問1」、architect役M7指摘。2026-09-06、architect役・premortem役との
複数往復の協議で収束）。

**決定8-1: かなスロットを指す設定トークンは`"VK_DBE_HIRAGANA"`の1綴りに
限定する**（`"VK_KANA"`/`"かな"`/`"カナ"`/`"Kana"`は`[[key_role]]`の
`from`/`to`としては拒否する）。

背景: `from_name`（`vk.rs:446`）は`"VK_KANA"`/`"Kana"`/`"かな"`/
`"カナ"`をすべて`VkCode(0x15)`に解決する既存の別名群を持つ。当初
「これを流用し、解決結果0x15を検出したら内部的に正規のVkCodeへ正規化
する」という案（変種A）を検討したが、**採用しなかった**。理由:

1. **rule tableに格納する値は`VK_DBE_HIRAGANA`(0xF2)でなければならない**
   （`decide_role_substitution`の`rule_target`がそのまま書き換え後vkに
   なるため。0x15のまま格納すると、決定2のSuppress条件の第一条件
   `event.vk_code == VK_DBE_HIRAGANA`を一度も満たさず**決定2の機構が
   一度も発火しない**うえ、`transport::plan`が既存のKANJIアームへ落ち
   `Allow`となった経路で`reinject()`が**VK_KANA(0x15)をOSへ送出し
   BUG-08〈合成VK_KANAによるかなロック反転〉を新規に踏みうる**——「沈黙」
   ではなく実害がある）。
2. 0x15→0xF2の正規化を採用すると、config読み込み・reload・GUI保存・
   GUI読み込みの**すべての入口**で正規化を通す義務が生じる。1箇所でも
   漏れると、`from="かな"`（0x15のまま）と`from="VK_DBE_HIRAGANA"`
   （0xF2）が別オブジェクトとして決定3の単射チェックをすり抜け、同一の
   物理キーに2つの写像先が定義された、写像ですらないテーブルになる
   （`awase-settings::is_muhenkan_thumb_key`のdocがissue #99の教訓として
   警告する二重管理バグと同型）。
3. `THUMB_KEY_OPTIONS`は「かな」=`VK_KANA`(0x15)・「カタカナ」=
   `VK_DBE_KATAKANA`(0xF1)・「ひらがな」=`VK_DBE_HIRAGANA`(0xF2)を
   **別項目として**提示しており、ADR-142決定B2は`[[key_role]]`のUIを
   親指キー設定と**同じ「キー設定」タブ**に置くと決めている。変種Aを
   採ると、同じタブの中で「かな」という綴りが片方では0x15（決定6が
   確認したとおり実機ではほぼ発火しない親指キー設定）、他方では0xF2に
   正規化される物理スロット、という**二重の意味**を持つ。

**UXの補い**: Phase CのGUI（決定8のプリセット選択式）では表示ラベルを
`THUMB_KEY_OPTIONS`が0xF2に既に付けている**「ひらがな」に揃える**
（物理キーであることを強調したい場合は「かなキー（物理）」のような
別表記でもよいが、**「かな」という単独のラベルは使わない**）。保存する
config値は`"VK_DBE_HIRAGANA"`に統一する。**「かな」というラベルを
key_role側のGUIに使うと、ADR-142決定B2が同一タブに置くTHUMB_KEY_OPTIONS
の「かな」(0x15、決定6が確認したとおり実機ではほぼ発火しない親指キー
設定)と衝突し、configテキストの層で排除したはずの多義性がGUIラベルの
層で復活する**（architect役指摘、2026-09-06）——理由3が引く
「`THUMB_KEY_OPTIONS`の『ひらがな』→`"VK_DBE_HIRAGANA"`と同じパターン」
を踏襲するなら、ラベルも「ひらがな」に揃えるのが筋である。手書きconfig
で拒否対象の綴り（`"VK_KANA"`/`"かな"`/`"カナ"`/`"Kana"`、および後述の
0xF0/0xF1/0xF3-0xF6）を書いた場合は、「認識できない設定」として黙るの
ではなく、なぜ別の綴りが必要かを含めたエラーメッセージを返す（例:
「`from = "かな"`（VK_KANA）は`[[key_role]]`では使えません。物理かな
キーは実機では`VK_DBE_HIRAGANA`として届くため、`from =
"VK_DBE_HIRAGANA"`と書いてください」）。

**決定8-2: `VK_DBE_ALPHANUMERIC`(0xF0)/`VK_DBE_KATAKANA`(0xF1)/
`VK_DBE_SBCSCHAR`(0xF3)/`VK_DBE_DBCSCHAR`(0xF4)/`VK_DBE_ROMAN`(0xF5)/
`VK_DBE_NOROMAN`(0xF6)も`[[key_role]]`の`from`/`to`として明示的に
拒否する**。これらは同じ物理かなキー（scan 0x70）が状態依存で生成する
別のvkであり、`from_name`はそれぞれ独立したVkCodeとして解決するため、
決定8-1と同じ理由（単射チェックのすり抜け）に加えて、0xF5/0xF6は
BUG-61（復旧不能な入力方式切替）に直結し、0xF0/0xF1は`transport.rs:
341-353`の無条件Suppressガードに掛かり指定しても機能しない。拒否・
正規化の判定は、決定3の単射チェックより**前**に行う。

**決定8-3: 層の区別を明記する**: 決定8-1・8-2はconfigのテキスト表現
（手書き・GUI保存値）に関する拒否であり、決定1の実行時scan照合とは
別の層である。決定1のscan照合では、物理かなキーが実際に`VK_KANA`
(0x15)として届いた場合も（非注入かつAlt非押下の場合に限る、決定1の
上流swallow分岐による絞り込み参照）、`VK_DBE_ALPHANUMERIC`/
`KATAKANA`(0xF0/0xF1)として届いた場合も、scanが0x70である限り代入
対象に含まれる——configの綴りを拒否したことと、実行時にこれらのvkが
代入判定の対象になることは矛盾しない。

**決定8-4: Phase Cへの申し送り**: `awase-settings`のkey_role設定
ドロップダウンは、`THUMB_KEY_OPTIONS`が提示する「かな」「カタカナ」
「ひらがな」の3項目に対し、「ひらがな」相当の1項目のみを提示する
非対称になる。この非対称の理由（決定8-1〜8-2）をPhase CのADRへ
明記して申し送ること。

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
`cargo test -p awase-windows --lib`で自動テスト化できる。**r6追加**:
決定2のfresh press時点ラッチ機構については、`plan()`単体の純粋関数
テストに加え、`kp_run_inner`側のラッチ状態遷移（fresh press→
auto-repeat→KeyUp→クリア）を対象にした状態遷移テストが必要——
fresh pressで確定したSuppress/Allowが、その後のauto-repeat・KeyUpで
再評価されず踏襲されることを確認する（NB8/NB9の回帰防止）。
`actuation_will_fire`（`shadow_toggled || delegate_will_turn_on`）の
真偽2ケース、および`delegate_will_turn_on`が`turn_on_direction ==
ShadowImeAction::TurnOn`の場合のみ真になること（`TurnOff`/`Toggle`
ではいずれも偽になること）もテストに含める。**r7追加（premortem役
R6-m3指摘）**: (i) auto-repeat2打目以降が1打目と同じdispositionになる
こと（NB9の回帰防止）、(ii) ラッチ`None`でのKeyUpがSuppress側になる
こと（r7で追加した安全側デフォルト、決定2参照）、(iii) 静的3条件
（`vk_code`・`!injected`・`scan_code`）を満たさないイベントはラッチを
一切読まないこと（R6-M2の回帰防止）、の3ケースを追加する。いずれも
`plan()`とラッチ状態遷移の組でLinux実行可能である。**r8追加
（architect役Minor1・premortem役R7-m1指摘）**: `RawKeyEvent`への
`role_substitution_fresh_press`フィールド追加は、`build_raw_key_event`
（hook.rs）だけでなく、journal replay基盤（`journal_replay.rs`）・
`golden_scenarios.rs`・各種ユニットテストの`RawKeyEvent`リテラル
構築箇所すべてへ波及する。これらの機械的更新と、journal記録
（`journal.rs`の`KeyInput`）のserde互換性（`#[serde(default)]`等に
よる旧journalとの互換維持）をテスト計画に加える。fresh press自身が
overflowで失われた場合のフォールバック（ラッチ`None`でのKeyDown
評価、決定2参照）のテストも追加する。**r9追加（premortem役R8-m3・
architect役の依存値一覧表指摘）**: `role_substitution_fresh_press`が
依拠する`{KEY}_WAS_DOWN`/`SCAN_KANA_WAS_DOWN`更新規則（auto-repeatでは
上書きしない・KeyUpでクリアする）が将来変わった場合に静かに壊れない
よう、hook.rs側の単体テストとして「auto-repeat KeyDownで`Some(false)`」
「KeyUpで`Some(false)`」「離してから再押下で`Some(true)`」の3ケースを
追加する（`windows-build` CI対象）。`reset_physical_key_state`
（`panic_reset()`経由で押下中に呼ばれうる）が呼ばれた場合の挙動
（決定2の既知の制限、下記参照）もテストで確認する。**r10追加
（premortem役R9-m3指摘）**: `RawKeyEvent`へのフィールド追加は、
`crates/awase-windows/tests/architecture_guard.rs`/
`layer_boundary_guard.rs`（ソーススキャン型のguardテスト）や
`src/config.rs`のround-trip系テストにも波及しうる。guardテストが
新フィールドを弾かないことの確認と、journal記録
（`journal.rs`の`KeyInput`）のserde後方互換（`#[serde(default)]`等に
よる旧journalとの互換維持、premortem役R8-m1で既出）を、決定計画に
並べて明記する。

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
   由来の0xF2（`scan_code != SCAN_KANA`）はSuppress側でラッチされる限り
   本ADRが新たにこの経路でImmCrossへ漏洩を増やすことは無い（物理かな
   キー自身の押下〈`scan_code == SCAN_KANA`〉についての既存の性質は
   本ADRのスコープ外、ADR-141決定4末尾がAltセンチネル構成の既存衝突に
   対して取った立場——悪化させないが解消もしない——と同型に整理する）。

## 未解決の疑問

1. **【解決済み、2026-09-06】かなスロットの`from_name`表現**: 決定8-1〜
   8-4で解決した（config記号は`"VK_DBE_HIRAGANA"`のみ受理、`"VK_KANA"`/
   `"かな"`/`"カナ"`/`"Kana"`と0xF0/0xF1/0xF3-0xF6は拒否、rule table
   格納値は0xF2、拒否・正規化は単射チェックより前）。GUI表示ラベルの
   文言のみPhase Cへ申し送り。
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
   方向のDownはSuppress側でラッチされた場合に発火するようになったため、この経路の発火
   頻度がr1時点より増しており、監査の優先度が上がっている。
7. **conv mode force policy（ADR-086）との相互作用**: 決定9-1に記載した
   `vk_may_mutate_conv`の意味論反転（無変換/スペース→かな、および
   その逆方向、双方向の反転）が、ADR-086のforce policyの前提を壊さない
   かの専用検証が必要。
8. **（r2で解消）ImmCrossプロファイルへの代入後0xF2漏洩の防止方式**:
   決定9-3のr2訂正のとおり、これは本ADR固有の新規ハザードではなく
   既存の性質であり、かつ決定2の再設計により役割代入由来の0xF2は
   Suppress側でラッチされる限りOSへ届かないため、防止方式の検討自体が不要になった。
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
- r5（2026-09-06）: Opus 2体の敵対的レビュー5ラウンド目で、r4で新設した
  `ime_will_be_turned_on_elsewhere`自体に、両エージェントが独立に同一の
  Blocker2件を発見した。(1) NB7/R4-B1——`effective_open()`をライブ値で
  読むと、`kp_stage_shadow_ime_toggle`（belief更新はplan()より前に
  同期的に実行される）の後では常にtrueを指すため、この機構が代入成功
  の全ケースで発火しなくなっていた（`half_width_alnum_toggle_before`が
  同じ理由で既に`_before`スナップショットとして存在する既知の罠と同型）。
  (2) NB8/R4-B2——KeyUp側でこの値を再評価すると、`kp_stage_shadow_ime_
  toggle`がKeyUpで即returnしbeliefがDownで書き込まれた値のまま残るため
  常にfalseとなり、成功パスそのもので毎打鍵、対応するDownを持たない
  0xF2のKeyUpがOSへ送出されていた。修正として、`ime_will_be_turned_on_
  elsewhere`を`ime_actuation_will_fire_before`へ改名し、`kp_stage_
  shadow_ime_toggle`呼び出し**直前**のスナップショットとして定義し
  直し、KeyUpの判定から完全に除外（静的3条件のみに簡素化、BUG-46の
  KANJI系KeyUp常時Suppress前例に揃える）。architect役NM11指摘により
  `kp_stage_shadow_ime_toggle`の戻り値を`shadow_toggled`・
  `delegate_owned`・`ime_open_before`を運ぶ構造体へ変更し、呼び出し元
  での二重計算を排除。premortem役R4-M1（近似であることの明記）・
  R4-M2（delegateの代入後構成での正しさ未検証）・architect役Minor1
  （決定5のscan省略の根拠明記）・Minor2（フラグ改名）・Nit1（カタカナ
  側delegateへの非該当明記）も反映。
- r6（2026-09-06）: Opus 2体の敵対的レビュー6ラウンド目で、architect役
  がauto-repeat経路の新規Blocker1件（NB9）を発見した——`ime_actuation_
  will_fire_before`はDown/Upこそ区別したが、fresh pressとauto-repeat
  KeyDownを区別していなかったため、1打目のactuationでbeliefが開いた
  後は2打目以降のauto-repeatで毎回`Allow`へフォールスルーし、押し
  っぱなしの間ずっと生の0xF2がOSへ送出されていた。決定6が許容する
  親指キー構成では実害が特に大きい経路だった。同時にpremortem役が
  R5-M1として、`ime_actuation_will_fire_before`の非delegate分岐が
  既に`plan()`の第3引数として渡っている`shadow_toggled`と同一の情報
  であり、独自のスナップショット（`ime_open_before`）を新設する必要が
  そもそも無かったことを指摘（r4〜r5のスナップショット機構は二重計算
  であり、かつ`is_japanese_ime()`ゲート等の早期return経路を区別できず
  非等価だった）。R5-M2としてdelegate分岐の述語が配線先の方向
  （TurnOn/TurnOff/Toggle）を見ていない欠陥も発見。

  これらを受け、決定2を全面的に整理した:
  (1) actuation発火の判定を独自スナップショットから`shadow_toggled ||
  delegate_will_turn_on`（`delegate_will_turn_on`は`Some(ShadowImeAction
  ::TurnOn)`との一致判定）へ簡素化し、`kp_stage_shadow_ime_toggle`の
  戻り値は2値（3値の`ShadowToggleOutcome`は不要）に縮小。
  (2) 動的条件の評価をfresh press（`was_down`がfalseからtrueへ遷移する
  瞬間）時点でのみ行い、判定結果（Suppress/Allow）を`plan()`呼び出し
  元に閉じた単一の`Option<bool>`ラッチへ格納、auto-repeat KeyDownと
  対応するKeyUpはこのラッチをそのまま踏襲する設計へ変更。これにより
  NB9に加え、Down/Upで別々の条件式を持つ必要（NB8対応の名残）・
  BUG-46前例の引用問題（NM12）・「機能未使用ユーザーには挙動が
  ビット単位で同一」という主張の綻び（R5-M3）が、いずれも発生条件
  自体の消滅により同時に解消した。決定2は分量が大きくなっていたため、
  r2〜r5の変遷の詳細説明を圧縮し最終設計を中心に書き直した（変遷の
  詳細はステータス節・本changelogを参照）。決定4のInputRelay関連の
  既知の制限も、ラッチ化により発生条件が無くなったため記述を整理した。
- r7（2026-09-06）: Opus 2体の敵対的レビュー7ラウンド目で、architect役
  がfresh press検出手段の未定義に起因する新規Blocker1件（NB10）を
  発見した——r6は「`was_down`がfalseからtrueへ遷移する瞬間」をfresh
  pressの判定根拠としていたが、`was_down`はhook.rs（フックスレッド）
  側の状態でありメインスレッドの`kp_run_inner`からは参照できない。
  「ラッチの`None`/`Some`からfreshnessを推論する」という代替も、
  KeyUpが`kp_run_inner`に到達しない3経路（`FOCUS_APP_DISABLED`早期
  return・overflowラッチ・`ProduceResult::Overflow`）でラッチが
  stale `Some`のまま残り、次のfresh pressをauto-repeatと誤認して
  古い決定を踏襲してしまう欠陥を持っていた（premortem役R6-M1が独立に
  同じ核心を指摘）。修正として、hook.rsが決定1の挿入点で既に判定
  できる`is_keydown && !was_down`を`is_fresh_press`という明示ビットで
  `RawKeyEvent`に運ぶ設計に変更し、fresh pressでは必ずラッチを上書き
  する自己修復性を持たせた。premortem役のR6-M2（ラッチを参照して
  よいイベントの限定が未記載）・R6-M3（ラッチのクリア経路の一部が
  フックスレッドで到達不能）も同時に解消した。architect役のNM13
  （ラッチ判定とInputRelay早期returnの評価順序が未定義）は、ADR-119
  が確立した既存の順序（InputRelayを優先）を尊重する選択をし、フォー
  カス遷移という狭い条件下でのDown/Up非対称を既知の限定的な制限として
  受容することにした。NM14（`delegate_will_turn_on`が既存の`turn_on_
  direction`の再実装だった）も、既存計算を流用する形へ訂正した。
- r8（2026-09-06）: Opus 2体の敵対的レビュー8ラウンド目で、両エージェント
  が独立に「r7の`is_fresh_press`計算位置が挿入点〈`hook.rs:1135`以降〉
  では既にこのKeyDown自身により更新済みの状態を読むため常にfalseに
  なる」という同一の欠陥を発見した（architect役NB11、premortem役
  R7-M1）——r4のNB7（`effective_open()`のライブ値読み）と同じ失敗
  モードがこのADRで3度目に再来するところだった。修正として、
  `is_fresh_press`の計算位置を`PHYSICAL_KEY_DOWN_AT_MS`の更新ブロック
  自体（`hook.rs:955-968`）へ移し、同ブロックに既に存在する
  `prev == 0`イディオム（auto-repeatでないKeyDownの既存の判定方法）を
  流用した。この位置に変えたことで、`RawKeyEvent`の全キーイベントに
  ついて意味のある値になり（premortem役R7-M2「フィールドが汎用core型
  なのに定義域が4オブジェクトのみ」も同時に解消）、かつ更新順序の
  汚染も受けなくなった。architect役NM16（vkキーであるため物理かな
  キー自身については信頼できない）を踏まえ、フィールドのdocに
  「`from`=かな方向には使わず決定4の`SCAN_KANA_WAS_DOWN`を使う」制約
  を追記。premortem役R7-M3（fresh press自身がoverflowで失われる
  ケース）に対応し、ラッチが`None`のまま静的3条件を満たすKeyDownを
  観測した場合も評価点として扱うフォールバックを追加（r6の推論方式を
  `is_fresh_press`のフォールバックとして併用）。ラッチのクリアが
  `plan()`内部の分岐に依存しないことを明記し、NM13の既知の制限に
  「orphan 0xF2 KeyUpがinertである保証は無い」という留保（r5 Minor4）
  を再併記した。
- r9（2026-09-06）: Opus 2体の敵対的レビュー9ラウンド目で、architect役
  が新規Blocker1件（NB12）を発見した——r8が導入した`is_fresh_press`
  （`PHYSICAL_KEY_DOWN_AT_MS`から計算）と、役割代入自身が持つ
  `{KEY}_WAS_DOWN`/`SCAN_KANA_WAS_DOWN`が、どちらも「fresh press」を
  表す独立した情報源になっており、クリア箇所が食い違っていた。
  `PHYSICAL_KEY_DOWN_AT_MS`は長押し時間計測のための別の状態機械で、
  そのクリア規律（`reset_physical_key_state`で全256スロット、
  `clear_hook_latches_for_app_disable`のLeave時はCtrl/Shiftの6スロット
  のみ）は、役割代入側の`{KEY}_WAS_DOWN`のクリア規律（ADR-141決定7の
  3箇所＋本ADR決定5の2箇所）と一致するのは`reset_physical_key_state`
  だけだった。`disable_apps`のEnter遷移やoverflowラッチ経路では、
  役割代入側だけがfresh pressとして再確定されるのに`is_fresh_press`
  はauto-repeatのままという食い違いが生じ、NB10が解決したはずの3経路
  のうち2つで自己修復性が再び失われていた。

  修正として、fresh press判定の情報源を役割代入自身の`{KEY}_WAS_DOWN`
  /`SCAN_KANA_WAS_DOWN`へ一本化した——`decide_role_substitution`
  （ADR-141決定2）は`was_down`を引数に取り更新後の状態を返す設計
  なので、決定1の挿入点でこの関数に渡すのと同じ`was_down`（更新前の
  値）を使えば、r7のNB11のような順序汚染を受けずに計算できる。r7が
  NB11でこれを見落としていたのは、汎用の`PHYSICAL_KEY_STATE`/
  `PHYSICAL_KEY_DOWN_AT_MS`だけを検討し、役割代入専用の情報源を見て
  いなかったためだった。情報源をこの1つに統一したことで、クリア箇所
  はADR-141決定7の3箇所＋本ADR決定5の2箇所に自動的に揃い、二重管理が
  構造的に消えた。フィールドの型も汎用の`bool`から`Option<bool>`
  （`role_substitution_fresh_press`、`None`=役割代入の対象外）へ改名
  し、injectedイベントでの不正確さ（premortem役R8-m1）も自然に解消
  した。この統一により、r8で必要だった「かなスロット自身にはvkの
  非決定性により信頼できない」という制約（NM16）も不要になった
  （`SCAN_KANA_WAS_DOWN`はscanベースのため）。premortem役R8-M1
  （`reset_physical_key_state`によるmid-holdの再確定）は、情報源統一
  後もADR-141が既に持つ性質として既知の制限に明記するに留めた
  （本ADRが新規に導入するハザードではない）。
- r10（2026-09-06）: Opus 2体の敵対的レビュー10ラウンド目。**両
  エージェントとも新規Blockerゼロと判定した初めてのラウンド**。
  architect役のMajor2件——NM17（r9がフォールバックの正当化を「取り
  こぼし対策として不要になった」と誤って書き換えていたが、フォール
  バック自体は本文に残っており、対で来ないinjectedイベントによる
  `was_down`のstuck〈ADR-141決定1が明記する危険〉に対する保険として
  引き続き必須だった）、NM18（ADR-141決定1とADR-142決定B7が、
  injectedイベントで`{KEY}_WAS_DOWN`を更新するか否かについて食い違って
  おり、本ADRがどちらに依存するかを表明していなかった）——に対応し、
  フォールバックの正当化を訂正、「injectedでは更新しない」への依存を
  明記してADR-141/142側へ申し送った。premortem役のMajor1件（R9-M1:
  決定9-3が却下した出自フィールドとの性質の違いが未記載）に対応し、
  `role_substitution_fresh_press`は出自ではなくキーライフサイクルの
  事実である旨を明記。Minor4件（`clear_hook_latches_for_app_disable`
  Leave時の同型再確定・`SCAN_KANA_WAS_DOWN`由来の値に現時点で消費者が
  無いことの明記・guardテスト/journal serde互換への波及・依存値一覧
  表への`event_eligible`行追加）も反映した。r0〜r9で繰り返された
  「改訂が新Blockerを生む」パターンのADR記述版として、NM17を教訓として
  残す。
