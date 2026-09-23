---
id: ADR-197
title: |-
  MS-IME「以前のバージョンのMicrosoft IMEを使う」互換モードの詳細キーカスタマイズを、
  実行時警告（不具合報告添付だけでなく）と学習/較正機能（ADR-195/176）の両方に対応させる
summary: |-
  `msime_legacy_keymap.rs`（ADR-148 Phase 2）は、旧UI（互換モード限定の詳細キーカスタマイズ、
  `HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME\keystyle`+`StyleList\<style>\key`）で無変換/変換
  キーに「IMEオン/オフ」トグルが割り当てられているかを検出できるが、**不具合報告への添付（読み取り専用の
  診断）にしか使われていない**。これは2箇所で非対称を生んでいる: (1)新UI側の同種検出
  （`msime_key_assignment.rs`）は不具合報告添付に加えて`WM_IME_KIND_CHANGED`確定時の警告ダイアログ
  （`check_and_warn`）を持つが旧UI側には無く、ユーザーは不具合報告を出すまで競合に気づけない。
  (2)ADR-195（キーマップ学習の製品化、草案）の段階0「既知構成の初期仮説」・段階6「学習を積極的に
  案内するか」・ADR-176 T12の段階8「較正結果の陳腐化検出（要再検証判定）」は、いずれもMS-IME本体の設定変化を
  `msime_key_assignment::current_registry_fingerprint_hash`（`IsKeyAssignmentEnabled`等、新UIの
  DWORD3値のみ）で判定しており、`keystyle`/`StyleList`（旧UI）の変化を一切見ていない。dragonflyg4
  実機で`NoTsf3Override2=1`（互換モード実際にON）・`keystyle=NATURAL`（現在アクティブ、新UI側は
  `IsKeyAssignmentEnabled=0`で「既定」判定）・`StyleList\Custom\key`に無変換/変換=IMEオン/オフ
  トグルの残留設定ありを確認した——この状態でユーザーが旧UIから`keystyle`を`Custom`に切り替えても、
  新UI側のフィンガープリントは変化しないため、ADR-195/176は「既知構成のまま・再較正不要」と誤判定
  し続ける。本ADRは`msime_legacy_keymap.rs`の検出結果を`check_and_warn`と対称な実行時警告に配線する
  （決定1〜3）。2026-09-23、[ADR-196](196-keymap-learn-truth-priority.md)（学習結果を内蔵表より
  優先する方針）がopus-adversarial-consult 5ラウンドで収束しdevelopへマージされ、ADR-195段階4/6/8の
  決定が置き換わった。ADR196-T5（陳腐化検出の置き換え）のタスク文書が本ADRの互換モードフラグ
  調査結果を名指しで前提条件として要求しているため、決定4はフィンガープリント合成・既知構成判定を
  自前で設計する当初案から、ADR196-T5が要求する読み取りプリミティブ（`NoTsf3Override2`）1つを
  提供するだけの薄いインターフェースへ縮小した（フィンガープリント合成・既知構成しきい値・UI文言は
  ADR-196決定1〜3・ADR196-T2/T4/T5が所有）。`key_effect_predictor`への直接統合（打鍵予測での学習
  省略用途）は、module docが「ON→OFF方向・S1key〜SEkeyの重ね合わせ・修飾子付きキーは未検証/未解読」
  と明記しているため本ADRの非目的のまま。
status: |-
  草案rev2(2026-09-23)。opus-adversarial-consult round1完了（Blocker2件・Must-fix5件・
  Should-fix7件・Minor2件、`197-opus-review-round1.md`）——B1(警告対象の実機未確認)はCI実機
  （GitHub Actions windows-latest、`ci/e2e-scenarios`）で裏取り中。round1の他の指摘（M1〜M5・
  S1〜S7）は未反映。決定4/5はADR-196マージを受けて全面改訂（旧内容は撤回）。
