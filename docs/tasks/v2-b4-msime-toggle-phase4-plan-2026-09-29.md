---
title: ADR-199 T17 Phase 4（MS-IME 本体の無変換/変換=値2 トグルの能動化）の実装計画と検証計画
status: 計画（実装はしない）。CI 計測は4回実施したが、値2そのものを CI で作れず、値2の未確認セルは実機待ち
created: 2026-09-29
related_adr: ["ADR-199", "ADR-192", "ADR-197", "ADR-202"]
---

# T17 Phase 4 計画（v2 に含める、所有者決定 2026-09-29）

範囲は「MS-IME 本体の `KeyAssignmentMuhenkan` / `KeyAssignmentHenkan` が**値2（IME-オン/オフのトグル）**のときだけ、
awase が無変換/変換の単独タップで IME を開閉する」こと（決定16 の MS-IME 本体版）。値0/1/3 は受動のまま。
この文書は実装しない。Phase 4 の実装内容・検証計画・実測・所有者への論点をまとめる。

## 0. 要点（先に結論）

1. 実装は小さい。**Engine（`src/engine/`）は無変更**、`msime_native_key_role` の無変換/変換の腕と、`enrich_thumb_key_role` の
   composing ガード（純関数1本）、`conflict_warning` からの値2削除が本体。新しい actuation 合流点も、新しい belief イベントも足さない。
2. **能動と受動の境界は「Engine が活性で、入力中でも変換中でもない状態の単独タップ」**。入力中・変換中・候補窓表示中は
   今日と同じ経路（`ModeKeyConfig`）に落とし、MS-IME 本来の動作（かな⇔カタカナ変換等）に任せる。M1（2026-09-28 実機）で分かっている
   「入力中は開閉が発火しない」を、awase が壊さないための条件。
3. **CI（windows-latest）では値2を作れない**ことを実測で確定した（§4）。レジストリ直書きは反映されず（T12 の CI 版再確認）、
   設定アプリの UIA は英語 UI だと「キーの割り当て」セクション自体が出ない。よって「値2で入力中/変換中/候補窓/確定直後がどう動くか」
   の未確認セルは**実機（日本語 UI）でしか測れない**。ただし §3 の設計は未確認セルの結果に依存しない（保守的に除外する）ので、
   実機確認は実装のブロッカーにせず、実装後の受け入れ確認として回せる。
4. 所有者に決めてほしい論点は §6（7件）。

## 1. 現状（コードの事実、develop 88f9c1f8 時点）

| 場所 | 現状 |
|---|---|
| `state/key_effect_predictor.rs::KeyEffectKeymap::msime_native_key_role` | 半角/全角（0xF3/0xF4）だけ `Some(ImeToggle)`（互換モード `Some(true)` は受動）。無変換/変換（0x1D/0x1C）は常に `None` |
| 同 `for_msime_native(assignment_enabled, henkan, muhenkan, compat)` | 値を `*_reassigned: bool`（値が存在するか）に潰している。生の値は指紋（`msime_native_keymap_fingerprint`）にだけ入る。**値==2 かの情報が構造体に無い** |
| `runtime/mod.rs::enrich_thumb_key_role`（`kp_run_inner` 冒頭、非injected・非リピートの無変換/変換 KeyDown のみ） | `thumb_forced_action(configured, ime.is_some(), modified, injected, \|\| ime.and_then(derive_key_shadow_action))` を Engine の `set_thumb_forced_open_actions` へ。GJI は既に配線済み。MS-IME は `msime_native_key_role` が `None` を返すので実質受動 |
| `runtime/mod.rs::derive_key_shadow_action` | `ime` で `gji_key_role`/`msime_native_key_role` を選び、`key_shadow_action`（config 重なり→役割なし、学習表の矛盾→受動）へ。IME 種別を見るのはここだけ |
| `state/key_effect_table.rs::toggle_contradiction` | 学習表の `Stage::None` セルだけを見て狭める。無変換/変換は `NARROWABLE_KEYS` に入っている。composing 除外と整合する（追加変更不要） |
| `src/engine/nicola_fsm.rs::resolve_pending_thumb_as_single`（優先順位1.5） | `forced_open_action` があり、明示 `*_solo_tap_ime_action` なし・`explicit_action_consumed` なし・`suppress_solo_output` なし・Shift なしなら、**composing を見ずに**開閉要求。`Engine::apply_ime_open_request` が `action.resolve(ctx.ime_on)` して `Effect::Ime(SetOpen)`（`origin: ExplicitUserAction`、`UserIntentSource::Command`）を出す |
| `msime_key_assignment.rs::conflict_warning` | 値2（`*_is_toggle`）を値0/1と同じ「二重オーナー」警告の対象にしている（Phase 4 未実装のための暫定、docコメントに「Phase 4実装時にこの分岐を外すこと」） |
| 互換モード | `KeyEffectKeymap::msime_compat_mode`（`read_legacy_compat_mode_enabled`、`NoTsf3Override2`）。半角/全角では `Some(true)` で受動。T12: 互換モード ON では KeyAssignment の値自体が効かない |

