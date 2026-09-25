---
id: ADR-199
title: |-
  キーの役割はユーザーのIMEキー設定から逆算する。awaseは原則として受動的に対応し、
  能動的に制御する例外は「IME ON/OFF トグル」の役割のキーと、awase 自身の ime_on/ime_off 設定のキーだけにする（VKで固定しない）
summary: |-
  半角/全角が開閉トグルになるのは Mozc/GJI のプリセットの「キー名×状態」割り当ての結果で、キーの性質ではない。
  それなのに awase は ADR-189/191 の固定セット（0x19/0xF3/0xF4）を VK 基準でトグルとして能動的に書いていた。
  所有者決定（2026-09-24）: (1) 役割はユーザーの IME キー設定から逆算し、原則受動。(2) 能動制御の例外は「IME ON/OFF トグル」の
  役割を持つキーだけ（キー名・VK で固定しない）。(3) 設定を知る手段は `config1.db`・MS-IME 本体のレジストリ・学習表（ADR-195/196）だけ（U7 回答で確定）。
  所有者回答（第2回、決定11〜18）: 全ての開状態で閉じるキーだけトグル（U1）、belief の観測追随は存続（U2）、モード指定で開くキーも
  開閉だけ書く（U3）、0x19 は現状維持を経て既知のトグルへ（U4）、awase 既定の `keys.ime_on/ime_off`（Ctrl+変換/Ctrl+無変換）は
  「awase 自身が actuate する設定」として**残す**（U5 修正・U10 消滅）。`keys.ime_toggle` の既定（`VK_KANJI`）は決定14 の移行と同時に空にする（2026-09-25 確定）、
  無変換/変換は単独タップと解決したときだけ能動（U6）、MS-IME 互換モードの半角/全角は受動（U8）、初期範囲の候補は
  半角/全角・F13〜F24・無変換/変換（U9）。
  要点: プリセットは定数表・動的逆算は CUSTOM だけ・学習表は狭める方向だけ・役割は保持せず打鍵時に求める・`config1.db` 不在は既定プリセット扱い・
  F13〜F24 は「その打鍵の最初の Down で awase が実際に書いたときだけ Down/リピート/Up を Suppress」（ADR-195 追記のラッチを一般化）・
  無変換/変換は ADR-192 決定3b の単独タップ確定点に合流。明示 config と役割由来が同じキーに重なったら config が優先（役割は付けない、Q2 回答で確定）。
  能動制御の例外は (1) IME 設定から逆算した役割トグル、(2) awase 自身の `keys.ime_on/ime_off` 設定（既定 Ctrl+変換/Ctrl+無変換）の2つ。
  未決事項は無い。
status: |-
  **草案（所有者決定反映済み、未決なし）。** 2026-09-24 起草、opus round1〜round7 反映済み。
  2026-09-25 所有者回答（U1〜U6・U8・U9）を決定11〜18として反映し、opus round7（収束・条件付き）の中程度3件（M1〜M3）を反映。
  同日の所有者回答で U7（MS-IME 本体のレジストリは `config1.db` と同様にユーザー設定の一次情報源）を確定。
  同日の所有者回答で U5 を修正（`keys.ime_on`/`ime_off` の既定は空にせず残す。awase 自身が actuate する設定として扱う）。これで U10（旧既定値の移行）は消滅。
  Q2（明示 config と役割が重なったら config 優先・役割なし）も確定。同日の所有者回答で `keys.ime_toggle` の既定（`VK_KANJI`）は空にする（U4 の移行と同時、T14）と確定し、未決事項は無くなった。opus round8 の軽微指摘2件も反映。
  所有者は実装着手を指示していない。実装は未着手。review-2026-09-24-08 の方針（(B) 案、PR #308 で実装済み）を一般化・置換する。
related_adr:
  - "ADR-189"
  - "ADR-191"
  - "ADR-192"
  - "ADR-195"
  - "ADR-196"
  - "ADR-186"
  - "ADR-187"
  - "ADR-197"
  - "ADR-176"
  - "ADR-198"
  - "ADR-141"
---

# ADR-199: キーの役割をユーザーの IME キー設定から逆算する（受動が原則、能動は IME ON/OFF トグルの役割と awase 自身の ime_on/off 設定だけ）

## ステータス

frontmatter の `status` 参照。裏取り基準は worktree `docs/adr-user-keymap-passive`（origin/develop `e3969f40`、PR #306 まで）で、
本文中の `crates/...:NNN` の行番号はこの基準で確認した。**決定11〜18・U10 と、それに合わせて直した箇所の行番号は origin/develop `bdd8f1ee`
（PR #308〈ADR-195 追記の実装〉・#309 まで）で確認した。** `e3969f40` からの差分で `transport.rs`・`runtime/mod.rs`・`key_pipeline.rs` は数行ずれている。

## 背景

### 1. 事実: 「半角/全角＝開閉トグル」はキーの性質ではなく、プリセットの書き方の結果

根拠は Mozc のソース（`google/mozc` master を取得して確認。行番号は取得時点。調査メモ全文はリポジトリ外の scratchpad
`mozc-hankaku-zenkaku-keymap.md`）。

| 層 | 決めるもの | 根拠 |
| --- | --- | --- |
| キーボードレイアウト DLL（kbd106） | 物理キー sc029 → VK（基本 0xF3/0xF4、Alt 付きで `VK_KANJI` 0x19） | Microsoft `Windows-driver-samples` `input/layout/fe_kbds/jpn/106/kbd106.c` L34-36（`T29 \| KBDSPECIAL`）、L548-558 付近（Alt で `VK_KANJI`） |
| IME（Mozc/GJI）の VK→キー名 | 0xF3 と 0xF4 を**同じキー名** `KeyEvent::HANKAKU` に畳む | `src/win32/base/keyevent_handler.cc` L57-60・L315-316 |
| 同上（0x19） | IMM32 モードでは `NO_SPECIALKEY`（コメント: 「IMM32 モードでは IME のキー割り当てに関係なく OS 側で IME を起動する」）、TSF モードでは `VK_DBE_DBCSCHAR` として扱い `HANKAKU` に畳む | `keyevent_handler.cc` L87-93、`src/win32/tip/tip_text_service.cc` L330-337 |
| IME のキー名→コマンド | **状態ごとに**任意のコマンドを割り当てられる。ユーザーがキー設定エディタで変えられる | `src/gui/config_dialog/keybinding_editor.cc` L140-143（半角/全角を `Hankaku/Zenkaku` として入力可）、L144（`Kanji` は非対応） |
| 状態の継承 | Suggestion→Composition、Prediction→Conversion、ZeroQuerySuggestion→Precomposition。その状態の行が無ければ継承元の行が使われる | `src/session/keymap.cc` L762-799（`GetCommandSuggestion` 等） |

プリセットでの半角/全角の割り当ては、`ms-ime.tsv` L42/104/135/146（DirectInput→`IMEOn`、Precomposition/Composition/Conversion→`IMEOff`）、
`atok.tsv` L29/75/95/107（DirectInput→`IMEOn`、それ以外→`CancelAndIMEOff`）。**「トグル」はこの状態別割り当てを並べた結果**である。
ATOK プリセットの変換/無変換は DirectInput→`IMEOn`（L96/L98）、Precomposition→`CancelAndIMEOff`（L108/L111）、
Composition→`Convert`/`ToggleAlphanumericMode`（L30/L35）で、「入力していない開状態では閉じるが、入力中は別の作用」になる。

キー設定エディタでの編集は、選択中のプリセットの TSV をコピーしたところから始まる（`config_dialog.cc` L765-788 `EditKeymap()`）。
`session_keymap == CUSTOM` のときは `custom_keymap_table` **だけ**が使われ、プリセットに重ねるのではなく置き換える
（`src/session/keymap.cc` L169-194、`ApplyPrimarySessionKeymap`）。プリセットの中身は GUI から変えられない（変えると CUSTOM になる）。
よってカスタム表には、半角/全角を変えていなくても `Hankaku/Zenkaku` 行が残る（ソースからの推論。実ファイルでは未確認、T1(a)）。
`Kanji`・`ON`・`OFF` の行はキー設定エディタに表示されない（`keymap_editor.cc` L125-127）。

MS-IME（新しいバージョン）で割り当てを変えられるのは、無変換・変換・Ctrl+Space・Shift+Space の4つだけで、半角/全角は固定
（`crates/awase-windows/src/msime_key_assignment.rs` 冒頭 L22-29 のレジストリ位置、および調査メモの外部資料）。
「以前のバージョン」（互換モード）の詳細キー設定は任意のキーに機能を割り当てられるが、awase は半角/全角について読んでいない（ADR-197）。

### 2. 事実: 現行 awase は役割を VK で決め打ちしている（逆算していない）

**能動的な書き込み（`shadow_action` の付与）は2箇所で、どちらもユーザーのキー設定を見ない**
（`crates/awase-windows/tests/architecture_guard.rs:802` `ime_relevance_shadow_action_writes_are_accounted_for` が2箇所に固定）:

- `hook.rs:272-299` `classify_ime_relevance` が `vk.rs:152-165` `ImeKeyKind::shadow_effect` から初期値を付ける。**修飾キーを見ない**。
  `VK_IME_ON`(0x16)→`TurnOn`、`VK_IME_OFF`(0x1A)→`TurnOff`、`VK_KANJI`(0x19)→`Toggle`（`vk.rs:156`）。**IME 種別に依らない**（ATOK・未検出でも付く、`vk.rs:181-184`）。
- `runtime/mod.rs:551-578` `enrich_ime_relevance` が、`event.vk_code.ime_kind()` が `Some`（`:564-566`、`ImeKeyKind` は Kana/Junja/Dbe*/Kanji/ImeOn/ImeOff だけで
  F キー・変換・無変換を含まない）、無修飾（`:567-569`）、`tsf_obs().table_ime_kind()` が `Some`（GJI か CLSID 同定済み MS-IME 本体。ATOK・未同定は `None`、
  `tsf/observer.rs:383-392`）で `is_open_toggle_for(ime)`（`vk.rs:189-196`、0xF3/0xF4 のとき真）なら `shadow_action = Some(Toggle)` を付ける。

`shadow_action` を持つキーは `runtime/transport.rs:174-344` `PhysicalKeyDisposition::plan` で Suppress されうる
（ImmCross では常に、それ以外では `ime_actuation_owned` かつ〈`shadow_toggled` または `is_dbe_mode_key_down`〈`:329-333`、0xF3/0xF4 の KeyDown〉
または KeyUp〉、`:293-340`）。`ime_actuation_owned` は `active_ime_kind` から求めるので ATOK（kind は `MicrosoftIme`）でも真になる
（`:305-307`、`key_sequence_policy.rs:55-57`）。`shadow_action` の無いキーは `:293-296` で即 `Allow`。無変換/変換は `:282-291` で `shadow_action` より**先に**判定され、
明示 config が消費した打鍵以外は `Allow`。

別経路の能動制御として、awase 自身の設定 `keys.ime_toggle`（既定 `["VK_KANJI"]`、`src/config.rs:583`）・`keys.ime_on`（既定 `Ctrl+変換`、`:581`）・
`keys.ime_off`（既定 `Ctrl+無変換`、`:582`）を Engine が消費して冪等な開閉要求にする（`src/engine/engine.rs:975` `apply_special_key_match`、`ImeToggle` は `:994-1001` で `!ctx.ime_on`）。
これらはいずれも awase の既定値で、ユーザーが明示した設定ではない。

MS-IME 本体の Ctrl+Space/Shift+Space にトグル（レジストリ値 2）が割り当たっていれば、それを Engine の自動トグルキーに加える
（`runtime/message_handlers.rs:878-887` `sync_ime_toggle_auto_detect`、`msime_key_assignment.rs:56` `to_combos`）。
これは「ユーザーの IME 設定から役割を逆算して awase が能動制御する」既存の唯一の実例で、形は本 ADR の方針と同じ。
**ただし適用条件が広すぎる**: 呼び出し条件は `ime_kind_detected() && active_ime_kind() == MicrosoftIme` だけ（`message_handlers.rs:939-942`、
`app/mod.rs:811-817`）で、`ms_ime_native_identified()` を見ない（`MicrosoftIme` は「GJI 以外」の意味で ATOK・Japanist・未知の TIP・IMM32 HKL も含む、
`key_pipeline.rs:1950-1953`）。自動キーの本番の設定箇所はこの1箇所だけで種別が変わっても消えず、さらに MS-IME 本体 ⇔ ATOK の切り替え（kind は同じ `MicrosoftIme`）では
`WM_IME_KIND_CHANGED` が post されない（`tsf/gji_monitor.rs` の post は kind 変化時 `:393-395`・`:435-437` と起動時 `:358-364` だけで、同定の更新 `:407-409`・`:442-444` では post しない）。
そのため MS-IME で Ctrl+Space をトグルにしていたユーザーが ATOK/GJI に切り替えると、Engine が Ctrl+Space を消費し続ける（決定1に反する。決定10）。

**予測（受動側）はユーザー設定を部分的に見ている**:

