#![allow(unsafe_code)]
// Win32 / COM の呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9と同じ作法)
//! ADR-261 段階 1: タスクバーの入力インジケーターの字形を取り込む**観測**（読むだけ。IME へ何も書かない・belief へ入れない）。
//!
//! 流れ:
//!
//! 1. 前面窓の変化（`EVENT_SYSTEM_FOREGROUND` の WinEvent。[`install_foreground_hook`]）を、コールバックが
//!    `Runtime::indicator_glyph_on_foreground`（`runtime/indicator_glyph_shadow.rs`）へ渡す。前置きの判断（間隔・件数）はそちら。
//! 2. 通ったものは、**待ち時間つきの依頼**を読み取りワーカーへ送る（[`spawn_indicator_glyph_worker`]）。字形は実状態に
//!    0.2〜0.7 秒遅れて追随する（ADR-261 の実測）ので、ワーカー側で待ってから取り込む（メインスレッドは止まらない）。
//! 3. ワーカーは専用スレッド（STA）で UI Automation を使い、タスクバーの入力インジケーターの要素を探して矩形を得る。
//!    **名前が接頭辞に当たる要素が複数あるときの選び方は純粋関数**（`state::indicator_glyph::select_indicator`。実機の発見:
//!    「あ」のボタンは名前が 1 行、IME 自身のアイコンは複数行）。DPI 対応のプロセスなので矩形は物理ピクセル。
//! 4. 矩形を GDI で取り込み（`BitBlt`）、グレースケールにして純粋な前処理（`state::indicator_glyph::preprocess`）にかけ、
//!    結果（大きさ・前景の点数・所要・失敗の理由だけ。**画素は返さない・保存しない**）を `WM_INDICATOR_GLYPH_READ` で返す。
//!
//! 取り込む前に「矩形の中心が `Shell_TrayWnd` 系の窓の上か」は確かめない: 実機（Windows 11・XAML のタスクバー）では
//! `WindowFromPoint` がタスクバーではなくデスクトップを返した（ADR-261 追記）。覆われている場合は一様・別の画像になるので、
//! 前処理の `EmptyGlyph`・段階 1 後半の表への照合の `Miss` に倒れる。

use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, SetWinEventHook, TreeScope_Descendants, UnhookWinEvent,
    HWINEVENTHOOK,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetForegroundWindow, EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT,
};

use crate::state::indicator_glyph::{
    name_is_indicator, preprocess, select_indicator, GlyphFeature, GlyphGray, IndicatorCandidate,
    ProbeMiss, ProbeOutcome,
};

/// 読み取りワーカーへの依頼。
#[derive(Clone, Copy, Debug)]
pub struct GlyphReadRequest {
    /// 前面窓の変化から取り込むまで待つ時間（ミリ秒）
    pub delay_ms: u16,
}

/// `SetWinEventHook` の RAII ガード。Drop 時に `UnhookWinEvent` を呼ぶ。
pub struct GlyphHookGuard(HWINEVENTHOOK);

impl std::fmt::Debug for GlyphHookGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("GlyphHookGuard").field(&self.0 .0).finish()
    }
}

impl Drop for GlyphHookGuard {
    fn drop(&mut self) {
        // SAFETY: `self.0` は `SetWinEventHook` が返した有効なフックハンドル。`Drop` は一度しか呼ばれない。
        unsafe {
            let _ = UnhookWinEvent(self.0);
        }
        tracing::info!("[indicator-glyph] FOREGROUND hook uninstalled");
    }
}

/// `EVENT_SYSTEM_FOREGROUND` のコールバックの型（`SetWinEventHook` の `WINEVENTPROC`）。
///
/// コールバックの本体は `with_app` を呼ぶので、`observer/` ではなくオーケストレーター側（`app/bootstrap.rs`）に置く
/// （`layer_boundary_guard` の B-1: `observer/` 内で `with_app` を直接呼ばない）。
pub type ForegroundEventProc =
    unsafe extern "system" fn(HWINEVENTHOOK, u32, HWND, i32, i32, u32, u32);

/// `EVENT_SYSTEM_FOREGROUND` の WinEvent フックを登録する。失敗したら `None`（観測できないだけで、挙動は変わらない）。
#[must_use]
pub fn install_foreground_hook(proc: ForegroundEventProc) -> Option<GlyphHookGuard> {
    // SAFETY: `proc` は `'static` な extern "system" fn ポインタ（呼び出し側が渡す）。`WINEVENT_OUTOFCONTEXT` により
    //         コールバックはメッセージループスレッド（メインスレッド）で実行される。返されたフックは
    //         `GlyphHookGuard::drop` で `UnhookWinEvent` される。
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if hook.is_invalid() {
        tracing::warn!("[indicator-glyph] failed to install FOREGROUND hook");
        None
    } else {
        tracing::info!(
            "[indicator-glyph] FOREGROUND hook installed (ADR-261 stage 1, observe only)"
        );
        Some(GlyphHookGuard(hook))
    }
}

