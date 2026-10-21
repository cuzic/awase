#![allow(unsafe_code)]
// Win32 の共有メモリ・プロセストークン・ntdll を呼ぶため unsafe が必須(lib.rsのクレート全体allowから個別移管、indicator_menu.rs と同じ作法)
//! ADR-265 段階 1: mozc-readable の TIP が書き出す共有メモリ（48 バイト、version 2）を**読むだけ**の観測。
//!
//! 仕様の出典は mozc-readable の `readable/docs/shared-memory.md`（awase 側に写しは無い）。読み取りの規約（ADR-265 D3）:
//!
//! - **読むたびに Open → Map → 読む → Unmap → Close**（ハンドルを持ち続けない。持ち続けると書き出し側のプロセスが終わった後も古いブロックが残る）。
//! - seqlock: `seq1` を読み、奇数なら捨て、中身を読み、`seq2` を読み、`seq1 == seq2` のときだけ採る（`fence(Acquire)`）。
//! - 受け入れ条件（`magic`・`version`・`seq`・`process_id`・`active`）の判定は、OS を呼ばない `state::mozc_shm::classify`。
//! - 通常のプロセスは `Local\mozc-readable-ime-<tid>`、AppContainer のプロセス（UWP など）は専用の名前空間
//!   `\Sessions\<n>\AppContainerNamedObjects\<SID>\mozc-readable-ime-<tid>` を `NtOpenSection` で開く
//!   （CI の実測: mozc-readable の run 39097062197 で NTSTATUS 0）。
//!
//! 何も書かない・belief に入れない・IME へ何もしない。呼び出し元は `runtime/mozc_shm_shadow.rs`。

use std::ffi::c_void;
use std::sync::atomic::{fence, AtomicU32, Ordering};
use std::time::Instant;

