---
id: ADR-191-companion-191-gji-state-scope-spec
title: |-
  ADR-191 GJI/MS-IME の開閉・変換モードの保持範囲（仕様調査：Mozc読解とCI実測）
type: companion-doc
related_adr:
  - "ADR-089"
  - "ADR-189"
  - "ADR-191"
---

# GJI / MS-IME の開閉・変換モードの保持範囲（仕様調査）

## 問い

[ADR-191](191-ime-is-source-of-truth-observe-not-write.md)の読めない条件（blind）で残った1件（seed 2 押下 #58）の実バグは、ADR-189のトグルON経路が
`UserImeOnEisuReset`（`state/eisu_recovery.rs::eisu_reset_on_ime_on`、Edge対策）で「IME ONでひらがなに戻る」と仮定して`ObservedEisu→AssumedRomaji`へ直すのに対し、
GJIは閉じても変換モードを保持し、開き直すと0x10のままだった、というものである。belief側で何を記憶すべきか決めるため、次を確定する:
**(Q1)閉じて開き直したとき変換モードは保持されるか。(Q2)その保持の単位は窓（HWND）か、スレッドか、プロセスか、全体か。(Q3)新しい窓（別スレッド/別プロセス）でIMEを開いたとき何から始まるか。**

## 1. Mozc（GJIのオープンソース版）のソース読解（仮説。配布版GJIと同一とは限らない）

読んだ版: `google/mozc` の `--depth 1`（2026-09-21 時点の HEAD）。引用は `src/` 相対。

- **状態の保持単位はTSFスレッド**: 開閉・変換モードは`TipThreadContext`が持つ`TipInputModeManager`（`win32/tip/tip_thread_context.cc`）と、TSFスレッドcompartment
  （`GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`／`GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION`、`win32/tip/tip_status.cc:46-105`）で持つ。窓（HWND）ごとの状態は持たない。
  `TipInputModeManager::OnInitialize`（`tip_input_mode_manager.cc:187-`）はTIPの有効化時にスレッドcompartmentの現在値を取り込み、`OnSetFocus`（`:201-`）はフォーカス移動時に同じスレッドcompartmentから読み直す。
- **compartmentが空のときの既定は 0x09**: `TipStatus::GetInputModeConversion`は、値が無ければ`kDefaultMode = TF_CONVERSIONMODE_NATIVE | TF_CONVERSIONMODE_FULLSHAPE`（ひらがな、0x09）で初期化する（`tip_status.cc:89-93`）。ROMANビット（0x10）は立てない。
- **IME ON/OFFで変換モードは消えない（サーバー側セッション）**: `Session::IMEOn`/`IMEOff`（`session/session.cc:1023,1036`）は`SetSessionState(PRECOMPOSITION/DIRECT)`を呼び、それが`Composer::Reset()`（`composer/composer.cc:580`）→`ResetInputMode()`（`:589`）→
  `SetInputMode(comeback_input_mode_)`で、**直前の入力モード（comeback）へ戻す**。閉じる操作は入力モードを捨てない。
- **クライアントは毎キーで、スレッドの現在モードをサーバーへ送る**: `keyevent_handler.cc:697-726`が`key->set_mode(mozc_mode)`（「歴史的な理由でvisibleなモードを渡す」）とし、`Session::IMEOn`は`key.mode()`があれば`ApplyCompositionMode`する。
  応答（`Output.status`）は`OnReceiveCommand`（`tip_input_mode_manager.cc:164-`）経由で`TipStatus::SetInputModeConversion`（`tip_edit_session_impl.cc:428`）としてスレッドcompartmentへ書かれる。よって**実効的な保持先はスレッドcompartment**。
- **`use_global_mode`**: `tip_thread_context.cc:46`が`WinUtil::IsPerUserInputSettingsEnabled()`（`base/win32/win_util.cc:541`、`SPI_GETTHREADLOCALINPUTSETTINGS`が偽なら真）を`use_global_mode`に入れる。真のとき`OnChangeConversionMode`は何もせず（`:247`）、
  `OnInitialize`/`OnSetFocus`はスレッドcompartmentから変換モードを取り込まない（`:191`、`:214`）。ただしCIの実測（後述）では、この設定が真（=グローバル）でも、変換モードはスレッドごとに独立だった。
