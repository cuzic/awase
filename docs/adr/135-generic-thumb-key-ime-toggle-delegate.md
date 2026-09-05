# ADR-135: 親指キー単独タップのIME ON/OFF/トグル意味論への汎用対応（BUG-115）

## ステータス

**設計フェーズ完了。Phase 1・Phase 2・Phase 3のいずれもOpus敵対的
レビューで収束済み、Phase 1〜3ともブランチ
`feat/bug115-gji-hiragana-katakana-ime-mode-keys`に実装・コミット済み
（2026-09-05）。`cargo test`（workspace全体1664件）・
`cargo nextest run -p awase-windows --test architecture_guard`
（67件）・`cargo clippy`・`cargo fmt --check`いずれもクリーン。**

**実装後のOpus敵対的コードレビュー1ラウンド目で新規ブロッカーC1と
既存バグC2を発見、いずれも対応済み**（下記「実装レビューで発覚した
C1」「C2」各節参照）:
- **C1（ブロッカー、修正済み）**: Phase 2/3の排他制御ゲート条件に
  「エンジン活性（belief ON）」が抜けており、belief OFF状態で
  Hiragana/Katakana親指キーによるIME復帰が固着しうるバグを発見。
  ゲート条件を「親指キー×delegate armed×belief ON」の3項の積に修正。
- **C2（既存の欠陥、Phase 1以前から存在、記録のみ・別issue化）**:
  実機検証により、既存のHenkan/Muhenkan delegate-to-open-axisの
  `TurnOn`方向（OFF→ON復帰）が活性ゲートの制約で構造的に発火できず、
  実際のON復帰は受動的な観測フォールバックに依存していたことが判明。
  観測できないアプリ（Imm32Unavailable、BUG-115の報告環境）では
  ON復帰手段が実質存在しない可能性がある。本PRのスコープ外として
  別issueを起票する。

**2ラウンド目のOpus敵対的コードレビューでC1修正を検証、D1（記述の
正確化のみ、設計変更なし——このゲートが実際に抑止しているのは
「belief的にno-opな書き込み」のみで、P1は防いでおらずchord打鍵ごとの
spurious intent記録を防ぐ機構であることの明記）を反映し、
「以上で全指摘、収束」を受領。Opus敵対的コードレビュー完了。**
残タスク: PR作成 → `/codex-review` → CI green確認 → developへマージ。

- **Phase 1**（config1.db修正〜設定UI修正）: 実装済み・Opus敵対的
  レビュー（コード差分2ラウンド・ランタイム設計2ラウンド・
  ホリスティック設計1ラウンド）収束済み。ホリスティックレビューで
  Phase 1自身のHiragana/Katakana暫定対応（actuation-autoへの反映処理）
  に二重actuationバグが発覚し、当該コードは撤去済み（「Phase 1の
  訂正」節参照）。
- **Phase 2**（`shadow_action`をGJI検出値でオーバーライド、対象は
  `VK_DBE_HIRAGANA`/`VK_DBE_KATAKANA`の2キー）: 当初v1
  （`nicola_fsm`の左右親指スロット汎用化）はOpus敵対的レビューで
  前提誤りが判明し撤回（「Phase 2の撤回」節）。再設計版はOpus敵対的
  設計レビュー3ラウンドで収束（ブロッカーB1「親指キーの場合に毎打鍵
  IME OFF/反転を作る」への対応として「オーバーライドは親指キーで
  ない場合にのみ適用する」条件を追加、他R1〜R3/M1〜M10反映済み）。
  **ただしPhase 2単独ではBUG-115の元シナリオ（Hiragana/Katakanaを
  親指キーにしている場合）を救えない**（「Phase 2単独の限界」節）。
- **Phase 3**（Henkan/Muhenkanの既存`delegate-to-open-axis`をHiragana/
  Katakanaへ拡張し、Phase 2単独の限界を埋める）: ユーザー判断
  （2026-09-05、「Phase 2とPhase 3を同時にやる方が効率的」）により
  新設し、本ADRへ統合。Opus敵対的設計レビュー3ラウンドで収束
  （「以上で全指摘、収束」）。1ラウンド目でブロッカーP1（Phase 2の
  静的`TurnOn`とPhase 3のdelegateが同一物理打鍵で衝突し、撤回した
  v1と同型の二重actuation/no-opを起こす）、2ラウンド目でブロッカー
  Q4（P1対策のゲート条件が甘く、非親指キーのPhase 2実効ケースまで
  殺してしまう——正しくは「親指キー×delegate armed」の積で判定する）
  が発覚し、いずれも解消済み。他must-fix（P2: コアにVK定数を書く
  ADR-019違反、P3: Passthrough→Consumeの挙動変化でInputRelay
  プロファイルのキーが機能しなくなる既知の限界、Q1: ゲートは
  `intent_kind`の`PhysicalImeKey`分岐条件に限定し`sync_direction`
  経路を守る、Q2: ゲートがEisu救済・半角英数復帰の副作用を巻き添えに
  しない設計、`.claude/rules/ime-belief-architecture.md`が要求する
  経路×救済対応表の更新込み）も全て反映済み。3ラウンド目は実装時の
  精緻化のみ（S1〜S6、設計変更なし）。

詳細な経緯（「Phase 2の静的`shadow_action`firingとの衝突と、その解消」
「Phase 3のリスク評価」等の各節）を参照。

**【2026-09-05、実機検証完了】** 設計フェーズで残っていた「未検証事項」
のうち、実装着手前に確認すべき項目（`[shadow-toggle] intent 昇格`ログの
実機出力、BUG-52非再発）を、プロジェクトオーナー機（dragonflyg4、
`develop` HEAD、Phase 2/3実装前のコード）で確認済み。物理ひらがな/
カタカナキー押下で期待どおり`kind=PhysicalImeKey`・`injected=false`の
ログが出力され、beliefがOFF→ONへ正しく遷移することを確認。BUG-52
（カタカナKeyDownのSuppress）も文字漏れ無しで確認。副次的に、Shift+
かなキーがカタカナではなくひらがなモードになる別件の現象を発見したが、
これはshadow-toggle機構（IME開閉軸のみ扱う）の対象外であり、Phase 2/3
のいずれの設計判断にも影響しない（「未検証事項」節参照）。
**設計・検証とも完了、実装着手可能な状態。**

**このADRは元々「単一のprotobufパースバグ修正」として始まり、
NICOLAコア(`src/engine/nicola_fsm.rs`)の親指キー抽象化そのものの
見直し（v1、撤回済み）→ 既存shadow-toggle機構へのオーバーライド方式
（Phase 2）→ Phase 2単独の限界を埋めるためのdelegate-to-open-axis
拡張（Phase 3）という経緯で、当初の見積りより大幅にスコープが
拡大している。経緯を本ADRに集約する。**

## 背景

### 発端: 不具合報告 `01M1R1T7VCB5K5DDM3Q00DCKXY`（2026-09-05）

GJI（Google 日本語入力）をUWPアプリで使用中、日本語をOFFにした後「変換」
キーでON復帰すると、日本語入力にはなる（音になる）が親指シフト変換には
ならない。「ひらがな/カタカナ」キーでON復帰すると正しく親指シフトになる。
報告者は「ひらがな/カタカナもCapsLockもGJIでは設定していないはず」と
述べていたが、実際にはawase側の`config.toml`で`engine_on =
["VK_DBE_HIRAGANA"]`が設定されており、GJI側の設定ではなくawase側の
`engine_on`（能動的にIMEを開き直すactuation）がこのキーだけを救っていた。
「変換」キーはawaseのどのactuation/probe設定にも登録されておらず、GJI
自身が独自にIMEをONにしてもawaseのbelief（`compute_state`が参照する
`ctx.ime_on`）が追随しない、というのが症状の直接の機序
（`src/engine/engine.rs:239-253`、`ImeOff`ゲートで非活性）。

### 調査の展開

1. `crates/awase-gji-config`（GJIの`config1.db`——protobufバイナリ——を
   読むクレート）に`session_keymap`のフィールド番号誤り（22と実装されて
   いたが、`google/mozc`本家`config.proto`を実際に取得すると41が正しい。
   22は無関係の`check_default`）を発見。実機`config1.db`（clipwire経由）
   で`field 22=1, field 41=0(CUSTOM)`を確認し実証。
2. `overlay_keymaps`（field 68、`session_keymap`とは独立）という、
   `OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`を含めると無変換→IMEOff・
   変換→IMEOnを状態非依存で重ね掛けする機構の存在も発見。
   `awase-gji-config`は未対応だった。
3. Step4b（ADR-092決定D、無変換/変換delegate-to-open-axis）はMS-IME
   レジストリ向けにのみ配線されており、GJI向けに対称配線されていな
   かった。ATOKプリセットは`DirectInput`でHenkan/Muhenkan→IMEOn、
   `Precomposition`で→`CancelAndIMEOff`という状態依存の反転を持つが、
   `ShadowImeAction::Toggle`（`!ctx.ime_on`）で正確に表現できることが
   判明（「表現不能」ではない、プロダクト判断でopt-in化）。
4. ユーザー指摘により、無変換/変換以外の親指キー（かなキー等）でも
   同型の問題が起きうると判明。Mozc本家`ms-ime.tsv`/`mobile.tsv`
   （Windows版GJIの実質デフォルトプリセット）は`DirectInput`で
   Hiragana/Katakana双方を`IMEOn`に割り当てており、**ATOKの無変換/
   変換より発生条件が緩い**（デフォルトプリセットのままでも該当）。
5. さらにユーザー指摘（本ADR起票のきっかけ）: 「ひらがなは単なる例示。
   変換、無変換以外のすべてのキーで、親指キーかつIME ON/OFFに設定して
   いることを考慮せよ」。Mozc本家tsvを全数調査した結果、`Eisu`（英数）・
   `Kanji`/`Hankaku-Zenkaku`（漢字キー、同一VK）・`ON`/`OFF`
   （`VK_IME_ON`/`VK_IME_OFF`、合成専用）も含め、Mozcのキーマップ
   システムが認識する**全キー語彙**（`awase-gji-config::keymap::
   MOZC_KEY_ALIASES`と同一集合）が対象になりうることを確認した。

### アーキテクチャ上の制約（Phase 2の起点、v1当時の認識——後述のPhase 2/Phase 3で一部訂正）

**（2026-09-05訂正）以下はv1（撤回済み）を起票した時点の認識であり、
「単純なフィールド追加では済まない」という結論は誤りだったことが後に
判明した。実際には対象を「Mozcが認識する全キー」ではなくHiragana/
Katakanaの2キーに絞れば、既存の`resolve_pending_thumb_as_single`の
分岐ロジック自体は書き換えず、専用フィールド2つを並列に追加するだけで
済む（下記「Phase 3」参照）。誤りだった理由は、当時「全キーを汎用的に
扱う」ことを前提にしていたため——特定2キーだけへの決め打ち拡張という
選択肢を検討していなかった。以下は歴史的経緯として残す。**

Step4b delegate-to-open-axis機構（`henkan_delegate_to_open_axis`/
`muhenkan_delegate_to_open_axis`）は、`src/engine/nicola_fsm.rs::
resolve_pending_thumb_as_single`内で`self.muhenkan_vk == Some(vk_code)`/
`self.henkan_vk == Some(vk_code)`という**専用フィールド2つへの決め打ち**
でしか参照されない。この2つの物理キー（`VK_NONCONVERT`/`VK_CONVERT`）
以外を親指シフトキーに設定した場合、`mode_key_config`/`dedicated_fn_key`/
`delegate_to_open_axis`の判定は全てスキップされ、「composing中はSuppress・
非composing中はPassthrough」という一律の挙動に落ちる
（`nicola_fsm.rs:1857-1984`で確認済み）。つまりHiragana/Katakana/Eisu/
Kanji等には、delegate-to-open-axis相当の安全な自動反映手段が構造的に
存在しない。単純なフィールド追加では済まず、`resolve_pending_thumb_as_
single`の分岐ロジック自体の書き換えが必要（Opus調査で確認）——
**という結論は「全キーを汎用的に扱う」場合の話であり、2キーへの決め打ち
拡張には当てはまらない（上記訂正参照）。**

NICOLAは本質的に「左親指キー1つ・右親指キー1つ」の2スロットシステムで
あり（無変換/変換は単なる**慣例的な既定割当て**にすぎない）、
`NicolaFsm::new()`には元々`_left_thumb_vk`/`_right_thumb_vk`という
未使用（`_`プレフィックスで握りつぶされている）パラメータが存在した。
これは元の設計が「左右スロット」という汎用概念を見据えていたが配線
されずに終わっていたことを示唆する。

## 決定

### Phase 1（実装済み）

`crates/awase-windows/src/gji_charset_autodetect.rs`に、`config1.db`の
3つの情報源（`overlay_keymaps`・`session_keymap==CUSTOM`の
`custom_keymap_table`内literalトークン・プリセット静的知識）を優先順位
付きで1つの結論（`ImeToggleKind::{On, Off, Toggle}`）にまとめる純粋関数
群（`classify_mode_key_ime_action`、Linux上でもテスト可能）を実装した。

- **プリセット静的知識**は`google/mozc`の`src/data/keymap/{ms-ime,atok,
  mobile,kotoeri,chromeos}.tsv`を実際に取得して確認した実データに基づく
  （日付: 2026-09-05）。
- **`Toggle`（状態依存で非冪等）のみ**、新設した`GeneralConfig::
  gji_thumb_key_ime_toggle`（既定`false`）のopt-inゲート対象。`On`/`Off`
  は情報源を問わず常に安全（冪等）なので無条件で反映する。
- 親指キーとして設定されている無変換/変換は既存のdelegate-to-open-axis
  （Step4b）へ、親指キーでない場合はStep4cと同じ`ime_on_auto`/
  `ime_off_auto`/`ime_toggle_auto`（actuation-auto）へ振り分ける
  （BUG-14の4エイリアス——`Kanji`/`Hankaku-Zenkaku`/`ON`/`OFF`/`Eisu`
  ——はactuation-autoには従来通り一切乗せない。この除外は本ADRでも
  維持する、下記Phase 2参照）。
