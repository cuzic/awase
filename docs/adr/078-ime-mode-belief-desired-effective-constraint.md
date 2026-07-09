# ADR-078: IME conv-mode belief の三分割（DesiredMode / EffectiveMode / ModeConstraint）と観測駆動書き込みの排除

## ステータス

一部実装済み（Phase 1a、2026-07-09、実機検証待ち）。全体の型設計（`DesiredMode`/
`EffectiveMode`/`ModeConstraint` 分割、`ModeEvent`/`ModeEffect`、トレイの明示的
intent 化、config1.db 対応）は未実装のまま提案中。

Phase 1a（増幅ループの実質的な撤去のみ、型分割なしの最小対策）は
`ConvModeMgr::needs_conv_restore_write`/`mark_conv_restore_written` として
実装済み。詳細は `docs/known-bugs.md` BUG-19 追補3を参照。

## コンテキスト

### 繰り返されてきた症状

Chrome/Edge（`Imm32Unavailable` プロファイル、GJI 使用）で、ユーザーが何も
変換操作をしていないのに conv mode（カタカナ/英数/JISかな/ひらがな）が
勝手に切り替わったように見え、入力が壊れる不具合が数ヶ月にわたり繰り返し
再燃した: BUG-17（CLSID ポーリング誤検出で `GjiFsm` 丸ごと再構築）、
BUG-18（無操作中の AppKind 往復で入力欠落）、BUG-19（一発カタカナ誤読が
ロックイン）、BUG-22（Uwp⇔TsfNative 往復後の Eisu 固着）。

そのたびに「同じ値を2回連続観測するまで確定しない」式のデバウンスを
個別に追加する対症療法を繰り返してきた（`docs/known-bugs.md` 参照）。

### 根本原因の再調査（2026-07-09）

本 ADR に先立つ調査で、以下が判明した:

1. **conv 値の外部観測の初回トリガーは未解明のまま**。「`GetForegroundWindow()`
   が候補ポップアップを一瞬指す」という `docs/known-bugs.md` の説明は、
   ドキュメント自身が「Windows 実機での再現待ち」と明記する未検証の仮説
   であり、引用されたタイムラインもフォーカス往復から conv 誤読まで
   28 秒の間隔があるなど根拠が弱い。
2. **ロックイン・増幅の実体はコードで確認できた**: `tsf/warmup/cold_warmup.rs`
   の `preamble()` は cold warmup のたびに（＝`GjiFsm::FocusChange` のたびに、
   BUG-18 のスプリアスなフォーカス往復を含む）、`self.output.conv_mode.get()`
   （awase 自身の belief）を見て `set_ime_romaji_mode_with_target_async()`
   で real IME へ実際に conv ビットを書き込む。同様の書き戻しが
   `key_pipeline.rs`、`probe_io.rs` にもある。つまり「一発の誤読がロック
   インする」の実態は、**awase 自身の復元機能が誤った belief を cold
   warmup のたびに real IME へ再書き込みし続ける自己参照ループ**である。
3. **`ConvModeAuthority`（`state/conv_mode.rs`）は関連するが別の粒度の
   ゲート**。エンジン ON/OFF に応じて「conv mutation を許可するか」を
   決める粗い gate であり、本 ADR が扱う「何を・なぜ書くか」の細粒度
   制御とは直交する。本 ADR の設計はこの gate の内側で動く。

### 設計方針の転換: 観測を信じない、awase 自身の追跡を権威にする

このリポジトリは ON/OFF（`desired_open` → `shadow_model`）と warm/cold
（`GjiFsm`）については、既に「awase 自身の追跡・意図を権威にする」方向へ
複数回の refactor で移行済みである。一方 **input_mode（conv/charset 分類）
だけは一度もこの移行を経ておらず**、`InputModeObserved` + confidence +
デバウンスという「観測を信じる」モデルのまま BUG-17/18/19/22 の温床に
なり続けている。この非対称性を解消する。