- `state/key_effect_predictor.rs:534-557` `KeyEffectKeymap::from_config` は `session_keymap`・`custom_keymap_table`・`overlay_keymaps` からプリセットを選ぶ。
- `:603-634` `predict_with_override` は、採用済み学習表にセルがあればそれを最優先し、無ければ `custom_table_overrides`（`:678-688`）が真のキーを予測しない。
- `custom_table_overrides` は「そのキー名の行が**ある**か」だけを見て、コマンドの中身を見ない。`mozc_tokens`（`:660-674`）は 0xF3/0xF4 を
  `hankaku`/`zenkaku`/`hankaku/zenkaku` と照合するが、**0x19 は `&[]`**（照合しない）。
- ただし `shadow_action` を持つキーは予測経路から除外される（`runtime/key_pipeline.rs:1874`、`:1929`）ので、**固定セットの 0x19/0xF3/0xF4 では予測もカスタム表も使われない**。
- 学習表のキーは `TableKey` の13種（`key_effect_predictor.rs:98-114`: Bs/Eisu/Enter/Esc/HankakuZenkaku/Henkan/Hiragana/ImeOff/ImeOn/Kanji/Katakana/Muhenkan/Space）に固定。
  F キーや修飾付きキーのセルは無い。

**コマンドの中身から役割を分類する関数 `awase-gji-config/src/keymap.rs:131-151` `extract_ime_keys` は既にあるが、診断にしか使われておらず**
（不具合報告 `message_handlers.rs:1429`、`gji_charset_autodetect.rs:172-227`、本番未使用の `state/keymap_initial_hypothesis.rs:106`。
`gji_charset_autodetect.rs:86-87` の doc の「較正結果の保存」は ADR-198 決定3 の撤去で古い）、そのままでは役割判定に使えない（代替案 D）。

まとめると、**現行実装は「役割をユーザー設定から逆算」できていない**。予測は設定を部分的に見るが（行の有無だけ）、能動制御は VK 固定である。

### 3. 事実: 0x19 は IME のキー設定から役割を逆算できない可能性がある

IMM32 経路では、0x19 は IME のキー割り当てに関係なく OS 側で開閉する（Mozc のコメント、背景1の表）。TSF 経路でも、同梱表の実測の disp
（未確定文字の扱い）が `Hankaku/Zenkaku` 行と食い違うことから OS 側で開閉していると推定する（GJI の ATOK プリセットで変換中の 0x19 は「閉・**確定**」〈`key_effect_table.rs:23`〉、
0xF3 は行どおり「閉・破棄」〈`:18`、`atok.tsv` L29/L75 は `CancelAndIMEOff`〉。確認は T1(b)、扱いは決定14）。
開閉（役割の判定）では 0x19 と 0xF3 は一致する。Windows では 0x19 が `KeyEvent::KANJI` にならない（`keyevent_handler.cc` L87-93）ので、
プリセットの `Kanji` 行は使われない行である。

### 4. 直近の関連決定

- review-2026-09-24-08: 「採用中の学習表でセルがトグル以外を示すときだけ 0xF3/0xF4 の固定を外す」（(B) 案）、0x19 は現状維持。
  前提だった「学習表が採用されない」問題は 01（PR #305）で解消済み。**本 ADR は 08 を一般化・置換する**（固定セットを「外す」のではなく、最初から役割で付ける）。
  (B) 案は起草後に PR #308（ADR-195 追記）で実装された: `runtime/mod.rs:620-661` `learned_table_omits_hz_toggle` が採用中の GJI 学習表を見て
  0xF3/0xF4 の `Toggle` を外し、その判定を KeyDown で確定して同じ物理キーの KeyUp・自動リピートまで持ち越すラッチ
  `hz_toggle_omit_latch`（`runtime/mod.rs:340`、`(scan_code, bool)`）と純関数 `omit_latch_step`（`state/key_effect_runtime.rs:573`）を足した。
  決定18 はこのラッチを F13〜F24 にも使う。
- review-2026-09-24-07 / ADR-198 決定3: ADR-176 手動較正は撤去済み（PR #304）。ADR-191 決定4（「較正」）はそのまま ADR-195 の学習に読み替える。
- review-2026-09-24-06: 学習表の指紋書き込みと `staleness::check` の実行時配線は実装済み。
- review-2026-09-24-03: 学習プロセス（`awase-keymap-learn-win`）の同梱は PR #306 でマージ済み。

## 決定

### 決定1（所有者決定・原則）: 役割はユーザーの IME キー設定から逆算し、awase は受動的に対応する

awase は、ユーザーが IME のキー設定（GJI ならキー設定エディタ、MS-IME なら「キーとタッチのカスタマイズ」）で決めた内容から、
各キーの**役割**を逆算する。能動制御の例外は次の2つだけで、どちらにも当たらないキーでは、awase は**受動的**に振る舞う:

1. **役割トグル**（IME の設定から逆算した「IME ON/OFF トグル」の役割を持つキー、決定2・決定4）。
2. **awase 自身の `keys.ime_on`/`ime_off` 設定**（既定 Ctrl+変換/Ctrl+無変換）。IME の設定から逆算する役割ではなく、awase が自分で actuate する設定として扱う
   （所有者の当初の例外定義「ime_on/off キー（CTRL+無変換・変換）」どおり。所有者回答 2026-09-25、U5 修正、決定15）。修飾付きなので決定4 の候補キー（無修飾）とは重ならない。

受動の振る舞い:

- 物理キーを Suppress しない（生キーを IME へ通す）。
- IME の設定を書き換えない（`config1.db`・レジストリは読み取り専用。現行の方針どおり）。
- キーの効果を awase の書き込みで上書きしない（`shadow_action` を付けない）。
- belief は ADR-191 の予測（`KeyEffectPredicted`）と観測に追随する（所有者決定 U2、決定12）。

**受動と能動の境目（所有者発言 2026-09-25、U1 関連）**: 能動の対象は、状態に依存せず「トグル」と明確に判断できるキーだけである。
ATOK プリセットの変換/無変換は、入力していない開状態では閉じるが、入力中・変換中は別の作用（`Convert`・`ToggleAlphanumericMode`）をするので**受動**。
一方、新しい MS-IME で変換/無変換をトグルに割り当てた場合や、GJI の CUSTOM で全ての開状態を `IMEOff` にした場合のように、状態に依らず開閉が反転するキーは能動の対象になる
（判定式は決定4・決定11、変換/無変換の配線は決定16）。

### 決定2（所有者決定・例外）: 能動制御するのは「IME ON/OFF トグル」の役割を持つキーだけ。判定は役割で行い、キー名や VK では固定しない

**IME ON/OFF トグル**: ユーザーの設定で、直接入力（IME OFF）の状態から押すと IME ON（ひらがな・カタカナ等のモードを指定して開くものを含む）になり、
IME ON の状態から押すと IME OFF に遷移する振る舞いになっているキー（所有者発言 2026-09-24:「ime_on/off は直接入力の状態から IME ON/ひらがな/カタカナ に設定して、
IME ON の状態なら IME OFF に遷移するようなキーです。ユーザーの設定てそうなっているキーをトグルキーとみなします」）。したがって:

- ユーザーが半角/全角を別の機能（例: ひらがなモード）にしたら、半角/全角は例外に当たらず、awase は触らない。
- ユーザーが別のキー（例: F13）をトグルにしたら、そのキーを awase が能動制御する（候補キーの範囲は決定4・決定18）。

「IME ON の状態」の範囲は決定11（全ての開状態）、判定式は決定4。
「だけ」は IME 設定から逆算する役割についての限定で、これとは別に awase 自身の `keys.ime_on`/`ime_off` 設定が例外(2)として能動制御する（決定1・決定15）。

### 決定3（所有者決定・取得手段）: ユーザー設定は `config1.db`・MS-IME 本体のレジストリ・学習表だけから知る。実行時の受動的観測は使わない

- GJI: `config1.db`（`awase-gji-config` が読む）。
- MS-IME 本体: レジストリ（`HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME` の `KeyAssignment*`、`msime_key_assignment.rs`、互換モードフラグ `NoTsf3Override2`）。
  `config1.db` と同じく、ユーザーが設定画面で決めた内容そのもの（一次情報源）であり、実行時の観測ではない（所有者決定 U7、2026-09-25）。
  使うのは MS-IME 本体と同定できたときだけ（決定10）。
- 学習表: ADR-195/196 の `awase-keymap-learn` が awase をバイパスして注入学習し、自己検証・採用判定を通した表。
- 実行時に「キーを押したら IME がどうなったか」を観測して役割を推定することはしない。awase 自身の注入・Suppress・belief 更新が観測を汚染し、
  正しい役割を学べないため（ADR-191 の実測: awase を通すと仕様からの一致率が 98.5%→84.5% に落ちた）。

**範囲（所有者決定 U2、決定12）**: 「実行時の受動的観測は使わない」は**役割を知る手段**についての決定である。
belief を観測に追随させる ADR-187/191 の仕組み（生キーを通した後の再読取り、`kp_stage_mode_key_follow`、drift 補正）は状態の追随なので存続させる。
観測を役割判定に**フィードバックする経路は作らない**（例: 「観測では半角/全角で開閉が反転しなかったので役割をトグルから外す」は禁止）。

### 決定4（提案）: 役割の判定式と候補キー

**状態表**: GJI の `(Mozc status, キー名) → コマンド`。判定に使う状態は DirectInput（閉）と、開状態 Precomposition・Composition・Conversion。
Suggestion/Prediction/ZeroQuerySuggestion は継承規則（背景1）で**実効コマンド**を求め、判定は実効コマンドで行う
（例: プリセットの半角/全角は Suggestion 行が無いが、Composition の `IMEOff` を継承するので Close）。

コマンドは3類に分ける（`awase-gji-config/src/command.rs:65-86` `classify_command` を拡張）:

- **Open**: 閉状態から開く。`IMEOn`、および DirectInput 行の `CompositionMode*`/旧名 `InputMode*`（Mozc `keymap.cc` L460-471 が DirectInput に登録。
  所有者定義の「ひらがな/カタカナに設定して」に当たる）。※ `kCompositionModeXCommandSupported` が偽のビルドでは DirectInput の `CompositionMode*` は
  `NONE` で登録される（同 L472-483）。Windows 版 GJI でどちらかは未確認（T1(c)）で、確認までは `CompositionMode*` を Open に数えない（受動側に倒す。決定13）。
- **Close**: 開状態から閉じる。`IMEOff`・`CancelAndIMEOff`。
- **その他**: 上記以外（`Convert`・`Reconvert`・`ToggleAlphanumericMode` 等）、未知のコマンド、実効コマンドが無い（何もしない）。

**IME ON/OFF トグル**: DirectInput が Open、かつ**全ての開状態**が Close（所有者決定 U1、決定11。ADR-191 決定1-2 の「状態完備」と同じ）。
**CUSTOM ではさらに、awase の書き込み手段が効くことを要求する**: 実効の表に DirectInput の `ON`→Open と、判定対象の開状態すべての `OFF`→Close があること。
awase が送る `VK_IME_ON`/`VK_IME_OFF`（0x16/0x1A）も Mozc では `KeyEvent::ON`/`OFF` に写され（`win32_base_keyevent_handler.cc` L84/L94）、効果はキーマップの
`ON`/`OFF` 行で決まる。キー設定エディタは `ON`/`OFF` 行を非表示のまま保存時に書き戻す（`keymap_editor.cc` L125-127・L369-372・L447）が、インポートや手作りの表では
欠けうる。欠けた表で半角/全角をトグルと判定すると、Suppress したうえで送る `VK_IME_ON/OFF` を GJI が無視し、誰も開閉しない「二重の空振り」（ADR-119 型）になる。
書き込み手段もユーザー設定なので、条件に含めるのが「逆算」と一貫する。プリセットは4種とも `ON`/`OFF` 行を持つ（Mozc TSV で確認済み）ので定数表側では不要。
当たらないキーは**受動**（例: ATOK プリセットの変換/無変換、DirectInput でだけ `IMEOn` の MS-IME プリセットの F13〈`ms-ime.tsv` L134〉・Hiragana/Katakana、Mozc の `ON`/`OFF`、
BUG-64 の残骸バインド〈DirectInput の F21=`IMEOn`・開状態の F22=`IMEOff`、方向固定の別キー〉）。

**候補キー（初期範囲）は無修飾の半角/全角（0xF3/0xF4）・F13〜F24（0x7C〜0x87）・親指キーの無変換/変換（0x1D/0x1C）**（所有者決定 U9、決定18）。
候補キー集合はここ1箇所（`vk.rs` の純関数1つ）で定義し、種類ごとに能動制御の入口が違う:
半角/全角は `enrich_ime_relevance` の `shadow_action`（決定8）、F13〜F24 は同じ入口にラッチ付きの配送規則を足したもの（決定18）、
無変換/変換は `shadow_action` を付けず ADR-192 決定3b の単独タップ確定点（決定16）。

- 0x19（`VK_KANJI`）は物理的に必ず Alt 付きで届く（背景1）ので無修飾ガードを通らない。候補集合には入れず、決定14 に従う。
- F13〜F24: Mozc は `VK_F13`〜`VK_F24` を `KeyEvent::F13`〜`F24` に写し（`keyevent_handler.cc` L192-203）、キー名 `F13`〜`F24` をキーマップで使える
  （`composer/key_parser.cc` L141 付近）。awase 側のキー名→VK 写像（`awase-gji-config/src/keymap.rs` の `F1`..`F24` 規則、`:34`・`:71`）も既にある。
  GJI だけが対象（MS-IME 本体の新しいバージョンは F キーに割り当てられず、互換モードの割り当ては読めない）。
