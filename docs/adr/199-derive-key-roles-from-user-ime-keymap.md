---
id: ADR-199
title: |-
  キーの役割はユーザーのIMEキー設定から逆算する。awaseは原則として受動的に対応し、
  能動的に制御する例外は「IME ON/OFF トグル」という役割のキーだけにする（VKで固定しない）
summary: |-
  半角/全角が開閉トグルになるのは Mozc/GJI のプリセットの「キー名×状態」割り当ての結果で、キーの性質ではない。
  それなのに awase は ADR-189/191 の固定セット（0x19/0xF3/0xF4）を VK 基準でトグルとして能動的に書いていた。
  所有者決定（2026-09-24）: (1) 役割はユーザーの IME キー設定から逆算し、原則受動。(2) 能動制御の例外は「IME ON/OFF トグル」の
  役割を持つキーだけ（キー名・VK で固定しない）。(3) 設定を知る手段は `config1.db` と学習表（ADR-195/196）だけ。
  要点: プリセットは定数表・動的逆算は CUSTOM だけ・学習表は狭める方向だけ・初期範囲の候補は半角/全角（0xF3/0xF4）だけ・
  役割は保持せず打鍵時に求める・`config1.db` 不在は既定プリセット扱い。所有者確認が要る点は「未決事項」。
status: |-
  **草案（未決事項あり、所有者確認待ち）。** 2026-09-24 起草、opus round1〜round6 反映済み。
  決定1〜3（原則・例外・取得手段）は所有者決定で確定。決定4以降は本 ADR の提案で、未決事項 U1・U3〜U9 の所有者確認が未了。
  実装は未着手。review-2026-09-24-08 の方針（(B) 案・0x19 現状維持）を一般化・置換する。
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

# ADR-199: キーの役割をユーザーの IME キー設定から逆算する（受動が原則、能動は IME ON/OFF トグルの役割だけ）

## ステータス

frontmatter の `status` 参照。裏取り基準は worktree `docs/adr-user-keymap-passive`（origin/develop `e3969f40`、PR #306 まで）で、
本文中の `crates/...:NNN` の行番号はこの基準で確認した。

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
（未確定文字の扱い）が `Hankaku/Zenkaku` 行と食い違うことから OS 側で開閉していると推定する（詳細は U4、確認は T1(b)）。
開閉（役割の判定）では 0x19 と 0xF3 は一致する。Windows では 0x19 が `KeyEvent::KANJI` にならない（`keyevent_handler.cc` L87-93）ので、
プリセットの `Kanji` 行は使われない行である。

### 4. 直近の関連決定

- review-2026-09-24-08: 「採用中の学習表でセルがトグル以外を示すときだけ 0xF3/0xF4 の固定を外す」（(B) 案）、0x19 は現状維持。
  前提だった「学習表が採用されない」問題は 01（PR #305）で解消済み。**本 ADR は 08 を一般化・置換する**（固定セットを「外す」のではなく、最初から役割で付ける）。
- review-2026-09-24-07 / ADR-198 決定3: ADR-176 手動較正は撤去済み（PR #304）。ADR-191 決定4（「較正」）はそのまま ADR-195 の学習に読み替える。
- review-2026-09-24-06: 学習表の指紋書き込みと `staleness::check` の実行時配線は実装済み。
- review-2026-09-24-03: 学習プロセス（`awase-keymap-learn-win`）の同梱は PR #306 でマージ済み。

## 決定

### 決定1（所有者決定・原則）: 役割はユーザーの IME キー設定から逆算し、awase は受動的に対応する

awase は、ユーザーが IME のキー設定（GJI ならキー設定エディタ、MS-IME なら「キーとタッチのカスタマイズ」）で決めた内容から、
各キーの**役割**を逆算する。役割が例外（決定2）に当たらないキーでは、awase は**受動的**に振る舞う:

- 物理キーを Suppress しない（生キーを IME へ通す）。
- IME の設定を書き換えない（`config1.db`・レジストリは読み取り専用。現行の方針どおり）。
- キーの効果を awase の書き込みで上書きしない（`shadow_action` を付けない）。
- belief は ADR-191 の予測（`KeyEffectPredicted`）と観測に追随する（この項は U2 の解釈に依存する。決定3の注記参照）。

### 決定2（所有者決定・例外）: 能動制御するのは「IME ON/OFF トグル」の役割を持つキーだけ。判定は役割で行い、キー名や VK では固定しない

**IME ON/OFF トグル**: ユーザーの設定で、直接入力（IME OFF）の状態から押すと IME ON（ひらがな・カタカナ等のモードを指定して開くものを含む）になり、
IME ON の状態から押すと IME OFF に遷移する振る舞いになっているキー（所有者発言 2026-09-24:「ime_on/off は直接入力の状態から IME ON/ひらがな/カタカナ に設定して、
IME ON の状態なら IME OFF に遷移するようなキーです。ユーザーの設定てそうなっているキーをトグルキーとみなします」）。したがって:

- ユーザーが半角/全角を別の機能（例: ひらがなモード）にしたら、半角/全角は例外に当たらず、awase は触らない。
- ユーザーが別のキー（例: F13）をトグルにしたら、そのキーを awase が能動制御する（候補キーの範囲は決定4・U9）。

