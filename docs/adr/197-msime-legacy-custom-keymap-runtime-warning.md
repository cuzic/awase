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
  し続ける。本ADRは`msime_legacy_keymap.rs`の検出結果を、(1)`check_and_warn`と対称な実行時警告、
  (2)ADR-195/176のフィンガープリント・既知構成判定、の両方に配線する。`key_effect_predictor`への
  直接統合（学習を省略する用途）は、module docが「ON→OFF方向・S1key〜SEkeyの重ね合わせ・修飾子付き
  キーは未検証/未解読」と明記しているため本ADRの非目的とし、ADR-195側の将来課題として記録するに留める。
status: |-
  草案(2026-09-23起票)。opus-adversarial-consultによるレビュー未実施。
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
- **ADR-195段階0「初期仮説表S」への統合（学習を一部省略する用途）はしない。** ADR-195本文の
  「Microsoft IME本体には経路2に相当するものが無い。したがって初期仮説Sは常に空（全キー要学習）」
  （195本文151行目）という前提は変更しない——`msime_legacy_keymap`の確認済み範囲を経路4として
  追加し学習を省略する最適化は理論上可能だが、本ADRの主眼は「気づけない」の解消であり学習コストの
  短縮ではない。ADR-195側の将来課題として「関連」節に記録するに留める。
- **ADR-195/176のフィンガープリント統合（決定4）は、ADR-195/176自体の改訂を伴う。** 両ADRは
  草案・多ラウンドレビュー中のドキュメントであり、本ADRが一方的に本文を書き換えることはしない
  ——本ADRの決定4はADR-197視点でのインターフェース要件（awase-windows側の関数シグネチャ・
  ハッシュ入力の変更）を定義し、ADR-195/176本文側の該当箇所（段階0経路3・段階6・T12）は、実装時
  または次のレビューラウンドで両ADRの整合を取る形で更新する。

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

### 決定4: ADR-176 T12の較正フィンガープリントに`keystyle`/`StyleList`を含める

`crates/awase-windows/src/msime_key_assignment.rs:278-289`の
`current_registry_fingerprint_hash(vk: VkCode) -> u64`は現在、`IsKeyAssignmentEnabled`/
`KeyAssignmentMuhenkan`/`KeyAssignmentHenkan`（新UI）のみをハッシュに含む。この関数の
戻り値は`gji_charset_autodetect.rs:371-372`経由で`ConfigFingerprint::MsIme { registry_
value_hash }`に格納され、ADR-195段階8（陳腐化検出）が「要再検証」を判定する唯一の入力になる。

**変更**: `msime_legacy_keymap.rs`に`legacy_registry_fingerprint_hash() -> u64`を新設し、
`keystyle`の文字列値と、対応する`StyleList\<keystyle>\key`の生バイト列（存在すれば）を
ハッシュに含める（値が無い場合も「無い」という事実自体を安定した値としてハッシュに含める
——`msime_legacy_keymap.rs`の既存関数群と同じ`Result<Option<..>, String>`の区別を踏襲する
か、フィンガープリント用途では簡略化するかは実装時に確定する）。呼び出し元
（`gji_charset_autodetect.rs:371-372`）で新UI側のハッシュと合成する（例:
`registry_value_hash: current_registry_fingerprint_hash(vk) ^ legacy_registry_
fingerprint_hash()`、あるいは`ConfigFingerprint::MsIme`にフィールドを1つ追加する——
具体的な合成方法はADR-195/176側との調整を要するため実装時に確定する）。

この変更により、`keystyle`の切り替え・`StyleList`の内容変更のいずれも段階8の「要再検証」
判定を発火させるようになる。**この決定はADR-176/195本文の該当箇所の改訂を要する**
（上記「非目的」参照、両ADR側での取り込みが前提）。

### 決定5: ADR-195段階0/段階6の「既知構成一致」判定に`keystyle`の一致条件を追加する

ADR-195段階6（195本文412-419行目）「検出したキーマップ構成が同梱の3種（ATOK/GJI+MS-IME
プリセット/Microsoft IME本体、いずれもカスタム設定なし）と一致する場合、学習を積極的に
案内しない」の判定条件に、**Microsoft IME本体については「`keystyle`がCI基準プリセット
（`NATURAL`、内蔵表`MSIME_NATIVE`が測定された設定）と一致すること」をANDで追加する**。