ただし「外部観測を一切使わない」という単純な二値化は、以下の理由で
不十分と判断した（codex および ChatGPT によるレビューで指摘）:

- アプリがパスワード欄等で正当に Eisu（半角英数）を強制するケースを
  救えなくなる。
- 言語バーからの手動切替を使うユーザーとの乖離が解消不能になる。
- 「観測された Eisu を無条件で信じて `desired` を上書きする」という
  非対称設計も、同じ誤読経路が将来 Eisu 方向にも誤読する可能性がある
  以上、危険である。

## 決定

### 適用範囲を `AppImeProfile` でスコープする（Standard は対象外）

本 ADR の新モデル（intent-authoritative、観測は `EffectiveMode` 止まり）は
**`Imm32Unavailable` / `TsfNative` プロファイルにのみ適用する**。

`AppImeProfile::Standard`（レガシー IMM32 が素直に使えるアプリ）では、
`focus/class_names.rs:154` のコメントが既に「IMM32 の状態値は信頼できない」
の対象を `Imm32Unavailable`/`TsfNative` に限定している通り、conv 観測は
そもそも信頼できる前提がある。今回発見した増幅ループ（`cold_warmup.rs`
の書き戻し）も、TSF 候補ポップアップ/`InputSite` 構造を持つ Chrome/Edge/
GJI 特有の経路であり、Standard アプリにはこの構造自体が存在しない。

したがって Standard プロファイルでは**現状の観測駆動モデルをそのまま
維持する**（`EffectiveMode` の観測が `DesiredMode` へ自由に昇格してよい）。
新設する intent-authoritative な制約・サーキットブレーカー・「現在の
モードを採用」救済コマンドは Imm32Unavailable/TsfNative 専用の実装で
よく、Standard 側の既存コードパスは変更しない。これにより実装範囲・
リグレッションリスクの両方が縮小する。

`belief` の型（`DesiredMode`/`EffectiveMode`/`ModeConstraint`）自体は
プロファイル共通で構わないが、reducer が適用する**ポリシー**（観測を
`DesiredMode` に昇格させてよいか）をプロファイルごとに切り替える形にする。

### belief を 3 つに分離する

単一の `ConvModeMgr`/`InputModeState` に混在していた意味を、責務ごとに
分離する。

```rust
/// ユーザーが「このモードにしたい」と awase が判断している状態。
/// 変更してよいのは物理IMEキー・awaseトレイ操作・「現在のモードを採用」
/// コマンド・awase 自身のモード切替コマンドのみ。
struct DesiredMode {
    mode: ConvMode,
    source: UserIntentSource,
    sequence: u64,
}

/// 実際の IME が現在どうなっているらしいか、という観測値。
/// 確認・診断専用。これを根拠に DesiredMode を書き換えたり、
/// IME へ再書き込みしたりしない。
struct EffectiveMode {
    mode: Option<ConvMode>,
    confidence: Confidence,
    source: ObservationSource,
    focus_epoch: FocusEpoch,
}

/// 現在のコントロールがアプリの都合で一時的に要求している制約
/// （パスワード欄の Eisu 等）。DesiredMode を消さずに上に被さる。
struct ModeConstraint {
    mode: Option<ConvMode>,
    source: ConstraintSource,
    focus_epoch: FocusEpoch,
}
```

`ModeConstraint` を導入することで、パスワード欄を抜けたときに
`DesiredMode`（例: ひらがな）がそのまま残っている、という復元が
自然に表現できる。これは現行の `key_pipeline.rs` の Shift 解放復元
コードがアドホックに個別対応していた問題（BUG-15 関連）を一般化した形。

### イベント / Effect を型で限定する

「観測イベントだけでは書き込み Effect が発生しない」ことをコンパイラで
強制するため、reducer が受理するイベントと発生させる Effect を分離する。

