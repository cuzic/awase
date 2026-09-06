# ADR-143: かな系キーの役割代入（Kana Key Role Substitution）

## ステータス

**収束（r25、2026-09-06、両エージェント最終承認・Blocker/Major/Minor/
Nitすべて解消）。** r21収束後、ユーザー要望で決定7に新機能
（Shift+代入先キーでカタカナへ切り替え、GJI限定）を追加し、これも
Blocker1件・Major5件を経て両エージェントの承認を得た（詳細は決定7
「r22」「r23」節を参照）。以下は
r21に至るまでの経緯。

**r21（2026-09-06、`engine_enabled`の置き場所を「サイクル単位の
fresh press無効化」へ一本化して確定、両エージェントへ再確認依頼中）**:
`engine_enabled`の扱いは6段階の試行錯誤を経た。(1) NM24でADR-141
決定1の`event_eligible`へ追加→(2) NM25でhold-storeまで連動し
`{KEY}_WAS_DOWN`が取り残されると判明、`kana_role_actuate`だけに移す→
(3) NB14でvkのセンチネル書き換え自体はゲートされず完全な死にキーに
なると判明、`to`方向のfresh press確定（`rule_target`をNoneとして
扱う）へ移す→(4) R13-M1で`to`方向だけのゲートは`from`方向との非対称
で全単射が破れると判明、`from`側にも対称に追加→(5) NB15/R14-M2で
`from`側の述語に直接混ぜると押下中のエンジントグルでKeyUpが取り残される
と判明→(6) NB16で「かなに直接触れる腕だけ」を無効化すると3-cycleの
残り1腕が生き残り全単射が破れると判明。**最終的に、決定3に「かな
スロットが関与するサイクル全体（3-cycle等の全ての腕を含む）を、
fresh press時点でconfig由来の単一の値`kana_cycle_active`により
一括で無効化する」という概念を新設し、決定1・決定2-2はどちらも
この値だけを参照する形に一本化した**（`is_kana_slot_press`・
`kana_role_actuate`いずれの式にも`engine_enabled`を直接混ぜない）。
かなスロットを含まない独立したサイクル（安全な3キーのみの構成）は
この判定と無関係なため、ユーザーが確認した製品方針「安全な3キー
同士の入れ替えはエンジンのON/OFFと独立に動作し続けてほしい（秀Caps
相当の常駐リマッパとして）」どおりエンジン非依存のまま動作し続ける
（ADR-141の`event_eligible`は無変更のまま）。NM23はdrift correction
緩和策自身が`is_japanese_ime()`にゲートされ、grace窓では機能しない
ことを既知の制限として明記。r14までの内容は以下のとおり。

r12の
アーキテクチャ転換をOpus 2体の敵対的レビュー（architect役・premortem役、
r0〜r11と同一エージェント）に再度かけ、r13でBlocker 1件（R12-B1）・
Major 1件（R12-M1）・Minor 3件・Nit 1件を反映後、architect役の独立
レビューでさらにBlocker 1件（NB13、rule tableとhold-stateの層の混同）・
Major 2件（NM19: BUG-10救済経路の記録漏れ、NM20:「NICOLA判定に一切
関与しない」という前提の書きすぎ）・Major相当の訂正1件（NM21:
「唯一の絞り点」が実は`reinject()`ではなく`send_input_safe`だった）・
Minor数件を発見、すべて反映した。r11で「収束」と宣言した設計は、実装
着手前の根本再調査で前提の1つが誤りだと判明したため撤回した。決定1・
決定3・決定5・決定8は維持、決定2を全面置換、決定4・決定6・決定7・
決定9を整合するよう調整した。旧決定2（r0〜r11の設計）は「旧設計
（r11まで）の記録」節に要約として残す。r0〜r11で発見した個別の
Blockerを表形式で再検証し、いずれも再発しないことを確認済み
（premortem役、詳細は決定2各節参照）。両エージェントへ再確認依頼中。**

**r13→r14で反映した指摘の要約**:
- **Blocker（NB13、architect役指摘）**: 決定2-2が「rule tableの
  `rule_target`をそのまま返し、hook.rs側でvkだけセンチネルへ差し替える」
  としていたが、これだと`{KEY}_CONFIRMED_TARGET`に実0xF2が残り、
  決定2-8・決定4が要求する「confirmed_target==センチネル」の前提と
  矛盾し、KeyUpのたび実0xF2がOSへ漏れる。hook.rsは戻り値の**2要素とも**
  センチネルへ差し替えることを明記（rule table自体は0xF2のまま、
  hold-stateはセンチネル、という2層の区別を確立）。
- **Blocker→再訂正（R12-B1→NM21、architect役指摘）**: 「OSへ絶対に
  出ない」保証の絞り点を、当初`reinject()`としていたが、真の唯一の
  チョークポイントは`win32::send_input_safe`（doc既載、`vk_may_mutate_
  conv`ゲートも同じ理由でここに置かれている）と判明。ガードを
  `send_input_safe`へ移設し、`reinject()`はそれを内部で呼ぶだけの
  経路の1つとして再整理。
- **Major（R12-M1→NM19、architect役が記録の甘さを再指摘）**: NB6
  （BUG-10食い逃げ救済）喪失について、`ir_apply_drift_correction`が
  タイマー駆動の独立した定期リフレッシュであり、打鍵と無関係に自動で
  乖離補正を試みることを実コードで確認し、「2打鍵で回復」という仮説
  ではなく「バックグラウンドで自動回復する」という検証済みの緩和策に
  差し替えた。
- **Major（NM20、architect役指摘）**: 「`to`=かな方向はNICOLA判定に
  一切関与しない」という前提が言い過ぎだった。正確には「`Char`に
  分類されないため同時打鍵の構成キーにはなりえないが、`Passthrough`
  イベントとしてengineは通過する」——複数箇所の記述を訂正。
- **Minor**: `is_sync_key`を根拠なく`true`にしていた点を`false`に訂正
  （決定2-3）、`reinject()`のシグネチャをスニペット上も実装と一致させる
  （premortem役Nit）、ドロップ時のログ追加、`plan()`側ガードが二重防御
  として消せない理由の明記、決定6の陳腐化した「親指キー判定」への
  言及を削除。
- **Nit**: configのどの入力経路からもセンチネル値が生成されないことの
  確認を追加（決定2-1）。

### r12でアーキテクチャを転換した理由

r11までの決定2は、**「OSへ合成`VK_DBE_HIRAGANA`(0xF2)をSendInputで
直接送ってはならない」という制約**を出発点にしていた。この制約を守る
ために、hook.rsの挿入点でvkを0xF2へ書き換えた上で、メインスレッドの
`transport.rs::PhysicalKeyDisposition::plan`の既存F2判定分岐を条件付き
Suppressへ拡張し、「別の場所で既に発火しているはずのIME actuation
（`kp_stage_shadow_ime_toggle`→`ime_controller::apply`）に便乗する」
という**間接設計**になっていた。この間接性——「actuationを呼ぶべきか」
の判断と「actuationを実際に呼ぶ」処理が別の場所にあり、前者が後者の
発火を*推測*しなければならない——が、r2〜r11で格闘した問題の大半
（`actuation_will_fire`の近似、fresh pressラッチ、`role_substitution_
fresh_press`フィールド、`kana_role_active`のSSOT、NM17のフォールバック、
NM18、R11-M1のクロススレッド不可能性）を生んでいた。

実装着手前の再調査で、この出発点の制約自体が2点で誤っていた:

1. **「合成0xF2を絶対に送らない」は誇張だった**:
   `crates/awase-windows/src/ime.rs::send_ime_mode_key_with_shift_
   release_prefix`が、**既に本番コードで**scan付き合成
   `VK_DBE_HIRAGANA`をSendInputしている（GJI半角英数トグルの出口処理、
   `output/mod.rs:1231`から呼ばれる）。そのガードは同`:1201`の
   `hook::ime_mode_key_injection_blocked_by_modifier()`（`hook.rs:296`）
   ——**Win/Alt押下中かどうかだけ**である。BUG-61（Alt+かなで復旧不能）が実際に
   問題にしているのは**Alt/Win押下中の**合成送信であって、「合成0xF2
   を送ること自体」ではない。
2. **より重要: そもそもvkを0xF2にする必要が無かった**。`to`=かな方向
   （安全な3キー→かな）は、代入後の意味論が「IMEをONにする」という
   純粋なモード切替であり、`Char`に分類されないため**NICOLA同時打鍵の
   構成キーにはなりえない**（正確な言い方——2026-09-06訂正、
   architect役NM20指摘。「NICOLA判定に一切関与しない」は言い過ぎで、
   `Passthrough`イベントとしてengineの`on_input`は通過する。背景節
   「`to`=かな方向がNICOLA判定に参加しないこと」参照）。したがって
   `classify_key`が代入後vkから何を導くかは重要ではなく、`Passthrough`
   に落ちさえすればよい。`from`=かな方向（物理かなキー→安全な3キー）
   は代入後のキーが実際に`Char`として同時打鍵の構成キーになりうる
   ため挿入点での書き換えが必須だが、r11の決定2は`to`方向にも同じ
   構造をそのまま踏襲した結果、0xF2という**実在するIME意味論を持つvk**
   を経由させ、下流のF2分岐（BUG-52/116ガード）・`kp_restore_hiragana_
   for_suppressed_mode_key`・`vk_may_mutate_conv`・
   `shift_katakana_passthrough`・delegate機構と全面的に絡み合わせて
   しまっていた。

**r12の決定**: `to`=かな方向のvk書き換え先を、実在するIME意味論を持つ
0xF2ではなく、**どのvk空間の値とも衝突しない専用センチネル**
（`VK_ROLE_KANA_ACTUATE`、決定2参照）にする。センチネルは下流のどの
既存判定にも一致しないため、F2分岐との絡み合いが構造的に消える。
その代わり`plan()`にセンチネル専用の無条件Suppress分岐を1つ置き、
actuationはメインスレッド側で**直接**呼ぶ。「呼ぶべきかの判断」と
「実際に呼ぶ処理」が同一スレッド・同一イベント処理の中で同期的に完結
するため、r11までの間接推論とそれが生んだ機構が丸ごと不要になる。

### r11から引き継ぐ確定事項

1. **実装順序**: ADR-141（Phase A基盤）→ADR-142（Phase B config/GUI）
   →本ADRの順で実装すること（変更なし）。
2. **かなスロットの`from_name`表現（決定8-1〜8-4）**: config記号は
   `"VK_DBE_HIRAGANA"`のみ受理（`"VK_KANA"`/`"かな"`/`"カナ"`/`"Kana"`
   と0xF0/0xF1/0xF3-0xF6は拒否）、rule table格納値は0xF2、拒否・正規化
   は単射チェックより前。GUI表示文言のみPhase C送り。**r12補足**:
   rule table上の格納値が0xF2であることと、実行時にhook.rsが書き換える
   先がセンチネルであることは別の話である（決定2参照）——テーブルは
   「かなスロットというオブジェクト」の識別子として0xF2を使い、
   `to`=かな方向の書き換え先だけがセンチネルになる。
3. **かなスロットの恒等補完の意味（決定3）**: 「恒等＝テーブル引き
   自体をスキップしvkを書き換えない」。「恒等＝rule_targetをかな
   スロットの正規VkCodeとしてテーブル引きする」実装だと、かなスロット
   を一切設定していないユーザーでもBUG-52/BUG-116（ADR-137）の既存
   挙動が壊れる。r12でも変更なし（この決定はセンチネル導入と独立に
   必要）。
4. **【r12で消滅】NM18（`{KEY}_WAS_DOWN`のinjected更新規則の不一致）**:
   r11の決定2はこの規則に**正しさを依存**していたが、r12設計では
   どちらの読み方でも最悪「当該押下1回分がactuationせずに終わる」
   だけで、次の物理KeyUpで自己修復する（決定2「injected/overflowでの
   取りこぼし」参照）。本ADRは依存しない。ADR-141/142側の不一致解消は
   引き続きそちらの課題として残る。
5. **【r12で消滅】injected KeyUpによるラッチ迂回（r11のBlocker相当
   ×1・Major相当×3）**: センチネルは**イベント種別・injected有無に
   関わらず常にSuppress**されるため、「Downは配送されたのにUpだけ
   Suppressされる（またはその逆）」という非対称が構造的に発生しない。
   ラッチ自体が不要になったため、その読み取り・書き込み・クリアの
   規律をめぐるr11の3段階の訂正もすべて消滅した。

Phase A実装時の受け入れ条件として以下の実機確認が必要（決定8のテスト
計画に加えて）: 代入先キー押下でGJI・MS-IME双方の実IMEが実際にONになる
こと、代入先キーを押しっぱなしにしてIMEが暴れないこと（auto-repeat時の
二重actuation防止の検証）、代入先キーからOSへ何も送出されないこと。

**Shift+代入先キー機能（決定7 r22/r23）の実機確認チェックリスト
（2026-09-06、premortem役指摘で集約）**: 上記4点に加えて以下も
Phase A実装時に確認する。
5. 要望を出したユーザーの環境がGJIかMS-IMEか（scan=0のためMS-IMEでは
   無反応の見込み＝既知の制限として確定させる前提情報）。
6. scan=0の`VK_DBE_KATAKANA`をGJIが「Shift+かな相当」として実際に
   解釈するか（未解決の疑問、決定7参照）。
7. `ime_open_before == true`のときのカタカナ送信が、実際にカタカナへ
   切り替わるか（かつ2回目以降の挙動が物理キーと一致するか）。
8. 右Shift押下時に、解放（両側`make_scan_key_input`）と復元（押されて
   いた側のみ）が正しく動き、左Shiftのstuckが起きないか。
9. `half_width_alnum_toggle_active`区間で競合しないか（ADR-137 M-3）。

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

### スレッド境界とキューの実態（r12の再調査で確定、決定2の設計根拠）

r1〜r11は「フックスレッドとメインスレッドの間で値を安全に渡せない」
という制約に繰り返し突き当たった（r1の`KANA_DOWN_WAS_ALLOWED`、r7の
NB10、r11のR11-M1）。この制約の正確な形を確定させる:

- フックコールバックは**本物の別OSスレッド**で動く:
  `hook.rs:747 install_hook()`が`std::thread::Builder::new().name
  ("awase-hook")`でスレッドをspawnし、`SetWindowsHookExW`＋
  `GetMessageW`専用ループを回す。
- そのスレッドとメイン（"engine"）スレッドの間は、
  `crates/awase-windows/src/hook_channel.rs`の`HookKeyRing`（SPSC
  リングバッファ、`CAP=1024`）**1本だけ**で繋がっている。運ばれるのは
  分類済みの抽象イベント`RawKeyEvent`（vk/scan/injected/
  `key_classification`/`ime_relevance`/`physical_pos`/`modifier_key`/
  `modifier_snapshot`）である。
- **フックは対象キーを常に即Suppressする**: `hook.rs:1195`の
  `HOOK_KEYS.produce(event)`の直後、`hook.rs:1199`が
  `ProduceResult::Accepted => LRESULT(1)`を返す。つまりキューへ投入
  した時点で応答が確定し、WH_KEYBOARD_LLの同期制約はここで切れている。
  「Allow」に相当する実際のOS配送は、メインスレッドが後から
  `RawKeyEventExt::reinject()`で**非同期に**SendInputする形で行われる
  （`crates/awase-windows/src/lib.rs:348-376`）。

この構造から2つの帰結が出る。(a) フックスレッドで決めなければならない
のは「OSへ即座に何を返すか」ではなく「メインスレッドへ**どんな抽象
イベントを渡すか**」だけである。(b) したがってフックスレッドが持つ
情報（`{KEY}_WAS_DOWN`等のhold-state）を、`RawKeyEvent`のフィールドと
して**運ぶ**ことは自然にできる一方、メインスレッド側の値をフック
スレッドから**読む**ことは依然としてできない。r1・r11が破綻したのは
後者を試みたためであり、r7〜r10が`role_substitution_fresh_press`という
新フィールドを作ろうとしたのは前者の正しい形だった。r12の決定2は、
その「運ぶ」対象を新フィールドではなく**既存の`ime_relevance`
フィールド**（もともとプラットフォーム層が事前分類してcoreへ渡す
ための場所）にすることで、新フィールドの追加自体を不要にする。

### `to`=かな方向がNICOLA判定に参加しないこと（r12の再調査で確定）