- セッション（`Client`）は`TipPrivateContext`（= ITfContextごと）に1つ（`tip_private_context.cc:58`）。ただし上記のとおり毎キーでスレッドのモードが上書きされるので、コンテキストごとのモードは観測できる差にならない。

## 2. CI実測（GitHub Actions windows-latest、`gji_state_scope_probe`）

プローブ: `crates/awase-windows/examples/gji_state_scope_probe.rs`（ブランチ `ci/e2e-gji-scope`、run 35623301916）。**状態はキーだけで作る**（IMM書き込みで状態を作らない。格子v1の教訓、
[191-calibration-experiments.md](191-calibration-experiments.md)）。W1で事前状態（開/閉 × 基準〈ひらがな〉/代替モード〈ATOK=F2でひらがな→0x10、MS-IMEプリセットとMS-IME本体=F1でカタカナ0x1B〉）を作り、次の窓へ移って観測する:
`edit2`=同じ最上位窓の別入力欄、`top2`=同じスレッドの別の最上位窓、`thread2`=同じプロセスの既存の別スレッドの窓、`thread2_new`=新しい別スレッドの窓、`proc2_new`=別プロセスの新しい窓（`--child`）。
観測はIMM（A: `ImmGet*`）、TSFスレッドcompartment（T）、グローバルcompartment（G）で、移動後+150/+700ms、移動先でIMEを開いた後、W1へ戻った後。構成は`gji-atok`（GJI、session_keymap=1）、`gji-msime`（GJI、=2）、`msime-native`（Microsoft IME本体）、各2回×セル2反復=4〜6観測。
**環境**: `SPI_GETTHREADLOCALINPUTSETTINGS = 0`（全6ジョブ）。すなわちMozcの`use_global_mode`は真（Windows既定の「入力設定はユーザー全体で共有」）。Gコンパートメントは全観測で`0x00/0x00`のまま（使われていない）。

### 2.1 結果（要約。全表は §5 の生データから `tools/e2e/ime_key_matrix/gji_scope_table.py` で再生成できる）

| 問い | GJI（ATOK/MS-IMEプリセット）| Microsoft IME 本体 |
|---|---|---|
| **Q1 閉→開で変換モードは保持されるか**（同じスレッド内で閉→`VK_IME_ON`） | **保持される**: ATOK 0x10→閉→開=0x10、MS-IMEプリセット 0x1B→閉→開=0x1B | **保持されない**: 0x1B→閉→開=**0x19（ひらがなに戻る）** |
| **Q2 保持の単位** | **スレッド**。同じスレッドの別入力欄・別最上位窓は状態を共有（`edit2`/`top2` は移動先でも W1 と同じ開閉・モード）。別スレッド（既存・新規）・別プロセスは独立で、W1の状態は移動後も変わらず、戻ると元のまま（`w1_back`）。HWND単位の記憶は観測されない | 同じ（スレッド単位。別スレッド/別プロセスは独立） |
| **Q3 新しい窓（別スレッド/別プロセス）でのIME状態** | **閉・0x09（ひらがな、ROMANビット無し）で始まる**。W1が開×0x10でも、閉×0x1Bでも継承しない。そこで開くと**0x09**（W1のモードではない） | **閉・0x19** で始まる。開いても0x19 |
| 既存の別スレッド窓（`thread2`） | その窓自身の直前の状態（事前リセットで開×0x09/0x19）のまま。W1の状態は伝わらない | 同じ |
| 0x09と0x19 | GJIは自然状態が0x09、ひらがなキー(F2)を通すと0x19系（ROMANビット付き）になることがある。**同じ「ひらがな」として扱う**（awaseの`Conv::from_raw`と同じ） | 0x19 |

- 保持された/されなかった観測は、GJI×2プリセット×2回×反復2、MS-IME本体×2回×反復2 で一貫した（矛盾する反復なし、`[SCOPE-ERR]`は0件）。

## 3. belief設計への含意

1. **保持単位はスレッド**（TSFのスレッドマネージャ）。**HWNDごとの記憶は不要**で、むしろ誤りになる: 同じスレッドの別HWND（別の入力欄・別の最上位窓）は状態を共有し、別スレッドの窓は独立する。
   awaseのhwndキャッシュ復元（`HwndCacheRestored`）は、「前回そのhwndで持っていたbelief」を再現するが、**同じスレッドの別hwndで状態が変わった後に、古いhwndのキャッシュを戻すと、実状態と食い違う**。キーは（プロセス、スレッド）にするのが正確。
   ただし、読めない窓では実スレッドの状態を観測できないため、実運用では「フォーカス変更時にhwndのスレッドIDを引き、同じスレッドのbeliefを引き継ぐ」以上の精緻化は要らない。