/// いまの前面窓の値（比較用。前面窓が無ければ 0）。
#[must_use]
pub fn foreground_window_value() -> isize {
    // SAFETY: 引数なしの読み取りだけ。
    unsafe { GetForegroundWindow() }.0 as isize
}

/// 経過ミリ秒を `u16` に飽和させる。
fn elapsed_ms_u16(since: Instant) -> u16 {
    u16::try_from(since.elapsed().as_millis()).unwrap_or(u16::MAX)
}

/// 画面の矩形 `(left, top, right, bottom)`（物理ピクセル）を GDI で取り込み、グレースケールにする。
///
/// # Safety
/// GDI の呼び出しだけ。ハンドルはこの関数の中で作って必ず解放する。
#[allow(clippy::many_single_char_names)]
unsafe fn capture_gray(rect: (i32, i32, i32, i32)) -> Option<GlyphGray> {
    let (l, t, r, b) = rect;
    let (w, h) = (r.checked_sub(l)?, b.checked_sub(t)?);
    if w <= 0 || h <= 0 || w > 1024 || h > 1024 {
        return None;
    }
    let screen = GetDC(None);
    if screen.is_invalid() {
        return None;
    }
    let mem = CreateCompatibleDC(Some(screen));
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).unwrap_or(0),
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let dib = CreateDIBSection(
        Some(mem),
        &raw const bmi,
        DIB_RGB_COLORS,
        &raw mut bits,
        None,
        0,
    );
    let gray = match dib {
        Ok(bmp) if !bits.is_null() => {
            let old = SelectObject(mem, bmp.into());
            let blit = BitBlt(mem, 0, 0, w, h, Some(screen), l, t, SRCCOPY);
            let out = if blit.is_ok() {
                let (uw, uh) = (usize::try_from(w).ok()?, usize::try_from(h).ok()?);
                // SAFETY: `bits` は `CreateDIBSection` が保証する、32 ビット × w × h の有効なバッファ。
                let px = std::slice::from_raw_parts(bits.cast::<u32>(), uw * uh);
                let g: Vec<u8> = px
                    .iter()
                    .map(|&p| {
                        // BGRA（リトルエンディアンの u32 では 0xAARRGGBB）。輝度 = (77R + 150G + 29B) / 256。
                        let (r8, g8, b8) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
                        ((77 * r8 + 150 * g8 + 29 * b8) >> 8) as u8
                    })
                    .collect();
                GlyphGray::new(uw, uh, g)
            } else {
                None
            };
            SelectObject(mem, old);
            let _ = DeleteObject(bmp.into());
            out
        }
        _ => None,
    };
    let _ = DeleteDC(mem);
    ReleaseDC(None, screen);
    gray
}

/// インジケーターの要素を探して矩形を返す（名前が接頭辞に当たる要素から、純粋関数で 1 つ選ぶ）。
///
/// # Safety
/// `automation` は有効な `IUIAutomation`（COM 初期化済みのスレッドで作ったもの）でなければならない。
unsafe fn find_indicator_rect(
    automation: &IUIAutomation,
) -> Result<(i32, i32, i32, i32), ProbeMiss> {
    let Ok(tray) = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) else {
        return Err(ProbeMiss::NoIndicator);
    };
    let (Ok(root), Ok(cond)) = (
        automation.ElementFromHandle(tray),
        automation.CreateTrueCondition(),
    ) else {
        return Err(ProbeMiss::NoIndicator);
    };
    let Ok(all) = root.FindAll(TreeScope_Descendants, &cond) else {
        return Err(ProbeMiss::NoIndicator);
    };
    let mut candidates: Vec<IndicatorCandidate> = Vec::new();
    for i in 0..all.Length().unwrap_or(0) {
        let Ok(el) = all.GetElement(i) else {
            continue;
        };
        let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
        if !name_is_indicator(&name) {
            continue;
        }
        let Ok(r) = el.CurrentBoundingRectangle() else {
            continue;
        };
        candidates.push(IndicatorCandidate {
            name,
            rect: (r.left, r.top, r.right, r.bottom),
        });
    }
    let chosen = select_indicator(&candidates).map_err(ProbeMiss::from)?;
    Ok(candidates[chosen].rect)
}