この判定は`msime_legacy_keymap.rs`が確認済みの「無変換/変換キーのIMEオン/オフトグル」という
狭い検出（決定1が使う範囲）より**粗いが広い**——`StyleList`の個々のコードの意味を解読できて
いなくても、「`keystyle`文字列がCIの基準と違う」という事実だけで「CIが測った内蔵表とは異なる
設定である」ことが機械的に確定できる。`keystyle`がNATURAL以外（Custom/ATOK/MS-IME2000/
VJE/WX）であれば、無変換/変換キーに実際に何が割り当てられているかに関わらず「既知構成では
ない」側に倒し、学習（段階6の案内）を積極的に案内する。

**判定不能時（レジストリ読み取り失敗等）の扱い**: 決定1と同じ思想（false negativeを避ける）
に倣い、「既知構成である」と確定できない場合は「既知構成ではない」側に倒す（学習を案内する側、
安全側）。

## 成功基準

1. dragonflyg4実機で、現状の設定（`keystyle=NATURAL`）のままawaseを再起動し、MS-IME確定時に
   **警告が出ないこと**を確認する（false positive否定）。
2. 旧UIの「ユーザー定義」タブで`keystyle`を`Custom`に切り替え（レジストリに残る既存の
   無変換/変換=IMEオン/オフトグル設定を再度有効化し）、awase再起動→MS-IME確定で**警告が出る
   こと**を確認する。
3. 新UI側（`IsKeyAssignmentEnabled`等）を意図的に有効化した状態と同時発生させ、両方の警告が
   独立に（順不同で構わない）出ることを確認する。
4. `keystyle=NATURAL`のまま較正済み状態にした後、`keystyle`を`Custom`へ切り替えてawaseを
   再起動し、ADR-195段階8（要再検証判定、決定4）が「要再検証」を検出すること、かつ
   `keystyle=NATURAL`のまま無変換/変換以外の値（無関係なレジストリの再読み取りタイミング差等）
   を変えずに再起動した場合には誤検知しないことを確認する。
5. `keystyle=ATOK`（無変換/変換とも既定でIMEオン/オフトグルではない値）に切り替えた状態で、
   決定5の「既知構成一致」判定が**NATURAL以外というだけで**「既知構成ではない」（学習を案内
   する側）と判定すること——決定1の警告（トグル検出）は発火しない一方で決定5は発火するという
   非対称な組み合わせが意図どおり両立することを確認する。
6. `cargo test --lib` / `cargo nextest run -p awase-windows --test architecture_guard
   --test golden_scenarios --test layer_boundary_guard`が緑のままであること。
7. `.claude/rules/fix-requires-evidence.md`の「IME belief」「キー選択」reincidence family
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
- **決定4/5がADR-195/176本文の改訂を要する具体的な範囲**（段階0経路3の説明文、段階6の判定条件、
  T12のフィンガープリント計算箇所）は未確定。両ADRが本ADRより先に実装される場合と後に実装される
  場合とで、どちらが「捨てる前提の暫定実装」になるかが変わる（ADR-195段階0の「ADR-192との順序
  リスク」記述〈195本文126-131行目〉と同型の問題）。実装順序を決める際に確認すること。
- **ADR-195段階0「経路4」としての初期仮説統合**（非目的で保留）は、ON→OFF方向の実機確認が取れた
  後にまとめて検討する候補として残す——GJIの経路2（`extract_mode_keys`が相対トグル系を「不明、
  要学習」に留める設計）と同型の扱いにできる可能性がある。

## 関連

[ADR-148](148-bug-report-ime-keymap-attachment.md)（`msime_legacy_keymap.rs`の出自、
Phase 2で実機確認したバイナリ形式・コード値・非対称な実効挙動）。`msime_key_assignment.rs`
（新UI側の先行実装、`check_and_warn`/デデュープラッチのパターン一式）。ADR-164フェーズ2
（グローバルstaticの構造体フィールド化パターン、本ADRの決定2が踏襲）。
[ADR-195](195-keymap-learn-productization.md)（キーマップ学習の製品化、草案rev7。段階0経路3・
段階6・段階8が本ADRの決定4/5の対象）。
[ADR-176](176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md) T12（較正
フィンガープリント`current_registry_fingerprint_hash`、本ADRの決定4が拡張対象とする関数の出自）。
[ADR-196](196-keymap-learn-truth-priority.md)（学習結果を内蔵表より優先する方針、草案。ADR-195の
段階4/6/8を修正する別ドラフトのため、本ADRの決定4/5と合わせて整合を確認する必要がある）。