2. **新しいスレッド/プロセスの窓の初期状態は「閉×ひらがな(0x09/0x19)」**（GJI、MS-IME本体とも）。これはawaseの既定の種（IME ON・ひらがな・ローマ字）と**開閉が違う**: 新しい窓は最初は閉。
   ただし、ユーザーが最初のキーでIMEを開く操作をすれば、開いた直後は**ひらがな**である（前の窓が0x10でも継承しない）。
3. **`UserImeOnEisuReset`（IME ONで`ObservedEisu`をひらがなに直す）の仮定の正誤**:
   - **正しい**: 新しいスレッド/プロセスの窓でIMEを開いたとき（0x09）。MS-IME本体で閉→開したとき（0x19に戻る）。
   - **誤り**: GJIで**同じスレッド内**で閉→開したとき（直前のモード0x10/0x1Bを保持）。blind s2 の実バグはここに当たる。
4. **blind s2 の実バグの修正の方向**: 「ON→ひらがなに戻る」を**IME種別とスレッドの新旧で分ける**。GJI（`ImeKindId::Gji`）では、追跡している変換モード（`key_track.conv`）が既知なら、閉→開でそのモードを保持し、`ObservedEisu`をひらがなに直さない（保持した0x10のまま）。
   追跡が不明（新しいスレッド、フォーカス変更で追跡を捨てた直後）のときだけ、既定のひらがな（0x09）を種にする。MS-IME本体では現行のリセット（開くとひらがな）が正しい。
   デッドロック対策（Edge、engineが`NotRomajiInput`でinactiveのままIME ONの経路が無くなる循環）は、追跡が不明なときに効き続ける。**新しい状態は追加しない**（`key_track.conv`は既にあり、閉じても保持する設計。ADR-191の予測追跡）。
5. 予測表（格子）はこの保持を既に反映している（`off-c10|hankaku-zenkaku → ON/0x10`、`off-c1B` 等）。ただしトグルキー（0x19/0xF3/0xF4）は`shadow_action`を持つため表が使われない（呼び出し側で除外）。
   トグルON経路でも、表または`key_track.conv`を参照して変換モードを保持するのが、修正の実体になる。

## 4. 未確認・限界

- CI（GitHub Actions windows-latest、Windows Server、GJI 最新版/chocolatey、`SPI_GETTHREADLOCALINPUTSETTINGS=0`）での結果。**実機（Windows 11、ユーザーの`custom_keymap_table`あり）や、TSFネイティブアプリ（Chrome、VS Code、WezTermなど、内部で複数スレッド/プロセスを使う）では未確認**。
  特にChromeはレンダラープロセスとブラウザプロセスでスレッドが違うため、同じ最上位窓でも「スレッドが変わって状態が切り替わる」可能性がある（実機のTsfNativeの再発ファミリーと関係しうる）。
- 同じスレッドの別の**HIMC**（`ImmCreateContext`で作った別の入力コンテキストを関連付けた窓）は測っていない。標準のEDITは既定のHIMCを共有する。
- 移動後の+1500ms観測は`--fast`で省略した（+700msまでで安定、格子の知見と同じ）。
- Mozcソースは配布版GJIと同一とは限らない。読解はCI実測で裏付けた範囲（保持・スレッド単位）のみ確定として扱う。
- MS-IME本体で「開くと0x19へ戻る」のが`VK_IME_ON`固有か、他の開き方（半角/全角）でも同じかは、`VK_IME_ON`だけで測った。GJIでも`VK_IME_ON`のみ（半角/全角では測っていない）。

## 5. 再現手順と生データ

- 生データ: CI run https://github.com/cuzic/awase/actions/runs/35623301916 のartifact `scope-{gji-atok,gji-msime,msime-native}-{1,2}`（保持期限7日）。
- 再現: ブランチ `ci/e2e-gji-scope` へpush（`.github/workflows/e2e-gji-scope.yml`、6ジョブ、約12分）。集計: `python3 tools/e2e/ime_key_matrix/gji_scope_table.py <log>...`。
- プローブ/ワークフロー/集計はブランチ `ci/e2e-gji-scope`（develop未マージの調査用）にある。