ADR-141決定1が挿入点を`classify_key`（`hook.rs:1171`）より**前**に
置いているのは、`classify_key`が**書き換え後のvk**でNICOLA同時打鍵の
親指キー判定を行う必要があるためである（ADR-019のcore/platform境界:
coreは生のvkを見ず、事前分類された`KeyClassification`だけを見る）。
これは`from`=かな方向（物理かなキー→安全な3キー、代入後はNICOLA判定に
参加する）には正当な制約である。

一方`to`=かな方向は、代入後の意味論が「IMEをONにする」という純粋な
モード切替であり、`Char`に分類されないため**NICOLA同時打鍵の構成
キーにはなりえない**（2026-09-06訂正、architect役NM20指摘——「NICOLA
判定には一切参加しない」は正確ではない。センチネルのイベントは
`Passthrough`として`HookKeyRing`経由でengineの`on_input`を通過し、
NICOLA FSMを素通りするわけではない。pending中の同時打鍵候補がある
状態でこのPassthroughイベントが届いた場合の挙動——flushの契機になる
かどうか——はPhase A実装時の確認項目とする、決定2-10参照）。したがって
`classify_key`が代入後vkから何を導くかは重要ではなく、**`Passthrough`
に落ちさえすればよい**。決定2のセンチネルは実際にそうなる:
`vk::is_passthrough`（`vk.rs:243-263`）はセンチネル値を含まないが、
続く`scan_to_pos(model, scan)`が`None`を返す（`scan`は書き換えられず
元の物理scanのまま——変換=0x79・無変換=0x7B・スペース=0x39・
かな=0x70はJIS/USどちらのテーブルにも存在しない、
`crates/awase-windows/src/scanmap.rs`）ため、`classify_key`は
`(KeyClassification::Passthrough, None)`を返す。

## スコープ

- **対象**: 物理かなキー（scan 0x70、以下「かなスロット」）と、ADR-141の
  安全な3キー（`VK_CONVERT`/`VK_NONCONVERT`/`VK_SPACE`）の間の役割代入。
  かなスロットを含めた4オブジェクトの全単射（決定3）として、ADR-141の
  モデルを拡張する。
- **対象外（本ADRでは扱わない）**:
  - `to`に`VK_DBE_ROMAN`/`VK_DBE_NOROMAN`(0xF5/0xF6)を割り当てること
    （決定2、BUG-61により構造的に禁止）。
  - `to`に`VK_DBE_ALPHANUMERIC`/`VK_DBE_KATAKANA`/`VK_DBE_SBCSCHAR`/
    `VK_DBE_DBCSCHAR`(0xF0/0xF1/0xF3/0xF4)を割り当てること（**r12訂正**:
    `to`=かなの意味論は「専用センチネルへ書き換え・常にSuppress・
    awase自身がIME ON相当をactuateする」の1通りに一本化されており、
    そもそもOSへ`VK_DBE_*`のいずれかを送る経路が存在しないため、
    どの`VK_DBE_*`亜種をGUIの選択肢に出す余地も無い）。
  - IME OFF方向の代入（決定2の既知の制限1、「かなスロット＝IME ON指示」
    という意味論に限定するため再現しない）。
  - config形式・設定GUIの具体的な表現（Phase C、ADR-142と同様に後続ADRへ
    切り出す。決定8参照）。
- かなスロットが自分自身へ写る（＝代入なし）場合、既存の挙動
  （BUG-52/116のガードを含む）は一切変更しない。本ADRの分岐は、かなスロット
  と安全な3キーのいずれかとの間に**非自明な**入れ替えが設定された場合にのみ
  発火する。

## 設計原則（本ADR全体を通じて4回発見された同型バグからの一般規則）

**同一イベント処理内で、先行ステージが書き込む値をガード条件に使う場合は、
必ずそのステージ実行前のスナップショット（`_before`命名規約）を取る。
ライブ値（実行後に読む値）をガードに使うと、恒真または恒偽になり
「保護しているように見えて実際には何もしていない」状態になる。**
（2026-09-06、architect役Nit指摘で一般化。個別の発見箇所は
`half_width_alnum_toggle_active`〈既存、Opusレビュー由来〉→r4のNB7
（`effective_open()`のライブ値読み）→r7のNB11（`is_fresh_press`を
挿入点で読むと`PHYSICAL_KEY_*`が更新済み）→r25のR25-M1（`ime_open_
before`導入前の`effective_open()`直接読み）の4回。次にこのADR・
`plan()`・actuation経路へ新しい判定材料を足す際は、この規則を
真っ先にチェックリストとして当てること。）

## 決定

### 決定1: from判定をかなスロット専用にscan値ベースへ拡張する

ADR-141決定1の挿入点（`hook.rs:1135`の`vk = rewritten_vk;`直後、`if
!is_injected`ブロック内側先頭）はそのまま維持する。安全な3キーの`from`
判定は引き続きvk値の一致で行う（ADR-141と同一）。かなスロットの`from`
判定のみ、以下の条件に変更する:

```
is_kana_slot_press = (scan == SCAN_KANA) && event_eligible
```

**`engine_enabled`はこの述語に含めない（2026-09-06、NB15・NB16・
R13-M1・R14-M2を経て決定3の「かな関与サイクル」概念に一本化して
確定）**: `is_kana_slot_press`は`scan`・`event_eligible`という
「イベントの出自」（押下中に変化しない性質）だけで決まる述語のまま
とし、可変のグローバル状態である`engine_enabled`をここに混ぜない
（NB15指摘: 混ぜると押下中のエンジントグルでKeyUpが取り残される）。
`engine_enabled`（正確には決定3が定義する`kana_cycle_active`）は、
fresh press時点で`rule_target`を`decide_role_substitution`へ渡す
かどうかの判断にのみ効かせる——詳細な機構・全単射を破らない理由は
決定3「エンジンOFF時は『かなスロットが関与するサイクル全体』を
無効化する」を参照。

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

### 決定2（r12で全面書き直し——r0〜r11の設計は「旧設計（r11まで）の記録」節を参照）: toがかなスロットになる場合、専用センチネルvkへ書き換え、常にSuppressし、メインスレッドが直接actuateする

**中核の設計判断**: 安全な3キーのいずれかがかなスロットへ代入される場合
（例:「変換→かな」）、hook.rsの挿入点で書き換える先を
`VK_DBE_HIRAGANA`(0xF2)のような**実在するIME意味論を持つvk**にはせず、
NICOLA判定・`classify_key`・下流のどのvk判定とも衝突しない**専用の
センチネルvk**にする。センチネルを載せたイベントは、
`PhysicalKeyDisposition::plan`が**無条件にSuppress**し、OSへは一切
配送しない。実際のIME ON操作は、メインスレッドの`kp_run_inner`が
既存のactuation経路へ**直接**投入する。

この3点（センチネル／無条件Suppress／同一スレッドでの直接投入）が、
r11までの決定2が抱えていた間接性——「別の場所で既に発火したはずの
actuationを推測して二重配送を止める」——を構造的に置き換える。

#### 決定2-1: センチネルvkの定義と値

```rust
// crates/awase-windows/src/vk.rs
/// 役割代入で「安全な3キー → かなスロット」方向が成立したことを表す内部
/// センチネル VK（ADR-143 決定2）。
///
/// Windows の VK コードは 0x00-0xFF に収まる（`KBDLLHOOKSTRUCT.vkCode` は
/// 1-254 と定義されている）ため、0x0100 は **OS・キーボードレイアウト・
/// 他プロセスのいずれからも生成されえない**。この値を持つイベントは
/// hook.rs の役割代入挿入点が作ったものだけである。
pub const VK_ROLE_KANA_ACTUATE: VkCode = VkCode(0x0100);
```

**値の選定根拠**:

- `VkCode`は`u16`（`src/types.rs:8`）なので0xFF超の値を表現できる。
  「Windows VK空間の外」であることが、衝突しないことの**構造的**保証に
  なる（未使用の0x88-0x8F等を選ぶと、将来OSが割り当てる／別レイアウトが
  生成する可能性を排除できない）。
- 256要素配列（`PHYSICAL_KEY_STATE`〈`hook.rs:186`〉・
  `PHYSICAL_KEY_DOWN_AT_MS`〈`:193`〉）は**いずれも`.get()`**で添字
  アクセスしており、範囲外は`None`で安全に素通りする（パニックしない）。
  センチネルはそもそもこれらに記録されるべきでない値なので、この挙動が
  正しい。
- `vk_may_mutate_conv`（`vk.rs:187-194`、対象は0x15/0x1C/0xF0-0xF6）・
  `is_ime_control`（`:267`）・`is_ime_context`（`:273`）・
  `ImeKeyKind::from_vk`・`is_synthetic_dbe_ime_hotkey`・
  `is_composition_confirm_key`・`is_passthrough`のいずれにも一致しない。
- `vk_to_char`系の数値演算（`vk.rs:649-650`）は`0x41..=0x5A`/`0x30..=0x39`
  のレンジガード内なので影響を受けない。

**ADR-141決定4の「Altセンチネル」とは別物である**: ADR-141が「センチネル」
と呼んでいるのは`left_thumb_key`/`right_thumb_key`の**設定文字列**
（`"Left Alt"`/`"Right Alt"`）であり、VK空間の値ではない。本ADRの
`VK_ROLE_KANA_ACTUATE`は、このリポジトリで初めてVK空間に導入する
センチネル値である。両者を混同しないこと。

**config経路からも生成されないことの確認（2026-09-06追加、premortem役
R12-n1指摘）**: 「OSからは来ない」に加えて、awase内部のどの入力経路
からもこの値が生成されないことを確認する——`VkCodeExt::from_name`
（`vk.rs`の照合表）は0x0100を返す綴りを持たず、`[[keymaps]]`/
`[[key_role]]`は生の数値表記（`"0x100"`等）を受け付けない（文字列を
`from_name`経由でのみ解決する設計、決定8参照）。したがって
`VK_ROLE_KANA_ACTUATE`はhook.rsの役割代入挿入点（決定2-2）だけが
生成しうる値であることが、OS側・config側の両方から閉じている。

#### 決定2-2: hook.rs挿入点での書き換えとactuationビット

決定1の挿入点（`hook.rs:1135`の`vk = rewritten_vk;`直後、
`decide_role_substitution`の呼び出し位置）で、解決済みルールが
「押された物理キーのスロット → かなスロット」だった場合:

```
（フックスレッド、決定1の挿入点、fresh press時点のみ）
rule_target がかなスロット && kana_cycle_active
  → decide_role_substitutionへ渡すrule_targetはそのまま
    （通常どおりconfirmed_target = Some(かなスロット)が確定しうる）
rule_target がかなスロット && !kana_cycle_active
  → decide_role_substitutionへ渡すrule_targetをNoneとして扱う
    （代入不成立。confirmed_target = None、vkは元の物理vkのまま）

（confirmed_targetがかなスロットとして確定した場合のみ）
  → vk = VK_ROLE_KANA_ACTUATE
  → kana_role_actuate = event_eligible && is_keydown && !was_down
```

`kana_cycle_active`は決定3が定義する「かなスロットが関与するサイクル
全体がエンジンON時のみ有効」という値（`= engine_enabled`、ただし
サイクル全体に対して一括で適用される）である。`kana_role_actuate`の
式には`engine_enabled`を重ねて含めない——エンジンOFFなら上記の
`rule_target`確定段階で`confirmed_target`が`None`になり、vkがセンチネル
へ書き換わること自体が無いため、`kana_role_actuate`側に同じ条件を
重ねても到達しない（二重管理を避ける）。

- `was_down`は、**その物理キー（安全な3キーのいずれか）自身の
  `{KEY}_WAS_DOWN`**（ADR-141決定2が新設するhold-state）の更新前の値
  であり、`decide_role_substitution`に渡すのとまったく同じ値である。
  したがって`is_keydown && !was_down`は「この押下のfresh press」を表す
  （r9のNB12対応で確立した「情報源を役割代入自身のhold-stateに一本化
  する」原則をそのまま継承する）。
- `event_eligible`は決定1・ADR-142決定B7と同一の`!alt_impersonated &&
  !is_injected`（変更なし）。
- **`engine_enabled`（`kana_cycle_active`）の置き場所は、この形に
  確定するまで4段階の試行錯誤を経た**（2026-09-06、NM24→NM25→NB14→
  NB15/NB16→確定。詳細な失敗の内容はステータス節の経緯と決定3の
  「かなスロットが関与するサイクル全体を無効化する」を参照）:
  `event_eligible`自体に含めるとhold-storeがエンジン状態に連動し
  `was_down`が取り残される（NM25）、`kana_role_actuate`にのみ含める
  とvk書き換え自体は止まらず完全な死にキーになる（NB14）、`to`方向
  だけ・`from`方向だけを個別にゲートすると全単射が破れる（NB15の
  逆方向・NB16の3-cycle）。**正しい置き場所は、決定3が定義する
  「サイクル単位のfresh press時点の`rule_target`無効化」だけ**であり、
  `is_kana_slot_press`（決定1）・`kana_role_actuate`のどちらの式にも
  `engine_enabled`を直接混ぜない。`confirmed_target`はfresh press
  時点で確定しKeyUpまで保持される（ADR-141決定2の既存規律）ため、
  押下中にエンジンがトグルされてもDown/Upは常に対称になる。
- **この判定と消費はどちらもフックスレッド内で完結する**——r7〜r10が
  `RawKeyEvent`に`role_substitution_fresh_press`という新フィールドを
  追加しようとしていたのは、判定（フックスレッド）と消費（メイン
  スレッドの`plan()`）が分かれていたためである。r12では消費先が
  `build_raw_key_event`（同じフックスレッド、`hook.rs:824-851`）に
  なるため、**新フィールドは不要**になる。

**auto-repeatとKeyUpでもvkの書き換え自体は行う**（`kana_role_actuate`が
偽になるだけ）。これが「Down/Up/auto-repeatが常に同じdisposition
（Suppress）になる」ことの根拠であり、r11がラッチで実現しようとした
性質を、値そのもので保証する形に置き換えている。

**センチネルへの差し替えは呼び出し側（hook.rs）で行い、
`decide_role_substitution`（ADR-141決定2の純粋関数）は変更しない**:
rule table自体は0xF2のまま保持する（決定8-1、config表現・全単射
検証・決定9の下流合流点の推論はすべて0xF2基準で行われるため）。
`decide_role_substitution`はrule tableの`rule_target`（0xF2）を
そのまま**受け取り**、`(書き換え後vk, 次に保持する確定役割)`という
戻り値の2要素双方に0xF2を返す——ここまではADR-141決定2の純粋関数を
一切変更しない。

**hook.rs側で、この戻り値の2要素とも**（書き換え後vkだけでなく、次に
保持する確定役割＝`{KEY}_CONFIRMED_TARGET`に格納する値も）**センチネル
へ差し替える**（2026-09-06訂正、architect役NB13指摘）。差し替えないと
`{KEY}_CONFIRMED_TARGET`に0xF2が残り、ADR-141決定2の「KeyUpは
confirmed_targetから対称に変換する」規律により、KeyUp側だけ実0xF2を
載せてしまう（Downはセンチネル）。このKeyUpは決定2-4のセンチネル
専用分岐にも`reinject()`のガードにも一致せず、既存のF2分岐
（`transport.rs:276`）へ落ちて`Allow`となり、**対応するDownを持たない
実0xF2のKeyUpが毎打鍵OSへ送出される**——さらに決定2-8・決定4の
「`{KEY}_CONFIRMED_TARGET == Some(VK_ROLE_KANA_ACTUATE)`のときADR-141
決定7のKeyUp注入を発火させない」という条件も一致しなくなり、二重に
実0xF2が漏れる。

まとめると: **rule table＝0xF2（config・全単射・決定9はこの層で完結）、
hold-state（`{KEY}_CONFIRMED_TARGET`）＝センチネル（決定2-4・決定2-8・
決定4はこの層で完結）**という2層の区別を明確に保つ。これは決定3が
既に呼び出し側に置いた判断（「かなスロットの`rule_target`が恒等なら
関数呼び出し自体を行わない」）と同じ層に属し、ADR-141側の純粋関数に
かなスロット固有の知識を持ち込まないためである。

`kana_role_actuate`は`build_raw_key_event`（`hook.rs:824-851`）へ
引数として渡す（`is_injected`等と同じ扱い）。フックスレッド内の
ローカル変数のまま完結するので、`static`もアトミックも新設しない。

#### 決定2-3: `ime_relevance`にactuation意図を載せる

`build_raw_key_event`（`hook.rs:824-851`）が`vk == VK_ROLE_KANA_ACTUATE`
の場合に構築する`ime_relevance`を次のとおり特別扱いする（それ以外の
vkについては`classify_ime_relevance(vk)`のまま、一切変更しない）:

```
ImeRelevance {
    may_change_ime:  kana_role_actuate,
    shadow_action:   None,
    is_sync_key:     false,
    sync_direction:  kana_role_actuate.then_some(ShadowImeAction::TurnOn),
    is_ime_control:  false,
}
```

**`is_sync_key`を`true`にしない理由（2026-09-06追加、premortem役R12-m1
指摘）**: 当初は`sync_direction`と対にして`true`にしていたが、
`IntentWitness::from_sync_key`（`evidence.rs:364-368`）は
`!injected && sync_direction.is_some()`しか見ておらず`is_sync_key`を
参照しない。`is_sync_key`の実際の読み出し箇所は現状コードベースに
存在せず（`focus_tracker.rs`の3箇所は書き込みのみ）、消費ロジックの
無いフィールドに値を立てる理由が無い
（`feedback_dont_provision_ahead_without_consumer_logic`の趣旨）。
`false`のままにする。

**なぜ`sync_direction`（`IntentKind::SyncKey`）であって
`shadow_action`（`IntentKind::PhysicalImeKey`）ではないか**——3つの
理由があり、いずれも実コードで検証済み:

1. **`is_japanese_ime()`ゲートを通らない**:
   `kp_stage_shadow_ime_toggle`（`key_pipeline.rs:1077-1086`）は
   `sync_direction`を最優先で採用し、`shadow_action`は
   `self.platform_state.ime.belief.is_japanese_ime()`が真のときにしか
   見ない。役割代入は**ユーザーがconfigで宣言した意味**であって、
   「日本語IMEがこのキーを報告した」という観測ではないため、probe
   ベースの確率的beliefでゲートするのは誤りである（ゲートすると、
   grace期間中の`is_japanese_ime()`偽答で「OSへも送らず・actuateも
   しない」完全な死にキーになる）。
2. **意味論として正直である**: `UserIntentSource::SyncKey`のdocは
   「設定された同期キー」（`state/ime_event.rs:73-77`）であり、
   `[[key_role]]`で`to="VK_DBE_HIRAGANA"`と宣言されたキーはまさに
   これに当たる。`PhysicalImeKey`は「物理KANJI押下」の意であり、
   物理的には変換キーであるこのイベントに名乗らせるのは偽装になる。
   `write_sync_key`と`write_physical_key`（`state/platform_state.rs:
   1256`/`:1309`）は`source`タグ以外の実装が同一なので、belief更新の
   挙動自体は変わらない。
3. **`plan()`の既存条件に引っかからない**: `transport.rs::plan`の
   `let is_kanji_event = event.ime_relevance.shadow_action.is_some();`
   （`:311`）に一致させないことで、ImmCross無条件Suppressアーム・
   BUG-46の`ime_actuation_owned`アームのどちらにも入らない。
   センチネルの配送判断は決定2-4の専用分岐**だけ**が決める。

**`shadow_action`を`None`のままにする副次効果**:
`IntentWitness::from_physical`（`state/evidence.rs:356-362`）は
`shadow_action.is_some()`を要求するため`None`を返し、
`IntentWitness::from_sync_key`（`:364-370`）だけが`Some`を返す。
`!e.injected`の要求は両者共通なので、**injectedイベントが役割代入
経由でactuationへ昇格することは型で不可能**（BUG-14の型化がそのまま
効く）。加えて`kp_stage_shadow_ime_toggle`自身が`event.injected`で
早期return（`key_pipeline.rs:1050-1060`）するため、二重に閉じている。

**`is_japanese_ime()`の即時true更新（ADR-093）はセンチネルでは
発火させない**: `should_upgrade_is_japanese_ime`（`vk.rs`）は
`is_synthetic_dbe_ime_hotkey`（0xF0-0xF4）に限定されており、
センチネルは含まれない。これは意図した設計である——ADR-093の論拠は
「このVKが届くこと自体が、何らかのIMEがこのキーを処理・報告している
証拠」だが、センチネルはawase自身が変換キーの押下から作った値であり、
IMEの存在証明にはならない。ここを拡張すると、日本語IMEが無い環境でも
`is_japanese_ime()`が真になり、force-ON actuation経路
（`is_eligible_for_ime_force_on()`）が誤って解禁される。

**`enrich_ime_relevance`との関係**: `kp_run_inner`冒頭
（`key_pipeline.rs:207`）が呼ぶ`enrich_ime_relevance`
（`runtime/mod.rs:435-447`）は、(a) 設定された`sync_toggle_keys`/
`sync_on_keys`/`sync_off_keys`にvkが含まれる場合に`sync_direction`を
上書きし、(b) `resolve_mode_key_shadow_override_for_event`で
`shadow_action`を上書きする。センチネルは(a)のいずれの集合にも入らず
（設定GUIの選択肢に無いvk空間外の値であり、`VkCodeExt::from_name`も
解決しない）、(b)は0xF2/0xF1の親指キー構成にのみ反応するため、
**どちらもセンチネルには作用しない**。決定2-3が載せた値はそのまま
`kp_stage_shadow_ime_toggle`へ届く。

**この結論が依存する実装の性質（2026-09-06追加、architect役Minor
指摘）**: `enrich_ime_relevance`の(a)がif/else-if連鎖で**一致した
場合にのみset**し、**非一致の場合に既存値をクリアしない**ことは、
現在のコードの性質であって、変わらないことが保証された契約ではない。
将来誰かがこの関数に`else { rel.is_sync_key = false; rel.
sync_direction = None; }`のような網羅的なクリア処理を足すと、決定2-3
が`build_raw_key_event`で載せた`sync_direction`が静かに消え、決定2の
機構全体が沈黙する。この依存を本節に明記しておくことで、
`enrich_ime_relevance`を変更する際のレビュー観点になる。

#### 決定2-4: `plan()`のセンチネル専用分岐（無条件Suppress）

`transport.rs::PhysicalKeyDisposition::plan`の**先頭**（InputRelay
早期return〈`:260`〉よりも前）に、次の分岐を追加する:

```rust
// ADR-143 決定2: 役割代入で「安全な3キー → かなスロット」に化けたキー。
// OS へ配送する意味を持たない内部センチネルなので、イベント種別・
// injected・profile に関わらず常に Suppress する。
if event.vk_code == crate::vk::VK_ROLE_KANA_ACTUATE {
    return Self::Suppress;
}
```

**この位置とこの無条件性の根拠**:

- **なぜAllowにできないか**: `Allow`は`RawKeyEventExt::reinject()`が
  `wVk: VIRTUAL_KEY(self.vk_code.0)`でSendInputすることを意味する
  （`lib.rs:348-376`）。センチネルをそのまま送ればOSにとって無意味な
  VKのSendInputになる。したがってセンチネルに対して`Allow`という選択肢
  は存在しない。
- **なぜinjectedを条件に含めないか**: センチネルを生成できるのは
  hook.rsの挿入点だけである。他プロセスが`SendInput(wVk=0x0100)`を
  試みることは理屈上できるが、`KBDLLHOOKSTRUCT.vkCode`はMicrosoft
  公式に1-254と定義されており、その値は正規のキーではない。仮に届いた
  としてもSuppressするのが安全側であり、ADR-119「解釈しない入力は
  消費しない」が守ろうとした「リモート側の**実在するキー**が無反応に
  なる」被害像には当たらない。
- **なぜInputRelay早期returnより前か**: 後ろに置くとInputRelay
  ウィンドウでセンチネルが`Allow`になり、上記の無意味なSendInputが
  実際に起きる。ADR-119が確立した「F2分岐よりInputRelayを先に見る」
  という順序（`transport.rs:246-259`のコメント）は0xF2という**実在
  するキー**についての判断であり、センチネルはその対象ではない
  ——この分岐は既存の順序を変更するのではなく、既存の判断列全体の
  手前に「そもそもOSへ出せない値」を落とすフィルタを1つ足すものである。
- **既知の制限（InputRelayウィンドウでの不発）**: この結果、
  InputRelayプロファイルのウィンドウにフォーカスがある間、`to`=かな
  方向の代入先キーは**OSへも届かず、awaseもactuateしない**（actuation
  側もADR-119の`AppImeProfile::InputRelay`ゲートで落ちる）ため完全に
  無反応になる。物理かなキーそのものは従来どおりAllowされる（決定1が
  `from`側の代入を設定していない限り）ので、影響は本機能の`to`=かな
  ルールを設定したユーザーに限られる。回避策は`disable_apps`に当該
  リレーウィンドウを登録すること——`FOCUS_APP_DISABLED`早期return
  （`hook.rs:980-982`）は決定1の挿入点より前にあり、役割代入自体が
  丸ごとバイパスされるため、元の物理キーがそのままOSへ届く。この
  既知の制限は、r11のNM13（押下中のフォーカス遷移でorphan KeyUpが
  出る）とは別種であり、**r12では押下中にフォーカスがInputRelayへ
  移ってもorphan KeyUpは発生しない**（Down/Upとも常にSuppressのため）
  ——NM13は解消済みとして扱う。

**`suppress_reason`の対応**（r11の既知の制限7に相当）:
`PhysicalKeyDisposition::suppress_reason`（`transport.rs:32-45`）に
センチネル用の理由ラベル（例: `"kana-role"`）を1アームだけ追加する。
r11が要求していた「Suppress判定の全条件を評価する共有ヘルパを新設し
`plan()`と`suppress_reason`の両方から呼ぶ」という手当ては**不要**に
なった——判定がvk値の単純な等値比較1つに縮んだため、二重管理の
リスクが構造的に無い（BUG-90調査が必要とする「役割代入起因のSuppress
と物理かなキー起因のGJI warmup契約〈`"tsf-f2"`〉を区別する」という
目的は、このラベル追加で満たされる）。

**Blocker（2026-09-06、premortem役R12-B1指摘）: 「OSへ絶対に出ない」
という保証を`plan()`の1ゲートだけに委ねてはならない**——`physical`
（`plan()`の戻り値）が実際に参照されるのは`executor.rs::execute_relay`
の2アーム（`Decision::PassThrough`と`Decision::PassThroughWith`）だけ
であり、**effect列経由で`Effect::Input(InputEffect::ReinjectKey(evt))`
が積まれて`reinject()`へ渡る経路は`physical`を一切参照しない**。実在
する生産点は少なくとも2つある: (1) `engine/engine.rs:328-332`の
`lifecycle.flush_pending_key_ups()`（保留中のKeyUpをstuck key対策として
再注入する経路）、(2) `message_handlers.rs:1400-1420`の`INPUT_DEFER`
output-drain replay。センチネルは`classify_key`で`KeyClassification::
Passthrough`になる（決定8参照）ため、NICOLA pending状態がある間に
押されると engine側でConsumeされ`KeyLifecycle`のpending KeyUpに載る
可能性があり、そこを経由すると`plan()`を一切通らずに`wVk:
VIRTUAL_KEY(0x0100)`のSendInputが発行されうる。これはr0（`transport::
plan`のF2分岐の位置）・r2（injected早期returnの順序）・r4
（`kp_stage_shadow_ime_toggle`の3つのゲート）で繰り返し露呈した「他にも
経路がある」という同じ失敗パターンである。

**決定（再訂正、2026-09-06、architect役NM21指摘）**: 「センチネルは
OSへ出ない」という大域的性質を、`reinject()`ではなく
**`win32::send_input_safe`（`win32.rs:229`）の先頭1箇所**で構造的に
保証する。この関数のdocコメントが既に「このクレートの全`SendInput`
呼び出しは本関数を経由する**唯一のチョークポイント**である」と明記
しており（`vk_may_mutate_conv`のconv_mutationゲートが同じ理由でここに
置かれている、`win32.rs:216-221`参照）、`reinject()`自身も内部で
`send_input_safe`を呼ぶ（`lib.rs:372`）。加えて`reinject()`は複数ある
呼び出し経路の1つに過ぎない——ADR-141決定7-3のKeyUp注入
（`make_key_input_ex`+`send_input_safe`直接呼び出し）、`ime.rs::
send_ime_mode_key`、`key_pipeline.rs`のf2_inputs、`hook.rs::
inject_alt_menu_mask`もすべて同じ関数を経由する。`reinject()`だけに
ガードを置くと、決定2-8・決定4が「設計ルール」として要求している
「`{KEY}_CONFIRMED_TARGET == Some(センチネル)`のときADR-141決定7の
KeyUp注入を発火させない」が、もし将来どこかで見落とされた場合に
`send_input_safe`まで達してしまう——これはR12-B1が警告した「到達経路の
網羅への依存」と同じ形のリスクである。

```rust
#[must_use]
pub(crate) fn send_input_safe(inputs: &[INPUT]) -> u32 {
    if inputs.iter().any(|i| {
        i.r#type == INPUT_KEYBOARD
            // SAFETY: type チェック済みなので Anonymous.ki は有効なフィールド。
            && unsafe { i.Anonymous.ki }.wVk.0 == crate::vk::VK_ROLE_KANA_ACTUATE.0
    }) {
        return 0; // ADR-143 決定2: センチネルはOSへ出さない
                  // (唯一のSendInputチョークポイントでの構造的保証)
    }
    // ...既存の実装（conv_mutationゲート等）
}
```

戻り値`0`は「送信した件数」という既存の契約（実際のSendInputが0件成功
した場合と同じ値）をそのまま使うため、シグネチャ変更が不要で
呼び出し元への波及が無い（NM22が指摘した「戻り値の意味が未定義な
bool導入」を避ける）。

**将来の拡張に向けた注記（2026-09-06追加、premortem役Nit指摘）**:
このガードは「バッチ内にセンチネルが1つでもあればバッチ全体を
`return 0`で捨てる」実装である。現時点でセンチネルを含むバッチは
`reinject()`が組み立てる単発`INPUT`のみであり、正規のキーとの混在は
発生しない。**将来、センチネルと他の正規キーを同一バッチで送る呼び出しを
追加する場合、この「バッチ全体を捨てる」実装ではセンチネル以外の
`INPUT`まで黙って落ちる**ため、その時点でフィルタ方式（センチネルの
要素だけを`inputs`から除外してから`SendInput`する）へ変更すること。

`plan()`の無条件Suppress（決定2-4本体）は削除せず、二重の防御として
残す——`send_input_safe`側のガードが「最後の砦（構造的）」、`plan()`
側のガードは「そもそも呼ばせない（設計上の一貫性・`suppress_reason`
ラベルによるBUG-90調査での区別・journal記録との整合のため、こちらも
消してはならない）」という別の役割を持つ。決定2-4本体にも「`physical`
は`execute_relay`の2アームしかゲートしない。effect列経由の
`ReinjectKey`やADR-141決定7-3のKeyUp注入は別経路であり、そちらは
`send_input_safe`側で落とす」ことを明記する。

**ログ（Minor、architect役指摘。2026-09-06、文言を再訂正）**:
ドロップ時に`log::debug!("[kana-role] sentinel drop (intentional,
not OS-blocked)")`のような1行を残す（このリポジトリの
`[relay-passthrough]`/`[relay-defer]`/`[kana-mode-restore]`等の
角括弧タグ命名規約に倣う）。戻り値`0`は「意図的にドロップした」場合と
「OSにブロックされた（UIPI等）」場合を区別しない——`inject_alt_menu_
mask`が`sent=2/2`を、`send_gji_half_width_alnum_toggle`が`sent=`を
ログに出すように、件数を見る呼び出し元は他に存在するため、ログ文言
自体に「意図的なドロップである」ことを明記し、この区別をログの読み手
（実機調査者）に伝える。実機調査で「そもそもセンチネルが送信経路に
到達したのか」を切り分けられるようにするため、決定2-10の受け入れ
条件(c)（Spy++等でSendInput送出ゼロを確認）の裏付けとしても機能する。

#### 決定2-5: actuationの呼び出し点

センチネルのイベントは`HookKeyRing`経由でメインスレッドへ渡り、
`kp_run_inner`（`key_pipeline.rs:206`）が通常どおり処理する。
`kp_stage_shadow_ime_toggle`（`:270`で呼ばれる）が決定2-3で載せた
`sync_direction = Some(TurnOn)`を読み、既存経路のまま
`write_sync_key(witness, true, tick_ms)`→belief更新→下流の
`executor.rs::dispatch_ime_set_open`→`ime_controller::apply`という
**既存のactuation合流点**へ流れる。