- 文字キー・Space は Engine が先に消費するので対象外。修飾付きの行も対象外（決定5）。
- **カタカナ(0xF1)・ひらがな(0xF2)**: kbd106 では物理キー「カタカナ/ひらがな」は無修飾で 0xF2、**Shift 付きのときだけ** 0xF1 を出す（`kbd106.c` L523-532）ので、
  0xF1 は無修飾ガードで常に受動になる。0xF2 は `transport.rs:211-217` の専用分岐（ADR-190）で `shadow_action` の判定より**前に** Allow/Suppress が決まるので、
  `Toggle` を付けると awase の `VK_IME_ON/OFF` と生の 0xF2 の両方が IME に届き開閉が2回反転する（BUG-46 型）。また Mozc は Eisu・Hankaku・Kana・Katakana の
  4キー名に限り修飾を消してからキーマップを引く（`keyevent_handler.cc` L330-347 `ClearModifyerKeyIfNeeded`）ので、「修飾付きは対象外」の規則とも食い違う。
- **英数(0xF0)**: kbd106 では `KBDNLS_TYPE_TOGGLE` で、別インデックスでは `VK_CAPITAL` を出す（`kbd106.c` L497-518）。物理キーの VK が状態で変わりうるので外す。
- **F1〜F12**: Mozc プリセットでは F6〜F10 等が入力中・変換中の変換に使われ、トグルにする構成はまず無い。
- 以上の候補外キーは受動のまま。対象にするなら別 ADR。

**プリセットは定数表、動的に逆算するのは CUSTOM だけ**（提案）: プリセット（ms-ime/atok/kotoeri/mobile）は有限で GUI から中身を変えられないので、
「プリセット → (VK, 役割)」の小さな定数表を持つ（テストで Mozc TSV と突き合わせて固定。TSV は同梱しない）。4プリセットとも `Hankaku/Zenkaku` はトグル形で
`ON`/`OFF` 行もある。判別は**生の `session_keymap` 値**で行う:

- `CUSTOM`(0) で `custom_keymap_table` が空でない → 上記の式で評価。
- ATOK/MSIME/KOTOERI/MOBILE → 定数表。
- フィールド無し・`NONE`(-1)・CUSTOM で表が空または無い → **MSIME の定数表**（Mozc が既定の TSV を読むケースに揃える。`keymap.cc` L167-176
  `ApplyPrimarySessionKeymap` の「fallback to default key map」、`GetKeyMapFileName` L213-243 の `NONE` → `default:`。`from_config` も `NONE` を MSIME とみなす、
  `key_effect_predictor.rs:541`）。
- その他（`OVERLAY_*`・`CHROMEOS`・未知の値）→ **受動**（パーサの誤りでありうるため。`OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF` なら Mozc は overlay の TSV だけを読み
  `Hankaku/Zenkaku` 行が無いので受動と一致する。Mozc は真に未知の値を既定に倒すが、ここでは決定6-3 の「不明なときに能動側へ倒さない」を優先する）。

`KeyEffectKeymap::from_config` の `preset` は流用しない（ATOK/MSIME 以外をすべて `KeymapPreset::Custom` にまとめる〈`key_effect_predictor.rs:539-542`〉ので、
kotoeri/mobile のユーザーで使われていない古い `custom_keymap_table` を評価してしまう）。`overlay_keymaps`（field 68）は Mozc が主キーマップの後に後勝ちで重ねる（`ApplyOverlaySessionKeymap`）。重ねた後の実効は評価せず受動に倒す（決定6-3）:
`OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`（100）があれば変換/無変換（決定16 で候補に入った）を受動に、それ以外の overlay 値があれば書き換える行が分からないので候補キーすべてを受動にする。
半角/全角・F13〜F24 は overlay 100 の対象外なので影響しない（PR #315 のコードレビュー M1。変換/無変換を候補に入れる前は「候補外なので使わない」としていた）。

### 決定5（提案）: 役割を持つキーで awase が「書いてよい」範囲

- **書いてよいのは開閉軸だけ**。手段は冪等な `VK_IME_ON`/`VK_IME_OFF`（ADR-189/191 決定1-1 と同じ）。変換モード軸（ひらがな/カタカナ）は書かない。
  トグルの役割を持つキーは、物理キーを Suppress し、`!belief` を目標に冪等キーを送る（現行 ADR-189 の仕組みそのもの、`ShadowImeAction::Toggle`）。
- モードを指定して開くキー（DirectInput→`CompositionModeFullKatakana` 等）も、トグルと判定したら開閉だけ書く（所有者決定 U3、決定13）。
  `VK_IME_ON` だけでは開いた後のモードが IME の保存値になり、ユーザー設定と食い違いうる（Mozc は conv を開閉にまたがって保存する、ADR-186）。
- **修飾付きは対象外**（初期範囲）。`enrich_ime_relevance` の無修飾ガード（`runtime/mod.rs:567-569`）と `extract_ime_keys` の修飾行除外に揃える。
  修飾付きのユーザー設定（例: MS-IME の Ctrl+Space トグル）は、既存の Engine 自動キー（`sync_ime_toggle_auto_detect`）が担う（適用条件は決定10で締める）。
- **awase 自身の設定（`keys.ime_on`/`ime_off`/`ime_toggle`）は、awase が自分で actuate する能動制御として別軸で存続する**
  （ADR-191 の「ユーザー設定はユーザーが何をしたいかの軸」、決定1 の例外(2)）。`ime_on`/`ime_off` は既定値（Ctrl+変換/Ctrl+無変換）も残す
  （所有者回答、U5 修正、決定15）。`ime_toggle` の既定（`VK_KANJI`）は空にする（決定14 の移行と同時、所有者回答 2026-09-25、決定15）。
  同じキーに役割由来のトグルと config.toml の値が重なったら config.toml が勝ち、役割は付けない（決定8・決定16。所有者回答 Q2 で確定）。
- IME の設定（`config1.db`・レジストリ）へは書かない（従来どおり）。

### 決定6（提案）: 情報源の優先順位と縮退動作

1. **役割を「付ける」根拠は `config1.db`（GJI）と MS-IME 本体のレジストリ（決定3、U7）だけ**。GJI は決定4の式で評価する。MS-IME 本体は
   仕様で固定のキー（決定6-4）とレジストリの割り当て（修飾付きトグルの自動キー〈決定10〉、変換/無変換〈決定16〉）で決める。
2. **学習表は「狭める」方向にだけ使う**。採用済み学習表（ADR-196、`RuntimeTableCache::is_active`、指紋一致・判定 Accepted・カバレッジ ≥ 0.80〈`key_effect_runtime.rs:50`〉）に、
   トグルとしたキーについて**トグルと矛盾するセル**が1つでもあれば、そのキーは受動にし、食い違いをログと不具合報告に記録する。
   根拠にするセルは開閉の反転そのものを測る2種だけ: 閉状態（DirectInput）で押して `after_open=false`、または開状態の未入力（`Stage::None`）で押して `after_open=true`。
   入力中・変換中のセルは記録だけにする（同梱表の MSIME_NATIVE は 206/227 セルが1試行のみ、MSIME は各2試行で非決定を検出しきれない〈`key_effect_table.rs` 冒頭〉。
   変換中セル1つのノイズで既定の半角/全角が受動に落ちると、ADR-189 導入前の退行〈TsfNative で8手順中4手順が反転しない〉に戻る）。セルが欠けているだけでは狭めない。
   学習表から能動側へ広げることはしない（学習の誤りが能動的な誤書き込みになる、ADR-195(A)。指紋一致で食い違うのはパーサ・GJI 版・学習のどれかの誤りで、
   どちらが正しいか分からない＝受動）。学習表で見られるのは `TableKey` の13種だけ。
   `use_learned_keymap_table = false`（opt-out）のときは狭めない（予測経路〈`key_pipeline.rs:1980`〉と同じ条件）。
3. **どちらからも決められないときは受動**（`shadow_action` を付けない＝生キーを通して予測・観測に追随）。
   対象: `config1.db` は**あるが**読めない/パースできない、パスが解決できない（`USERPROFILE` 未設定、`gji_charset_autodetect.rs:271-280`）、
   GJI 以外で設定の取得手段が無い IME（ATOK 本体・Japanist・未同定 TIP・IMM32 HKL のみ＝`table_ime_kind()` が `None`）。**不明なときに能動側へ倒さない**
   （Mozc は壊れたファイルでも既定で動くが、awase のパーサは非公開フォーマットの非公式実装で、失敗は awase 側の誤りでありうる）。
   **`config1.db` が存在しないときは不明ではない**: Mozc はファイル不在を既定設定（Windows では `session_keymap = MSIME`）として扱い、読み込み側では書き出さない
   （`src/config/config_handler.cc` `ConfigHandlerImpl::Reload` L258-276、`GetDefaultKeyMap` L330-338）。よって不在（パスが解決でき、`io::ErrorKind::NotFound`）は
   MS-IME プリセットとみなしてトグルにする（しないと、GJI の設定画面を一度も開いていないユーザーで半角/全角が受動に落ち、ADR-189 導入前の退行になる）。
   区別の実装は決定8（`read_key_effect_keymap` が不在のとき既定の keymap を返す）。
4. **IME の仕様でユーザーが変えられないキーは、既知の役割を持つ**: MS-IME 本体（新しいバージョン）の半角/全角はトグル（背景1）。
   ユーザーが変えられない以上「逆算した役割」と矛盾しないが、固定セットの残存であることを明記する（学習表による狭めは同様に適用する）。
   **互換モード（以前のバージョン）はこの前提が成り立たない**ので含めない。互換モードの半角/全角は受動（所有者決定 U8、決定17）。

### 決定7（提案）: IME ごとの扱い

| IME | 設定の取得手段 | 役割の決め方 | 現行からの変化 |
| --- | --- | --- | --- |
| GJI（プリセット ATOK/MS-IME 等） | `config1.db`（`session_keymap`=プリセット）＋プリセット定数表＋学習表（狭めのみ） | 決定4・6。半角/全角はトグル。変換/無変換はプリセットではトグルでないので受動（決定16） | 既定構成では不変（`config1.db` 不在も既定プリセット扱い〈決定6-3〉。ホストテストで固定） |
| GJI（CUSTOM） | `config1.db` の `custom_keymap_table`＋学習表（狭めのみ） | 決定4・6。半角/全角を別機能にしていれば受動 | 0xF3/0xF4 固定が外れうる。F13〜F24（決定18）・無変換/変換（決定16）に役割が付きうる |
| GJI（ATOK プリセット＋古い `custom_keymap_table` が残る構成） | `config1.db` | GJI は ATOK 選択時に custom 表を無視する（ADR-186(c)、`gji_charset_autodetect.rs:207-214`）ので、custom 表を読まない | 不変 |
| GJI（`session_keymap` が未知の値） | `config1.db` | 受動（決定4） | 0xF3/0xF4 固定が外れる |
| MS-IME 本体（新しいバージョン） | 仕様（半角/全角は変更不可）＋レジストリ（U7 で一次情報源と確定）＋学習表（狭めのみ） | 半角/全角はトグル（決定6-4）。修飾付きトグルは既存の自動キー（決定10）。変換/無変換はレジストリでトグルと判断できる割り当てのときだけ単独タップで能動（決定16） | 変換/無変換のトグル割り当てが能動になりうる（値の実機確認後、T12） |
| MS-IME 本体（互換モード） | 互換モードフラグ（ADR-197 決定4）のみ | 半角/全角は受動（決定17） | 半角/全角の固定が外れる（能動→受動） |
| ATOK 本体・Japanist・その他 | 無し | 受動。0x19 は決定14（IME 種別に依らず現行どおり） | MS-IME レジストリ由来の自動トグルキーが効かなくなる（決定10）。それ以外は不変 |

### 決定8（提案）: 役割は保持せず、候補キーの打鍵のときだけ求める

- **役割表は持たない**（round5 S3）。`kp_run_inner` の冒頭（`enrich_ime_relevance` の直前、`key_pipeline.rs:265`、`&mut self`）で、
  イベントが候補キー（決定4。無修飾の 0xF3/0xF4・F13〜F24。無変換/変換は決定16）のときだけ役割を求め、enrich に渡して `shadow_action = role.map(..)` を**代入**する
  （付け外しを1回で決める。代入は1箇所のままで `ime_relevance_shadow_action_writes_are_accounted_for` の件数は不変。
  ただし `is_open_toggle_for` の文字列を前提にした別のガード〈`bug116_...`〉の差し替えが要る、T4）。