「IME ON の状態」の範囲は U1、判定式は決定4（提案）。

### 決定3（所有者決定・取得手段）: ユーザー設定は `config1.db` と学習表だけから知る。実行時の受動的観測は使わない

- GJI: `config1.db`（`awase-gji-config` が読む）。
- 学習表: ADR-195/196 の `awase-keymap-learn` が awase をバイパスして注入学習し、自己検証・採用判定を通した表。
- 実行時に「キーを押したら IME がどうなったか」を観測して役割を推定することはしない。awase 自身の注入・Suppress・belief 更新が観測を汚染し、
  正しい役割を学べないため（ADR-191 の実測: awase を通すと仕様からの一致率が 98.5%→84.5% に落ちた）。

**本 ADR の解釈（所有者確認 U2）**: 「実行時の受動的観測は使わない」は**役割を知る手段**についての決定と解釈する。
belief を観測に追随させる ADR-187/191 の仕組み（生キーを通した後の再読取り、`kp_stage_mode_key_follow`、drift 補正）は状態の追随なので存続させる。
観測を役割判定に**フィードバックする経路は作らない**（例: 「観測では半角/全角で開閉が反転しなかったので役割をトグルから外す」は禁止）。

### 決定4（提案）: 役割の判定式と候補キー

**状態表**: GJI の `(Mozc status, キー名) → コマンド`。判定に使う状態は DirectInput（閉）と、開状態 Precomposition・Composition・Conversion。
Suggestion/Prediction/ZeroQuerySuggestion は継承規則（背景1）で**実効コマンド**を求め、判定は実効コマンドで行う
（例: プリセットの半角/全角は Suggestion 行が無いが、Composition の `IMEOff` を継承するので Close）。

コマンドは3類に分ける（`awase-gji-config/src/command.rs:65-86` `classify_command` を拡張）:

- **Open**: 閉状態から開く。`IMEOn`、および DirectInput 行の `CompositionMode*`/旧名 `InputMode*`（Mozc `keymap.cc` L460-471 が DirectInput に登録。
  所有者定義の「ひらがな/カタカナに設定して」に当たる）。※ `kCompositionModeXCommandSupported` が偽のビルドでは DirectInput の `CompositionMode*` は
  `NONE` で登録される（同 L472-483）。Windows 版 GJI でどちらかは未確認（T1(c)）で、確認までは `CompositionMode*` を Open に数えない（受動側に倒す）。
- **Close**: 開状態から閉じる。`IMEOff`・`CancelAndIMEOff`。
- **その他**: 上記以外（`Convert`・`Reconvert`・`ToggleAlphanumericMode` 等）、未知のコマンド、実効コマンドが無い（何もしない）。

**IME ON/OFF トグル**: DirectInput が Open、かつ**判定対象の開状態すべて**が Close（判定対象は U1。推奨は3状態すべて、ADR-191 決定1-2 の「状態完備」と同じ）。
**CUSTOM ではさらに、awase の書き込み手段が効くことを要求する**: 実効の表に DirectInput の `ON`→Open と、判定対象の開状態すべての `OFF`→Close があること。
awase が送る `VK_IME_ON`/`VK_IME_OFF`（0x16/0x1A）も Mozc では `KeyEvent::ON`/`OFF` に写され（`win32_base_keyevent_handler.cc` L84/L94）、効果はキーマップの
`ON`/`OFF` 行で決まる。キー設定エディタは `ON`/`OFF` 行を非表示のまま保存時に書き戻す（`keymap_editor.cc` L125-127・L369-372・L447）が、インポートや手作りの表では
欠けうる。欠けた表で半角/全角をトグルと判定すると、Suppress したうえで送る `VK_IME_ON/OFF` を GJI が無視し、誰も開閉しない「二重の空振り」（ADR-119 型）になる。
書き込み手段もユーザー設定なので、条件に含めるのが「逆算」と一貫する。プリセットは4種とも `ON`/`OFF` 行を持つ（Mozc TSV で確認済み）ので定数表側では不要。
当たらないキーは**受動**（例: ATOK プリセットの変換/無変換、DirectInput でだけ `IMEOn` の MS-IME プリセットの F13〈`ms-ime.tsv` L134〉・Hiragana/Katakana、Mozc の `ON`/`OFF`）。

**候補キー（初期範囲）は無修飾の半角/全角（0xF3/0xF4）だけ**（提案）。候補キー集合はここ1箇所で定義する:

- 0x19（`VK_KANJI`）は物理的に必ず Alt 付きで届く（背景1）ので無修飾ガードを通らない。U4 の結論まで `hook.rs` の静的 `Toggle` の経路から動かさない（候補集合には入れない）。
- F13〜F24 は U9 で所有者が (a) を選んだ場合だけ加える（推奨 (b)〈加えない〉。制約と費用は U9）。
- 文字キー・Space・親指キー（変換/無変換、U6）は Engine が先に消費するので対象外。修飾付きの行も対象外（決定5）。
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
kotoeri/mobile のユーザーで使われていない古い `custom_keymap_table` を評価してしまう）。`overlay_keymaps` は変換/無変換にしか効かず候補外なので使わない。

### 決定5（提案）: 役割を持つキーで awase が「書いてよい」範囲