**新しい呼び出し点は作らない**。`.claude/rules/fix-requires-evidence.md`
の「IME actuation合流点」（ADR-119、issue #136の教訓——新しいgateを
1箇所に置いて満足すると、他の合流点が素通しになる）と対称の理由で、
新しい**actuation経路**を足すのも同じリスクを持つ。r12設計は既存の
`kp_stage_shadow_ime_toggle`という単一の合流点をそのまま使い、
そこへ渡す入力（`ime_relevance`）だけをプラットフォーム層で決める
——これはADR-019が定める「プラットフォーム層が事前分類し、判断は
既存の単一箇所で行う」という層境界そのものである。

**r11との決定的な違い**: r11は「actuationが**既に**発火しているはず
だから、二重配送だけ止める」という*推測*を`plan()`側で行っていた
（`actuation_will_fire = shadow_toggled || delegate_will_turn_on`）。
r12は「actuationを発火させる入力を自分で作り、配送は無条件に止める」
——推測する対象が存在しない。したがって以下がすべて不要になる:

| r11の機構 | r12で不要になった理由 |
| --- | --- |
| `actuation_will_fire`（`shadow_toggled`/`turn_on_direction`を`plan()`へ運ぶ） | Suppressが無条件のため、actuationの発火有無を配送判断に使わない |
| `kp_stage_shadow_ime_toggle`の戻り値の構造体化 | 同上（`shadow_toggled: bool`のまま） |
| fresh press時点でのSuppress/Allowラッチ（`Option<bool>`） | disposition が押下中ずっと同じ（常にSuppress）のため、ラッチする対象が無い |
| `RawKeyEvent::role_substitution_fresh_press`フィールド | fresh press判定の消費先がフックスレッド内（決定2-2）に移り、運ぶ必要が消えた |
| フォールバック（ラッチ`None`のKeyDownを評価点にする、NM17） | ラッチが無い |
| `kana_role_active`（`DbeModeKeyContext`の4つ目のフィールド）とそのSSOT要件 | センチネルの**存在自体**が「かなルールが有効」の証拠。別経路で計算したboolと乖離しうる構造が無い |
| injected KeyUp専用のSuppress分岐と、ラッチ読み取り/クリアのinjected規律 | Down/Up/injectedのすべてが同じ無条件Suppressになる |
| `kp_restore_hiragana_for_suppressed_mode_key`への`scan_code == SCAN_KANA`除外条件（r11の既知の制限2） | 同関数は`event.vk_code != VK_DBE_HIRAGANA`で即return（`key_pipeline.rs:90`）。センチネルは0xF2ではないので**改修不要** |
| `delegate_will_turn_on`／`hiragana_delegate_to_open_axis`の方向判定 | センチネルは親指キーになりえない（決定6参照）ため`delegate_owned`が構造的に偽 |

#### 決定2-6: auto-repeatとKeyUpの扱い

- **KeyUp**: `kp_stage_shadow_ime_toggle`は先頭でKeyUpを即return
  （`key_pipeline.rs:1009-1011`）するため、actuationは発火しない。
  配送は決定2-4により無条件Suppress。**Down/Upは常に対称**である。
- **auto-repeat KeyDown**: `kana_role_actuate`が偽になるので
  `sync_direction`が`None`となり、`kp_stage_shadow_ime_toggle`は
  `intent_kind`が`None`で早期return（`:1087-1089`）する。二重
  actuationは起きない。
- **二重の防御になっている点**: 仮に`kana_role_actuate`の計算が
  （injected/overflow等で）誤って真になっても、`kp_stage_shadow_ime_
  toggle`のno-op分岐（`:1159`、`effective_open() == current`）が
  belief書き込みとapply-imeを見送るため、実IMEへの二重actuationには
  ならない。これは物理かなキーを押しっぱなしにしたときの既存挙動と
  同一である（no-op分岐内の`eisu_reset_on_turn_on_while_open`だけは
  repeatごとに走るが、これも物理かなキー押しっぱなしと同じ既存挙動で
  あり、本ADRが新規に持ち込むものではない）。

#### 決定2-7: injected／overflowでの取りこぼしとその影響

`kana_role_actuate`が誤って偽になり、その押下がactuationしないまま
終わる経路が2つある:

1. **injectedイベントが`{KEY}_WAS_DOWN`を更新する読み方を採った場合**
   （ADR-141決定1とADR-142決定B7の不一致、r11のNM18）: injectedな
   KeyDownが`was_down=true`にすると、直後の物理KeyDownがfreshと判定
   されない。
2. **`ProduceResult::Overflow`（`hook.rs:1203-1205`）**: 挿入点
   （`:1135`）はキュー投入（`:1195`）より前なので、hold-stateへの
   storeが済んだ後にイベント自体が破棄されうる。

**いずれも「その押下1回がactuationせずに終わる」だけで、状態は壊れない**
——配送は常にSuppressなので、r11が恐れた「Downだけ配送されてUpが
Suppressされる（またはその逆）」という非対称は発生しない。次の物理
KeyUpが`{KEY}_WAS_DOWN`を偽に戻すため、次の押下は正しくfreshになる
（自己修復する）。r11がこの2経路のために必要としていたフォールバック
機構と、NM18の「injectedでは更新しない」への依存は、どちらも不要に
なった。

#### 決定2-8: ADR-141決定7（KeyUp注入）との関係

ADR-141決定7は、`{KEY}_CONFIRMED_TARGET`が`Some(vk)`のまま押下が
中断される経路（`reset_physical_key_state`・
`clear_hook_latches_for_app_disable`・
`passthrough_or_swallow_for_impersonation`）で、代入後vkのKeyUpを
OSへ注入して stuck Down を防ぐ。

**決定**: `{KEY}_CONFIRMED_TARGET == Some(VK_ROLE_KANA_ACTUATE)`の
場合は**KeyUp注入を行わない**。センチネルのDownは一度もOSへ配送されて
いない（決定2-4の無条件Suppress）ため、対応するUpを注入する意味が無く、
注入すれば無意味なVKのSendInputになる。

この条件は`{KEY}_CONFIRMED_TARGET`という**フックスレッド側の値だけ**で
判定できるため、r11がぶつかった「ラッチはメインスレッドの値なので
決定7-3（フックスレッド）から参照できない」という問題
（premortem役R11-M1）は発生しない。r11は「Downが常にSuppressされる」
という前提が決定2のr4以降で崩れたために条件をラッチの値へ移さざるを
得なかったが、r12ではこの前提が**無条件に**成り立つため、単純な
`confirmed_target`の等値比較へ戻せる。

**危険度の優先順位との整合**: 本ADR・ADR-141が従う「stuck Downを避ける
ことを優先し、orphan/重複Upは許容する」という優先順位に対し、本決定は
「注入しない」側を選んでいる。これは優先順位の例外ではなく**適用範囲外**
である——OS側にセンチネルのDownが存在しないので、避けるべきstuck Down
自体が存在しない。

**この例外はADR-141本体にも参照ポインタを追加する**（ADR-143だけに
書くと、ADR-141を単体で読む実装者が無条件でKeyUp注入を実装してしまう。
ADR-142がr5で行ったのと同じ手当て）。

#### 決定2-9: Alt/Win押下時の安全性

r0〜r1が出発点にしていた「Alt/Win押下中に合成`VK_DBE_HIRAGANA`を
SendInputするとBUG-61（復旧不能）を誘発しうる」という実機診断
（`key_pipeline.rs:1983-1989`、2026-08-17）に対して、r12設計は
**そもそも合成`VK_DBE_*`をSendInputしない**ため該当しない。
センチネルはOSへ送られず、actuationがOSへ送るVKは
`GjiDirectStrategy`/`MsImeDirectStrategy`の`VK_IME_ON`(0x16)、
`KanjiToggleStrategy`の`VK_KANJI`(0x19)であり、いずれも
`VK_DBE_*`一族ではない（MS-IMEのON操作は2026-08-06・BUG-50対応で
`VK_DBE_HIRAGANA`から`VK_IME_ON`単発へ変更済み、`ime_controller.rs:
190-203`）。

**この論拠が成立しなくなる条件**（r11から変更なし）: 将来
`key_sequence_policy`の送出VK選択がIME ON操作用に`VK_DBE_*`一族へ
戻される変更が入った場合、この安全性の論拠は崩れる。そのような変更を
行う際は本ADRの前提が崩れることを明記し、Alt押下時のガードを別途
追加する必要がある。なお、既に本番コードに存在する合成0xF2送出
（`ime.rs::send_ime_mode_key_with_shift_release_prefix`、GJI半角英数
トグルの出口）は`hook::ime_mode_key_injection_blocked_by_modifier()`
でWin/Alt押下中をガードしており、本ADRはこの経路には触れない。

#### 決定2-10: 既知の制限・確認項目

0. **エンジンOFF中は、かなスロットが関与するサイクル全体が恒等に
   戻る**（2026-09-06追加、premortem役R13-M1・architect役NB15/NB16
   指摘を経て決定3の「かな関与サイクル」概念に一本化）: 無変換3連打
   等でエンジンをOFFにしている間、かなスロットが関与するサイクルに
   属するルール（例:「変換⇔かな」の2-cycle、または「変換→かな→
   スペース→変換」のような3-cycleならその**全ての腕**）は不成立になり、
   関係する物理キーはすべてそれぞれの素の挙動（`VK_CONVERT`・実機
   依存の`VK_DBE_*`・`VK_SPACE`等）に戻る。これは意図的なトレードオフ
   である——個別の腕（`to`=かな方向だけ、`from`=かな方向だけ）を
   個別にゲートすると、死にキー（NB14）・全単射違反（NB15の逆方向・
   NB16の3-cycle）のいずれかが必ず生じるため、**サイクル単位で一括
   無効化するのが唯一の安全な選択**だった（詳細は決定3参照）。
   **かなスロットを含まない独立したサイクル（例: 変換⇔無変換の
   2-cycle）はこの判定と無関係で、エンジンOFF中も引き続き動作する**
   ——ユーザーが確認した製品方針「安全な3キー同士の入れ替えはエンジン
   のON/OFFと独立に動作し続けてほしい」はこちらで満たされる。
   3-cycle構成では「変換⇔かな」を設定するとサイクルに含まれる他の
   ルールもエンジンOFF中は一緒に止まる、という帰結をPhase CのGUI
   説明に含める必要がある（決定3のPhase Cへの申し送り参照）。

1. **IME OFF方向・charset軸（カタカナ/半角英数→ひらがな）の復帰は
   再現しない**（r11から変更なし）: 代入先キーは**open軸（IME ON）
   のみ**を操作する。`VK_IME_ON`はcharset軸には一切触れない
   （`ime_controller.rs:224-228`）。ADR-100決定2以降の「eager warmupが
   open軸のみ」という片肺化（ADR-137 M-6の真因）と同型の制限であり、
   意図的に選択したトレードオフである。将来charset軸まで再現したい
   場合は、GJI/MS-IMEでexit実装を共有しない（BUG-25/ADR-107）という
   制約を満たす専用経路を新たに設計する必要がある。
2. **`composition_native_f2_down`が呼ばれなくなる**（r12で新規に
   明記）: `kp_stage_execute`（`key_pipeline.rs:2236-2244`）は
   `event.vk_code == VK_DBE_HIRAGANA`のKeyDownに対して
   `mark_cold` + eager warmupを行う。センチネルは0xF2ではないので
   この副作用が起きない。r11設計（vk=0xF2）では起きていたので、
   これはr12で変わる点である。actuation経路（`ime_controller::apply`）
   自身がwarmup契約を持つため理論上は問題ないはずだが、**Phase A実装時
   の確認項目**とする（Chrome等のTSFネイティブアプリで、代入先キーで
   IME ONした直後の1文字目がリテラル化しないか）。
3. **InputRelayウィンドウでの不発**: 決定2-4の既知の制限を参照。
2.5. **【2026-09-06追加、architect役NM20指摘】センチネルがPassthrough
   イベントとしてNICOLA FSMを通過することの影響**: センチネルは
   `Char`に分類されないため同時打鍵の構成キーにはなりえないが、
   `classify_key`が`Passthrough`を返す以上、`HookKeyRing`経由で
   engineの`on_input`を通過すること自体は避けられない（r11の0xF2
   設計でも同型だったため退行ではない）。pending中の同時打鍵候補が
   ある状態でこのPassthroughイベントが届いた場合に、既存のFSMが
   それを解決/flushの契機として扱うかどうかを**Phase A実装時に
   確認する**（物理変換キーを押した以上、他の保留中候補をflushする
   挙動はむしろ自然であり、実害を想定してはいないが、確認済み事項
   として明記する必要がある）。
3.5. **【Major、2026-09-06追加、premortem役R12-M1指摘】NB6
   （BUG-10「食い逃げ」救済経路）が失われる**: r4でarchitect役が発見した
   NB6——`kp_stage_shadow_ime_toggle`のno-op分岐（`key_pipeline.rs:1159`、
   `effective_open() == current`）により、beliefが既にONのときは
   actuationが発火しない——は、r11では`actuation_will_fire`が偽の場合に
   `Allow`へフォールスルーし、MS-IME/非TSFが物理0xF2をネイティブ処理
   することで救済されていた。r12は配送を**無条件Suppress**にしたため、
   この救済経路が構造的に消えている。結果、belief=ON・実IME=OFFという
   乖離状態では、代入先キーを何度押しても「OSへも出ない・actuateも
   しない」完全な無反応になる（`transport.rs:264-274`のコメントが
   「ここで消すと物理ひらがなキーが食い逃げされ、intent/Engineだけ
   ONで実IMEがOFFのまま乖離する（BUG-10、2026-07-06実機）」と警告
   している状況そのもの）。**これはr12で意識的に受け入れるトレード
   オフとして記録する**（8ラウンド追跡した論点なので黙って消さない。
   2026-09-06、architect役NM19が再指摘し記録の追加を要求）。

   **検証済みの緩和策（(a)を実コードで確認、architect役の要求に対応）**:
   `runtime/ime_refresh.rs::ir_apply_drift_correction`は`ir_stage_notify`
   （`:227-233`、末尾で`reschedule_ime_refresh()`により次回実行を
   スケジュールする**タイマー駆動の定期リフレッシュ**）から呼ばれ、
   ユーザーの打鍵とは独立に周期的に`desired`（belief）と`observed`
   （実IMEの観測値）の乖離を検知し、乖離していれば`actuation_for`
   経由でactuationを再送する。この再送は「beliefを実状態に合わせる」
   のではなく「実状態をbeliefに合わせにいく」向き（`desired`を目標に
   actuationする）であり、`kp_stage_shadow_ime_toggle`のno-op分岐が
   ブロックした押下起点のactuationとは**別の、独立した経路**である。
   したがってbelief=ON・実IME=OFFの乖離は、代入先キーの再押下を待たず、
   このリフレッシュサイクル（実測間隔はPhase A実装時に確認）が回るたび
   に自動的に補正が試みられる——「2打鍵で回復する」という仮説
   （architect役が検証を要求した案(a)）ではなく、**打鍵と無関係な
   バックグラウンド回復**という、より確実な形で成立する。
   もう1点の緩和策として、(b) 物理かなキー自身は（`from`=かなのルールを
   設定していない限り）従来どおり`Allow`されるため、ユーザー操作による
   即時の回復手段も別途残る。

   **緩和策(a)の前提条件と、それが崩れる窓（2026-09-06追加、architect役
   NM23指摘）**: `ir_apply_drift_correction`自身も冒頭で
   `!self.engine.is_user_enabled() || !self.platform_state.ime.belief.
   is_japanese_ime()`という早期returnを持つ（`ime_refresh.rs:574`）。
   決定2-3が`sync_direction`（SyncKey）を選んだ第一の理由は「probe
   ベースの確率的belief`is_japanese_ime()`でゲートすると、grace期間中の
   偽答で完全な死にキーになる」ことだった。ところが緩和策(a)自身は
   その`is_japanese_ime()`に依存している。スリープ復帰・フォーカス
   変更直後のgrace窓で`is_japanese_ime()`が偽答している間は、(1)
   actuation経路自体は決定2-3のおかげで通るがbeliefのno-op分岐で
   止まりうる、(2) 物理配送は無条件Suppress、(3) drift correctionも
   このゲートで走らない——**3つの経路が同時に塞がる**。この窓は
   一時的（grace期間が終われば`is_japanese_ime()`は正答に戻り、
   緩和策(a)は定常的に機能する）だが、「打鍵と無関係に自動回復する」
   という上記の主張はこの窓の間は成立しないことを明記する。
4. **`deferred_vks`への残留は起こらない**（r11の既知の制限6が消滅）:
   センチネルは常にSuppressなので`check_output_guard_defer`に到達
   せず、`deferred_vks`（`transport.rs:110-125`）に入らない。
   `check_keyup_symmetry`への影響も無い。
