#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! MS-IME「キーとタッチのカスタマイズ」割当ての起動時検出と解除案内
//!
//! # 背景（防ぐバグクラス: IME 状態の二重オーナー）
//!
//! MS-IME は設定で無変換/変換キーそれぞれに独立してIME-オン・IME-オフ・トグル等を
//! 割り当てられる（下記「レジストリ位置」節の値の意味を参照。無変換=オフ・変換=オンに
//! 固定されているわけではない）。
//! 一方 awase は無変換/変換を親指シフトキーとして扱い、単独タップを
//! `Key(0x1D/0x1C)` として OS に素通しする。割当てが有効だと、この素通しを
//! MS-IME が処理して **OS 側だけ** IME 状態が反転し、awase の belief と乖離する
//! （2026-07-06 実機: IME ON の 92ms 後の無変換単独タップで OS IME だけ OFF になり
//! 「IME OFF・Engine ON」で親指シフト入力が生ローマ字化。TSF-native アプリでは
//! 観測経路がなく自己修復しない）。
//!
//! awase は全プロファイルで IME ON/OFF を自前制御できる（ImmCross /
//! GjiDirect / MsImeDirect、`state/key_sequence_policy.rs` 参照）ため、
//! MS-IME 側の割当ては不要かつ有害。アクティブ IME が MS-IME と確定した
//! 最初の `WM_IME_KIND_CHANGED` で検出してユーザーに解除を案内する
//! （GJI 利用中はチェック自体をスキップする）。
//!
//! # レジストリ位置（2026-07-06 設定アプリ操作前後の実機 diff で確定）
//!
//! `HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME`:
//! - `IsKeyAssignmentEnabled` (DWORD) — 割当てマスタースイッチ（設定アプリが即書き込む）
//! - `KeyAssignmentMuhenkan`（無変換）/`KeyAssignmentHenkan`（変換） (DWORD) — 共通で
//!   **0 = IME-オン・1 = IME-オフ・2 = IME-オン/オフ（トグル）**。3 だけ無変換/変換で意味が
//!   分かれる（無変換=ひらがな/カタカナ切替・変換=再変換）。2026-09-26 実機（dragonflyg4、
//!   設定アプリをUI Automationで自動操作）で確認（[ADR-199](../../../docs/adr/199-derive-key-roles-from-user-ime-keymap.md)
//!   T12）。以前は「0 = 既定（かな切替/再変換）、1 だけ意味を持つ」という前提だったが、
//!   実際には**0も明示的な割り当て（IME-オン）であり既定ではない**——この前提の逆転を踏まえ、
//!   値0/3の実機的意味（予測・警告への影響）が確認できるまでは、予測側は明示値があれば
//!   一律に安全側（予測しない）で扱う（`state/key_effect_predictor.rs::KeyEffectKeymap::
//!   for_msime_native`、決定C R3）。
//!
//! レジストリは**読み取り専用**。書き換えによる自動解除は行わない
//! （動作中 IME への反映タイミングが保証されず、ユーザー設定への侵襲になるため）。
//! 解除はユーザー自身に `ms-settings:regionlanguage-jpnime` で行ってもらう。

/// `KeyAssignmentCtrlSpace`/`KeyAssignmentShiftSpace`（ADR-092 決定D Step4a）が
/// トグル（値2）に設定されているか。
///
/// 2026-08-15 実機（dragonflyg4）で `IsKeyAssignmentEnabled=1` かつこれら2値が
/// `2` になることを確認済み。個別オン/オフの割当ては MS-IME の「キーとタッチの
/// カスタマイズ」に存在しないと確認済みのため、`2`（トグル）以外は
/// 「宣言なし」として扱う（推測しない、決定C R3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MsImeToggleAssignment {
    /// `KeyAssignmentCtrlSpace == 2`
    pub ctrl_space_is_toggle: bool,
    /// `KeyAssignmentShiftSpace == 2`
    pub shift_space_is_toggle: bool,
}

