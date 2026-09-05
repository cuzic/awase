# ADR-092: 外部ソース由来のキー意味論の吸収と、親指キー設定群の再編

## ステータス

**Step1・Step2・Step4a・Step4b・Step4c・Step6 実装済み（2026-08-15、Opus
コードレビューを経て確定）。** Step3・Step5 は今回のセッションでも見送り
（下記「Step3/Step5見送りの最終判断」参照）。

**Step4c の前提の一部訂正（2026-09-05、BUG-115）**: 下記「GJI は親指キーに
IME ON/OFF を割り当てる手段を持たない」という結論は、`custom_keymap_table`
（field 42）経由の**ユーザーが変更可能なキーマップ**に関しては実データでも
裏付けられたが、`overlay_keymaps`（`config.proto` field 68、`session_keymap`
とは独立の repeated フィールド）を通じて無変換→IMEOff・変換→IMEOnを
（`session_keymap`の値に関わらず）重ね掛けする
`OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF` という**別経路**の存在を見落として
いた。加えて、Step4c 実装の `session_keymap` フィールド番号自体が22（誤り、
実際は41）になっていたバグも発見・修正した。Step4b
（無変換/変換 delegate-to-open-axis）は、その後 GJI 側にも対称配線した
（overlay/ATOKプリセット/CUSTOMキーマップのliteralトークンの3情報源、
`Toggle`のみopt-inゲート）。詳細と実装状況は
[docs/known-bugs.md BUG-115](../known-bugs.md) 参照。

**Step4（決定4 上段・肩代わり本体）の実装状況（追記、2026-08-15）**:
- **Step4a（Ctrl+Space/Shift+Space トグル）**: 実機（dragonflyg4）で
  `KeyAssignmentCtrlSpace`/`KeyAssignmentShiftSpace` がトグル（値`2`）
  以外を取り得ないと確認済み（「キーとタッチのカスタマイズ」に個別
  オン/オフの選択肢が無い、ユーザー確認）。このため当初懸念していた
  「個別オン/オフ値の解釈」は不要と判明し、`KeysConfig.ime_toggle`
  （方向不定コンボの新規リスト）と`SpecialKeyMatch::ImeToggle`のみで
  完結した。
- **Step4b（無変換/変換 delegate-to-open-axis）**: 当初案の
  `UserIntentSource::SyncKey` witness登録方式はOpusレビューで
  「か」等の通常打鍵のたびにIMEがON/OFFする致命的回帰を招くと判明し
  撤回、ワンショットチャネル方式（`ime_open_requested`）へ全面設計変更。
  詳細は決定D Step4bの「実装セッションでの設計変更・発見」を参照。
- **Step4c（GJI config1.db 側の宣言読み取り）**: 当初案は「無変換/変換/
  Ctrl+Space/Shift+Space に該当するVKがGJI側でon/off/toggleに含まれて
  いれば」という**特定4キーとの一致判定**を想定していたが、実装時に
  `awase-gji-config::keymap::mozc_key_to_vk_name`の出力範囲がF1-F24と
  `Kanji`/`Hankaku-Zenkaku`/`ON`/`OFF`/`Eisu`の4エイリアスのみに
  限定されており、無変換/変換のVK名がそもそも出力され得ないと判明した
  （GJIは親指キーにIME ON/OFFを割り当てる手段を持たない）。このため
  「特定4キーとの一致」ではなく、**GJIが宣言する任意のVKのうち安全範囲
  （F15-F24、ADR-091の`is_in_safe_autodetect_range`を再利用）内のものを
  無条件でon/off/toggle自動候補として採用する**方式に一般化した——
  Step4aがCtrl+Space/Shift+Space以外の任意コンボも許容する設計である
  こととも整合する。`VK_KANJI`/`VK_IME_ON`/`VK_IME_OFF`/
  `VK_DBE_ALPHANUMERIC`（4エイリアス）はBUG-14（注入イベント衝突
  リスク）を理由に安全範囲から機械的に除外される。GJI離脱時、
  `ime_on_auto`/`ime_off_auto`/`ime_toggle_auto`を全て解除する。

**Step4c実装への2巡目Opusコードレビュー（2026-08-15、実機テストプローブで
実証）で判明した4件のmust-fixを反映済み**:
1. `apply_ime_open_request`が`prev_activation`を推進する経路
   （`ime_set_open_effects`、`build_ime_set_open_decision`と共有）を経由
   せず直接`push_effect`していたため、確定した単独タップによる
   `SetOpen`の**次の**打鍵で`ActivationSync`起点の重複`SetOpen`+
   不要な`EngineStateChanged`が再発火する回帰があった。共有ヘルパーへ
   統合して修正。
2. `EngineCommand::ToggleEngine`/`SwapLayout`（`on_command`経由、内部の
   flushが保留中の親指キーを`ComposingHint::Trusted`で強制的に単独タップ
   確定させうる）が`ime_open_requested`を消費していなかったため、
   トレイ操作等の無関係な外部イベントで強制解決された「単独タップ」が
   無関係な次の打鍵でスプリアスな`SetOpen`を発火させていた。両アームで
   `discard_ime_open_request`により明示的に捨てるよう修正（適用ではなく
   破棄——「単独タップ=IME切替意図」という解釈自体が推定であり、外部
   イベントによる強制解決はその推定をさらに弱めるため）。
3. GJI離脱時に`ime_toggle_auto`を解除しない当初の判断は、その根拠
   （「GJI再突入時に必ず自分の現在値で上書きする」）が誤りだった——
   `set_gji_ime_on_off_toggle_auto_keys`への到達経路には複数の早期return
   があり必ずしも上書きされず、GJI→(MS-IMEでもGJIでもない状態)への
   遷移では誰も解除しない欠落があった。`message_handlers::
   sync_ime_kind_from_observation`のGJI同期をMS-IME同期より**先**に
   呼ぶ順序へ変更し、GJI離脱時に3リスト全て解除するよう修正（順序を
   変えたことで、GJI→MS-IME遷移時はGJI離脱の解除の直後にMS-IME側が
   新しい値で上書きする正しい順序になる）。
4. Step4cのIME ON/OFF/トグルキー自動検出が
   `muhenkan_dedicated_fn_key_is_manual()`（専用Fnキーの手動設定判定、
   本来Step4cとは無関係）の早期returnより後に置かれており、専用Fnキーを
   手動設定しているユーザーはStep4cの機能を丸ごと失っていた。
   手動優先ガードを専用Fnキー検出のみに限定するよう構造を修正。

加えて2件のnice-to-haveも反映: `ime_on_auto`/`ime_toggle_auto`の
マッチ判定に`event.injected`（合成イベント）除外を追加（BUG-14と同種の
リスクへの予防）。手動優先を検証するテストのうち手動/自動に同じキーを
割り当てていた1件（優先順位が逆転していても観測上の差が出ない検証
不能な設計だった）を、異なるキーを使う設計に修正し、`ime_off`側の
欠落していた対称テストと変換（henkan）側のdelegate_to_open_axisテストも
追加。

**Step3/Step5見送りの最終判断（2026-08-15）**: Step0調査の結果
`899b416f`以降BUG-50再現記録なしと確認できたため、Step3
（engine非活性時バイパス経路の抑止）は動機を欠くと判断し見送った。
Step5はStep3完了が前提のため連動して見送り。いずれも「消費者のいない
機構を先回りして作らない」という本ADR全体の原則（決定D Step2の
`DelegateToOpenAxis` variant見送り等と同じ判断軸）に基づく——Opus
による実装タスクリストレビューでも独立に同じ結論（動機の再確認なしに
Step3/5へ着手すべきでない）が示された。

**実装済みStepの要約**:
- **Step1**: `engine_on_ime_key`/`engine_off_ime_key`の既定値をNoneへ変更
  （`src/config.rs`）。`AppConfig::save`が全フィールドを明示出力するため、
  設定GUIで一度でも保存済みの既存ユーザーには効かない（新規/config.toml
  未生成ユーザーのみ影響）。**実機（dragonflyg4）でのEngine ON/OFFトグル時の
  IME挙動確認はまだ未実施**（実装・テストは完了、実機確認のみ残タスク）。
- **Step2**: 決定Bの`ThumbKeySoloTapGuard`→`ModeKeyConfig`/`TextKeyConfig`
  再設計を実装。**本文からの意図的な逸脱**: `#[serde(alias=...)]`による
  config.tomlフィールド名の直接移行は行わず、`GeneralConfig`の8個のflat
  boolフィールドは無変更のまま、`ModeKeyConfig::from_legacy_bools()`で
  都度変換するブリッジ方式を採用した（自己評価で「serde後方互換の具体形は
  未検証」と挙げていたリスクを、config.toml形式を変えずに済むこの方式で
  回避）。専用Fnキー（`muhenkan_solo_tap_dedicated_fn_key`、ADR-091 F21
  自動検出）は`ModeKeyConfig`に統合せず独立フィールドのまま維持
  （config reload時に自動検出値が上書き消去される回帰を避けるため）。
  `SoloTapAction`の4つ目のvariant`DelegateToOpenAxis`（Step4専用）は
  今回追加していない（消費者のいないvariantを作らない原則、Step4着手時に
  追加すること）。
- **Step6**: `tab_ime_detect`廃止・`tab_keys`内`egui::CollapsingHeader`
  「上級者向け設定」への統合、決定E round4の対称命名（「awase → IME
  ON/OFFキー」/「IME → awase ON/OFFキー」）、決定A-5残タスク（半角/全角
  オプション追加）を実装。`SettingSource`バッジ表示（決定E-1〜E-5）は
  MS-IME/GJIレジストリ由来のAutoDetectedソースがStep4未実装のため対象外
  のまま（GUI側は別プロセスのためどのみち実行中Runtimeの自動検出値を
  読み戻す経路が無いことが実装時に判明、専用Fnキーの自動検出値についても
  同様）。

Opusコードレビュー（2巡目）で追加指摘された8件（is_japanese_ime belief
グローバル書き換えの波及範囲・ドキュメント追随漏れ・`Default`derive の
罠・`muhenkan_solo_tap_is_passthrough`の二重管理等）はいずれも本文・
コードへ反映済み。詳細は`feat/adr092-step1-2-6-key-semantics`ブランチの
コミット履歴参照。

以下は設計時点（実装着手前、2026-08-15）の記述。Opus エージェント（設計提案、2026-08-15）→ 別セッションの
Opus エージェント（批判的レビュー、round1: **Go with modifications**）→ ユーザーによる
スコープ拡大の判断（round2〜round4、決定A-3/A-4/A-5・決定E の複数回の修正）→
**別セッションの Opus エージェントによる2巡目レビュー（2026-08-15、must-fix 6件
M1〜M6を指摘、本文に反映済み）**を経て、本文を反映した版。ADR-091 の追補ではなく
独立した ADR とする（ADR-091 は charset 軸、本 ADR は主に open 軸への「吸収」と
設定構造の再編が主題であり、関心の分離のため）。

**2巡目レビューの要点**: round1 以降に追加された決定（A-4/A-5/決定Eの4回の
改訂）は round1 と同水準の検証を経ておらず、**特に決定A-5は round1 が指摘した
「既存機構を見落として同じ意味の型/機構を作ろうとする」誤りの再発だった**
（`vk.rs::ImeKeyKind::shadow_effect()` が同じ判定を既に実装済み、追加すると
安全性が後退する）。この指摘を受け決定A-5は大幅に縮小し、決定C/E/A-4にも
修正を加えた。詳細は各節の「2巡目レビュー」表記を参照。

**round1 レビューで、当初案の中心的主張（Step 1a が BUG-50 に効く／Step 1b は
`PhysicalImeKey` を素直に使える）がいずれも事実誤認だったと判明した。** 詳細は各節、
特に決定D を参照。

