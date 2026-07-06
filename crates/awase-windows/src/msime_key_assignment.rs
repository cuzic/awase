//! MS-IME「キーとタッチのカスタマイズ」割当ての起動時検出と解除案内
//!
//! # 背景（防ぐバグクラス: IME 状態の二重オーナー）
//!
//! MS-IME は設定で 無変換キー=IME-オフ / 変換キー=IME-オン を割り当てられる。
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
//! - `KeyAssignmentMuhenkan` (DWORD) — 1 = IME-オフ / 0 = かな切替（既定）
//! - `KeyAssignmentHenkan` (DWORD) — 1 = IME-オン / 0 = 再変換（既定）
//!
//! レジストリは**読み取り専用**。書き換えによる自動解除は行わない
//! （動作中 IME への反映タイミングが保証されず、ユーザー設定への侵襲になるため）。
//! 解除はユーザー自身に `ms-settings:regionlanguage-jpnime` で行ってもらう。

/// MS-IME キー割当ての読み取り結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsImeKeyAssignment {
    /// `IsKeyAssignmentEnabled` — 割当て機能のマスタースイッチ
    pub enabled: bool,
    /// `KeyAssignmentMuhenkan` == 1 — 無変換キーに IME-オフが割り当てられている
    pub muhenkan_ime_off: bool,
    /// `KeyAssignmentHenkan` == 1 — 変換キーに IME-オンが割り当てられている
    pub henkan_ime_on: bool,
}