impl MsImeToggleAssignment {
    /// `Engine::set_ime_toggle_auto_keys` に渡す `ParsedKeyCombo` 列へ変換する。
    ///
    /// `skip_shift_space` が `true` の場合、Shift+Space は含めない
    /// （呼び出し元が Space を親指キーに設定している場合に使う——Shift+Space
    /// 親指キーのリテラル送出機能（`text_key_space.shift_literal`）と
    /// Phase 1 の特殊キーマッチが衝突しないようにするため、Opus コード
    /// レビュー指摘）。
    #[must_use]
    pub fn to_combos(self, skip_shift_space: bool) -> Vec<awase::config::ParsedKeyCombo> {
        let mut combos = Vec::new();
        if self.ctrl_space_is_toggle {
            combos.push(awase::config::ParsedKeyCombo {
                ctrl: true,
                shift: false,
                alt: false,
                vk: crate::vk::VK_SPACE,
            });
        }
        if self.shift_space_is_toggle && !skip_shift_space {
            combos.push(awase::config::ParsedKeyCombo {
                ctrl: false,
                shift: true,
                alt: false,
                vk: crate::vk::VK_SPACE,
            });
        }
        combos
    }
}

/// bug report用（ADR-148）: `MSIME`直下の5つのDWORD値を解釈せず生のまま
/// 返す。`MsImeKeyAssignment`等の解釈済み型（`== Some(1)`/`== Some(2)`で
/// 分岐、それ以外は「宣言なし」に潰す）と違い、未知の値（将来値`3`以降等）
/// やマスタースイッチOFF時の実際の登録値も報告からそのまま読み取れる
/// ようにするため。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawKeyAssignmentDwords {
    pub is_key_assignment_enabled: Option<u32>,
    pub key_assignment_muhenkan: Option<u32>,
    pub key_assignment_henkan: Option<u32>,
    pub key_assignment_ctrl_space: Option<u32>,
    pub key_assignment_shift_space: Option<u32>,
}

/// MS-IME キー割当ての読み取り結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsImeKeyAssignment {
    /// `IsKeyAssignmentEnabled` — 割当て機能のマスタースイッチ
    pub enabled: bool,
    /// `KeyAssignmentMuhenkan` == 0 — 無変換キーに IME-オンが割り当てられている。
    /// ADR-199 T12実機確認で「0は既定ではなく明示的なIME-オン割当て」と確定した
    /// （opusコードレビュー指摘、値1/2と同じ「二重オーナー」リスクがあるため警告対象に含める）。
    pub muhenkan_ime_on: bool,
    /// `KeyAssignmentHenkan` == 0 — 変換キーに IME-オンが割り当てられている（無変換と対称）。
    pub henkan_ime_on: bool,
    /// `KeyAssignmentMuhenkan` == 1 — 無変換キーに IME-オフが割り当てられている
    pub muhenkan_ime_off: bool,
    /// `KeyAssignmentHenkan` == 1 — 変換キーに IME-オフが割り当てられている（無変換と対称。
    /// ADR-199 T12で確定するまでは「1 = IME-オン」という誤った前提だった）
    pub henkan_ime_off: bool,
    // 値2（IME-オン/オフのトグル）は警告対象にしない: ADR-199 T17 Phase 4 で awase が単独タップの開閉を肩代わりする
    // （`KeyEffectKeymap::msime_native_key_role`、発火は ADR-206 の role_open_action（単独タップが Passthrough のときだけ。Suppress は IME を動かさない））。
    /// MS-IME本体の「以前のバージョンのMicrosoft IMEを使う」互換モード（ADR-197決定4）。
    /// `Some(true)`のときは、この値がどれであっても実際には効かない（T12実機確認）ので、
    /// [`Self::conflict_warning`]は警告そのものを抑制する（誤警告防止）。
    pub compat_mode: Option<bool>,
}