- ~~Hiragana/Katakanaが親指キーとして設定されている場合は、
  delegate-to-open-axisに対応する専用フィールドが無いため、暫定的に
  自動反映せず警告のみ（「無理せず」の方針、ATOKの`Toggle`既定OFFと
  同じ判断軸）。~~ **2026-09-05、Phase 1ホリスティックレビューで撤去
  （下記「Phase 1の訂正」参照）。** Hiragana/Katakanaを親指キーとして
  設定しているかに関わらず、GJI由来のIME意味論をactuation-autoへ
  載せる処理そのものを削除した。
- 設定UIに`gji_thumb_key_ime_toggle`のチェックボックスを追加
  （「上級者向け設定」タブ）。
- 副次的に発見・修正した別件: 設定UI（`awase-settings.exe`）が
  `keys.ime_detect`（GUIに編集ウィジェットが無いフィールド）を、設定
  画面を開いたまま外部エディタで編集すると「適用」のたびに古い
  スナップショットで上書きしてしまうstale read-modify-writeバグ。
  保存直前にディスクから最新値を拾い直す修正を実施。

### Phase 1の訂正（ホリスティックレビューで発覚した二重actuation、2026-09-05）

Phase 2 v1（`nicola_fsm`汎用化案）を撤回した後、「実装済みのPhase 1も
同じ観点でOpusに俯瞰的・敵対的にレビューしてほしい」という指示のもと
再レビューしたところ、**既にコミット前の作業ツリーに実装されていた
Phase 1自身が、撤回したPhase 2 v1と全く同型の欠陥を持っていた**ことが
判明した。

`windows_impl::sync_gji_charset_autodetect`のHiragana/Katakanaループ
（`classify_mode_key_ime_action`の結果を、親指キーでなければ`ime_on_auto`/
`ime_off_auto`/`ime_toggle_auto`へ直接pushしていた部分）は、
`VK_DBE_HIRAGANA`/`VK_DBE_KATAKANA`が`vk.rs::ImeKeyKind::from_vk`の
静的マップで既に固定の`shadow_action`（`TurnOn`）を持ち、
`kp_stage_shadow_ime_toggle`が毎打鍵belief更新とactuationの両方を
行っていることを見落としていた。同一打鍵で`kp_stage_shadow_ime_toggle`と
`Engine::match_ime_on_off_auto`→`ime_set_open_effects`が両方発火する
（後者は状態が変化しなくても`Effect::Ime(SetOpen)`を無条件に積む、
`engine.rs:810-829`）。

**Phase 2 v1のレビューとPhase 1のレビューで、同じ盲点（shadow-toggle
機構の存在）を見落としていた点が重要**——コード差分レベルのレビューでは
「このループは単体で見て正しいか」しか検証できず、「他の既存機構との
相互作用」は俯瞰的な設計レビューでなければ発見できなかった。影響範囲は
ATOKの特殊ケースに限らず、config1.dbが読めない場合のfail-open
フォールバック込みで、**MS-IME/MOBILEプリセット（GJIの実質既定）で
Hiragana/Katakanaを親指キーに設定していないGJIユーザーほぼ全員**に
該当していた（`On`はopt-inゲートを素通りするため）。

**対応**: `gji_charset_autodetect.rs`からこのループと専用の警告関数
（`warn_mode_key_thumb_key_unsupported_if_needed`）・デデュープラッチ
（`LAST_MODE_KEY_THUMB_WARNING`）を完全に削除した。無変換/変換の
delegate-to-open-axis/actuation-auto配線は対象外のキー（`VK_CONVERT`/
`VK_NONCONVERT`は`ImeKeyKind::from_vk`に含まれない）なので変更していない。
`classify_mode_key_ime_action`関数自体とそのHiragana/Katakana向け回帰
テストは、下記「Phase 2（再設計）」実装時に再利用する前提でそのまま
残した（呼び出し元だけ削除）。詳細は
[docs/known-bugs.md BUG-115](../known-bugs.md)参照。

### Phase 2の撤回（当初設計v1、2026-09-05）

**当初設計した「`nicola_fsm`の左右親指スロット汎用化」は、Opus敵対的
設計レビューで前提そのものが誤りと判明し撤回した。** 以下、撤回の経緯を
記録する（`experiment-logging.md`と同じ理由——なぜ捨てたかを残さないと、
次のセッションが同じ設計を再発見して再導入してしまう）。

**v1の主張（誤り）**: 「Hiragana/Katakana/Eisu/Kanji等にはdelegate-to-
open-axis相当の安全な自動反映手段が構造的に存在しない」（当初の
「アーキテクチャ上の制約」節、および`gji_charset_autodetect.rs`の
`ModeKeyCandidate`/`has_delegate_to_open_axis_support`のdocコメントに
同じ主張があった）。

**実際（Opusレビューで発見、`crates/awase-windows/src/vk.rs:82-151`・
`hook.rs:127-138`・`runtime/key_pipeline.rs:823-925`で実装を確認して
裏付け済み）**: これらのキーには**既に**別の追従機構がある。
`ImeKeyKind::from_vk`が`VK_KANA(0x15)`/`VK_IME_ON(0x16)`/
`VK_JUNJA(0x17)`/`VK_KANJI(0x19)`/`VK_IME_OFF(0x1A)`/
`VK_DBE_ALPHANUMERIC(0xF0)`/`VK_DBE_KATAKANA(0xF1)`/
`VK_DBE_HIRAGANA(0xF2)`/`VK_DBE_SBCSCHAR(0xF3)`/`VK_DBE_DBCSCHAR(0xF4)`
を認識し、`hook.rs`が`shadow_action`（`ShadowImeAction`——delegate-to-
open-axisと同じ型）を付与、`key_pipeline.rs::kp_stage_shadow_ime_toggle`
が物理（非注入）キー押下をIME意図（`IntentKind::PhysicalImeKey`）へ
昇格させてbelief追従＋actuationまで行う。**delegate-to-open-axisが
Step4bで必要だったのは、この静的マップに`VK_CONVERT(0x1C)`/
`VK_NONCONVERT(0x1D)`（無変換/変換）だけが含まれていないから**であり、
存在条件は「親指キーかどうか」ではなく「そのVKが`shadow_action`を
持たないかどうか」だった。

v1をそのまま実装すると、`shadow_action`を既に持つキーに対して
`kp_stage_shadow_ime_toggle`（既存経路）と`resolve_pending_thumb_as_
single`のdelegate-to-open-axis（v1の新規経路）が**同一の物理打鍵に対して
二重に発火**する。`VK_KANJI`（Toggle）では二重トグル＝実質no-op、
`VK_DBE_HIRAGANA`（TurnOn）でも actuation の generation 競合窓を作る。
これは`fix-requires-evidence.md`が記録するissue #136の自己回帰
（1箇所にgateを置いて満足し、別経路を素通しさせた）と**同型の事故を
逆向きに作るもの**だった。

v1のその他の欠陥（Opus指摘、実装しないため詳細は簡潔に記録）:

- **BUG-14との相互作用の見落とし**: v1は「delegateは別コードパスだから
  BUG-14の`event.injected`非チェックとは無関係」と主張したが誤り。
  `key_pipeline.rs::on_input`にはinjectedの門番が無く、注入された
  `VK_DBE_HIRAGANA`等（BUG-14で実機確認済みの注入パターン）は親指キー
  判定を経て`PendingThumb`に到達しうる。既存のHenkan/Muhenkan delegateが
  安全なのは「同じ経路を使っているから」ではなく「対象VKが
  `VK_CONVERT`/`VK_NONCONVERT`——MS-IME/CTFが注入しないキー——だから」。
- **走査対象8キーのうち実際に到達しうるのは3キーのみ**: `VK_IME_ON`/
  `VK_IME_OFF`は合成専用（物理押下では届かない）。`VK_KANJI`は
  ADR-133の実機調査により、物理の半角/全角キーは実際には
  `VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`として届くため、親指キーVKとの
  比較が永久に不一致になる死んだコード。実質意味があるのは英数/
  ひらがな/カタカナの3つだけで、それらは前述の通りshadow-toggleが
  既にカバー済み。
- **stale delegateによるSpace/Enter乗っ取りのリスク**: delegateを
  「スロット位置」に紐づけると、config reload後にIME種別が未検出の
  ままだと再計算が走らず、スロットのVKが変わった（例:
  右親指をVK_SPACEに変更）後も古いdelegateが残留し、Space単独打鍵を
  誤ってSetOpenとして飲み込みうる。対策（VKとactionを同梱する）は
  あるが、v1のフィールド設計はこれを踏んでいなかった。
- **bootstrap.rsへの配線漏れ**: `NicolaFsm::new()`の`_left_thumb_vk`/
  `_right_thumb_vk`は未使用のまま、`app/bootstrap.rs`側の初期化経路にも
  新フィールドへの配線が設計に含まれていなかった。
- **挙動変化の見落とし**: 現状Hiragana親指キーの非composing単独タップは
  OSへPassthroughされるが、delegateを付けるとSuppress（飲み込み）に
  変わる。`transport.rs::plan`は独立に物理キーをOSへ配送しうるため、
  「FSMが飲み込むから二重にならない」という前提も成立しない。

## Phase 2（再設計）: `shadow_action`をGJI検出値でオーバーライドする

正しい修正対象は`nicola_fsm.rs`ではなく、既存のshadow-toggle機構
（`ImeKeyKind::from_vk`の**静的**VK→action対応表）である。この対応表は
「ひらがなキー＝常にTurnOn」のように固定されており、GJIの実際の設定
（`config1.db`のCUSTOM literalトークンでOff/Toggleかもしれない、
ATOKプリセットではそもそも該当なしかもしれない）を反映できない。
Phase 1が実装した`classify_mode_key_ime_action`の出力を、この対応表への
オーバーライドとして流し込む。

### スコープ確定: 対象はHiragana/Katakanaの2キーのみ（2026-09-05、設計具体化時に確定）

「Phase 2の撤回」節が引用した「実質意味があるのは英数/ひらがな/カタカナの
3つだけ」という結論を、実装着手前にさらに絞り込む。`google/mozc`の
`src/data/keymap/{ms-ime,atok,mobile,kotoeri,chromeos}.tsv`を実際に
再取得し（2026-09-05、`gh api repos/google/mozc/contents/src/data/keymap/*.tsv`）、
`DirectInput`状態の全行を確認した結果は以下の通り:

```
ms-ime.tsv / mobile.tsv: DirectInput {Eisu,Hankaku/Zenkaku,Henkan,Hiragana,Kanji,Katakana,ON} → 各種
atok.tsv:                DirectInput {Hankaku/Zenkaku,Henkan,Kanji,Muhenkan,ON} → 各種（Hiragana/Katakana行なし）
kotoeri.tsv:              DirectInput {Hankaku/Zenkaku,Kanji,ON} → 各種（Hiragana/Katakana行なし）
chromeos.tsv:              DirectInputセクション自体が無い
```

**Eisu（`VK_DBE_ALPHANUMERIC`）はスコープ外とする。** ms-ime.tsv/mobile.tsv
に`DirectInput Eisu IMEOn`という行が実在し、これは「EisuキーはIME OFF
（DirectInput）から押すとIMEをONにする」ことを意味する——awaseの静的
`ImeKeyKind::Alphanumeric → ShadowImeEffect::TurnOff`（`vk.rs`）とは
**逆方向**であり、一見この静的値こそ誤りでオーバーライドが必要に見える。
しかし`.claude/rules/experiment-logging.md`が記録する通り、
`VK_DBE_ALPHANUMERIC`は本プロジェクトで**複数回**IME OFFキー（actuation
送信対象）として採用・撤回されており、その都度「これは半角英数（IME ON）
であって直接入力ではない」という同じ事実が再発見されている
（`534051a3`→`098c6633`、`9c3f11e2`→`668a131a`）。これは「Mozcの
composition内部状態としてはON、しかしawase側のengine/UI観点では
OFF相当として扱いたい」という、単純なON/OFF二値に収まらない既知の
未解決の緊張関係であり、**shadow_action（IME open belief）とengineの
own state machineという別の2つの概念が絡む**。今回のPhase 2は
「GJIの検出値をshadow_actionへ機械的に反映する」という限定されたスコープ
であり、この緊張関係を解く場ではない。Eisuを含めると、過去に複数回
振り出しに戻った論点を検証不十分なまま作り込むことになるため、
明示的にスコープ外とする（将来Eisuを扱うなら、この緊張関係の解消を
独立したADRとして先に決着させること）。

**Kanji/Hankaku-Zenkaku（`VK_KANJI`）はスコープ外とする。** 「Phase 2の
撤回」節で既に確認済みの通り、ADR-133の実機調査により物理の半角/全角キーは
実際には`VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`として届き、`VK_KANJI`
（0x19）は事実上注入専用（BUG-14の`event.injected`早期returnで弾かれる）。
オーバーライドを`VK_KANJI`に書いても物理打鍵では絶対に参照されない
死んだエントリになる。仮に`VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`へ
オーバーライド対象を付け替えるとしても、この2つのVKは「半角方向への遷移で
SBCSCHAR、全角方向への遷移でDBCSCHAR」とOS側が遷移方向ごとに別のVKを
報告する形で**既にトグルを表現しており**（静的`shadow_action`が
`Deactivate→TurnOff`/`ActivatePair→TurnOn`と定義済み）、GJIの`Kanji`
トークンに対する単一の`ImeToggleKind`判定値ではこの方向依存を表現
できない。この2VKは対象外のまま、既存の静的定義を維持する。