related_adr:
  - "ADR-148"
  - "ADR-092"
  - "ADR-195"
  - "ADR-176"
  - "ADR-196"
---

# ADR-197: MS-IME旧UI（互換モード限定の詳細キーカスタマイズ）を実行時警告と学習/較正機能に対応させる

## 背景

### 「以前のバージョンのMicrosoft IMEを使う」チェックボックスの実体（Web調査で確認）

設定アプリの `時刻と言語 → 言語と地域 → オプション → Microsoft IME → 全般 → 互換性` にある
このチェックボックスは、以下のレジストリ値に対応する
（[「以前のバージョンのMicrosoft IMEを使う」をコマンドで変更する方法を検証してみた](https://mitsushima.work/archives/26597847.html)）。

```
HKCU\SOFTWARE\Microsoft\Input\TSF\Tsf3Override\{03b5835f-f03c-411b-9ce2-aa23e1171e36}
  NoTsf3Override2 (DWORD) = 1（旧バージョンを使う=ON） / 0（新バージョン=OFF）
```

CLSID `{03b5835f-f03c-411b-9ce2-aa23e1171e36}` は `crates/awase-windows/src/state/ime_kind.rs:31`
が `ImeKindId::MsIme` 判定に使うMS-IME本体のCLSIDと**同一**である。したがってこのチェックボックスの
ON/OFFはawaseが観測するTIPのCLSID・`ImeKindId`判定には影響しない——影響するのはMS-IME**内部**で
どちらのキーカスタマイズ機構（新UI＝`MSIME`直下のDWORD4値／旧UI＝`keystyle`+`StyleList`のバイナリ
テーブル）が有効かだけである。

**2026-09-23、dragonflyg4実機で`NoTsf3Override2`を直接読み取り、値が`1`（互換モードON）であることを
確認した。** これはこのユーザーの現在の日常利用環境そのものであり、仮説ではない。

### `msime_legacy_keymap.rs`（ADR-148 Phase 2）が既にできていること・できていないこと

`crates/awase-windows/src/msime_legacy_keymap.rs`は、`keystyle`（現在有効なプリセット名）と
`StyleList\<keystyle>\key`（そのプリセットのキー割当てテーブル、Shift-JISテキストをNUL区切りで
連ねたREG_BINARY）を読み、無変換/変換キー（修飾子なし）に「IMEオン/オフ」トグルが割り当てられて
いるかを`LegacyMsImeToggleAssignment`として返す（`read_legacy_toggle_assignment()`）。ただし
モジュールdocが明記するとおり検出範囲は限定的である:

- 実機確認済みなのは**1列目（直接入力→ON方向）のみ**。2〜6列目（ON→OFF方向）は「重複行により
  後から追加された行が優先され実効性がない」と推測されるだけで確定していない。
- `Ctrl+`/`Shift+`/`Alt+`修飾子付きの無変換/変換、`S1key`〜`SEkey`補助テーブルとの重ね合わせ規則、
  「IMEオン/オフ」以外の機能コードはいずれも範囲外（検出しない）。

この検出結果は現状、`crates/awase-windows/src/runtime/message_handlers.rs:1614-1623`の
`build_bug_report_legacy_msime_keymap_summary`から**不具合報告への添付（ADR-148、読み取り専用の
診断情報）としてのみ**呼ばれている。`grep -rn msime_legacy_keymap crates/awase-windows/src`の
結果はこの1箇所のみであり、実行時の警告・awase自身の打鍵予測（`key_effect_predictor`）のいずれにも
配線されていない。

### 新UI側（`msime_key_assignment.rs`）には対称な実行時警告が既にある

新UI（`MSIME`直下の`IsKeyAssignmentEnabled`/`KeyAssignmentMuhenkan`/`KeyAssignmentHenkan`）を
検出する`msime_key_assignment.rs`は、不具合報告添付に加えて`check_and_warn`
（`crates/awase-windows/src/msime_key_assignment.rs:153-175`）を持つ。これは
`sync_ime_kind_from_observation`（`crates/awase-windows/src/runtime/message_handlers.rs:939-943`）
が`WM_IME_KIND_CHANGED`でMS-IME確定を検知するたびに呼ばれ、競合が見つかれば警告ログ＋
設定画面を開くか尋ねるポップアップ（別スレッド、`spawn_yes_open_ime_settings_dialog`）を出す。
同一内容の再警告を防ぐデデュープラッチ（`Runtime::msime_key_assignment_warned: Option<u8>`、
`swap_msime_key_assignment_warned`/`reset_msime_key_assignment_warned`、
`crates/awase-windows/src/runtime/mod.rs:364-369, 1335-1345`、ADR-164フェーズ2でグローバル
staticから構造体フィールド化済み）も備えている。

旧UI側にはこの実行時経路が丸ごと無い。**ユーザーが互換モードONのまま旧UIで無変換/変換キーに
「IMEオン/オフ」トグルを割り当てても、awaseは不具合報告を出すまで一切気づかず、ログにも警告にも
残らない。** これは新UI/旧UIという設定画面の違いだけで、awaseとの競合という実害の性質
（`crate::msime_key_assignment`モジュールdocが記す「OS側だけIME状態が反転し、awaseのbeliefと
乖離する」）自体は同じであり、この非対称は放置すべきではない。

### dragonflyg4実機の現在値（2026-09-23、`reg export`で確認）

```
keystyle = "NATURAL"（現在有効なプリセット）
StyleList\NATURAL\key: 無変換=97 28 28 28 28 28, 変換=87 06 2D 30 07 06
  → これはNATURALプリセットの素のネイティブ機能（無変換=IME OFF/変換=IME ON）であり、
    awaseが元々前提にしている標準MS-IME挙動そのもの。「IMEオン/オフ」トグル（コード`CE`）ではない。
StyleList\Custom\key: 無変換=CE CD CD CD CD CD, 変換=CE CD CD CD CD CD
  → module docが記す「IMEオン/オフ」トグル割当てそのもの（1列目`CE`＝直接入力→ON方向で実効）。
    ただし現在`keystyle=NATURAL`のため**今はアクティブではない**（過去のADR-148 Phase 2実機調査で
    作られたテスト用設定がそのままレジストリに残っている状態と見られる）。
```

現状のdragonflyg4では警告は発生しないはずの状態（NATURALがアクティブ）だが、ユーザーが旧UIの
「ユーザー定義」タブで`keystyle`を`Custom`に切り替えた瞬間、この既存の残留設定がそのまま有効になり、
awaseは気づけない。これが本ADRの動機。

### ADR-195/176（学習/較正機能）も同じ穴を持つ（ユーザー指摘、2026-09-23）

`msime_legacy_keymap.rs`が不具合報告添付にしか配線されていないという非対称は、`check_and_warn`
（実行時警告）だけでなく、[ADR-195](195-keymap-learn-productization.md)（キーマップ学習の製品化、
草案rev7）・[ADR-176](176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md) T12
（較正結果の陳腐化検出）にも
同型の形で存在する。

- **ADR-195段階0「経路3」**（195本文109行目）は「`msime_key_assignment.rs`: レジストリから
  Microsoft IME本体のキー再割り当てを検出する」とあるが、これは**新UIの`IsKeyAssignmentEnabled`等
  DWORD3値のみ**を見ている。`keystyle`/`StyleList`（旧UI）は一切見ていない。
- **ADR-195段階6「同梱表と同じ構成なら学習を勧めない」**（195本文412-419行目）の判定も、この経路3
  の検出結果に依存する。
- **ADR-176 T12の較正フィンガープリント**
  （`crates/awase-windows/src/msime_key_assignment.rs:278-289`
  `current_registry_fingerprint_hash(vk)`、`gji_charset_autodetect.rs:371-372`の
  `ConfigFingerprint::MsIme { registry_value_hash: .. }`経由でADR-195段階8の「要再検証」判定にも
  使われる）は、**`IsKeyAssignmentEnabled`/`KeyAssignmentMuhenkan`/`KeyAssignmentHenkan`の3値だけを
  ハッシュに含めており、`keystyle`/`StyleList`を一切含まない**（同ファイル281-289行目、実コードで
  確認済み）。

つまりdragonflyg4のように「新UI側は既定（`IsKeyAssignmentEnabled=0`）だが旧UI側で`keystyle`を
切り替えるとキー挙動が変わる」という状態では、ADR-195/176は**フィンガープリントが変化しないため
「既知構成のまま・再較正不要」と誤判定し続ける**。これは`check_and_warn`が無いことによる「ユーザーが
気づけない」問題と同根であり、こちらは「較正・学習の仕組み自体が気づけない」という一段深刻な形で
現れる——警告が無くてもユーザー自身が誤変換に気づいて不具合報告を出す経路は残るが、学習パイプラインが
静かに古い（誤った）表を使い続ける場合、ユーザーには「なぜか時々おかしい」としか見えない。

## 目的

`msime_legacy_keymap.rs`が既に確認済みの範囲（無変換/変換キー・修飾子なし・直接入力→ON方向）に
限定して、(1)新UI側の`check_and_warn`と対称な実行時警告、(2)ADR-195/176の既知構成判定・陳腐化
フィンガープリント、の両方に配線する。

## 非目的

- **`key_effect_predictor`（awase自身の打鍵予測）への統合はしない。** ON→OFF方向が実機で確認
  できていない状態でこれを予測に組み込むと、誤った前提で`InputModeApplied`/belief予測を行う
  リスクがある（`.claude/rules/ime-belief-architecture.md`のconfidence規律に抵触しうる）。
  ON→OFF方向の実機確認ができ、モジュールdocのコメントを更新できた段階で別ADRとして検討する。
- **`S1key`〜`SEkey`補助テーブル、修飾子付き（Ctrl+/Shift+/Alt+）の無変換/変換、「IMEオン/オフ」
  以外の機能コードの解読はしない。** `msime_legacy_keymap.rs`が既に確認した範囲のみを配線対象にする。
- **レジストリへの書き込み・自動解除はしない。** 新UI側と同じ方針（`msime_key_assignment.rs`の
  doc「レジストリは読み取り専用。書き換えによる自動解除は行わない」）を踏襲する。
- **`Tsf3Override\...\NoTsf3Override2`（互換モードチェックボックス自体）の読み取り・警告条件への
  組み込みはしない。** `keystyle`/`StyleList`の値は互換モードのON/OFFに関わらずレジストリ上に
  存在し続け、それが実際に有効化されるかはこのチェックボックスの状態と当該レジストリの読み取り
  だけからは確定できない（未検証、下記「残された未検証事項」参照）。読み取り専用の診断情報として
  ログに残す価値はあるが、警告を出す/出さないの判定条件には使わない——false negativeで実害のある
  競合を見逃す方が、false positiveで無害な警告を1回多く出すより悪いという既存方針
  （`msime_legacy_keymap.rs`の`Option<bool>`設計、判定不能と確認済みfalseを区別する思想）に倣う。
- **新UI警告（`msime_key_assignment::check_and_warn`）との単一ダイアログへの統合はしない。**
  両者は別レジストリ・別UIの独立した設定であり、両方同時に検出されるケースは稀と見込まれる。
  実装コストの低い「別ダイアログのまま両方出す」を採用し、統合UIは将来の改善として保留する。
- **学習の初期仮説・既知構成判定・フィンガープリント合成・段階UI文言の設計はしない。**
  2026-09-23のADR-196マージにより、これらはすべて[ADR-196](196-keymap-learn-truth-priority.md)
  決定1〜3・[ADR196-T1〜T5](../tasks/)が所有する（詳細は決定4参照）。本ADRが提供するのは
  ADR196-T5が名指しで要求する1つの読み取りプリミティブ（互換モードフラグ）のみ。

## 決定

### 決定1: `msime_legacy_keymap`に`check_and_warn`相当を新設し、既存の新UI警告と並べて呼ぶ

`crates/awase-windows/src/runtime/message_handlers.rs:939-943`の
`sync_ime_kind_from_observation`内、既存の

```rust
if detected && matches!(kind, crate::tsf::observer::ActiveImeKind::MicrosoftIme) {
    crate::msime_key_assignment::check_and_warn(app);
    sync_ime_toggle_auto_detect(app);
}
```

に、同条件下で`crate::msime_legacy_keymap::check_and_warn(app)`（新設）を追加で呼ぶ。

新設する`check_and_warn`は`msime_key_assignment.rs:153-175`と同型の構造にする:

1. `read_legacy_toggle_assignment()`を呼ぶ。
2. `muhenkan_ime_on_toggle == Some(true)` または `henkan_ime_on_toggle == Some(true)`
   の場合のみ警告対象とする（`None`＝判定不能、`Some(false)`＝確認済み割当てなし、いずれも
   警告しない——既存の`Option<bool>`設計をそのまま条件に流用する）。
3. 警告文言は実機確認済みの範囲に限定する。例:
   「MS-IME（以前のバージョンの互換モード）の詳細キーカスタマイズで、無変換/変換キーに
   『IMEオン/オフ』が割り当てられています。awase は無変換/変換キーを親指シフトキーとして使う
   ため、直接入力中にこのキーを単独で押すと OS 側だけ IME が ON になり、awase の管理外で状態が
   ずれる可能性があります。」——`msime_key_assignment.rs:122-131`の`conflict_warning`文言を
   下敷きにしつつ、「ON→OFF方向は未確認」という限定を誤解なく伝える（過大な確実性を主張しない）。
4. 解除導線は新UIと同じ`ms-settings:regionlanguage-jpnime`を開くか尋ねるダイアログ
   （`spawn_yes_open_ime_settings_dialog`を再利用）。旧UIの「ユーザー定義」タブ自体は
   `ms-settings:`から数クリック奥のため、案内文でその旨を明記する。

### 決定2: デデュープラッチを`Runtime`に追加する（ADR-164フェーズ2と同じパターン）

`crates/awase-windows/src/runtime/mod.rs`の`msime_key_assignment_warned: Option<u8>`
（:364-369）と対になる`msime_legacy_keymap_warned: Option<(LegacyKeyStyle, bool, bool)>`
（または同等にハッシュ化した値）を追加し、`swap_msime_legacy_keymap_warned`/
`reset_msime_legacy_keymap_warned`を新設する。裸のグローバルstaticにしない
（`.claude/rules`のADR-164方針、`feedback_no_raw_global_statics_prefer_static_struct`）。

### 決定3: 不具合報告添付（ADR-148 Phase 2）側の実装は変更しない

`build_bug_report_legacy_msime_keymap_summary`（`message_handlers.rs:1614-1623`）は
そのまま維持する。決定1の`check_and_warn`は`read_legacy_toggle_assignment()`を独立に
呼ぶ（結果をキャッシュしない、新UI側の`check_and_warn`も同様に毎回レジストリを読み直す
設計のため対称性を保つ）。

### 決定4（2026-09-23 ADR-196マージを受けて全面改訂）: 互換モードフラグの読み取り関数だけを提供し、フィンガープリント合成・既知構成判定はADR-196に委譲する

起票当初の決定4/5は、`msime_legacy_keymap.rs`にADR-195段階8のフィンガープリント合成・
段階6の既知構成判定を直接実装する案だった。**2026-09-23、[ADR-196](196-keymap-learn-truth-priority.md)
（学習結果を内蔵表より優先する方針）がopus-adversarial-consult 5ラウンド（Blocker/Must-fix
0件）で収束しdevelopへマージされ、ADR-195段階4/6/8の決定そのものが置き換わった**
（別セッションからの通知、`docs/tasks/adr196-t1〜t5-*.md`）。このうち
[ADR196-T5](../tasks/adr196-t5-revalidation-not-invalidation.md)（旧段階8＝陳腐化検出を
「失効」から「要再検証」へ）のフロントマターが、本ADRを名指しで前提条件として参照している:

> **Microsoft IMEレガシー互換モードフラグのレジストリ位置**:
> `msime_legacy_keymap.rs`は`keystyle`とStyleListしか読んでおらず、「以前のバージョンの
> Microsoft IMEを使う」設定そのものを読むコードは存在しない。実機でのレジストリdiffで
> 確定するまで、Microsoft IME本体のフィンガープリント・既知構成判定は実装できない。

本ADRの背景節が確定した事実（`HKCU\SOFTWARE\Microsoft\Input\TSF\Tsf3Override\{03b5835f-
f03c-411b-9ce2-aa23e1171e36}\NoTsf3Override2`、dragonflyg4実機で`1`＝互換モードON）は、
まさにこの前提条件そのものである。したがって決定4は以下に縮小する:

1. **`msime_legacy_keymap.rs`に、この値を読む関数（例: `read_legacy_compat_mode_enabled()
   -> Option<bool>`、`NoTsf3Override2 == Some(1)`ならON）を新設する**。読み取り専用、
   既存の`read_raw_value`ヘルパーと同じ`ERROR_FILE_NOT_FOUND`区別のパターンを踏襲する。
2. **フィンガープリントの合成方法・既知構成判定のしきい値・UI文言は本ADRでは定めない
   ——[ADR196-T5](../tasks/adr196-t5-revalidation-not-invalidation.md)決定3b
   （Microsoft IME本体のフィンガープリント＝OSビルド番号＋本項の互換モードフラグ＋
   `keystyle`＋`msime_key_assignment.rs`の再割当て検出の4値）と
   [ADR196-T2](../tasks/adr196-t2-mismatch-adjudication.md)決定1c（既知構成＝`keystyle`が
   既定値・新UI再割当てなし・レガシー互換モード無効の3条件、**本項が未実装のうちは既知構成と
   判定しない**というfail-safeを含む）が既に所有する。** 本ADRの決定1〜3（実行時警告）とは
   独立した別の合流点であり、ここで競合する設計を追加しない。
3. `StyleList\<keystyle>\key`の生バイト列そのもの（無変換/変換以外の内容を含む）を
   フィンガープリントに含めるかどうかもADR196-T5側の設計判断とする——本ADRが確認した
   `LegacyKeyStyle`（`msime_legacy_keymap.rs`の`keystyle`分類、6値+Other）はADR196-T5の
   4値フィンガープリントの「`keystyle`」項目としてそのまま使える形になっている。

### 決定5（旧決定5は撤回）

起票当初の決定5（ADR-195段階6「既知構成なら学習を積極的に案内しない」への`keystyle`条件
追加）は、**前提としていたUI方針自体がADR-196決定2で撤回された**（「構成に関わらず学習を
同じ導線で案内する」、`docs/tasks/adr196-t4-ui-status-and-adoption.md`実装対象1）ため、
そのまま撤回する。既知構成の判定は決定4の2で述べたとおりADR196-T2/T4が所有し、
「既知構成でも学習ボタンを隠す」という用途にはもう使われない（採否判定・状態表示文言の
選択にのみ使われる）。

## 成功基準

1. dragonflyg4実機で、現状の設定（`keystyle=NATURAL`）のままawaseを再起動し、MS-IME確定時に
   **警告が出ないこと**を確認する（false positive否定）。
2. 旧UIの「ユーザー定義」タブで`keystyle`を`Custom`に切り替え（レジストリに残る既存の
   無変換/変換=IMEオン/オフトグル設定を再度有効化し）、awase再起動→MS-IME確定で**警告が出る
   こと**を確認する。
3. 新UI側（`IsKeyAssignmentEnabled`等）を意図的に有効化した状態と同時発生させ、両方の警告が
   独立に（順不同で構わない）出ることを確認する。
4. 決定4の`read_legacy_compat_mode_enabled()`が、互換モードON/OFF双方の実機（またはCI、
   `NoTsf3Override2`をレジストリで直接切り替えたテスト）で正しい`Option<bool>`を返すこと。
   ADR196-T5/T2側の採否は別ADR（ADR-196）の完了条件でカバーするため、本ADRでは関数単体の
   正しさのみを確認する。
5. `cargo test --lib` / `cargo nextest run -p awase-windows --test architecture_guard
   --test golden_scenarios --test layer_boundary_guard`が緑のままであること。
6. `.claude/rules/fix-requires-evidence.md`の「IME belief」「キー選択」reincidence family
   に該当するため、`docs/known-bugs/`への記録は不要（新規バグ修正ではなく既存検出機構の
   実行時経路への拡張のため）だが、本ADR自体が設計記録を兼ねる。

## 残された未検証事項

- **`Tsf3Override\...\NoTsf3Override2`が0（互換モードOFF）でも`keystyle`/`StyleList`の
  割当てがMS-IME内部で実際に有効化されるか**は未検証。もし「互換モードOFFなら旧UIの設定は
  一切効かない」ことが実機で確認できれば、決定1の警告条件に`NoTsf3Override2==1`のANDを
  足すことでfalse positiveをさらに減らせる可能性がある（ただしその場合も、読めない・
  未検出時は安全側〈警告する側〉に倒すこと）。実装前にこの点を実機A/B（互換モードOFFの状態で
  `keystyle=Custom`のまま無変換キー単独タップを試す）で確認することを推奨する。
- ON→OFF方向（2〜6列目）の実効性は本ADRでも未確認のまま。決定1の警告文言はこれを明記して
  対応し、確認が取れ次第、文言と`key_effect_predictor`統合の要否を再検討する別ADRを起票する。
- **決定4の`read_legacy_compat_mode_enabled()`をADR196-T2/T5側が実際にどう消費するか**
  （フィンガープリントの4値目としてそのまま使うのか、別の形に変換するのか）は、
  [ADR196-T5](../tasks/adr196-t5-revalidation-not-invalidation.md)側の実装時に確定する
  ——本ADRは関数シグネチャの提案までに留め、消費側の設計には立ち入らない。

## 関連

[ADR-148](148-bug-report-ime-keymap-attachment.md)（`msime_legacy_keymap.rs`の出自、
Phase 2で実機確認したバイナリ形式・コード値・非対称な実効挙動）。`msime_key_assignment.rs`
（新UI側の先行実装、`check_and_warn`/デデュープラッチのパターン一式）。ADR-164フェーズ2
（グローバルstaticの構造体フィールド化パターン、本ADRの決定2が踏襲）。
[ADR-195](195-keymap-learn-productization.md)（キーマップ学習の製品化、草案rev7。段階4/6/8は
ADR-196により置き換え済み）。
[ADR-176](176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md) T12（較正
フィンガープリント`current_registry_fingerprint_hash`、ADR196-T5が拡張対象とする関数の出自）。
[ADR-196](196-keymap-learn-truth-priority.md)（学習結果を内蔵表より優先する方針、develop
マージ済み2026-09-23。決定4が委譲する先。[ADR196-T2](../tasks/adr196-t2-mismatch-adjudication.md)
〈既知構成判定〉・[ADR196-T4](../tasks/adr196-t4-ui-status-and-adoption.md)〈状態表示〉・
[ADR196-T5](../tasks/adr196-t5-revalidation-not-invalidation.md)〈陳腐化検出、本ADRを前提
条件として名指しで参照〉が決定4の直接の消費者）。