impl MsImeKeyAssignment {
    /// awase と競合する割当てが有効なら、警告文（診断ログ/ポップアップ共用の本文）を返す。
    ///
    /// マスタースイッチが無効、全キーとも既定（かな切替/再変換）、または互換モードON
    /// （値が効かないので警告しても誤り、T12）なら `None`。値2（トグル）は ADR-199 T17 Phase 4 で
    /// awaseが決定16のとおり尊重・肩代わりする（互換モードOFFのとき）ので警告しない。
    #[must_use]
    pub fn conflict_warning(&self) -> Option<String> {
        if !self.enabled || self.compat_mode == Some(true) {
            return None;
        }
        let assigned: Vec<&str> = [
            self.muhenkan_ime_on.then_some("無変換キー → IME-オン"),
            self.henkan_ime_on.then_some("変換キー → IME-オン"),
            self.muhenkan_ime_off.then_some("無変換キー → IME-オフ"),
            self.henkan_ime_off.then_some("変換キー → IME-オフ"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if assigned.is_empty() {
            return None;
        }
        Some(format!(
            "MS-IME のキー割り当てが awase と競合しています:\n  {}\n\
             awase は無変換/変換キーを親指シフトキーとして使うため、\
             この割り当てが有効だと IME の ON/OFF が awase の管理外で切り替わり、\
             親指シフト入力が生ローマ字で出る等の不具合の原因になります。\n\
             IME の ON/OFF は awase のキー設定をご利用ください（既定: Ctrl+変換 / \
             Ctrl+無変換。無変換/変換の単独キーも、awase 側に bare で設定すれば \
             単独タップ確定時の強制ON/OFFとして使用できます）。",
            assigned.join("、")
        ))
    }
}

#[cfg(windows)]
mod windows_impl {
    use crate::runtime::Runtime;

    use super::MsImeKeyAssignment;

    /// アクティブ IME が MS-IME と確定したときに呼ぶ: 競合割当てを検出したら
    /// 警告ログ + 解除案内ポップアップを出す（同一内容の警告はプロセス内で一度だけ）。
    ///
    /// 呼び出しタイミングは `WM_IME_KIND_CHANGED`（CLSID ベース判定の確定/変化時）。
    /// GJI 利用中はこの関数自体が呼ばれないため、MS-IME 非ユーザーには表示されない。
    /// awase 起動後にレジストリを変更した場合、次の kind 確定イベント（GJI⇔MS-IME
    /// 切替 or 再起動）で再チェックされる。
    /// ダイアログは別スレッドに出す — メインスレッドの `MessageBoxW` はモーダル
    /// メッセージループでフックのスレッドメッセージ処理を止めてしまうため。
    /// `app`（`&mut Runtime`）はデデュープラッチの読み書きにのみ使う——`Runtime`は
    /// スレッド跨ぎ不可（`SingleThreadCell`前提）なので、下で呼ぶ
    /// `spawn_yes_open_ime_settings_dialog`が起動する別スレッドへは渡さないこと。
    pub(crate) fn check_and_warn(app: &mut Runtime) {
        let assignment = read_from_registry();
        tracing::info!("[msime-keyassign] {assignment:?}");
        let Some(warning) = assignment.conflict_warning() else {
            // 競合なし → 警告履歴をリセット（後で有効化されたら再警告できるように）
            app.reset_msime_key_assignment_warned();
            return;
        };
        let packed = u8::from(assignment.henkan_ime_off)
            | (u8::from(assignment.muhenkan_ime_off) << 1)
            | (u8::from(assignment.henkan_ime_on) << 4)
            | (u8::from(assignment.muhenkan_ime_on) << 5);
        if app.swap_msime_key_assignment_warned(packed) == Some(packed) {
            return; // 同じ内容で警告済み
        }
        tracing::warn!("[msime-keyassign] {}", warning.replace('\n', " "));
        let text = format!(
            "{warning}\n\n\
             いますぐ Windows の設定画面を開いて解除しますか？\n\n\
             開いたら「キーとタッチのカスタマイズ」で\n\
             「キーの割り当て」をオフにするか、\n\
             無変換/変換キーの割り当てを既定（かな切替 / 再変換）に戻してください。"
        );
        spawn_yes_open_ime_settings_dialog("awase - MS-IME キー割り当ての競合", text);
    }

    const MSIME_SUBKEY: windows::core::PCWSTR =
        windows::core::w!("Software\\Microsoft\\IME\\15.0\\IMEJP\\MSIME");

    /// `HKCU\...\MSIME` の DWORD 値を読む。値が存在しなければ `None`。
    fn read_dword(value_name: windows::core::PCWSTR) -> Option<u32> {
        use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
        let mut data: u32 = 0;
        let mut size = u32::try_from(size_of::<u32>()).unwrap_or(4);
        // SAFETY: HKEY_CURRENT_USER は擬似ハンドル。サブキー・値名は NUL 終端済み UTF-16。
        //         data/size は呼び出し中有効なスタック上のバッファ。
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                MSIME_SUBKEY,
                value_name,
                RRF_RT_REG_DWORD,
                None,
                Some((&raw mut data).cast()),
                Some(&raw mut size),
            )
        };
        result.is_ok().then_some(data)
    }