- **書いてよいのは開閉軸だけ**。手段は冪等な `VK_IME_ON`/`VK_IME_OFF`（ADR-189/191 決定1-1 と同じ）。変換モード軸（ひらがな/カタカナ）は書かない。
  トグルの役割を持つキーは、物理キーを Suppress し、`!belief` を目標に冪等キーを送る（現行 ADR-189 の仕組みそのもの、`ShadowImeAction::Toggle`）。
- モードを指定して開くキー（DirectInput→`CompositionModeFullKatakana` 等）は、`VK_IME_ON` だけでは開いた後のモードが IME の保存値になり、
  ユーザー設定と食い違いうる（Mozc は conv を開閉にまたがって保存する、ADR-186）。扱いは U3。
- **修飾付きは対象外**（初期範囲）。`enrich_ime_relevance` の無修飾ガード（`runtime/mod.rs:567-569`）と `extract_ime_keys` の修飾行除外に揃える。
  修飾付きのユーザー設定（例: MS-IME の Ctrl+Space トグル）は、既存の Engine 自動キー（`sync_ime_toggle_auto_detect`）が担う（適用条件は決定10で締める）。
- **awase 自身の設定（`keys.ime_on`/`ime_off`/`ime_toggle`）のうち config.toml に明示された値は、ユーザーが awase に明示した能動制御として別軸で存続する**
  （ADR-191 の「ユーザー設定はユーザーが何をしたいかの軸」）。既定値の扱いは U5。
- IME の設定（`config1.db`・レジストリ）へは書かない（従来どおり）。

### 決定6（提案）: 情報源の優先順位と縮退動作

1. **役割を「付ける」根拠は `config1.db`（GJI）だけ**。決定4の式で評価する。MS-IME 本体のレジストリは既に予測と能動（修飾付きトグルの自動キー）の
   両方で使っている（背景2）。これを所有者決定3の「`config1.db`」に含めてよいかは事後確認（U7）。
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
   **互換モード（以前のバージョン）はこの前提が成り立たない**ので含めない。扱いは U8。

### 決定7（提案）: IME ごとの扱い

| IME | 設定の取得手段 | 役割の決め方 | 現行からの変化 |
| --- | --- | --- | --- |
| GJI（プリセット ATOK/MS-IME 等） | `config1.db`（`session_keymap`=プリセット）＋プリセット定数表＋学習表（狭めのみ） | 決定4・6。半角/全角はトグル。変換/無変換は候補外で受動（U6） | 既定構成では不変（`config1.db` 不在も既定プリセット扱い〈決定6-3〉。ホストテストで固定） |
| GJI（CUSTOM） | `config1.db` の `custom_keymap_table`＋学習表（狭めのみ） | 決定4・6。半角/全角を別機能にしていれば受動 | 0xF3/0xF4 固定が外れうる（U9 で (a) なら F13〜F24 に役割が付きうる） |
| GJI（ATOK プリセット＋古い `custom_keymap_table` が残る構成） | `config1.db` | GJI は ATOK 選択時に custom 表を無視する（ADR-186(c)、`gji_charset_autodetect.rs:207-214`）ので、custom 表を読まない | 不変 |
| GJI（`session_keymap` が未知の値） | `config1.db` | 受動（決定4） | 0xF3/0xF4 固定が外れる |
| MS-IME 本体（新しいバージョン） | 仕様（半角/全角は変更不可）＋学習表（狭めのみ）＋レジストリ（既存、U7） | 半角/全角はトグル（決定6-4）。修飾付きトグルは既存の自動キー（決定10） | 不変 |
| MS-IME 本体（互換モード） | 互換モードフラグ（ADR-197 決定4）のみ | U8 | U8 次第 |
| ATOK 本体・Japanist・その他 | 無し | 受動。ただし 0x19 は U4/U5 の結論まで現行どおり | MS-IME レジストリ由来の自動トグルキーが効かなくなる（決定10）。それ以外は不変 |

### 決定8（提案）: 役割は保持せず、候補キーの打鍵のときだけ求める

- **役割表は持たない**（round5 S3）。`kp_run_inner` の冒頭（`enrich_ime_relevance` の直前、`key_pipeline.rs:265`、`&mut self`）で、
  イベントが候補キー（決定4、無修飾の 0xF3/0xF4）のときだけ役割を求め、enrich に渡して `shadow_action = role.map(..)` を**代入**する
  （付け外しを1回で決める。代入は1箇所のままで `ime_relevance_shadow_action_writes_are_accounted_for` の件数は不変。
  ただし `is_open_toggle_for` の文字列を前提にした別のガード〈`bug116_...`〉の差し替えが要る、T4）。
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
  `read_key_effect_keymap` の呼び出し元は予測（`key_pipeline.rs:1958`）のほかに2つあり、不在時の挙動がそれぞれ変わる（どちらも改善の方向）:
  ADR-192 の警告（`runtime/mod.rs:1252` `check_state_dependent_mode_keys`）は、全対象 VK が `CannotPredict(AmbiguousKeymap)`（`key_effect_table.rs:527-531`）
  から MSIME 表での分類になる。学習プロセスの指紋（`key_effect_runtime.rs:442` `current_fingerprint_probe`、`awase-keymap-learn-win` main.rs:470・942）は
  `Unavailable` から計算可能になり、`Rejected(FingerprintUnavailable)` で棄却されず採用されうる（ADR-196 の挙動の変化。書き手と読み手が同じ関数なので指紋は構造的に一致する）。
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
- `keys.ime_detect`（`src/config.rs:495-510`、VK 固定で belief を動かす静的規則）: 物理キーを消費しない belief の追随（U2 の範囲）なので対象外。

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