use windows::core::{s, w, HSTRING};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{
    GetTokenInformation, TokenAppContainerSid, TokenIsAppContainer, TokenSessionId, TOKEN_QUERY,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ,
};
use windows::Win32::System::Threading::{
    OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

use crate::state::mozc_shm::{
    app_container_section_path, classify, format_sid, local_mapping_name, ComposeBlock,
    ComposeReading, CompositionFields, FocusTicks, RawBlock, ReadFailure, Reading, Route,
    BLOCK_SIZE, V3_BLOCK_SIZE, V5_BLOCK_SIZE,
};

/// seqlock の再試行の上限（書き込みは数命令なので、実際には 1〜2 回で終わる）。
const SEQLOCK_ATTEMPTS: usize = 8;

/// `STATUS_OBJECT_NAME_NOT_FOUND`
const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xC000_0034;
/// `STATUS_OBJECT_PATH_NOT_FOUND`
const STATUS_OBJECT_PATH_NOT_FOUND: u32 = 0xC000_003A;
/// `SECTION_MAP_READ`
const SECTION_MAP_READ: u32 = 0x0004;
/// `OBJ_CASE_INSENSITIVE`
const OBJ_CASE_INSENSITIVE: u32 = 0x0000_0040;

#[repr(C)]
#[allow(dead_code)] // FFI 用の構造体。フィールドは `NtOpenSection` が読む
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

#[repr(C)]
#[allow(dead_code)] // FFI 用の構造体。フィールドは `NtOpenSection` が読む
struct ObjectAttributes {
    length: u32,
    root_directory: HANDLE,
    object_name: *mut UnicodeString,
    attributes: u32,
    security_descriptor: *mut c_void,
    security_quality_of_service: *mut c_void,
}

type NtOpenSectionFn = unsafe extern "system" fn(*mut HANDLE, u32, *const ObjectAttributes) -> i32;

/// 対象プロセスが AppContainer かの判定の結果。
enum AcProbe {
    /// AppContainer ではない（通常のプロセス）
    NotAppContainer,
    /// AppContainer。名前空間のパスを作るための材料
    AppContainer { session_id: u32, sid: String },
    /// 判定できなかった（段階の名前）
    Unresolved(&'static str),
}

/// ウィンドウ（`hwnd` のアドレス）を作ったスレッドの ID。無効な hwnd や取得できないときは 0。
///
/// `runtime/` には `unsafe` を書けない（独自 lint）ので、Win32 の呼び出しはここに置く。
#[must_use]
pub fn thread_of_window(hwnd_addr: usize) -> u32 {
    // SAFETY: `GetWindowThreadProcessId` はどのスレッドからも呼べる非ブロッキング API。無効な hwnd は 0 を返す。
    unsafe { GetWindowThreadProcessId(HWND(hwnd_addr as *mut _), None) }
}

/// 現在のキーボードフォーカスのウィンドウ（`GetGUIThreadInfo().hwndFocus`、取れなければ前景窓）の
/// （hwnd のアドレス、それを作ったスレッド、プロセス）。
///
/// TSF の状態はフォーカスのスレッドごと（XAML のアプリでは、最上位のウィンドウのスレッドではない）。取れなければ `None`。
/// 内部で `get_focused_hwnd`（ワーカースレッドで実行して最大 30ms 待つ）を呼ぶので、フックのコールバックの中から呼ばない
/// （BUG-34）。呼び出し元は `TIMER_TSF_PROBE` のハンドラ（メインスレッド）。最初のローマ字の送信が最大 30ms 遅れうる。
#[must_use]
pub fn focus_thread_and_process() -> Option<(usize, u32, u32)> {
    // SAFETY: メインスレッドから呼ぶ（`get_focused_hwnd` の要件）。Win32 を呼ぶだけで、結果の hwnd は無効でもよい。
    let hwnd = unsafe { crate::ime::get_focused_hwnd() };
    if hwnd.0.is_null() {
        return None;
    }
    let hwnd_addr = hwnd.0 as usize;
    let (tid, pid) = thread_and_process_of_window(hwnd_addr);
    (tid != 0 && pid != 0).then_some((hwnd_addr, tid, pid))
}

/// ウィンドウ（`hwnd` のアドレス）を作ったスレッドの ID とプロセスの ID。取得できないときは `(0, 0)`。
#[must_use]
pub fn thread_and_process_of_window(hwnd_addr: usize) -> (u32, u32) {
    let mut pid = 0u32;
    // SAFETY: `GetWindowThreadProcessId` はどのスレッドからも呼べる非ブロッキング API。`pid` はスタック上の出力先。無効な hwnd は 0 を返す。
    let tid = unsafe { GetWindowThreadProcessId(HWND(hwnd_addr as *mut _), Some(&raw mut pid)) };
    (tid, pid)
}

/// 対象のスレッド（`thread_id`、プロセスは `process_id`）の共有メモリを 1 回読む。所要時間（マイクロ秒）も返す。
///
/// 通常のプロセスは `Local\` の名前で開く。開けなければ、対象が AppContainer のときだけ専用の名前空間のパスで開き直す。
/// どちらでも、ここは**ブロックを読むだけ**で、受け入れるかどうかは `state::mozc_shm::outcome_of` が決める。
#[must_use]
pub fn read(thread_id: u32, process_id: u32) -> (Reading, u32) {
    let started = Instant::now();
    let reading = read_inner(thread_id, process_id);
    let read_us = u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX);
    (reading, read_us)
}

/// 共有メモリのハンドルを開く（通常のプロセスは `Local\`、開けなければ AppContainer の名前空間）。
fn open_handle(thread_id: u32, process_id: u32) -> (Route, Result<HANDLE, ReadFailure>) {
    let name = HSTRING::from(local_mapping_name(thread_id));
    // SAFETY: `name` は NUL 終端の有効な文字列。失敗は `Err` で返る。
    if let Ok(handle) = unsafe { OpenFileMappingW(FILE_MAP_READ.0, false, &name) } {
        return (Route::Local, Ok(handle));
    }
    match probe_app_container(process_id) {
        AcProbe::NotAppContainer => (Route::Local, Err(ReadFailure::NotFound)),
        AcProbe::Unresolved(stage) => (Route::AppContainer, Err(ReadFailure::Unresolved(stage))),
        AcProbe::AppContainer { session_id, sid } => {
            let path = app_container_section_path(session_id, &sid, thread_id);
            (Route::AppContainer, open_section(&path))
        }
    }
}

fn read_inner(thread_id: u32, process_id: u32) -> Reading {
    let (route, opened) = open_handle(thread_id, process_id);
    Reading {
        route,
        block: opened.and_then(read_view),
    }
}

/// 入力中の文字の確認（ADR-270）のために、対象のスレッドの共有メモリを読む。
///
/// 先頭 48 バイトで受け入れ条件（`classify`）を確かめ、`version >= 3` のときだけ 80 バイトまで読み直して追記分
/// （入力の段階・`server_ready`・入力スコープの指定）を取る（ADR-267 D9: 先頭 48 バイトだけを map して `version` を
/// 読み、足りれば 80 で map し直す）。belief にも IME にも何も書かない。読めない・受け入れられない・version 2 のときは
/// `Unreadable`。
#[must_use]
pub fn read_compose(thread_id: u32, process_id: u32) -> ComposeReading {
    let (_route, opened) = open_handle(thread_id, process_id);
    let Ok(handle) = opened else {
        return ComposeReading::Unreadable;
    };
    let reading = read_compose_inner(handle, process_id);
    // SAFETY: `handle` は `open_handle` が返した有効なハンドルで、ここで 1 回だけ閉じる。
    let _ = unsafe { CloseHandle(handle) };
    reading
}

fn read_compose_inner(handle: HANDLE, process_id: u32) -> ComposeReading {
    // 1 回目（48 バイト）は、80 バイトを map してよいか（`version >= 3`）の判定だけに使う（ADR-267 D9）。
    let Ok(head) = read_bytes::<BLOCK_SIZE>(handle) else {
        return ComposeReading::Unreadable;
    };
    if !matches!(classify(&RawBlock::parse(&head), process_id), Ok(s) if s.version >= 3) {
        return ComposeReading::Unreadable;
    }
    let Ok(full) = read_bytes::<V3_BLOCK_SIZE>(handle) else {
        return ComposeReading::Unreadable;
    };
    // 値はすべて 2 回目（80 バイト、1 回の seqlock）から取る。1 回目と 2 回目を組み合わせると、別の `seq` の値が混ざる。
    let mut first = [0u8; BLOCK_SIZE];
    first.copy_from_slice(&full[..BLOCK_SIZE]);
    let raw = RawBlock::parse(&first);
    let Ok(state) = classify(&raw, process_id) else {
        return ComposeReading::Unreadable;
    };
    let fields = CompositionFields::parse(&full);
    ComposeReading::Block(ComposeBlock {
        active: true,
        effective_open: state.effective_open,
        effective_mode: raw.effective_mode,
        phase: fields.phase,
        scope_forced_mode: fields.scope_forced_mode,
        server_ready: fields.server_ready != 0,
        key_seq: fields.key_seq,
        composition_key_seq: fields.composition_key_seq,
    })
}

/// 対象のスレッドの共有メモリから、キーボードフォーカスの時刻（version 5、診断用）を読む。
///
/// 先頭 48 バイトで受け入れ条件（`classify`）を確かめ、`version >= 5` のときだけ 144 バイトまで読み直す。version 4 以下・
/// 読めない・受け入れられないときは `None`。belief にも IME にも何も書かない。
#[must_use]
pub fn read_focus_ticks(thread_id: u32, process_id: u32) -> Option<FocusTicks> {
    let (_route, opened) = open_handle(thread_id, process_id);
    let handle = opened.ok()?;
    let ticks = read_focus_ticks_inner(handle, process_id);
    // SAFETY: `handle` は `open_handle` が返した有効なハンドルで、ここで 1 回だけ閉じる。
    let _ = unsafe { CloseHandle(handle) };
    ticks
}

fn read_focus_ticks_inner(handle: HANDLE, process_id: u32) -> Option<FocusTicks> {
    let head = read_bytes::<BLOCK_SIZE>(handle).ok()?;
    let state = classify(&RawBlock::parse(&head), process_id).ok()?;
    if state.version < 5 {
        return None;
    }
    let full = read_bytes::<V5_BLOCK_SIZE>(handle).ok()?;
    // 値はすべて 2 回目（1 回の seqlock）から取る。受け入れ条件も 2 回目の先頭 48 バイトで確かめ直す。
    let mut first = [0u8; BLOCK_SIZE];
    first.copy_from_slice(&full[..BLOCK_SIZE]);
    classify(&RawBlock::parse(&first), process_id).ok()?;
    Some(FocusTicks::parse(&full))
}

/// マップしたビューから seqlock の手順で 48 バイトを読み、ビューとハンドルを閉じる。
fn read_view(handle: HANDLE) -> Result<RawBlock, ReadFailure> {
    let bytes = read_bytes::<BLOCK_SIZE>(handle);
    // SAFETY: `handle` は呼び出し元が渡した有効なハンドルで、ここで 1 回だけ閉じる。
    let _ = unsafe { CloseHandle(handle) };
    bytes.map(|b| RawBlock::parse(&b))
}

/// ハンドルの先頭 `N` バイトを seqlock の手順で読む（ビューは閉じる。**ハンドルは閉じない**）。
#[allow(clippy::cast_ptr_alignment)] // ビューはページ境界から始まり、`seq` はオフセット 8 なので u32 の境界に揃っている
fn read_bytes<const N: usize>(handle: HANDLE) -> Result<[u8; N], ReadFailure> {
    // SAFETY: `handle` は有効なセクション／ファイルマッピングのハンドル。読み取り専用で先頭 `N` バイトをマップする。
    let view = unsafe { MapViewOfFile(handle, FILE_MAP_READ, 0, 0, N) };
    if view.Value.is_null() {
        return Err(ReadFailure::Unresolved("map-view"));
    }
    let base = view.Value.cast::<u8>().cast_const();
    let mut result = Err(ReadFailure::Torn);
    for _ in 0..SEQLOCK_ATTEMPTS {
        // SAFETY: `base` から `N` バイトは有効な読み取り専用のビュー。`seq`（オフセット 8）は u32 の境界に揃っている。
        //         書き出し側は `InterlockedIncrement`（全バリア）で `seq` を更新するので、原子的な読み取りと対になる。
        let seq1 = unsafe { AtomicU32::from_ptr(base.add(8).cast::<u32>().cast_mut()) }
            .load(Ordering::Acquire);
        if seq1 % 2 == 1 {
            std::hint::spin_loop();
            continue;
        }
        let mut bytes = [0u8; N];
        for (i, b) in bytes.iter_mut().enumerate() {
            // SAFETY: `i < N`。書き出し側が同時に書くことがあるので volatile で読み、結果は `seq` の一致で確かめる。
            *b = unsafe { base.add(i).read_volatile() };
        }
        fence(Ordering::Acquire);
        // SAFETY: 上と同じ。
        let seq2 = unsafe { AtomicU32::from_ptr(base.add(8).cast::<u32>().cast_mut()) }
            .load(Ordering::Relaxed);
        if seq1 == seq2 {
            result = Ok(bytes);
            break;
        }
    }
    // SAFETY: `view` は `MapViewOfFile` が返したビューで、1 回だけ解除する。
    unsafe {
        let _ = UnmapViewOfFile(view);
    }
    result
}

/// 対象プロセスが AppContainer かを、トークンから判定する（ADR-265 D3、CI の実測と同じ手順）。
///
/// `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `OpenProcessToken(TOKEN_QUERY)` →
/// `GetTokenInformation(TokenIsAppContainer / TokenAppContainerSid / TokenSessionId)`。
fn probe_app_container(process_id: u32) -> AcProbe {
    if process_id == 0 {
        return AcProbe::Unresolved("no-pid");
    }
    // SAFETY: `process_id` は `GetWindowThreadProcessId` 経由で得た値。最小権限で開く（hook.rs の `foreground_window_is_elevated` と同じ流儀）。
    let Ok(process) =
        (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) })
    else {
        return AcProbe::Unresolved("open-process");
    };
    let mut token = HANDLE::default();
    // SAFETY: `process` は直前に開いた有効なハンドル、`token` はスタック上の出力先。`process` は 1 回だけ閉じる。
    let opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) };
    let _ = unsafe { CloseHandle(process) };
    if opened.is_err() {
        return AcProbe::Unresolved("open-token");
    }
    let probe = probe_token(token);
    // SAFETY: `token` は `OpenProcessToken` が返した有効なハンドルで、1 回だけ閉じる。
    let _ = unsafe { CloseHandle(token) };
    probe
}