    /// レジストリから MS-IME キー割当てを読み取る。
    ///
    /// 値が存在しない場合は既定（割当てなし）として扱う。
    #[must_use]
    fn read_from_registry() -> MsImeKeyAssignment {
        use windows::core::w;
        let muhenkan = read_dword(w!("KeyAssignmentMuhenkan"));
        let henkan = read_dword(w!("KeyAssignmentHenkan"));
        MsImeKeyAssignment {
            enabled: read_dword(w!("IsKeyAssignmentEnabled")) == Some(1),
            muhenkan_ime_on: muhenkan == Some(0),
            henkan_ime_on: henkan == Some(0),
            muhenkan_ime_off: muhenkan == Some(1),
            henkan_ime_off: henkan == Some(1),
            compat_mode: crate::msime_legacy_keymap::read_legacy_compat_mode_enabled(),
        }
    }

    /// レジストリから `KeyAssignmentCtrlSpace`/`KeyAssignmentShiftSpace`
    /// （ADR-092 決定D Step4a）を読み取る。マスタースイッチ
    /// （`IsKeyAssignmentEnabled`）が無効なら両方 `false`（MS-IME 自身も
    /// この割当てを無視するため）。
    #[must_use]
    pub(crate) fn read_toggle_assignment_from_registry() -> super::MsImeToggleAssignment {
        use windows::core::w;
        if read_dword(w!("IsKeyAssignmentEnabled")) != Some(1) {
            return super::MsImeToggleAssignment::default();
        }
        super::MsImeToggleAssignment {
            ctrl_space_is_toggle: read_dword(w!("KeyAssignmentCtrlSpace")) == Some(2),
            shift_space_is_toggle: read_dword(w!("KeyAssignmentShiftSpace")) == Some(2),
        }
    }

    /// bug report用（ADR-148）: 5つのDWORD値を解釈せず生のまま読む。
    /// マスタースイッチの値に関わらず個々の値を読む（`read_toggle_
    /// assignment_from_registry`と違い、マスタースイッチOFF時の実際の登録値を
    /// 隠さないため）。
    #[must_use]
    pub(crate) fn read_raw_key_assignment_dwords() -> super::RawKeyAssignmentDwords {
        use windows::core::w;
        super::RawKeyAssignmentDwords {
            is_key_assignment_enabled: read_dword(w!("IsKeyAssignmentEnabled")),
            key_assignment_muhenkan: read_dword(w!("KeyAssignmentMuhenkan")),
            key_assignment_henkan: read_dword(w!("KeyAssignmentHenkan")),
            key_assignment_ctrl_space: read_dword(w!("KeyAssignmentCtrlSpace")),
            key_assignment_shift_space: read_dword(w!("KeyAssignmentShiftSpace")),
        }
    }

    /// ADR-191: Microsoft IME本体の打鍵時予測（`key_effect_predictor`）に使うキーマップを、キー割り当て
    /// （`IsKeyAssignmentEnabled`/`KeyAssignmentHenkan`/`KeyAssignmentMuhenkan`）と互換モード
    /// （ADR-197決定4、ADR-199決定17・T13）から作る。呼び出しは`KeymapCache`が版
    /// （[`native_assignment_stamp`]）の変化時だけに絞る。
    pub(crate) fn read_key_effect_keymap_native(
    ) -> crate::state::key_effect_predictor::KeyEffectKeymap {
        read_key_effect_keymap_native_with_reassignment_bits().0
    }