## 既存 ADR・実装への影響

| 対象 | 現状 | 本 ADR 後 |
| --- | --- | --- |
| ADR-189（固定セット 0x19/0xF3/0xF4） | VK 基準で常にトグル（GJI・MS-IME 本体） | 「既定プリセットの半角/全角がトグルの役割を持つ」場合の一例に格下げ。ステータスに「ADR-199 で役割判定に一般化」と追記 |
| ADR-191 決定1-1（静的に残す唯一の例外）・RM3（固定が常に勝つ） | 固定セットは表・設定と矛盾しても勝つ | **置換**。ユーザー設定から逆算した役割が勝つ。決定1-2（表駆動の追加、状態完備条件）は決定4の判定式として採用 |
| ADR-192（状態依存キーの警告） | 0xF3/0xF4 のカスタム行で `UserOverride`（ログのみ） | 役割が付かないキーは受動なので「awase が握りつぶす」警告は不要。状態依存（受動キー）の警告は存続。対象 VK（`state_dependent_key_warning.rs:10`）の見直しは T6。`config1.db` 不在時は `AmbiguousKeymap` から MSIME 表での分類に変わる（決定8） |
| ADR-195(A)（actuation 許可リストを学習で自動拡張しない） | 許可リスト＝ADR-189＋ユーザー明示 config | **軽微な追記**。「学習で拡張しない」は維持（決定6-2）。許可リストの出どころが「`config1.db` から逆算した役割」に変わる |
| ADR-196（学習結果が真実） | 予測にのみ適用。`config1.db` 不在の GJI では指紋 `Unavailable` で学習結果を棄却 | 役割判定では狭める方向にだけ適用（決定6-2）。不在時も指紋が計算でき、学習結果が採用されうる（決定8） |
| ADR-186/187（ATOK の変換/無変換の追随） | 受動（follow） | 初期範囲では不変（候補外、U6）。U1・U6 の答え次第で見直し |
| ADR-197（MS-IME 互換モード） | 検出ロジック撤回、フラグ読取のみ | U8 次第（(a) なら互換モードフラグを半角/全角の役割判定に使う） |
| ADR-176（較正） | 撤去済み（ADR-198 決定3） | 影響なし |
| review-08（(B) 案・0x19 現状維持） | 方針決定済み・未実装 | **置換**。(B) は決定6-2 の特殊ケースとして包含。0x19 は U4/U5 次第 |
| `vk.rs:152-196`（`shadow_effect`・`is_open_toggle_for`） | VK で効果を決める | `is_open_toggle_for` を撤去し役割の参照に置換。0x19 の静的 `Toggle` は U4 まで、0x16/0x1A の静的 `TurnOn`/`TurnOff` は存置（決定9） |
| `runtime/mod.rs:551-578`（`enrich_ime_relevance`） | `table_ime_kind()` と VK で `Toggle` を付けるだけ | `kp_run_inner` で求めた役割を受け取り、候補キーには `shadow_action = role.map(..)` を代入（決定8）。早期 return は現行のまま（0xF3/0xF4 は `ime_kind()` を通る）。バッチ前処理では候補キーに触らない |
| `runtime/transport.rs:293-340`（Suppress 判定） | `is_dbe_mode_key_down` が `is_open_toggle_for` で 0xF3/0xF4 の KeyDown を常に Suppress | 役割を引き直さない（`ImeRelevance` にフィールドは足さない）。`is_dbe_mode_key_down` を「KeyDown かつ `shadow_action == Some(Toggle)` かつ VK が 0xF3/0xF4」に置き換える（0xF3/0xF4 は `is_japanese_ime()` が真に上がるので二重の空振りにならない）。`:322-328` のコメント前提を更新。無変換/変換の先行 `Allow`（`:282-291`）は不変 |
| `gji_charset_autodetect.rs:287-317`（`read_config1_db`・`read_key_effect_keymap`） | ファイル不在も読取り/パース失敗も `None` | 不在（パス解決済みかつ `NotFound`）は `from_config(None, None, &[])`、それ以外の失敗は `None`。呼び出し元3つ（予測・ADR-192 警告・学習プロセスの指紋）すべてに効く（決定8） |
| `key_effect_predictor.rs`（`KeyEffectKeymap`） | `preset` だけ保持 | 生の `session_keymap` を保持（決定8）。`config1.db` 不在で予測が MS-IME プリセットで動く |
| `tests/architecture_guard.rs:4516-4541`（`bug116_shift_katakana_guards_are_present_in_production_code`）・`:816` の説明文 | transport.rs 本番コードに `is_open_toggle_for` があることを assert（Linux での Suppress 判定の唯一の防波堤） | 必須トークンを新しい判定（`Some(ShadowImeAction::Toggle)` と 0xF3/0xF4 の組）に差し替え、否定側メッセージと `:816` の説明を更新（T4。削るだけにしない） |
| `awase-gji-config`（`extract_ime_keys`・`mozc_key_to_vk_name`） | 状態完備を見ない。`Hankaku/Zenkaku`→`VK_KANJI` のみ（doc も不正確） | 継承規則つきの状態表と決定4の判定関数（純粋関数）を追加。キー名→VK 写像（`Hankaku/Zenkaku`→0xF3/0xF4）を予測側と一本化し、別名表の doc を直す |
| `state/key_effect_table.rs:511-`（ADR-192 分類） | 0x19 を `Kanji` として独立扱い | U4 の結果に従う |
| `src/config.rs:581-583`（`keys.ime_on`/`ime_off`/`ime_toggle` 既定）・`engine.rs:975` | awase の既定値で Engine が消費 | U5 |
| `message_handlers.rs:878-887`・`:939-942`・`app/mod.rs:811-817`・`gji_monitor.rs`（`sync_ime_toggle_auto_detect`・`check_and_warn`） | GJI 以外すべてで適用、種別変更で消えない、同定の変化で再評価されない | 決定10 |
| `msime_key_assignment.rs`（`check_and_warn`） | 変換/無変換の「IME-オン/オフの割り当ては有害なので解除を」と案内 | ユーザー設定を尊重する原則と逆向き。U6 の間は案内の内容は据え置き、矛盾を status に記録。呼び出し条件は決定10-1 |

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