## 2. Phase 4 の実装内容（どこをどう変えるか）

### 2.1 変更一覧

| # | ファイル | 変更 |
|---|---|---|
| A | `state/key_effect_predictor.rs` | `KeyEffectKeymap` に `muhenkan_toggle: bool` / `henkan_toggle: bool`（= `assignment_enabled && value == Some(2)`）を追加。`for_msime_native` で設定、他のコンストラクタは `false`。指紋は既に生の値を含むので変更しない（ADR196-T5 の `env_version` と学習表の陳腐化検出はそのまま効く） |
| B | 同 `msime_native_key_role` | 0x1D→`muhenkan_toggle`、0x1C→`henkan_toggle`。どちらも `msime_compat_mode != Some(true)` のときだけ `Some(ImeToggle)`（互換モード ON は値が効かないので受動）。値0/1/3・マスタースイッチ OFF・値なしは `None`。docコメントの「保留中」記述を更新 |
| C | `state/key_effect_runtime.rs` | 純関数 `msime_thumb_toggle_blocked(composing: bool, stage: Stage) -> bool`（= `composing \|\| stage != Stage::None`）を追加（ホストテスト対象）。`thumb_forced_action` の型は変えない |
| D | `runtime/mod.rs::enrich_thumb_key_role` | `role` クロージャで `ime == ImeKindId::MsIme` のとき、`ime_composition_active_now()` と `platform_state.ime.model().key_track().stage` を読んで C が真なら役割を求めず `None`。**GJI 側の経路は変えない**（GJI は決定11 により composing 中も発火する） |
| E | `msime_key_assignment.rs` | `conflict_warning` から `muhenkan_is_toggle`/`henkan_is_toggle` を外す（値0/1の警告は残す）。`MsImeKeyAssignment` の2フィールドと `check_and_warn` の `packed` ビットを削除し、テストを更新。案内文言の「単独キーは awase 側に bare で設定すれば…」は値2の利用者には不要になる |
| F | ADR-199 | T10 行・T17 行・決定16・影響表（MS-IME 本体の行）・status を「Phase 4 実装済み」へ。T12 の「トグル割り当てが能動になりうる（値の実機確認後）」を確定表現に |
| G | テスト | §2.5 |

`src/engine/`・`transport.rs`・`hook.rs`・`ime_controller.rs`・`output/` は触らない。

### 2.2 受動→能動の境界（状態別）

「単独タップ」は ADR-192 決定3b の確定点（KeyUp で `PendingThumb` を単独と解決）のこと。同時打鍵（チョード）と解決したら発火しない。

| 状態（無変換/変換の KeyDown 時） | 値2の MS-IME 本体での awase の動作 | 根拠 |
|---|---|---|
| Engine 非活性（IME 閉、半角英数等） | 受動（生キーが IME へ届き、IME 自身が開閉。belief は観測で追随） | 決定16「エンジン非活性は能動にしない」。既存挙動 |
| Engine 活性・入力なし（アイドル、確定直後を含む） | **能動**: 単独タップと解決したら `Toggle` → `Effect::Ime(SetOpen(!ime_on))`。生キーは `PendingThumb` として消費されるので MS-IME は無変換/変換を見ず二重に開閉しない | T12: アイドルは真のトグル（open 1→0、直接入力→0→1） |
| 入力中（composing）・変換中・候補窓表示中 | 役割なし（D で除外）→ 今日と同じ `ModeKeyConfig.for_composing(true)` の経路 | M1（2026-09-28 実機）: 入力中は開閉が発火せず本来動作（かな⇔カタカナ）が優先 |
| 修飾付き押下・injected・config 由来あり | 従来どおり（config 由来が勝つ、injected は役割を引かない） | `thumb_forced_action` の既存条件 |

除外条件を `composing || stage != Stage::None`（OR）にする理由: 除外が外れる方向（実際は入力中なのに「入力中でない」と誤判定）は**入力中の未確定文字列を閉じて捨てる**事故になる。
除外が余る方向は今日の挙動（ユーザーが値2の警告を受けている状態）に戻るだけ。よって安全側の OR。`ime_composition_active_now()` は
`EVENT_OBJECT_IME_SHOW/HIDE` 由来、`key_track().stage` は打鍵履歴からの追跡（ADR-191、変換中3種を含む）で、独立した2つの証拠。