    /// [`read_key_effect_keymap_native`]と、ADR-192状態依存キーモード警告
    /// （`state_dependent_key_warning::detect_msime`）が使うdedup用ビット
    /// （無変換/変換に明示値があるか）を、**同じレジストリ読み取り結果**から組み立てる。
    /// `runtime/mod.rs::check_state_dependent_mode_keys`と予測・役割判定の両経路が別々に
    /// レジストリを読んで解釈をずらさないための一本化（ADR-199 T17 opusレビュー M3）。
    pub(crate) fn read_key_effect_keymap_native_with_reassignment_bits(
    ) -> (crate::state::key_effect_predictor::KeyEffectKeymap, u8) {
        let raw = read_raw_key_assignment_dwords();
        let assignment_enabled = raw.is_key_assignment_enabled == Some(1);
        let compat_mode = crate::msime_legacy_keymap::read_legacy_compat_mode_enabled();
        // ADR-254: 旧UIのキーテンプレート(`keystyle`)が既定でない構成は、`MSIME_NATIVE`の予測を止める。
        let (keystyle, legacy_hash) = crate::msime_legacy_keymap::read_keystyle();
        let keymap =
            crate::state::key_effect_predictor::KeyEffectKeymap::for_msime_native_with_legacy(
                assignment_enabled,
                raw.key_assignment_henkan,
                raw.key_assignment_muhenkan,
                compat_mode,
                keystyle,
                legacy_hash,
            );
        // ADR-254 第二段: 互換 ON の Custom は、無変換/変換だけ、Custom の表(S4key・key)から予測する。
        // それ以外の止める構成(名前付き・未知・互換が読めない)は表を読まない。
        let keymap = if matches!(
            keystyle,
            crate::state::key_effect_predictor::MsImeKeystyle::Custom
        ) && compat_mode == Some(true)
        {
            let (s4key, key) = crate::msime_legacy_keymap::read_custom_s4key_and_key();
            keymap.with_legacy_custom_cells(
                crate::state::key_effect_predictor::legacy_custom_cells(
                    s4key.as_deref(),
                    key.as_deref(),
                ),
            )
        } else {
            keymap
        };
        // 値0もADR-199 T12で明示的な割り当て(IME-オン)と確定した(既定ではない)ので、
        // 「値があるか」だけを見る(`!= 0`ではない、M5)。マスタースイッチ
        // (IsKeyAssignmentEnabled)もbitsに含める——`for_msime_native`のreassigned判定
        // (`assignment_enabled && v.is_some()`、実際の警告分類henkan_reassigned/
        // muhenkan_reassignedを左右する)と同じ条件でないと、マスタースイッチだけが
        // 有効化/無効化された場合にreassignedはfalse→trueへ変わるのにbitsが不変のまま
        // となり、新規に出すべき警告がWarningTrackerのdedupで握り潰される
        // (opusコードレビュー指摘)。
        let bits = u8::from(assignment_enabled)
            | (u8::from(assignment_enabled && raw.key_assignment_henkan.is_some()) << 1)
            | (u8::from(assignment_enabled && raw.key_assignment_muhenkan.is_some()) << 2);
        (keymap, bits)
    }

    /// `read_key_effect_keymap_native`の版。3つのDWORDの値（不在は`u32::MAX`で表す）と
    /// 互換モード（`Some(true)`=1・`Some(false)`=0・`None`=`u32::MAX`）、旧UIのキーテンプレート
    /// （ADR-254）を詰めた値（レジストリの再読み取りだけで、ファイルは読まない）。互換モードは
    /// 1つ目のタプル要素の上位32bit（`IsKeyAssignmentEnabled`は下位32bitしか使わない）に詰め、
    /// 2つ目はハッシュ。
    pub(crate) fn native_assignment_stamp() -> (u64, u64) {
        let raw = read_raw_key_assignment_dwords();
        let compat_mode = crate::msime_legacy_keymap::read_legacy_compat_mode_enabled();
        let v = |x: Option<u32>| u64::from(x.unwrap_or(u32::MAX));
        // 2つ目は「版」として等値比較されるだけの不透明な値なので、旧UIのキーテンプレート
        // (`keystyle`の種別と、`Custom`なら`key`・`S4key`の中身、ADR-254)を混ぜたハッシュにする。
        let mut second = crate::msime_legacy_keymap::STYLE_HASH_SEED;
        for (name, x) in [
            ("henkan", v(raw.key_assignment_henkan)),
            ("muhenkan", v(raw.key_assignment_muhenkan)),
            ("keystyle", crate::msime_legacy_keymap::keystyle_stamp_mix()),
        ] {
            second =
                crate::msime_legacy_keymap::mix_style_value(second, name, Some(&x.to_le_bytes()));
        }
        (
            v(raw.is_key_assignment_enabled) | (v(compat_mode.map(u32::from)) << 32),
            second,
        )
    }

