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
/// 単発の誤判定では確定しないようにする。学習キーは `(process_name, class_name)` とし、
/// winit の `"Window Class"` のような汎用クラス名によるプロセス間の衝突（BUG-107）
/// も防ぐ。
///
/// `process_name` は `AppImeProfile::resolve` と同じ遅延クロージャ方式（`get_process_name`
/// が Win32 プロセスハンドルを開く高コスト API のため）。**必ず `new_app_kind == Win32`
/// 判定の直後・「既に学習済みか」判定の直前で1回だけ評価すること**——呼び出し元
/// （`runtime/focus_tracking.rs::classify_focus_probe`）はこの評価タイミングを前提に、
/// クロージャ内で得た値を `ClassifiedFocus::process_name` へ横取りして
/// `CurrentFocus::update_with_process_name` に再利用し、同一フォーカスプローブ内で
/// `get_process_name` が2回呼ばれることを防いでいる。この関数の早期return順序を
/// 変えてクロージャの評価タイミングがずれると、呼び出し元の再利用の前提が崩れ、
/// 黙って二重取得に戻る。
///
/// # Safety
/// Win32 API (`ImmGetDefaultIMEWnd`) を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn learn_imm_capability_on_focus(
    platform: &mut crate::platform::WindowsPlatform,
    hwnd: HWND,
    process_name: impl FnOnce() -> String,
    class_name: &str,
    new_app_kind: AppKind,
) {
    if new_app_kind != AppKind::Win32 {
        return;
    }
    let process_name = process_name();
    if process_name.is_empty() {
        return;
    }
    if platform
        .focus
        .imm_capability(&process_name, class_name)
        .is_some()
    {
        return;
    }

    if unsafe { crate::imm::get_ime_wnd(hwnd) }.is_none() {
        log::info!(
            "IMM32 capability: ImmGetDefaultIMEWnd=NULL, 疑いを記録 \
             (process={process_name}, class={class_name})。\
             閾値回連続で観測されたら Unavailable として確定する（BUG-56対策）"
        );
        platform.record_imm_null_probe(process_name, class_name.to_string());
    } else {
        platform.clear_imm_pending_unavailable(&process_name, class_name);
    }
}
