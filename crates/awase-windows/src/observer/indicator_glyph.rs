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
//! 4. 矩形を取り込み（ADR-261 決定8: 既定は `Shell_TrayWnd` 自身に描かせる `PrintWindow`、失敗したら画面 DC からの
//!    `BitBlt` へフォールバック。[`capture_bgra`] 参照）、グレースケールにして純粋な前処理（`state::indicator_glyph::preprocess`）
//!    にかけ、結果（大きさ・前景の点数・所要・失敗の理由だけ。**画素は返さない・保存しない**）を `WM_INDICATOR_GLYPH_READ` で返す。
//!
//! 取り込む前に「矩形の中心が `Shell_TrayWnd` 系の窓の上か」は確かめない: 実機（Windows 11・XAML のタスクバー）では
//! `WindowFromPoint` がタスクバーではなくデスクトップを返した（ADR-261 追記）。`BitBlt` フォールバック時、覆われている
//! 場合は一様・別の画像になるので、前処理の `EmptyGlyph`・段階 1 後半の表への照合の `Miss` に倒れる。
//! `PrintWindow` 経路では、覆われていても `Shell_TrayWnd` 自身の内容を返す可能性があるため、この「覆われたら Miss」
//! という前提が成り立たない場合がある。全画面アプリの手前での DWM 側の描画の鮮度（古い内容のまま返るか）は未測定
//! （ADR-261 決定8 の既知の限界）。
//!
//! **ADR-283 §10(2026-10-29・10-30): T2〜T4 の共通トリガーとして、インジケーター要素自身の `Name` プロパティの
//! 変化を UIA push（`IUIAutomation::AddPropertyChangedEventHandler`）で受ける**（ADR-261 決定5追補3の実測を本番へ
//! 配線。GJI で 6/6 検知・遅延 56〜80ms、`SetWinEventHook` 系は HWND を持たない UIA 論理要素には届かず不採用）。
//! 読み取りワーカーのスレッド（[`spawn_indicator_glyph_worker`]）自身が、起動時に 1 度だけ
//! [`find_indicator_element`] で要素を見つけて登録する（UIA のイベント配送は呼び出し元スレッドのメッセージポンプを
//! 要求しない。実測〔CI run 40632943535〕で、登録元スレッドが `SendInput` の間隔待ちで休止中でも通知が届くことを確認
//! 済み）。登録に失敗しても T1 の動作には影響しない（ログだけ、フェイルセーフ）。通知は `WM_INDICATOR_GLYPH_PUSH_NOTIFY`
//! でメインスレッドへ渡し、実際の字形の読み取りは [`Runtime::indicator_glyph_on_push_notify`]（`runtime/indicator_glyph_shadow.rs`）
//! が既存の取り込み依頼（[`GlyphReadRequest`]）として送る。`new_value` 自体は状態の内容を運ばない（固定文字列）ため、
//! 「字形を今すぐ再読みしてよい」という契機のトリガーとしてだけ使う。

use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationPropertyChangedEventHandler,
    SetWinEventHook, TreeScope_Descendants, TreeScope_Element, UIA_NamePropertyId, UnhookWinEvent,
    HWINEVENTHOOK,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetForegroundWindow, GetWindowRect, EVENT_SYSTEM_FOREGROUND, PW_RENDERFULLCONTENT,
    WINEVENT_OUTOFCONTEXT,
};

use crate::state::indicator_glyph::{
    name_is_indicator, preprocess, select_indicator, GlyphFeature, GlyphGray, IndicatorCandidate,
    ProbeMiss, ProbeOutcome,
};

/// [`push_sink::GlyphPushSink`]（`#[implement(...)]` 生成コードが pedantic/nursery lint に触れるため、
/// `examples/compartment_notify_probe.rs` と同じく専用モジュールへまとめて `allow` する）。
#[allow(clippy::ref_as_ptr, clippy::inline_always)]
mod push_sink {
    use windows::core::implement;
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Accessibility::{
        IUIAutomationElement, IUIAutomationPropertyChangedEventHandler_Impl, UIA_PROPERTY_ID,
    };