5. **`to`に`VK_DBE_ROMAN`/`NOROMAN`(0xF5/0xF6)・その他の`VK_DBE_*`
   亜種を割り当てることは引き続き禁止する**: `to`=かなスロットの
   意味は「センチネルへ書き換え・常にSuppress・awase自身がIME ON
   相当をactuateする」に一本化されており、OSへ`VK_DBE_*`を送る経路
   自体が存在しないため、他の値を`to`として選択させる余地が無い。
6. **判別子の健全性**（r11の既知の制限8を継承・簡素化）: 挿入点で
   vkを書き換える機構はAlt impersonationと役割代入の2つのみであり、
   Alt impersonationの書き換え先は`resolve_thumb_key`
   （`alt_impersonation.rs:38-45`）により`VK_NONCONVERT`/`VK_CONVERT`
   に限られる。したがって`vk == VK_ROLE_KANA_ACTUATE`は役割代入由来
   であることの健全な判別子である。r11が必要としていた
   `event.scan_code != SCAN_KANA`という**間接的な**判別子（0xF2が
   物理かなキー由来か役割代入由来かをscanで見分ける）は、値そのものが
   一意になったため不要になった。
7. **`reset_physical_key_state`によるmid-holdの再確定**（r11の既知の
   制限10を継承・影響を縮小）: `reset_physical_key_state()`
   （`hook.rs:334-347`）や`clear_hook_latches_for_app_disable`の
   `SuppressionEdge::Leave`が押下中に走ると`{KEY}_WAS_DOWN`が
   クリアされ、直後のauto-repeat KeyDownがfresh pressと再判定されて
   `kana_role_actuate`が真になる。r11ではこれがラッチの反転
   （Suppress→Allowで生の0xF2が流出）を招いたが、r12では
   `kp_stage_shadow_ime_toggle`のno-op分岐が余分なactuationを吸収する
   （決定2-6）だけで、配送は変わらない。ADR-141が既に持つ性質であり、
   本ADRが新たに導入するハザードではない。
8. **Phase Aの受け入れ条件**: 実装完了後、(a) 代入先キーの押下で実際に
   GJI・MS-IME双方の実IMEがONになること、(b) 代入先キーを押しっぱなしに
   してもIMEが暴れないこと（auto-repeat時の二重actuation防止）、(c)
   代入先キーの押下・auto-repeat・KeyUpのいずれからもOSへ何も送出され
   ないこと（Spy++等でSendInputを観測）、(d) 上記2のcold-start確認、
   の4点を実機で確認する。

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

**エンジンOFF時は「かなスロットが関与するサイクル全体」を無効化する
（2026-09-06追加、architect役NB15・NB16指摘、premortem役R13-M1・
R14-M2の議論を統合）**: `engine_enabled`をどこに置くかで4段階の
試行錯誤があった（詳細はステータス節参照）。最終的に、決定1（`from`
方向）・決定2-2（`to`方向）に個別の`engine_enabled`条件を書き込む
のではなく、**config読み込み/reload時**に以下を計算し、フックスレッド
はその結果（1個の真偽値、以下`kana_cycle_active`）だけを参照する
形に一本化する:

1. 4要素{変換, 無変換, スペース, かな}上の置換（決定3の全単射）を
   サイクル分解する。
2. かなスロットが属するサイクルが**非自明**（かなスロット自身への
   恒等ではない）場合、そのサイクルに属する**すべての要素**（かな
   スロットに直接触れない要素も含む——例: 3-cycle「変換→かな→
   スペース→変換」なら`スペース→変換`の腕も）を「かな関与サイクル」
   としてマークする。
3. `kana_cycle_active = engine_enabled`（このサイクルに属するルールが
   有効かどうかは、エンジンがONの間だけ）。かなスロットを含まない
   独立したサイクル（例: 「変換⇔無変換」の2-cycle、上記3-cycle例で
   言えば無変換の恒等）はこの判定と無関係で、常時有効のまま。

fresh press時点（決定1の`from`判定、決定2-2の`to`判定のいずれか）で、
解決済みの`rule_target`が「かな関与サイクル」に属するルールを指して
おり、かつ`!kana_cycle_active`の場合、`decide_role_substitution`へ
渡す`rule_target`を`None`として扱う（決定3が確立した「恒等は
テーブル引きスキップ」と同じ機構をそのまま使う）。それ以外
（`rule_target`がかな関与サイクルに属さない、またはサイクルが
アクティブ）は通常どおり処理する。

この一本化により以下がすべて解決する:
- **NB15/R14-M2**（`is_kana_slot_press`に直接`engine_enabled`を
  織り込むと押下中のトグルでKeyUpが取り残される）: `is_kana_slot_press`
  自体は`(scan == SCAN_KANA) && event_eligible`のまま変更しない。
  `kana_cycle_active`はfresh press時点の`rule_target`確定判断にのみ
  効き、`confirmed_target`が押下期間中ラッチするため（ADR-141決定2・
  決定4のhold-state）、押下中に`engine_enabled`が変化してもDown/Upは
  常に対称になる。
- **NB16**（かな関与サイクルの一部の腕だけを無効化すると3-cycleで
  全単射が破れる）: サイクル単位で一括無効化するため、サイクルの
  どの腕も個別に生き残らない。全単射は常にサイクル全体で恒等に戻る
  か、サイクル全体で有効になるかのどちらかであり、部分適用は起こら
  ない（ADR-141決定1-2の「個別ルールのskipではなくルール集合全体を
  無効化する」、決定5の`swallow_alt_kana_input_method_switch=false`
  時の措置と同型）。
- **R13-M1**（`to`方向だけ・`from`方向だけを個別にゲートすると非対称
  になる）: 両方向とも同一の`kana_cycle_active`を参照するため、
  対称性は定義から保証される。

**Phase Cへの申し送り**: 「変換⇔かな」を設定すると、同じサイクルに
含まれる他のルール（3-cycle構成の第三の腕）もエンジンOFF中は一緒に
無効化される、という帰結をGUI説明に含める必要がある。安全な3キーの
みで完結する独立したサイクル（かなスロットを含まない構成）は、
ユーザーが確認した製品方針どおりエンジンのON/OFFと独立に動作し続ける。

**既知の制限（Minor、2026-09-06追加、architect役指摘）**: 同一サイクル
内の2キーが、押下中のエンジントグルを跨いで一時的に同じvkを生成しうる
過渡窓が残る。例（3-cycle 変換→かな→スペース→変換）: エンジンONで
スペースを押す（`confirmed_target = Some(VK_CONVERT)`）→押したまま
エンジンOFF→変換を新規押下（`rule_target = None`で恒等→`VK_CONVERT`）
——この間、スペース（ラッチ済みの旧写像）と変換（新しい判定＝恒等）が
同時に`VK_CONVERT`を生成する。これはADR-141決定3がconfig reloadに
ついて既に受け入れている性質（「reload中にルールが変わってもKeyUp時の
変換は確定時点の値を使う」）と同じ形の過渡窓であり、`confirmed_target`
ラッチ規律そのものに内在する——本ADRが新規に導入するものではない。
ただしconfig reloadが設定GUI経由の稀な操作であるのに対し、エンジン
トグルは無変換3連打という打鍵中に起こりうる操作のため、露出頻度は
一段高い。対処にはサイクル単位のラッチ（同一サイクルの他キーが押下中は
再確定を止める）が必要になり実装コストに見合わないため、既知の制限
として記録するに留める。

### 決定4（r2で簡素化、r12で`from`方向専用であることを明確化）: hold-state・KeymapLatchをかなスロット専用ペアとして追加する（`from`=かな方向のみ）

**r12での位置づけの明確化**: 本決定が追加するhold-stateペアは
**`from`=かな方向（物理かなキー→安全な3キー）専用**である。`to`=かな
方向（安全な3キー→かなスロット）には**新しいhold-stateを追加しない**
——その方向で押される物理キーは安全な3キーのいずれかであり、
ADR-141決定2が既に`{KEY}_WAS_DOWN`/`{KEY}_CONFIRMED_TARGET`のペアを
持っているためである。決定2-2のfresh press判定（`is_keydown &&
!was_down`）も、そのADR-141側の既存ペアをそのまま使う。

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

**「安全な3キー→かな」方向にはKeyUp注入を行わない（r12で無条件化）**:
決定2-8のとおり、`{KEY}_CONFIRMED_TARGET == Some(VK_ROLE_KANA_ACTUATE)`
の場合はADR-141決定7のKeyUp注入を**無条件に**行わない。センチネルの
Downは一度もOSへ配送されていないため、Upを注入する意味が無い。

**r0〜r11からの変化**: r2〜r5は「DownがSuppress・UpがAllowになりうる
例外」（InputRelayへのフォーカス遷移〈NM6〉、`kana_role_active`の押下中
反転〈R3-M1〉、auto-repeatでの反転〈NB9〉）を個別に検討し、r6は
fresh pressラッチでそれらの発生条件を消し、r11はさらに「ラッチが
`Some(true)`のときだけ注入しない」という条件付きの形へ訂正した上で、
「そのラッチはメインスレッドの値なので決定7-3（フックスレッド）からは
参照できない＝そこでは常に注入側になる」という但し書きを必要とした
（premortem役R11-M1）。r12ではDown/Up/auto-repeat/injectedのすべてが
無条件Suppressになるため、条件そのものが`{KEY}_CONFIRMED_TARGET`という
**フックスレッド側の値だけ**で完結し、この但し書きが不要になった。
決定5が新設する2つのswallow分岐は`SCAN_KANA_*`（`from`=かな方向、
targetは安全な3キーのいずれかのvkでありセンチネルにはならない）が対象
であり、この規則とは無関係である。

**この例外はADR-141決定7自身への申し送りとしてADR-141本体にも参照
ポインタを追加する**（ADR-142がr5で行ったのと同じ手当て——ADR-143だけ
に書くと、ADR-141を単体で読む実装者が無条件でKeyUp注入を実装して
しまう）。

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
`:1066-1087`）とは別の合流点である（合計5箇所）。**r12訂正
（r7のarchitect役Minor2への回答を差し替え）**: これら5箇所はいずれも
フックスレッドのhold-state（`SCAN_KANA_WAS_DOWN`/
`SCAN_KANA_CONFIRMED_TARGET`）を扱うものである。r7〜r11は「これらは
決定2が新設するメインスレッドのラッチとは別物」という注記を必要と
していたが、r12ではメインスレッドのラッチ自体が存在しないため、この
区別を意識する必要が無くなった。`to`=かな方向のfresh press判定
（決定2-2）が使う`{KEY}_WAS_DOWN`はADR-141決定2の既存ペアであり、
その5箇所でのクリア規律にそのまま従う。

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

- 物理かなキーを押すと代入でvk=VK_CONVERT等に書き換わり、親指キー
  （`right_thumb_vk = VK_DBE_HIRAGANA`）としては認識されなくなる。
  これは「秀Caps的な入れ替え」が意図する挙動そのものであり、バグでは
  ない（ADR-141決定4がr2で「全単射により親指キーの役割は逆写像で
  正しく再配置される、これは要望そのものが意図する挙動」と結論したのと
  同型）。
- **r12訂正（逆方向は成立しなくなった）**: r11までは「`right_thumb_vk
  = VK_DBE_HIRAGANA`かつ『変換↔かな』を設定した場合、物理変換キーを
  押すと代入でvk=0xF2に書き換わり、`update_thumb`が正しくこれを親指
  キー押下として認識する」としていた。r12では`to`=かな方向の書き換え先
  がセンチネル（`VK_ROLE_KANA_ACTUATE`）になるため、`update_thumb`の
  `vk == config.right_thumb_vk`（0xF2との比較）は成立せず、**代入先の
  物理変換キーは親指キーにならない**。センチネルは「NICOLA同時打鍵に
  参加しない、純粋なIMEモード切替」を意味する値なので、これは設計上
  一貫した帰結である（背景節「`to`=かな方向がNICOLA判定に参加しない
  こと」参照）。
- **その帰結として、`thumb_key`に「ひらがな」(0xF2)を選び、かつ
  かなスロットを含む非自明な役割代入を設定した構成では、その親指キーが
  どの物理キーからも到達不能になる**（物理かなキーは代入で別のvkへ、
  代入先の安全なキーはセンチネルへ化けるため）。これは決定9-2が既に
  必須要件としている「意図しない反転をconfig検証時に警告する」対象に
  そのまま該当するので、Phase C（設定GUI）の警告リストにこの組み合わせ
  を加える。**ADR-141決定4のr2の教訓（全面禁止は既定configで機能を
  丸ごと無効化しうる致命的な欠陥になりやすい）に従い、拒否ではなく
  警告に留める。**
- したがって親指キー設定と役割代入設定を同時に有効化することを禁止する
  理由はなく、追加のバリデーション（拒否）は不要と判断する。
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
「代入後vk基準で下流ロジック（既定ホットキー判定〈決定9参照〉）が
一貫して動く」という全単射モデルの性質そのものに委ねる（**r12訂正、
architect役Minor指摘**: 親指キー判定は上記のとおりr12ではセンチネルに
より到達不能になる側なので、この一貫性の主語から外す——親指キー判定
自体は「一貫して動く」のではなく「到達不能になり、Phase Cの警告対象に
なる」という別の扱いである）。

**r12: delegate機構との相互作用は構造的に消滅した（r4のNM10・r5の
Nit1・R4-M2をまとめて解消）**: r4〜r11は、`right_thumb_vk =
VK_DBE_HIRAGANA`かつ「変換↔かな」という構成で、代入後vk=0xF2の押下が
`kp_stage_shadow_ime_toggle`の`delegate_owned`ゲート
（`key_pipeline.rs:1074-1075`）に掛かってshadow-toggle actuationが
スキップされ、「誰も何もしない」状態（BUG-115の元症状）になる可能性に
対処するため、決定2に`ime_actuation_will_fire_before`（後の
`delegate_will_turn_on`）という分岐を必要としていた。

r12では`to`=かな方向の代入後vkがセンチネルになるため:

- `delegate_owned`の実体は`mode_key_delegate_owns_shadow_toggle
  (event.vk_code)`（`runtime/mod.rs:1375-1385`）で、内部で
  `is_configured_thumb_key(vk)`を見る。センチネルは親指キーになりえない
  （上記）ため`delegate_owned`は**構造的に常に偽**である。
- したがって代入先キーの押下は必ず通常のintent経路（決定2-3の
  `sync_direction`）を通り、actuationが発火する。「誰も何もしない」
  状態は発生しない。
- r5のNit1（カタカナ側delegateへの言及）・r5のR4-M2（delegateが代入後
  構成で実際にopen軸をONにするかが未検証）という2件の残課題も、
  delegateが関与しなくなったことで対象自体が消滅した。

### 決定7（r2でほぼ解消、r12でさらに強化）: Shift併用時の懸念は決定2の再設計により大部分が消滅した

r0は「Shift併用時にカタカナへ切り替わらない」という（誤った）既知の
制限を記載し、r1はこれをhook.rs側のmodifierゲート（Shift押下中は代入
不発火）で対処しようとしていた。r2の決定2再設計により、`to`=かな方向は
OSへ生のvkを一切送らなくなったため、「Shift併用でOS/IMEが0xF2を
カタカナ切替と誤解釈する」という問題自体（r0-M2、`shift_katakana_
passthrough`の分岐可能性）が構造的に発生しなくなった。したがって`to`
方向についてはhook.rs側のShiftゲートは不要であり、決定2から削除した。

**r12補足**: この性質はさらに強くなった。r2〜r11では代入後vkが0xF2
だったため、`shift_katakana_passthrough`（`transport.rs:103-109`）の
条件式（`vk_code == VK_DBE_KATAKANA`）にたまたま一致しないことに依存
していた。r12ではセンチネルが`plan()`の先頭で無条件Suppressされるため、
`shift_katakana_passthrough`を含む`plan()`内のどの判定にもそもそも
到達しない。

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

**r23（2026-09-06、ユーザー要望・r22への両エージェント指摘を反映して
全面訂正）: `to`=かな方向でもShift併用時にカタカナへ切り替える**

これまで`to`=かな方向のShift併用は「特別な分岐が無い」だけだった
（fresh pressで`kana_role_actuate`が発火し、Shiftの有無にかかわらず
常にIME ON〈ひらがな相当〉になる）。ユーザーから、「かな↔変換」の
ようなswap設定をした場合に、**代入先の変換キーでもShift併用時に
カタカナへ切り替わってほしい**という要望が出た。