**ON/OFF（`VK_IME_ON`/`VK_IME_OFF`）はスコープ外とする。** 物理キーボードに
存在しない合成専用VKで、物理押下では届かない（「Phase 2の撤回」節で
確認済み）。オーバーライドしても`kp_stage_shadow_ime_toggle`の
`event.injected`早期returnで常に弾かれ、意味を持たない。

**結論: Phase 2のオーバーライド対象は`VK_DBE_HIRAGANA`(0xF2)/
`VK_DBE_KATAKANA`(0xF1)の2つに限定する。** いずれもPhase 1で実装済みの
`classify_mode_key_ime_action`が既に正しく分類できている
（ATOKプリセット→`None`、ms-ime/mobileプリセット→`Some(On)`、
CUSTOM literalトークン/overlay→3ソース優先順位判定、いずれも回帰
テスト済み）。新規のMozcデータ調査や新しい分類ロジックは不要で、
Phase 1の消費先を差し替えるだけで済む。

### Phase 2単独の限界: 親指キーの場合は救わない（Opus敵対的レビューB1で発覚、Phase 3で解決、2026-09-05）

**Hiragana/Katakanaが親指シフトのチョードキーとして設定されている構成
では、Phase 2のオーバーライドを`Off`/`Toggle`に適用してはならない。**
`kp_stage_shadow_ime_toggle`（`key_pipeline.rs:823-826`）は単独タップ
確定を待つdelegate-to-open-axis（`resolve_pending_thumb_as_single`）とは
根本的に性質が異なり、**`KeyDown`であれば無条件で毎回発火する**
（チョード判定・単独タップ確定等のゲートが一切無い）。現状これが無害
なのは静的値が`TurnOn`固定で、かな入力中はbeliefが既にopenのため毎回
no-opに落ちるからにすぎない——「shadow-toggleだから安全」ではなく
「値がTurnOnだから安全」。

Phase 2のオーバーライドがこの値を`Off`/`Toggle`に書き換えると、
NICOLAのチョード打鍵（ひらがなキーを親指として使う同時打鍵）のたびに
`shadow_action`が発火し、**チョード1回ごとにIME OFFが再アサートされる
（`Off`の場合、opt-inゲート対象外なので既定設定でも到達）か、IMEが
反転する（`Toggle`+opt-inの場合）**。後者は本ADR自身が「却下した
代替案」に挙げているADR-092:1136-1145の「毎打鍵ごとに反転する」致命的
回帰そのものであり、書き込み先が違うだけで同じ暴走を再現する。到達性は
机上の空論ではない——`awase-gji-config/src/keymap.rs`の
`classify_and_push`はCUSTOMキーマップから`off`/`toggle`を実際に生成でき、
かつBUG-115の記録どおり**プロジェクトオーナー自身の実機が
`session_keymap=CUSTOM`**である。

**したがって設計を修正する: オーバーライドは「そのVKが現在の親指キーで
ない場合」にのみ適用する**（`gji_charset_autodetect.rs`の
`is_configured_thumb_key`を再利用）。親指キーとして設定されている場合は
オーバーライドを適用せず、静的な`TurnOn`のまま——GJI側の実際の割当てが
`On`ならたまたま正しく動作し、`Off`/`Toggle`なら追従できない（警告のみ）。

**この帰結として、shadow_actionオーバーライド（以下Phase 2と呼ぶ）が
単独で救えるのは「Hiragana/Katakanaを親指キーにしていない、GJI側の
割当てが静的マップと食い違う場合」だけであり、BUG-115の元の報告
シナリオ（ひらがな/カタカナを親指キーにしているエッジケース）は
Phase 2単独では未解決のまま残る。** これはPhase 1が「非親指キーは
actuation-autoへ、親指キーは警告のみ」と振り分けていた判断のうち、
**振り分けの判断軸自体（親指キーは警告のみに留める）は結果的に
正しかった**ことを意味する。誤っていたのは非親指キー側の実装方法
（既存のshadow-toggleと二重発火するactuation-autoへ載せたこと）だけ
であり、Phase 2はその非親指キー側だけを正しい実装（shadow_action
オーバーレイ）に置き換えるものである。

**2026-09-05、ユーザー指摘を受け設計を統合: 親指キーのケースは
「警告止まり」で終わらせず、Henkan/Muhenkanが既に持つ
`delegate-to-open-axis`（チョード確定ロジックを経由する、単独タップの
ときだけ発火する既存の安全な機構）をHiragana/Katakanaへも拡張すること
で正面から解決する。** これを新たにPhase 3として設計し、Phase 2と
同時にレビュー・実装する（下記「Phase 3」節参照）。Phase 2（非親指
キー）とPhase 3（親指キー）は、`is_configured_thumb_key`の真偽で
互いに排他的に担当領域が分かれる**同一問題の2つの半分**であり、
別々の時期に分けて実装する理由が薄い（同じ`classify_mode_key_ime_action`
の出力を消費先だけ変えて両方に配線するため、設計・レビューの重複が
大きい）という判断で統合した。

### 判断軸の訂正: 「On/Offは冪等だから常に安全」はPhase 2にそのまま持ち込めない（Opus敵対的レビューB2）

Phase 1の判断軸（`gate_thumb_key_ime_actions`、`Toggle`のみopt-in、
`On`/`Off`は情報源を問わず常に安全）は、**delegate-to-open-axisという
単独タップ確定経路にしか呼ばれない機構**を前提に成立していた——
`resolve_pending_thumb_as_single`は「単独タップと確定したとき」にしか
呼ばれないため、`Off`は「ユーザーが意図的に単独で押した」ときにだけ
発火する。

shadow-toggleは毎`KeyDown`で発火する。この文脈では「1回の発火が冪等
かどうか」ではなく「**そのVKがNICOLAのチョード判定に参加するか
（＝毎打鍵発火してよい値かどうか）**」が安全性の基準になる。上記の
「親指キーでない場合のみ適用」というゲートを入れることで実害は消えるが、
判断軸そのものをPhase 1からそのまま流用しない——親指キーでないことが
確認された後は、`On`/`Off`/`Toggle`いずれも「ユーザーがそのキーを
明示的に単独で押した」という意味で安全（この場合は「毎打鍵発火」自体が
安全の根拠ではなく、「そのキーがチョードに参加しない＝押されたら常に
そのキー単体の意図」であることが根拠）。`Toggle`のみopt-inというゲート
自体はPhase 1と同じ理由（GJI側の設定がATOKプリセット由来で状態依存の
反転を持ちうる、というプロダクト判断）で維持する。

### 設計骨子（Opus敵対的設計レビュー3ラウンド収束済み、未実装）

- **データ構造は専用の2フィールド**（`Option<ShadowImeAction>`を
  Hiragana用・Katakana用にそれぞれ1つ）とし、汎用的な`Vec<(VkCode,
  ShadowImeAction)>`のようなテーブルにはしない。**この2フィールドが
  対応するVK自体は固定**（Hiragana用フィールドは常に`VK_DBE_HIRAGANA`
  を指す）なので、「Phase 2の撤回」節が指摘したstaleness問題
  （親指スロットの**VK自体**がreload後に変わり得ることに由来する
  問題）はそもそも発生しない——`henkan_delegate_to_open_axis`/
  `muhenkan_delegate_to_open_axis`と同型の、キー名を冠した固定2
  フィールドで十分。新しい型（`GjiModeKeyShadowOverrides`のような
  構造体）でこの2フィールドを束ねてもよいが、意味的な複雑さを増やさない
  ため`Runtime`の直接フィールドとする。**ただし「この値を今適用して
  よいか」（＝そのVKが現在親指キーかどうか）はユーザー設定で動的に
  変わるため、この判定はフィールドの保持場所や書き込みタイミングとは
  別に、常に消費時に評価する（Opus敵対的レビュー2ラウンド目R1/R2、
  下記「オーバーライドの適用条件」参照）。「対象VKが固定だから
  stalenessが起きない」という主張は、あくまで「フィールドとVKの
  対応」についてのものであり、「適用可否の判定」には及ばない。
- **保持場所は`crate::runtime::Runtime`**（`gji_thumb_key_ime_toggle_opt_in`
  等、既存のGJI関連キャッシュフィールドと同じ並び）。`FocusTracker`には
  置かない——`FocusTracker`は「ユーザーがconfig.tomlで明示した同期キー」
  （`sync_toggle/on/off_keys`）専任の集約点であり、doc コメントの
  スコープも「IME sync キー情報」に限定されている。今回の値はGJI設定を
  awase側が**自動検出**した結果であり意味的に別物（`sync_direction`と
  `shadow_action`が優先順位上も別レーンである既存設計と対応する）。
- **書き込み点は1つに限定する**（当初案の「2つ」——GJI側と
  MS-IME側——から訂正）。`msime_key_assignment.rs`を確認したところ、
  MS-IMEレジストリには`KeyAssignmentCtrlSpace`/`ShiftSpace`/
  `Muhenkan`/`Henkan`の4値しか存在せず、Hiragana/Katakanaに対応する
  レジストリ値が無い（MS-IMEはこの2キーの動作をユーザー設定で変更
  できない）。よって「MS-IME側の書き込み点」で書くべき値がそもそも
  存在しない。書き込みは`gji_charset_autodetect::
  sync_gji_charset_autodetect`（GJI側）のみで行う。**この書き込みは
  親指キー判定を一切行わず、`classify_mode_key_ime_action`＋opt-inゲート
  の結果だけを、毎回2フィールドとも無条件に（`Some`か`None`を必ず
  代入する形で）書く**（Opus敵対的レビュー2ラウンド目R1指摘、Phase 1の
  `set_gji_thumb_key_delegate_to_open_axis`と同じ「ペア全書き」規律）。
  親指キーかどうかの判定は下記のとおり消費時に行うため、書き込み時には
  一切関与させない——これにより「書き込み後に親指キー割当てだけが
  変わり、再計算が別のタイミングまで走らない」という時間差が生じても、
  常に消費時の最新状態で正しく判断される。GJI離脱時
  （`is_gji == false`の早期returnブロック、既存の
  `app.clear_gji_ime_on_off_auto_keys()`と同じ場所）に**両方を`None`へ
  戻す**。`sync_gji_charset_autodetect(app, is_gji)`は
  `sync_ime_kind_from_observation`から**MS-IME側の同期より必ず先に**
  無条件で呼ばれる（既存の`ime_toggle_auto`と同じ呼び出し順序保証、
  `message_handlers.rs`のコメント参照）ため、GJI→MS-IME遷移時は
  「GJI離脱でオーバーライド解除→（MS-IME側にはこの値を書く経路が
  そもそも無いのでこのまま）」という単純な片方向クリアで整合する。
- **オーバーライドの適用条件（B1対策、必須、2ラウンド目R1で消費時判定に
  訂正）: そのVKが現在の親指キーでない場合にのみ適用する。この判定は
  `sync_gji_charset_autodetect`（書き込み時）ではなく、
  `Runtime::enrich_ime_relevance`の中（消費時）で行う。** 理由:
  親指キーの割当ては`config.toml`のreloadで動的に変わるが、その変更が
  `hook::set_thumb_vk_codes`へ反映されるタイミングと
  `sync_gji_charset_autodetect`が再計算されるタイミング（GJI検出済み時
  のみ）は独立している。書き込み時に判定すると、「ひらがなが親指キー
  でない状態でオーバーライドが`Off`/`Toggle`で書き込まれた後、
  ユーザーがひらがなを親指キーへ変更したが、その直後は
  `ime_kind_detected()`がfalse等の理由で再計算が走らない」という窓で
  古い判定のまま`Off`/`Toggle`が適用され続け、B1がそのまま再発する
  （2ラウンド目R1で指摘された具体的な失敗シナリオ）。消費時判定なら
  `crate::hook::thumb_vk_codes()`（`AtomicU32`読み取り、コストは無視
  できる）を毎イベント参照するため、常に最新の親指キー割当てで判断
  され、この時間差問題が構造的に起きない。Altなりすまし
  （`hook.rs`のVK書き換えが`classify_ime_relevance`より前に走る構成）
  でも、書き換え後のVKで判定されるため自動的に整合する。
  具体的には`Runtime::enrich_ime_relevance`（`runtime/mod.rs:434-437`、
  `key_pipeline.rs:48`から呼ばれる既存の唯一の合流点）に、
  `self.focus_tracker.enrich_ime_relevance(event)`の**直後**に新しい
  ステップを追加する: `event.vk_code`が`VK_DBE_HIRAGANA`/
  `VK_DBE_KATAKANA`のいずれかで、対応する`Option<ShadowImeAction>`が
  `Some`、かつ`is_configured_thumb_key(event.vk_code)`が`false`のときのみ
  `event.ime_relevance.shadow_action`をその値で上書きする（親指キーの
  場合、または`Some`が無い場合は何もしない＝静的`ImeKeyKind::from_vk`
  由来の値がそのまま残る）。`kp_stage_shadow_ime_toggle`自体は変更
  不要——`intent_kind`の決定順序（`sync_direction` > `shadow_action`）
  はそのまま、`shadow_action`という**同じフィールドの値だけ**を書き
  換える形にする（当初案は`kp_stage_shadow_ime_toggle`の分岐に3段目を
  追加する設計だったが、既存の2段判定はそのままで済むためシンプルに
  なる）。
- **親指キーの場合の警告は書き込み時のスナップショットで判定してよい**
  （Opusレビュー承認済み——警告はイベント毎ではなく設定変化時に1回だけ
  出したいものであり、消費時判定と異なる責務のため書き込み時のままで
  正しい）。Phase 1で実装しホリスティックレビュー時に「actuation-auto
  への配線と一緒に」完全削除した警告関数
  `warn_mode_key_thumb_key_unsupported_if_needed`とデデュープラッチ
  `LAST_MODE_KEY_THUMB_WARNING`を**復活させる**（削除すべきだったのは
  actuation-autoへの配線であって警告そのものではなかった。警告は
  親指キーケースで唯一取りうる正しい対応として復活させる——実装は
  Phase 1削除前のコードをほぼそのまま戻せる）。`sync_gji_charset_
  autodetect`内で、書き込み時点の`is_configured_thumb_key`判定結果を
  使い、`classify_mode_key_ime_action`が`Off`/`Toggle`を返しているのに
  そのVKが親指キーである場合に警告を出す。