    /// UIA push 通知の受け口。中身は運ばず、`WM_INDICATOR_GLYPH_PUSH_NOTIFY` をメインスレッドへ送るだけ
    /// （ADR-283 §10: `new_value` は固定文字列で状態を運ばないため読まない）。
    #[implement(windows::Win32::UI::Accessibility::IUIAutomationPropertyChangedEventHandler)]
    pub(super) struct GlyphPushSink;

    impl IUIAutomationPropertyChangedEventHandler_Impl for GlyphPushSink_Impl {
        fn HandlePropertyChangedEvent(
            &self,
            _sender: windows_core::Ref<IUIAutomationElement>,
            _propertyid: UIA_PROPERTY_ID,
            _newvalue: &VARIANT,
        ) -> windows_core::Result<()> {
            crate::win32::post_to_main_thread_with(crate::WM_INDICATOR_GLYPH_PUSH_NOTIFY, 0, 0);
            Ok(())
        }
    }
}
use push_sink::GlyphPushSink;

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

/// 画面の矩形 `(left, top, right, bottom)`（物理ピクセル）を取り込み、上位から `0xAARRGGBB` の画素列として返す
/// （幅・高さ・画素。`max_w`/`max_h` を超える矩形は取り込まない）。
///
/// ADR-261 決定8: まず [`capture_via_print_window`]（`Shell_TrayWnd` 自身に `PrintWindow` で描かせる、
/// 画面キャプチャではない経路）を試す。実測（ベースライン・ダークテーマ・IME状態の切り替え前後の3軸、
/// GJI/MS-IME 双方）では `BitBlt`(画面キャプチャ) と画素完全一致したが、未測定の環境（DPI 違い・覆われた
/// 状態・RDP・将来の Windows バージョン等）では `PrintWindow` が **`TRUE` を返しながら空・古い画素を返す**
/// 恐れがある（Opus レビューで指摘、2026-10-28）ため、`capture_via_print_window` 側で RGB 全ゼロを検知して
/// `None` に倒す。それでも検知できない「古い内容のまま」という失敗は未解決（既知の限界、ADR-261 決定8）。
/// いずれかで `None` が返ったら、既存の画面キャプチャ（[`capture_bgra_screen`]）へフォールバックする。
///
/// # Safety
/// GDI・UI Automation 以外の Win32 API（`FindWindowW`/`GetWindowRect`/`PrintWindow`）も呼ぶ。ハンドルは
/// この関数の中で作って必ず解放する。
#[allow(clippy::many_single_char_names)]
unsafe fn capture_bgra(
    rect: (i32, i32, i32, i32),
    max_w: i32,
    max_h: i32,
) -> Option<(usize, usize, Vec<u32>)> {
    let (l, t, r, b) = rect;
    let (w, h) = (r.checked_sub(l)?, b.checked_sub(t)?);
    if w <= 0 || h <= 0 || w > max_w || h > max_h {
        return None;
    }
    if let Some(out) = capture_via_print_window(rect) {
        tracing::debug!("[indicator-glyph] capture_bgra: via PrintWindow");
        return Some(out);
    }
    // 既定の実行時ログ(info、`app/bootstrap.rs`)でも残す: フォールバックは「何かが期待通りでなかった」
    // 兆候であり、不具合報告(ADR-148型 allowlist)からこの切り替わりを追跡できるようにする(Opus レビュー
    // round2 指摘A、2026-10-28)。`debug!` だと既定フィルタで落ちて観測できない。
    tracing::info!(
        "[indicator-glyph] capture_bgra: PrintWindow 失敗・フォールバック(BitBlt)へ切り替え"
    );
    capture_bgra_screen(rect, w, h)
}