impl MsImeKeyAssignment {
    /// awase と競合する割当てが有効なら、警告文（診断ログ/ポップアップ共用の本文）を返す。
    ///
    /// マスタースイッチが無効、または両キーとも既定（かな切替/再変換）なら `None`。
    #[must_use]
    pub fn conflict_warning(&self) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let assigned: Vec<&str> = [
            self.muhenkan_ime_off.then_some("無変換キー → IME-オフ"),
            self.henkan_ime_on.then_some("変換キー → IME-オン"),
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
             IME の ON/OFF は awase のキー設定（既定: Ctrl+変換 / Ctrl+無変換）をご利用ください。",
            assigned.join("、")
        ))
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::sync::atomic::{AtomicU8, Ordering};

    use super::MsImeKeyAssignment;

    /// 前回警告を出した割当て内容（bit0=変換, bit1=無変換）。`NOT_WARNED` = 未警告。
    ///
    /// 同じ内容で繰り返しポップアップを出さないためのデデュープ。競合が解消された
    /// 観測でリセットされるため、割当てを解除→再度有効化した場合は再警告される。
    static LAST_WARNED: AtomicU8 = AtomicU8::new(NOT_WARNED);
    const NOT_WARNED: u8 = 0xFF;

    /// アクティブ IME が MS-IME と確定したときに呼ぶ: 競合割当てを検出したら
    /// 警告ログ + 解除案内ポップアップを出す（同一内容の警告はプロセス内で一度だけ）。
    ///
    /// 呼び出しタイミングは `WM_IME_KIND_CHANGED`（CLSID ベース判定の確定/変化時）。
    /// GJI 利用中はこの関数自体が呼ばれないため、MS-IME 非ユーザーには表示されない。
    /// awase 起動後にレジストリを変更した場合、次の kind 確定イベント（GJI⇔MS-IME
    /// 切替 or 再起動）で再チェックされる。
    /// ダイアログは別スレッドに出す — メインスレッドの `MessageBoxW` はモーダル
    /// メッセージループでフックのスレッドメッセージ処理を止めてしまうため。
    pub fn check_and_warn() {
        let assignment = read_from_registry();
        log::info!("[msime-keyassign] {assignment:?}");
        let Some(warning) = assignment.conflict_warning() else {
            // 競合なし → 警告履歴をリセット（後で有効化されたら再警告できるように）
            LAST_WARNED.store(NOT_WARNED, Ordering::Relaxed);
            return;
        };
        let packed = u8::from(assignment.henkan_ime_on)
            | (u8::from(assignment.muhenkan_ime_off) << 1);
        if LAST_WARNED.swap(packed, Ordering::Relaxed) == packed {
            return; // 同じ内容で警告済み
        }
        log::warn!("[msime-keyassign] {}", warning.replace('\n', " "));
        std::thread::spawn(move || show_conflict_dialog(&warning));
    }

    const MSIME_SUBKEY: windows::core::PCWSTR =
        windows::core::w!("Software\\Microsoft\\IME\\15.0\\IMEJP\\MSIME");

    /// `HKCU\...\MSIME` の DWORD 値を読む。値が存在しなければ `None`。
    fn read_dword(value_name: windows::core::PCWSTR) -> Option<u32> {
        use windows::Win32::System::Registry::{
            RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD,
        };
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
        MsImeKeyAssignment {
            enabled: read_dword(w!("IsKeyAssignmentEnabled")) == Some(1),
            muhenkan_ime_off: read_dword(w!("KeyAssignmentMuhenkan")) == Some(1),
            henkan_ime_on: read_dword(w!("KeyAssignmentHenkan")) == Some(1),
        }
    }

    /// 競合警告のポップアップを表示し、Yes なら MS-IME 設定画面を開く。
    ///
    /// `MessageBoxW` はユーザー応答まで呼び出し元をブロックする
    /// （起動時の `autostart::ask_user` と同じ扱い。フック導入前なので入力への影響はない）。
    fn show_conflict_dialog(warning: &str) {
        use windows::core::{w, PCWSTR};
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDYES, MB_ICONWARNING, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
        };

        let text = format!(
            "{warning}\n\n\
             いますぐ Windows の設定画面を開いて解除しますか？\n\n\
             開いたら「キーとタッチのカスタマイズ」で\n\
             「キーの割り当て」をオフにするか、\n\
             無変換/変換キーの割り当てを既定（かな切替 / 再変換）に戻してください。"
        );
        let text_wide = crate::win32::to_wide(&text);

        // SAFETY: text_wide は NUL 終端済み UTF-16 で呼び出し中有効。タイトルは静的リテラル。
        let result = unsafe {
            MessageBoxW(
                None,
                PCWSTR(text_wide.as_ptr()),
                w!("awase - MS-IME キー割り当ての競合"),
                // MB_TOPMOST | MB_SETFOREGROUND: バックグラウンドスレッドの owner なし
                // MessageBox はフォアグラウンドロックで現在のウィンドウの裏に出る
                // （タスクバー点滅のみで気づけない）ため、最前面に強制する。
                MB_YESNO | MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND,
            )
        };
        if result == IDYES {
            open_ime_settings();
        }
    }

    /// `ms-settings:regionlanguage-jpnime`（Microsoft IME 設定ページ）を開く。
    fn open_ime_settings() {
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
            log::info!("[msime-keyassign] ms-settings:regionlanguage-jpnime を開きました");
        } else {
            log::warn!("[msime-keyassign] 設定画面を開けませんでした (result={result:?})");
        }
    }
}

#[cfg(windows)]
pub use windows_impl::check_and_warn;

#[cfg(test)]
mod tests {
    use super::MsImeKeyAssignment;

    fn assign(enabled: bool, muhenkan: bool, henkan: bool) -> MsImeKeyAssignment {
        MsImeKeyAssignment {
            enabled,
            muhenkan_ime_off: muhenkan,
            henkan_ime_on: henkan,
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
        let w = assign(true, true, false).conflict_warning().unwrap();
        assert!(w.contains("無変換キー → IME-オフ"));
        assert!(!w.contains("変換キー → IME-オン"));
    }

    #[test]
    fn warns_on_henkan_ime_on() {
        let w = assign(true, false, true).conflict_warning().unwrap();
        assert!(w.contains("変換キー → IME-オン"));
        assert!(!w.contains("無変換キー → IME-オフ"));
    }

    #[test]
    fn warns_on_both_assignments() {
        let w = assign(true, true, true).conflict_warning().unwrap();
        assert!(w.contains("無変換キー → IME-オフ、変換キー → IME-オン"));
    }
}