/// 1 回取り込んで前処理まで進める。返すのは結果と**前処理後の特徴**（20×20 の 2 値画像など。**元の画素は返さない**）。
///
/// # Safety
/// `automation` は有効な `IUIAutomation`（COM 初期化済みのスレッドで作ったもの）でなければならない。
unsafe fn probe_once(automation: &IUIAutomation) -> (ProbeOutcome, Option<GlyphFeature>) {
    let started = Instant::now();
    let miss = |m: ProbeMiss, w: u16, h: u16| {
        (
            ProbeOutcome {
                result: Err(m),
                width: w,
                height: h,
                dots: 0,
                read_ms: elapsed_ms_u16(started),
            },
            None,
        )
    };
    let rect = match find_indicator_rect(automation) {
        Ok(r) => r,
        Err(m) => return miss(m, 0, 0),
    };
    let Some(gray) = capture_gray(rect) else {
        return miss(ProbeMiss::CaptureFailed, 0, 0);
    };
    let (w, h) = (
        u16::try_from(gray.width()).unwrap_or(u16::MAX),
        u16::try_from(gray.height()).unwrap_or(u16::MAX),
    );
    // `preprocess` が返す失敗は、一様・前景が少ない画像だけ（表に関する `Miss` は分類側）。
    preprocess(&gray).map_or_else(
        |_| miss(ProbeMiss::EmptyGlyph, w, h),
        |f| {
            (
                ProbeOutcome {
                    result: Ok(()),
                    width: w,
                    height: h,
                    dots: u16::try_from(f.dots()).unwrap_or(u16::MAX),
                    read_ms: elapsed_ms_u16(started),
                },
                Some(f),
            )
        },
    )
}

/// 読み取りワーカーを起動する。専用スレッドで COM（STA）と `IUIAutomation` を持ち、依頼を 1 件ずつ処理して
/// 前処理後の特徴を別のチャネル（戻り値の `Receiver`）へ送り、`WM_INDICATOR_GLYPH_READ`（`wParam`/`lParam` = [`ProbeOutcome::pack`]）
/// をメインスレッドへ返す。依頼は届いた順に処理するので、
/// 結果も依頼の順に返る（呼び出し側はその順で文脈を対応づける）。
///
/// 戻り値の `WorkerThread` をアプリ終了まで保持すること（drop 時に停止・join される）。
#[must_use]
pub fn spawn_indicator_glyph_worker() -> (
    win32_worker::WorkerThread,
    mpsc::Sender<GlyphReadRequest>,
    mpsc::Receiver<Option<GlyphFeature>>,
) {
    let (tx, rx) = mpsc::channel::<GlyphReadRequest>();
    let (tx_feature, rx_feature) = mpsc::channel::<Option<GlyphFeature>>();
    let worker = win32_worker::WorkerThread::spawn("indicator-glyph-worker", move |token| {
        // SAFETY: CoInitializeEx はスレッド開始直後に一度だけ呼ぶ。STA を初期化し、同スレッド内の COM 操作を安全にする。
        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            if hr.is_err() {
                tracing::warn!("[indicator-glyph] CoInitializeEx failed: {hr:?}");
            }
        }
        // SAFETY: CoInitializeEx のあとに呼ぶ。失敗時は ok() が None を返す。
        let automation: Option<IUIAutomation> =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() };
        let Some(automation) = automation else {
            tracing::warn!("[indicator-glyph] failed to create IUIAutomation; shadow disabled");
            return;
        };
        tracing::info!("[indicator-glyph] worker thread started");

        loop {
            // recv_timeout でシャットダウン通知も定期的に確認する
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(req) => {
                    // 待ち時間は 50ms ずつ刻んで、シャットダウンに応じられるようにする。
                    let wake = Instant::now() + Duration::from_millis(u64::from(req.delay_ms));
                    while Instant::now() < wake && !token.is_shutdown() {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    if token.is_shutdown() {
                        break;
                    }
                    // SAFETY: `automation` は上で作った有効な COM オブジェクトで、同じスレッドから呼ぶ。
                    let (outcome, feature) = unsafe { probe_once(&automation) };
                    // 特徴は結果のメッセージより先に送る（メインスレッドは結果を受けたら特徴を 1 つ取り出す。依頼の順に対応する）。
                    let _ = tx_feature.send(feature);
                    let (wp, lp) = outcome.pack();
                    crate::win32::post_to_main_thread_with(crate::WM_INDICATOR_GLYPH_READ, wp, lp);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if token.is_shutdown() {
                        tracing::info!(
                            "[indicator-glyph] worker: shutdown signal received, exiting"
                        );
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::info!("[indicator-glyph] worker: channel closed, exiting");
                    break;
                }
            }
        }
    });
    (worker, tx, rx_feature)
}