## リスク

- **TsfNative での退行**: 役割が付かなくなったキー（例: 半角/全角を別機能にしたユーザー）は受動になり、観測できない窓では予測だけが頼り。
  それでも「ユーザーの設定が動かない」現状よりは良い、という判断が所有者決定1。
- **誤判定で能動側に倒れる**: 状態表の読み違い（未知コマンド、継承規則の誤り等）でトグルと誤判定すると、入力中の変換を壊す。
  決定4は「判定対象の開状態すべてで Close」を要求し、未知コマンドは「その他」に倒し、学習表の矛盾セルで受動に狭めることで、誤判定を受動側に寄せる。
- **`config1.db` は非公開フォーマット**: field 41/42/68 は Mozc 由来の非公式知識（`awase-gji-config/src/wire.rs:11-19`）。パース失敗は受動に縮退する。
- **プリセット定数表の陳腐化**: GJI の版でプリセット TSV が変わると定数表と食い違う。学習表の狭め（決定6-2）と ADR-196 の再検証で検出する。
- **KeyDown と KeyUp の間で役割が変わる**: キャッシュの読み直し（`config1.db` の保存直後、学習表の採否の変化〈学習の完了・再検証・陳腐化〉）が同じ打鍵の Down と Up の間に入ると、
  Down は Suppress・Up は Allow になり、IME に孤立した KeyUp が届きうる（BUG-131/132 と同系統）。窓は「その直後に半角/全角を押している最中」に限られるので、
  ラッチは足さずに記録だけする。実害が出たら `vk::should_release_thumb_latch` と同じ scan 一致の形で足す。
- **awase 自身が注入する IME キーの効果もユーザー設定に依存する**: `output/mod.rs:1239`（HalfWidthAlnum Exit で `VK_DBE_HIRAGANA`）や TSF warmup の F2 は、
  プリセットの Hiragana 行を前提にしている。CUSTOM で Hiragana を別機能（例: `IMEOff`）にした構成では、awase の warmup が IME を閉じうる。範囲外（別件）として記録だけする。
- **別件（予測の既存バグ）**: `custom_table_overrides` が効くのは `session_keymap` が MSIME/ATOK で古い `custom_keymap_table` が残る構成だけで、GJI はそこでカスタム表を無視する。
  正しくは「`session_keymap != CUSTOM` ならカスタム表を見ない」（決定8 (i) の生の値で1行）。役割判定とは独立なので本 ADR の範囲外。
- **複雑性**: 判定関数とプリセット定数表を新設する。代わりに `is_open_toggle_for`・08 の (B) 案の分岐を撤去する
  （complexity-budget は未発効だが、撤去量を PR に記録する）。

## 移行・実装タスクの分割案

