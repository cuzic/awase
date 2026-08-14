#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(msime_key_assignment.rsと同じ個別移管パターン)
//! GJI向け設定支援ポップアップ（ADR-091 §D3.2「設定未完了時のポップアップ」/
//! §4 Phase1-5）。
//!
//! 無変換単独タップが GJI 既定の「文字種変更」動作のまま手動で「素の
//! パススルー」に設定されており（`muhenkan_solo_tap_always_suppress = false`）、
//! かつ専用Fnキー変換がまだ有効でない（`state::gji_charset_autodetect` の
//! 自動判定でも見つからなかった）場合、設定完了を促すポップアップを表示する。
//! 同意されれば `gji_charset_write` で `config1.db` へ書き込む。
//!
//! `msime_key_assignment.rs::check_and_warn` と同じパターン（別スレッドで
//! `MessageBoxW`、同一セッション中の重複表示を防ぐラッチ）を踏襲する。

#[cfg(windows)]
pub(crate) use windows_impl::maybe_show_setup_popup;

#[cfg(windows)]
mod windows_impl {
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::runtime::Runtime;

    /// ADR-091 §D3.2 の推奨Fnキー（実測済みで安全、ADR-057）。
    /// GJI キー表記（`awase_gji_config`/Mozc `key_parser` 形式）と、
    /// awase 側の VK 名の両方が必要（前者は書き込み対象、後者はエンジンへの
    /// 即時反映用）。
    const RECOMMENDED_GJI_KEY: &str = "F21";
    const RECOMMENDED_VK_NAME: &str = "VK_F21";

    /// 同一プロセス内で一度ポップアップを見せたら（同意/拒否問わず）再度出さない。
    /// GJI 検出のたびに条件チェック自体は行うが、表示は一度だけに絞る
    /// （config1.db は書き込み後も GJI 再起動まで反映されないため、書き込み
    /// 直後の再チェックで同じ条件のまま連続表示されるのを防ぐ）。
    static ALREADY_SHOWN: AtomicBool = AtomicBool::new(false);

    /// `state::gji_charset_autodetect::sync_gji_charset_autodetect` の直後に
    /// GJI 検出時のみ呼ぶ。
    pub(crate) fn maybe_show_setup_popup(app: &Runtime) {
        if ALREADY_SHOWN.load(Ordering::Relaxed) {
            return;
        }
        if app.muhenkan_dedicated_fn_key_active() {
            return; // 既に有効（手動 or 自動判定）
        }
        if !app.muhenkan_solo_tap_is_passthrough() {
            return; // 既定の「抑止」のまま = 対象外（D3.1 の通常経路）
        }
        ALREADY_SHOWN.store(true, Ordering::Relaxed);
        std::thread::spawn(show_setup_dialog);
    }

    /// 設定支援ダイアログを表示し、同意されれば書き込みを実行する。
    /// `MessageBoxW` は呼び出し元をブロックするため、必ず別スレッドから呼ぶ
    /// （メインスレッドのフックコールバックを止めないため、
    /// `msime_key_assignment.rs::show_conflict_dialog` と同じ制約）。
    fn show_setup_dialog() {
        use windows::core::{w, PCWSTR};
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDYES, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
        };

        let text = "Google 日本語入力（GJI）を検出しました。\n\n\
             無変換キーの単独打鍵が「素のパススルー」に設定されているため、\
             GJI 側のデフォルト動作（文字種変更）に依存しています。\n\n\
             専用のFnキー（F21）を使った、より安全な変換方式を有効にしますか？\n\
             （GJI の設定ファイルに、変換中のかな形状トグル用のバインドを1つ追加します。\
             反映には GJI の再起動が必要です）";
        let text_wide = crate::win32::to_wide(text);

        // SAFETY: text_wide は NUL 終端済み UTF-16 で呼び出し中有効。タイトルは静的リテラル。
        let result = unsafe {
            MessageBoxW(
                None,
                PCWSTR(text_wide.as_ptr()),
                w!("awase - GJI 設定支援"),
                MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND,
            )
        };
        if result == IDYES {
            apply_and_notify();
        }
    }

    /// ユーザー同意後、実際に `config1.db` へ書き込み、結果を通知する。
    ///
    /// この関数はダイアログスレッド（非メインスレッド）で動く。`RUNTIME`
    /// （`SingleThreadCell`）はメインスレッド以外からのアクセスを許さないため
    /// （`crate::with_app` を直接呼んではいけない）、成功時は
    /// `WM_GJI_CHARSET_FN_KEY_ACTIVATED` をメインスレッドへ投函し、
    /// `runtime::message_handlers::handle_wm_gji_charset_fn_key_activated` が
    /// `with_app` 経由で実際の反映を行う。
    fn apply_and_notify() {
        match crate::gji_charset_write::apply_dedicated_fn_key_binding(RECOMMENDED_GJI_KEY) {
            Ok(()) => {
                log::info!(
                    "[gji-charset-popup] config1.db へ専用Fnキー変換を書き込みました \
                     （{RECOMMENDED_GJI_KEY}）"
                );
                // 書き込みと同時に、次回 GJI 起動を待たずともこのプロセス内では
                // 即座にエンジン側も有効化する（手動設定ではなく自動判定扱い、
                // 手動設定フラグは立てない）。メインスレッドへ投函するのみで、
                // ここでは Runtime に一切触れない。
                if let Some(vk) =
                    <awase::types::VkCode as crate::vk::VkCodeExt>::from_name(RECOMMENDED_VK_NAME)
                {
                    crate::win32::post_to_main_thread_with(
                        crate::WM_GJI_CHARSET_FN_KEY_ACTIVATED,
                        usize::from(vk.0),
                        0,
                    );
                }
                // GJI が起動中のまま config1.db を直接書き換えているため、GJI
                // 終了時にメモリ上の（書き込み前の）内容で上書きされるリスクが
                // ある。「タスクトレイから終了→起動」はまさにこの上書きを
                // 誘発しうるため案内しない。サインアウト/インはセッション内の
                // 全プロセスを確実に終了させてから起動し直すため、この上書き
                // リスクを避けられる。
                show_result_dialog(
                    "設定を追加しました。\n\n\
                     Google 日本語入力（GJI）は現在起動中のため、今すぐ確実に反映するには\
                     サインアウトしてからサインインし直してください。\n\
                     （GJI が起動したまま「タスクトレイから終了して再起動」を行うと、\
                     GJI 終了時に書き込み前の設定で上書きされ、この変更が消えることがあります）",
                );
            }
            Err(err) => {
                log::warn!("[gji-charset-popup] config1.db への書き込みに失敗: {err}");
                show_result_dialog(&format!("設定を追加できませんでした。\n\n{err}"));
            }
        }
    }

    fn show_result_dialog(text: &str) {
        use windows::core::{w, PCWSTR};
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
        };

        let text_wide = crate::win32::to_wide(text);
        // SAFETY: text_wide は NUL 終端済み UTF-16 で呼び出し中有効。タイトルは静的リテラル。
        let _ = unsafe {
            MessageBoxW(
                None,
                PCWSTR(text_wide.as_ptr()),
                w!("awase - GJI 設定支援"),
                MB_OK | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND,
            )
        };
    }
}