**判定時刻は KeyDown**（Engine に composing を持ち込まない）。単独タップは押している間に文字を打てば同時打鍵になり発火しないので、KeyDown から KeyUp の間に
入力中へ変わる経路は「チョードと解決される」か「キーを押したまま別キーを打たない放置」だけ。Engine の `forced_open_action` は composing を見ない設計（ADR-192 決定3b）のまま保つ。

### 2.3 互換モード（NoTsf3Override2）との関係

- 互換モード ON: `KeyAssignment*` の値は効かない（T12）。B で `msime_compat_mode == Some(true)` なら無変換/変換も受動（半角/全角の決定17と同じ扱い）。
- `None`（フラグが読めない）: 半角/全角の決定17と同じく**トグル側**（値2かつマスタースイッチ ON の明示があるので、読めない環境でも誤って能動にする可能性は低い。
  ただし T1(d) で CI（windows-latest）にフラグ自体が無い既定環境を確認済みなので、`None` は「互換モードを触っていない」と推定する）。
- 互換モードの旧UI（`keystyle=Custom`、ADR-197 の `msime_legacy_keymap`）の無変換/変換は別の読み取り経路で、この Phase 4 の対象外（`KeyAssignmentMuhenkan/Henkan` は新UI）。

### 2.4 GJI との対称性

| 観点 | GJI（配線済み、T10） | MS-IME 本体（Phase 4） |
|---|---|---|
| 役割の一次情報源 | `config1.db`（プリセット＋カスタム表） | レジストリ `IsKeyAssignmentEnabled` ＋ `KeyAssignmentMuhenkan/Henkan == 2` |
| トグルと言える条件 | 全開状態で閉じる（決定11） | 値2（状態に依らずトグル、T12） |
| 学習表による狭め | `TableKey::Muhenkan/Henkan` の `Stage::None` セル | 同じ（`get_native`、指紋に値を含む） |
| 入力中の扱い | **発火する**（決定11: IME 側もこのキーで閉じる） | **発火しない（除外）**（M1: MS-IME は入力中は本来動作が優先） |
| 発火点 | ADR-192 決定3b の単独タップ確定点（`forced_open_action`） | 同じ（合流点を増やさない） |
| Engine 非活性 | 受動 | 受動 |
| 互換モード | 概念なし | 受動（B） |

非対称は「入力中」の1点だけで、それは IME 側の実挙動の違い（M1）に由来する。コード上は D の1分岐（IME 種別で分ける箇所は `derive_key_shadow_action` に続く2箇所目）になる。

### 2.5 fix-requires-evidence の再発ファミリー影響と必要な回帰テスト

| ファミリー | 触るか | 対応 |
|---|---|---|
| IME belief（`state/key_effect_predictor.rs` が表に載っている） | 触る（ただし予測ではなく役割判定のみ。`KeyEffectPredicted` の入力は変えない） | (a) 回帰テスト必須 |
| キー選択（`resolve_pending_thumb_as_single`） | 触らない | Engine 無変更を保つ。ホストの `src/engine/tests.rs` に「forced Toggle は composing=true の KeyUp でも発火する（composing の判定は Windows 層の KeyDown で済んでいる）」の特性化テストを1本足すと契約が固定される |
| 物理IMEキーの Suppress/Allow（`transport.rs::plan`） | 触らない | 無変換/変換には `shadow_action` を付けない（付けると BUG-46 型二重 actuation）。既存 `enrich_thumb_key_role` の doc とテストで維持 |
| IME actuation 合流点 | 新規の呼び出し元なし | 既存 `Effect::Ime(SetOpen)`→`dispatch_ime_set_open` に合流。`RESTRICTED_CALLS`・`.apply_ime_open_with_view(` 件数ガードは不変。complexity-budget ルールの対象外（合流点も tuning 定数も足さない） |
| 物理キー押下ラッチ | 触らない | 親指キーの `thumb_forced_open_actions` は打鍵ごとに書き直す既存方式 |

追加するテスト（すべてホストで動くか、`cargo check --target x86_64-pc-windows-msvc ... --tests` で通るもの）:

1. `state/key_effect_predictor.rs`: 既存 `msime_native_key_role_thumb_keys_stay_passive_until_phase4` を差し替え。マスタースイッチ{ON,OFF} × 値{なし,0,1,2,3} × 互換{None,Some(false),Some(true)} × VK{0x1C,0x1D} の全組み合わせで、`Some(ImeToggle)` になるのは「ON・値2・互換≠Some(true)」だけ。0x1C は henkan、0x1D は muhenkan の値だけを見る（取り違え防止）。
2. `state/key_effect_runtime.rs`: `msime_thumb_toggle_blocked` の真理値表（composing × `Stage` 5値）。`thumb_forced_action` に「ブロック時は役割を引かず config 由来に戻る」の1本。
3. `msime_key_assignment.rs`: 値2のみ→`conflict_warning` は `None`。値2＋値1→値1だけが列挙される。互換モード ON は従来どおり `None`。`check_and_warn` の重複警告ビットの更新。
4. `tests/architecture_guard.rs`: `enrich_thumb_key_role` のソースに `msime_thumb_toggle_blocked` が含まれること（composing ガードが黙って消されないためのソース走査。消えると入力中に閉じる事故になる、M1）。
5. Windows e2e（§3.3）。

## 3. 検証計画

### 3.1 未確認セルと、結果が設計を変えるか

「値2」列は**実機（日本語 UI）でのみ測れる**（§4）。CI 列は既定の割り当て（対照）で測った。

| 状態 | キー | 値2の実機確認 | CI 対照（既定割り当て） | 結果が設計を変えるか |
|---|---|---|---|---|
| 直接入力（IME 閉） | 無変換 | T12 済（0→1） | 無変換は開かない（既定）、変換は 0→1 で開く | 変えない |
| アイドル（IME 開） | 無変換 | T12 済（1→0） | 変化なし（既定） | 変えない |
| 入力中（composing） | 無変換 | M1 済（開閉不変、かな→カタカナ） | 同（`あい`→`アイ`、開閉不変） | これが除外の根拠 |
| 入力中 | 変換 | **未確認** | `あい`→`愛`（漢字変換）、開閉不変 | 除外しているので変えない |
| 変換中（Space 1回） | 無変換/変換 | **未確認** | 文字列は変わる、開閉不変 | 除外しているので変えない |
| 候補窓表示中（Space 2回） | 無変換/変換 | **未確認** | 同上 | 同上 |
| 確定直後（Enter 後） | 無変換/変換 | **未確認** | 開閉不変（既定） | **効く**: 確定直後をアイドルと同じ扱い（能動）にしているので、値2で確定直後に本当にトグルするかは受け入れ確認の必須項目 |

実機で測る手順（日本語 UI の実機。`docs/adr/199` T12 の手順と同じ道具、今回 `msime_native_composing_probe` に `--matrix` を足した版を使う。
足した版は測定用ブランチ `ci/b4-msime-toggle-probe` にあり、develop には入れていない）:

```powershell
# 1. マスタースイッチと無変換/変換を値2にする（設定アプリを UIA で操作、SystemSettings.exe を強制終了する副作用あり）
cargo run -p awase-windows --example msime_key_assignment_settings_probe --release -- --set-master=on "--set-muhenkan=IME-オン/オフ" "--set-henkan=IME-オン/オフ"
# 2. 互換モードが OFF であること（NoTsf3Override2 が無いか 0）を確認してから、全マトリクス
cargo run -p awase-windows --example msime_native_composing_probe --release -- --matrix
# 3. 元の設定へ戻す（--set-master=off、無変換/変換を元の値へ）
```

`--matrix` は 無変換/変換 × {直接入力・アイドル・入力中2種・変換中・候補窓・確定直後(300ms後/20ms後)} を回し、`msime_native_composing_probe_result.json`
に前後の `open`/`comp_str` と押下前後の UTC 時刻を残す。注意: シナリオ間で TSF の composition が完全には初期化されず `before` の文字列が前シナリオの残りを含む
（CI 対照で確認）。**信頼できるのは `open` の遷移で、`comp_str` の差は定性的**。

### 3.2 awase の観測件数（observed）の見方

awase 起動中の計測は `ime_key_matrix_spike --seq`（既存ハーネス、awase がスパイクの窓の IME 状態を観測できる）を使う。
`msime_native_composing_probe` を awase 起動下で回すのは**不適**（§4 R4: 注入した VK_IME_ON に belief が追随せず、`SkipTyping` で observed=0 のまま Engine が非活性になる。
「起きなかった」ではなく「観測経路に乗らなかった」）。observed の定義は `check_drift_recovery.py` と同じ
`[stage-observe] observer_poll=Some|ObserverReported`、手順ごとの窓は `[その押下, 次の押下)`。