- **明示 config と重なったら config が優先（役割を付けない）**（round7 M1）: 無修飾のそのキーが config.toml の `keys.ime_on`/`ime_off`/`ime_toggle`
  （読み込んだ実効値、`SpecialKeyCombos`）に含まれるなら役割を求めない（`shadow_action` なし）。読み込み後の `KeysConfig` では、ユーザーが書いた値と
  既定値（`KeysConfig::default()`・`AppConfig::save` が書き出した既定）を実行時に区別できないので、比較対象は既定値を含む実効値になる。
  既定値のうち無修飾は `ime_toggle` の `VK_KANJI` だけで、0x19 は候補キーでない（決定4・決定14）ので既定値との重なりは起きない（決定15 で空にした後も同じ）。重ねると1回の押下で開閉が2回書かれ打ち消し合う:
  `kp_run_inner` は `kp_stage_shadow_ime_toggle`（`key_pipeline.rs:328`）が belief を反転した**後**で ctx を作り（`:338`）`engine.on_input`（`:426`）を呼ぶ。
  Engine の `ImeToggle` は反転後の `!ctx.ime_on` を読む（`engine.rs:994-1001`）ので元に戻し、`VK_IME_OFF`→`VK_IME_ON` が続けて送られる。
  `match_event` が ime 系コンボの照合を止めるのは `sync_direction.is_some()` のときだけ（`engine.rs:1070-1083`、ADR-092 の `ime_detect` との二重処理と同じ形）で、
  役割由来の `shadow_action` では止まらない。例: 以前から GJI の CUSTOM で F13 をトグルにし、awase にも `ime_toggle = ["F13"]` と書いているユーザー。
  向きは決定16（config 由来が優先）と揃える。逆向き（`match_event` のガードに `shadow_action.is_some()` を足す）は役割が config に勝つので採らない。
  config の実効値は Runtime が保持する（決定16 の `thumb_forced_open_actions` の保持と同じく Runtime のフィールド、ADR-164 に従いグローバル static にしない）。
  所有者回答 Q2（2026-09-25）で確定: 同じキーが GJI のトグルで config.toml にも書かれていれば config.toml を優先し、役割は付けない。
  既定値のまま残る `ime_on`/`ime_off`（Ctrl 付き）は無修飾の候補キーと重ならないので、この規則が効くのは config.toml に無修飾のキーを書いた場合だけ。
- 求め方は `tsf_obs().table_ime_kind()` で分岐する: `None`（ATOK・未同定等）→ 役割なし（受動。現行 enrich のゲートと同じ）、`Gji` → `config1.db` の
  keymap で決定4・決定6-2、`MsIme` → 決定6-4・決定6-2。**その打鍵の時点の同定で分岐するので、IME を切り替えたときに古い役割が残ることは無い**
  （round5 S1: 保持した表を使う設計では、GJI → ATOK の切り替え後も 0xF3/0xF4=Toggle が残り、ATOK の半角/全角を Suppress していた）。
- keymap と学習表の取得は予測経路（`key_pipeline.rs:1954-1987`）と**同じインスタンス・同じ引数**の `KeymapCache::get`/`RuntimeTableCache::get` を使う。
  取得部分を1つのヘルパーに切り出し、予測と役割判定の両方から呼ぶ。間引き（2000ms ごとの stat）も同じなので I/O は増えない。
  評価コストは候補キーの打鍵ごとに custom 表の1キー名ぶんの走査だけ。世代管理・作り直し判定・`peek()`・温める仕組みは作らない。
- **共有キャッシュの型の変更**（round5 S2）: (i) `KeyEffectKeymap` に生の `session_keymap`（`Option<i64>`）を1フィールド足す（決定4の判別に使う。
  指紋はハッシュで復元できない）。(ii) `read_key_effect_keymap`（`gji_charset_autodetect.rs:307-315`）は、ファイル不在（パス解決済みかつ `NotFound`）のとき
  `from_config(None, None, &[])` を返す。読めない・パース失敗・パス未解決は従来どおり `None`。これで「不在」と「読めない」の区別に3値の型は要らない。
  副作用として、不在のとき**予測も MS-IME プリセットで動き**、指紋 `gji_keymap_fingerprint(None, None, &[])` で学習表も引ける（`from_config` が既にフィールド不在・`NONE` を
  MSIME とみなしているのと同じ扱いで、Mozc の挙動とも一致する）。予測の変化なのでホストテストを付ける。
  `read_key_effect_keymap` の呼び出し元は予測（`key_pipeline.rs:1958`）のほかに3つあり、不在時の挙動がそれぞれ変わる（いずれも改善の方向）:
  PR #308 の半角/全角の `Toggle` 除外（`runtime/mod.rs:620` `learned_table_omits_hz_toggle`、`:646` で読む）は、不在のとき keymap が `None` で `false`（除外しない）だったのが、
  `get_for_keymap`→`hz_omit_verdict` まで届き、採用中の学習表が半角/全角をトグルでないと示せば `Toggle` を外すようになる（決定6-2 の向き。能動→受動の方向だけ）。
  ADR-192 の警告（`runtime/mod.rs:1252` `check_state_dependent_mode_keys`）は、全対象 VK が `CannotPredict(AmbiguousKeymap)`（`key_effect_table.rs:527-531`）
  から MSIME 表での分類になる。学習プロセスの指紋（`key_effect_runtime.rs:442` `current_fingerprint_probe`、`awase-keymap-learn-win` main.rs:470・942）は
  `Unavailable` から計算可能になり、`Rejected(FingerprintUnavailable)` で棄却されず採用されうる（ADR-196 の挙動の変化。書き手と読み手が同じ関数なので指紋は構造的に一致する）。
  学習プロセスの内蔵表との突き合わせ（`gji_charset_autodetect.rs` `bundled_preset_for_adjudication`、ADR196-T2 決定1c/1e）も不在を `ConfigUnreadable`（突き合わせを飛ばす）から
  既定の既知構成 `Known(MsIme)` に揃える（採用されうるようになった表が突き合わせを通らないのを防ぐ。PR #315 のコードレビュー M2）。
  付随して、ユーザーが GJI の設定画面で初めて保存すると指紋が `(None,None,[])` から `(Some(2),None,[])` に変わり、挙動は同じでも学習表は1回だけ要再検証になる（記録のみ）。
- バッチ前処理の enrich（`message_handlers.rs:1732`）は候補キーに触らない（`shadow_action` は `kp_run_inner` で付く。その間に `shadow_action` を読む経路が無いことは T4 で確認）。
- `config1.db` が変われば学習表の指紋も変わり、学習表は陳腐化として使われなくなる（06、`staleness::check`）。このとき役割は `config1.db` だけから決める
  （狭めが外れるだけ）。ADR-196 決定3 の「要再検証」は、再検証が通るまで学習表を使わない、という点で両立する。GJI 本体の更新で挙動が変わった場合は
  ADR-196 の再検証（`revalidation.rs`）に従う。

### 決定9（提案）: 初期範囲で現状維持にするもの

変えるときは別 ADR。

- `VK_IME_ON`/`VK_IME_OFF`（0x16/0x1A）の静的 `TurnOn`/`TurnOff`（`vk.rs:154-155`）: 方向固定の冪等キーで所有者定義のトグルには当たらず、**「唯一の例外」の外側に残る能動経路**。
  存置の理由は、Mozc の `ON`/`OFF` 行はキー設定エディタで編集できず、awase は物理キーと同じ VK を同じ方向に送り直すだけなので IME の設定を握りつぶさない
  （効果は生キーを通したのと同じで、belief の更新が付くだけ）こと。
- `keys.ime_detect`（`src/config.rs:495-510`、VK 固定で belief を動かす静的規則）: 物理キーを消費しない belief の追随（決定12 で存続）なので対象外。
- 0x19（`VK_KANJI`）の静的 `Toggle`: 決定14 の移行までそのまま。

### 決定10（提案）: 既存の MS-IME レジストリ由来の自動トグルキーを、MS-IME 本体と同定できたときだけに締める

役割判定とは独立した既存経路の適用範囲の不具合（背景2）なので、BUG を起票して小さな fix PR として先に切り出してよい（キー選択ファミリー）。

1. `sync_ime_toggle_auto_detect` と、同じ `if` の中の `msime_key_assignment::check_and_warn`（割り当ての解除案内）の呼び出し条件を
   `ms_ime_native_identified()` に絞る（予測経路 `key_pipeline.rs:1960-1965` と同じ条件）。`sync_ime_kind_from_observation`（`message_handlers.rs:939-942`）と
   `app/mod.rs::reload_config`（`:811-817`）の両方。
2. **検出済み**で本体と同定できないとき（GJI・ATOK 等）は `set_ime_toggle_auto_keys(vec![])` で自動キーを空にする。未検出（`detected=false`）のときは触らない
   （`reload_config` の経路と揃える）。
3. **再評価のきっかけを足す**: `gji_monitor.rs` のポーリング（`:388-414`）と GJI アタッチ時（`:432-447`）で、tick 内の `kind_changed || identity_changed` を集計し、
   最後に1回だけ `WM_IME_KIND_CHANGED` を post する（round5 S5）。これで MS-IME 本体 ⇔ ATOK の切り替えでも再評価され、kind と同定が同時に変わる tick
   （MS-IME 本体 ⇔ GJI）でも post は1回なので `GjiFsm::new()` が二重にならない（`output/tsf_warmup_coord.rs:110-113`）。kind が `MicrosoftIme` のまま同定だけ変わる場合に
   `set_active_ime_kind` が作り直すのは状態を持たない unit struct の `MsImeStrategy`（`tsf/warmup/warmup_strategy.rs:122`）なので実害は無い。
4. テスト: `sync_ime_kind_from_observation` は `#[cfg(windows)]` の中なので、「(kind, detected, identified) → 付ける／空にする／触らない」を返す純粋関数に切り出してホストでテストする。

### 決定11（所有者決定 U1）: 「IME ON の状態」は全ての開状態。全ての開状態で閉じるキーだけをトグルとする

判定対象の開状態は Precomposition・Composition・Conversion と、継承で実効コマンドを求める Suggestion・Prediction・ZeroQuerySuggestion のすべて（決定4）。
1つでも Close 以外（`Convert`・`ToggleAlphanumericMode`・何もしない等）があればトグルではなく受動。「未入力のときだけ能動」の切り替えは作らないので、
awase の「未確定文字があるか」の推定（予測器の Stage）は役割判定に使わない。ATOK プリセットの変換/無変換（Composition が `Convert`/`ToggleAlphanumericMode`）はトグルではない。

### 決定12（所有者決定 U2）: 実行時の受動的観測を禁止するのは役割の推定だけ。belief の観測追随は存続

決定3 の範囲の確定。ADR-187/191 の観測追随（生キーを通した後の再読取り、`kp_stage_mode_key_follow`、drift 補正）はそのまま残す。
観測の結果を役割に戻す経路は作らない（決定3 の注記）。

### 決定13（所有者決定 U3）: モードを指定して開くトグルも開閉だけ書く。実機で確認できるまでは受動

- DirectInput の `CompositionMode*`/`InputMode*` で本当に開くか（T1(c)）の実機確認が済むまでは、これらを Open に数えない。所有者の例
  （ひらがな/カタカナのモードを指定して開くキー）はそれまで受動。
- 開くと確認できたら Open に数える。awase が書くのは開閉だけ（`VK_IME_ON`/`VK_IME_OFF`）で、変換モード軸は書かない（決定5、ADR-191）。
  開いた後のモードは IME の保存値になり、ユーザー設定のモードと食い違いうる（リスク）。
- 開かないと分かったら、そのキーはトグルではない（受動のまま）。

### 決定14（所有者決定 U4）: 0x19 は当面現状維持。T1(b) の後に「ユーザー設定で変えられない既知のトグル」として扱う

- 当面: `hook.rs:272-299` の静的 `Toggle`（`vk.rs:156`、IME 種別に依らない）をそのまま使う。
- T1(b)（TSF 経路で 0x19 が `Hankaku/Zenkaku` 行に従わないことの実機確認）の後: 決定6-4 と同じ「既知のトグル」とし、採用中の学習表に
  矛盾セル（決定6-2 の2種、`TableKey::Kanji`）があるときだけ受動に狭める。IME 種別に依らない点は変えない（学習表があるのは GJI・MS-IME 本体だけなので、
  狭めが効くのもその2つだけ）。T1(b) で行に従うと分かった場合は、0x19 を役割判定に入れるかを改めて決める（別 round）。
- どの段階でも 0x19 は決定4 の候補集合・無修飾ガードに通さない（Alt 付きで届くため）。`keys.ime_toggle` の既定を空にする変更（決定15、所有者回答 2026-09-25 で確定）はこの移行と同じ変更で行う。

### 決定15（所有者決定 U5・2026-09-25 修正）: awase 既定の `keys.ime_on`/`ime_off` は残す（awase 自身が actuate する設定）。`keys.ime_toggle` の既定は決定14 の移行と同時に空にする

- **`ime_on`（`Ctrl+変換`）・`ime_off`（`Ctrl+無変換`）の既定は変えない**（`KeysConfig::default()`、`src/config.rs:576-590`）。所有者回答（2026-09-25）で、
  これは「IME の設定から逆算する役割」ではなく「awase 自身が actuate する設定」として扱う例外（決定1 の例外(2)、所有者の当初の例外定義
  「ime_on/off キー（CTRL+無変換・変換）」どおり）と確定した。当初の U5 回答「既定を空にする」は `ime_on`/`ime_off` について撤回。
  既定が変わらないので、既存 config.toml に書き出された旧既定値の移行問題（旧 U10）は生じない。設定画面の JIS 配列切替の書き込み（`crates/awase-settings/src/main.rs:2587-2591`）も
  `ime_on`/`ime_off` については現状維持。
- **既知の衝突（所有者が awase 側の設定として受け入れた）**: ユーザーが IME 側（例: GJI のキー設定）で Ctrl+変換/Ctrl+無変換 を別のコマンドに割り当てていても、
  awase の既定の `ime_on`/`ime_off` が先に消費して開閉を書く。決定1 の「ユーザーの IME 設定を尊重する」の例外として所有者が受け入れた。変えたいユーザーは config.toml で上書きする（リスク節）。
