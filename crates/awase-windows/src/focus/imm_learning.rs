#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! IMM32 クロスプロセス制御能力の学習（ImmGetDefaultIMEWnd による初回判定）

use crate::focus::AppKind;
use crate::state::imm_learning_plan::{
    plan_imm_learning, plan_probe_result, ImmLearnPlan, ImmProbeRecord,
};
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
    // observe → decide（純粋、state/imm_learning_plan.rs）。`process_name` は Win32 のときだけ、
    // ちょうど 1 回評価する（上記ドキュメントの契約）。学習済みの照会は名前が空でないときだけ。
    let process_name = if new_app_kind == AppKind::Win32 {
        process_name()
    } else {
        String::new()
    };
    let already_learned = new_app_kind == AppKind::Win32
        && !process_name.is_empty()
        && platform
            .focus
            .imm_capability(&process_name, class_name)
            .is_some();
    if plan_imm_learning(new_app_kind, process_name.is_empty(), already_learned)
        != ImmLearnPlan::Probe
    {
        // 空のプロセス名で諦める理由（BUG-107）は `ImmLearnSkip::EmptyProcessName` の doc を参照。
        // 代替案（空文字列をキーとして進める）は採らない: 「プロセス名を解決できない全プロセス」の
        // 共有バケツになり、本モジュール冒頭が説明する BUG-107 を別の軸で再現する。
        // トレードオフ: 名前を解決できないウィンドウは `Imm32Unavailable` を学習できず、
        // フォーカスのたびに ImmCross 経路を試みるが、`state/actuation_chain.rs` のフォールバック
        // チェーンが個々の失敗を吸収するため、誤学習で他プロセスを巻き込むより安全側。
        return;
    }

    // execute: ImmGetDefaultIMEWnd を読み、結果を記録する。
    let ime_wnd_is_null = unsafe { crate::imm::get_ime_wnd(hwnd) }.is_none();
    match plan_probe_result(ime_wnd_is_null) {
        ImmProbeRecord::RecordNullProbe => {
            tracing::info!(
                "IMM32 capability: ImmGetDefaultIMEWnd=NULL, 疑いを記録 \
                 (process={process_name}, class={class_name})。\
                 閾値回連続で観測されたら Unavailable として確定する（BUG-56対策）"
            );
            platform.record_imm_null_probe(process_name, class_name.to_string());
        }
        ImmProbeRecord::ClearPending => {
            platform.clear_imm_pending_unavailable(&process_name, class_name);
        }
    }
}