- `Toggle`は既存の`GeneralConfig::gji_thumb_key_ime_toggle`のopt-inゲート
  をそのまま通す（親指キーでないことが確認された後は、`On`/`Off`は
  常時安全、`Toggle`のみopt-in——理由は「上記の判断軸の訂正」節参照）。
  `gate_thumb_key_ime_actions`のHenkan/Muhenkan向けゲート処理と同型の
  変換（`ImeToggleKind::On/Off/Toggle` → `Option<ShadowImeAction>`、
  `Toggle`はopt-in`false`なら`None`）を`sync_gji_charset_autodetect`内で
  行う。
- **`raw`（config1.dbの内容）が読めない場合はオーバーライドを書かない**
  （fail-open対策、Opusレビュー指摘）。`classify_mode_key_ime_action`は
  `raw`不在時に`default_raw`（`session_keymap: None`）で評価され
  Hiragana/Katakanaに対し`Some(On)`を返すが、これはたまたま静的
  `shadow_action`（`TurnOn`）と一致するだけで、意味論的には
  「config1.dbが読めない＝GJIの設定は分からない」のに「`On`と判定した」
  ことにする嘘になる。将来`ImeKeyKind`の静的マップが変わった場合に
  黙って乖離する潜在バグを避けるため、`raw`が`None`のときは
  `classify_mode_key_ime_action`の結果を無視し、オーバーライドを
  書かない（`None`のまま）。
- **`classify_mode_key_ime_action`が`None`を返す場合（ATOK/KOTOERI/
  CHROMEOSプリセット）、オーバーライドは書かない**——静的
  `TurnOn`が残る。これは「GJIはこのキーにIME on/offを割り当てていない
  ＝押してもGJI側は何もしないはずだが、awaseだけがbeliefをONにする」
  という既知の乖離を意味するが、意図的に許容する（`shadow_action`を
  `None`にクリアすると`transport.rs`の`is_kanji_event`判定や
  `IntentWitness::from_physical`の成立条件まで動き、物理キーの
  Suppress/Allow判断に波及するため、値の上書きのみに留める設計を
  優先する）。解消は本ADRのスコープ外。
- **無変換/変換キー（`VK_CONVERT`/`VK_NONCONVERT`）はこのオーバーライド
  の対象外のまま**——`ImeKeyKind::from_vk`がそもそもこの2キーを認識
  しないため`shadow_action`が生えず、`kp_stage_shadow_ime_toggle`の
  昇格条件（`shadow_action.is_some()`）を満たさない。Phase 1で実装済み
  のdelegate-to-open-axis（Step4b）が引き続きこの2キー専用の担当を
  続ける。**したがって「無変換/変換用のdelegate-to-open-axis」と
  「Hiragana/Katakana用のshadow_actionオーバーライド」は、対象キーが
  重複しない2つの独立した機構として共存する**（nicola_fsm.rsは
  変更しない）。

### 実効的な挙動変化の範囲（Phase 2単独の場合、正直な自己評価、Opusレビューr3指摘）

B1対策後、Phase 2**単独**（親指キーでない場合の経路）が実際に挙動を
変えるのは、以下すべてを満たす場合**だけ**である: (1) 対象が
Hiragana/Katakana、(2) そのキーが親指キーでない、(3)
`classify_mode_key_ime_action`が`Off`または`Toggle`を返す
（CUSTOMキーマップに`Precomposition Hiragana IMEOff`相当がある場合のみ
——ms-ime/mobileプリセットや情報源不在は`On`で静的値と同値のためno-op、
ATOK/kotoeri/chromeosは`None`で不適用）、(4) `Toggle`の場合はopt-inが
true、(5) そのVKが`[keys.ime_detect]`に列挙されていない。かなり狭い
交差条件であり、これは「設計が悪い」のではなく「もともと静的
`shadow_action`マップがHiragana/Katakanaに関してはだいたい正しかった」
ことを意味する。**Phase 2単独の実質的な主成果物は、挙動変化そのものより
むしろ「親指キーの場合の正しい警告」の復活である**——ただしこの警告は
Phase 3を実装しない場合の話であり、下記のとおりPhase 3を統合すると
親指キーの場合も実際に挙動を変えられるようになる。実機ソークでPhase 2
単独の挙動変化が観測できなくても、それ自体は「壊れている」兆候では
ない——条件(3)（CUSTOMキーマップでの`Off`/`Toggle`割当て）に実際に
該当する環境が無いだけの可能性が高い。

## Phase 3: 親指キーの場合はdelegate-to-open-axisをHiragana/Katakanaへ拡張（2026-09-05、Phase 2と統合）

### 動機

Phase 2単独では、BUG-115の本来の報告シナリオ（Hiragana/Katakanaを
親指シフトのチョードキーにしている場合）を解決できない
（上記「Phase 2単独の限界」参照）。Henkan/Muhenkanには既に
`delegate-to-open-axis`（`resolve_pending_thumb_as_single`、単独タップ
確定時にのみ発火、チョード判定とは物理的に衝突しない）という、この
問題を正しく解く既存の安全な機構があるため、これをHiragana/Katakana
へも拡張する。ユーザー指摘（「Phase 2とPhase 3を同時にやる方が効率的
ではないか」、2026-09-05）を受け、別フェーズに分けず統合して設計・
レビューする。

### なぜ以前のv1（撤回済み）と違うのか

「Phase 2の撤回」節で撤回した当初案（v1）は、`nicola_fsm.rs`の
Henkan/Muhenkan専用フィールドを「左右親指スロット」という汎用概念へ
書き換え、**Mozcが認識する8キー全て**（Kana/ImeOn/Junja/KanjiToggle/
ImeOff/Alphanumeric/Katakana/Activate等）に対して機械的に適用しようと
していた。今回のPhase 3は以下の点で明確に異なる、より狭いスコープの
提案である:

1. **対象は2キーのみ**（`VK_DBE_HIRAGANA`/`VK_DBE_KATAKANA`）。v1が
   踏んだ「走査対象8キーのうち実際に到達しうるのは3キーのみ」
   （Eisu/Hiragana/Katakana）という指摘のうち、Eisuは
   「スコープ確定」節の理由で対象外のまま。
2. **汎用スロット抽象化はしない**。`henkan_delegate_to_open_axis`/
   `muhenkan_delegate_to_open_axis`と同型の、キー名を冠した専用
   フィールドを2つ追加するだけ（v1が「却下した代替案」で検討し
   「8個案よりスロット案が優れる」と結論した比較は、それ自体が
   Henkan/Muhenkan以外のキーを汎用スロットに載せる設計を前提として
   おり、今回は「スロット」概念を使わないため、この比較の対象外）。
   対象VKは常に固定（`VK_DBE_HIRAGANA`/`VK_DBE_KATAKANA`）で
   ユーザー設定によって変わらないため、v1のstaleness問題
   （スロットのVKがreloadで変わりうる）はそもそも発生しない
   （Phase 2の「データ構造は専用の2フィールド」節と全く同じ理由）。
3. **v1が見落としていたBUG-14相当の穴を、本設計では最初から塞ぐ**
   （下記「BUG-14ガード: `injected`を`ClassifiedEvent`まで伝播させる」
   参照）。v1のレビューでは「delegateは別コードパスだからinjected
   非チェックとは無関係」という誤った主張がされ、実際には
   `PendingThumb`遷移に`injected`ガードが無いことが見過ごされていた。
   今回はこの穴を実コード確認済みの上で正面から設計に組み込む。
4. **既に持つ2キーだけを対象にするため、無変換/変換の`delegate_to_
   open_axis`判定（`resolve_pending_thumb_as_single`内の分岐）を
   書き換えず、並列に2本追加するだけで済む**——既存の分岐ロジック
   自体は変更しない（v1は分岐ロジックの書き換えが必要と指摘されていた
   が、それは「どのキーでも汎用的に動くように」という設計だったため。
   今回は「Henkan/Muhenkan判定」「Hiragana/Katakana判定」を単純に
   併記するだけで済む）。

### 設計骨子（2ラウンド目レビューP2で訂正: VK定数はPlatform層から渡す）

- **`NicolaFsm`に専用フィールドを4つ追加**:
  `hiragana_vk: Option<VkCode>`/`katakana_vk: Option<VkCode>`
  （Platform層が解決した「現在Hiragana/Katakanaが親指キーかどうか」を
  保持する。`muhenkan_vk`/`henkan_vk`と同型）、
  `hiragana_delegate_to_open_axis: Option<ShadowImeAction>`/
  `katakana_delegate_to_open_axis: Option<ShadowImeAction>`
  （GJI検出値、`henkan_delegate_to_open_axis`/
  `muhenkan_delegate_to_open_axis`と同型）。**コア`awase`クレートに
  `VK_DBE_HIRAGANA`のような生VK定数を書いてはならない**（Opus2ラウンド目
  レビューP2指摘、CLAUDE.md「no raw VK-code magic numbers outside
  `crates/awase-vkmap`」、および既存の`muhenkan_vk`のdocコメント
  「実際のVK番号はPlatform層の責務であり、coreは渡された値と等値比較
  するだけ」に従う）。`resolve_pending_thumb_as_single`内の既存の
  `self.muhenkan_vk == Some(vk_code)`/`self.henkan_vk == Some(vk_code)`
  判定と全く同型の`self.hiragana_vk == Some(vk_code)`/
  `self.katakana_vk == Some(vk_code)`判定を並列に追加し、対応する
  delegateフィールドが`Some`ならHenkan/Muhenkanと同じ
  `ime_open_requested`（one-shot channel、`Engine::apply_ime_open_
  request`が消費）を使って反映する。
- **Platform層の配線が2箇所必要**（`muhenkan_vk`/`henkan_vk`と同じ
  パターン、Opus2ラウンド目レビューP2指摘）: `crates/awase-windows/
  src/app/bootstrap.rs`（起動時、`set_thumb_key_solo_tap_config`
  付近）と`crates/awase-windows/src/runtime/mod.rs::apply_config_
  update`（reload時）の両方で、`[left_thumb_vk, right_thumb_vk].
  into_iter().find(|&vk| vk == crate::vk::VK_DBE_HIRAGANA)`のような
  形でPlatform層が解決し、`Engine`経由で`NicolaFsm::hiragana_vk`/
  `katakana_vk`へ渡す。**片方だけだと、起動直後から最初のreloadまで
  Phase 3が沈黙する**（v1のB3「bootstrap配線漏れ」と同型の穴、
  ※以下の`hiragana_vk`/`katakana_vk`のsetter配置に関するS1の制約と
  合わせて読むこと）。**S1（3ラウンド目レビュー指摘、実装時に必ず
  守ること）: `runtime/mod.rs::apply_config_update`側の
  `hiragana_vk`/`katakana_vk` setterは、`if let (Some((left, ..)),
  Some((right, ..))) = (resolve_thumb_key(..), resolve_thumb_key(..))`
  という既存のif-letブロックの**中**、`set_thumb_key_solo_tap_config`
  呼び出しに隣接して置くこと。** ブロックの外に置くと、
  「Invalid thumb key names」パス（`resolve_thumb_key`失敗時にブロック
  ごとスキップされ`hook::thumb_vk_codes()`が前回値を返し続ける経路）で、
  Phase 1由来の所有権ゲート（`hook::thumb_vk_codes()`をliveに読む）と
  FSM（`hiragana_vk`、reload時に書かれた値を読む）が別々の親指キー
  ペアを見る状態が作れてしまう。bootstrap側も`left_thumb_vk`/
  `right_thumb_vk`から直接導出する既存パターンに揃える。
  今回は明示的に対応する）。
- **書き込み元は`classify_mode_key_ime_action`＋opt-inゲート**——
  Phase 2の書き込みロジック（「`raw`が読めない場合は書かない」
  「`None`のときは書かない」等の設計骨子の判断も含め）を**そのまま
  再利用**する。相違点は消費先だけ: Phase 2は`Runtime`の2フィールド
  （`shadow_action`オーバーライド用）へ、Phase 3は`Engine`経由で
  `NicolaFsm`の`hiragana_delegate_to_open_axis`/
  `katakana_delegate_to_open_axis`へ、**同じ`classify_mode_key_ime_
  action`の出力を両方に配線する**（`set_gji_thumb_key_delegate_to_
  open_axis`がHenkan/Muhenkanに対して既にやっているのと同じパターンを、
  `sync_gji_charset_autodetect`内でHiragana/Katakanaにも並列に行う
  だけ）。