- **`ime_toggle`（`VK_KANJI`）の既定は空にする**（所有者回答 2026-09-25 で確定）。時期は決定14 の移行（U4、0x19 を既知のトグルとして扱う変更）と同じ変更（T14）で、
  それまで 0x19 は現状維持。`ime_on`/`ime_off`（修飾付き、既定を残す）とは扱いが分かれる。影響:
  物理の 0x19 は Alt 付きで届き、Engine のコンボ照合は修飾の完全一致（`engine.rs:1013-1018` `matches_key_combo`）なので、無修飾の既定 `VK_KANJI` に
  一致するのは無修飾の 0x19 を出す構成（リマッパー等）だけと推定する（T14 で確認）。0x19 の開閉は hook 経路の静的 `Toggle` が担い続ける。
  このとき JIS 配列切替の書き込み（`main.rs:2591`）の `ime_toggle` も空に揃え、`AppConfig::save`（`src/config.rs:830-833`）が構造体全体を書くことで
  既存 config.toml に書き出された旧既定値 `ime_toggle = ["VK_KANJI"]`（旧 U10 と同型）が残る。実行時はユーザーが書いた値と区別できない（決定8）ので、
  移行処理の要否は T14 の実装時に確認する（所有者判断を要する未決ではない。上記のとおり無修飾 `VK_KANJI` が一致する構成は限られる）。
- 文書の追随（T11、T14 と同時）: 同梱の `config.toml:27-29`（`ime_detect.toggle` の注意書き）、`docs/usage.html:674-675`・`:779`、
  `docs/usage.en.html` の対応箇所、`crates/awase-settings/src/main.rs:4856-4883`（漢字キーを既定とする説明）。`ime_on`/`ime_off` の既定の記述（`README.md:82-83` 等）は変えない。

### 決定16（所有者決定 U6）: 無変換/変換がトグルの役割を持つ設定では、単独タップと解決したときだけ能動制御する（チョード優先）

**合流点は ADR-192 決定3b の既存の入力 `forced_open_action` 1つだけ**（新しい入力・新しい判定点は作らない）。

- 役割: GJI の `config1.db` から決定4 の判定式（決定11 の全開状態）で求める（キー名 `Muhenkan`/`Henkan`）。`table_ime_kind()` が `None`（ATOK 等）なら付けない。
  MS-IME 本体の変換/無変換は、レジストリ（U7 で一次情報源と確定、決定3）の割り当てが状態に依らずトグルと判断できるときだけ役割を付ける（決定1 の境目）。
  現行コードが知っている値は `KeyAssignmentMuhenkan`=1（IME-オフ）・`KeyAssignmentHenkan`=1（IME-オン）・0（かな切替/再変換）だけ（`msime_key_assignment.rs:22-26`）で、
  いずれも方向固定かトグル以外なので**受動**。トグルに当たる値は未確認なので、実機で値を確かめてから足す（T12。推測で値を決めない、同ファイル L33-37 の決定C R3 と同じ）。
  学習表は決定6-2 と同じく狭める方向だけ（`TableKey` に Henkan/Muhenkan がある）。
- 配線: `kp_run_inner`（`key_pipeline.rs:264`）で親指キーの非injected・非リピートの KeyDown のときだけ役割を求め、`thumb_forced_open_actions`
  （`runtime/mod.rs:39-66`、config.toml の bare `keys.ime_*` 由来）の結果を優先して `config由来.or(役割由来)` を
  `engine.set_thumb_forced_open_actions`（`nicola_fsm.rs:842-849`）に設定し直す。打鍵ごとに求め直すので、IME を切り替えたときに古い役割が残らない（決定8 と同じ考え方）。
  Engine（OS 非依存）には VK と役割の組ではなく `ShadowImeAction` だけを渡す（ADR-192 決定3b と同じ境界）。
- 発火: `defers_solo_until_release`（`nicola_fsm.rs:893-911`）が `forced_open_action` を見て KeyUp で解決し、`resolve_pending_thumb_as_single`
  （`nicola_fsm.rs:2135`、優先順位1.5〈`:2178`〉）が単独タップと確定したときだけ開閉を要求する。同時打鍵と解決した打鍵では発火しない（チョード優先）。
  既存ガード（`*_solo_tap_ime_action` が同じ VK にあれば自己無効化・`explicit_action_consumed`・`suppress_solo_output`・押下後の Shift）はそのまま効く。
  優先は 専用Fnキー ＞ config.toml の bare `keys.ime_*` ＞ 役割由来 ＞ `ModeKeyConfig`。composing 中も発火する（ADR-192 決定3b と同じ。決定11 により IME 側もこのキーで閉じる設定なので食い違わない）。
- 物理配送は変えない: エンジン活性中の親指キーの KeyDown は `PendingThumb` として `Decision::Consume` される（ADR-192 決定3b round4 C-1）。
  `shadow_action` は付けない（付けると `transport.rs:283-291` の先行 `Allow` と awase の書き込みで二重 actuation になる、BUG-46 型）。
- エンジン非活性（IME OFF 等で FSM が動かない）のときは能動にしない。生キーを IME へ通し、IME 自身がユーザー設定どおり開く（受動）。
- ADR-192 決定3b の「既定では発火しない（完全なオプトイン）」の前提は変わる: config.toml に何も書かなくても、GJI の CUSTOM で無変換/変換をトグルにしたユーザーでは発火する。
  ADR-192 の status に追記する（T7）。

### 決定17（所有者決定 U8）: MS-IME 互換モード（以前のバージョン）の半角/全角は受動

- `msime_legacy_keymap.rs:452-456` `read_legacy_compat_mode_enabled()` が `Some(true)` のとき、MS-IME 本体の半角/全角に役割を付けない（`shadow_action` なし）。
- `Some(false)` は決定6-4（既知のトグル）。`None`（値が無い、または読めない）は**現状どおりトグル**（起草者判断）: チェックボックスを一度も触っていない既定の
  環境では値が無いと推定し（T1(d) で確認）、`None` を受動にすると MS-IME 本体の大半のユーザーで ADR-189 導入前の退行になるため。
  `None` を「既知構成ではない」とみなす ADR196-T2 の fail-safe とは向きが逆になるので、T1(d) の結果で見直す。
- レジストリの読み取りは打鍵ごとにせず、予測経路の MS-IME 本体のレジストリ読み取り（`key_pipeline.rs:1960-1965`）と同じ間引きに載せる（T13）。

### 決定18（所有者決定 U9）: 初期範囲の候補に F13〜F24 を加える。二重の空振りは「最初の Down で実際に書いた打鍵だけ Suppress」で防ぐ

**F13〜F24 だけに要る理由**（0xF3/0xF4 との違い、いずれもコードで確認）:

1. **`is_japanese_ime()` が真に上がらない**: 0xF3/0xF4 は受信そのものを IME の証拠として `is_japanese_ime()` を真に上げる（`vk.rs:336`
   `should_upgrade_is_japanese_ime`、`key_pipeline.rs:1264`）。F13〜F24 は物理キーが実在しうる（プログラマブルキーボード等）ので IME の証拠にならず、上げてはならない
   （ADR-093 の基準、BUG-14 と同じ理由）。`shadow_toggled` は `is_japanese_ime()` が偽だと立たない（`key_pipeline.rs:1356-1364`）。さらに enrich（`:265`）と
   `kp_stage_shadow_ime_toggle`（`:328`）の間の `kp_stage_focus_probe`（`:318`）が `is_japanese_ime` を偽に下げうる（`:2919-2922`）。
2. **現行の配送規則では書かないのに Suppress しうる**: `transport.rs` は ImmCross なら `shadow_action` があるだけで Down/Up とも Suppress（`:298-302`）、
   それ以外でも KeyUp は常に Suppress（`:337-340`）。`Toggle` を付けたが書かなかった打鍵は、ImmCross では二重の空振り（ADR-119 型）、
   それ以外では Down だけが IME に届き Up が消える。
3. **自動リピートと通常の Down/Up の組がある**: 0xF3/0xF4 は押すたびに VK が交互に変わり KeyUp が来ないことがある（`transport.rs:55-58` のコメント）が、
   物理の F13 はリピートする。`kp_stage_shadow_ime_toggle` はリピートを区別しないので、そのままではリピートのたびに開閉が反転する。

**規則**: F13〜F24 は、**その打鍵の最初の Down で awase が実際に開閉を書いた（`shadow_toggled`）ときだけ**、その Down・自動リピートの Down・Up を Suppress する。
書かなかった打鍵は Down/Up とも Allow（受動、IME がユーザー設定どおり処理する）。ImmCross でも同じ。

**実装（新しい仕組みは足さず、3箇所の小さな変更）**:

- (i) **ラッチ**: PR #308 の `hz_toggle_omit_latch`＋`omit_latch_step`（背景4）を、候補キー全体で1本の「打鍵ごとのラッチ」に一般化する。
  **値の意味はキーの種類で変えない**（round7 M3）: #308 のラッチの値は「Toggle を外すか」（true＝外す、`runtime/mod.rs:335-340`）で、F キーに「書いたか」（true＝付ける）を
  そのまま同じスロットに入れると意味が逆になる。そこで値を**「この打鍵の最終的な `shadow_action`」**（`(scan_code, Option<ShadowImeAction>)`）に統一する。
  半角/全角では enrich の判定結果（学習表で外せば `None`）をそのまま入れ、F13〜F24 では `kp_stage_shadow_ime_toggle` の直後（`key_pipeline.rs:328` の次）で
  最初の Down（非injected・`!was_down`）のときだけ `shadow_toggled` から `Some(Toggle)`/`None` に上書きする
  （ラッチの書き込み箇所が1つ増えるが、`shadow_action` の代入は enrich の1箇所のまま）。自動リピートの Down と Up では enrich がラッチの値をそのまま代入する。
  識別は #308 と同じ scan_code（F13〜F23 は 0x64〜0x6E、F24 は 0x76、いずれも拡張ビットだけが違う双子キーが無い）。
  **scan が一致しないとき**: 現行の `omit_latch_step` は scan が一致しなければ `fresh()` で判定し直す（`state/key_effect_runtime.rs:583-598`）。
  F13 を押したまま半角/全角を押すと、半角/全角の最初の Down がラッチを上書きし、後から来た F13 の Up は一致しない。F キーでここを `fresh()`（＝役割）にすると、
  書かなかった打鍵の Up だけが Suppress され Down だけがアプリに残る（逆向きでは Up だけが IME に届く）。よって **F13〜F24 の Up・リピートで scan が一致しないときは `None`
  （Allow）** とする（孤立した Up が IME に届く向きのほうが害が小さい）。半角/全角の不一致時は #308 のまま（`fresh()`）。
  **`hook.rs` の `HookState` の親指ラッチ（`left/right_thumb_down_scan`、BUG-131/132）は再利用しない**: フックのスレッドで親指の押下状態を捕捉時スナップショットに
  埋めるためのもので、配送判定はメインスレッドのパイプラインにあり、#308 のラッチが既に同じ場所にある。再利用するのは「vk でなく scan で Down と Up を対応させる」規律
  （`vk::should_release_thumb_latch` と同じ考え方）だけ。
- (ii) **リピートでは書かない**: `kp_stage_shadow_ime_toggle` で、F13〜F24 の役割由来の `Toggle` は `event.was_down` の Down では intent に昇格させない（0xF3/0xF4・0x19 の挙動は変えない）。
- (iii) **配送規則**: `transport.rs::plan` の injected の分岐（`:238`）・無変換/変換の分岐（`:283-291`）の後、ImmCross の分岐（`:298`）の前に F13〜F24 の分岐を1つ置く:
  最初の Down は `shadow_toggled`、リピートの Down と Up は `shadow_action.is_some()`（＝ラッチ）で Suppress。`suppress_reason` のラベルも分ける。

**物理 IME キー配送（0xF0〜0xF6）への影響は無い**: F13〜F24 は 0xF0〜0xF6 の外なので、0xF2 の専用分岐（`:212`）・0xF3/0xF4 の `is_dbe_mode_key_down`（`:333`）・
`may_change_ime`（`vk.rs:201-206`）は変わらない。0xF3/0xF4 の規則（Down は常に Suppress）もそのまま。無変換/変換の先行 `Allow`（`:283-291`）は F キーに当たらない。
`PassthroughQueue`（`:47-60`）から見ると、書いた打鍵は Down/Up とも Suppress、書かなかった打鍵は Down/Up とも Allow で、どちらも対称。

**効かない・受動のままのもの**:

- ソフトウェアのリマッパーが出す F キーは `LLKHF_INJECTED` 付きで `:238` で Allow、`kp_stage_shadow_ime_toggle` でも昇格しない（BUG-14）。能動になるのは
  `Scancode Map`・ファームウェアのリマップ・実在の F13〜F24 だけ。injected の F キーを IME が開閉しても、F キーは `may_change_ime` の対象外で学習表のセルも無い（`TableKey` の13種に無い）ので、
  TsfNative では belief がずれうる（リスク）。
- awase 自身が送る F キー（ADR-091 の専用 Fn キー F15〜F24）は awase のマーカー付きの injected なので候補にならない。
- 学習表で狭められない（F キーのセルが無い）。役割は `config1.db` だけから決まる。