fn probe_token(token: HANDLE) -> AcProbe {
    let mut is_app_container = 0u32;
    let mut returned = 0u32;
    // SAFETY: `token` は有効なトークンハンドル。出力先は `u32` 1 個分で、そのサイズを渡す。
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenIsAppContainer,
            Some((&raw mut is_app_container).cast()),
            4,
            &raw mut returned,
        )
    };
    if queried.is_err() {
        return AcProbe::Unresolved("token-is-app-container");
    }
    if is_app_container == 0 {
        return AcProbe::NotAppContainer;
    }

    let mut session_id = 0u32;
    // SAFETY: 上と同じ（`TokenSessionId` の出力は `u32`）。
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenSessionId,
            Some((&raw mut session_id).cast()),
            4,
            &raw mut returned,
        )
    };
    if queried.is_err() {
        return AcProbe::Unresolved("token-session-id");
    }

    // `TOKEN_APPCONTAINER_INFORMATION { PSID TokenAppContainer }` と、その SID 本体が同じバッファに入る。
    // u64 の配列で確保して、先頭のポインタを境界どおりに読めるようにする。
    let mut buffer = [0u64; 64];
    // SAFETY: バッファは 512 バイトで、そのサイズを渡す。
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenAppContainerSid,
            Some(buffer.as_mut_ptr().cast()),
            u32::try_from(size_of_val(&buffer)).unwrap_or(0),
            &raw mut returned,
        )
    };
    if queried.is_err() {
        return AcProbe::Unresolved("token-app-container-sid");
    }
    // SAFETY: 成功したので、先頭の 8 バイトは SID（バッファ内を指す）へのポインタ。`buffer` は u64 配列なので境界は揃っている。
    let sid_ptr = unsafe { buffer.as_ptr().cast::<*const u8>().read() };
    if sid_ptr.is_null() {
        return AcProbe::Unresolved("sid-null");
    }
    // SAFETY: `sid_ptr` はバッファ内の SID を指す。SID は 2 バイト目が下位権限の数で、長さは 8 + 4 × その数（最大 15）。
    let sid_bytes = unsafe {
        let count = usize::from(sid_ptr.add(1).read());
        std::slice::from_raw_parts(sid_ptr, 8 + 4 * count.min(15))
    };
    format_sid(sid_bytes).map_or(AcProbe::Unresolved("sid-format"), |sid| {
        AcProbe::AppContainer { session_id, sid }
    })
}