```rust
enum ModeEvent {
    UserPhysicalKey { key: NormalizedImeKey, focus_epoch: FocusEpoch },
    UserTrayIntent { mode: ConvMode },
    UserAdoptCurrentMode,                 // 言語バー派への救済路（後述）
    AppConstraintObserved { constraint: ModeConstraint, confidence: Confidence, focus_epoch: FocusEpoch },
    ModeObserved { mode: Observed<ConvMode>, focus_epoch: FocusEpoch },  // EffectiveMode のみ更新
    FocusChanged { focus_epoch: FocusEpoch, target: FocusTarget },
    KeyMapReloaded { generation: KeyMapGeneration },
}

enum ModeEffect {
    ApplyExplicitIntent(ConvMode),     // Desired 由来、real IME へ書く
    ApplyOneShotCorrection(ConvMode),  // 後述のサーキットブレーカー参照
}
```

`ModeObserved` と `FocusChanged` は単体では `ModeEffect` を一切発生させない
（コード上、reducer のこの2ケースから `ModeEffect` を返すパスが存在しない
ことを型・テストで保証する）。これが `cold_warmup.rs` の増幅ループを
構造的に再発不能にする核心。

### 物理IMEキーの shadow tracking と confirm のサーキットブレーカー

`vk.rs::ImeKeyKind::shadow_effect()` は現状 ON/OFF 軸（`TurnOn`/`TurnOff`/
`Toggle`）しか区別しておらず、`Katakana`/`Activate`(ひらがな)/`ActivatePair`
(全角) はすべて同じ `TurnOn` に潰されている。charset 軸の shadow tracking
はこの ADR で新規に追加する。

物理キー押下を検知した時点で `DesiredMode` は即座に確定するが、
`EffectiveMode` との一致は非同期に確認する。不一致が続く場合は
盲目的に再試行し続けず、段階的に降格する:

```
Controlled（confirm-retry で収束を試みる）
  → Uncertain（不一致を検知、1回だけ補正書き込み）
  → ObserverOnly（同一 epoch/短時間で複数回不一致 → 自動書き戻し停止、
                   入力は通す、トレイ表示で通知）
```

**重要な不変条件**: 同一の物理キー押下から発生する補正書き込みは最大1回。
フォーカスが往復するたびに再試行しない（`cold_warmup.rs` の旧ループが
まさにこの条件を欠いていた）。

補正の SET 操作は冪等なもの（`IMC_SETCONVERSIONMODE` 等の絶対値指定）に
限定し、`VK_KANJI` のようなトグル系コマンドを confirm-retry の対象には
しない（トグルは不一致時の再送で逆方向に飛ぶリスクがあるため）。

### GJI: config1.db によるキー解釈（Phase 2、初期実装はデフォルトマッピング）

物理キーがどの conv コマンドに対応するかは、GJI 側でユーザーがカスタマイズ
可能なため、`config1.db` を読んで正確に解釈する。

**構造**: `config1.db` は `mozc.config.Config` をシリアライズした protobuf
ファイル。`custom_keymap_table` フィールド（`bytes`）の中身がタブ区切り
テキストの `(状態, キー, コマンド)` 表（`status\tkey\tcommand\n` マーカー
+ 各行）。旧 `crates/awase-windows/src/gji.rs`（`8c39b8e` で削除済み、
config1.db への書き込みパッチ方式を廃止した際に消えた）の `find_block()`
がこのブロックの位置検出ロジックを既に持っていたため、読み取り用に流用可能。

**落とし穴（実装時に必ず対応する）**:

- `session_keymap != CUSTOM` の場合 `custom_keymap_table` は空/無関係。
  Mozc の組み込みキーマップ（MS-IME 互換/ATOK/ことえり等）を別途
  vendor する必要がある。**Phase 1 ではこのケースを `KeyMapAvailability::
  KnownBuiltin` として扱い、組み込みキーマップテーブルを埋め込む。**