**複雑にしてでも入れる根拠**: 所有者が F13〜F24 を初期範囲に含めると決めた（決定2 の後半「他のキーをトグルに設定したら awase が能動的に制御する」の初期範囲での実体）。
上の3つの性質は 0xF3/0xF4 に無いので、半角/全角の規則のままでは二重の空振り・Up の欠落・リピートでの反転が起きる。それぞれ最小の手当てが (i)〜(iii) で、
(i) は既存ラッチの一般化、(ii) は1条件、(iii) は1分岐。

## 既存 ADR・実装への影響

| 対象 | 現状 | 本 ADR 後 |
| --- | --- | --- |
| ADR-189（固定セット 0x19/0xF3/0xF4） | VK 基準で常にトグル（GJI・MS-IME 本体） | 「既定プリセットの半角/全角がトグルの役割を持つ」場合の一例に格下げ。ステータスに「ADR-199 で役割判定に一般化」と追記 |
| ADR-191 決定1-1（静的に残す唯一の例外）・RM3（固定が常に勝つ） | 固定セットは表・設定と矛盾しても勝つ | **置換**。ユーザー設定から逆算した役割が勝つ。決定1-2（表駆動の追加、状態完備条件）は決定4の判定式として採用 |
| ADR-192（状態依存キーの警告） | 0xF3/0xF4 のカスタム行で `UserOverride`（ログのみ） | 役割が付かないキーは受動なので「awase が握りつぶす」警告は不要。状態依存（受動キー）の警告は存続。対象 VK（`state_dependent_key_warning.rs:10`）の見直しは T6。`config1.db` 不在時は `AmbiguousKeymap` から MSIME 表での分類に変わる（決定8） |
| ADR-192 決定3b（bare `keys.ime_*` の親指キー単独タップ） | config.toml に bare で書いたときだけ発火（完全なオプトイン） | 同じ入力 `forced_open_action` に役割由来の値も入る（config 由来が優先、決定16）。「既定では発火しない」の前提が変わるので status に追記（T7） |
| ADR-195(A)（actuation 許可リストを学習で自動拡張しない） | 許可リスト＝ADR-189＋ユーザー明示 config | **軽微な追記**。「学習で拡張しない」は維持（決定6-2）。許可リストの出どころが「`config1.db` から逆算した役割」に変わる |
| ADR-195 追記（PR #308、学習表で半角/全角の `Toggle` を外す＋ラッチ） | 0xF3/0xF4 専用の分岐とラッチ | 分岐は決定6-2 に包含。ラッチ `hz_toggle_omit_latch`/`omit_latch_step` は候補キー全体の「打鍵ごとのラッチ」に一般化し、値を「この打鍵の最終的な `shadow_action`」に統一する（F13〜F24 は最初の Down で書いたときだけ `Some(Toggle)`、scan 不一致は `None`。決定18 (i)） |
| ADR-196（学習結果が真実） | 予測にのみ適用。`config1.db` 不在の GJI では指紋 `Unavailable` で学習結果を棄却 | 役割判定では狭める方向にだけ適用（決定6-2）。不在時も指紋が計算でき、学習結果が採用されうる（決定8） |
| ADR-186/187（ATOK の変換/無変換の追随） | 受動（follow） | ATOK プリセット・既定構成では不変（全開状態で閉じないのでトグルでない、決定11）。GJI の CUSTOM で無変換/変換をトグルにした構成だけ、単独タップで能動（決定16） |
| ADR-197（MS-IME 互換モード） | 検出ロジック撤回、フラグ読取のみ | 互換モードフラグを MS-IME 本体の半角/全角の役割判定に使う（`Some(true)` で受動、決定17） |
| ADR-176（較正） | 撤去済み（ADR-198 決定3） | 影響なし |
| review-08（(B) 案・0x19 現状維持） | (B) 案は PR #308 で実装済み（ADR-195 追記） | **置換**。(B) は決定6-2 の特殊ケースとして包含。0x19 は決定14 |
| `vk.rs:152-196`（`shadow_effect`・`is_open_toggle_for`） | VK で効果を決める | `is_open_toggle_for` を撤去し役割の参照に置換。候補キー集合（0xF3/0xF4・F13〜F24・無変換/変換）の純関数を1つ足す（決定4）。0x19 の静的 `Toggle` は決定14 の移行まで、0x16/0x1A の静的 `TurnOn`/`TurnOff` は存置（決定9） |
| `runtime/mod.rs:559-588`（`enrich_ime_relevance`、`bdd8f1ee`） | `table_ime_kind()` と VK で `Toggle` を付け、#308 のラッチで学習表による除外を Down→Up に持ち越す | `kp_run_inner` で求めた役割を受け取り、候補キーには `shadow_action = role.map(..)` を代入（決定8）。F13〜F24 は `ime_kind()` の早期 return（`:571-573`）を通らないので、候補集合の判定をその前に置く（決定18）。リピートの Down と Up は一般化したラッチを読む。バッチ前処理では候補キーに触らない |
| `runtime/key_pipeline.rs`（`kp_run_inner`・`kp_stage_shadow_ime_toggle`） | — | `:328` の直後で F13〜F24 の最初の Down のときだけラッチに `shadow_toggled` を書く。F13〜F24 の役割由来の `Toggle` は `was_down` の Down で昇格させない（決定18 (i)(ii)）。親指キーの KeyDown で役割由来の `forced_open_action` を設定し直す（決定16） |
| `runtime/transport.rs:293-340`（Suppress 判定） | `is_dbe_mode_key_down` が `is_open_toggle_for` で 0xF3/0xF4 の KeyDown を常に Suppress | 役割を引き直さない（`ImeRelevance` にフィールドは足さない）。`is_dbe_mode_key_down` を「KeyDown かつ `shadow_action == Some(Toggle)` かつ VK が 0xF3/0xF4」に置き換える（0xF3/0xF4 は `is_japanese_ime()` が真に上がるので二重の空振りにならない）。`:322-328` のコメント前提を更新。F13〜F24 の分岐を injected・無変換/変換の分岐の後、ImmCross の前に1つ足す（決定18 (iii)）。無変換/変換の先行 `Allow`（`:282-291`）は不変 |
| `gji_charset_autodetect.rs:287-317`（`read_config1_db`・`read_key_effect_keymap`） | ファイル不在も読取り/パース失敗も `None` | 不在（パス解決済みかつ `NotFound`）は `from_config(None, None, &[])`、それ以外の失敗は `None`。呼び出し元3つ（予測・ADR-192 警告・学習プロセスの指紋）すべてに効く（決定8） |
| `key_effect_predictor.rs`（`KeyEffectKeymap`） | `preset` だけ保持 | 生の `session_keymap` を保持（決定8）。`config1.db` 不在で予測が MS-IME プリセットで動く |
| `tests/architecture_guard.rs:4516-4541`（`bug116_shift_katakana_guards_are_present_in_production_code`）・`:816` の説明文 | transport.rs 本番コードに `is_open_toggle_for` があることを assert（Linux での Suppress 判定の唯一の防波堤） | 必須トークンを新しい判定（`Some(ShadowImeAction::Toggle)` と 0xF3/0xF4 の組）に差し替え、否定側メッセージと `:816` の説明を更新（T4。削るだけにしない） |
| `awase-gji-config`（`extract_ime_keys`・`mozc_key_to_vk_name`） | 状態完備を見ない。`Hankaku/Zenkaku`→`VK_KANJI` のみ（doc も不正確） | 継承規則つきの状態表と決定4の判定関数（純粋関数）を追加。キー名→VK 写像（`Hankaku/Zenkaku`→0xF3/0xF4）を予測側と一本化し、別名表の doc を直す |
| `state/key_effect_table.rs:511-`（ADR-192 分類） | 0x19 を `Kanji` として独立扱い | 決定14（移行後は `Kanji` セルで狭める） |
| `src/config.rs:581-583`（`keys.ime_on`/`ime_off`/`ime_toggle` 既定）・`engine.rs:975` | awase の既定値で Engine が消費 | `ime_on`/`ime_off` は不変（awase 自身が actuate する設定、決定1 の例外(2)・決定15）。`ime_toggle` の既定（`VK_KANJI`）は決定14 の移行と同時に空にする（T14、所有者回答 2026-09-25 で確定） |
| `crates/awase-settings/src/main.rs:2587-2591`（JIS 配列へ切替時の既定値書き込み） | 既定値を書き込む | `ime_on`/`ime_off` は不変。`ime_toggle`（`:2591`）だけ T14 で既定に揃える（決定15） |
| `src/engine/nicola_fsm.rs`（`forced_open_action`・`resolve_pending_thumb_as_single`）・`runtime/mod.rs:39-66`（`thumb_forced_open_actions`） | bare `keys.ime_*` 由来だけ | Engine 側は変えない。Windows 側で `config由来.or(役割由来)` を渡す（決定16） |
| `message_handlers.rs:878-887`・`:939-942`・`app/mod.rs:811-817`・`gji_monitor.rs`（`sync_ime_toggle_auto_detect`・`check_and_warn`） | GJI 以外すべてで適用、種別変更で消えない、同定の変化で再評価されない | 決定10 |
| `msime_key_assignment.rs`（`check_and_warn`） | 変換/無変換の「IME-オン/オフの割り当ては有害なので解除を」と案内 | 決定16（GJI では無変換/変換のトグルを尊重する）と逆向き。U7 でレジストリはユーザー設定と確定したので、トグルの割り当ては尊重する側（決定16）に揃えて案内の文言を直す（T12）。呼び出し条件は決定10-1 |

## 検討した代替案（棄却理由）

- **A. ADR-191 RM3 を維持し、警告を強化する（08 の (C) 案）**: 所有者決定1（ユーザー設定を尊重、握りつぶさない）に反する。カスタム割り当てが動かないまま。
- **B. 08 の (B) 案のまま（学習表がトグル以外を示すときだけ 0xF3/0xF4 の固定を外す）**: 半角/全角専用で、別キーをトグルにしたユーザーに対応しない（決定2に反する）。
  学習表が無いユーザーではカスタム割り当てが動かない。
- **C. `custom_table_overrides`（行の有無）を流用して固定を外す**: カスタム表はプリセットのコピーから始まるので、カスタム表の利用者全員で外れる（誤検出）。
- **D. `extract_ime_keys` をそのまま役割判定に使う**: (1) `IMEOn`/`IMEOff` 以外の行を捨てるので状態の完全性を見ず、ATOK プリセットの変換（Composition で `Convert`）を
  トグルと誤判定し、入力中の変換を壊す。(2) 別名表（`keymap.rs:58-68`）は `Hankaku/Zenkaku` を `VK_KANJI` だけに写し（`:60`）、0xF3/0xF4 に写らない（予測側の
  `mozc_tokens` と逆向きに食い違う）。(3) `custom_keymap_table` しか読まない（プリセット選択時は空、`lib.rs:46-54`）。
- **E. 実行時の観測から役割を推定する**: 所有者決定3で禁止。awase の注入・Suppress が観測を汚染する（ADR-191 の 98.5%→84.5%）。
- **F. 役割判定をやめて全キー受動にする（開閉トグルも書かない）**: 観測できないアプリ（TsfNative）で開閉のずれを直す手段が無くなる
  （ADR-189 の CI 実測: 固定セット導入前は8手順中4手順が反転せず）。所有者決定2の例外にも反する。
- **G. `config1.db` にユーザー設定を書き戻して awase 向けに整える**: 決定1（書き込まない）に反する。ADR-192(4) も書き換えを禁止。
- **H. プリセット TSV を同梱して汎用の状態表を組み立てる**（草案 rev0）: プリセットは有限で中身が変わらないので、定数表で足りる。
- **I. 学習表を役割の付与（拡大方向）にも使う**（草案 rev0）: 学習の誤りが能動的な誤書き込みになる（ADR-195(A)）。
- **J. 役割表を保持し、キャッシュの読み直し時だけ作り直す**（round4 までの決定8）: 今の `KeymapCache`/`RuntimeTableCache` の API は読み直したかを返さず、
  世代管理が要る。IME 同定ごとのキー付けも要る（無いと round5 S1 の後退）。打鍵時に求める方が単純（決定8）。
- **K. F13〜F24 の受信で `is_japanese_ime()` を真に上げる**（0xF3/0xF4 と同じ扱い）: F13〜F24 は物理キーが実在しうるので IME の証拠にならない。
  真に上げると force-ON 等の actuation 経路が解禁される（ADR-093・BUG-14 と同じ理由で不可）。フォーカスプローブによる偽への下降（決定18 の理由1）も防げない。
- **L. F13〜F24 の Down/Up の対応に `hook.rs` の `HookState` の親指ラッチを使う**: フックのスレッドの状態で、配送判定のあるメインスレッドから読むにはスレッドをまたぐ。
  同じ場所に #308 のラッチがあるのでそれを一般化する（決定18 (i)）。
- **M. 無変換/変換にも `shadow_action = Toggle` を付けて F キーと同じ経路で能動制御する**: `transport.rs:283-291` が無変換/変換を先に `Allow` するので、生キーと awase の書き込みで
  二重 actuation になる（BUG-46 型）。`Allow` を変えると ADR-141/153 の「@」対策（BUG-113/124）の隣を触ることになる。チョード優先も満たせない。ADR-192 決定3b の単独タップ確定点に合流する（決定16）。
- **N. 無変換/変換の役割由来の能動制御を、エンジン非活性（IME OFF）のときにも行う**: FSM が動かないので単独タップかチョードかを決める点が無い。所有者決定 U6 は「単独タップと解決したときだけ」。