- **`is_configured_thumb_key`によるPhase 2/Phase 3の排他性は、判定
  タイミングを揃える必要がない**（Phase 2のR1が抱えていた「書き込み時
  判定は陳腐化する」問題は、Phase 3側には存在しない）。理由:
  `resolve_pending_thumb_as_single`が`vk_code`に対して呼ばれること
  自体が、その物理キーが**現在**`hook.rs::classify_key`
  （`config.general.left_thumb_key`/`right_thumb_key`駆動）で
  `LeftThumb`/`RightThumb`に分類されたことを意味する。**ただしこの
  保証は、上記「Platform層の配線が2箇所必要」で`hiragana_vk`/
  `katakana_vk`をbootstrap/reloadの両方で最新に保つことに暗黙に
  依存している**（Opus2ラウンド目レビューP4指摘）——配線が片方
  だけだと、`PendingThumb`が生成されてから解決されるまでの窓
  （最大`threshold_ms`）でreloadが挟まった場合に、解決時点で
  `hiragana_vk`が古いままの状態でdelegateが誤発火しうる。上記の
  bootstrap/reload両方の配線が前提条件であることを明記する。
  Phase 2側（`enrich_ime_relevance`）だけが独自にこの判定をやり直す
  必要があったのは、shadow-toggleが`KeyClassification`を一切参照
  しない別系統のパイプラインだから（「オーバーライドの適用条件」
  節参照）——Phase 3はNICOLAのチョード判定システムに乗る形なので、
  この種の判定の重複はそもそも発生しない。**S2（3ラウンド目レビュー、
  修正不要・受容する残留事項）: ゲート評価（KeyDown時点、
  `hook::thumb_vk_codes()`をlive読み）とFSMの解決（最大`threshold_ms`
  後）の間にconfig reloadが挟まると、1打鍵だけ次のいずれかが起こり
  うる——(a) reloadでHiraganaが親指キーでなくなった場合、ゲートは
  既にshadow-toggleを止めており、FSM側は`hiragana_vk = None`で
  delegateも発火しない結果その1打鍵だけbeliefが追従しない（キー自体は
  既定分岐でOSへ送出されるためGJI自体は動く、次のprobeで収束する）。
  (b) 逆にreloadでHiraganaが親指キーになった場合、ゲートは「非owned」
  と判断してshadow-toggleを通し、FSM側はdelegateを発火する結果その
  1打鍵だけP1の衝突が起きる（ON→OFFが1回）。いずれもreload境界の
  一過性であり恒久的な状態破壊にはならないため、対策（ゲート判定を
  `PendingThumbData`へスナップショットして持ち回る等）は過剰と判断し、
  受容する。`delegate_armed`がGJI離脱で途中クリアされる場合も同種の
  一過性が起こりうるが、同じ扱いで問題ない。**
- **`Toggle`は既存の`GeneralConfig::gji_thumb_key_ime_toggle`のopt-in
  ゲートをそのまま通す**（Phase 2と同じ判断軸・同じゲート関数を
  共有する）。
- **v1のB2（stale delegateによるSpace/Enter乗っ取り）は再発しない**
  （Opus2ラウンド目レビューP2で確認済み）: delegateは固定VK
  （`hiragana_vk`/`katakana_vk`）に紐づき、これらは対象キーが親指キー
  でなければ`None`になるため、v1が検討した「スロット」案と違って
  `VK_SPACE`のような無関係なキーを指すことが原理的にありえない。

### BUG-14ガード: `injected`を`ClassifiedEvent`まで伝播させる（新規、必須）

v1のレビューが指摘し、今回のPhase 3設計で実コードを確認して裏付けた
問題: `src/engine/confirm_policy.rs:75-79`（`idle_wait`）は
`ev.key_class.is_thumb()`だけで`PendingThumb`への遷移を決めており、
`event.injected`を一切見ない。`src/engine/input_tracker.rs:161-178`
（`InputTracker::classify`、`RawKeyEvent → ClassifiedEvent`変換）を
確認したところ、`ClassifiedEvent`（`fsm_types.rs:43-61`）には
そもそも`injected`フィールドが存在せず、**`RawKeyEvent.injected`は
`Engine`の境界（`engine.rs::on_input`）で握りつぶされ、
`NicolaFsm`まで伝播しない**（`ADR-019`のプラットフォーム非依存原則
そのものが理由——`NicolaFsm`はプラットフォーム層が事前分類した
情報しか受け取らない設計）。

これはHenkan/Muhenkanの既存delegateがこれまで安全だった理由が
「チョード判定を経由するから」ではなく**「MS-IME/CTFが実際には
`VK_CONVERT`/`VK_NONCONVERT`を注入しないから」という運**に支えられて
いたことを意味する。`VK_DBE_HIRAGANA`/`VK_DBE_KATAKANA`はBUG-14で
実機確認済みの通り**実際に注入される**キーであり、この2キーへ
delegate-to-open-axisを無条件に拡張すると、外部注入された偽の単独
タップが`resolve_pending_thumb_as_single`をそのまま通過し、BUG-14と
同型の脆弱性を新規に作る。

**対策: `injected: bool`を`ClassifiedEvent`と`PendingThumbData`へ
追加する。** `is_ime_control`/`modifier_key`と同じ「プラットフォーム層
が事前分類した意味論的な事実」であり、生のVKコードや`windows-rs`型を
持ち込むわけではないため、ADR-019（core `awase`クレートのプラット
フォーム非依存原則）には抵触しない——`RawKeyEvent.injected`は既に
プラットフォーム非依存の`bool`として`awase::types`に存在する。
`InputTracker::classify`で`event.injected`をそのまま`ClassifiedEvent.
injected`へコピーし、`PendingThumbData::from_event`で
`ClassifiedEvent.injected`をそのまま引き継ぐ。`resolve_pending_
thumb_as_single`内のHiragana/Katakana分岐（上記「設計骨子」）は、
`thumb.injected`が`true`の場合はdelegateを発火させず、Henkan/Muhenkan
を含む既存の分岐と同じ既定動作（Suppress/Passthroughの通常判定）へ
フォールバックする。**Henkan/Muhenkanの既存分岐にはこのガードを
追加しない**（挙動変更を避ける——長年「注入されないから安全」で
動いてきた経路に新しい早期returnを挿すのはこのADRのスコープ外の
リグレッションリスクを持ち込む。ただし将来の`/code-review`等で
「Henkan/Muhenkanにも同じガードを対称に追加すべきでは」という指摘が
出ることは想定内であり、その場合は別issueとして検討する）。

**injected時のフォールバック先はPassthrough相当でなければならない**
（Opus2ラウンド目レビューP6指摘）。`injected == true`でdelegateを
スキップして既定分岐へフォールスルーすると、非composing時は
`KeyAction::Key(vk_code)`を返し、出力層が`INJECTED_MARKER`付きで
`SendInput`するためOSへ届く。これにより「awaseもactuateせずOSにも
届かない二重の空振り」（ADR-119決定4が禁じるパターン）にはならない
——ただしこれは**フォールバック先がPassthrough相当であることに依存
する**。将来「injectedならSuppressでよい」と最適化するとADR-119
違反になるため、実装時は「injected時はdelegateを発火させないが、
キー自体は既定分岐でOSへ送出される（Suppressにしてはならない）」と
コードコメントに明記すること。

### Phase 2の静的`shadow_action`（TurnOn固定）firingとの衝突と、その解消（P1、Opus2ラウンド目レビューでブロッカー発覚・対策確定）

**当初「無害に共存する」と記述していたが誤りだった。** `key_pipeline.rs`
の実際の実行順序（`kp_stage_shadow_ime_toggle`が先、その戻り値を使って
`build_input_context`が`ctx.ime_on`を組み立て、NicolaFsmはその後で
呼ばれる）を確認したところ、**Phase 2の静的`TurnOn`とPhase 3の
delegateは同一の物理打鍵に対して両方発火し、実際に競合する**ことが
判明した。

**失敗シナリオ（IME OFF、Hiragana=右親指キー、GJIが`Toggle`＝opt-in
済み）**: (1) KeyDown → `kp_stage_shadow_ime_toggle`が静的`TurnOn`で
発火 → belief OFF→ON、`SetOpen(true)`を実際にactuate。(2) FSMは
`PendingThumb`へ。(3) 単独タップ確定 → Phase 3delegate`Toggle`
→`!ctx.ime_on`。`ctx.ime_on`は手順(1)の結果を反映して`true`
→`SetOpen(false)`。(4) 正味: ON→OFF、**キーを押しても何も起きない**。
これは撤回したv1の欠陥（`VK_KANJI`のToggleが二重トグルで実質no-opに
なる）と同型。**`Off`の場合はさらに悪く**、1打鍵で逆方向2回の
actuationとIMEのちらつきが起きる（cold-startアプリでのリテラル漏れ
リスクを伴う）。`On`の場合だけは`ime_set_open_effects`の
`prev_activation`重複抑止により実害が出ないが、それは静的値と同値の
ケース＝そもそもPhase 3が不要な場合でしかない。

**対策（2ラウンド目レビューQ1/Q2/Q4で3回訂正）: `kp_stage_shadow_ime_
toggle`の`intent_kind`決定に、明示的な所有権ゲートを条件として組み
込む。**

**ゲート条件（Q4で訂正）: `is_configured_thumb_key(vk) &&
delegate_armed(vk)`の積。** 「delegateが armed かどうか」の二値だけ
では不十分——書き込み側（`sync_gji_charset_autodetect`）は親指キー
判定を一切行わず、Phase 2の`Runtime`側override用フィールドとPhase 3の
`Engine`側delegate用フィールドへ**同じ`classify_mode_key_ime_action`
の出力を無条件に両方配線する**（「設計骨子」節参照）ため、Hiragana/
Katakanaが親指キーでなくてもdelegateはarmedになりうる。「armedなら
常にshadow-toggleを止める」という当初案では、非親指キーでGJIが
`Off`/`Toggle`を返すケース（＝Phase 2の唯一の実効ケース）で、
shadow-toggleもFSM delegateも両方発火しない「誰も何もしない」状態を
作ってしまう（Phase 2の適用条件は`!is_configured_thumb_key(vk)`なので、
delegate armedだけを見るゲートはPhase 2の適用領域と重なってしまう）。
`is_configured_thumb_key(vk) && delegate_armed(vk)`という積にすることで、
Phase 2の適用条件（`!is_configured_thumb_key(vk)`）とちょうど補集合に
なり、Phase 2とPhase 3が入力空間を過不足なく分割する。**この不変条件
（ゲートはFSM側の実際の発火条件——`hiragana_vk == Some(vk_code)`かつ
delegateが`Some`——と厳密に一致していなければならない。片方だけ真に
なる領域は「どちらも発火しない穴」または「両方発火する衝突」を作る）
を実装コメントに明記すること。**

**ゲートの配置（Q1で訂正）: 関数先頭の早期returnにしない。**
`kp_stage_shadow_ime_toggle`の`intent_kind`決定
（`key_pipeline.rs:878-887`）は`sync_direction`（`SyncKey`）を
`shadow_action`（`PhysicalImeKey`）より先に見る2段判定であり、
`shadow_action = None`方式を採らない理由（`VK_DBE_KATAKANA`の
`transport.rs:224`での`is_kanji_event`判定、BUG-52対策）は既に確認
済みだが、**ゲートを関数先頭に置くと`sync_direction`経路
（ユーザーが`[keys.ime_detect]`に明示指定したキー）まで巻き込んで
殺してしまう**——これはPhase 2自身が確立した「ユーザーの明示設定が
自動検出に優先する」方針に反する。正しくは、`intent_kind`を決める
`else if`分岐（`shadow_action`由来の`PhysicalImeKey`昇格の条件式）
の中に条件として追加する形にする（例:
`else if is_japanese_ime() && !delegate_owns_open_axis(vk) { ... }`）。
`sync_direction`が立っている場合は、このゲートの影響を受けず従来
どおり`SyncKey`として昇格する。

**ゲートが巻き込んではならない3つの副作用（Q2で追加、必須対応）:**
`kp_stage_shadow_ime_toggle`の昇格パスには、belief更新・actuation
以外に3つの副作用がぶら下がっている（`key_pipeline.rs:940-1010`
付近）: `eisu_reset_on_turn_on_while_open`（IMEが既にopenで`TurnOn`が
来たときのstale`ObservedEisu`救済、2026-07-09 MS Edge実害の対策）、
`eisu_reset_on_ime_on`（OFF→ON遷移時の同種救済）、
`kp_restore_kana_from_half_width`（半角英数持続トグルON中の復帰）。
**ゲートで昇格自体をスキップすると、owned キー（Hiragana/Katakanaが
親指キーでdelegate armed）についてこの3つの救済が全て失われる。**
Phase 3のdelegateはこれの代替にならない——`TurnOn`で`ctx.ime_on`が
既にtrueならdelegateも`ime_set_open_effects`の`prev_activation`で
no-opになり、`PostSetOpenEisuReset`も発火しないため。結果として、
2026-07-09のMS Edge事案（IME open + convがEisu固着 →
ひらがなキーで復帰できない → `ObservedEisu`がengine activationを
塞いで永久inactive）が、ひらがな親指キー構成で再発しうる。

**したがってゲートは「belief書き込みとactuationだけをスキップし、
Eisu救済・半角英数復帰の副作用は従来どおり実行する」形にする**
（Opusレビュー推奨案(a)、代替案(b)「delegate側でEisu救済を対で配線
する」はdelegateがno-opになるケースで救済も落ちる穴が残るため不採用）。

**S3（3ラウンド目レビューで訂正、実装は当初想定より遥かに単純）:**
「現在ひとつながりになっている処理を分岐させる必要がある」という
当初の記述は誤りで、実装者に過剰なリファクタを誘発しかねないため
訂正する。実際の`kp_stage_shadow_ime_toggle`の構造
（`key_pipeline.rs`、概略）は`match kind { .. write_physical_key(..)
.. }` → `if effective_open() == current { ..
eisu_reset_on_turn_on_while_open(..); return false; }` →
`on_ime_toggled()` → `eisu_reset_on_ime_on(..)` → actuation、という
順序になっている。**owned キーで`write_physical_key`の呼び出しを
1つスキップするだけで、書き込みをしていないため
`effective_open() == current`が恒真になり、制御は自然に既存の
no-op分岐へ落ちる。** その結果、`eisu_reset_on_turn_on_while_open`
（救済1）と`kp_restore_kana_from_half_width`（半角英数復帰）は
**何もしなくても従来どおり実行され**、`return false`で抜ける。
新しい分岐構造を作る必要はない。

この帰結として、owned キーでは`eisu_reset_on_ime_on`（救済2、
OFF→ON遷移時）は構造的に到達不能になる（OFF→ON遷移は書き込み無しには
起こらないため）が、これは正しい状態である——owned キーのOFF→ONは
Phase 3のdelegateが担当し、そのSetOpen(true)はDecision経由なので
`PostSetOpenEisuReset`（`ime-belief-architecture.md`が「activation側の
救済はDecision経由SetOpen(true)限定」と定義する、まさにその救済）が
対で効く。**`state/eisu_recovery.rs`の経路×救済対応表には次の形で
記載する**（`.claude/rules/ime-belief-architecture.md`が要求する対応、
`tests/architecture_guard.rs::user_ime_on_paths_are_paired_with_
eisu_reset`が新しい対応関係を正しく検知することを実装時に確認する）:

| 経路 | 救済 |
|---|---|
| owned キーのshadow-toggle（belief書き込みなし、TurnOn while open） | `eisu_reset_on_turn_on_while_open`（従来どおり実行される） |
| owned キーのPhase 3 delegate（`SetOpen(true)`、OFF→ON） | `PostSetOpenEisuReset`（Decision経由の既存救済） |
| 非owned キーのshadow-toggle | `eisu_reset_on_ime_on`/`eisu_reset_on_turn_on_while_open`（従来どおり） |

**S4（ログ、実装時注意）**: `write_physical_key`呼び出しの直前にある
`log::info!("[shadow-toggle] intent 昇格: vk=... kind=... {}→{}", ..)`
は、2026-08-05の「IME OFF後に勝手にONへ戻る」再発調査のために
追加されたtriage上load-bearingなログ（「このステージがlast_intentを
書き換える唯一経路の一つでありながら従来INFOログが無く、実機ログ
だけではどのVKが昇格を発火させたか判別できなかった」）である。owned
キーで書き込みをスキップする際にこのログをそのまま流用すると、
実際には昇格していないのに「昇格」と出力され実機ログが嘘になる。
owned キーでスキップする場合は専用のログ行（例:
`[shadow-toggle] vk=0x{:02X}はFSM delegate所有 →
belief書き込み/actuationをスキップ`）に分けること。

**S5（3ラウンド目レビューで検証済み、`shadow_toggled=false`の帰結）:**
owned キーで関数が`false`を返すことにより、`transport.rs::plan`へ
渡る`shadow_toggled`もfalseになる。影響を確認した結果、問題は無い:
`VK_DBE_HIRAGANA`(0xF2)は`plan`冒頭の専用F2分岐が`shadow_toggled`を
読む前にreturnするため無関係。`VK_DBE_KATAKANA`(0xF1)のKeyDown抑止
条件は`ime_actuation_owned && (shadow_toggled || is_dbe_mode_key_down
|| KeyUp)`であり、既定の`DbeModeKeyPolicy::Suppress`下では
`is_dbe_mode_key_down`が常にtrueになるため`shadow_toggled`と無関係に
Suppressが維持される（**BUG-52は再発しない**）。隠し設定
`dbe_mode_key_policy = Passthrough`の場合のみ`is_dbe_mode_key_down`が
falseになるが、owned キーは定義上親指キーなのでFSMが
`Decision::Consume`を返し、`executor.rs::execute_relay:398-443`が
reinjectを積まない——`Decision::Consume`経路ではtransportの
Allow/Suppress判定はそもそも参照されないため、実際の挙動に差は
出ない。

これにより「**owned キー（親指キー×delegate armed）については、
belief書き込みとactuationはPhase 3のdelegateだけが単独タップ確定時に
行い、shadow-toggleは同じ書き込みを行わないが、Eisu救済・半角英数復帰
の副作用は従来どおりKeyDownのたびに実行される**」という設計になる。
これ以外（非owned キー、すなわちdisarmed、または親指キーでない）では
shadow-toggleは全く変更されない（従来どおり毎打鍵で昇格・書き込み・
副作用の全てが走る）。

**「`kp_stage_shadow_ime_toggle`自体は変更不要」というPhase 2設計骨子
の記述は、Phase 3統合により成立しなくなる。** この新ゲートは
`feedback_new_actuation_gate_must_cover_all_choke_points`（IME
actuationに新gateを足すときは全合流点を洗い出す）の対象であり、
今回の合流点は「shadow-toggleのbelief書き込み・actuation部分」
「Phase 3 delegate」の2つで全てである（Eisu救済・半角英数復帰の
副作用は合流点ではなく、両方の経路から独立して実行され続けるため
対象外）。Henkan/Muhenkanのdelegateは対象外のVKのため無関係。

### 実装レビューで発覚したC1（ブロッカー）: ゲート条件に「エンジン活性（belief ON）」が抜けていた

**上記のゲート条件は「親指キー×delegate armed」の積として設計・実装
したが、これだけでは不十分だった。** Opus敵対的コードレビュー（実装後、
2026-09-05）で、ゲート条件がFSM側の実際の発火条件と完全には一致して
いないことが発覚した。

Phase 3のdelegateは`resolve_pending_thumb_as_single`からしか発火
しない。この関数へ到達するには`NicolaFsm`が`PendingThumb`へ入る必要が
あり、そのためには`Engine::on_input_body`の活性ゲート
（`compute_state`が`!ctx.ime_on`なら`InactiveReason::ImeOff`で
即`PassThrough`、`engine.rs:264-275`/`:506-517`）を通過しなければ
ならない。**つまりbelief（`effective_open()`）がOFFの間、delegateは
構造的に発火できない。** ところが元のゲート条件（「親指キー×delegate
armed」）にはbelief状態が含まれておらず、belief OFFのままでもゲートが
成立し、shadow-toggleのbelief書き込みを止めてしまっていた。

**失敗シナリオ（BUG-115の元症状そのものの再現）**: ひらがな＝親指キー、
GJIはms-ime/mobileプリセット（Windows版GJIの実質既定）で`On`判定→
delegate armed。IMEをOFFにした状態でひらがなキーを単独タップすると:
`kp_stage_shadow_ime_toggle`は「親指キー✓×armed✓」でゲート成立→
belief書き込みをスキップ。`on_input_body`は`ctx.ime_on==false`→
`InactiveReason::ImeOff`→PassThrough、**FSMに入らないのでdelegateも
発火しない**。物理キーはOSへ届きGJI自身はIMEをONにするが、**awaseの
beliefはOFFのまま固着**——エンジンが非活性のまま、親指シフトにならない。
実IME状態を観測できないアプリ（Imm32Unavailable/TsfNative/InputRelay、
まさにBUG-115の報告環境）ではこの固着が一時的ではなく恒久的になる。

**対策（実装済み）**: ゲート条件に`effective_open()`（belief ON）を
AND する——`is_configured_thumb_key(vk) && delegate_armed(vk) &&
effective_open()`の3項の積にする
（`crates/awase-windows/src/runtime/key_pipeline.rs::kp_stage_shadow_
ime_toggle`）。**「ゲート条件はFSM側の実際の発火条件と厳密に一致
していなければならない」という不変条件（Q4で最初に立てたもの）は、
実装レビューで初めて全項が判明した**——「親指キー×armed」だけでは
不十分で、「エンジン活性（belief ON）」も発火条件の一部だった。

**この修正後に残る、意図的に許容する残留事項（belief OFF始点のみ、
一過性）**: belief OFFから押した場合、ゲートは不成立（`effective_
open()`がfalse）のため通常どおり静的`TurnOn`でbeliefがOFF→ONへ昇格・
actuateされる。ところが**同一イベント内**でその後`build_input_
context`が`ctx.ime_on=true`を読むため、活性ゲートを通過してFSMが
`PendingThumb`へ入り、単独タップ確定時に**delegateも発火する**
（P1と同型の1イベント内二重発火が、belief OFF始点のケースだけ復活する）。
GJI側の判定値ごとの帰結:
- **`On`**（ms-ime/mobileプリセット＝実質既定、最多ケース）: delegate
  も`TurnOn`。手順1で既にONなので`ime_set_open_effects`の
  `prev_activation`重複抑止により実害なし。**既定ケースは無害。**
- **`Off`**（CUSTOMキーマップで明示設定した場合のみ）: ON→OFFの
  ちらつきが1回起きるが、最終状態はGJIの意味論どおりOFFに収束する。
  1打鍵で逆方向2回のactuationが走るため、cold-startアプリでの
  リテラル漏れリスク（BUG-02/BUG-70系）は残る。
- **`Toggle`**（`gji_thumb_key_ime_toggle=true`のopt-in時のみ到達）:
  delegateが`!ctx.ime_on`=falseを評価しOFFへ戻す。**押しても何も
  起きない**（P1と同型の症状）。

**対応方針（採用: 既知の限界として記録、修正は見送る）**: 1ショット
フラグでdelegate発火を抑止する代替案も検討したが、`take_ime_open_
requested`の消費点が`on_input`/`on_timeout`の2箇所あり、両方を
正しく塞がないと`feedback_new_actuation_gate_must_cover_all_choke_
points`が警告する「合流点の洗い出し漏れ」（issue #136と同型）を
新たに背負う。実害は`Toggle`opt-in（自己責任として既に位置づけ済み）
と`Off`（CUSTOMキーマップ限定）に閉じ、既定ケース（`On`）は無害の
ため、コストに見合わないと判断し見送る。`docs/known-bugs.md`
BUG-115に記録する。

**D1（3ラウンド目レビューで指摘、記述の正確化のみ・設計変更なし）:
このゲートが実際に抑止しているものの正確な性質。** 3項の積
（親指キー×delegate armed×belief ON）が成立する状況を辿ると、
必ず次が成り立つ: (1) ゲート成立⇒そのVKは親指キー⇒Phase 2の
overrideは適用されない（非親指キー限定のため）⇒`shadow_action`は
静的値のまま=`TurnOn`。(2) ゲート成立⇒`effective_open()==true`
（ゲート評価と`current`読み取りの間にbeliefを変える処理は無い）
⇒`current==true`。(1)(2)から`new_val = true == current`——
**ゲートがスキップする書き込みは、常にbelief的にno-opである。**
つまりこのゲートはbelief変化もactuationも一切抑止しない
（構造的に必要なbelief更新を誤って殺すことがありえないという
安全性の証明でもある）。ゲートの現在の実効は、`write_physical_key`
による`last_intent`/`IntentStore`へのintent記録——チョード打鍵の
たびに記録されていたspuriousな記録——を止めることだけであり、
S6が「方向としては改善」と記録した効果そのものである。

**したがって「Phase 2の静的shadow_action firingとの衝突と、その解消」
という本節のタイトルが示す当初の目的（P1の同一打鍵内二重actuationを
解消すること）は、3項の積では**構造的には**達成されていない。**
P1の衝突ケース（belief OFF始点）はまさにこのゲートが不成立になる
領域であり、上記のとおり「意図的に許容する残留事項」として別途
受容している。ゲートを「P1を防ぐ機構」ではなく「chord打鍵ごとの
spurious intent記録を防ぐ機構（副次効果としてbelief的にno-opな
書き込みを整理する）」と理解すること——将来このゲートを
「beliefを何も変えないから不要」と誤って削除すると、intent記録の
抑止（S6の改善）が失われる。

### 実装レビューで発覚したC2（既存の欠陥、Phase 1発見時点では未検出）: Henkan/Muhenkan delegateのTurnOn方向は構造的に発火できない

C1と同じ活性ゲートの制約から、**既存のHenkan/Muhenkan delegate-to-
open-axis（ADR-092決定D Step4b、本ADRより前から存在）のTurnOn方向は、
OFF→ONの遷移を起こすことが原理的にできない**ことが判明した。delegate
は`resolve_pending_thumb_as_single`からしか発火せず、この関数へは
belief ON（エンジン活性）でなければ到達できない。したがって
`TurnOn`のdelegateが実際に評価される時点では、すでにbeliefはON
——「既にONのときにONにする」という常にno-opの経路にしかならない。

**2026-09-05、実機（dragonflyg4、develop相当のPhase 1以前のコード
——この欠陥はPhase 1・Phase 2/3のどちらとも無関係に、ADR-092
Step4b以来ずっと存在していた）で確認済み**: GJIアクティブ、IME OFFの
状態で物理「変換」キー（`VK_CONVERT`=0x1C、既定で右親指キー）を
単独タップしたところ、ログは

```
[engine-input] vk=0x1C KeyDown ... ime_on=false ...
[relay-guard] vk=0x1c down ...
[reinject] vk=0x1c down (queued passthrough now firing)
```

と、`resolve_pending_thumb_as_single`に到達した形跡（「IME open axis
delegated」ログ）が一切無いまま即PassThroughされた。約6秒後、
`[idle-conv-check] TsfNative: conv observation open=true reason=
NativeToggleShadowOff ... → ObserverReported として記録 (engine は
actuate しない)`という**受動的な観測**によってbeliefがようやく
ONへ訂正されている——delegateではなく、既存の「TsfNative conv
観測」フォールバック経路が実際にIME状態を復旧させていた。

**この帰結として、Phase 1がBUG-115の報告症状（「変換キーでON復帰
しても親指シフトにならない」）への対策として配線したHenkan側delegate
は、ON方向には効いていない可能性が高い。** 実際に効いているのは
Muhenkan→`TurnOff`（ON中に押してOFFにする）とATOKの`Toggle`（ON中
にしか発火しないため実質Off専用）だけで、ON復帰そのものは今回確認した
受動観測フォールバック（TsfNative/Standardプロファイルなど観測可能な
アプリでのみ効く）に依存している。**観測できないアプリ
（Imm32Unavailable、BUG-115が実際に報告されたUWP環境）では、この
フォールバックも効かず、Henkan/Muhenkan delegateのTurnOn方向も
効かないため、ON復帰手段が実質存在しない可能性がある。**

**対応方針（本PRのスコープ外、記録のみ・別issue化）**: `fix-requires-
evidence.md`の再発ファミリー「キー選択」「IME belief」に該当するため
記録は必須。恒久対策の候補は`ime_on_auto`
（`Engine::match_ime_on_off_auto`、`check_special_keys`経由で
活性ゲートより**前**に評価されるため非活性でも効く）だが、親指キーへ
素で載せるとチョード打鍵のたびにconsumeされチョードが壊れる
（Phase 1で「親指キーはactuation-autoに載せない」と判断した理由が
そのまま当てはまる）。「belief OFFのときだけ有効なactuation-auto」の
ような設計が必要になるため、本PRのスコープ外として別issueを起票する。
`docs/known-bugs.md` BUG-115の「Phase 1が何を解決したか」の記述も
この発見に合わせて訂正する。

