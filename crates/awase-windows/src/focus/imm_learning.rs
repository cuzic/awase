#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! IMM32 クロスプロセス制御能力の学習（ImmGetDefaultIMEWnd による初回判定）

use crate::focus::AppKind;
use windows::Win32::Foundation::HWND;

/// ImmGetDefaultIMEWnd=NULL の場合、そのアプリの IMM32 制御を `Unavailable` と記録する。
///
/// `new_app_kind` が `Win32` かつ `class_name` が未学習の場合にのみ
/// `ImmGetDefaultIMEWnd` を呼び出して結果をキャッシュに反映する。
///
/// BUG-56（2026-08-07 実機）: 以前は NULL を1回観測しただけで即座に `Unavailable` を
/// 確定していたが、Qt 等のジェネリックなウィンドウクラス名（例: `Qt663QWindowIcon`）は
/// 本物のテキスト入力欄とは無関係な一時ウィンドウ（通知アイコン等）でも使い回されるため、
/// その一時ウィンドウがたまたま NULL を返しただけで、同じクラス名を持つ本物の入力欄まで
/// 巻き込んで IMM32 クロスプロセス制御（`ImmCrossProcessStrategy`）を諦めてしまっていた。
/// LINE でこれが発生し、`ImmCrossProcessStrategy` から VK ベースの `Blacklist force-ON`
/// へ切り替わった結果、物理 IME キーが LINE 側の composition に漏れて文字が重複コミット
/// される（「でででで」「はははは」）不具合が実機で確認された。
/// `ImmCapabilityStore::record_null_probe`（閾値回連続で確定）に委譲することで、
/// 単発の誤判定では確定しないようにする。
///
/// # Safety
/// Win32 API (`ImmGetDefaultIMEWnd`) を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn learn_imm_capability_on_focus(
    platform: &mut crate::platform::WindowsPlatform,
    hwnd: HWND,
    class_name: &str,
    new_app_kind: AppKind,
) {
    if new_app_kind != AppKind::Win32 {
        return;
    }
    if platform.focus.imm_capability(class_name).is_some() {
        return;
    }

    if unsafe { crate::imm::get_ime_wnd(hwnd) }.is_none() {
        log::info!(
            "IMM32 capability: ImmGetDefaultIMEWnd=NULL, 疑いを記録 (class={class_name})。\
             閾値回連続で観測されたら Unavailable として確定する（BUG-56対策）"
        );
        platform.record_imm_null_probe(class_name.to_string());
    } else {
        platform.clear_imm_pending_unavailable(class_name);
    }
}