## リスク

- **TsfNative での退行**: 役割が付かなくなったキー（例: 半角/全角を別機能にしたユーザー）は受動になり、観測できない窓では予測だけが頼り。
  それでも「ユーザーの設定が動かない」現状よりは良い、という判断が所有者決定1。
- **誤判定で能動側に倒れる**: 状態表の読み違い（未知コマンド、継承規則の誤り等）でトグルと誤判定すると、入力中の変換を壊す。
  決定4は「判定対象の開状態すべてで Close」を要求し、未知コマンドは「その他」に倒し、学習表の矛盾セルで受動に狭めることで、誤判定を受動側に寄せる。
- **`config1.db` は非公開フォーマット**: field 41/42/68 は Mozc 由来の非公式知識（`awase-gji-config/src/wire.rs:11-19`）。パース失敗は受動に縮退する。
- **プリセット定数表の陳腐化**: GJI の版でプリセット TSV が変わると定数表と食い違う。学習表の狭め（決定6-2）と ADR-196 の再検証で検出する。
- **KeyDown と KeyUp の間で役割が変わる**: キャッシュの読み直し（`config1.db` の保存直後、学習表の採否の変化）が同じ打鍵の Down と Up の間に入る場合。
  半角/全角は PR #308 のラッチが Down の判定を Up に持ち越すので起きない（起草時点では未実装だった）。F13〜F24 も一般化したラッチで同じ（決定18 (i)）。
  無変換/変換は Down で設定した `forced_open_action` が Up の解決まで残るが、その間に設定の再読込（`apply_config_update`、`runtime/mod.rs:1601-1612`）が入ると
  config 由来だけに戻り、その1打鍵は受動になる（二重 actuation にはならない方向なので記録だけ）。
- **ソフトウェアのリマッパーの F13〜F24 は受動**（決定18）: IME はユーザー設定どおり開閉するが、awase の belief は TsfNative では追随できずずれうる
  （F キーは `may_change_ime` の対象外、学習表のセルも無い）。能動にするには BUG-14 の原則（注入はユーザー意図にしない）を変える必要があり、範囲外。
- **エンジン非活性時の無変換/変換は受動**（決定16）: CUSTOM の行は `custom_table_overrides` で予測されない（背景2）ので、TsfNative では生キーで IME が開いても
  belief が OFF のままになりうる（エンジンが活性にならない）。観測できる窓では既存の追随で直る。
- **モード指定で開くキーの開いた後のモード**（決定13）: T1(c) で開くと確認できた後、開いた後のモードが IME の保存値になり、ユーザーが指定したモードと食い違いうる。
- **専用 Fn キーとの衝突**: ユーザーが GJI で、awase の専用 Fn キー（ADR-091、`muhenkan_solo_tap_dedicated_fn_key`、F15〜F24）と同じ F キーをトグルにすると、
  awase が送る専用 Fn キーで IME が開閉する。既存の注意書き（`src/config.rs:276-280`）の範囲で、awase の役割判定では防がない（注入は候補にならない）。
- **互換モードフラグが読めない（`None`）ときはトグル**（決定17）: 実は互換モードなのに値が読めない環境では、読めない割り当てを awase が上書きする。T1(d) で見直す。
- **awase 既定の `ime_on`/`ime_off`（Ctrl+変換/Ctrl+無変換）とユーザーの IME 設定の衝突**（決定15）: IME 側で同じキーを別コマンドに割り当てたユーザーでは awase の開閉が勝つ。所有者が awase 側の設定として受け入れた（config.toml で上書きできる）。
- **awase 自身が注入する IME キーの効果もユーザー設定に依存する**: `output/mod.rs:1239`（HalfWidthAlnum Exit で `VK_DBE_HIRAGANA`）や TSF warmup の F2 は、
  プリセットの Hiragana 行を前提にしている。CUSTOM で Hiragana を別機能（例: `IMEOff`）にした構成では、awase の warmup が IME を閉じうる。範囲外（別件）として記録だけする。
- **別件（予測の既存バグ）**: `custom_table_overrides` が効くのは `session_keymap` が MSIME/ATOK で古い `custom_keymap_table` が残る構成だけで、GJI はそこでカスタム表を無視する。
  正しくは「`session_keymap != CUSTOM` ならカスタム表を見ない」（決定8 (i) の生の値で1行）。役割判定とは独立なので本 ADR の範囲外。
- **複雑性**: 判定関数とプリセット定数表を新設する。U9 の拡大で、F13〜F24 の配送分岐1つ・リピート条件1つ・ラッチへの書き込み1箇所（決定18）と、
  親指キーの `forced_open_action` への役割由来の合成（決定16）が加わる。代わりに `is_open_toggle_for`・08 の (B) 案の分岐を撤去する
  （complexity-budget は未発効だが、撤去量を PR に記録する）。

## 移行・実装タスクの分割案

| # | 内容 | 既存 docs/tasks との対応 |
| --- | --- | --- |
| T0 | （完了）所有者確認。U1〜U10・Q2 と `keys.ime_toggle` 既定（空にする、決定15）まで回答済みで、未決は無い（決定3・決定11〜18） | — |
| T1 | 実機確認（(c) を最優先。決定13 で所有者の例〈モード指定で開くキー〉が対象になるかを決めるため）: (c) DirectInput の `CompositionMode*` で開くか、(a) 半角/全角を変えていないカスタム TSV に `Hankaku/Zenkaku` 行が残るか、(b) TSF 経路で 0x19 が `Hankaku/Zenkaku` 行に従うか（カスタム表で半角/全角だけ変えて Alt+半角/全角を押す1回。決定14 の移行の前提）、(d) 互換モードのチェックボックスを触っていない環境で `NoTsf3Override2` が無いか（決定17 の `None` の扱い）、(e) `Scancode Map` で F13 を出し、GJI の CUSTOM で F13 をトグルにした構成の実タイピング（TsfNative 1つ以上、決定18） | 08 の未確認点を引き継ぐ |
| T2 | `awase-gji-config`: 継承規則つきの状態表（CUSTOM のみ）、決定4の判定関数（対象キー名は `Hankaku/Zenkaku`・`F13`〜`F24`・`Muhenkan`・`Henkan`）、プリセット定数表（Mozc TSV との突き合わせテスト付き）、キー名→VK 写像の一本化（純粋関数）。`Kanji` 行は 0x19 に写さない。awase-windows 側: `KeyEffectKeymap` に生の `session_keymap`、`read_key_effect_keymap` の不在→既定 keymap（決定8） | 08 のタスク「判定を純粋関数として」 |
| T3 | 学習表による狭め（`state/`、決定6-2。半角/全角・無変換/変換。F キーはセルが無いので対象外）と食い違い記録。PR #308 の分岐を包含 | 01・06・PR #308 と経路を共有 |
| T4 | 予測経路の keymap/学習表取得をヘルパーに切り出し、`kp_run_inner` 冒頭で候補キーのときだけ役割を求めて enrich に渡す（決定8）、`vk.rs` の `is_open_toggle_for` 撤去（`vk.rs:1347/1360` のテストも）、`transport.rs` の Suppress 判定更新（影響表）。`architecture_guard.rs` の `bug116_...` の必須トークンを新しい判定に差し替え、`:816` の説明文を更新。バッチ前処理と `kp_run_inner` の間で `shadow_action` を読む経路が無いことの確認。PR #308 のラッチを候補キー全体の打鍵ごとのラッチに一般化。`vk.rs` の候補キー判定は T2 の `awase_gji_config::role::ROLE_CANDIDATE_VK_NAMES` から作る（定義を2箇所にしない）。**0x19 は決定14 の移行まで `hook.rs` の経路から動かさない** | 08 のタスク「影響洗い出し」 |
| T5 | （欠番）旧「`keys.ime_on`/`ime_off` の既定を空にする」は所有者回答（2026-09-25、U5 修正）で撤回。`ime_on`/`ime_off` の既定は残す（決定15）ので既定変更・移行のタスクは無い | — |
| T6 | ADR-192 警告の対象・文言の更新 | 08 論点 (A)・(C) |
| T7 | ADR-189/191/195 の status・summary 追記（RM3 置換、195(A) の追記）と、残る能動書き込みの棚卸し（09）への反映 | 10（status 同期）・08（191 summary 訂正）・09 |
| T8 | 決定10（別件の BUG・fix PR として先行してよい） | — |
| T9 | 決定18（F13〜F24）: 候補集合の入口（`ime_kind()` の早期 return の前）、ラッチへの `shadow_toggled` の書き込み（`key_pipeline.rs:328` の直後）、`was_down` の Down で昇格させない条件、`transport.rs::plan` の F キー分岐と `suppress_reason` のラベル。T4 の後 | — |
| T10 | 決定16（無変換/変換）: 親指キーの KeyDown で役割を求め、`config由来.or(役割由来)` を `set_thumb_forced_open_actions` に渡す。ADR-192 決定3b のテスト群（`src/engine/tests.rs`）に役割由来のケースを足す | — |
| T11 | 決定15 の文書追随（`ime_toggle` の既定を空にする T14 と同時、別タスク）: 同梱 `config.toml:27-29`、`docs/usage.html:674-675`・`:779`、`docs/usage.en.html` の対応箇所、`crates/awase-settings/src/main.rs:4856-4883` の説明。`ime_on`/`ime_off` の記述は変えない | 10（status・文書同期） |
| T12 | U7（確定）: MS-IME 本体の変換/無変換の役割（レジストリ）。トグルに当たるレジストリ値を実機で確認してから足す（既知の 0/1 は受動）。`check_and_warn` の案内文言 | — |
| T13 | 決定17: 互換モードのとき MS-IME 本体の半角/全角を受動に（レジストリ読み取りは予測経路と同じ間引き） | — |
| T14 | 決定14 の移行（T1(b) の後）: 0x19 を既知のトグルとして学習表の `Kanji` セルで狭める＋`keys.ime_toggle` の既定を空に（決定15、所有者回答で確定。JIS 切替の書き込み `main.rs:2591` を空に揃える、既存 config.toml に残る `VK_KANJI` の移行処理の要否の確認、0x19 が無修飾コンボに一致しないことの確認を含む） | — |

## テスト方針

- **ホスト Linux**（`cargo test -p awase-gji-config`、`cargo test --lib`、`cargo nextest run -p awase-windows --lib`）:
  - 決定4の判定関数: プリセット定数表と Mozc の ms-ime/atok TSV の突き合わせ、ATOK の変換＝受動、CUSTOM で半角/全角＝`CompositionModeHiragana`→受動、
    **半角/全角で Suggestion 行なし→継承でトグル／Suggestion にだけ別コマンド→トグルでない**、Composition 行だけ欠ける→受動、修飾行→対象外、
    候補外キー（0xF0/0xF1/0xF2・F1〜F12）→対象外、**CUSTOM で `ON`/`OFF` 行が無い表→半角/全角がトグル形でも受動**、
    **`session_keymap` が KOTOERI/MOBILE で古い `custom_keymap_table` が残る→定数表（表は読まない）／未知の値→受動**、
    **`NONE`→MSIME の定数表でトグル／CUSTOM で表が空→MSIME の定数表でトグル**。
    F13〜F24（決定18）: CUSTOM で F13＝DirectInput `IMEOn`・全開状態 `IMEOff`→トグル、MS-IME プリセットの F13（DirectInput 行のみ）→受動、
    BUG-64 型（F21=`IMEOn` だけ・F22=`IMEOff` だけ）→どちらも受動、修飾付きの F13→対象外。
    無変換/変換（決定16）: CUSTOM で全開状態 `IMEOff`→トグル、ATOK プリセット（Composition が `Convert`）→受動、Precomposition だけ `IMEOff`→受動（決定11）。
    モード指定で開く行（決定13）: T1(c) 前は DirectInput の `CompositionModeHiragana`→Open に数えない（受動）。
  - `config1.db`: **不在→MS-IME プリセット扱いでトグル（予測も MS-IME プリセット）／あるがパース失敗→受動／パスが解決できない→受動**。
  - 役割の取得: **`table_ime_kind()` が `None`（ATOK 等）→役割なし**（GJI の直後でも。round5 S1 の回帰）、役割が外れたキーは `shadow_action` が外れる（代入）。
  - 決定6の合成: 学習表の矛盾セル〈閉状態で開かない／未入力の開状態で閉じない〉で狭める・**変換中セル1つの矛盾では狭めない**・欠けセルでは狭めない・
    学習表だけでは広げない・両方不明→受動・`use_learned_keymap_table=false` では狭めない。
  - 決定10の純粋関数（MS-IME 本体同定→付ける、ATOK〈kind は MicrosoftIme・未同定〉→空、GJI→空、未検出→触らない）。
  - 決定18 の配送規則（`transport.rs` の `plan_tests`。`runtime/` は `#[cfg(windows)]` なので Linux では動かない、下記の Windows ターゲットのコンパイルと windows-build CI で確認）:
    最初の Down で書いた→Down/リピート/Up とも Suppress、書かなかった（`is_japanese_ime` 偽・フォーカスプローブで下降）→Down/Up とも Allow、ImmCross でも同じ、
    injected の F13→Allow、0xF3/0xF4 の規則は不変。ラッチの一般化は `omit_latch_step` と同じ純関数のホストテスト（`state/` はホストで動く）で、値が
    「この打鍵の最終的な `shadow_action`」であること（半角/全角で外した→`None`、F キーで書いた→`Some(Toggle)`）・リピートでの再利用・
    **別 scan を挟んだ Down→他キー Down→Up**（F13 Down→半角/全角 Down→F13 Up で、F13 の Up は `None`＝Allow）を固定する。`was_down` の Down で昇格しないこと。
  - 明示 config との重なり（決定8、round7 M1）: F13 が config.toml の `ime_toggle` にも書かれている→`shadow_action` なし、Engine の照合だけで開閉を1回だけ書く。
    半角/全角を `ime_on` に書いた場合も同じ（役割なし）。判定は純関数にして `state/` に置く。
  - 決定16: `src/engine/tests.rs`（ホスト）に、役割由来の `forced_open_action` で単独タップ→開閉要求、チョード→要求なし、`*_solo_tap_ime_action` 併設→役割由来は自己無効化、
    config 由来と役割由来が両方あれば config 由来、を足す（ADR-192 決定3b の既存テストと同じ形）。`config由来.or(役割由来)` の合成は純関数にしてホストでテストする。
  - 決定15: `KeysConfig::default()` の `ime_on`/`ime_off` が Ctrl+変換/Ctrl+無変換 のまま（既存テストで固定されていなければ足す）、config.toml に書いた値はそのまま読める
    （`src/config.rs` のテスト）。T14 で `ime_toggle` の既定が空になったこと（`KeysConfig::default()` と JIS 配列切替の書き込みの両方）を足す。
  - 決定17: 互換モード `Some(true)`→受動、`Some(false)`・`None`→トグル（純関数に切り出す）。