    /// 別スレッドで Yes/No の警告ダイアログを表示し、Yes なら MS-IME 設定画面を
    /// 開く。`check_and_warn`（キー割り当て競合）と `tray::show_kana_lock_help_dialog`
    /// （かな入力ロック検知、issue #137）が共有する——どちらも「別スレッドで
    /// MessageBoxW→IDYESでopen_ime_settings」という同一構造だったのを統合した。
    ///
    /// `MessageBoxW` はユーザー応答まで呼び出しスレッドをブロックするが、
    /// 別スレッドなのでメインのメッセージループ/フック処理は止めない。
    pub(crate) fn spawn_yes_open_ime_settings_dialog(title: &'static str, text: String) {
        spawn_yes_dialog(title, text, open_ime_settings);
    }

    /// 別スレッドでYes/No警告を表示し、Yesなら呼び出し元が指定した遷移先を開く。
    pub(crate) fn spawn_yes_dialog(
        title: &'static str,
        text: String,
        on_yes: impl FnOnce() + Send + 'static,
    ) {
        std::thread::spawn(move || {
            use windows::core::PCWSTR;
            use windows::Win32::UI::WindowsAndMessaging::{
                MessageBoxW, IDYES, MB_ICONWARNING, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
            };

            let title_wide = crate::win32::to_wide(title);
            let text_wide = crate::win32::to_wide(&text);

            // SAFETY: title_wide/text_wide は NUL 終端済み UTF-16 で呼び出し中有効。
            let result = unsafe {
                MessageBoxW(
                    None,
                    PCWSTR(text_wide.as_ptr()),
                    PCWSTR(title_wide.as_ptr()),
                    // MB_TOPMOST | MB_SETFOREGROUND: バックグラウンドスレッドの owner
                    // なし MessageBox はフォアグラウンドロックで現在のウィンドウの裏に
                    // 出る（タスクバー点滅のみで気づけない）ため、最前面に強制する。
                    MB_YESNO | MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND,
                )
            };
            if result == IDYES {
                on_yes();
            }
        });
    }

    /// `ms-settings:regionlanguage-jpnime`（Microsoft IME 設定ページ）を開く。
    pub(crate) fn open_ime_settings() {
        use windows::core::{w, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        // SAFETY: 引数はすべて静的リテラルの NUL 終端 UTF-16。
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                w!("ms-settings:regionlanguage-jpnime"),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecuteW returns HINSTANCE > 32 on success
        if result.0 as isize > 32 {
            tracing::info!("[msime-keyassign] ms-settings:regionlanguage-jpnime を開きました");
        } else {
            tracing::warn!("[msime-keyassign] 設定画面を開けませんでした (result={result:?})");
        }
    }
}

#[cfg(windows)]
pub(crate) use windows_impl::{
    check_and_warn, native_assignment_stamp, open_ime_settings, read_key_effect_keymap_native,
    read_key_effect_keymap_native_with_reassignment_bits, read_raw_key_assignment_dwords,
    read_toggle_assignment_from_registry, spawn_yes_dialog, spawn_yes_open_ime_settings_dialog,
};

#[cfg(test)]
mod tests {
    use super::{MsImeKeyAssignment, MsImeToggleAssignment};

    fn assign(enabled: bool, muhenkan: bool, henkan: bool) -> MsImeKeyAssignment {
        assign_with_compat_mode(enabled, muhenkan, henkan, None)
    }

    fn assign_with_compat_mode(
        enabled: bool,
        muhenkan: bool,
        henkan: bool,
        compat_mode: Option<bool>,
    ) -> MsImeKeyAssignment {
        MsImeKeyAssignment {
            enabled,
            muhenkan_ime_on: false,
            henkan_ime_on: false,
            muhenkan_ime_off: muhenkan,
            henkan_ime_off: henkan,
            compat_mode,
        }
    }

    fn assign_ime_on(
        enabled: bool,
        muhenkan_ime_on: bool,
        henkan_ime_on: bool,
    ) -> MsImeKeyAssignment {
        MsImeKeyAssignment {
            enabled,
            muhenkan_ime_on,
            henkan_ime_on,
            muhenkan_ime_off: false,
            henkan_ime_off: false,
            compat_mode: None,
        }
    }

    #[test]
    fn no_warning_when_master_switch_disabled() {
        // 値が残っていてもマスタースイッチ OFF なら MS-IME は割当てを無視する
        assert_eq!(assign(false, true, true).conflict_warning(), None);
    }

    #[test]
    fn no_warning_when_both_keys_are_default() {
        assert_eq!(assign(true, false, false).conflict_warning(), None);
    }