/// `Shell_TrayWnd` を `PrintWindow(PW_RENDERFULLCONTENT)` で描かせ、`crop`（画面座標、物理ピクセル）を
/// 切り出す。窓が見つからない・`PrintWindow` が失敗した・切り出した結果の RGB が全ゼロ（`PrintWindow` が
/// `TRUE` を返しながら実際には何も描かなかった、DWM/DComp 側の既知の失敗モード）なら `None`（呼び出し側は
/// 画面キャプチャへフォールバックする、ADR-261 決定8）。
///
/// # Safety
/// GDI の呼び出しだけ。ハンドルはこの関数の中で作って必ず解放する。
#[allow(clippy::many_single_char_names)]
unsafe fn capture_via_print_window(crop: (i32, i32, i32, i32)) -> Option<(usize, usize, Vec<u32>)> {
    let tray = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok()?;
    let mut tray_rect = RECT::default();
    GetWindowRect(tray, &raw mut tray_rect).ok()?;
    let (fl, ft, fr, fb) = (
        tray_rect.left,
        tray_rect.top,
        tray_rect.right,
        tray_rect.bottom,
    );
    let (fw, fh) = (fr.checked_sub(fl)?, fb.checked_sub(ft)?);
    // 2026-10-28 の実測(CI、1024×48 の解像度 1 通り)では Shell_TrayWnd はこの範囲に収まった。
    // 8192×2048 はマルチモニタ・縦置きタスクバーも含めた想定上限であり、実測した上限ではない。
    // これを超えたら(異常系・未対応の構成)取り込まず、画面キャプチャへフォールバックする。
    if fw <= 0 || fh <= 0 || fw > 8192 || fh > 2048 {
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
            biWidth: fw,
            biHeight: -fh, // top-down
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
    let out = match dib {
        Ok(bmp) if !bits.is_null() => {
            let old = SelectObject(mem, bmp.into());
            let ok = PrintWindow(tray, mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
            let out = if ok {
                let dims = (usize::try_from(fw).ok(), usize::try_from(fh).ok());
                if let (Some(uw), Some(uh)) = dims {
                    // SAFETY: `bits` は `CreateDIBSection` が保証する、32 ビット × fw × fh の有効なバッファ。
                    let px = std::slice::from_raw_parts(bits.cast::<u32>(), uw * uh);
                    crop_from(px, uw, uh, (fl, ft), crop).filter(|(_, _, c)| !all_rgb_zero(c))
                } else {
                    None
                }
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
    out
}

/// 画素列の RGB（下位24ビット、alpha は見ない）が全てゼロ(真っ黒)か。`PrintWindow` が `TRUE` を返しながら
/// 実際には何も描かなかった場合の検知用(ADR-261 決定8、Opus レビューで指摘)。
fn all_rgb_zero(px: &[u32]) -> bool {
    !px.is_empty() && px.iter().all(|&p| p.trailing_zeros() >= 24)
}

/// `full`(原点 `origin` からの幅 `fw`・高さ `fh` の画素列)から、画面座標の `crop` 矩形を切り出す。
/// `crop` が `full` の外に出る部分(上下左右いずれも)は位置をずらさずに `0`(透明黒)で埋める。`crop` と
/// `full` の交差が空(矩形が完全に外)なら `None` を返す。
fn crop_from(
    full: &[u32],
    fw: usize,
    fh: usize,
    origin: (i32, i32),
    crop: (i32, i32, i32, i32),
) -> Option<(usize, usize, Vec<u32>)> {
    let (ox, oy) = origin;
    let (cl, ct, cr, cb) = crop;
    let (cw, ch) = (
        usize::try_from((cr - cl).max(0)).unwrap_or(0),
        usize::try_from((cb - ct).max(0)).unwrap_or(0),
    );
    if cw == 0 || ch == 0 {
        return None;
    }
    let bound_w = i32::try_from(fw).unwrap_or(i32::MAX);
    let bound_h = i32::try_from(fh).unwrap_or(i32::MAX);
    // crop(画面座標)と full(origin 起点、0..bound_w × 0..bound_h)の交差が空なら、切り出す画素が1つも無い
    // (タスクバーの矩形がインジケーター取得後に動いた等)ので None(フォールバックへ)。
    if cr <= ox || cl >= ox.saturating_add(bound_w) || cb <= oy || ct >= oy.saturating_add(bound_h)
    {
        return None;
    }
    let mut cropped = vec![0u32; cw * ch];
    for y in 0..ch {
        let Some(sy) = (ct - oy).checked_add(i32::try_from(y).unwrap_or(0)) else {
            continue;
        };
        if sy < 0 || sy >= bound_h {
            continue;
        }
        let usy = usize::try_from(sy).unwrap_or(0);
        for x in 0..cw {
            let Some(sx) = (cl - ox).checked_add(i32::try_from(x).unwrap_or(0)) else {
                continue;
            };
            if sx < 0 || sx >= bound_w {
                continue;
            }
            cropped[y * cw + x] = full[usy * fw + usize::try_from(sx).unwrap_or(0)];
        }
    }
    Some((cw, ch, cropped))
}

/// 画面の矩形 `(left, top, right, bottom)`（物理ピクセル）を `BitBlt` で画面 DC から直接取り込む
/// ([`capture_via_print_window`] のフォールバック)。
///
/// # Safety
/// GDI の呼び出しだけ。ハンドルはこの関数の中で作って必ず解放する。
#[allow(clippy::many_single_char_names)]
unsafe fn capture_bgra_screen(
    rect: (i32, i32, i32, i32),
    w: i32,
    h: i32,
) -> Option<(usize, usize, Vec<u32>)> {
    let (l, t, ..) = rect;
    if w <= 0 || h <= 0 {
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
    let out = match dib {
        Ok(bmp) if !bits.is_null() => {
            let old = SelectObject(mem, bmp.into());
            let blit = BitBlt(mem, 0, 0, w, h, Some(screen), l, t, SRCCOPY);
            let out = if blit.is_ok() {
                let (uw, uh) = (usize::try_from(w).ok()?, usize::try_from(h).ok()?);
                // SAFETY: `bits` は `CreateDIBSection` が保証する、32 ビット × w × h の有効なバッファ。
                let px = std::slice::from_raw_parts(bits.cast::<u32>(), uw * uh);
                Some((uw, uh, px.to_vec()))
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
    out
}

/// 画面の矩形 `(left, top, right, bottom)`（物理ピクセル）を GDI で取り込み、グレースケールにする。
///
/// # Safety
/// `capture_bgra` と同じ。
unsafe fn capture_gray(rect: (i32, i32, i32, i32)) -> Option<GlyphGray> {
    let (w, h, px) = capture_bgra(rect, 1024, 1024)?;
    let g: Vec<u8> = px
        .iter()
        .map(|&p| {
            // BGRA（リトルエンディアンの u32 では 0xAARRGGBB）。輝度 = (77R + 150G + 29B) / 256。
            let (r8, g8, b8) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            ((77 * r8 + 150 * g8 + 29 * b8) >> 8) as u8
        })
        .collect();
    GlyphGray::new(w, h, g)
}

/// 実験: 取り込んだ矩形の周辺を画像で残す（`AWASE_GLYPH_DUMP_DIR` を指定したときだけ。既定オフ）。
///
/// 残すのは**タスクバーの帯**（`Shell_TrayWnd` の窓矩形）と**切り抜き**（インジケーターの矩形）の 2 枚の BMP だけで、
/// 画面全体や他の窓は撮らない（文書・チャットの本文を写さないため）。目的は、切り抜きの位置がずれて見本に合わない
/// （`miss-shape`）原因を、帯の画像と切り抜きの座標から確かめること。古いファイルは `DUMP_KEEP_FILES` 件を超えたら消す。
struct DumpSink {
    dir: std::path::PathBuf,
    seq: u32,
}

/// 帯の最大の大きさ（物理ピクセル）。
const DUMP_STRIP_MAX_W: i32 = 8192;
const DUMP_STRIP_MAX_H: i32 = 512;
/// 残すファイル数の上限（帯 + 切り抜きで 2 件ずつ）。
const DUMP_KEEP_FILES: usize = 80;

impl DumpSink {
    fn from_env() -> Option<Self> {
        let dir = std::path::PathBuf::from(std::env::var_os("AWASE_GLYPH_DUMP_DIR")?);
        std::fs::create_dir_all(&dir).ok()?;
        tracing::info!(
            "[indicator-glyph] debug dump enabled: {} (taskbar strip + crop only)",
            dir.display()
        );
        Some(Self { dir, seq: 0 })
    }

    /// 24 ビットの BMP（上から下）として書く。
    fn write_bmp(path: &std::path::Path, w: usize, h: usize, px: &[u32]) -> std::io::Result<()> {
        let row = (w * 3).div_ceil(4) * 4;
        let size = 54 + row * h;
        let mut buf = Vec::with_capacity(size);
        buf.extend_from_slice(b"BM");
        buf.extend_from_slice(&u32::try_from(size).unwrap_or(0).to_le_bytes());
        buf.extend_from_slice(&[0; 4]);
        buf.extend_from_slice(&54u32.to_le_bytes());
        buf.extend_from_slice(&40u32.to_le_bytes());
        buf.extend_from_slice(&i32::try_from(w).unwrap_or(0).to_le_bytes());
        buf.extend_from_slice(&(-i32::try_from(h).unwrap_or(0)).to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&24u16.to_le_bytes());
        buf.extend_from_slice(&[0; 24]);
        for y in 0..h {
            for x in 0..w {
                let p = px[y * w + x];
                buf.extend_from_slice(&[
                    (p & 0xFF) as u8,
                    ((p >> 8) & 0xFF) as u8,
                    ((p >> 16) & 0xFF) as u8,
                ]);
            }
            buf.extend(std::iter::repeat_n(0u8, row - w * 3));
        }
        std::fs::write(path, buf)
    }

    /// 帯と切り抜きを書き、座標をログに残す。
    ///
    /// # Safety
    /// `capture_bgra` と同じ。
    unsafe fn dump(&mut self, crop: (i32, i32, i32, i32)) {
        let mut tray = RECT::default();
        let strip_rect = match FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) {
            Ok(h) if GetWindowRect(h, &raw mut tray).is_ok() => {
                (tray.left, tray.top, tray.right, tray.bottom)
            }
            _ => return,
        };
        let Some((sw, sh, strip)) = capture_bgra(strip_rect, DUMP_STRIP_MAX_W, DUMP_STRIP_MAX_H)
        else {
            return;
        };
        let Some((cw, ch, cropped)) = capture_bgra(crop, 1024, 1024) else {
            return;
        };
        self.seq += 1;
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let stem = format!("glyph-{ms}-{:03}", self.seq);
        let _ = Self::write_bmp(&self.dir.join(format!("{stem}-strip.bmp")), sw, sh, &strip);
        let _ = Self::write_bmp(&self.dir.join(format!("{stem}-crop.bmp")), cw, ch, &cropped);
        tracing::info!(
            "[indicator-glyph] dump {stem} strip_rect={strip_rect:?} crop_rect={crop:?} crop_in_strip=({},{},{},{}) strip={sw}x{sh} crop={cw}x{ch}",
            crop.0 - strip_rect.0,
            crop.1 - strip_rect.1,
            cw,
            ch,
        );
        self.prune();
    }

    /// `glyph-*.bmp` を新しい順に `DUMP_KEEP_FILES` 件だけ残す。
    fn prune(&self) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut names: Vec<_> = rd
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .filter(|n| {
                let n = n.to_string_lossy();
                n.starts_with("glyph-") && n.ends_with(".bmp")
            })
            .collect();
        names.sort();
        let excess = names.len().saturating_sub(DUMP_KEEP_FILES);
        for n in names.into_iter().take(excess) {
            let _ = std::fs::remove_file(self.dir.join(n));
        }
    }
}

/// インジケーターの候補の要素（名前が接頭辞に当たるもの）を、矩形つきで列挙する。[`find_indicator_rect`]・
/// [`find_indicator_element`] の共有部分（名前が接頭辞に当たる要素が複数あるときの選び方自体は純粋関数
/// `select_indicator`、ここでは COM の走査だけを行う）。
///
/// # Safety
/// `automation` は有効な `IUIAutomation`（COM 初期化済みのスレッドで作ったもの）でなければならない。
unsafe fn collect_indicator_candidates(
    automation: &IUIAutomation,
) -> Result<(Vec<IndicatorCandidate>, Vec<IUIAutomationElement>), ProbeMiss> {
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
    let mut elements: Vec<IUIAutomationElement> = Vec::new();
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
        elements.push(el);
    }
    Ok((candidates, elements))
}

/// インジケーターの要素を探して矩形を返す（名前が接頭辞に当たる要素から、純粋関数で 1 つ選ぶ）。
///
/// # Safety
/// `automation` は有効な `IUIAutomation`（COM 初期化済みのスレッドで作ったもの）でなければならない。
unsafe fn find_indicator_rect(
    automation: &IUIAutomation,
) -> Result<(i32, i32, i32, i32), ProbeMiss> {
    let (candidates, _elements) = collect_indicator_candidates(automation)?;
    let chosen = select_indicator(&candidates).map_err(ProbeMiss::from)?;
    Ok(candidates[chosen].rect)
}

/// インジケーターの要素そのもの（push 通知の登録対象）を探す。選び方は [`find_indicator_rect`] と同じ
/// （ADR-283 §10: T2〜T4 共通トリガーの登録先。起動時に 1 度だけ呼ぶ）。
///
/// # Safety
/// `automation` は有効な `IUIAutomation`（COM 初期化済みのスレッドで作ったもの）でなければならない。
unsafe fn find_indicator_element(
    automation: &IUIAutomation,
) -> Result<IUIAutomationElement, ProbeMiss> {
    let (candidates, elements) = collect_indicator_candidates(automation)?;
    let chosen = select_indicator(&candidates).map_err(ProbeMiss::from)?;
    Ok(elements[chosen].clone())
}

/// 1 回取り込んで前処理まで進める。返すのは結果と**前処理後の特徴**（20×20 の 2 値画像など。**元の画素は返さない**）。
///
/// # Safety
/// `automation` は有効な `IUIAutomation`（COM 初期化済みのスレッドで作ったもの）でなければならない。
unsafe fn probe_once(
    automation: &IUIAutomation,
    dump: Option<&mut DumpSink>,
) -> (ProbeOutcome, Option<GlyphFeature>) {
    let started = Instant::now();
    let miss = |m: ProbeMiss, w: u16, h: u16| {
        (
            ProbeOutcome {
                result: Err(m),
                width: w,
                height: h,
                dots: 0,
                read_ms: elapsed_ms_u16(started),
                dark: false,
            },
            None,
        )
    };
    let rect = match find_indicator_rect(automation) {
        Ok(r) => r,
        Err(m) => return miss(m, 0, 0),
    };
    if let Some(d) = dump {
        d.dump(rect);
    }
    let Some(gray) = capture_gray(rect) else {
        return miss(ProbeMiss::CaptureFailed, 0, 0);
    };
    let (w, h) = (
        u16::try_from(gray.width()).unwrap_or(u16::MAX),
        u16::try_from(gray.height()).unwrap_or(u16::MAX),
    );
    let dark = gray.background_is_dark();
    // `preprocess` が返す失敗は、一様・前景が少ない画像だけ（表に関する `Miss` は分類側）。
    preprocess(&gray).map_or_else(
        |_| {
            let (mut o, f) = miss(ProbeMiss::EmptyGlyph, w, h);
            o.dark = dark;
            (o, f)
        },
        |f| {
            (
                ProbeOutcome {
                    result: Ok(()),
                    width: w,
                    height: h,
                    dots: u16::try_from(f.dots()).unwrap_or(u16::MAX),
                    read_ms: elapsed_ms_u16(started),
                    dark,
                },
                Some(f),
            )
        },
    )
}

/// 読み取りワーカーを起動する。
///
/// 専用スレッドで COM（STA）と `IUIAutomation` を持ち、依頼を 1 件ずつ処理して、前処理後の特徴を別のチャネル（戻り値の
/// `Receiver`）へ送り、`WM_INDICATOR_GLYPH_READ`（`wParam`/`lParam` = [`ProbeOutcome::pack`]）をメインスレッドへ返す。
/// 依頼は届いた順に処理するので、結果も依頼の順に返る（呼び出し側はその順で文脈を対応づける）。
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
        let mut dump = DumpSink::from_env();

        // ADR-283 §10: T2〜T4 共通の push トリガー。起動時に 1 度だけ登録する。登録に失敗しても
        // T1（前面窓の変化）には影響しない（フェイルセーフ、ログだけ）。UIA のイベント配送はこのスレッドの
        // メッセージポンプを要求しないため、下の `rx.recv_timeout` のポーリングループのままでよい
        // （実測で、登録元スレッドが休止中でも通知が届くことを確認済み。モジュール doc 参照）。
        // `handler` はここで作ってループの終わりまで（この関数のスコープの終わりまで）生かし続ける
        // （COM 側の参照保持に依存せず、Rust 側のドロップ順でも登録が切れないようにする）。
        let push_registered = match unsafe { find_indicator_element(&automation) } {
            Ok(element) => {
                let handler: IUIAutomationPropertyChangedEventHandler = GlyphPushSink.into();
                // SAFETY: `automation`・`element`・`handler` はいずれもこのスレッドで作った有効な COM オブジェクト。
                let registered = unsafe {
                    automation.AddPropertyChangedEventHandlerNativeArray(
                        &element,
                        TreeScope_Element,
                        None,
                        &handler,
                        &[UIA_NamePropertyId],
                    )
                };
                match registered {
                    Ok(()) => {
                        tracing::info!(
                            "[indicator-glyph] UIA push notify registered (ADR-283 §10, T2-T4 trigger)"
                        );
                        Some(handler)
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[indicator-glyph] AddPropertyChangedEventHandlerNativeArray failed: {e:?}"
                        );
                        None
                    }
                }
            }
            Err(e) => {
                tracing::warn!("[indicator-glyph] push notify: indicator element not found: {e:?}");
                None
            }
        };

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
                    let (outcome, feature) = unsafe { probe_once(&automation, dump.as_mut()) };
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
        if push_registered.is_some() {
            // SAFETY: `automation` はこのスレッドで作った有効な COM オブジェクトで、まだ生きている。
            let _ = unsafe { automation.RemoveAllEventHandlers() };
        }
    });
    (worker, tx, rx_feature)
}

#[cfg(test)]
mod tests {
    use super::{all_rgb_zero, crop_from};

    /// `fw`×`fh` の盤目(値 = y*fw+x+1、0 は使わない)を作る。
    fn checker(fw: usize, fh: usize) -> Vec<u32> {
        (0..fw * fh)
            .map(|i| u32::try_from(i + 1).unwrap())
            .collect()
    }

    #[test]
    fn crop_fully_inside_matches_source() {
        let full = checker(10, 10);
        let (w, h, px) = crop_from(&full, 10, 10, (0, 0), (2, 3, 5, 6)).expect("inside crop");
        assert_eq!((w, h), (3, 3));
        // origin=(0,0) なので画面座標 = full の添字そのもの。
        assert_eq!(px[0], full[3 * 10 + 2]);
        assert_eq!(px[w * h - 1], full[5 * 10 + 4]);
    }

    #[test]
    fn crop_overflowing_top_left_does_not_shift_content() {
        // full は原点 (100, 100) から 10x10。crop は左へ2・上へ3はみ出す。
        let full = checker(10, 10);
        let (w, h, px) =
            crop_from(&full, 10, 10, (100, 100), (98, 97, 104, 103)).expect("partial overlap");
        assert_eq!((w, h), (6, 6));
        // はみ出した左2列・上3行は0で埋まり、ずれて中身が詰まってはいけない。
        assert_eq!(px[0], 0); // (0,0): full の (-2,-3) 相当 → 範囲外
        assert_eq!(px[1], 0); // (1,0): full の (-1,-3) 相当 → 範囲外
        assert_eq!(px[2 + 3 * w], full[0]); // (2,3): full の (0,0) 相当 → 1
    }

    #[test]
    fn crop_fully_outside_returns_none() {
        let full = checker(10, 10);
        assert!(crop_from(&full, 10, 10, (0, 0), (100, 100, 110, 110)).is_none());
        assert!(crop_from(&full, 10, 10, (0, 0), (-50, -50, -40, -40)).is_none());
    }

    #[test]
    fn crop_zero_size_returns_none() {
        let full = checker(10, 10);
        assert!(crop_from(&full, 10, 10, (0, 0), (2, 2, 2, 5)).is_none());
    }

    #[test]
    fn all_rgb_zero_ignores_alpha() {
        assert!(all_rgb_zero(&[0xFF00_0000, 0x0000_0000]));
        assert!(!all_rgb_zero(&[0x0000_0001]));
        assert!(!all_rgb_zero(&[]));
    }
}