- **source-scanning ガード**（Linux）: `architecture_guard.rs` の `shadow_action` 書き込み箇所数、`bug116_...`（差し替え後のトークン）、`layer_boundary_guard`。役割の判定以外から `Toggle` を付ける経路が無いこと。
- **Windows ターゲットのコンパイル**: `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`（`runtime/` は `#[cfg(windows)]` で Linux のテストバイナリに存在しない）。
- **windows-build CI**: `ime_key_sequence_golden`（トグルで送るキー列）、`thumb_context_guard`。
- **CI 実機 E2E**（`.github/workflows/e2e-ime.yml` 系）: GJI 既定プリセットで半角/全角のトグルが従来どおり（ADR-189 の `--hz` 8手順）であることだけを確認する。
  F13〜F24 は CI の SendInput が injected になり能動経路を通らない（決定18）ので、CI では検証できない（T1(e) の実機で確認）。
  CUSTOM 構成は CI 上で CUSTOM の `config1.db` を作る手段が無い（awase-gji-config は読み取り専用、GUI 自動化は重い）ので、ホスト上の純粋関数テストで固定する。
- **実機（ユーザー実機）**: TsfNative（Chrome・VS Code・Windows Terminal）での実タイピング確認（API の読取り値だけで判断しない）。T1 の3点。

## 所有者回答（未決なし）

### 回答済み（2026-09-25 反映、覆さない）

| # | 問い（要約） | 所有者の回答 | 反映先 |
| --- | --- | --- | --- |
| U1 | 「IME ON の状態」はどこまでか | 全ての開状態で閉じるキーだけトグル | 決定11 |
| U2 | 実行時の受動的観測の禁止の範囲 | 禁止は役割の推定だけ。belief の観測追随は存続 | 決定12 |
| U3 | モードを指定して開くトグル | 開閉だけ書く。実機で DirectInput からモード指定で開くかが分かるまでは受動 | 決定13 |
| U4 | 0x19 の役割 | 現状維持を経て既知のトグル扱い | 決定14 |
| U5（2026-09-25 修正） | awase 既定の `keys.ime_toggle`/`ime_on`/`ime_off` | 当初回答「既定を空にする」を `ime_on`/`ime_off` について修正: 既定（Ctrl+変換/Ctrl+無変換）は**残す**。IME 設定から逆算する役割ではなく awase 自身が actuate する設定として扱う（当初の例外定義「ime_on/off キー（CTRL+無変換・変換）」どおり）。IME 側の割り当てとの衝突は awase 側の設定として受け入れる。`ime_toggle` の既定（`VK_KANJI`）は**空にする**（2026-09-25 確定。時期は U4 の既知のトグルへの移行と同時、T14） | 決定1（例外(2)）・決定5・決定14・決定15 |
| U10 | 既存 config.toml に書き出された旧既定値の扱い | U5 修正で `ime_on`/`ime_off` の既定が変わらなくなり、問い自体が消滅（移行処理なし） | 決定15 |
| Q2 | 同じキーが GJI のトグルで config.toml にも書かれている場合 | config.toml を優先し、awase の役割判定は付けない | 決定5・決定8・決定16 |
| U6 | 無変換/変換がトグルの設定のとき | 単独タップと解決したときだけ能動（チョード優先、ADR-192 決定3b の KeyUp 解決の合流点） | 決定16 |
| U8 | MS-IME 互換モードの半角/全角 | 受動 | 決定17 |
| U9 | 初期範囲の候補 | 半角/全角（0xF3/0xF4）・F13〜F24・無変換・変換をすべて含める（起草者推奨の「0xF3/0xF4 のみ」は却下） | 決定18（F13〜F24）・決定16（無変換/変換） |
| U7 | MS-IME 本体のレジストリを所有者決定3の「`config1.db`」に含めてよいか | 含める。`config1.db` と同様にユーザー設定の一次情報源で、実行時観測ではない（MS-IME 本体と同定できたときだけ使う決定10 は維持）。能動の対象は状態依存なくトグルと明確に判断できるキーだけで、ATOK プリセットの変換/無変換は受動 | 決定1・決定3・決定6-1・決定16 |

### 未決

- 無し（2026-09-25 の `keys.ime_toggle` 既定の回答で解消）。

## レビュー反映メモ

各ラウンドの指摘全文は scratchpad の `opus-adr199-round{1..7}.md`（リポジトリ外）。反映した指摘は本文に取り込み済みなので、ここには
**反映しなかった・形を変えて反映した判断**と**撤回した設計**だけを残す。

| round・指摘 | 扱い | 理由 |
| --- | --- | --- |
| r1 C1（読み方 C） | 形を変えて反映 | 所有者の追加発言で「ime_on/off キー」はトグルの定義そのものと確定し、2役割を1つに統合。(b) の「未入力のときだけ能動」の書き込み範囲は Stage 推定依存で複雑になるので決定5に書かず U1 へ |
| r1 C4（0x19 は必ず Alt 付き） | 形を変えて反映 | 修飾ガードに例外を足さず、0x19 は U4 まで hook 経路から動かさない |
| r1 M5（打鍵経路のファイル I/O） | 部分反映 | 予測経路と同じキャッシュを共有して I/O を増やさない。確認をタイマー・focus 変更へ移す案は現行予測にも同じ性質があり範囲外 |
| r2 N1（役割統合は所有者確認を経ていない） | 非反映 | 所有者発言は1つのキーが両方向を満たすものをトグルと定義しており、文言どおりに読むと統合になる。2役割案は MS-IME プリセットの F13・Hiragana・Katakana・Eisu が能動側に入り既定構成が変わる |
| r2 M3（起動直後はキャッシュが空） | 撤回 | 入れた「`peek()` で参照し種別確定時・リロード時に温める」は、`RuntimeTableCache` に `peek()` が無く成立しない（r4 R3）。現行は決定8（打鍵時に求める） |
| r3 S3（F13〜F24 はほぼ働かない） | 部分反映 | `Scancode Map` は非 injected の現実的な経路なので「ほぼ働かない」は言い過ぎ。削ると所有者発言の後半が初期範囲から消えるので U9 として所有者確認 |
| r3 m2（transport 用に `ImeRelevance` に bool を足す） | 形を変えて反映 | フィールドを足さず、enrich が付けた `shadow_action` と静的 VK 集合で判定 |
| r4 R1（F13〜F24 の二重の空振り） | 形を変えて反映→所有者回答で再反映 | 当初は修正案 (ii)（ラッチ）を採らず U9 の推奨を (b) にした。所有者が (a) を選んだので、PR #308 で入ったラッチの一般化＋配送規則1分岐＋リピート条件で対処（決定18） |
| r4 R2（OVERLAY も Mozc は既定に倒す） | 一部訂正 | 既定に倒れるのは `OVERLAY_FOR_TEST` だけで、`OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF` は overlay TSV だけが読まれる。いずれも「未知の値は受動」で結果が一致するので個別規則は足さない |
| r4 R3（役割表を保持して読み直し時に作り直す） | 撤回（r5） | 保持をやめ、打鍵時に求める形へ（代替案 J） |
| r5 S1（ATOK 切り替え後に古い Toggle が残る） | 反映 | S3 の形で構造的に解消（打鍵時の `table_ime_kind()` で分岐、`None` は受動）。回帰テストを追加 |
| r5 S2（共有キャッシュの型の変更が隠れている） | 形を変えて反映 | 3値の戻り値にせず、不在のとき `from_config(None, None, &[])` を返す＋生の `session_keymap` を1フィールド足す、の2点に縮めた。副作用（不在で予測も MS-IME プリセットで動く）は決定8に明記 |
| r5 S6（`kp_run_inner` の行番号） | 非反映 | 指摘が誤り。`e3969f40` で `kp_run_inner` の宣言は `key_pipeline.rs:264`、`enrich_ime_relevance` の呼び出しは `:265`（grep で確認、r6 も確認済み） |
| r6 N3（`custom_table_overrides` のプリセット差分化） | 範囲外へ移動 | 役割判定と独立した予測の既存バグで、正しい修正も「`session_keymap != CUSTOM` ならカスタム表を見ない」。T6・影響表・複雑性節から外しリスク節に1行記録 |
| r5 冗長2（決定10を別 BUG に分ける） | 部分反映 | BUG ファイルの起票は本作業の編集範囲外。決定10を圧縮し、別の fix PR として先行してよいと明記 |
| r5 冗長3（U1 を所有者への質問から外す） | 非反映 | U1 は所有者定義の解釈で起草者が確定しない。初期範囲で差が出る構成が狭いことと、回答までは (a) で実装することを U1 に明記した（所有者は (a) と回答、決定11） |
| 所有者回答（2026-09-25） | 反映 | U1〜U6・U8・U9 を決定11〜18 に昇格。U9 の起草者推奨（0xF3/0xF4 のみ）は却下された。反映中に U10 を発見 |
| r7 M1（明示 config と役割由来 Toggle の二重書き込み） | 反映 | 決定8 に「重なったら config 優先・役割を付けない」を追加（決定16 と同じ向き）。`match_event` 側のガード案は役割が config に勝つので不採用。所有者確認は Q2 |
| r7 M2（U10 の前提が未裏取り） | 反映 | 「保存したことがある人は従来どおり、ない人は更新時に空になる（分布不明）」に訂正。旧推奨 (a) は取り下げ、(a') を足して Q1 として所有者に問い直す。その後 U5 修正（`ime_on`/`ime_off` の既定を残す）で U10 自体が消滅した |
| r7 M3（ラッチの意味が逆・scan 不一致） | 反映 | 修正案 (a)（値を最終的な `shadow_action` に統一）と (b)（F キーの scan 不一致は `None`）の両方を採用。テストを「別 scan を挟んだ Down→他キー Down→Up」に具体化 |
| r7 m1〜m7・簡素化案 | 未反映 | 本反映の範囲外（中程度3件と所有者回答のみを反映）。実装前の次 round で扱う |
| 所有者回答 U7（2026-09-25） | 反映 | レジストリを `config1.db` と同様の一次情報源とし決定3 に明記。MS-IME 本体の変換/無変換は、トグルと判断できる値を実機で確認してから能動に足す（T12） |
| 所有者回答 U5 修正・Q2（2026-09-25） | 反映 | `keys.ime_on`/`ime_off` の既定は空にせず awase 自身の actuate 設定として残す（決定1 の例外(2)・決定15）。既定を空にする記述と T5（既定変更・移行）を撤回し、U10 は消滅。Q2 は config 優先・役割なしで確定。`keys.ime_toggle` 既定だけ未確認として残す（下の行で確定） |
| 所有者回答 `keys.ime_toggle` 既定（2026-09-25） | 反映 | 既定（`VK_KANJI`）は空にする（U4 の移行と同時、T14）で確定。決定14・決定15・T0/T11/T14・影響表・テスト方針の「未確認」を確定に直し、未決節を空にした |
| round8 軽微（r7 M2 の行に U10 消滅を追記） | 反映 | 上の r7 M2 の行に追記 |
| PR #315 コードレビュー M1（overlay） | 反映 | 決定4 末尾の「overlay は候補外なので使わない」は U9 で変換/無変換を候補に入れる前の記述だった。overlay 100 は変換/無変換を、未知の overlay は全候補を受動にする |
| PR #315 コードレビュー M2（`config1.db` 不在の影響） | 反映 | 決定8 の呼び出し元に PR #308 の Toggle 除外を追加し、学習プロセスの突き合わせも不在を既定の既知構成として扱うよう揃えた |
| round8 軽微（決定8 の「明示値」） | 反映 | 読み込み後の `KeysConfig` ではユーザーが書いた値と既定値を実行時に区別できないので、比較対象は既定値を含む実効値だと明記（既定の無修飾は `VK_KANJI` だけで候補キーと重ならない） |