- コマンドは `(現在の conv, 物理VK) → 新しい conv` に単純に縮約できない。
  Mozc のコマンドは `PersistentModeChange` / `ToggleOpenState` /
  `CompositionOnly`（未確定文字列のみ変換、モードは変えない）/
  `ConversionOnly` / `CancelOrCommit` / `Unknown` に分類され、
  `PersistentModeChange` 以外は `DesiredMode` を変更しない。
- ファイルは atomic rename で更新される。`FILE_SHARE_READ |
  FILE_SHARE_WRITE | FILE_SHARE_DELETE` で開き、変更通知後は debounce
  してから再 open、parse 成功後に `Arc<KeyMapSnapshot>` を immutable
  swap する。parse 失敗時は last-known-good を破棄しない。
- config.proto は Google の正式サポート ABI ではない（Mozc は非公式）。
  特定コミットに固定して必要フィールドのみ vendor し、未知フィールド/
  state/key/command は fail-closed（`KeyMapAvailability::Unavailable`）
  にする。精度向上の adapter であり、制御システム全体の成立条件には
  しない。

**MS-IME**: キーカスタマイズ機能が乏しいため、ハードコードのデフォルト
マッピングで十分。ただし「絶対にカスタマイズされない」わけではない
（キーボードレイアウト、OS リマップ、PowerToys 等のリマッパー、RDP 等で
崩れ得る）ため、バージョン管理された「MS-IME デフォルトプロファイル」
として扱い、ユーザー上書きの余地を残す。

### Eisu（アプリ強制）パススルーは ModeConstraint 経由に変更

旧提案（Eisu 方向の観測だけ無条件で `desired` に反映）は撤回し、
`ModeConstraint` 経由にする。`AppConstraintObserved` は強い証拠がある
場合のみ `ModeConstraint` を更新し、`DesiredMode` には触れない。
Eisu 以外の方向（全角カタカナのふりがな欄、ひらがなの読み仮名欄、
半角英数の郵便番号欄等、`InputMethod.PreferredImeConversionMode` /
`InputScope` で正当にアプリが要求し得るモード）も同じ `ModeConstraint`
機構で表現できるようにし、「Eisu だけ特別扱い」というモデルの歪みを避ける。

制約解除時の復元は、新しい `focus_epoch` が確定し、新ターゲットに
制約がなく、同じ epoch でユーザーの入力意図が発生した場合のみ行う
（フォーカス遷移中の中途半端な復元を避ける）。

### 言語バー利用者への救済路（Imm32Unavailable/TsfNative 限定）

この弱点は Imm32Unavailable/TsfNative プロファイルにのみ存在する
（Standard は観測が引き続き自動追随するため言語バー操作もそのまま
反映される）。Imm32Unavailable/TsfNative では外部観測を belief に
反映しなくなることで、言語バーから手動でモードを変えたユーザーとの
乖離が解消不能になる UX 上の弱点がある。デフォルトは awase 優先のまま、
トレイメニューに **「現在の IME モードを採用」**（`UserAdoptCurrentMode`）
コマンドを 1 つ追加し、明示操作のときだけ `EffectiveMode` の現在値を
`DesiredMode` に昇格できるようにする。

### トレイメニューの明示的 intent 化（既存の欠陥修正）

現状 `runtime/message_handlers.rs` の `TrayCommand::ImeHiragana` 等は
`crate::ime::set_ime_mode()` という生 Win32 API 呼び出しのみを行い、
belief には一切通知していない（次の observation 頼み）。これを
`UserTrayIntent` dispatch と同時に行うよう修正する。

## 実装ファイル一覧（想定、実装時に確定）