**位置づけ（architect役指摘、r23で追記）**: これは新規のユーザー要望と
いうより、**決定7が既に記録している「Shift併用効果（`shift_katakana_
passthrough`経由のカタカナ入力）が代入によって失われる」という既知の
制限を、代入先の位置へ「移す」ことで復元する機能**である。全単射で
役割が移動するという本ADRの中核の意味論——物理かなキーが失った機能が
どこか別の場所に必ず現れる——と一致しており、決定7の既存の制限と
対になる話として理解すると設計の必然性が通る。

**設計方針（変更なし）**: awase側でconv-mode状態を読み取って次の状態を
計算するのではなく（この挙動の正確なメカニズムはBUG-116自身が「未解明」
と認めている）、**物理Shift+かなキーが実際に生成するのと同じvk
（`VK_DBE_KATAKANA`、0xF1）を合成してIMEへ送り、その後の解釈は完全に
IME自身に委ねる**。ユーザーが明示した原則「IMEの実際の状態を無視して
特定の動作を強制してはいけない」に沿う。

**r22で見落としていた点（architect役NB17・premortem役R22-M1指摘）**:
「既存の`send_ime_mode_key_with_shift_release_prefix`をそのまま流用する」
としていたが、この関数は内部で`vk == VK_DBE_HIRAGANA`かどうかでscan値の
有無を分岐しており（`ime.rs:379-386`）、`VK_DBE_KATAKANA`は`else`側
（`make_key_input_ex`、**scan=0**）に入る。ところが:
- **GJI**: scan=0のままカタカナへ切り替わることをADR-137が2026-09-05に
  実機確認済み（`shift_katakana_passthrough`のdoc、`transport.rs:82-95`）
  → **動く**。
- **MS-IME**: `key_pipeline.rs:1955-1961`の2026-07-07実機記録
  「scan=0の`send_ime_mode_key`ではMS-IME(TSF)がモードキーとして処理
  しない」→ **無反応**の見込み。

自然な直し方（`if`側の条件を0xF1にも広げてscanを付ける）は、
**BUG-15追補7/BUG-61の「scan付きDBEキー注入によるJISかな固着」
ハザードに直行する**（`kp_restore_hiragana_for_suppressed_mode_key`が
同じ経路に`read_kana_lock()`のABORTガードを持つのはこのため）。

**決定（範囲の限定）**: scanは付けない（**GJI限定機能とする**）。
MS-IMEでは本機能は無反応になることを既知の制限として受け入れる
——BUG-61（復旧不能）のリスクを新たに背負うより、機能をGJIに限定する
方が安全側の選択である。**ただし「MS-IMEで無反応になる」という予測は、
`VK_DBE_KATAKANA`(0xF1)自体の実測ではなく`VK_DBE_ALPHANUMERIC`(0xF2)
についての2026-07-07実測（`key_pipeline.rs:1955-1961`）からの類推
であり、Phase A実装時に0xF1でも同様であることを実機確認する**
（architect役指摘）。scan付き注入によるMS-IME対応は、
`read_kana_lock()`等の追加ガードとともに将来の検討課題とし、本ADRの
スコープ外とする。この選択によりBUG-15追補7のガード（`read_kana_lock`）
は不要になる——scan=0の合成注入はそもそもこのハザードの対象外だから
である（`shift_katakana_passthrough`のdocが「scanを付与しないため
BUG-15追補7/BUG-61のハザードを一切踏まない」と明記している対称）。

**呼び出しに必要なガード（既存の類似経路が持つものを再現する、
architect役・premortem役指摘）**:
1. **`conv_mutation_allowed.get()`**（`kp_restore_hiragana_for_
   suppressed_mode_key`と同じゲート、ADR-086の`ConvModeAuthority::
   UserOwned`契約——エンジンuser-disabled中はconv-modeに触れない）。
2. **`ime_mode_key_injection_blocked_by_modifier()`**
   （`win_key_held() || alt_key_held()`）。決定2-9のAlt安全性論拠
   （送出vkがDBE系でない）はこの新経路には適用されない（まさにDBE系
   vkを送るため）ので、この経路専用に必須。**置き場所は`send_ime_
   mode_key_with_shift_release_prefix`関数自身の内部**とする
   （2026-09-06、architect役NM29の再指摘で確定）。`send_input_safe`
   への移設は、物理DBEキーのrelay中継（`reinject()`経由）まで巻き込み
   ADR-119「解釈しない入力は消費しない」に反するため不採用——センチネル
   （awase以外が生成しえない値）を`send_input_safe`にドロップした
   r12のR12-B1とは事情が異なる。一方、呼び出し側テストで代替する案も
   「到達経路の網羅への依存」を残すため見送り、より狭い共通点である
   **この関数自身**（awase自身のactuation専用、relayトラフィックを
   通さない）に置く。具体的には、`ime.rs:339`の既存ガード
   `if crate::hook::win_key_held() { ... return false; }`を
   `if crate::hook::ime_mode_key_injection_blocked_by_modifier() { ...
   return false; }`へ拡張する（Winのみ→Alt/Win両方）。既存の呼び出し元
   （`output/mod.rs:1201`のGJI半角英数トグル出口、`key_pipeline.rs:
   2004`の`kp_restore_kana_from_half_width`）は呼び出し側で既に
   同じ判定を行っているため、この拡張は冪等で挙動が変わらない。この
   1行の拡張で、本機能の新しい呼び出し元だけでなく将来の呼び出し元も
   自動的にカバーされる。**（Minor、2026-09-06、premortem役指摘）**
   これにより既存2呼び出し元では判定が呼び出し側・関数内部の2箇所で
   行われる二重チェックになる（`transport.rs:65-69`が戒める「同じ
   判定を同一イベントに対して2回計算しない」という規律とは軽く矛盾
   するが、挙動は変わらず実害も無い）。実装時に呼び出し側の重複判定を
   削るか、コメントで「関数内部が唯一の判定点である」ことを明記して
   重複を許容するかは実装判断とする。
3. **`half_width_alnum_toggle_active`が真の間は発火しない**
   （ADR-137 M-3と同型の競合——この区間は`kp_restore_kana_from_
   half_width`が独自にscan付き`VK_DBE_HIRAGANA`を注入するため、
   同時に0xF1を送ると競合する）。
4. **`ime_open_before`（`kp_stage_shadow_ime_toggle`実行**前**の
   `effective_open()`スナップショット）でカタカナ送信の可否を決める**
   （2026-09-06、architect役・premortem役が独立に到達したR25-M1指摘で
   最終確定。当初「カタカナ送信の直前に`effective_open()`を確認する」
   としていたが、これは`kp_stage_shadow_ime_toggle`が
   `write_sync_key`でbeliefを**同期的に**書き込んだ**後**に、自分が
   直前に書いた値を読み返すだけの**恒真ガード**になっていた——r4の
   NB7（`effective_open()`のライブ値読み）・NB11（`is_fresh_press`）・
   `half_width_alnum_toggle_active`（ライブ値だと常にすり抜けるとdocが
   警告）に続く4度目の同型の罠。加えて「`effective_open()`は実IMEの
   観測値」という当初の記述自体も誤りで、正しくは**belief**（`ImeModel`
   内部の意図値）であり、実IMEの観測値〈`observed`側〉とは別物
   （両者の乖離は決定2-10のNB6/BUG-10として既に受容している）。

**決定（`_before`スナップショット方式、NM30の解決を兼ねる）**:
`half_width_alnum_toggle_before`と同じ命名規約で、`kp_stage_shadow_
ime_toggle`を呼ぶ**前**に`ime_open_before = effective_open()`を
取得しておく。**命名の注意（2026-09-06、architect役Nit指摘）**:
この名前はr4〜r11で使われ、r12で機構ごと撤回された`ShadowToggleOutcome
{ ..., ime_open_before }`（配送判断`actuation_will_fire`のための
pre-toggleスナップショット）と同名である。値としては同じ
「toggle実行前の`effective_open()`」だが、**消費者が異なる**
（撤回された旧機構は配送判断に使っていたが、本機能はcharset送信の
可否判断に使う）。実装時に変更履歴のr11関連記述と混同しないこと。

- **`ime_open_before == false`**（この押下の前はIMEが閉じていた）:
  この押下の役割は「IMEを開くこと」に限定し、**カタカナ送信は行わない**
  （`kp_stage_shadow_ime_toggle`だけ呼ぶ）。
- **`ime_open_before == true`**（この押下の前から既にIMEが開いていた）:
  `kp_stage_shadow_ime_toggle`はno-op分岐になる（r5のNB6）が、それとは
  独立に**カタカナ送信を行う**。

この決定は、当初NM30が挙げた3択（(a)effectバッチ完了後へ回す、
(b)beliefが元から開いていた場合のみ即送信、(c)順序反転を受容する）の
うち**(b)を採用する形**になる——`ime_open_before`は`kp_stage_shadow_
ime_toggle`実行前の値なので、IME ONのeffectが非同期（ImmCross等）で
未完了のまま先にカタカナが飛ぶ、というNM30が懸念した順序反転の窓
そのものが構造的に発生しない（IMEが閉じていた押下ではカタカナを
そもそも送らないため）。副次的な利点として、この設計はユーザーが
実機で報告した物理Shift+かなキーの挙動「1回目はひらがな、2回目で
カタカナ」を、awase自身がconv-mode遷移ロジックを一切モデル化する
ことなく自然に再現する——1回目は`ime_open_before=false`でIMEを開く
だけ、2回目は`ime_open_before=true`でカタカナが送られる、という
対応になる（ただしこれは結果が一致するだけであり、実IME内部が本当に
同じ機構で動いているという証明ではない——設計判断の根拠はあくまで
「IME ONのeffectと競合しない」という安全性であり、実機挙動との一致は
独立した傍証として記録するに留める）。

**`prepend_synthetic_shift_up = true`、ただし既存関数に1行の修正が
必要（2026-09-06、architect役NB19指摘で最終確定——premortem役の`true`
案・architect役の当初の`false`案は、ともに半分だけ正しかった）**:

- `prepend=false`の場合、Shift解放は`push_release`（held_modifiers.rs:
  52-63）が汎用`VK_SHIFT`（0x10、scan無し）で行う。ところが
  `ime.rs:346-364`のdocが警告するとおり、`MapVirtualKeyW`は汎用
  `VK_SHIFT`から**左Shiftのscan(0x2A)しか返さず**、Windowsの内部
  キー状態は`VK_LSHIFT`のみ更新され`VK_RSHIFT`側は更新されない。
  本機能はユーザーが右Shiftを押している場合も普通にあるため、
  `prepend=false`だと右Shift押下中はOSから見たShiftが押下中のまま
  残り、決定0 M4が確定させた「Shift押下中はDBEキーのKeyDown自体が
  フックに配送されない」条件に抵触し、**送信そのものが不発**になり
  うる。
- `prepend=true`の場合、左右両方のscan付きShift-upを明示的に送るため
  解放は確実になるが、現状の実装（`ime.rs:352`）は
  `held_skip_alt.shift = held.shift && !prepend_synthetic_shift_up`
  としており、`prepend=true`だとこれが`false`になる。`push_restore`
  は`self.shift`（＝`held_skip_alt.shift`）が`false`だと、物理的に
  まだ押されているShiftを**復元しない**（NM27で確認済みの問題が
  そのまま残る）。

**根本原因**: 既存関数は「呼び出し時点で物理Shiftは既に解放済み」
という呼び出し元（左Shiftタップ検出・緊急解除）しか持たなかったため、
「確実な解放」と「正しい復元」を両立させる必要が無かった。本機能
（ユーザーがShiftを押し続けたまま代入先キーを押す）は、**物理Shiftが
押下中のまま呼ばれる初めての呼び出し元**であり、この2つを両立させる
必要がある。

**決定（既存関数への修正を含む）**: `send_ime_mode_key_with_shift_
release_prefix`の`held_skip_alt`の構築（`ime.rs:350-354`）を、
`prepend_synthetic_shift_up`の値に関わらず`shift: held.shift`に変更する
（`&& !prepend_synthetic_shift_up`の条件を削除）。本機能からは
`prepend_synthetic_shift_up = true`で呼ぶ。この修正後:
- `prepend=true`により`push_release`が左右両方のscan付きShift-upに
  加えて汎用`VK_SHIFT`のupも1つ余分に送るが、KeyUpの重複は同ファイル
  他所の既存判断（重複は無害）と同型で実害が無い。
- `push_restore`は`held_skip_alt.shift = held.shift`（修正後は常に
  物理状態をそのまま反映）に基づき、復元時点の実際の物理キー状態を
  正しく見て復元する。
- **既存の呼び出し元（GJI半角英数トグルの出口）への影響**（2026-09-06、
  premortem役指摘で precise 化）: `held.shift == false`の間（＝通常時、
  既存呼び出し元がこの状態にある大多数のケース）は無影響。ただし
  「呼び出し時点でまだ物理的にShiftが押されているレース」が仮に
  起きた場合、修正前は復元されなかったものが修正後は正しく復元される
  ようになる——これは既存呼び出し元にとっても悪化ではなくより正しい
  方向への変化である。

この修正自体もfix-requires-evidence.mdの再発ファミリー
（force-write/actuationターゲット、`ime.rs`）に該当するため、
実装時に回帰テスト（「物理Shiftを保持したまま呼んだ場合に正しく
復元されること」「既存呼び出し元の挙動が変わらないこと」の両方）が
必要。

**さらに1点、上記修正後も残る問題（2026-09-06、premortem役R23-M1
指摘）**: `push_restore`自体の復元送信（`held_modifiers.rs:87-89`）は
`make_key_input_ex(VK_SHIFT, false, marker)`——**汎用VK_SHIFT**であり、
左右どちらの物理キーが押されていたかを区別しない。ユーザーが右Shiftを
押していた場合、`push_release`側は`prepend=true`のLSHIFT/RSHIFT両方の
scan付きupにより両方確実に解放されるが、**`push_restore`が送る復元は
汎用VK_SHIFTのdown**であり、`MapVirtualKeyW`の性質上これは内部的に
「左Shiftが押された」と記録される。ユーザーが実際に離すのは右Shiftな
ので、この合成的な「左Shift押下」に対応するupは永久に来ず、**左Shiftが
OS内部でstuckする**（ADR-141決定7が繰り返し警戒してきた「stuck-trueは
危険側」）。

**決定（restoreも左右を区別する）**: 本機能専用に、`HeldModifiers::read()`
が既に判定している`is_physical_key_down(VK_LSHIFT)`/
`is_physical_key_down(VK_RSHIFT)`を使い、呼び出し時点でどちらの物理
Shiftが押されているかを記録しておく。解放は`prepend=true`のとおり
両方をscan付きで送ってよい（重複解放は無害）が、**復元は記録しておいた
側だけを`make_scan_key_input`で送る**（復元時点で該当側がまだ物理的に
押されているかを再確認したうえで）。これは既存関数の2つのモード
（`prepend=true`/`false`）のどちらにも無い第3の振る舞いのため、
`send_ime_mode_key_with_shift_release_prefix`に第3のモードを追加する
か、本機能の呼び出し側で個別に組み立てるかは実装判断とするが、
**「解放は両側可、復元は実際に押されていた側のみ」という要件は
両立させる必要がある**ことをここに明記する。両Shiftが同時に押されて
いる稀なケース（例: 両手でShiftを押しながら片手で代入先キーを押す）
では両側とも復元対象になる。

**判定を行うスレッド（premortem役R22-m1指摘）**: `event.modifier_
snapshot`は決定1の挿入点（`hook.rs:1137`付近）には存在せず
（`hook.rs:1173`で構築される）、r4のNB7/R4-M2で2度踏んだのと同じ
「ライブ値/挿入点に存在しない値」の罠になる。したがって本機能の
Shift判定は**メインスレッドの`kp_run_inner`側**で行う——
`event.modifier_snapshot.shift`と、その押下が実際に`to`=かな方向の
fresh pressだった証跡（`event.ime_relevance.sync_direction ==
Some(ShadowImeAction::TurnOn)`、決定2-3が載せる値）の両方を条件にする。

**決定9-1への追記（premortem役R22-m2指摘）**: `VK_DBE_KATAKANA`は
`vk_may_mutate_conv`が真を返すvk範囲（0xF0-0xF6）に含まれるため、
本経路は`send_input_safe`の`conv_mutation::bump()`を通る——決定9-1が
「センチネル方式では役割代入由来のイベントがconv_mutationゲートを
通らなくなった」と整理していたことへの例外が1件増える。
`fix-requires-evidence.md`の「conv mode」再発ファミリー該当のため、
実装時に回帰テストか`known-bugs.md`記録が必要。