### 3.3 Phase 4 実装後の e2e（受け入れ）

| 何を | どこで | 備考 |
|---|---|---|
| 値2でアイドル/確定直後に単独タップ→ awase が閉じ、二重に開閉しない | **実機**（日本語 UI、§3.1 の手順で値2にして `ime_key_matrix_spike --seq` か手動） | CI で値2を作れない間はここが唯一 |
| 入力中の単独タップで開閉しない・未確定文字列が残る | 同上 | M1 の再確認（awase 起動下） |
| awase 側の配管（役割→`forced_open_action`→開閉→belief 追随）のみ | CI 可能（案あり、論点3）: テスト専用にキー割り当てを注入する、または `keys.ime_toggle = ["VK_NONCONVERT"]` の明示 config で MS-IME 本体に対して既存の ADR-192 決定3b 経路を回す | 後者は役割判定（Phase 4 の本体）を通らないので配管確認に留まる |

## 4. 実測結果（GitHub Actions windows-latest、測定用ブランチ `ci/b4-msime-toggle-probe`）

環境: windows-latest、UICulture=en-US、`NoTsf3Override2` なし（互換モード OFF 相当）、`KeyAssignment*` なし、TIP 登録済み（0秒）。
測定用ブランチ（develop へマージしない）に追加したもの: `crates/awase-windows/examples/msime_native_composing_probe.rs` の `--matrix`、
`.github/workflows/b4-msime-toggle-probe.yml`（e2e-ime.yml とは別の小さいワークフロー。`gh workflow run` は既定ブランチにワークフローが無いと使えないため、branch push で起動）、
`tools/e2e/ime_key_matrix/check_b4_toggle.py`・`check_b4_seq.py`。

| run | 内容 | 結果 |
|---|---|---|
| 36541369039 | 対照（既定割り当て）・UIA でマスター/コンボ探索・レジストリ直書き（`IsKeyAssignmentEnabled=1`,`Muhenkan=Henkan=2`＋ctfmon 再起動）・awase 起動下の `composing_probe` | 下記 R1〜R4 |
| 36542333857 | UIA の Key template 探索（PowerShell 5.1 が非ASCII で構文エラーになり空振り）＋ `ime_key_matrix_spike --seq`（awase なし/あり × 無変換/変換） | seq は有効（R3）。UIA は空振り |
| 36543097358 | 上の修正版（pid で窓を引いたが `SystemSettings` の窓は ApplicationFrameHost 配下で null） | `no window`。seq は再現（R3） |
| 36543901149 | 窓を名前＋子孫数で引く版 | R2 の確定 |

**R1 レジストリ直書きは反映されない（T12 の CI 再確認、n=1）**: 値2＋マスタースイッチ ON を直書きして ctfmon を再起動した後の全マトリクスが、対照と `open` 遷移まで一致。
**R2 CI の英語 UI では「キーの割り当て」が設定アプリに出ない**: 「Key & touch customization」ページの項目は Key template（選択肢は `Microsoft IME` と `ATOK` の2つ）と Touch keyboard だけで、
マスタースイッチも無変換/変換のコンボも UIA ツリーに無い（run 36541369039 のダンプ、36543901149 で選択肢を列挙）。`keystyle=NATURAL` がレジストリにある。
UI 言語は job 内では変えられない（サインアウトが要る）ので、CI での値2は当面作れない。
**R3 既定割り当てでの awase の観測件数（n=2 run × 2キー）**: `ime_key_matrix_spike --seq=F2,K,K,41,49,K,20,K,0D,K`（K=無変換 0x1D または変換 0x1C）。
awase なしの実 IME: F2 で 0→1 のあと、K・Space・Enter のどの手順でも `open` は 1 のまま（既定割り当てでは開閉しない）。awase あり: 同じ `open` 遷移で、
observed（`observer_poll=Some` または `ObserverReported`）は F2 の手順が 9〜10 件、K の手順が 0〜6 件（多くは 2〜4）。戦略の内訳は `OsPoll` と `SkipTyping` が混在（打鍵直後は `SkipTyping`）。
Engine は F2 の手順で活性化（`A`）し、以降 K の KeyDown は `decision="Consume"`（`ime_on=true`）で消費された。「observed>0 でも `SkipTyping` の窓は判断に届いていない」ことに注意。
run 36542333857 と 36543097358 で差は ±2 件程度（36543097358 では最後の K が無変換・変換とも 0 件で、`SkipTyping` の窓だけだった）。
**R4 awase 起動下の `composing_probe` は観測経路に乗らない（無効な計測）**: 全16手順で observed=0、押下窓内の Engine 活性化ログ0、strategy は `SkipTyping` が各窓0〜1件、
無変換/変換の判定は `PassThrough`（Engine 非活性）か `Consume`（変換中・候補窓）。これは「awase が観測しなかった」ではなく「注入した VK_IME_ON が BUG-14 の原則で意図に昇格せず、
`SkipTyping` のため belief が追随せず Engine が非活性のまま」だったことによる。結論を出す材料にしない。