| # | 内容 | 既存 docs/tasks との対応 |
| --- | --- | --- |
| T0 | 未決事項 U1・U3〜U9 の所有者確認 | — |
| T1 | 実機確認（(c) を最優先。U3 と所有者の例〈モード指定で開くキー〉が対象になるかを決めるため）: (c) DirectInput の `CompositionMode*` で開くか、(a) 半角/全角を変えていないカスタム TSV に `Hankaku/Zenkaku` 行が残るか、(b) TSF 経路で 0x19 が `Hankaku/Zenkaku` 行に従うか（カスタム表で半角/全角だけ変えて Alt+半角/全角を押す1回） | 08 の未確認点を引き継ぐ |
| T2 | `awase-gji-config`: 継承規則つきの状態表（CUSTOM のみ）、決定4の判定関数、プリセット定数表（Mozc TSV との突き合わせテスト付き）、キー名→VK 写像の一本化（純粋関数）。`Kanji` 行は 0x19 に写さない。awase-windows 側: `KeyEffectKeymap` に生の `session_keymap`、`read_key_effect_keymap` の不在→既定 keymap（決定8） | 08 のタスク「判定を純粋関数として」 |
| T3 | 学習表による狭め（`state/`、決定6-2）と食い違い記録 | 01・06 と経路を共有 |
| T4 | 予測経路の keymap/学習表取得をヘルパーに切り出し、`kp_run_inner` 冒頭で候補キーのときだけ役割を求めて enrich に渡す（決定8）、`vk.rs` の `is_open_toggle_for` 撤去（`vk.rs:1347/1360` のテストも）、`transport.rs` の Suppress 判定更新（影響表）。`architecture_guard.rs` の `bug116_...` の必須トークンを新しい判定に差し替え、`:816` の説明文を更新。バッチ前処理と `kp_run_inner` の間で `shadow_action` を読む経路が無いことの確認。**0x19 は U4 まで `hook.rs` の経路から動かさない** | 08 のタスク「影響洗い出し」 |
| T5 | awase 既定値 `keys.ime_toggle`/`ime_on`/`ime_off` の扱い（U5 の結論に従う。ルート `awase` クレート、`src/engine/tests.rs`） | 08 |
| T6 | ADR-192 警告の対象・文言の更新 | 08 論点 (A)・(C) |
| T7 | ADR-189/191/195 の status・summary 追記（RM3 置換、195(A) の追記）と、残る能動書き込みの棚卸し（09）への反映 | 10（status 同期）・08（191 summary 訂正）・09 |
| T8 | 決定10（別件の BUG・fix PR として先行してよい） | — |

## テスト方針

- **ホスト Linux**（`cargo test -p awase-gji-config`、`cargo test --lib`、`cargo nextest run -p awase-windows --lib`）:
  - 決定4の判定関数: プリセット定数表と Mozc の ms-ime/atok TSV の突き合わせ、ATOK の変換＝受動、CUSTOM で半角/全角＝`CompositionModeHiragana`→受動、
    **半角/全角で Suggestion 行なし→継承でトグル／Suggestion にだけ別コマンド→トグルでない**、Composition 行だけ欠ける→受動、修飾行→対象外、
    候補外キー（0xF0/0xF1/0xF2・F1〜F12）→対象外、**CUSTOM で `ON`/`OFF` 行が無い表→半角/全角がトグル形でも受動**、
    **`session_keymap` が KOTOERI/MOBILE で古い `custom_keymap_table` が残る→定数表（表は読まない）／未知の値→受動**、
    **`NONE`→MSIME の定数表でトグル／CUSTOM で表が空→MSIME の定数表でトグル**。
    U9 で (a) なら: CUSTOM で F13＝DirectInput `IMEOn`・全開状態 `IMEOff`→トグル、MS-IME プリセットの F13（DirectInput 行のみ）→受動、を追加。
  - `config1.db`: **不在→MS-IME プリセット扱いでトグル（予測も MS-IME プリセット）／あるがパース失敗→受動／パスが解決できない→受動**。
  - 役割の取得: **`table_ime_kind()` が `None`（ATOK 等）→役割なし**（GJI の直後でも。round5 S1 の回帰）、役割が外れたキーは `shadow_action` が外れる（代入）。
  - 決定6の合成: 学習表の矛盾セル〈閉状態で開かない／未入力の開状態で閉じない〉で狭める・**変換中セル1つの矛盾では狭めない**・欠けセルでは狭めない・
    学習表だけでは広げない・両方不明→受動・`use_learned_keymap_table=false` では狭めない。
  - 決定10の純粋関数（MS-IME 本体同定→付ける、ATOK〈kind は MicrosoftIme・未同定〉→空、GJI→空、未検出→触らない）。
- **source-scanning ガード**（Linux）: `architecture_guard.rs` の `shadow_action` 書き込み箇所数、`bug116_...`（差し替え後のトークン）、`layer_boundary_guard`。役割の判定以外から `Toggle` を付ける経路が無いこと。
- **Windows ターゲットのコンパイル**: `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`（`runtime/` は `#[cfg(windows)]` で Linux のテストバイナリに存在しない）。
- **windows-build CI**: `ime_key_sequence_golden`（トグルで送るキー列）、`thumb_context_guard`。
- **CI 実機 E2E**（`.github/workflows/e2e-ime.yml` 系）: GJI 既定プリセットで半角/全角のトグルが従来どおり（ADR-189 の `--hz` 8手順）であることだけを確認する。
  CUSTOM 構成は CI 上で CUSTOM の `config1.db` を作る手段が無い（awase-gji-config は読み取り専用、GUI 自動化は重い）ので、ホスト上の純粋関数テストで固定する。
- **実機（ユーザー実機）**: TsfNative（Chrome・VS Code・Windows Terminal）での実タイピング確認（API の読取り値だけで判断しない）。T1 の3点。

## 未決事項（所有者への確認事項）

各項目に選択肢と推奨を添える。推奨は本 ADR 起草者の判断で、所有者決定ではない。所有者に聞くのは U1・U3〜U9。U2 は解釈の確認。