| ファイル | 変更内容 |
|---------|---------|
| `state/conv_mode.rs` | `ConvModeMgr` を `DesiredMode`/`EffectiveMode`/`ModeConstraint` の3構造体に分離。`ConvModeAuthority` はそのまま外側の gate として残す |
| `state/ime_event.rs` | `ModeEvent` バリアント追加（`UserPhysicalKey`/`UserTrayIntent`/`UserAdoptCurrentMode`/`AppConstraintObserved`/`ModeObserved`/`KeyMapReloaded`） |
| `state/conv_classify.rs` | `KatakanaShadowOff`/`NativeToggleShadowOff` の観測駆動 engine 同期を撤去。`ModeConstraint` ベースの判定に置換 |
| `tsf/warmup/cold_warmup.rs` | `preamble()` の「belief を毎回 real IME へ書き戻す」処理を、物理キー起点の一発補正のみに限定 |
| `runtime/key_pipeline.rs`, `runtime/executor.rs`, `output/probe_io.rs` | 同上の書き戻し呼び出しサイトをサーキットブレーカー付きに統一 |
| `vk.rs` | `ImeKeyKind` に charset 軸（カタカナ/ひらがな/半角/全角/JISかな）の shadow tracking を追加 |
| `runtime/message_handlers.rs` | トレイの `TrayCommand::Ime*` ハンドラで `UserTrayIntent` を dispatch |
| `tray.rs` | 「現在のIMEモードを採用」メニュー項目を追加 |
| （新設）`gji_keymap.rs` 相当 | Phase 2: config1.db 読み取り・パース・`KeyMapSnapshot`。旧 `gji.rs`（`8c39b8e` で削除）の `find_block()` を流用 |

## フェーズ分割

- **Phase 1**: belief 3分割、`ModeEvent`/`ModeEffect` の型的分離、
  `cold_warmup.rs` 等の増幅ループ撤去、トレイの明示的 intent 化、
  「現在のモードを採用」コマンド追加、MS-IME デフォルトマッピングでの
  物理キー shadow tracking。**config1.db には依存しない。**
- **Phase 2**: GJI `config1.db` 読み取り対応（`session_keymap` 判定、
  組み込みキーマップ vendor、コマンドタクソノミー、ファイル監視/atomic
  swap）。Phase 1 が「GJI はデフォルトマッピング前提」で動作した上での
  精度向上として追加する。

## 不変条件（実装時にテスト/型で強制する）

1. `ModeObserved` イベント単体では `ModeEffect` が発生しない。
2. `FocusChanged` イベント単体では `ModeEffect` が発生しない。
3. 1つの明示 intent（物理キー/トレイ）から発生する補正書き込みは最大1回。
4. `AppConstraintObserved` は `DesiredMode` を消さない。
5. 未知の GJI コマンド（`Unknown` 分類）では `DesiredMode` を変更しない。
6. config1.db 読み込み失敗時に last-known-good を破棄しない。
7. 古い `focus_epoch` の観測は `EffectiveMode` を確定させない
   （ADR-077 の epoch admission パターンを流用）。

## 関連 ADR

- ADR-029: IME 状態検出の耐障害性と SSOT 設計 — `ime_force_on_guard` は
  「恒常的な SSOT ではなく一時的な遷移期間限定のガード」と明記。ON/OFF
  については踏み込まなかった「恒常的 SSOT 化」を、本 ADR は input_mode
  について実施する。
- ADR-074: ObservedEisu 自動直接入力切替 — 本 ADR の `ModeConstraint`
  導入後は、Eisu 検出の一部がこちらから `ModeConstraint` 経由に移行し得る。
- ADR-075: ImmCrossProbe による belief 補正（`derive_open()` の設計）
- ADR-077: ObservationAdmission Layer — FocusEpoch による probe 受理
  ポリシー。本 ADR の `EffectiveMode` epoch フィルタはこのパターンを
  conv-mode 側に適用したもの。
- `docs/known-bugs.md` BUG-17/18/19/22 — 本 ADR が解消を狙う症状群。
- `.claude/rules/ime-belief-architecture.md` — Observe → pure decision
  → belief の三層分離規約。本 ADR はこの規約に `ModeConstraint` という
  新しい正当な書き込み経路を追加する形で整合させる。