### 優先順位競合の確認（Opus2ラウンド目レビューP7で精査済み・断定可）

`docs/known-bugs.md`のBUG-115には既存の記述として、
`muhenkan_solo_tap_dedicated_fn_key`（専用Fnキー機能）が設定済みの
場合、無変換のdelegate-to-open-axisが`resolve_pending_thumb_as_single`
内の優先順位で黙って無効化される（Henkan側にはこの概念自体が無い
非対称）という既知の挙動がある。**Hiragana/Katakana側の優先順位は、
`resolve_pending_thumb_as_single`（`nicola_fsm.rs:1880-1990`）の実際の
判定順を読むことで、実装前の時点で確定できる**（Opus2ラウンド目
レビューP7指摘、以下は「実装時に精査が必要」という当初の留保を置き
換える確定済みの結論）:

1. `modifier_key.is_some()` → 無条件Suppress（親指キーがOS修飾キーに
   割り当てられている場合）
2. `dedicated_fn_key`（`muhenkan_solo_tap_dedicated_fn_key`、
   **無変換専用**）
3. `delegate_to_open_axis`
4. `mode_key_config`（`muhenkan`/`henkan`専用）
5. Space/Enterの`TextKeyConfig`
6. 既定（composing → Suppress / それ以外 → `Key(vk_code)`）

Hiragana/Katakanaは2・4・5のいずれの対象にも該当せず、1にも該当しない
（DBEモードキーはOS修飾キーではない）。**したがって3（delegate_to_
open_axis）に単純に並列追加すれば、既存の優先順位と競合しないと
断定してよい。** Henkan/Muhenkanと異なる優先順位クラスへ新規に入る
可能性は無い。

### この方式が既存のリスクを構造的に回避する理由（Phase 2について）

以下はPhase 2（`shadow_action`オーバーライド）の設計にのみ当てはまる
リスク評価であり、**Phase 3（delegate-to-open-axis拡張）には当てはま
らない**（Opus2ラウンド目レビューP3指摘。Phase 3独自のリスク評価は
次節「Phase 3のリスク評価」参照）。

- **二重actuationが起きない**（Phase 2について）: `shadow_action`を
  持つキーの追従は`kp_stage_shadow_ime_toggle`という単一の合流点に
  一本化されたまま。Phase 2は新しい合流点を追加しない
  （**Phase 3はこの限りではない**——「Phase 2の静的`shadow_action`
  firingとの衝突」節で扱った、専用ゲートで解消済みの衝突がある）。
- **BUG-14ガードを無償で継承**（Phase 2について）: オーバーライドは
  `event.ime_relevance.shadow_action`という値そのものを
  `enrich_ime_relevance`の中で書き換えるだけで、`kp_stage_shadow_ime_
  toggle`はオーバーライド由来かどうかを区別せず同じ`shadow_action`
  として扱う。`event.injected`早期returnと`IntentWitness`型化
  （ADR-089 §2.2）は`kp_stage_shadow_ime_toggle`が`shadow_action`の値を
  読む**前**に無条件で効くため、値の出所に関わらず自動的に適用される。
- **staleness/bootstrap配線漏れが起きない**（Phase 2について、`Runtime`
  の2フィールドの話）: これらのフィールドが対応するVK自体は固定
  （Hiragana用フィールドは常に`VK_DBE_HIRAGANA`を指す）であり、
  `nicola_fsm.rs`側にVK状態を持たない。**Phase 3はこの限りではない**
  ——`NicolaFsm`に`hiragana_vk`/`katakana_vk`というVK状態を新規に持つ
  ため、bootstrap/reload両方の配線が必須（「設計骨子」節参照、v1の
  B3型の穴を明示的に埋めている）。
- **挙動変化が起きない**（Phase 2について）: `shadow_action`の値だけが
  変わり（何もない→On、TurnOn→Off等）、消費経路自体は変わらない
  ため、Passthrough/Suppressの分岐構造に影響しない。**Phase 3はこの
  限りではない**——次節「Phase 3のリスク評価」参照。
- **config1.db読み取り失敗時（fail-open）は書き込み自体をしないため
  無害**（設計骨子の「`raw`が読めない場合はオーバーライドを書かない」
  参照）。仮に`classify_mode_key_ime_action`をそのまま呼んでいたら
  `default_raw`評価でHiragana/Katakanaに`Some(ImeToggleKind::On)`が
  返り（`classify_absent_session_keymap_yields_on_for_hiragana_katakana`
  で固定済み）、静的`shadow_action`（`TurnOn`）とたまたま一致するため
  結果的にno-opにはなるが、それは偶然の一致に依存した安全性であり
  設計として採用しない（Opusレビュー指摘）。`raw`が`None`のときは
  そもそもオーバーライドを書かないため、偶然の一致に依存せず構造的に
  無害。
- **`sync_direction`（ユーザーがconfig.tomlの`[keys.ime_detect]`へ
  Hiragana/Katakanaを明示指定した場合）が常にオーバーライドより優先
  される**（Opusレビュー確認済み）。`kp_stage_shadow_ime_toggle`の
  `intent_kind`決定は`sync_direction` > `shadow_action`の順であり、
  `enrich_ime_relevance`内で`sync_direction`を書いた後に`shadow_action`
  を書き換えても別フィールドなので衝突しない。既定値
  （`on: ["IMEオン"], off: ["IMEオフ"], toggle: []`、`src/config.rs`）は
  Hiragana/Katakanaを含まないため既定ユーザーには無関係だが、明示的に
  `[keys.ime_detect]`へこの2キーを列挙しているユーザーではオーバー
  ライドは効かない——これは「ユーザーの明示設定が自動検出に優先する」
  という既存の一貫した優先順位方針どおりであり、意図的。

### Phase 3のリスク評価: Passthrough→Consumeの挙動変化とInputRelayプロファイル（Opus2ラウンド目レビューP3、決定要）

**Phase 3は「挙動変化が起きない」わけではない。** 現状（Phase 3導入前）
のHiragana親指キーの非composing単独タップは、`resolve_pending_thumb_
as_single`の既定分岐が`KeyAction::Key(vk_code)`を返し、OSへ送出
される（`KeyClass::Passthrough`相当、GJI自身がこの物理キーを見て
IMEをONにする）。Phase 3ではdelegateがempty actions + `SetOpen`
effectを返すため、**物理キーはOSに届かず、awase側のactuationに
置き換わる**（`Decision::Consume`相当）。`executor.rs::execute_relay`
は`Decision::Consume`では一切reinjectしない
（`physical`dispositionが効くのは`PassThrough`/`PassThroughWith`のみ）
ため、この挙動変化は実際にOSへ届くキーストリームに影響する。

**具体的な懸念: `AppImeProfile::InputRelay`（MWB/RDP等）でキーが
機能しなくなる。** `executor.rs`の`if profile == AppImeProfile::
InputRelay { ... }`分岐は、InputRelayプロファイルのアプリでは
awase自身がactuationを持たず、物理キーをそのまま配送することを前提と
した設計（ADR-119決定4、issue #136で確立）になっている。Phase 3の
delegateがConsumeへ倒すと、InputRelayアプリ内でHiragana/Katakanaを
親指キーにしているユーザーの、この2キーの単独タップが機能しなくなる
リスクがある。

**ただしこれは新規の欠陥ではない**——既存のHenkan/Muhenkanの
delegate-to-open-axisも全く同じ性質（Consume化）を既に持っており、
InputRelay環境でのこの限界は既に存在する。Phase 3はこれをHiragana/
Katakanaの2キー分、対象を拡大するだけである（`fix-requires-evidence.md`
の「IME actuation合流点」ファミリーに該当）。

**この設計では、以下のいずれかを実装時に選択する必要がある**
（Opusレビュー指摘、未決定——次回レビューで方針を確定する）:

- **(i) 既知の限界として許容し、記録する**: Henkan/Muhenkanと同じ
  リスク・対応方針とし、`docs/known-bugs.md`に「InputRelayプロファイル
  下でHiragana/Katakanaを親指キーにしている場合、delegate-to-open-axis
  発火時に物理キーがOSへ届かない」と明記する。実装コストが最小。
- **(ii) InputRelay時はdelegateを発火させず、既定分岐（Passthrough）
  へ意図的に落とす**: 判定に`AppImeProfile`（Platform層の情報）が
  必要なため、コア`NicolaFsm`にプロファイル概念を持ち込まず、
  `Runtime`側でdelegateフィールドをマスク/クリアする形にする
  （`hiragana_delegate_to_open_axis`を`Runtime`が`AppImeProfile ==
  InputRelay`のときだけ`None`で上書きしてから`Engine`へ渡す、等）。
  実装コストは(i)より高いが、Henkan/Muhenkanの既存の穴も同時に塞げる
  わけではない点に注意（今回新設する2キーだけの対策になる）。

**(i)を採用する（2ラウンド目レビューQ3で確定、ただし3点を必ず明記する
条件付き）。** Opusレビューにより、(i)の判断自体は影響範囲の狭さ
（ひらがなを親指キーにしている×リレーウィンドウにフォーカス）を考えれば
許容できるが、「既存のHenkan/Muhenkanと同じ既知の限界」という説明は
**正当化ではなく現状追認にすぎない**——ADR-119は次を明文化された
不変条件として定義している:

> awaseが意図として解釈しない/actuateしない入力を、awaseが消費しては
> ならない。Suppressは「awaseが代わりにactuateするから物理キーは
> 食う」というバーターであり、awaseがactuateしない（できない）状況で
> Suppressだけを行うと、OS側にもawase側にも誰もIMEを切り替えない
> 「二重の空振り」になる。

Phase 3のInputRelayケースはこの定義に**そのまま該当する既知の違反**
である——`executor.rs:788-791`が`NotOwned`を返して即returnする
（actuateしない）のに、`Decision::Consume`は`execute_relay:398-443`で
reinjectを一切積まない（消費する）ため。**したがって(i)を採る場合は
以下を明記する**:

1. これはADR-119が明文化した不変条件の**既知の違反**であること。
   「Henkan/Muhenkanにも同じ穴がある」は説明であって正当化ではない。
2. **なぜtransport側の担保が効かないか**: ADR-119決定4のcondition (b)
   （物理IMEモードキーをsuppressしない）は`transport.rs::plan`に
   実装されているが、`plan`の結果が参照されるのは`Decision::
   PassThrough`/`PassThroughWith`のときだけで、`Decision::Consume`
   経路には構造的に効かない（`executor.rs::execute_relay:398-443`）。
   この一文が無いと、次の担当者は「transportでAllowしているから
   大丈夫」と誤読する。
3. Henkan/Muhenkanの既存delegateにも同じ穴があるため、恒久修正は
   Phase 3側だけでなく共通の1 issueとして起票すること（Phase 3側だけ
   直しても片肺になる）。

`fix-requires-evidence.md`の「IME actuation合流点」ファミリーに該当し
(a)回帰テストか(b)known-bugs記録のいずれかが必須——実機依存で自動化
しにくいため(b)を採用し、`docs/known-bugs.md`にエントリを追加する
（BUG-115の子問題として、または新規BUG番号として記録する）。

**S6（3ラウンド目レビュー、記録推奨）: intent記録の意味論が変わる。**
owned キーで`write_physical_key`をスキップすることで、`last_intent`/
`IntentStore`への記録が「チョード打鍵のたび」から「単独タップ確定時
のみ（Phase 3 delegateの`handle_engine_set_open`経由）」に変わる。
これらを読む消費者は`last_user_explicit_off_ms`、
`from_explicit_off_intent`、drift correctionなど。**方向としては
改善**である——チョード打鍵は「かなを入力している」だけであってIME
操作の意図ではないので、そこで`PhysicalImeKey` intentを記録していた
従来の方が不正確だった。ただし意図せぬ副作用ではないことを示すため
記録する——drift correctionの挙動が変わりうる領域なので、実機ソークで
想定外のdrift訂正が観測された際に、この変更を思い出せるようにして
おく。

### 実装時の補足事項（Opus敵対的レビューM3・M7・M8・M9、実装着手前に確認/実施）

- **クリア処理の配置**: GJI離脱時の2フィールドクリアは、既存の
  `app.clear_gji_ime_on_off_auto_keys()`と**同じ`if LAST_GJI_STREAK_
  CHECKED.swap(NOT_GJI, ..) == GJI_CHECKED`ブロックの中**に置くこと。
  外に出しても動作は無害だが、ラッチと書き込み状態の対応が2箇所に
  分裂し、後から「なぜここだけラッチ外なのか」が読めなくなる
  （Opusレビュー指摘）。
- **監視スレッド死亡時のクリア漏れ**: `sync_ime_kind_from_observation`
  はgji-monitorの変化通知（`WM_IME_KIND_CHANGED`）と起動時pullでのみ
  呼ばれる。監視スレッドが「非検出」を報告しない経路（スレッド死亡等）
  ではクリアが走らない。`ime_on_auto`と同じ既存リスクで新規ではないが、
  記録として残す（対策は別スコープ）。
- **純粋関数を2段に分けて実装すること（3ラウンド目レビュー指摘、実装時
  に必須の配置）**: R1で親指キー判定を消費時に移した結果、決定ロジックは
  「書き込み時（GJI設定→オーバーライド値）」と「消費時（オーバーライド
  値＋親指キー→適用可否）」の2段になる。両方を`#[cfg(windows)]
  mod windows_impl`の**外**（`classify_mode_key_ime_action`/
  `gate_thumb_key_ime_actions`と同じ階層）に置き、Linux上でテストする。
  消費時の純粋関数は`crate::hook::thumb_vk_codes()`を直接呼ばず、
  親指キーペアを引数で受け取る形にする（例:
  `fn resolve_shadow_override(vk: VkCode, hiragana_ov: Option<ShadowImeAction>,
  katakana_ov: Option<ShadowImeAction>, thumb_pair: (VkCode, VkCode)) ->
  Option<ShadowImeAction>`）。`hook::thumb_vk_codes()`の呼び出しは
  `Runtime::enrich_ime_relevance`側（Windows専用）にのみ置く——
  `windows_impl`内に置いたままだと外に出せず、CLAUDE.mdが警告する
  「`#[cfg(windows)]`配下の`#[cfg(test)]`はLinuxのテストバイナリに
  存在すらしない（`cargo test --list`にも出ずエラーも出ない）」という
  罠を踏む。