- **U1: 「IME ON の状態なら IME OFF」の「IME ON の状態」はどこまでか**（U6 とまとめて確認）
  - (a) 全ての開状態（未入力・入力中・変換中）で閉じるキーだけをトグルとする。
  - (b) 未入力の開状態（Precomposition）で閉じればトグルとし、入力中・変換中に別の作用をしてもよい（典型例は ATOK プリセットの変換/無変換）。
  - **推奨: (a)**。(b) では入力中・変換中は生キーを通し、未入力のときだけ能動にする切り替えが要り、awase の「未確定文字があるか」の推定（予測器の Stage）が新しい失敗点になる。
    初期範囲（候補は半角/全角だけ、4プリセットとも全開状態で Close）で (a)(b) の差が出るのは、CUSTOM で半角/全角を「Precomposition でだけ閉じる」ように書き換えた構成だけ。
    回答までは (a) で実装する（受動側に倒れる）。
- **U2: 決定3の範囲**（解釈の確認） — 「実行時の受動的観測は使わない」を「役割の推定に使わない」と解釈し、belief の観測追随（ADR-187/191）は存続させてよいか。
  **推奨: 存続**（観測追随を止めると、読める窓でのずれ回復手段が無くなる。役割推定へのフィードバックだけを禁止する）。
- **U3: モードを指定して開くトグル**（DirectInput→`CompositionModeFullKatakana` 等）を能動制御するとき、開いた後のモードをどうするか。
  選択肢: (a) 開閉だけ書く（モードは IME の保存値のまま＝ユーザー設定と食い違いうる）、(b) このキーは受動にする、(c) 開く方向だけ生キーを通す（二重 actuation の危険、BUG-46 型）。
  **推奨: (a)**（変換モード軸は書かない〈ADR-191〉が、所有者定義ではトグルに含まれ、開閉のずれ回復を優先する）。
  T1(c) で Windows 版 GJI が DirectInput の `CompositionMode*` で開かないと分かれば、この問いは消える。
  **T1(c) が済むまでは、所有者の例（ひらがな/カタカナのモードを指定して開くキー）は受動になる**（決定4）。
- **U4: 0x19（VK_KANJI、物理的には Alt+半角/全角）の役割をどう決めるか** — IMM32 経路では IME のキー設定に関係なく OS 側で開く（Mozc コメント）。
  TSF 経路も、Mozc のソースは 0x19 を `VK_DBE_DBCSCHAR`→`HANKAKU` に写すと書く（`tip_text_service.cc` L330-337）が、同梱表の実測では GJI の ATOK プリセットで
  変換中の 0x19 が「閉・**確定**」（`key_effect_table.rs:23`）、0xF3 は行どおり「閉・破棄」（`:18`、`atok.tsv` L29/L75 は `CancelAndIMEOff`）で、行に従っていないと推定する。
  開閉では 0x19 と 0xF3 は一致する。
  選択肢: (a) `Hankaku/Zenkaku` 行に従う、(b) ユーザーが設定で変えられないキーとして決定6-4 と同様に「既知のトグル」とし、学習表の矛盾セルでだけ狭める、(c) T1(b) まで現状維持。
  **推奨: (c) を経て (b)**。いずれでも 0x19 を `enrich_ime_relevance` の無修飾ガードに通さない。
- **U5: awase 自身の既定値 `keys.ime_toggle=["VK_KANJI"]`・`keys.ime_on=Ctrl+変換`・`keys.ime_off=Ctrl+無変換`** — いずれもユーザーが明示した設定ではない。
  GJI の CUSTOM で Ctrl+無変換 に別のコマンドを割り当てたユーザーでは、awase の既定値が打鍵を消費して IME の設定を握りつぶす（決定1に反する）。
  選択肢: (a) 既定を空にし、config.toml に明示された値だけを尊重する、(b) 既定を残す（awase 設定は別軸）、
  (c) 既定を残すが、`config1.db` 上でそのキーに開閉以外のコマンドが割り当たっているときだけ無効にする。
  **推奨: (a)**（所有者決定1に最も沿い、仕組みも単純）。依頼の要約にあった例示「既定では Ctrl+無変換・変換」（所有者の逐語発言には無い）がこの awase 既定を指すのか
  もあわせて確認する。0x19 の二重処理の経緯（`src/config.rs:495-507`）を踏まえ、`ime_toggle` は U4 と同時に変える。
- **U6: 親指キー（無変換/変換）がトグルの役割を持つ設定の場合** — NICOLA のチョードと衝突する。`transport.rs:282-291` は無変換/変換を先に `Allow` するので、
  `shadow_action` を付けても Suppress されず二重 actuation になる。
  選択肢: (a) 親指キーは候補外（受動）、(b) 単独タップと解決したときだけ能動制御（ADR-192 決定3b の KeyUp 解決の合流点を使う）、(c) 親指キーでも能動制御し、チョードは諦める。
  **推奨: (a) を初期範囲とし、(b) は別 ADR**。MS-IME の `check_and_warn` は (a) の間は据え置き、原則との矛盾は status に記録する。
- **U7（事後確認）: MS-IME 本体のレジストリを所有者決定3の「`config1.db`」に含めてよいか** — レジストリは既に予測（`key_pipeline.rs:1960-1965` →
  `msime_key_assignment.rs:249` `read_key_effect_keymap_native`）と能動（`sync_ime_toggle_auto_detect`）の両方で使われている。レジストリは MS-IME にとっての
  `config1.db`（ユーザーが決めた設定そのもの）で、実行時観測ではない。
  **推奨: 含めてよい**（使い方は現状維持。ただし MS-IME 本体と同定できたときだけに締める、決定10）。含めない場合は、この2経路を撤去する別件になる。