**決定2-10項目1・決定7の既知の制限との整合（NM26/R22-M2指摘）**:
決定2-10項目1は「代入先キーはopen軸のみを操作し、charset軸の復帰は
再現しない」としていたが、本決定はcharset軸（カタカナ）を**GJI限定で**
操作する専用経路を追加するものであり、決定2-10項目1が言う「BUG-25/
ADR-107の制約〈GJI/MS-IMEでexit実装を共有しない〉を満たす専用経路」に
該当する（GJIとMS-IMEで実装を分けずに済んでいるのは、本機能が
**MS-IME側では発火しない**——scanを付けていないため——ことの裏返しで
あり、両IMEで同一の挙動を共有しているわけではない）。決定2-10項目1・
決定7の既知の制限は、「open軸のみ」「Shift併用効果は再現されない」を
それぞれ「GJIに限りcharset軸〈カタカナ〉も操作する。MS-IMEは対象外」
「`to`=かな方向はGJIに限りShift併用でカタカナ相当を再現する」と訂正
する。

**未解決の疑問（実装前に確認が必要、次回レビューで検証）**:
1. GJI限定機能であることをPhase Cのユーザー向け説明（GUI）にどう
   表現するか。
2. `from`=かな方向のhold-state（決定4）との相互作用——`to`方向の
   fresh press判定は安全な3キー側の`{KEY}_WAS_DOWN`を使うため直接の
   衝突は無いはずだが、次回レビューで確認する。
3. **【解決済み、architect役NM30→R25-M1の`_before`スナップショット
   方式で解決】IME ONとカタカナ送信の順序保証**: 決定7-r23の項目4
   参照。`ime_open_before`（toggle実行前のスナップショット）で
   分岐することにより、IME ONのeffectが非同期で未完了のままカタカナが
   飛ぶという順序反転の窓が構造的に発生しなくなった（IMEが閉じていた
   押下ではカタカナをそもそも送らない）。

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
   ——ただし**r12で理由が変わった**。rule table上の0xF2は「かなスロット
   というオブジェクトの識別子」であり、`to`=かな方向で実際に書き換え先
   となるvkは`VK_ROLE_KANA_ACTUATE`（決定2-1）である。したがって
   `rule_target`をそのまま書き換え後vkにするのは`from`=かな方向
   （target=安全な3キーのvk）だけで、`to`=かな方向はセンチネルへ差し替える。

   **それでも0xF2に一本化する理由**（r12でも有効）: (a) 決定3の単射
   チェックは「4つのオブジェクトの識別子」の上で行うため、かなスロットが
   常に同じ1つの値で表される必要がある（0x15と0xF2が混在すると、同じ
   物理キーが2つの別オブジェクトとして単射チェックをすり抜ける、下記2）。
   (b) `from`=かな方向のルールで、対になる安全なキー側の`to`が
   「かなスロット」を指す場合の照合も同じ値で行われる。(c) `VK_KANA`
   (0x15)を格納すると、`from`=かな方向以外の経路で万一この値が
   書き換え後vkとして使われたときに`reinject()`が**VK_KANA(0x15)をOSへ
   送出しBUG-08〈合成VK_KANAによるかなロック反転〉を踏みうる**
   ——r12設計では`to`=かな方向がセンチネル化されてこの経路は塞がれたが、
   識別子として0x15を残す積極的な理由も無い。
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

**確認済み事項（r2追加、r12で範囲を拡大して再確認）**:
`classify_key`（`hook.rs:42-63`）は挿入点直後で`(vk, scan)`の両方を
消費するが、かなスロット・センチネルのいずれについても安全であることを
確認済み。

- **かなスロット（`from`=かな方向）**: `scan_to_pos_jis`（現行JIS
  テーブル）にscan 0x70のエントリが無いため`Char`には落ちず
  `Passthrough`になり、0xF0-0xF2は`is_passthrough`のVK範囲
  （0x70..=0x87、VK_F1〜VK_F24の意味）の外にある。
- **センチネル（`to`=かな方向、r12追加）**: `VK_ROLE_KANA_ACTUATE`は
  `is_passthrough`（`vk.rs:243-263`）の列挙に無いが、続く
  `scan_to_pos(model, scan)`が`None`を返すため`Passthrough`になる。
  `scan`は書き換えられず元の物理scanのまま届き、変換=0x79・
  無変換=0x7B・スペース=0x39はJIS/USどちらのテーブルにも存在しない
  （`crates/awase-windows/src/scanmap.rs`、JISは`0x02-0x35`＋
  `0x73`/`0x7D`、USは`0x02-0x35`のみ）。
- ただしこれは現行スキャンテーブルの内容に依存した安全性であり、
  JIS/USいずれかのテーブルが将来scan 0x70/0x79/0x7B/0x39を持つように
  なった場合は再検証が必要である。**この不変条件をユニットテストで
  固定する**（`scan_to_pos(model, scan).is_none()`をこの4つのscanと
  両モデルについてアサートする、Linux実行可能）。
- **r12訂正（r3のR2-m3への回答を差し替え）**: r3〜r11は「親指キーが
  `VK_DBE_HIRAGANA`に設定された構成では、代入後の0xF2は`LeftThumb`/
  `RightThumb`に分類される（決定6の論拠そのもの）」としていたが、
  r12ではセンチネルが親指VKと一致しないため`Passthrough`になる。
  決定6のr12訂正を参照。
- **「NICOLA判定に参加しない」の意味の明確化（2026-09-06追加、
  premortem役R12-m2指摘）**: 「`to`=かな方向はNICOLA同時打鍵判定に
  参加しない」とは、**同時打鍵の構成キーになりえない**という意味で
  あって、`Passthrough`キーとしてpending状態のflush契機になること
  まで否定するものではない（物理変換キーを押した以上、他の保留中の
  同時打鍵候補をflushする挙動はむしろ自然）。この区別は決定2-4の
  Blocker（R12-B1、`KeyLifecycle`のpending KeyUpに載りうる経路）と
  直結するため、ここでは「NICOLA判定への不参加」と「OSへの配送」を
  混同しないこと——後者は決定2-4・`reinject()`側のガードのみが
  保証する。

**テスト・実装計画（r12で全面的に書き直し）**: r2〜r10で積み上げた
テスト項目の大半は、対象となる機構（ラッチ・`kana_role_active`・
`role_substitution_fresh_press`フィールド・`kp_restore_hiragana_for_
suppressed_mode_key`の除外条件）がr12で消滅したため不要になった。
r12設計で必要なのは以下である。

**(A) `transport.rs::plan`の純粋関数テスト（Linux実行可能、
`cargo test -p awase-windows --lib`）**:

1. `vk_code == VK_ROLE_KANA_ACTUATE`のKeyDown/KeyUp/injected=true/
   `AppImeProfile::InputRelay`/`AppImeProfile::ImmCross`/
   `is_tsf_mode`・`f2_warmup_owned`の全組み合わせで**常に`Suppress`**
   を返すこと（決定2-4の無条件性の回帰防止。特にInputRelayとの
   評価順序——センチネル分岐が先に来ること——を名指しでテストする）。
2. センチネルを使わない既存の全ケースが**ビット単位で従来どおり**
   であること（`DbeModeKeyContext`にフィールドを増やさない設計なので、
   既存テストが無改修で通ることがそのまま証拠になる。既存テストの
   期待値を1つも変更していないことをレビューで確認する）。
3. `suppress_reason`がセンチネルに対して新ラベル（`"kana-role"`）を
   返し、`VK_DBE_HIRAGANA`に対しては従来どおり`"tsf-f2"`を返すこと。

**(B) hook.rs側（`windows-build` CI対象、`#[cfg(windows)]`）**:

4. `decide_role_substitution`が`to`=かなのルールに対してセンチネルを
   返すこと（純粋関数部分はLinuxでもテスト可能なら含める）。
5. `kana_role_actuate`の3ケース——「離してから再押下で真」
   「auto-repeat KeyDownで偽」「KeyUpで偽」——を、ADR-141決定2の
   `{KEY}_WAS_DOWN`更新規則に依拠する形でテストする（決定2-2）。
   規則が将来変わった場合に静かに壊れないための固定。
6. `build_raw_key_event`がセンチネルに対して
   `sync_direction = Some(TurnOn)`（fresh pressのとき）／`None`
   （auto-repeat・KeyUp・非eligibleのとき）を載せ、`shadow_action`は
   **常に`None`**であること（決定2-3）。
7. `event_eligible`が偽（injected／alt impersonated）のとき
   `kana_role_actuate`が偽になること。
8. **【R12-B1対応、2026-09-06追加】`RawKeyEventExt::reinject()`が
   `vk_code == VK_ROLE_KANA_ACTUATE`のとき、呼び出し元（`physical`の
   値、effect列経由のいずれか）に関わらず実際の`SendInput`を一切
   発行せず`true`を返すこと。`windows`クレート型（`INPUT`）に依存する
   ため`#[cfg(windows)]`・`windows-build` CI対象。この1テストが
   R12-B1（複数の到達経路の網羅ではなく単一の絞り点での保証）の
   回帰防止そのものである。

**(C) 決定4（`from`=かな方向）**: 従来どおり
`SCAN_KANA_WAS_DOWN`/`SCAN_KANA_CONFIRMED_TARGET`の4要素スロット
混線テスト（本節の冒頭で述べたADR-141決定2の4要素版）と、決定5の
swallow分岐でのKeyUp注入テストが必要。加えてr12では、
`{KEY}_CONFIRMED_TARGET == Some(VK_ROLE_KANA_ACTUATE)`のときADR-141
決定7のKeyUp注入が**発火しない**ことをテストする（決定2-8）。

**(D) 波及範囲（r8〜r10の指摘のうち、r12でも有効なもの）**:
`RawKeyEvent`への**新フィールド追加が無くなった**ため、r8のMinor1／
r10のR9-m3が挙げていた波及（journal replay基盤・`golden_scenarios.rs`
・各種`RawKeyEvent`リテラル構築箇所の機械的更新、`journal.rs`の
`KeyInput`のserde後方互換、`architecture_guard.rs`/
`layer_boundary_guard.rs`への影響）は**すべて発生しない**。r12で新たに
波及するのは`vk.rs`への定数1つの追加と、`transport.rs`の分岐1つ・
`suppress_reason`の1アーム・`build_raw_key_event`の特別扱い1箇所
だけである。ただし`VkCode`に0xFF超の値が入ることは初めてなので、
`vk.rs`の既存テスト（`0x00u16..=0xFF`を全走査する`to_char`不変条件
テスト等、`vk.rs:819-822`）がセンチネルを走査対象に含めていないこと
（含める必要が無いこと）を確認する。

### 決定9（r12で大幅に簡素化）: 代入後vkを基準に評価される下流の合流点を棚卸しする

premortem役M6指摘を受けて始めた棚卸し。**r12でセンチネル設計に転換した
結果、`to`=かな方向についてはこの節の大半が不要になった**——センチネルは
下流のどの既存判定にも一致しない値として選んであり（決定2-1）、
「一致しないこと」が設計の目的だからである。以下は`from`=かな方向
（物理かなキー→安全な3キー、こちらは従来どおり実在するvkへ書き換えて
OSへ配送する）を中心とした棚卸しに縮小した。

1. **`vk_may_mutate_conv`（`vk.rs:187-194`、ADR-084/086 conv mode force
   policyの再発ファミリー）**: `VK_CONVERT`(0x1C)・`VK_KANA`(0x15)・
   0xF0-0xF6は`true`、`VK_NONCONVERT`(0x1D)・`VK_SPACE`は`false`。
   実際の露出点は`win32.rs:169`の`send_input_safe`が参照する
   conv_mutationゲートであり、`reinject()`が渡す代入後vkを見る。
   - **`to`=かな方向（r12で解消）**: 代入先キーは`plan()`が無条件
     Suppressするため`reinject()`されず、`send_input_safe`にも到達
     しない。「無変換/スペース→かなで、その物理キーが代入後に
     conv-mutatingへ反転する」というr2の懸念（architect役Minor2）は
     **発生しない**（センチネル自体も`vk_may_mutate_conv`の対象外だが、
     そもそも送出されないのでこの事実に依存する必要すら無い）。
   - **`from`=かな方向（引き続き要検証）**: 「かな→無変換」または
     「かな→スペース」では、物理かなキーがconv-mutating（0xF0-0xF2）
     から非conv-mutating（`VK_NONCONVERT`/`VK_SPACE`）へ反転する。
     この**片方向の**反転をADR-086のforce policyが正しく扱えるかは
     未検証であり、実装前に`state/conv_mode.rs`・
     `runtime/conv_actuation.rs`との相互作用を確認する必要がある
     （`.claude/rules/fix-requires-evidence.md`の「conv mode」ファミリー
     に該当、未解決の疑問7）。r11時点の「双方向の反転」から
     「片方向のみ」へ検証範囲が縮小した。
2. **既定ホットキー（`Ctrl+変換`=IME ON、`Ctrl+Shift+変換`=engine ON等）・
   `runtime/focus_tracker.rs::enrich_ime_relevance`のper-app sync key
   （ADR-141決定6-2）・`vk::is_composition_confirm_key`（`vk.rs:319`、
   対象は`VK_SPACE`/`VK_RETURN`/`VK_ESCAPE`）**: これらはいずれもVK値で
   判定されるため、決定3の全単射制約が保たれる限り、判定対象は
   「代入後にそのVKを生成する物理キー」へ自動的に追従する（決定6と
   同型の論拠——2-cycleなら入れ替わった相手キーで、3-cycle以上でも
   巡回先のキーで、必ずどこかの物理キーがそのVKを生成し続ける）。
   したがって**機能そのものの喪失は起こらない**（全単射性による構造的
   保証）。**r12補足**: センチネルはこれらのどのVK集合にも属さないが、
   全単射により「かなスロットへ写る安全なキー」はちょうど1つであり、
   その1つが持っていたVK（例:`VK_CONVERT`）は逆写像の相手キー
   （物理かなキー）が生成し続ける。したがってセンチネル導入は
   この構造的保証を弱めない。

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
   へそのまま拡張する。**r12追加**: 決定6が新たに特定した「`thumb_key`
   ＝ひらがな × 非自明なかな役割代入 → その親指キーが到達不能になる」
   組み合わせも、この警告リストに含める。GUI文言の検討は未解決の疑問3と
   統合。
3. **`AppImeProfile::ImmCross`への漏洩は起こらない**（r12で確定、
   r2訂正の結論をさらに強化）: r1決定9-3はこれを本ADR固有の新規
   ハザードとして扱い`RawKeyEvent`への出自フィールド追加まで検討課題に
   挙げていたが、これはスコープクリープだった（**r2訂正、architect役
   NM5・premortem役R1-M6指摘**）。r12では、`to`=かな方向のイベントは
   `plan()`の先頭でSuppressされ、ImmCrossアーム（`profile.
   can_use_imm32_cross_process()`）にそもそも到達しない。
   `from`=かな方向は代入後vkが安全な3キーのいずれかになり、これらは
   `shadow_action`を持たないため`is_kanji_event`が偽で`Allow`される
   ——これは代入前の物理変換キー等とまったく同じ扱いであり、
   `feedback_immcross_owns_kanji`が守ろうとした「ImmCrossアプリには
   物理IMEキーを見せない」という原則は**むしろ強化される**（物理かな
   キーがIMEキーとしてImmCrossアプリへ届く経路が1つ減る）。
   物理かなキー自身の押下（代入なしの場合）についての既存の性質は
   本ADRのスコープ外である。
4. **`kp_restore_hiragana_for_suppressed_mode_key`（BUG-116決定2、
   `key_pipeline.rs:81-201`）は改修不要**（r12でr11の既知の制限2が
   消滅）: 同関数は`event.vk_code != VK_DBE_HIRAGANA`で即return
   （`:90`）する。センチネルは0xF2ではないため、役割代入由来の
   イベントはこの関数に一切触れない。r11は代入後vkが0xF2だったため、
   同関数のコメント（`:104-108`）が明記する「`physical == Suppress`
   かつ0xF2は`plan()`のF2分岐でのみ成立し、それは`is_tsf_mode &&
   f2_warmup_owned`の必要十分条件である」という同値性を壊し、MS-IME
   環境でGJI専用のscan付き0xF2注入（BUG-25/ADR-107が禁じる「GJI/MS-IME
   でexit実装を共有しない」原則の違反、BUG-15追補7のかなロックトグル
   ハザードの当事者）を誤発火させるため、`event.scan_code ==
   SCAN_KANA`という除外条件の追加を必要としていた。r12ではこの同値性が
   そのまま保たれるため、**同関数には一切手を入れない**。