**round2（ユーザー判断）: 決定4 上段（肩代わり本体）を No-go から着手対象へ格上げする。**
対象を無変換キーのみから、MS-IME の「キーとタッチのカスタマイズ」が扱う4キー
（無変換・変換・Ctrl+Space・Shift+Space、レジストリの仕組み自体がこの4つに
限定されている）と、**GJI の config1.db が宣言する任意のキー**（VKの種類に
よる安全フィルタあり、決定A-3）へ拡大し、宣言される意味に基づいて awase 側の
belief を経由して冪等キー（`VK_IME_ON`/`VK_IME_OFF`）へ変換・送信する
actuation を行う、というユーザーの明示判断による。round1 が挙げた
`architecture_guard`/`IntentWitness` との衝突（懸念4-1）は、**`UserIntentSource::SyncKey`
という既存の別 witness 経路**（`PhysicalImeKey` とは別物、「設定された同期キー
(Shift+Space 等)」がまさに doc の例示）を使うことで解消できると判明した
（決定A参照）。宣言が内部で矛盾する「揺れ」を検出した場合は、警告 + 素の
パススルーへフォールバックする（決定A-3）。詳細は各節を参照。

## 背景

### この ADR に至った経緯

2026-08-15、ADR-091 Phase 1（専用 Fn キー変換によるGJI charset軸制御）の実機検証中、
ユーザーから「無変換・ひらがな・カタカナ等のキーは、MS-IME/GJI 自身の設定次第で
『IME オン』の意味を持つことも『IME モード変更』の意味を持つこともあるのに、awase の
実装はこれを一切見ず画一的に扱っている」という指摘があった。

これは新しい発見ではない。**2026-07-06 のセッションで既に同種の分析（「MS-IME 二重
オーナー問題」）を行い、「レジストリは読み取り専用で検出し、検出した意味に応じて
awase 側の対応するパイプラインへ変換する（吸収方式）」という恒久対応方針に合意していた
が、当時は分析のみで実装に着手しなかった。** 本 ADR はこの積み残しを正式に設計として
確定させるものである。

### 現状のコード（2026-08-15、codex CLI による read-only 調査で確認）

- `crates/awase-windows/src/msime_key_assignment.rs` は
  `HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME` の `IsKeyAssignmentEnabled`/
  `KeyAssignmentMuhenkan`/`KeyAssignmentHenkan` を読み取り可能（`:140`
  `read_from_registry()`、現在 `private`）。ただし使用箇所は
  `check_and_warn()`（`:95`）による警告ポップアップ表示に閉じており、
  awase 自身のディスパッチ判断には一切配線されていない。**現在の読み取りは
  `read_dword(...) == Some(1)` で DWORD を bool に潰しており、`0`/`1` 以外の
  未知値もすべて「既定＝かな切替」として扱われる（round1 レビュー懸念1-3）。
  ディスパッチの根拠に昇格させる場合はこの潰し方を直す必要がある（決定A参照）。**
- `src/engine/nicola_fsm.rs:1156 resolve_pending_thumb_as_single` が無変換/変換の
  単独タップ確定時の分岐点。順序は modifier(`:1165`) → 専用Fnキー変換(`:1178`、
  ADR-091) → 無変換 `always_suppress`(`:1192`) → 変換 `always_suppress`(`:1200`) →
  composing guard(`:1207-1224`) → 素の VK 送出(`:1226`)。**いずれの分岐も、
  MS-IME/GJI がそのキーに実際どんな意味を与えているかを一切参照しない。**
- **この関数は engine が active な場合にしか呼ばれない。** `engine.rs:303-310`
  （Phase 2）が `compute_active(ctx)` が false（`Inactive(ImeOff)`/
  `NotJapaneseIme`/`UserDisabled`/`NotRomajiInput` 等）のとき `pass_through()`
  を返して Phase 3（`NicolaFsm`、`resolve_pending_thumb_as_single` を含む）へ
  到達しない。**つまり IME OFF 中や非対応 IME 使用中の物理無変換単独タップは、
  `always_suppress=true` であっても本節の分岐を一切通らず生 VK のまま OS へ届く
  （round1 レビュー懸念3-2）。** この構造的な穴は本 ADR の Step 1a/1b が扱う
  範囲の**外側**にある（決定D参照）。
- **FSM は IME への副作用（open/close）を出す経路を持たない。**
  `src/engine/fsm_types.rs:133 ParseAction` の全 variant は
  `actions: SmallVec<[KeyAction; 2]>` のみを持ち、`KeyAction` は OS へ送る
  キーの列挙でしかない。`fsm_adapter.rs:163 response_to_effects` が唯一の
  出口で `Effect::Input(InputEffect::SendKeys)` に翻訳するだけである。
  `ImeEffect::SetOpen { open, origin }`（`src/engine/decision.rs:46`）を
  発行しているのは `engine.rs::apply_special_key_match`
  （`SpecialKeyMatch::ImeOn/ImeOff` 等）という Engine 層の別経路のみで、
  無変換単独タップの確定処理からは到達できない。
- **`UserIntentSource::PhysicalImeKey`（`state/ime_event.rs:77`）は無変換から
  合法的に発行できない。** `evidence.rs:350-357 from_physical()` は
  `e.ime_relevance.shadow_action.is_some()` を要求し、これは
  `ImeKeyKind::from_vk`（`crates/awase-windows/src/vk.rs:118-132`）由来
  だが**無変換/変換
  （0x1C/0x1D）はここに含まれない**。含めると `is_ime_control` 系の分類が
  発火し、親指シフト自体が壊れる。さらに
  `architecture_guard.rs:878-890` が「`SyncKey`/`PhysicalImeKey` はリテラルで
  名乗ってはいけない、新しい variant を追加する場合は witness に載せられる
  外部事実があるかをまず検討すること」（ADR-089 §2.2、2026-07-05 の
  「AI の近道防止」目的で設置）と明示的にガードしている（round1 レビュー
  懸念4-1）。**本 ADR の実装コストの本体は、単に effect 経路が無いことでは
  なく、この意図的な型ガードをどう扱うかにある。**
- **`UserIntentSource::SyncKey`（round2、`state/ime_event.rs:73-75`）は
  `PhysicalImeKey` とは別の witness variant で、無変換/変換からも合法的に
  発行できる。** doc コメントが「設定された同期キー (Shift+Space 等)」と
  明記しており、まさに「ハードコードされた物理 IME キーではなく、設定
  次第で意味が変わるキー」を想定した既存の区分である。
  `IntentWitness::from_sync_key(e)`（`evidence.rs:358-362`）は
  `e.ime_relevance.sync_direction.is_some()` だけを要求し、`shadow_action`
  （`PhysicalImeKey` 用、`ImeKeyKind::from_vk` 限定）を要求しない。
  `sync_direction` は `FocusTracker::enrich_ime_relevance`
  （`runtime/focus_tracker.rs:47-64`）が `ImeDetectConfig` の
  `toggle`/`on`/`off`（`VkCode` のリスト、任意の VK を追加可能）に基づいて
  設定する——**このリストに無変換/変換の `VkCode` を追加するだけで、
  `architecture_guard.rs:878-890` の型ガードに触れずに合法な witness が
  得られる**（round1 懸念4-1 は解消）。
  `ImeEvent::UserImeSetIntent { target, source }`
  （`state/ime_model.rs:1096-1099` 等で使用例あり）が `desired_open` を
  直接更新する経路であることもテストから確認済み。
- **ただし `SyncKey`/`ImeDetectConfig` は、元々「OS が自然に処理する
  キーを awase が後から観測する」（Shift+Space のような、awase が抑止
  しなければ IME に届いて OS 側で処理される想定の）ためのものであり、
  「awase 自身が能動的に別のキーへ変換して送信する」（F21 と同型の
  actuation）用途にそのまま使われている実績はまだない。** 決定Aで
  この用途拡張の設計を示す。
- awase には既に「このキーが押されたら IME はこうなったと解釈する」という
  **観測（belief 追随）用の既存機構**が2つある。決定Aで再利用する:
  - `ShadowImeAction { TurnOn, TurnOff, Toggle }`（`src/types.rs:143`）
  - `ImeDetectConfig { toggle, on, off }`（`src/config.rs:378-385`）。
    `awase-gji-config::keymap.rs:70-73` の doc も
    「`ImeDetectConfig.toggle/on/off` へそのまま反映できる形」と明記している。
  - GJI 側の宣言読み取り `GjiImeKeys { on, off, toggle }`
    （`crates/awase-gji-config/src/keymap.rs:74-82`）も同じ3値の語彙で
    既に実装済み・型がある。**この読み取り機構は現在 `awase-windows` から
    `read_gji_mode_keys` 経由でしか参照されておらず、IME ON/OFF 抽出
    （`read_gji_ime_keys`）自体には消費者がいない**（round1 レビュー
    懸念5-1）。
- ADR-091 Phase 1 は決定4（MS-IME 肩代わり）以外ほぼ実装済み
  （`gji_charset_autodetect.rs`/`gji_charset_write.rs`/`gji_charset_popup.rs`、
  `runtime/message_handlers.rs:381 sync_ime_kind_from_observation` が
  IME 種別確定の単一の合流点として機能）。
- `dbe_mode_key_policy` は **transport 層**（`runtime/transport.rs:177-186`）
  で全 DBE キーを既定 Suppress にする**強い**保証（engine の活性状態に
  依存しない）。一方 `muhenkan_solo_tap_always_suppress` は **engine 活性下の
  単独タップ確定という限定条件でのみ効く弱い**保証である。両者は「モード
  キーの抑止」という同じ言葉で語られがちだが、**保証の強さが違う**
  （round1 レビュー懸念5-2）。

### round2: 対象4キーへの拡大と、実装経路が2種類に分かれる理由