- **書き込み時の決定ロジックも`is_gji: bool`を引数に取る純粋関数にする**
  （GJI離脱時は`(None, None)`を返す）ことで、GJI離脱時のクリアも
  `gate_thumb_key_ime_actions`と対称な形で純粋関数テストにできる
  （Windows専用テストとして切り離す案より、この対称性を優先する）。
- **回帰テスト（Linux上で可能、最低4本）**: (1) 対象VKが親指キーのとき
  `resolve_shadow_override`が`Off`/`Toggle`を反映しない（B1の固定）、
  (2) 親指キーでないとき反映する（`Toggle`はopt-in trueのときのみ、
  書き込み時の決定ロジック側でテスト）、(3) `raw`が読めないとき書き込み
  側がオーバーライドを書かない、(4) `is_gji=false`で書き込み側が両方
  `None`を返す（GJI離脱時のクリア）。
- **architecture_guardテスト1本追加**: `event.ime_relevance.shadow_action`
  へ書き込む本番コードが`hook::classify_ime_relevance`と
  `Runtime::enrich_ime_relevance`の2箇所だけであることをテキスト検査で
  固定する（先例: `uia_async_focus_kind_handler_does_not_write_belief`）。
- **`enrich_ime_relevance`の呼び出し点は2つ、いずれも消費時判定
  （R1）と両立することを確認済み**: `key_pipeline.rs:48`（通常経路）と
  `message_handlers.rs:1382`（`INPUT_DEFER` drain経路）。
  `hook::thumb_vk_codes()`は`AtomicU32`の単純読み取り（ロック不要、
  `hook.rs:508-511`）なので両経路とも問題なし。drain経路は「フックが
  イベントを捕捉した時刻」と「enrichする時刻」にズレがあり、その間に
  reloadで親指キー割当てが変わると保留イベントは**新しい**割当てで
  判定されるが、これは既存の`sync_direction`判定（`focus_tracker.
  enrich_ime_relevance`も捕捉時ではなくdrain時の`sync_*_keys`で判定）と
  同じ性質であり、新しい不整合を持ち込まない。また`kp_run_inner`が
  IME-OFF rescueのreplayで自分自身を再帰呼び出しし同じイベントに対して
  `enrich_ime_relevance`が2回走るケースも確認したが、オーバーライドは
  同じフィールドに同じ値を書くだけで冪等、かつ単一スレッドのメッセージ
  ループ内で2回の間に状態が変わることもないため無害（3ラウンド目
  レビューで確認済み）。実処理を`Runtime::enrich_ime_relevance`に置く
  設計であれば追加対応不要。
- **`shadow_action`のブラスト半径（Opusレビューで調査済み、4箇所のみ）**:
  値そのものを使うのは`key_pipeline.rs:867,883`（変更先そのもの）だけ。
  `transport.rs:224`と`state/evidence.rs:357`は`.is_some()`のみを見る
  （`state/platform_state.rs:2257`はテストヘルパ）。オーバーライドが
  `None`を書かない設計（上記「`classify_mode_key_ime_action`が`None`を
  返す場合、オーバーライドは書かない」）である限り`is_some()`側の
  判定は不変なので、影響は`kp_stage_shadow_ime_toggle`に閉じる。
- **ADR内の古い記述の整理**: 「アーキテクチャ上の制約（Phase 2の起点）」
  節（26-92行台）はPhase 2 v1の誤った前提
  （「Hiragana/Katakana等には安全な自動反映手段が構造的に存在しない」）
  を断定形のまま残しており、上から読むと最初に誤結論に出会う構成に
  なっている。実装時に「※以下はv1の前提であり、後述のとおり誤りと
  判明した」という前置きを追加するか、「Phase 2の撤回」節へ統合する。
  同じ主張が`gji_charset_autodetect.rs`の`ModeKeyCandidate`/
  `has_delegate_to_open_axis_support`のdocコメント（Phase 1実装時点の
  もの、B1対策で警告関数を復活させる際に手を入れる箇所と重なる）にも
  残っているため、実装時に併せて書き換える。
- **【2026-09-05、実機検証済み】** 実機ログ（dragonflyg4、`RUST_LOG=debug`、
  `develop` HEAD、Phase 2/3実装前のコード）で
  `[shadow-toggle] intent 昇格: vk=0xF2 ... kind=PhysicalImeKey`
  が実際に出力されることを確認した。IME OFFの状態で物理ひらがなキーを
  単独タップした結果:
  ```
  [hook] IME-mode vk=0xF2 down self_injected=false injected=false scan=0x70 extra=0x0
  [shadow-toggle] intent 昇格: vk=0xF2 scan=0x70 action=TurnOn kind=PhysicalImeKey injected=false false→true
  ```
  期待どおり`injected=false`・`kind=PhysicalImeKey`で昇格し、belief が
  正しくfalse→trueへ遷移した。`vk=0xF1`（物理カタカナ、Shift+かな
  キー）でも同様に`action=TurnOn`で正しく発火することを確認済み
  （両VKともbelief上は「IMEを開く」効果のみで、ひらがな/カタカナの
  変換モード自体はGJI側の解釈に委ねられる——後述の観察参照）。
  Opusレビューがコード読解のみで導いた結論
  （「grace期間中の誤答は物理VK_DBE_HIRAGANA/KATAKANAに関しては
  構造的に起こらない」）が実機でも裏付けられ、**実装着手の前提条件は
  満たされた**。
- **【2026-09-05、実機検証済み】BUG-52非再発の裏付け**: 物理カタカナ
  キー（Shift+かなキー）を押下してもテキスト入力欄に文字が漏れない
  ことを確認済み（ゆっくり確実に操作しても再現性は同じ）。Phase 3設計
  のS5分析（`shadow_toggled=false`にしてもBUG-52対策
  `is_dbe_mode_key_down`によるSuppressは独立して効き続ける）の前提が
  実機でも成立していることの裏付けになる。
- **【2026-09-05、実機検証で新規発見・別件、Phase 2/3のスコープ外】**
  Shift+かなキー（物理カタカナVK）を押しても、GJI側の変換モードが
  カタカナではなくひらがなになる現象を確認した（ゆっくり確実に操作
  しても再現）。ログ上は物理ドライバがShift有無を正しく`vk=0xF1`
  （カタカナ）/`vk=0xF2`（ひらがな）に区別して送出できているため、
  awase側のVK分類の問題ではなさそうだが、原因未特定。**この現象は
  shadow-toggle機構（IME開閉のON/OFF軸だけを扱う）の対象外**——
  ひらがな/カタカナの変換モード選択はGJI自身が受け取ったVKをどう
  解釈するかの問題であり、Phase 2/3のどの設計判断にも影響しない。
  別途調査するかは未定、`docs/known-bugs.md`に観察として記録するに
  留める。
- BUG-115報告者の症状再現時、実際に`engine_on = ["VK_DBE_HIRAGANA"]`
  （awase側の手動config）と`shadow_action`（GJI/MS-IME非依存の静的
  機構）のどちらが「かなキーで正しく動く」結果を生んでいたかは、
  報告者本人の環境では引き続き未検証だが、**上記の実機検証により
  「`engine_on`にHiraganaを含めなくても、既定設定の`engine_on`
  （`Ctrl+Shift+変換`のみ）のままshadow_action単体でIME OFF→ONが
  正しく起きる」ことを別環境（プロジェクトオーナー機、dragonflyg4）で
  確認済み**——shadow_actionだけで十分という結論の一般性を支持する
  傍証が得られた。`docs/known-bugs.md` BUG-115の記述を参照。

### Phase 3の回帰テスト計画（実装時、コアクレート`src/engine/`側）

- `resolve_pending_thumb_as_single`のHiragana/Katakana分岐に対して、
  Henkan/Muhenkanの既存テスト（delegate発火/非発火の境界）と同型の
  ケースを追加する: (1) 対応フィールドが`Some`かつ`injected=false`で
  単独タップ確定 → `ime_open_requested`が正しい値で発火する、(2) 同じ
  条件で`injected=true` → 発火しない（BUG-14ガード）、(3) チョードとして
  解決された場合 → そもそも`resolve_pending_thumb_as_single`に到達
  しない（既存のチョード判定テストで間接的にカバーされる想定、新規
  追加は不要か確認する）、(4) 対応フィールドが`None`（GJI側が`On`/
  `None`と判定、またはPhase 3自体が未配線）→ 発火せず既定のSuppress/
  Passthrough判定に落ちる。
- `ClassifiedEvent`/`PendingThumbData`への`injected`フィールド追加は
  破壊的変更になりうる——両型を直接構築しているテストヘルパ
  （`confirm_policy.rs`のテスト関数群、`fsm_types.rs`内のテスト等）を
  `cargo check --lib`で洗い出し、全て更新する。**加えて
  `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests
  --lib`も実行し、Windows側（`#[cfg(windows)]`配下）の構築点も
  取りこぼさないこと**（Opus2ラウンド目レビューP8指摘、CLAUDE.md記載の
  確認方法——`cargo check --lib`だけではWindows専用テストの構築点が
  検出されない）。
- `InputTracker::classify`の変更（`event.injected`を`ClassifiedEvent.
  injected`へコピー）に対する直接のユニットテストを1本追加する。
- **P1対策（所有権ゲート）の回帰テストを追加する**（Opus2ラウンド目
  レビューP8指摘）: 「Phase 3が所有するキーではshadow-toggleの昇格が
  起きない」ことを固定するテストが必要。このゲート自体は
  `kp_stage_shadow_ime_toggle`（`#[cfg(windows)]`配下）にあるため、
  CLAUDE.mdが警告するとおりLinuxのテストバイナリには存在しない。
  判定ロジック（「このVKのopen軸をFSMが所有するか」を
  `hiragana_delegate_to_open_axis`/`katakana_delegate_to_open_axis`の
  Some/Noneから決める部分）を純粋関数に切り出してLinuxでテストする
  か、`windows-build` CI専用テストとして割り切るかを実装前に決める。
- **Q2対策の回帰テスト**: 「owned キー（親指キー×delegate armed）でも
  Eisu救済（`eisu_reset_on_turn_on_while_open`/`eisu_reset_on_ime_on`）
  と半角英数復帰（`kp_restore_kana_from_half_width`）は従来どおり
  実行される（belief書き込みとactuationだけがスキップされる）」ことを
  固定するテストを追加する。`tests/architecture_guard.rs::
  user_ime_on_paths_are_paired_with_eisu_reset`が新しい経路×救済の
  対応関係を正しく検知することも確認する。

## 却下した代替案

- **`ime_detect`（probe）への自動追加**: ADR-092:1136-1145で既に撤回済み
  の「毎打鍵ごとに反転する」致命的回帰と同じ罠を踏む（`kp_stage_shadow_
  ime_toggle`がKeyDownのたびに無条件で`write_sync_key`するため、
  チョードキーとして頻繁に押される親指キーでは暴走する）。不採用。
- **`ModeKeyCandidate`ごとに個別の`Option<ShadowImeAction>`フィールドを
  8個追加する**: `muhenkan_delegate_to_open_axis`/
  `henkan_delegate_to_open_axis`と同じパターンをキー数分増殖させる案。
  却下理由: NICOLAは物理的に2スロットしか存在しないため、キーの「名前」
  ではなく「位置（左/右）」でフィールドを持つ方が状態数が少なく
  （8個→2個）、かつ「同時に2つ以上のキーが親指キーになる」という
  そもそも起こり得ない状態を型で表現しなくて済む。
  ※この判断軸自体（v1、`nicola_fsm`側にdelegateを持つ設計）は上記の
  「Phase 2の撤回」により不採用となったため、この比較は歴史的記録として
  残す。仮に将来再びFSM側で汎用delegateを検討する機会があれば、
  8個案・2スロット案のどちらでもなく、**「スロットにVKとactionを同梱する」
  案**（`Option<(VkCode, ShadowImeAction)>`、Opusレビュー指摘）が
  stalenessも状態数増加も避けられる上位互換であることを先に検討すること。
  **注（2026-09-05、Phase 3追記）**: Phase 3は結果として「`ModeKeyCandidate`
  ごとに個別フィールドを追加する」という、ここで却下した8個案と**同じ
  形の**設計（専用フィールドの増殖）を採用している。矛盾に見えるが
  対象が2個（Hiragana/Katakana）に絞られており、却下理由だった
  「8個→2個の方が状態数が少ない」という比較の土俵自体が変わっている
  （Henkan/Muhenkan用の既存2個 + 今回の2個 = 合計4個の専用フィールド
  であり、「同時に2つ以上のキーが親指キーになる」という型で防ぎたかった
  不可能状態も、対象4キーがそれぞれ独立した専用フィールドである限り
  発生しない）。また対象VKが固定である点は「スコープ確定」節と同じ
  理由でstaleness問題も生まない。8キー全部を対象にした汎用スロット化
  だけが却下されたのであり、「特定2キーへの専用フィールド追加」自体は
  最初から却下対象ではなかった。

## 関連

- [ADR-092](092-external-key-semantics-absorption-and-thumb-key-restructure.md)
  決定D Step4b（本ADRが汎用化する対象）。
- [docs/known-bugs.md](../known-bugs.md) BUG-115（症状・調査経緯の詳細、
  Phase 1の実装状況）。