    #[test]
    fn warns_on_muhenkan_ime_off() {
        // 「変換キー → IME-オフ」は「無変換キー → IME-オフ」の部分文字列なので、単純な
        // contains の否定では区別できない。結合直後の行として一致するかで確認する。
        let w = assign(true, true, false).conflict_warning().unwrap();
        assert!(w.contains("競合しています:\n  無変換キー → IME-オフ\nawase は"));
    }

    #[test]
    fn warns_on_henkan_ime_off() {
        let w = assign(true, false, true).conflict_warning().unwrap();
        assert!(w.contains("競合しています:\n  変換キー → IME-オフ\nawase は"));
    }

    #[test]
    fn warns_on_both_assignments() {
        let w = assign(true, true, true).conflict_warning().unwrap();
        assert!(w.contains("無変換キー → IME-オフ、変換キー → IME-オフ"));
    }

    /// ADR-199 T17 m1: 互換モードON（値が効かない、T12実機確認）では値1の警告も誤りになるので出さない。
    #[test]
    fn no_warning_when_compat_mode_on_even_with_value_1() {
        assert_eq!(
            assign_with_compat_mode(true, true, true, Some(true)).conflict_warning(),
            None
        );
    }

    #[test]
    fn warns_when_compat_mode_off_or_unknown() {
        assert!(assign_with_compat_mode(true, true, false, Some(false))
            .conflict_warning()
            .is_some());
        assert!(assign_with_compat_mode(true, true, false, None)
            .conflict_warning()
            .is_some());
    }

    /// ADR-199 T17 Phase 4: 値2（トグル）は awase が肩代わりするので警告しない（`MsImeKeyAssignment`は値2を持たない。
    /// 値2だけの構成は他の割り当てフラグがすべて false になり、警告なしになる）。値2と値1が混在するときは値1だけが列挙される。
    #[test]
    fn value_2_alone_does_not_warn_and_mixed_lists_only_value_1() {
        assert_eq!(assign(true, false, false).conflict_warning(), None);
        let w = assign(true, true, false).conflict_warning().unwrap();
        assert!(w.contains("無変換キー → IME-オフ"));
        assert!(!w.contains("トグル"));
    }

    /// opusコードレビュー指摘: ADR-199 T12実機確認で「値0は既定ではなく明示的な
    /// IME-オン割当て」と確定したため、値1/2と同様に警告対象に含めること。
    #[test]
    fn warns_on_muhenkan_ime_on() {
        let w = assign_ime_on(true, true, false).conflict_warning().unwrap();
        assert!(w.contains("無変換キー → IME-オン"));
    }

    #[test]
    fn warns_on_henkan_ime_on() {
        let w = assign_ime_on(true, false, true).conflict_warning().unwrap();
        assert!(w.contains("変換キー → IME-オン"));
    }

    // ── ADR-092 決定D Step4a: MsImeToggleAssignment::to_combos ──

    #[test]
    fn to_combos_empty_when_neither_is_toggle() {
        let assignment = MsImeToggleAssignment::default();
        assert!(assignment.to_combos(false).is_empty());
    }

    #[test]
    fn to_combos_includes_ctrl_space_when_toggle() {
        let assignment = MsImeToggleAssignment {
            ctrl_space_is_toggle: true,
            shift_space_is_toggle: false,
        };
        let combos = assignment.to_combos(false);
        assert_eq!(combos.len(), 1);
        assert!(combos[0].ctrl && !combos[0].shift);
    }

    #[test]
    fn to_combos_includes_shift_space_when_toggle_and_not_skipped() {
        let assignment = MsImeToggleAssignment {
            ctrl_space_is_toggle: false,
            shift_space_is_toggle: true,
        };
        let combos = assignment.to_combos(false);
        assert_eq!(combos.len(), 1);
        assert!(combos[0].shift && !combos[0].ctrl);
    }

    /// Space が親指キーの場合、Shift+Space の自動検出は反映しない
    /// （Space 親指キーの Shift リテラル送出機能との衝突を避けるため）。
    #[test]
    fn to_combos_skips_shift_space_when_requested() {
        let assignment = MsImeToggleAssignment {
            ctrl_space_is_toggle: true,
            shift_space_is_toggle: true,
        };
        let combos = assignment.to_combos(true);
        assert_eq!(combos.len(), 1);
        assert!(combos[0].ctrl && !combos[0].shift);
    }
}