追加の観察（対照、既定割り当て、n=1）: 変換キー（0x1C）は直接入力から押すと IME が開く（0→1）が、無変換（0x1D）は開かない。
入力中は 無変換=かな→カタカナ（`あい`→`アイ`）、変換=漢字変換（`あい`→`愛`）でどちらも `open` 不変。
**composing 信号の有効性は未確定**: R3 の awase 起動下で、A・I を打った後の無変換 KeyDown の `ctx.composing` は false だったが、この時 awase の Engine が A・I をかなに変換して
出力していた（`Consume`）ので、実際に IME の未確定文字列があったか自体が不明。§2.2 の OR を採る根拠（片方の信号だけに頼らない）を補強するが、`ime_composition_active_now()` が MS-IME 本体で
入力中に真になるかは、実装時に診断ログ（KeyDown 時の `composing` と `stage`）を入れて実機で確認する。

## 5. 実装の進め方（案）

- PR は1本、コミット2つ（(1) 役割判定＋composing ガード＋テスト、(2) 警告削除＋ADR-199 更新）。差分は数十〜二百行程度。専用 worktree/branch（`develop` 先端から）。
- コミット前に `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows --tests --lib`、ホストで `cargo test --lib`・`cargo nextest run -p awase-windows --test architecture_guard`。
- v1 ライン（`v1-develop`）へは**入れない**（v2 の機能追加で、`v1-develop` は保守専用）。
- 実装後の受け入れ: §3.3 の実機確認（所有者の実機、またはユーザー承認済みの clipwire 経由）。

## 6. 所有者に決めてほしい論点

1. **入力中の除外を、Phase 4 の必須の安全条件として固定してよいか**（`composing || stage != None` の OR、判定は KeyDown）。除外時は今日と同じ挙動に落ちる。
2. **半角/全角（0xF3/0xF4）の入力中の扱い**: 現状は静的 `Toggle`（入力中も付く）。M1 は無変換だけの実測で、MS-IME 本体の半角/全角が入力中にどう動くかは未確認。
   Phase 4 と同じ実機確認（マトリクスに 0xF3/0xF4 を足すだけ）で測れるが、範囲を広げてよいか。
3. **CI で値2の配管を確認する手段**: (a) 実機でだけ確認する、(b) テスト専用の注入（`AWASE_TEST_INJECTION` のような環境変数でキー割り当て 2,2 を与える。製品バイナリに検査用の分岐が1つ入る）、
   (c) 実機で設定アプリの操作前後のレジストリ差分（`reg export` の diff）を取って**本当の保存先**を特定し、CI から書く（T12 の「直書きは反映されない」は、キーが別にあるか、通知が要るかの可能性がある）。
   推奨は (a)＋(c) を先に、(b) は (c) が失敗したときのみ。
4. **値0/1（方向固定の IME-オン/オフ）も同じ配線で能動にするか**: `ShadowImeAction::On/Off` の `forced_open_action` はそのまま使えるので追加は数行だが、所有者決定の範囲は値2のみ。今回は範囲外にして残す。
5. **`Toggle` の解決は belief 依存**（`action.resolve(ctx.ime_on)`）。MS-IME 本来のトグルは実状態に対して働くので、belief がずれている間は結果が食い違いうる。GJI（T10）と同じ性質で、
   ここでは GJI と同水準として受け入れるか。
6. **値2の警告（`conflict_warning`）を Phase 4 と同じ PR で外すか**: 推奨は同じ PR（外し忘れると、awase が肩代わりする設定に「競合」と警告し続ける）。
   逆に、実機確認が済むまで警告を残す運用（能動化はするが警告は残す）も可能で、その場合は警告文言を「awase が肩代わりします」へ変える。
7. **実機確認の実施者**: 所有者の実機で手動、または承認済みの clipwire ターゲット経由で §3.1 の手順を実行（この計画のセッションでは、所有者の実機の設定・TSF プロファイルを変えるため実行していない）。

## 7. 参照