MS-IME の「キーとタッチのカスタマイズ」は、無変換・変換・**Ctrl+Space**・
**Shift+Space** の4キーに機能を割り当てられる（Web調査で確認、
[エンジニアの備忘録: Windows 11のMicrosoft IMEで日本語入力をCtrl+Spaceで切替える方法](https://engrmemo.jp/win/msime-ctrl-space/)
等、複数の記事が GUI 操作を説明している）。ユーザーの判断により、本 ADR の
対象をこの4キー全てへ拡大する。

**Ctrl+Space・Shift+Space に対応するレジストリ値の名前は、Web調査
（2026-08-15）では確認できなかったが、直後にユーザーが実機
（dragonflyg4、clipwire 経由）で4キー全てを「IME ON/OFF」に割り当てて
レジストリを直接確認し、確定した:**

```
KeyAssignmentMuhenkan   : 2
KeyAssignmentHenkan     : 2
KeyAssignmentCtrlSpace  : 2
KeyAssignmentShiftSpace : 2
```

`KeyAssignmentCtrlSpace`/`KeyAssignmentShiftSpace` という名前自体が
実在すると確定した（`IsKeyAssignmentEnabled: 1` も同時に確認済み）。

**値 `2` の解釈（重要な訂正）**: 2026-07-06 の先行調査では
`KeyAssignmentMuhenkan == 1` が「IMEオフ」、`KeyAssignmentHenkan == 1` が
「IMEオン」だった。今回ユーザーが選んだのは個別の「IME-オン」/「IME-オフ」
ではなく**「IME ON/OFF」という単一のトグル選択肢**であり、これが4キー
共通で値 `2` にエンコードされている。**つまりこの列挙値は
`ShadowImeAction::Toggle` にちょうど対応する、実在する第3の値**である
（決定Aが `ShadowImeAction{TurnOn,TurnOff,Toggle}` を再利用する設計を
裏付ける、実測による確認）。

**未確認のまま残る点**: Ctrl+Space/Shift+Space について「IME-オンのみ」
「IME-オフのみ」を個別に選んだ場合の数値（無変換/変換の「1」に相当する
値）は今回試していない。個別方向の割り当てにも対応する場合は追加で実機
確認が必要。またキー毎に列挙値の意味が同一かどうか（例: 全キー共通で
「1=オン, 2=オフ, 3=トグル」のような統一エンコーディングか、キーごとに
選択肢の順序が異なるか）も未確認。**このため、Ctrl+Space/Shift+Space の
実装は「トグル（値2）」のケースのみを対象に着手し、個別オン/オフの
対応は別途実機確認してから追加する。**

**4キーは、awase 内での実装経路が2種類に分かれる**（実装上重要な区別）:

- **Ctrl+Space / Shift+Space**（修飾キー+キーの組み合わせ）: NICOLA の
  親指シフト方式とは無関係な、通常の「コンボキー」である。awase には
  既に `KeysConfig.ime_on`/`ime_off`（`src/config.rs`、`Vec<String>` で
  `"Ctrl+Shift+F12"` のようなコンボ文字列を受け付ける、
  `engine_toggle_hotkey` と同形式）という、コンボキー→
  `apply_special_key_match`（`engine.rs:649-656`）→
  `build_ime_set_open_decision` という**既に完成した actuation 経路**が
  ある。**MS-IME レジストリが「Ctrl+Space=IMEオン」等を宣言していて、
  かつユーザーが `keys.ime_on`/`ime_off` を明示設定していなければ、
  この文字列をリストへ自動的に追加するだけで済む可能性が高い**
  （新しい dispatch 点も新しい witness も不要、決定C の優先順位規則
  「明示 > 自動」で自動追加分は明示設定に譲る）。
- **無変換 / 変換**（修飾キーなしの単独タップ）: これは NICOLA の親指
  シフト方式におけるチョード（同時打鍵）検出にも使われる特別なキーで
  あり、`match_special_keys`（Phase 1/2、Engine層）が生の keydown 段階
  で先取りしてしまうと、チョード検出（Phase 3、`NicolaFsm`）に到達
  できず親指シフト自体が壊れる。**したがって `keys.ime_on`/`ime_off`
  への単純な追加はできない**——「背景」節の通り、単独タップ確定後の
  `resolve_pending_thumb_as_single`（Phase 3 内部）からのみ安全に
  分岐できる。この差が決定Aで `SyncKey` witness を新設計する理由。

### 設定の乱立（2026-08-15 の棚卸し調査より）

- 親指キー系 8 個の bool 設定（`src/config.rs` の
  `space_thumb_ignore_composing_guard`/`space_thumb_shift_literal`/
  `muhenkan_solo_tap_ignore_composing_guard`/`muhenkan_solo_tap_always_suppress`/
  `henkan_solo_tap_ignore_composing_guard`/`henkan_solo_tap_always_suppress`/
  `enter_thumb_ignore_composing_guard`/`enter_thumb_shift_literal`）が、
  4 キー分の組み合わせで命名パターン・既定値ともにばらばらである。
- 無変換単独タップだけで3設定（`always_suppress`/`ignore_composing_guard`/
  `dedicated_fn_key`）に分裂し、優先順位はソースコメント頼み。
- `dbe_mode_key_policy`（Suppress/Passthrough）・`conv_mode_policy`
  （Observe/Force）・`engine_on_ime_key`/`engine_off_ime_key`
  （既定 `VK_DBE_DBCSCHAR`/`VK_DBE_SBCSCHAR`）が、同じ「IME 変換モードを
  awase がどう扱うか」という領域に別 enum・別意味論で存在する。
  `engine_on_ime_key` の既定値は ADR-091 決定1（open 軸は
  `VK_IME_ON`/`VK_IME_OFF` で決着済み）より前の機構の残骸であり、
  2026-08-11 の別調査で「複合副作用キー（開く+モード強制を1発で行う
  危険なキー）」に分類済みにもかかわらず既定値として生存している。

### `CharsetSlot` の教訓（踏まえるべき前例）

ADR-091 §3 に記録の通り、awase が belief に基づいて IME 状態を能動的に
判断・絶対指定コマンドを送る `CharsetSlot` という機構が、MS-IME 向け→GJI 向けと
2 度設計・実機検証されたのち、いずれも撤回された。同日には ADR-091 の F21 に
「将来使うかもしれない」という動機で F22-F24 の予備バインドを追加する提案も、
同種の未解決の前提（awase 側の判断ロジックが不在）を抱えていたため即日撤回
している（`5ada7bcd`/`82f6a890`/`bbb350e2` の revert）。**本 ADR はこの轍を
踏まないことを設計の中心的な制約とする。**

**round1 レビューで、本 ADR 自身が最初のドラフトでこの轍の一歩手前まで
行っていたと判明した**（決定Aで独自の新型 `DeclaredOpenRole` を作ろうと
し、既存の `ShadowImeAction`/`ImeDetectConfig`/`GjiImeKeys` という3つの
類似機構を見落としていた）。以下の決定はこの指摘を反映して修正済み。

## 決定

### 決定A: 「キーの意味」の受け皿は、新しい型を作らず既存の3値語彙を再利用する

**当初案からの変更点**: `DeclaredOpenRole { TurnOn, TurnOff }` という新しい
2値 enum を作る案を**撤回する**。理由（round1 レビュー懸念1-1・1-2）:

- `ShadowImeAction { TurnOn, TurnOff, Toggle }`（`src/types.rs:143`）が
  既に存在し、`DeclaredOpenRole` はこれから `Toggle` を1つ削っただけの
  劣化コピーになる。
- GJI 側の宣言は「キー」単位ではなく「(GJI 内部状態, キー) → コマンド」
  であり、`extract_ime_keys` は既にこれを `toggle` を含む3値で分類できる
  （`keymap.rs:21-32`）。GJI をいずれ対象に含めるなら、2値専用の型は
  その時点で作り直しになる。「消費者のいない variant を作らない」
  （決定Aの元の理由2）を守るなら、**逆に「供給元が増えたときに壊れない
  型を最初から使う」方が一貫している。**

改めて `KeySemantics { Open, Charset, Reconvert, Unknown }` のような
網羅的 enum は**依然として作らない**。理由は変わらない: charset 軸には
awase 側の冪等な代替機構が存在せず、`CharsetCycle` のような variant を
作っても行うことは「今まで通り抑止する」以外にない。

**採用する方針**: 新型を追加せず、既存の `ImeDetectConfig`
（`src/config.rs:378-385`、`toggle`/`on`/`off` の3フィールド）の語彙を
そのまま使う。MS-IME レジストリから読めた値は、`ShadowImeAction` の
variant にマップして `ImeDetectConfig` 相当の解釈に渡す。`Toggle` の
消費者は `VK_KANJI` 経路として既に存在するため、型を増やしても
「消費者のいない variant」にはならない。

**レジストリ読み取りの修正**（round1 レビュー懸念1-3）:
`read_from_registry()`（`msime_key_assignment.rs:141-146`）の
`== Some(1)` という潰し方を、明示的な match へ変更する:

```rust
// 2026-08-15 実機確認（dragonflyg4）: KeyAssignmentMuhenkan=2 は
// 「IME ON/OFF」(トグル)選択時の値。既知の値は Muhenkan={1: IMEオフ
// (2026-07-06確認), 2: トグル(今回確認)}。「IMEオンのみ」に対応する値は
// 未確認のため、確認できるまで安全側(None)にフォールバックする。
match read_dword(w!("KeyAssignmentMuhenkan")) {
    Some(1) => Some(ShadowImeAction::TurnOff),
    Some(2) => Some(ShadowImeAction::Toggle),
    Some(0) | None => None, // 既定(かな切替)・未読み取りとも「宣言なし」
    Some(_) => None,        // 未確認の値は安全側=既存経路へ(推測しない)
}
```

`Some(0)`（既定のかな切替）を `None` として扱う点に注意: これは
「この物理キーには open 軸の宣言が無い」ことを意味するのであって、
「charset 軸の宣言がある」ことを意味しない。charset 軸の扱い（抑止する
か等）は決定Bの `SoloTapAction::Suppress` が引き続き担う。

**キャッシュ規約**: `Runtime` に保持するが **belief ではなく外部設定の
ミラー**である。`sync_ime_kind_from_observation` の IME 種別確定イベント
のみで更新し、IME 種別が変われば必ず解除する
（`gji_charset_autodetect.rs:112` の既存ラッチと同一規約）。ポーリングは
しない。

**round1 レビュー懸念4-3（ミラーの stale 化）への対応**: MS-IME 単独
ユーザーはセッション中に IME 種別確定イベントが再発生しないため、
ユーザーが設定アプリで割当てを変更しても awase はセッション中ずっと
古い宣言を使い続けうる。round2 で actuation まで対象にしたため、この
stale 化は observation より実害が大きくなりうる。設定リロード時の再読み、
または `RegNotifyChangeKeyValue` によるイベント駆動更新を Step 4
（決定D）着手時に組み込むこと。

#### 決定A-2（round2追加）: actuation 経路は2種類——コンボキーは既存機構の自動設定、無変換/変換は新しい witness 経由

「round2: 対象4キーへの拡大と、実装経路が2種類に分かれる理由」（背景節）
の通り、4キーは実装経路が異なる。

**Ctrl+Space / Shift+Space**: 新しいコードパスは不要。MS-IME レジストリの
宣言を読み、`KeysConfig.ime_on`/`ime_off`（既存の `Vec<String>` コンボ
リスト）に対応する文字列（`"Ctrl+Space"`/`"Shift+Space"`）を**まだ
含まれていない場合のみ**追加する。これは `apply_special_key_match`
（`engine.rs:649-656`）→ `build_ime_set_open_decision` という、
`desired_open` の更新と実際の `VK_IME_ON`/`VK_IME_OFF` 送出を両方正しく
行う、既に完成した経路をそのまま使う。決定C R1（明示 > 自動）に従い、
ユーザーが `keys.ime_on`/`ime_off` を既に手動設定している場合は追加
しない。

**無変換 / 変換**: 「背景」節の通り `resolve_pending_thumb_as_single`
（NicolaFsm Phase 3、単独タップ確定後）でのみ安全に分岐できる。この
分岐点から `build_ime_set_open_decision` 相当の処理（`desired_open`
更新 + `VK_IME_ON`/`VK_IME_OFF` 送出）を呼べるようにする実装が必要
（Engine 層と NicolaFsm 層をまたぐ、決定D Step 4 で詳述）。この経路が
生成する `IntentWitness` の `source` には
`UserIntentSource::SyncKey`（`PhysicalImeKey` ではない）を使う——
`ImeDetectConfig.toggle`/`on`/`off` に無変換/変換の `VkCode` を追加し、
`enrich_ime_relevance` に `sync_direction` を設定させることで、
`IntentWitness::from_sync_key()` が合法的に構築できる
（`architecture_guard.rs:878-890` に抵触しない、round1 懸念4-1 の解消）。

#### 決定A-3（round2追加）: GJI の config1.db も同じ宣言ソースとして扱う

**ユーザー判断: MS-IME レジストリだけでなく、GJI の config1.db が
IME ON/OFF 動作を宣言している場合も同様に actuation の対象にする。**

`awase-gji-config::keymap.rs::extract_ime_keys()` は、`custom_keymap_table`
から `GjiImeKeys { on: Vec<String>, off: Vec<String>, toggle: Vec<String> }`
（VK 名のリスト）を既に抽出できる（`STATUSES_WHEN_IME_OFF = ["DirectInput"]`、
`STATUSES_WHEN_IME_ON = ["Precomposition", "Composition", "Conversion",
"Prediction", "Suggestion"]` で status を分類、`command.rs::classify_command`
の `GjiModeCommand::ImeOn`/`ImeOff` で判定）。**この関数は
`read_gji_ime_keys()` として公開済みだが、round1 レビュー懸念5-1が
指摘した通り現在 `awase-windows` 側に消費者が無い。** 本 Step でこの
「読めるのに使われていない」機構に初めて消費者を与える。

**採用する設計**: `on`/`off`/`toggle` の各リストにある VK 名を
`ShadowImeAction::{TurnOn,TurnOff,Toggle}` へ変換し、決定A-2 の下流
（`SoloTapAction::DelegateToOpenAxis` または `keys.ime_on`/`ime_off` への
自動追加）へ**同じ経路として**流し込む。読み取り元（MS-IME レジストリ /
GJI config1.db）はソースが違うだけで、下流の型・優先順位規則
（決定C R1-R4）・witness（`SyncKey`）は共通化する。

**当初案からの訂正（ユーザー指摘）**: 「無変換・変換・Ctrl+Space・
Shift+Space の4キーに限定する」という当初の絞り込みは、**具体的な根拠の
無い恣意的な制限だった**。GJI の config1.db が「キー X は IMEOn」と
宣言していれば、GJI 自身が実際にそう解釈して動く——これは awase の推測
ではなく、GJI が起動している限り実際に起きる外部事実である。awase が
同じ解釈を採用しても、GJI 自身の挙動に awase の理解を合わせるだけで
新しいリスクを生まない。**したがって、GJI 側は特定の4キーに限定せず、
config1.db が実際に宣言している任意の VK を対象にする。**

**安全のための絞り込み（VK の種類による、キー名の列挙ではない）**:
唯一の具体的なリスクは、NICOLA が実際の打鍵（かな入力・チョード検出）
に使っている VK が誤って対象に入ることである。特に BUG-64
（旧 awase 実験由来の残骸バインドが config1.db に残存する既知の事実、
`docs/known-bugs.md`）のように、**ユーザーの意図しない古いバインドが
混入している可能性**があるため、次の3分類でルーティングする:

1. **NICOLA の親指キー役割（`muhenkan_vk`/`henkan_vk`/`space_thumb_vk`/
   `enter_thumb_vk`）に一致する VK**: Step 4b（`resolve_pending_thumb_as_single`、
   Phase 3）へ。この分岐点自体が「確定した単独タップ」にしか発火しない
   ため、対象がこの4役割のどれであっても構造的に安全。
2. **現在ロード中のレイアウト（`.yab`）でかな文字を生成しない VK**
   （Ctrl+Space 等の修飾キー組み合わせ、F-key、他の未使用VK等）:
   Step 4a（`keys.ime_on`/`ime_off` への自動追加）へ。
3. **上記のいずれでもない、現在のレイアウトでかな文字を生成する素の
   VK**（NICOLA が実際に打鍵で使う文字キー）: **不採用**。BUG-64 の
   ような残骸バインドが実際の打鍵キーを誤って捕まえるリスクがあり、
   awase 側で判定材料（そのキーが今チョード検出に使われているか）を
   安定して持てないため、決定C R3（曖昧なら推測しない）に従い無視する。

MS-IME レジストリ側は「キーとタッチのカスタマイズ」機構自体がこの4キー
にしか機能を割り当てられない（Web調査・実機確認で確認済み）ため、
この一般化は不要——決定A-2 のまま4キー固定でよい。**一般化が必要なのは
GJI 側だけ**、というのがユーザー指摘への回答。

**衝突・曖昧さの検出と、その場合の方針（ユーザー追加要望）**: 同じ VK が
`detect_dedicated_fn_key`（ADR-091、`toggle_kana_type` から F21 相当を
検出）と `extract_ime_keys`（本Step、`on`/`off`/`toggle`）の両方に異なる
意味で現れる、または `on`/`off`/`toggle` の複数リストに同じ VK が矛盾して
現れる等、**IME の内部状態（config1.db の宣言）に「揺れ」（一貫しない
定義）が検出された場合**:

1. **起動時（GJI 検出時）に警告を出す**（`msime_key_assignment.rs::check_and_warn`
   と同様のポップアップ、またはログ + 設定画面での表示）。
2. 対象キーは actuation（`DelegateToOpenAxis`）を採用せず、**素の
   パススルー**（`SoloTapAction::Passthrough`）にフォールバックする。
   `Suppress`（安全側の既定）ではなく **`Passthrough`** を選ぶ理由:
   ユーザーが GJI 側で何らかの意図を持ってそのキーを設定した可能性が
   高く（でなければ「揺れ」自体が生じにくい）、awase が黙って抑止すると
   ユーザーが設定した機能を勝手に奪うことになる。パススルーであれば
   GJI/MS-IME 自身の（awase からは一貫して見えなくても GJI 自身に
   とっては一貫しているはずの）ネイティブな解釈にそのまま委ねられる。
   これは新しいリスクの導入ではなく、**既存の`Passthrough`という
   選択肢（ADR-091 §D3.6 の「上級者向けの逃げ道」）を、awase が
   確信を持てない場合の自動フォールバック先として使うだけ**である。
3. この「揺れ検出時のフォールバック」は決定C R4（自動判定は
   `Suppress → DelegateToOpenAxis`/`DedicatedFnKey` の方向にのみ書き
   換えてよい）の**例外**として明記する: 曖昧さを検出した場合に限り、
   `Passthrough` への遷移も自動判定の結果として許容する。

#### 決定A-4（round2追加）: `session_keymap` がCUSTOM以外の場合、Mozc組み込みプリセットの実データで意味を解決する

これまでの決定A-2/A-3は「`session_keymap == CUSTOM`（`custom_keymap_table`
にユーザー独自の割当てがある）」場合のみを対象にしていた。**GJI の
「キー設定」で MS-IME/ATOK/ことえり 等のプリセットを選んでいる場合
（`session_keymap != CUSTOM`）、`custom_keymap_table` は GJI 自身が
参照しない**（`awase-gji-config::SESSION_KEYMAP_CUSTOM` 判定は実装済み、
ADR-091 で確認済み）ため、読み取り元が無く決定A-2/A-3は機能しない。
**Mozc本家の組み込みプリセット定義（ソースコード）を確認し、
プリセットごとに Muhenkan/Henkan の意味を awase 側にハードコードする。**

**2026-08-15、`google/mozc` の `src/data/keymap/{ms-ime,atok,kotoeri}.tsv`
を直接確認した結果（実装セッションで再訂正、下記「取得元コミット」参照）:**

| プリセット | 状態 | Muhenkan | Henkan |
|---|---|---|---|
| **ms-ime.tsv** | DirectInput | (未定義) | `Reconvert` |
| | Precomposition | `CompositionModeSwitchKanaType` | `Reconvert` |
| | Composition | `SwitchKanaType` | `Convert` |
| | Conversion | `SwitchKanaType` | `ConvertNext` |
| **atok.tsv** | DirectInput | `IMEOn` | `IMEOn` |
| | Precomposition | `CancelAndIMEOff` | `CancelAndIMEOff` |
| **kotoeri.tsv** | (Muhenkan/Henkan の定義自体が無い、macOS向けのため) | — | — |

**訂正（実装セッション、2026-08-15）**: 当初表は「DirectInput: Muhenkan/Henkan
とも未定義」としていたが誤りだった。実際には **DirectInput の Henkan は
`Reconvert` が定義済み**（Muhenkanのみ未定義）。またComposition/Conversion
状態の行が表から抜けていた（上表で補完）。ただし本節の結論
（ms-ime.tsvにIMEOn/IMEOffに相当する行が無い＝MS-IMEプリセットはopen軸の
決定A-2/A-3の対象外のまま）は変わらない——Henkanの`Reconvert`はopen軸と
無関係（「解釈」節参照）。

**解釈**:

- **ATOK プリセットは決定A-2/A-3に乗るが、「クリーンなケース」という
  round2時点の評価は不正確だった（2巡目レビュー指摘M6、訂正）。**
  DirectInput（IME OFF 相当）の `IMEOn` は単純な `TurnOn` で問題ない。
  一方 Precomposition の **`CancelAndIMEOff` は「composition 破棄」+
  「IME OFF」の複合コマンド**であり、MS-IME の `CompositionModeSwitchKanaType`
  と同じ「複合副作用」の系統に属する。`extract_ime_keys`
  （`STATUSES_WHEN_IME_ON`、`keymap.rs:26`）は `Composition`/`Conversion`
  も IME-ON 側に含むため、変換中に無変換/変換を押した場合、実際の
  ATOK なら「未確定文字を破棄しつつ IME OFF」になるが、Step 4b が代わりに
  送る `VK_IME_OFF` は IME OFF のみを行い、**未確定文字の破棄（Cancel）を
  再現しない**——ATOK 使用時に awase 経由だと未確定文字の扱いが未定義に
  なる。**採用する軽減策**: `CancelAndIMEOff` の actuation は
  **Precomposition 状態（未入力、Cancel すべき composition が存在しない
  状態）に限定**し、Composition/Conversion 状態（未確定文字が存在しうる
  状態）では `Default`（決定C R3、awase は手を出さずネイティブ処理へ
  委ねる）へフォールバックする。DirectInput の `IMEOn` は複合コマンドで
  はないため、この制限は不要——Muhenkan・Henkan **両方**が対象になるのは
  DirectInput→IMEOn の側のみ確定で、Precomposition→CancelAndIMEOff 側は
  上記の状態限定つきで対象になる。**実機確認事項に追加**（ATOK 環境が
  無いため机上の対策であり、実機で Composition 中の Cancel 挙動を確認
  できるまでは Precomposition 限定を維持すること）。
- **MS-IME プリセットの Muhenkan は決定A-2/A-3の対象外のまま。**
  `CompositionModeSwitchKanaType` は charset 軸の絶対設定コマンドで
  あり、2026-08-11 の危険度分類調査が言う「複合副作用（開く+モード
  強制を1発で行う）」の GJI 版に相当する——単純な `TurnOn` とは言えない。
  これは ADR-091 §D3.2 の専用Fnキー変換（F21）がまさに対象としている
  ケースであり、**本 ADR では手を出さず ADR-091 の既存フローに委ねる**。
  Henkan（`Reconvert`）は open 軸と無関係なので対象外。
- **ことえりプリセットは対象外**（Muhenkan/Henkan の定義自体が無い。
  Windows で JIS キーボードを使いつつことえりプリセットを選ぶという
  非典型構成でも、単に何も起きない——安全側）。

**実装方針**: `session_keymap` を読み、`CUSTOM` ならこれまで通り
`custom_keymap_table` を解析（決定A-3）。`ATOK` なら上記のハードコード
テーブルで Muhenkan/Henkan を `ShadowImeAction` として即座に決定
（config1.db の追加解析は不要）。`MSIME`/`KOTOERI`/その他は「対象外」
として決定Cの `Default` へフォールバックする。**このテーブル自体は
Mozc の公開ソースという外部の事実の記録であり、awase が新しい belief
を持つことにはならない**（ADR-091 §1.4 の F15-F19 実測データの扱いと
同種）。

**メンテナンス上の注意**: Mozc 本家がこれらの TSV を変更した場合、
awase 側のハードコードテーブルは追随しない（config1.db 読み取りとは
違い、実行時ではなくビルド時に固定される）。取得元コミット
（本節執筆時点の `google/mozc` HEAD）を記録し、将来の GJI アップデートで
挙動が変わった疑いが出た場合は本節を読み直し、最新の TSV と再照合
すること。

**取得元コミット（2026-08-15確認、GitHub API直接照会）**:
- `src/data/keymap/ms-ime.tsv`: `b4bbc42ff5524ec16a53cb4914166f6aed45056a`
  （2025-12-17、"Rename InputMode* -> CompositionMode*..."）
- `src/data/keymap/atok.tsv`・`kotoeri.tsv`: `bc9eab5b11e1ad9fcf863ff0c8a2736dc10f64b0`
  （2020-09-26）
- 確認時点の `master` HEAD: `851c3fe33060d2a6090363e4d7ec44fafde2c03d`
  （2026-08-14）

#### 決定A-5（round4追加、2巡目レビューで撤回・縮小）: 複合副作用キーの観測は既に `vk.rs` で実装済みであり、`ImeDetectConfig` への追加は行わない

**ユーザー要求**: 「IME → awase ON/OFF」については、ひらがなキーのような
「IME ON になる副作用のある」キーも対象に含めてほしい。

**round4時点の当初案（撤回）**: `VK_DBE_HIRAGANA`/`KATAKANA`/`DBCSCHAR`
→`on`、`VK_DBE_ALPHANUMERIC`/`SBCSCHAR`→`off` として `ImeDetectConfig`
に追加する案を出した。**2巡目レビューで、この案には2つの問題があると
判明し、撤回する。**

1. **既に実装済みの機構の重複だった。** `crates/awase-windows/src/vk.rs:118-147`
   の `ImeKeyKind::shadow_effect()` が、この5 VK 全てを寸分違わず同じ
   方向（ひらがな/カタカナ/全角→`TurnOn`、英数/半角→`TurnOff`）で
   既に分類している。`hook.rs:121` がこれを `shadow_action` として
   `RawKeyEvent.ime_relevance` に載せ、`IntentWitness::from_physical()`
   （`evidence.rs:350-357`）が `UserIntentSource::PhysicalImeKey` として
   観測する経路が**既に完成している**。この5 VK の観測という目的は
   追加実装ゼロで既に達成済みだった。round1 レビューが指摘した
   「`DeclaredOpenRole` という新型を作ろうとして既存の3機構を見落として
   いた」という誤りの構図が、round4 で再発していたことになる。
2. **`ImeDetectConfig` への追加は安全性の後退になる。** `key_pipeline.rs:803-812`
   では `sync_direction`（`SyncKey` witness）が `shadow_action`
   （`PhysicalImeKey` witness）より優先され、かつ後者だけが
   `is_japanese_ime()` ゲートを通る仕組みになっている。この5 VK を
   `ImeDetectConfig` に入れると、(a) 日本語 IME ゲートを迂回し、
   (b) 決定D のリスク節で Step 4b について警告している「`SyncKey` は
   BUG-51 追補3 の優先順位規則の下で安全弁より優先される」という
   格上げを、この observation 用途でも引き起こしてしまう。**これは
   ユーザーが求めた「より確実に拾う」の逆——安全側の判定が弱くなる方向
   の変更になる。**

**訂正した結論**: `VK_DBE_HIRAGANA`/`KATAKANA`/`ALPHANUMERIC`/`DBCSCHAR`/
`SBCSCHAR` の観測は、`ImeDetectConfig`/`SyncKey` 経路ではなく、
**既存の `vk.rs`/`PhysicalImeKey` 経路がそのまま担う**。本 ADR はこの
5 VK について `ImeDetectConfig` へのコード変更を行わない（既定値
`ImeDetectConfig::default()` への追加もしない）。「actuation で危険な
キーが observation でも危険とは限らない、という非対称性」自体は事実
だが、これは本 ADR が新たに発見したものではなく、**`vk.rs` に以前から
実装されていた既存の設計判断の再発見**であり、「本 ADR の実質的な
発見」という自己評価（round4時点の表現）は取り下げる。

**GUI 上の残タスク**: `main.rs:2210-2217` の「IME → awase ON/OFFキー」
ドロップダウン候補は `THUMB_KEY_OPTIONS`（`main.rs:2261-2280`、
カタカナ/ひらがなを含む）・`IME_DETECT_EXTRA_OPTIONS`（`main.rs:2312-2316`、
漢字/IMEオン/IMEオフ）・`IME_MODE_KEY_OPTIONS`（`main.rs:2304`、英数）を
chain して構成されており、**ひらがな/カタカナ/英数は既に候補に存在
する**。真に欠けているのは `VK_DBE_SBCSCHAR`（半角）/`VK_DBE_DBCSCHAR`
（全角）の2つだけであり、これを `IME_DETECT_EXTRA_OPTIONS` へ追加する
（上級者がまれな構成で手動指定したい場合の選択肢を増やすだけで、
既定値やディスパッチ経路には触れない）。これが本決定の唯一の実装項目
である。

**関連する別軸の穴（本 ADR の対象外）**: この5 VK の観測自体は
`PhysicalImeKey` 経路（`vk.rs`）で既に実装済みだが、その前提条件
`is_japanese_ime()` にはスリープ復帰/フォーカス変更直後の grace 期間中
に一時的な false negative が既知で存在する（`key_pipeline.rs:1940-1943`）。
この5 VK の受信を `is_japanese_ime()` の即時 true 更新トリガーに使う
ことで精度を上げられる可能性があるが、これは外部宣言の吸収という本 ADR
の主題ではなく、内部 belief の精度改善という別軸の問題のため、
[ADR-093](093-dbe-hotkey-observation-upgrades-japanese-ime-belief.md)
として切り出した。

### 決定B: 親指キー8 bool を「2 種族×専用の軸」として再設計する

現状の混乱の真因は命名の不統一ではなく、**Space/Enter（テキスト入力の
正規機能を持つキー）と 無変換/変換（IME のモードキー）が本質的に別種の
キーなのに、同じ bool 名の型に押し込まれている**ことにある。

**当初案からの変更点1**（round1 レビュー懸念2-1）: `TextKey` から
`ignore_composing_guard` 相当を削らない。`src/engine/tests.rs:914
test_space_thumb_suppressed_while_composing_when_guard_disabled` が
「`space_thumb_ignore_composing_guard=false` なら composing 中は suppress
される」という**非既定の実挙動**を既に固定しており、これを表現できる
場所が新型に無いと、既定以外のユーザーの挙動が消える（golden diff では
検知できない、後述）。

**当初案からの変更点2**（round1 レビュー懸念2-2）: `ModeKey` の
`solo_tap × while_composing` という直積をやめる。直積だと
`{DedicatedFnKey(F21), while_composing: Suppress}` のような
「専用Fnキーは常に最優先」という現行仕様と矛盾する組み合わせや、
`{Suppress, while_composing: Passthrough}` のような**現行に存在しない
未定義の状態**が発生する。代わりに、idle/composing それぞれに同じ
行動型を持たせる**総関数**にする:

```rust
pub struct TextKeyConfig {
    /// composing 中も常に生 VK を送出するか。既定 true（正規機能のため）。
    pub ignore_composing_guard: bool,
    pub shift_literal: bool,
}

pub struct ModeKeyConfig {
    /// composing していないときの単独タップの行き先。
    pub idle: SoloTapAction,
    /// composing 中の単独タップの行き先。
    pub composing: SoloTapAction,
}

pub enum SoloTapAction {
    /// 既定。OSへ一切送らない(2026-08-07実機インシデントへの防御)。
    Suppress,
    /// 素のVKを送る(上級者向け、自己責任)。
    Passthrough,
    /// GJI向け専用Fnキーへ変換(ADR-091 §D3.2、自動判定or手動)。
    DedicatedFnKey(VkCode),
    /// MS-IMEレジストリの宣言(決定A-2)に基づき、build_ime_set_open_decision
    /// 相当の処理でIMEのopen軸を操作する(round2、自動判定のみ。決定C R4)。
    DelegateToOpenAxis(ShadowImeAction),
}
```

`DelegateToOpenAxis` は決定C の優先順位規則に従い、**自動判定でのみ**
選ばれる（`Resolved<ModeKeyConfig>.source == AutoDetected`）。ユーザーが
`SoloTapAction` を明示設定している場合（`source == Manual`）は、
MS-IME レジストリの宣言があっても上書きしない（R1）。

**最優先ガード（2巡目レビュー指摘M5、`ModeKeyConfig` の外側）**:
`nicola_fsm.rs:1165-1171` は、親指キーが OS 修飾キー（Alt なりすまし、
`modifier_key.is_some()`）に割り当てられている場合、fn_key/
always_suppress/ignore_guard のいずれよりも手前で**無条件に suppress**
する（Alt なりすまし機構が生 Alt を OS へ流出させないための保護）。
この分岐は `SoloTapAction`/`ModeKeyConfig` の**外側**に置いたまま残す
——`modifier_key = Some(_)` の場合は下記の表を評価する前に確定して
`Suppress` になる。決定Bのリファクタでこの分岐を落とすと、Alt なりすまし
親指キー + `idle: Passthrough` の組み合わせで生 Alt が OS へ流れる回帰に
なるため、決定Bの網羅テスト要件（下記「合格条件の強化」）にこのケースを
含めること。

現行の実効表（`modifier_key = None` の場合のみ。`resolve_pending_thumb_as_single`
を実際にシミュレートして確認、round1 レビューによる検証済み）:

| fn_key | always_suppress | ignore_guard | idle | composing |
|---|---|---|---|---|
| Some(F21) | 任意 | 任意 | **DedicatedFnKey(F21)** | **DedicatedFnKey(F21)** |
| None | true | 任意 | Suppress | Suppress |
| None | false | true | Passthrough | Passthrough |
| None | false | false | Passthrough | Suppress |

この表がそのまま `ModeKeyConfig { idle, composing }` の4行に機械的に
写像でき、**`modifier_key = None` の範囲では意味未定義の組み合わせが
ゼロになる**（Fn キーは両スロットに同じ値を入れるだけで「常に最優先」
という現行仕様を保てる。`modifier_key = Some(_)` は上記ガードにより
`ModeKeyConfig` に到達する前に確定するため、この表の対象外）。

**決定Cとの接続**（round1 レビュー懸念2-3）: `DedicatedFnKey` の
自動/手動の区別（`muhenkan_dedicated_fn_key_is_manual()` 相当）は
`SoloTapAction` の外、決定Cの `Resolved<T> { value, source }` の
`T = ModeKeyConfig` として持たせる。GJI 離脱時の解除
（`set_muhenkan_dedicated_fn_key_auto(None)`）は
`Resolved<ModeKeyConfig>.source` を `AutoDetected` から `Default` へ
戻す操作として定義し、下地の `idle: Suppress, composing: Suppress` が
自動的に復活するようにする。

**当初案から変更なし**: `dbe_mode_key_policy`（物理 ひらがな/カタカナ/
Eisu キー）は「モードキー」という同じ種族だが、**統合は決定D Step 5
（強制レイヤーの統一後）まで行わない**（round1 レビュー懸念2-4、
下記決定D参照）。`conv_mode_policy`（conv モードの force-write、
ADR-084/086）は名前が似ているだけの**別軸**であり、統合しない。

**「意味が判明すれば `Suppress` は不要になるか」への回答は No。**
`muhenkan_solo_tap_always_suppress` が防いでいるのは
`KeyAssignmentMuhenkan=0`（MS-IME 既定の「かな切替」）のケースであり、
これは「意味が不明だから抑止している」のではなく「意味が判明していて、
その意味が危険だから抑止している」。決定Aの読み取り結果は `Suppress`
を消すのではなく、`DelegateToOpenAxis`（決定D Step 4）が選ばれる根拠を
与えるだけである。`Suppress` はキーの意味が判明していてもなお危険な
ケース（`KeyAssignmentMuhenkan=0`）向けの既定として残る。

**移行**: `#[serde(alias = ...)]` で旧8 bool を読み、起動時に新表現へ
正規化する。**既定の実挙動は完全に不変**とする。

**合格条件の強化**（round1 レビュー懸念2-1・2-2、当初案の「golden 差分
ゼロ」だけでは非既定の挙動変化を検知できないという指摘への対応）:
上記の実効表を `src/engine/tests.rs` に**非既定の組み合わせを含めて
網羅的に固定するテストを先に追加**し、そのうえで golden 差分ゼロを
確認する。既定値だけを見る golden diff は合格条件として**単独では
不十分**とする。

### 決定C: 優先順位を「明示 > 自動 > 既定」として全体で明文化する

`gji_charset_autodetect.rs:119`（`muhenkan_dedicated_fn_key_is_manual` なら
自動判定は一切介入しない）が既にこの原則を部分的に実装している。これを
設定全体の規約として明文化する。

```rust
struct Resolved<T> { value: T, source: SettingSource }
enum SettingSource { Manual, AutoDetected { from: &'static str }, Default }
```

- **R1**: `Manual` があれば自動判定は書き込まない（既存踏襲）。
- **R2**: 自動判定の**計算**は IME 種別確定イベントのたびに毎回行う
  （`Manual` が設定されていても計算自体は止めない——理由は下記
  「`ResolvedList`」と R5 を参照）。IME 種別が変われば計算結果を必ず
  再評価する（`autodetect.rs:112` の `None` リセットと同型を MS-IME 側
  にも義務づける）。
- **R3**: 読めなかった／曖昧（候補複数）なら **`Default` へフォール
  バックし、決して推測しない**（`detect_dedicated_fn_key` の
  「安全範囲内の候補がちょうど1つのときだけ採用」と同じ規律）。
- **R4**: 自動判定が差し替えてよいのは `Suppress → DedicatedFnKey` の
  方向、および決定D Step 1a/3 の **observation**（belief 追随のみ、
  ユーザーの `SoloTapAction` 設定そのものは書き換えない）に限る。
  **ユーザーが明示した `SoloTapAction` を自動で書き換えることはしない**
  （round1 レビュー懸念3-3を受け、当初案にあった「Step 1a が明示
  `Passthrough` 設定を自動で `Suppress` へ上書きする」という例外を
  撤回した。詳細は決定D参照）。
- **R5**（2巡目レビューで追加）: `AutoDetected` として表示されている値を
  ユーザーが GUI 上で直接編集すると、その場で `Manual` へ切り替わる
  （決定E-3 が GUI 側の挙動として既に前提していたが、R1-R4 のどれにも
  明文化されていなかったため追加）。この切り替えはユーザー起点の
  編集操作であり、R1（自動判定はManualを上書きしない）の対象外——
  「自動が明示を上書きする」のではなく「ユーザーが明示的に上書き
  した」ケースであるため矛盾しない。

**要素単位の `SettingSource`（`Vec` 値の設定、2巡目レビューで追加）**:
`ImeDetectConfig.on`/`off`/`toggle` や `KeysConfig.ime_on`/`ime_off` の
ような `Vec<String>` 設定は、リスト全体に1つの `source` ではなく
**要素（VK/コンボキー文字列）ごとに** `Manual`/`AutoDetected`/`Default`
が異なりうる（例: ユーザーが1つだけ手で追加し、残りは自動判定由来）。
このため `Resolved<T>` をリストにそのまま被せず、次の設計にする:

- **`config.toml` に永続化されるのは `Manual` エントリのみ**——
  `Vec<String>` という現行のシリアライズ形を**変更しない**（後方互換、
  スキーマ移行不要）。ユーザーが GUI/`config.toml` で書いたエントリは
  常に `Manual` として扱う。
- **`AutoDetected` エントリは `config.toml` に書き込まず、IME 種別確定
  イベントのたびに決定A-2/A-3/A-4 の読み取り結果から都度計算する
  ライブなオーバーレイ**として扱う。**この計算は `Manual` 側に既に
  同じ VK が存在していても止めない**（R2）——理由は「自動判定に戻す」
  ボタン（決定E-4）の復帰先を確保するため。`Manual` に同じ VK が
  あれば、GUI 表示・実際のディスパッチとも `Manual` の値を優先する
  （R1）が、**計算結果自体は破棄せず保持し続ける**。
- GUI 表示時は「`config.toml` の `Manual` リスト」と「ライブ計算した
  `AutoDetected` の候補」をマージした
  `Vec<Resolved<String>>`（`type ResolvedList<T> = Vec<Resolved<T>>`）
  を都度構築してバッジ付きで見せる（決定E参照）。同一 VK が両方に
  存在する場合は `Manual`側のみを表示し、`AutoDetected` 側は隠す
  （バッジの二重表示を避ける）——ただし内部的な計算そのものは
  上記の通り継続しているため、「自動判定に戻す」ボタン押下時に
  `Manual` エントリを削除すれば、次回描画時に同じ VK の
  `AutoDetected` 表示が自然に現れる。
- スカラー値の `Resolved<T>`（決定Bの `ModeKeyConfig` 等）にも同じ
  原則を適用する: `Manual` が設定されていても自動判定の計算（例:
  `detect_dedicated_fn_key`）は止めず、結果を保持しておくことで
  「自動判定に戻す」ボタンが機能するようにする。

`auto_start`（`"enabled"`/`"disabled"` 文字列）はこの枠組みの外に置く。
レジストリの実登録が SSOT で awase が書き込む側であり、「起動時に読み取り、
食い違えば表示するだけ」に留める（優先度は低い）。

設定画面には `SettingSource` を表示する（ADR-091 §D3.6 「現在有効な
モードの表示のみ」の具体化）。

### 決定E（round2追加、round3・round4で再修正）: `tab_keys`（キー設定）へ方向別の対称な名前で統合し、上級者向けとして折りたたむ

**round3案（常設フラット統合）からのさらなる修正（ユーザー指摘）**:
「初めて見た人にとって分かりやすいか」という問いを受けて自己点検した
結果、round3案には2つの未解決の問題があった。(a) 「IME 制御」（awase
が送信する側）と「IME ON/OFFキー」（awase が受信/解釈する側）という
名前が似すぎていて、方向の違いが名前だけでは伝わらない。(b) 親指キー
割り当て（大多数が触る）と、エンジン制御キー・IME制御キー・IME ON/OFF
キー・トグルホットキー（既定値のままで問題ない上級者向け設定）が
同じ階層にフラットに並び、Progressive Disclosure（段階的開示）の
定石に反する。この2点をユーザーに指摘したところ、**一度は「IME
ON/OFFキー設定を完全に廃止し、レジストリ/config1.db 読み取りだけに
一本化する」案が浮上したが、サードパーティIME・MS-IME未設定・GJI×
ことえり等、読み取り元が存在しないケースで設定手段が完全に失われる
（2つ前のユーザー発言「ネイティブUIを持たないIMEのためawase側で
設定UIを提供したい」と矛盾する）ため、「設定は残すが、名前を対称的に
し、上級者向けとして折りたたむ」方針に着地した。**

**修正した方針**:

1. **名前を送信/受信の対称なペアに変更する**:
   - 「IME 制御」（`config.keys.ime_on/ime_off`）→
     **「awase → IME ON/OFFキー」**（awase が能動的に送信するキー）
   - 「IME ON/OFFキー」（round3案の名前、`ImeDetectConfig{toggle,on,off}`）→
     **「IME → awase ON/OFFキー」**（外部IMEの状態変化を awase が
     解釈するためのキー）
   矢印の向きをラベルに埋め込むことで、ホバーテキストを読む前に
   方向の違いが視覚的に伝わるようにする。`config.toml` 側のフィールド名
   （`ime_detect`）も実装時にこの対称性に合わせて改名を検討する
   （旧名との互換は serde alias で吸収）。
2. **「awase → IME ON/OFFキー」と「IME → awase ON/OFFキー」の2つを、
   `tab_keys` 内で `egui::CollapsingHeader`（既定で折りたたみ）に
   まとめ、「上級者向け設定」の見出しを付ける。** 親指キー割り当て・
   エンジン制御（エンジン ON/OFF・単独5連打）・トグルホットキーは
   引き続き常時展開表示のまま（多くのユーザーが実際に触る設定のため）。
   これにより Progressive Disclosure に沿った構造になる。
3. **「IME → awase ON/OFFキー」内の `toggle`/`on`/`off` の3つのリストは
   折りたたみを開いた後、それぞれ明確なラベル付きで個別に表示する**
   （`bare_key_list_ui` を3回呼ぶ既存構造のまま——1つの曖昧なリストに
   まとめない、というユーザーの明示的な要求）。
4. **タブとしての出し分け（round2案）はしない**——折りたたみの開閉は
   ユーザー操作で行い、自動判定の成否による条件表示ロジックは実装しない
   （round3の判断を維持）。

**`ImeDetectConfig` のデータ構造・`config.toml` 手動編集能力は維持する**
（完全廃止案は不採用。フィールド名の改名のみ実施予定）。

**Step 4c の実装順序に対する影響**: round3までと同様、この GUI
リファクタ（名称変更・折りたたみ化）は決定A-2〜A-4 の自動判定ロジックに
依存しないため、Step 4 を待たず単独で先行着手できる。

**他の GUI 変更点（決定B/決定Cのおさらい）**:
- 決定Bの `TextKeyConfig`/`ModeKeyConfig` への再編に伴い、無変換/変換/
  Space/Enter の4つの似た見た目のセクション（`awase-settings/src/main.rs:918-1012`）
  は「テキストキー」「モードキー」の2グループへ整理される（挙動は不変、
  表示の整理のみ）。
- 決定A-3の「揺れ」検出時は、`msime_key_assignment.rs::check_and_warn`
  と同型の警告ポップアップが追加される可能性がある（常設の表示ではなく、
  条件が揃った時だけ出る通知）。

#### `SettingSource`（決定C）の具体的な表示方法（round4追加）

決定Cは `SettingSource{Manual, AutoDetected{from}, Default}` というデータ
モデルを定義したが、GUI 上でどう見せるかは未確定だった。ユーザーから
「awase 内独自設定（`Manual`/`Default`）と、レジストリ/config1.db から
自動取得された内容（`AutoDetected`）を区別して表示したい」という要求を
受け、次の表示方針とする。

既存の `main.rs:1420-1422`（`変更あり`/`保存済み` を
`egui::RichText::new(..).color(Color32::..)` で色分け表示する既存パターン）
と `color_legend`（`main.rs:2759`、yab エディタのセル色凡例）を踏襲する。

1. **各キーリストの各エントリの右側に、`SettingSource` に応じた小さな
   色付きバッジを表示する**（`egui::RichText` + `Color32`、新規ウィジェット
   フレームワークは導入しない）:
   - `AutoDetected { from }` → 青系バッジ、ラベルは `from` の内容。
     **`from` はユーザー向けには内部のファイル名・レジストリ名ではなく、
     製品名で表示する**（ユーザー判断: 「config1.db」ではなく
     「Google日本語入力設定」と説明的に表示する）。対応表:
     - 決定A-2（MS-IME レジストリ読み取り）→「自動: Microsoft IME 設定」
     - 決定A-3（GJI config1.db、`session_keymap == CUSTOM`）→
       「自動: Google 日本語入力設定」
     - 決定A-4（GJI×ATOKプリセット）→「自動: ATOK 設定」

     `from` フィールド自体（`&'static str`、決定Cのデータモデル）は
     内部識別用の技術的な文字列のまま保持し、上記の表示名への変換は
     GUI 表示層（`main.rs`）で行う一覧テーブル（`SettingSource` の
     `from` 識別子 → 表示名）を介する。ADR 内の他の記述（決定A-2/A-3/A-4
     本文、コード上の識別子）はこれまで通り `config1.db`/レジストリ等の
     技術名で表記する——ここで変更するのは**ユーザーに見せる GUI 上の
     文言のみ**。
   - `Manual` → 緑系バッジ「手動」。
   - `Default` → グレー系バッジ「既定」。
   `Manual` と `Default` は "awase 内で完結している" という点で共通する
   ため近い色調（緑/グレーいずれも寒色の `AutoDetected` の青とは明確に
   区別できる色）にする。
2. **セクション先頭に `color_legend` 形式の凡例を1回だけ出す**（各エントリに
   毎回説明文を付けない。「IME → awase ON/OFFキー」「awase → IME
   ON/OFFキー」の折りたたみを開いた直後に表示）。
3. **`AutoDetected` な値をユーザーが直接編集すると、決定C R5 の通り
   `Manual` へ即座に切り替わる**（GUI 側はこの切り替わりをバッジの
   色変化で可視化するだけで、別途確認ダイアログ等は挟まない——誤操作
   より、自動判定を信頼しているユーザーが意図せず上書きすることの方を
   軽視できない、という判断）。
4. **`Manual` に切り替わったエントリを「自動判定に戻す」ボタン**を
   バッジの隣に用意する（クリックで該当エントリを `Manual` リストから
   削除する）。決定Cの「要素単位の `SettingSource`」設計（R2の計算継続）
   により、`Manual` 側にエントリが有る間も `AutoDetected` の計算は
   ライブに継続しているため、削除した瞬間に同じ VK の `AutoDetected`
   表示が（該当すれば）自然に復帰する——`Manual` の値を退避しておいて
   復元するのではなく、**そもそも自動判定の計算結果を捨てていない**
   ことでボタンの約束を満たす（2巡目レビュー指摘M4への対応）。
5. この表示は「awase → IME ON/OFFキー」「IME → awase ON/OFFキー」
   （決定E）に加え、決定Bの `SoloTapAction::DedicatedFnKey` 自動選択
   （`gji_charset_autodetect.rs` 由来）にも同じバッジ規約を適用する
   ——`SettingSource` は決定Cで全体共通の型として定義されているため、
   表示規約も特定セクション限定にせず横展開する。

### 決定D: 実装順序

`CharsetSlot` の教訓（一度に全部やらない）に従い、段階的に進める。
**round1 レビューを受け、当初案の Step 1a/1b の内容と優先順位を大きく
見直した。**

#### Step 0（コード変更ゼロ・最優先）

**当初案からの変更**（round1 レビュー懸念3-1・3-2）: 確認対象を
「`[msime-keyassign]` ログ1行」から差し替える。BUG-50 仮説A
（`docs/known-bugs.md` 該当箇所）が疑うのは「無変換単独タップの生 VK
素通し」だが、これは **2026-08-07 のコミット `899b416f`
（`muhenkan_solo_tap_always_suppress` を既定 `true` 化）で既定構成では
既に塞がれている**。したがって確認すべきは:

1. **`899b416f` 以降（2026-08-07 以降）に BUG-50 が再現したか。**
   再現していなければ仮説Aは実質クローズで、本 ADR に BUG-50 由来の
   動機は残らない（Step 1a/3 は「念のための構造的な穴塞ぎ」という
   位置づけに格下げする）。
2. 再現している場合、**その時点で engine が active だったか**
   （`Engine deactivated (reason=...)` ログの有無で判別）。非活性
   だったなら、真因は「背景」節に記載した Phase 2 の早期 return による
   バイパス経路であり、Step 1a/3（`resolve_pending_thumb_as_single` 内
   の変更）は無関係と確定する——対策の置き場所は engine の活性/非活性に
   依存しない transport 層になる。

`[msime-keyassign]` のログ自体は Step 1a/3 着手時の補助情報として
引き続き有用だが、**BUG-50 との関連づけの根拠にはしない。**

#### Step 1（当初案の Step 4 を繰り上げ、最優先で着手）

`engine_on_ime_key`/`engine_off_ime_key`（`src/config.rs:422/428`、既定
`VK_DBE_DBCSCHAR`/`VK_DBE_SBCSCHAR`、消費は `app/bootstrap.rs:395/400` の
2箇所のみ）の既定値を `None` へ落とす。ADR-091 決定1（open 軸は
`VK_IME_ON`/`VK_IME_OFF` で決着済み）より前の機構の残骸であり、複合
副作用キーが既定で生存しているのは筋が悪い。**実挙動が変わるため実機
確認必須。** round1 レビューで「消費箇所が少なく根拠も明快、費用対効果
が最も高い」と評価されたため、当初案の Step 4 から繰り上げる。

#### Step 2（決定Bのリファクタ、Step 0 の結果を待たず並行して着手可）

決定Bの `TextKeyConfig`/`ModeKeyConfig` への移行。合格条件は決定B記載の
通り「非既定の組み合わせを含めた網羅的固定 + golden 差分ゼロ」。

#### Step 3（Step 0 の結果次第、observation のみ・actuation はしない）

Step 0 で「engine 非活性時のバイパス経路」が実害を持つと確認できた
場合に限り着手する。**当初案の Step 1a（`resolve_pending_thumb_as_single`
内で `Suppress` へ自動フォールバックする）は撤回する**（round1 レビュー
懸念3-1・3-3: 既定構成では no-op であり、明示 `Passthrough` 設定の
自動上書きは決定C R1/R4 に違反する）。

代わりに検討する内容: engine 非活性時に無変換/変換が生 VK のまま OS へ
届く経路（`engine.rs:303-310` の Phase 2 早期 return）を、
`dbe_mode_key_policy` と同じ **transport 層**で閉じるか否か。これは
`resolve_pending_thumb_as_single`（engine 活性下でしか呼ばれない）の
外側の問題であり、決定Bの `ModeKeyConfig` とは独立した変更になる。
**この Step は observation（belief 追随）ではなく抑止範囲の拡大であり、
ユーザーの既存設定を上書きしない形（新しい既定の追加、または明示的な
オプトイン）で設計すること。**

#### Step 4（決定4 上段・肩代わり本体、round2で着手対象へ格上げ）

**round1 の No-go 判定は round2 で解除する。** 根拠: round1 懸念4-1
（`architecture_guard`/`PhysicalImeKey` 衝突）は `UserIntentSource::SyncKey`
の使用で解消済み（決定A-2）。ただし round1 が指摘した他の懸念は依然
有効な設計上の注意点として残す。**宣言ソースは MS-IME レジストリ
（決定A-2）と GJI config1.db（決定A-3）の2つがあるが、下流の
dispatch（Step 4a/4b）は共通**——ソースが違うだけで、変換先の
`ShadowImeAction` を得た後の処理は同じ経路を通る。

**Step 4c（新規、読み取り側の配線）**: `sync_gji_charset_autodetect`
（`gji_charset_autodetect.rs`）が GJI 検出時に `config1.db` を読む処理は
既存（F21 検出用）。同じタイミングで `read_gji_ime_keys()`
（決定A-3）も呼び、無変換・変換・Ctrl+Space・Shift+Space に該当する
VK が `on`/`off`/`toggle` のどれかに含まれていれば
`ShadowImeAction` に変換して Step 4a/4b へ渡す。MS-IME レジストリ読み
（`msime_key_assignment.rs`、決定A-2）と GJI config1.db 読み
（本Step）は排他——`sync_ime_kind_from_observation` が IME 種別ごとに
どちらか一方だけを呼ぶ既存の分岐（`runtime/message_handlers.rs:381`）
にそのまま乗せられる。

**Step 4a: Ctrl+Space / Shift+Space（トグル値のみ、着手可）**

2026-08-15 実機確認（dragonflyg4）で `KeyAssignmentCtrlSpace`/
`KeyAssignmentShiftSpace` の実在と、「IME ON/OFF」選択時の値が
`2`（`ShadowImeAction::Toggle` 相当）であることを確認済み。決定A-2の
通り、`keys.ime_on`/`ime_off` へ `"Ctrl+Space"`/`"Shift+Space"` を条件
付き自動追加するだけで良い**はず**だが、`Toggle`（方向不定、現在の
IME状態に応じてON/OFFが決まる）を `keys.ime_on`/`ime_off`（方向固定の
リスト）にどう落とすかは詰める必要がある——`KeysConfig` に
`ime_toggle: Vec<String>` 相当が無ければ追加するか、`apply_special_key_match`
の `SpecialKeyMatch` に `Toggle` variant を追加することになる
（`ImeDetectConfig.toggle` が `VK_KANJI` 用に同種の「方向不定」を既に
扱っているため、その実装を参考にできる）。「IME-オンのみ」/
「IME-オフのみ」個別選択時の値は未確認のため、その対応は追加の実機
確認後に別途行う。GJI 側（`GjiImeKeys.on`/`off`）は方向が確定している
ため、この Toggle 特有の課題は無い（`on`→`keys.ime_on`、
`off`→`keys.ime_off` に素直に対応付けられる）。

**Step 4b: 無変換 / 変換（新しい dispatch 点、より慎重に）**

決定A-2の通り、`resolve_pending_thumb_as_single`（NicolaFsm Phase 3）に
新しい分岐を追加し、`SoloTapAction::DelegateToOpenAxis(ShadowImeAction)`
のとき `build_ime_set_open_decision` 相当の処理（`desired_open` 更新 +
`VK_IME_ON`/`VK_IME_OFF` 送出）を呼ぶ。宣言ソースが MS-IME レジストリ
（`KeyAssignmentMuhenkan`/`Henkan`）でも GJI config1.db
（`GjiImeKeys.on`/`off`/`toggle`）でも、この分岐点は共通。round1 が
指摘した以下の注意点は Step 4b 特有のリスクとして残る
（Ctrl+Space/Shift+Space は既存の combo-key 経路をそのまま使うため
該当しない）:

1. **「単独タップ=IME切替意図」という解釈は awase の推定である。**
   親指シフトの同時打鍵判定の取りこぼしが単独タップとして誤確定した
   場合、意図しない IME 切替が起きうる（2026-07-06 の実害の再生産
   リスク、リスク節参照）。**回帰テストで「chord のタイミングウィンドウ
   内の誤確定では発火しない」ことを固定すること。**
2. `SetOpenOrigin::ExplicitUserAction` の doc
   （`src/engine/decision.rs:68-69`、「ユーザーが明示的に要求した」）は
   レジストリ宣言経由の解釈にはそのまま当てはまらない（round1
   懸念4-4）。doc の更新、または `UserIntentSource::SyncKey` に対応する
   独自の origin 表現を検討すること。
3. レジストリミラーの stale 化（決定A参照）に対応するため、設定
   リロード時の再読み、または `RegNotifyChangeKeyValue` によるイベント
   駆動更新を Step 4b と同時に実装すること。

回帰テストは `src/engine/tests.rs`（Linux 実行可）に追加する。
`conflict_warning()` の警告撤去は**実機で肩代わりが効くと確認できた
後の別コミット**とする（それまでは二重警告でも実害はない）。

**実装セッション（2026-08-15）での設計変更・発見（Opusコードレビュー・
実装過程で判明、当初案からの重要な訂正）**:

1. **witnessは`UserIntentSource::SyncKey`ではなく既存の`Command`を使う。**
   当初案（`ImeDetectConfig`に無変換/変換のVKを登録し`SyncKey` witnessを
   得る）はOpusコードレビューで**致命的な欠陥**が判明し撤回した:
   `enrich_ime_relevance`は登録VKに`sync_direction`を無条件に立て、
   `kp_stage_shadow_ime_toggle`（`engine.on_input`より前に実行）が
   `sync_direction`があればKeyDownのたびに`write_sync_key(!effective_open())`
   する——単独タップ確定かどうかを一切見ない。無変換は左親指キーとして
   ほぼ全打鍵で押されるため、登録すると「か」を打つたびにIMEがON/OFF
   する壊滅的な回帰になる。代わりに、`NicolaFsm`に`engine_off_requested`
   と同型のワンショットチャネル（`ime_open_requested: Option<ShadowImeAction>`）
   を追加し、`resolve_pending_thumb_as_single`が確定した単独タップでのみ
   これをセット、`Engine::on_input`/`on_timeout`が取り出して
   `Effect::Ime(SetOpen{origin: ExplicitUserAction})`を（`Decision`の
   既存効果を保ったまま`push_effect`で）追加する。これは`ime_on`/`ime_off`
   コンボキーが既に使っている完成経路と同じで、`origin ==
   ExplicitUserAction`なら自動的に`UserIntentSource::Command`
   （「awaseエンジン内部の判断」向けに元々存在する）として記録される。
   round1懸念4-4（`ExplicitUserAction`のdocが「ユーザーが明示的に要求した」
   でレジストリ宣言経由に当てはまらない）はこれで実質解消——`Command`は
   元々「Engineから SetOpen要求等」という一般的な文言のdocを持つ。
2. **`SoloTapAction`に`DelegateToOpenAxis`variantは追加しなかった。**
   `DedicatedFnKey`と同じ理由（自動検出値がconfig reloadで消去される
   のを防ぐ）で、`muhenkan_delegate_to_open_axis`/
   `henkan_delegate_to_open_axis: Option<ShadowImeAction>`という
   `NicolaFsm`の独立フィールドとして実装した。優先順位:
   専用Fnキー＞delegate_to_open_axis＞`ModeKeyConfig`ベースの判定。
3. **新たに判明した構造的な範囲境界**: `Engine::compute_active`は
   `ctx.ime_on`を判定条件に含むため、`ime_on=false`の間はPhase 2で
   無条件`pass_through()`を返しPhase 3（`resolve_pending_thumb_as_single`
   を含む）に到達しない。つまり`DelegateToOpenAxis`は**IMEが既にON
   の状態からの操作でしか発火し得ない**（`TurnOff`/`Toggle`（ON→OFF側）
   は届くが、`TurnOn`（OFFからONへ）は届かない）。これは実装のバグ
   ではなく「背景」節が既に指摘していた構造的な穴（Step 3の対象）の
   自然な帰結——engineが非活性（IME OFF）の間はawaseが無変換/変換の
   生VKをそもそも横取りしないため、MS-IME/GJI自身のネイティブなキー
   割当て処理（`KeyAssignmentHenkan=1`等）にそのまま委ねられる形に
   なる（＝TurnOn方向は代わりにIME自身のネイティブ処理が担う）。
   回帰テスト（`delegate_to_open_axis_*`系）は全て`ime_on_ctx()`
   （engine active）を前提にする。

#### Step 5（`dbe_mode_key_policy` の `ModeKeyConfig` への統合）

**当初案からの変更**（round1 レビュー懸念2-4・5-2）: 「背景」節に
記載の通り `dbe_mode_key_policy`（transport 層、強い保証）と無変換
`always_suppress`（engine 活性下限定、弱い保証）は保証の強さが違う。
**強制レイヤーを揃える作業（Step 3 の transport 層統一）より前に
統合しない。** Step 3 の結果、無変換の抑止も transport 層に寄った
場合にのみ、この統合を検討する。

#### Step 6（`tab_ime_detect` を `tab_keys` へ統合し方向別対称名に改称・上級者向け折りたたみ化、round2追加・round3/round4修正）

決定E（round4修正版）の通り、独立タブ「IME検出」（`tab_ime_detect`）を
廃止し、`tab_keys`（キー設定、`main.rs:886-1084`）内へ統合する。
「IME 制御」→「awase → IME ON/OFFキー」、`ImeDetectConfig` 由来の
セクション→「IME → awase ON/OFFキー」と、送信/受信の向きが伝わる対称な
名前に変更したうえで、両者を `egui::CollapsingHeader`（既定折りたたみ、
見出し「上級者向け設定」）にまとめる。親指キー割り当て・エンジン制御・
トグルホットキーは常時展開のまま残す。「IME → awase ON/OFFキー」内の
`toggle`/`on`/`off` の3リストは、折りたたみを開いた状態でそれぞれ
個別ラベル付きで表示する（1つのリストに統合しない）。条件表示
（round2案）・完全廃止（round4で検討し不採用）のどちらも採らない。
決定A-2〜A-4 の自動判定ロジックに依存しない純粋な GUI リファクタなので、
Step 4（自動判定実装）を待たず単独で先行着手できる。`ImeDetectConfig`
自体（データ構造・`config.toml` 手動編集）は変更なく残すが、フィールド名
（`ime_detect`）は新名称に合わせて実装時に改名を検討する（serde alias で
旧 config.toml との互換を保つ）。

**本 Step の範囲に `SettingSource` バッジ表示（決定E-1〜E-5）も含める**
（2巡目レビュー指摘: バッジ表示がどの Step に属するか本文に明記が
無かった）。ただし E-5 が要求する `SoloTapAction::DedicatedFnKey`
自動選択への横展開は、Step 6（GUI）単独では完結しない——決定Cの
「要素単位/スカラーの `SettingSource`」実装（Step 2、決定Bのリファクタと
セット）が前提になるため、E-5 部分は Step 2 完了後に着手する。

## リスク: `CharsetSlot` の轍を踏まないための自己検証

**問い**: 「無変換の意味を読み取って吸収する」は `CharsetSlot`
（awase が能動的に IME 状態を判断する設計）への回帰ではないか。

回帰ではないと判断する根拠:

1. **判定材料が awase の推定ではなく外部の宣言**（レジストリ DWORD）
   である。`CharsetSlot`（MS-IME 版）が破綻した直接の理由は「今
   composition 中か」を awase から安定観測できないことだった
   （ADR-091 §3.1 理由1）。今回の判定は composition 状態に一切
   依存しない。
2. **決定Dで actuation（Step 4）と observation（Step 1/3）を明確に
   分離した。** observation は既存の `ImeDetectConfig`/`ShadowImeAction`
   という実証済みの belief 追随機構の延長であり、新しい判断ロジックを
   持ち込まない。actuation（能動的に `VK_IME_ON/OFF` 等を送る、Step 4）
   は round2 でユーザー判断により着手対象へ格上げしたが、
   `UserIntentSource::SyncKey` という既存の witness 経路を通す設計に
   限定し、新しい判断ロジック（belief）は持ち込まない。
3. 対象が2値+トグル（`ShadowImeAction`、open 軸）であり、5値サイクル
   （charset）ではない。

**危ういと認める点（round1 レビューで追加・round2 で更新）**:

- **「単独タップ=IME を切り替える意図」という解釈自体は awase の推定
  である。** 親指シフトの同時打鍵判定の取りこぼしが単独タップとして
  確定した場合、意図しない IME 切替が起きうる——2026-07-06 の実害の
  再生産になりかねない。Step 4b が実装される場合、この誤検知は
  `SyncKey` 由来の明示意図として belief に書き込まれることになり、
  BUG-51 追補3（`6ac8ea0f`「belief と warrant で安全弁 vs 明示意図の
  優先順位が逆」）の優先順位規則の下では**安全弁より優先される**。
  これは「あとで観測により是正され得る乖離」を「正当な意図として
  記録された乖離」に格上げする方向であり、素朴には**緩和ではなく
  悪化**になりうる（round1 レビュー懸念4-2）。**round2 でユーザーが
  この risk を理解した上で着手を判断したが、実装時は同時打鍵の
  タイミングウィンドウ内での誤確定が発火しないことを回帰テストで
  固定すること（Step 4b 参照）。**
- **engine 非活性時のバイパス経路**（「背景」節参照）は、Step 4b の
  スコープでは触れられない場所にある。Step 3 で別途扱う。
- 決定Aで新型を追加せず既存の3値語彙（`ShadowImeAction`）を再利用する、
  という運用ルールを維持する。消費者のいない variant を先回りで作らない。
- MS-IME が抑制後に別経路（アプリ内ショートカット等）で反応しない
  保証はない。実機確認事項。
- **レジストリミラーの stale 化**（決定A参照）。MS-IME 単独ユーザーは
  セッション中に更新されないため、Step 4b 実装時に対応必須（Step 4b
  前提条件3）。
- **Ctrl+Space/Shift+Space の「個別オン/オフ」割当て時の数値が未確認**
  （round2 追加）。現状確認できているのは「トグル」割当て（値2）のみ。
- **GJI 側の任意VK吸収（決定A-3）が依拠する「現在のレイアウトでかな
  文字を生成するか」の判定は、実装がまだ無い。** `.yab` レイアウトの
  逆引き（VK→かな文字の有無）が必要で、レイアウト切替時にこの判定結果
  も追随させる必要がある（キャッシュの stale 化と同種の懸念）。
  誤判定（かな生成キーを非生成と誤認）した場合の実害は「実際の打鍵
  キーがIME on/off吸収の対象になる」であり、BUG-64級のリスクなので、
  この判定ロジック自体に十分なテスト（全レイアウト×全VKの網羅、
  または既存の `NicolaFsm` が持つ「このVKは現在かなを生成するか」の
  判定を流用できないか確認）が要る。

## 自己評価

**妥当性スコア: 7/10**（round1 レビュー前の初回設計案は7/10、round1で
判明した誤りにより6/10へ引き下げたが、round2 で `SyncKey` という
実在する解決策が見つかり、かつ実機でレジストリ値を確認できたため
7/10へ戻した。**2巡目レビュー（round1後にA-4/A-5/決定Eが4回改訂される
間、round1と同水準の検証を経ないまま積み上がったことを受けた再レビュー）
では、新しく足された部分ほど検証密度が低い状態にあると指摘され、
一時的には5〜6/10相当と判定された。指摘されたM1〜M6（決定A-5の
vk.rs重複・安全性後退、GUIギャップの事実誤認、決定Cのデータモデルが
要素単位のバッジ表示を表現できない構造的欠落、「自動判定に戻す」
ボタンの復帰先欠如、決定Bの総関数表がAlt親指なりすまし優先ガードを
落としていた点、決定A-4のATOK評価の誤り）を本文に反映済み。反映後は
7/10で妥当と判断する**——ただし Step 4b は依然、リスク節記載の「単独
タップ=意図」推定に関する不確実性を抱えている）。

**2巡目レビューが示した教訓**: 本 ADR の中心制約（「消費者のいない
設定を作らない」「awase の推測で外部事実を置き換えない」）は round1
以降、**ADR 自身の改訂プロセスには適用されていなかった**（決定A-5が
その典型）。今後さらに改訂を重ねる場合、新規追加のたびに既存コードを
再 grep して重複が無いか確認すること。

最も自信が低い箇所、確認が必要な順:

1. **Step 0 の確認結果そのもの。** `899b416f` 以降の BUG-50 再現有無、
   および再現時の engine 活性状態。これが Step 1/3 にどれだけ実質的な
   動機が残るかを決める（Step 4 の動機は BUG-50 とは独立、決定D参照）。
2. **`build_ime_set_open_decision` 相当の処理を NicolaFsm Phase 3 から
   呼ぶ具体的な実装方法**（Step 4b）。Engine 層と NicolaFsm 層をまたぐ
   配線の詳細は未設計。
3. **Ctrl+Space/Shift+Space の Toggle 表現**（Step 4a）。
   `keys.ime_on`/`ime_off`（方向固定）に「方向不定のトグル」をどう
   落とすかは `ImeDetectConfig.toggle`/`VK_KANJI` の実装を参考にする
   必要があるが、詳細は未検証。
4. **GJI 側の「かな文字を生成するVKか」の判定ロジック**（決定A-3の
   安全フィルタ）。実装方法・既存機構の流用可否とも未検証。BUG-64級の
   実害があり得るため、Step 4c 着手前に固めること。
5. 決定B の serde 後方互換の具体形は未検証。
6. **決定A-4のATOK `CancelAndIMEOff` を Precomposition 限定にする対策**
   （2巡目レビュー指摘M6）は机上の設計であり、ATOK 実機での検証が
   まだない。
7. ~~Mozc 取得元コミットハッシュが未記録~~ — 2026-08-15実装セッションで記録済み
   （決定A-4節参照）。記録の過程で表の誤り（DirectInput Henkanの`Reconvert`
   欠落、Composition/Conversion行の欠落）も発見・訂正した。

## 関連

- [ADR-091](091-idempotent-charset-axis-gji-recommended-msime-self-responsibility.md)
  （charset 軸、`CharsetSlot` 撤回の経緯、F22-F24 予備バインド即日撤回）
- `docs/known-bugs.md` BUG-13・BUG-50・BUG-51（無変換/open軸関連）
- 2026-08-11 の IME 制御キー危険度分類調査（`engine_on/off_ime_key` の
  危険性指摘の初出）
- `.claude/rules/ime-belief-architecture.md`（`IntentWitness`/
  `UserIntentSource` の型ガードの設計意図）