- **U8: MS-IME 互換モード（以前のバージョン）の半角/全角** — 詳細キー設定で半角/全角にも任意の機能を割り当てられるが、awase はその割り当てを読めない
  （ADR-197 はコード `CE` の信号が実挙動と相関しないと確認し、検出を撤回した）。互換モードかどうかはレジストリ `NoTsf3Override2` で読める（ADR-197 決定4）。
  選択肢: (a) 互換モードのときは半角/全角を受動にする（決定6-3「不明なときは受動」と一致。既定のままの互換モードユーザーでは TsfNative の開閉のずれが直らなくなる）、
  (b) 現状どおり能動（トグル）にし、学習表の矛盾セルでだけ狭める（決定1の例外として明記が要る）。
  実装はどちらも数行。**推奨: (a)**（(b) は設定を知らないまま書く唯一の経路になる）。回答までの暫定は現状どおり（(b) 相当）。
- **U9: F13〜F24 を初期範囲の候補に加えるか** — 所有者発言の後半「他のキーを IME ON/OFF トグルに設定したら awase が能動的に制御します」の初期範囲での実体は
  F13〜F24（親指キー〈U6〉・かな/カタカナ・英数〈決定4〉は初期範囲外）。
  - (a) 加える。費用の要点（詳細は round4/5 のメモ）:
    - ソフトウェアのリマッパーが出す F キーは `LLKHF_INJECTED` 付きで `transport.rs:239-245` で必ず Allow になり効かない（効くのは `Scancode Map`・ファームウェアのリマップだけ）。
    - F13〜F24 は `should_upgrade_is_japanese_ime`（`vk.rs:336`）で `is_japanese_ime()` が真に上がらず、grace 期間に Suppress すると二重の空振りになる。避けるには Down/Up 非対称を扱うラッチ（BUG-131/132 系）が要る。
    - 候補集合の入口（`ime_kind()` の早期 return を通らないので enrich の入口）の変更と、F キーの Suppress の配送影響調査が要る。
  - (b) 加えない。初期範囲は半角/全角（0xF3/0xF4）だけ、F13〜F24 は別 ADR（T4 は `is_open_toggle_for` を役割の参照に置き換えるだけで、ラッチも配送の影響調査も不要）。
  - **推奨: (b)**（判定式・情報源・縮退の規則は (a)(b) で共通で、決定1の主目的〈CUSTOM で半角/全角を別機能にしたユーザーの受動化〉は (b) でも達成される）。
    ただし (b) では所有者発言の後半が初期範囲では実現しない。**所有者がこれを初期範囲で必須とするなら (a)＋ラッチ**（設計は別 round で詰める）。
    二択は起草者が決めず所有者に確認する。本文は (b) を前提に書き、(a) の場合の追加分は本項とテスト方針に置いた。

## レビュー反映メモ

各ラウンドの指摘全文は scratchpad の `opus-adr199-round{1..6}.md`（リポジトリ外）。反映した指摘は本文に取り込み済みなので、ここには
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
| r4 R1（F13〜F24 の二重の空振り） | 形を変えて反映 | 修正案 (ii)（ラッチ）は仕組みを足す方向なので採らず、U9 の推奨を (b) に変更。二択は所有者確認 |
| r4 R2（OVERLAY も Mozc は既定に倒す） | 一部訂正 | 既定に倒れるのは `OVERLAY_FOR_TEST` だけで、`OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF` は overlay TSV だけが読まれる。いずれも「未知の値は受動」で結果が一致するので個別規則は足さない |
| r4 R3（役割表を保持して読み直し時に作り直す） | 撤回（r5） | 保持をやめ、打鍵時に求める形へ（代替案 J） |
| r5 S1（ATOK 切り替え後に古い Toggle が残る） | 反映 | S3 の形で構造的に解消（打鍵時の `table_ime_kind()` で分岐、`None` は受動）。回帰テストを追加 |
| r5 S2（共有キャッシュの型の変更が隠れている） | 形を変えて反映 | 3値の戻り値にせず、不在のとき `from_config(None, None, &[])` を返す＋生の `session_keymap` を1フィールド足す、の2点に縮めた。副作用（不在で予測も MS-IME プリセットで動く）は決定8に明記 |
| r5 S6（`kp_run_inner` の行番号） | 非反映 | 指摘が誤り。`e3969f40` で `kp_run_inner` の宣言は `key_pipeline.rs:264`、`enrich_ime_relevance` の呼び出しは `:265`（grep で確認、r6 も確認済み） |
| r6 N3（`custom_table_overrides` のプリセット差分化） | 範囲外へ移動 | 役割判定と独立した予測の既存バグで、正しい修正も「`session_keymap != CUSTOM` ならカスタム表を見ない」。T6・影響表・複雑性節から外しリスク節に1行記録 |
| r5 冗長2（決定10を別 BUG に分ける） | 部分反映 | BUG ファイルの起票は本作業の編集範囲外。決定10を圧縮し、別の fix PR として先行してよいと明記 |
| r5 冗長3（U1 を所有者への質問から外す） | 非反映 | U1 は所有者定義の解釈で起草者が確定しない。初期範囲で差が出る構成が狭いことと、回答までは (a) で実装することを U1 に明記した |