- ADR-199 決定16・決定17・T10・T12・T17: `docs/adr/199-derive-key-roles-from-user-ime-keymap.md`
- 実装箇所: `crates/awase-windows/src/state/key_effect_predictor.rs`（`msime_native_key_role`, `for_msime_native`）、`crates/awase-windows/src/runtime/mod.rs`（`enrich_thumb_key_role`, `derive_key_shadow_action`）、
  `crates/awase-windows/src/state/key_effect_runtime.rs`（`thumb_forced_action`, `key_shadow_action`）、`crates/awase-windows/src/msime_key_assignment.rs`、`src/engine/nicola_fsm.rs`（`resolve_pending_thumb_as_single`）、`src/engine/engine.rs`（`apply_ime_open_request`）
- 既存ハーネス: `crates/awase-windows/examples/msime_native_composing_probe.rs`, `msime_key_assignment_settings_probe.rs`, `ime_key_matrix_spike.rs`（`--seq`）, `tools/e2e/ime_key_matrix/check_consistency.py`
- CI run: 36541369039, 36542333857, 36543097358, 36543901149（`ci/b4-msime-toggle-probe`、artifact `b4-logs`、保持7日）

## 8. 追加実測（所有者決定 2026-09-29 を受けた CI 検証、run 36545534536 / 36546369830 / 36547197438 / 36550383544）

所有者決定: 入力中/変換中/候補窓の除外（`composing || stage != None`、KeyDown 時判定）を安全条件として固定、実機確認は CI で行う、能動は値2のみ、
Toggle の belief 依存は許容（固着＝何度押しても変わらない、を不具合と定義）、値2の警告は同じ PR で外す。半角/全角の入力中は計測のみ。
この節の結論: **Phase 4 は実装に進めない**（理由は 8.1 と 8.2 の2つ、どちらも実装前に解決が要る）。

### 8.1 CI で値2を作れない（(a) 未達）

ADR196-T2 の知見（ja-JP のみ＋MS-IME TIP のリスト）を流用し、設定アプリ（`ms-settings:regionlanguage-jpnime` →「Key & touch customization」）に
`IsKeyAssignmentEnabled` のトグルが出るかを段階的に試した（UIA の AutomationId で判定。`Has-Section` の出力汚染による偽陽性を run 36547197438 で修正）:

| 段階 | 操作 | 結果 |
|---|---|---|
| ベース | 言語リスト＝ja-JP のみ＋MS-IME TIP（`Set-WinUserLanguageList`）、`Set-WinDefaultInputMethodOverride`、`Set-WinUILanguageOverride ja-JP` | ページは Key template（Microsoft IME / ATOK）と Touch keyboard のみ。UICulture は en-US のまま |
| S1 | `Set-WinHomeLocation -GeoId 122`（日本）＋`Set-Culture ja-JP` | 変化なし（`SystemSettings_Language_JapaneseIME_` の ID は KeyTemplate/Kana10KeyInputMode/HowToUseLink の3つだけ） |
| S2 | JP106 キーボード上書き（`i8042prt\Parameters` の `LayerDriver JPN`/`OverrideKeyboardType=7`/`Subtype=2`） | 変化なし（再起動なしなので反映されない可能性あり。未確定） |
| S3 | `Install-Language -Language ja-JP`（run 36547197438、15分でタイムアウト） | 完了せず。UI 言語は job 内でサインアウトなしには切り替わらない |
| レジストリ差分 | 設定アプリ操作前後の `HKCU\Software\Microsoft\IME`/`Input` の `reg export` 差分 | 操作できる項目が無く差分ゼロ（本当の保存先は特定できず） |

残る手段は (i) 日本語表示言語パックの導入とサインアウト/再起動を含む job 分割（windows-latest は Server 系で `Install-Language` が完走しない）、
(ii) 実機での設定アプリ操作前後のレジストリ差分。**どちらも CI 単独では届いていない**。直書き（run 36541369039）は反映されない（T12 と一致）。

### 8.2 composing 信号は MS-IME 本体で偽のまま（安全条件の前提が崩れる）

`msime_native_composing_probe --matrix` を **awase 起動下**（Engine 非活性、生キーが IME に届き実際に未確定文字列ができる状態）で回し、無変換/変換/半角/全角の
KeyDown 時の `[engine-input] ... composing=`（`ime_composition_active_now()`、`EVENT_OBJECT_IME_SHOW/HIDE` 由来）を、同じ手順で IMM32 が返す `comp_str` と突き合わせた
（run 36541369039 と 36550383544、計2回）:

- `comp_str` が `あい`（入力中）・`愛会`（変換中）・`アイアイ会`（候補窓）と**実際に非空**の全シナリオで、KeyDown 時の `ctx.composing` は **false**（全32手順、2 run とも）。
- 観測件数は全手順 observed=0（`SkipTyping`。注入した VK_IME_ON が意図に昇格せず belief が追随しないため Engine は非活性）。これは「観測経路に乗らなかった」であり、
  信号が偽なのは `composing` の直接ログ値で確認している（observed とは別の証拠）。

つまり所有者が固定した除外条件 `composing || stage != None` のうち `composing` 側は MS-IME 本体では**入力中でも立たない**。`stage`（`key_track` の隠れ状態）は
打鍵履歴からの追跡で、**Engine が消費して自分で出力した文字（NICOLA の打鍵）は通したキーの追跡に入らない**（`kp_predict_key_effect` は通したキーだけを更新する）。
NICOLA で入力した直後の入力中は `stage == None` のままになりうる。この2つが両方偽だと、除外が効かず**入力中に単独タップで IME を閉じ、未確定文字列を捨てる**
（M1 が警告した事故）。安全条件を実装で満たすには、少なくとも次のどれかが必要:

1. awase 自身が「最後の確定/取消キー以降に出力した文字がある」を持つ（Engine の出力状態から導く。IME 側の自動確定は見えない）。
2. MS-IME 本体の未確定文字列を直接読む（UIA の `IUIAutomationTextEditPattern::GetActiveComposition`、`run_with_timeout` 配下。新しい観測機構）。
3. Phase 4 の対象を「直前に awase が文字を出力していない」ことが分かる状態に限る（保守的、実用範囲は狭い）。

いずれも新しい機構で、「新しい観測/belief 機構を足さない」という Phase 4 の前提を超える。**所有者の判断が要る**。

### 8.3 半角/全角の入力中（計測のみ、既定割り当て、awase 起動下、run 36550383544）

MS-IME 本体の F3/F4（0xF3/0xF4）は、入力中・変換中・候補窓表示中は `open` も未確定文字列も変わらず（NO_EFFECT）、確定後（Enter の後）は 1→0 に閉じた。
F4 はアイドル（コンテキストが空のとき）でも 1→0 と直接入力から 0→1 を確認。つまり **半角/全角も入力中は発火しない**（無変換と同じ）。現状の静的 `Toggle`
（`derive_key_shadow_action` 経由の `shadow_action`、入力中も付く）は、入力中の半角/全角で awase が閉じる書き込みをすると MS-IME 本来の動作と食い違う可能性がある
（この行は前シナリオの未確定文字列が残る汚染があり `open` 遷移のみ信頼、n=1）。別 PR で扱うか、Phase 4 と同じ除外を半角/全角にも適用するかを決める必要がある。

### 8.4 結論と次の一手

- 値2を CI で作れず、composing 信号も偽のため、「結果が安全条件を支持する」を満たせない。**実装（feat/v2-msime-toggle-phase4）は作成していない。**
- 次の一手の候補: (A) §8.2 の1〜3のどれで入力中を検出するかを決めて Phase 4 の設計に組み込む（推奨は 1 と 2 の併用を小さく試作して CI で信号を測る）、
  (B) 値2の作成は実機（日本語 UI）でのレジストリ差分の取得を先にやる、(C) Phase 4 を v2 から外し、値2の利用者には警告（現状）を維持する。
- 測定用ブランチ `ci/b4-msime-toggle-probe` は develop にマージしない（`--matrix` に半角/全角を追加、`phase1-pre` の環境探索、`check_b4_*.py`）。

### 8.5 所有者の最終方針と実装（PR #379）

所有者決定（2026-09-29）: 「入力確定文字列を捨てていい。安全にする必要自体がありません」。よって 8.2 の入力中の除外（`composing`/`stage`）は**不要**とし、
`ctx.composing` が偽固定だった問題も扱わない（§2.2 の除外条件と 8.4 の (A) は破棄）。新方針: MS-IME 本体で値==2 のとき無変換/変換を、状態（休止中・入力中・変換中・候補窓）に
関係なくトグルに該当するキーとして扱い、GJI と同水準で awase が belief に従う明示 ON/OFF を注入する（ADR-206 の枠組み）。

実装は feat/v2-msime-toggle-phase4（PR #379）: `KeyEffectKeymap` に値==2 のフラグ、`msime_native_key_role` の無変換/変換の腕（互換モードは受動）、`conflict_warning` から値2を削除、
回帰テスト（役割判定の全組み合わせ、警告文、`architecture_guard` の必須トークン）、ADR-199 T17 行の更新。値2は CI で作れず**未検証・ホストテストのみ**（所有者了承）。