/// `NtOpenSection` で、AppContainer の名前空間の共有メモリを読み取り専用で開く。
fn open_section(path: &str) -> Result<HANDLE, ReadFailure> {
    // SAFETY: ntdll はすべてのプロセスに読み込み済み。`NtOpenSection` は (PHANDLE, ACCESS_MASK, POBJECT_ATTRIBUTES) -> NTSTATUS の関数
    //         （examples/typing_stress/suspend.rs の `nt_fns` と同じ動的解決）。
    let nt_open_section: NtOpenSectionFn = unsafe {
        let Ok(module) = GetModuleHandleW(w!("ntdll.dll")) else {
            return Err(ReadFailure::Unresolved("ntdll"));
        };
        let Some(entry) = GetProcAddress(module, s!("NtOpenSection")) else {
            return Err(ReadFailure::Unresolved("nt-open-section"));
        };
        std::mem::transmute::<unsafe extern "system" fn() -> isize, NtOpenSectionFn>(entry)
    };

    let mut wide: Vec<u16> = path.encode_utf16().collect();
    let Ok(byte_len) = u16::try_from(wide.len() * 2) else {
        return Err(ReadFailure::Unresolved("path-too-long"));
    };
    let mut name = UnicodeString {
        length: byte_len,
        maximum_length: byte_len,
        buffer: wide.as_mut_ptr(),
    };
    let attributes = ObjectAttributes {
        length: u32::try_from(size_of::<ObjectAttributes>()).unwrap_or(0),
        root_directory: HANDLE::default(),
        object_name: &raw mut name,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: std::ptr::null_mut(),
        security_quality_of_service: std::ptr::null_mut(),
    };
    let mut handle = HANDLE::default();
    // SAFETY: `handle`・`attributes`・`name`・`wide` はこの呼び出しの間スタック／ヒープ上で有効。
    let status =
        unsafe { nt_open_section(&raw mut handle, SECTION_MAP_READ, &raw const attributes) };
    if status >= 0 {
        return Ok(handle);
    }
    let status = u32::from_ne_bytes(status.to_ne_bytes());
    if status == STATUS_OBJECT_NAME_NOT_FOUND || status == STATUS_OBJECT_PATH_NOT_FOUND {
        Err(ReadFailure::NotFound)
    } else {
        Err(ReadFailure::Denied(status))
    }
}