5. **`kp_stage_execute`の`composition_native_f2_down`（`key_pipeline.rs:
   2236-2244`）は発火しなくなる**（r12で新規に特定）: 同じく
   `event.vk_code == VK_DBE_HIRAGANA`のKeyDownを条件とする。r11設計
   （代入後vk=0xF2）では代入先キーの押下でも`mark_cold` + eager warmup
   が走っていたが、r12では走らない。決定2-10の確認項目2として
   Phase A実装時の実機確認に回す。
6. **`should_upgrade_is_japanese_ime`（ADR-093）は意図的に対象外**:
   決定2-3参照。センチネルを対象に含めてはならない。

## 未解決の疑問

1. **【解決済み、2026-09-06】かなスロットの`from_name`表現**: 決定8-1〜
   8-4で解決した（config記号は`"VK_DBE_HIRAGANA"`のみ受理、`"VK_KANA"`/
   `"かな"`/`"カナ"`/`"Kana"`と0xF0/0xF1/0xF3-0xF6は拒否、rule table
   格納値は0xF2、拒否・正規化は単射チェックより前）。GUI表示ラベルの
   文言のみPhase Cへ申し送り。
2. **`awase-settings`の`THUMB_KEY_OPTIONS`とkey_role設定GUIの表示上の整合**:
   決定6を撤回した結果、追加のバリデーション（拒否）GUIは不要になったが、
   ユーザーが両機能を組み合わせて設定した場合の挙動（決定6の説明）を
   どうGUI文言で伝えるかはPhase Cの課題として残る。**r12で範囲拡大**:
   決定6が新たに特定した「`thumb_key`＝ひらがな × 非自明なかな役割代入
   → その親指キーが到達不能」の警告文言もここに含める。
3. **かなスロットを`to`にする代入・既定ホットキーの再配置についてのユーザー
   向け説明**（決定9-2と統合）: 「変換→かな」設定時、変換キー本来のIME
   変換機能は失われるが、既定ホットキー自体は全単射により別の物理キーへ
   自動的に付いていく。この「機能の移動」をユーザーにどう伝えるか
   （GUI文言、Phase C）は未決定。
4. **【r12で解消】MS-IME環境での`reinject()`の`wScan: 0`問題**:
   r1時点では`reinject()`の`wScan: 0`がMS-IME環境で実際にモードキーと
   して処理されるかが未検証だったが、r2以降（そしてr12でも）`to`=かな
   方向はOSへ何も送らない設計であり、この懸念は該当しない。actuation
   経路自体（`ime.rs::send_ime_mode_key_with_shift_release_prefix`が
   `make_scan_key_input`をimportしており、scan付きで送る経路を持つ）は
   本ADRが変更しないため対象外。
5. **物理かなキーの役割代入がIME belief観測に与える影響**: 決定1の
   副作用として記載した、`classify_ime_relevance`経由のshadow belief
   観測が代入後は失われる点について、既存のbelief更新ロジック
   （`state/ime_model.rs`）への実害の有無は未検証。**r12補足**:
   `to`=かな方向については決定2-3の`sync_direction`が代わりに
   `write_sync_key`経由でbeliefを更新するため観測経路は残る。未検証
   なのは`from`=かな方向（物理かなキーが安全な3キーへ化け、
   `shadow_action`をもはや生成しない）のみである。
6. **【r12で大幅に縮小】`kp_restore_hiragana_for_suppressed_mode_key`の
   各ゲートの役割代入対応監査**: r2〜r11は、決定2が同関数の発火条件を
   拡張することを前提に`is_configured_thumb_key`・
   `half_width_alnum_toggle_before`・`kana_mode_restore_key_down`・
   `conv_mutation_allowed`・`is_composition_warm()`複合条件・
   `read_kana_lock()`によるABORT（いずれも`key_pipeline.rs:81-201`の
   範囲内）の監査を必要としていた。r12では同関数に一切触れない
   （決定9-4）ため、この監査は不要になった。**残る確認項目**は
   決定2-10の(2)（`composition_native_f2_down`が呼ばれなくなることの
   実機影響）と、下流actuationゲートの棚卸し（下記8）である。
7. **conv mode force policy（ADR-086）との相互作用**: 決定9-1に記載した
   `vk_may_mutate_conv`の意味論反転が、ADR-086のforce policyの前提を
   壊さないかの専用検証が必要。**r12で範囲縮小**: 検証対象は
   `from`=かな方向の片方向反転（物理かなキーがconv-mutatingから
   非conv-mutatingへ）のみになった。
8. **下流actuationゲートの棚卸し**（r11の決定2「既知の制限3」から
   独立させた）: 決定2-5のactuationは`kp_stage_shadow_ime_toggle`→
   belief更新→`executor.rs::dispatch_ime_set_open`→
   `ime_controller::apply`という既存経路に乗るが、この経路には
   ADR-119の`AppImeProfile::InputRelay`ゲートをはじめ複数の
   preconditionがある（`.claude/rules/fix-requires-evidence.md`の
   「IME actuation合流点」参照）。InputRelayについては決定2-4の
   既知の制限として整理済みだが、それ以外の下流ゲートでactuationが
   落ちる組み合わせが無いかは未検証——belief遷移は起きたが実際には
   誰もIMEをONにしない場合、代入先キーが（OSへも送られないため）
   完全な死にキーになる。Phase A実装の前提条件とする。
9. **（r3で解消、削除）「安全な3キー→かな」方向の実装方式**: r2は
   Suppressされたイベントをactuation経路へ「渡す」実装方式を未確定と
   していた。r12では決定2-3/2-5が実装方式を明示的に確定させたため、
   この疑問は完全に閉じた。

## 旧設計（r0〜r11）の記録

r12で撤回した旧決定2の骨格と、その過程で見つかった知見を、同じ失敗を
繰り返さないために要約して残す（各ラウンドの詳細はステータス節の
「レビュー指摘との対応表」と変更履歴を参照）。

**旧設計の骨格（r11時点）**:

1. hook.rsの挿入点で`to`=かな方向のvkを`VK_DBE_HIRAGANA`(0xF2)へ
   書き換える。
2. 代入後vk基準の`classify_ime_relevance`により`ShadowImeAction::
   TurnOn`が付き、`kp_stage_shadow_ime_toggle`（`plan()`より前）で
   actuationが**既に発火している**。
3. したがって`plan()`が決めるべきは「OSへの二重配送を止めるか」だけ
   になる。しかし「actuationが本当に発火したか」は`plan()`からは
   直接見えないため、`actuation_will_fire = shadow_toggled ||
   delegate_will_turn_on`という近似を作り、`kp_stage_shadow_ime_
   toggle`の戻り値を構造体化して`plan()`へ運んだ。
4. この近似はイベントごとに再評価すると壊れる（KeyUpでは常に偽、
   auto-repeatでは2打目以降常に偽）ため、fresh press時点で
   Suppress/Allowを`Option<bool>`ラッチに確定させ、以後の同一押下は
   それを踏襲する設計にした。
5. fresh press自体をどこで検出するかで4ラウンド（r7〜r10）を要し、
   最終的に「役割代入自身の`{KEY}_WAS_DOWN`から計算し、
   `RawKeyEvent::role_substitution_fresh_press: Option<bool>`という
   新フィールドでメインスレッドへ運ぶ」形に落ち着いた。
6. さらに、機能未使用ユーザーへ影響を広げないための`kana_role_active`
   フラグ（`DbeModeKeyContext`の4つ目のフィールド、SSOT要件つき）、
   物理かなキー由来の0xF2と区別するための`scan_code != SCAN_KANA`
   判別子、injected KeyUpがラッチを迂回する経路への専用Suppress分岐、
   ラッチのクリア規律のinjectedゲート、`kp_restore_hiragana_for_
   suppressed_mode_key`への除外条件、`suppress_reason`の二重管理を
   防ぐ共有ヘルパ、が積み上がっていた。

**なぜ撤回したか**: 上記3〜6はすべて、1が「実在するIME意味論を持つvk」
を選んだことの帰結だった。0xF2は`transport.rs::plan`のF2分岐・BUG-52/116
のガード・`kp_restore_hiragana_for_suppressed_mode_key`・
`vk_may_mutate_conv`・`shift_katakana_passthrough`・delegate機構という
既存の判定群と全面的に絡み合っており、「役割代入由来の0xF2」と
「物理かなキー由来の0xF2」を後段で区別し直す作業が延々と必要になった。
センチネルを使えばこの区別が値そのもので付くため、区別のための機構が
まとめて不要になる。

**旧設計から引き継ぐべき知見（r12設計でも有効）**:

- **r3の教訓**: 新しく依拠する既存経路の安全性は「経路を共有するから
  安全」ではなく「送出される値そのものが危険な組み合わせに該当しない
  から安全」という、検証可能な形で論じる（決定2-9はこの形になっている）。
- **r4〜r8の教訓**: actuation/belief関連の値は「正しい値を指すか」だけ
  でなく「いつ読まれるか（呼び出し前後でどちらのスナップショットか）」
  まで具体的なコード行で検証する。`half_width_alnum_toggle_before`と
  いう命名規約がこのリポジトリの既存の答えである。
- **r9の教訓**: 既存のイディオム（判定式）は流用してよいが、その状態の
  ライフサイクル（誰がいつ、どの範囲をクリアするか）まで一緒に借りては
  ならない。
- **r1/r7/r11の教訓（r12設計の直接の土台）**: スレッド境界を跨ぐ値の
  受け渡しを暗黙に仮定しない。フックスレッドの値は`RawKeyEvent`で
  **運べる**が、メインスレッドの値をフックスレッドから**読む**ことは
  できない。r12設計はこの非対称性を正面から受け入れ、判定と消費を
  同一スレッド内に閉じることで解決している（決定2-2・決定2-8）。
- **r10の教訓**: 「Blockerが無いこと」と「記述が実装者を正しく導ける
  こと」は別の基準である。特に、前ラウンドで入れた仕掛けの正当化が、
  別の変更のついでに書き換えられて意図を失う（NM17）ことに注意する。
- **r12自身の教訓**: 11ラウンドの敵対的レビューは、**与えられた
  出発点の中で**設計を正しくすることには極めて有効だったが、
  出発点そのもの（「合成0xF2を絶対に送ってはならない」という誇張された
  制約と、「`to`方向も`from`方向と同じ構造にする」という無自覚な
  踏襲）を疑うことはできなかった。ラウンドを重ねても複雑さが単調に
  増え続ける場合、個々の指摘に応答するのをやめて前提を読み直す。

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
- r11（2026-09-06）: r10で持ち越した3点（実装順序・NM18・`from_name`
  表現）を解消する過程で、architect役・premortem役との追加協議により
  Blocker相当1件（かなスロットの恒等補完の意味——「恒等＝テーブル引きを
  スキップ」であって「恒等＝正規VkCodeでテーブル引き」ではない、決定3）
  とMajor相当3件（injected KeyUpがラッチを迂回し、対応するDownを持たない
  0xF2のKeyUpがOSへ送出されうる経路——条件を`confirmed_target`から
  ラッチの値そのものへ／例外の置き場所をhook.rsから`plan()`のKeyUp専用
  分岐へ／ラッチのクリアも`!event.injected`でゲート、の3段階訂正）を
  新規に発見・修正した。この時点で「収束」と宣言したが、r12で決定2ごと
  撤回された（詳細はr12の項および「旧設計（r11まで）の記録」節）。
- r12（2026-09-06）: **敵対的レビューではなく、実装着手前の根本再調査に
  よるアーキテクチャ転換**。「レビューを重ねすぎて過度に慎重・複雑に
  なっていないか」というユーザーの指摘を受けて2段階の調査を行い、
  r0〜r11の決定2が置いていた出発点の前提が2点で誤っていたと確定した。
  (1) 「OSへ合成`VK_DBE_HIRAGANA`をSendInputで直接送ってはならない」
  という制約は誇張だった——`ime.rs::send_ime_mode_key_with_shift_
  release_prefix`が既に本番コードでscan付き合成0xF2を送っており
  （`output/mod.rs:1231`から呼ばれるGJI半角英数トグルの出口）、
  そのガードは`ime_mode_key_injection_blocked_by_modifier()`＝
  Win/Alt押下中かどうかだけである。BUG-61が問題にしているのは
  Alt/Win押下中の合成送信であって合成0xF2そのものではない。
  (2) より重要な発見として、`to`=かな方向はNICOLA同時打鍵判定に一切
  参加しない純粋なIMEモード切替であり、`classify_key`より前でvkを
  書き換える必然性がそもそも無かった（`from`=かな方向には必要）。
  r11までの決定2は`to`方向にも`from`方向と同じ構造を無自覚に踏襲した
  結果、0xF2という実在するIME意味論を持つvkを経由させ、下流のF2分岐・
  BUG-52/116ガード・`kp_restore_hiragana_for_suppressed_mode_key`・
  `vk_may_mutate_conv`・`shift_katakana_passthrough`・delegate機構と
  全面的に絡み合わせていた。

  併せて`crates/awase-windows/src/hook_channel.rs`の`HookKeyRing`
  （SPSCリングバッファ、`CAP=1024`）を再確認し、フックが対象キーを
  常に即Suppressして（`hook.rs:1199`）キュー投入だけで応答を返し、
  実際のOS配送はメインスレッドの非同期`reinject()`が行うという構造を
  背景節に明記した。r1・r7・r11が繰り返しぶつかった「スレッド境界を
  跨ぐ値」の問題は、フック→メインの一方向は`RawKeyEvent`で運べる／
  逆方向は読めない、という非対称として整理できる。

  **決定2を全面書き直し**: `to`=かな方向の書き換え先を、実在するIME
  意味論を持つ0xF2ではなく、Windows VK空間の外にある専用センチネル
  `VK_ROLE_KANA_ACTUATE = VkCode(0x0100)`にする（決定2-1）。fresh press
  判定はフックスレッド内で完結し（決定2-2）、`build_raw_key_event`が
  `ime_relevance.sync_direction = Some(TurnOn)`を載せる（決定2-3、
  `IntentKind::SyncKey`を選ぶ理由は`is_japanese_ime()`ゲートを通らない
  こと・意味論的な正直さ・`plan()`の`is_kanji_event`に一致しないこと
  の3点）。`plan()`の先頭にセンチネル専用の無条件Suppress分岐を1つ
  置き（決定2-4）、actuationは既存の`kp_stage_shadow_ime_toggle`
  という単一合流点をそのまま使う（決定2-5、新しいactuation経路は
  作らない）。これにより`actuation_will_fire`・`kp_stage_shadow_ime_
  toggle`の戻り値構造体化・`Option<bool>`ラッチ・`RawKeyEvent::
  role_substitution_fresh_press`フィールド・NM17のフォールバック・
  `kana_role_active`とそのSSOT要件・injected KeyUp専用Suppress分岐・
  ラッチのクリア規律・`kp_restore_hiragana_for_suppressed_mode_key`
  への除外条件・`suppress_reason`の共有ヘルパが**すべて不要**になった
  （対応表は決定2-5）。NM13（押下中のフォーカス遷移でorphan KeyUp）・
  NM18（`{KEY}_WAS_DOWN`のinjected更新規則への依存）・R11-M1
  （クロススレッド参照で実装不可能だった例外）も同時に解消した。

  **他の決定の調整**: 決定4を「`from`方向専用」と明確化し（`to`方向は
  ADR-141決定2の既存hold-stateを使うので新規追加不要）、KeyUp注入の
  抑止条件を`{KEY}_CONFIRMED_TARGET == Some(VK_ROLE_KANA_ACTUATE)`の
  無条件判定へ戻した（決定2-8）。決定6は、代入先キーがセンチネルに
  なるため親指キーとして認識されなくなること（r11の記述と逆になる）と、
  その帰結として「`thumb_key`＝ひらがな × 非自明なかな役割代入」で
  親指キーが到達不能になることをPhase Cの警告対象に加えた。同時に
  delegate機構（NM10・r5 Nit1・R4-M2）は`delegate_owned`が構造的に
  偽になるため対象自体が消滅した。決定9は`to`方向の合流点がほぼ全て
  非該当になったため大幅に簡素化し、代わりに新しく特定した
  `composition_native_f2_down`の不発火（決定9-5）を確認項目として
  追加した。未解決の疑問6を大幅縮小、疑問8（下流actuationゲートの
  棚卸し）を独立させた。旧決定2の骨格と、r0〜r11で得た再利用可能な
  教訓は「旧設計（r11まで）の記録」節に残した。
