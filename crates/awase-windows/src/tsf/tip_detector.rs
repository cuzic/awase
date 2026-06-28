//! TSF TIP (Text Input Processor) の種別検出。
//!
//! `ITfInputProcessorProfileMgr::GetActiveProfile` により現在アクティブな TIP の CLSID を取得し、
//! GJI / MS-IME を識別する。GJI の CLSID はバージョンや環境で変わりうるため、
//! 起動時に `EnumProfiles` + display name マッチングで動的に発見してセッションキャッシュに格納する。
//!
//! ## スレッドモデル
//!
//! このモジュールの全関数は COM STA 初期化済みのスレッド（`gji-io-monitor`）から呼ぶこと。
//! COM インターフェース（`ITfInputProcessorProfileMgr` 等）は STA アパートメントに束縛されるため、
//! 生成スレッド以外で使ってはいけない。

use std::path::PathBuf;
use std::sync::OnceLock;

use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::TextServices::{
    CLSID_TF_InputProcessorProfiles, GUID_TFCAT_TIP_KEYBOARD, ITfInputProcessorProfileMgr,
    ITfInputProcessorProfiles, TF_INPUTPROCESSORPROFILE, TF_PROFILETYPE_INPUTPROCESSOR,
};
use windows::core::{Interface as _, GUID};

use super::observer::ActiveImeKind;

/// このセッションで発見した GJI の TIP CLSID キャッシュ。
///
/// `discover_and_cache_gji_clsid` により一度だけセットされる。
/// `None` = GJI 未インストールまたは `EnumProfiles` で発見できなかった。
static GJI_CLSID: OnceLock<GUID> = OnceLock::new();

/// キャッシュファイルの保存先ディレクトリ（exe と同じ場所）。
static CACHE_BASE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// `monitor_loop` 開始前に呼ぶ: キャッシュファイルの保存先を設定する。
pub(super) fn set_base_dir(dir: PathBuf) {
    let _ = CACHE_BASE_DIR.set(dir);
}

// ── COM オブジェクト生成 ──────────────────────────────────────────────────

/// `monitor_loop` 先頭で呼ぶ: COM プロファイルオブジェクトを生成する。
///
/// 失敗しても `None` を返すだけで `monitor_loop` の既存 GJI モニタリングは継続する。
pub(super) fn create_profile_ctx(
) -> Option<(ITfInputProcessorProfileMgr, ITfInputProcessorProfiles)> {
    unsafe {
        let mgr: ITfInputProcessorProfileMgr =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| log::warn!("[tip-detect] CoCreateInstance(ProfileMgr) failed: {e}"))
                .ok()?;
        let profiles: ITfInputProcessorProfiles = mgr
            .cast()
            .map_err(|e| log::warn!("[tip-detect] cast(ITfInputProcessorProfiles) failed: {e}"))
            .ok()?;
        Some((mgr, profiles))
    }
}

// ── 起動時 GJI CLSID 発見 ──────────────────────────────────────────────────

/// 日本語 TIP を列挙して GJI の CLSID をキャッシュに格納する（冪等）。
///
/// display name に "Google" を含む TIP を GJI として識別する。CLSID はバージョンや
/// インストール環境によって変わりうるためハードコードしない。
/// 既にキャッシュ済みの場合は即返却する。
pub(super) fn discover_and_cache_gji_clsid(
    mgr: &ITfInputProcessorProfileMgr,
    profiles: &ITfInputProcessorProfiles,
) {
    if GJI_CLSID.get().is_some() {
        return;
    }
    // キャッシュから読み込む（あれば EnumProfiles をスキップ）
    if let Some(clsid) = try_load_gji_clsid_from_cache() {
        let _ = GJI_CLSID.set(clsid);
        log::info!("[tip-detect] GJI CLSID loaded from cache: {}", fmt_guid(&clsid));
        return;
    }
    match find_gji_clsid(mgr, profiles) {
        Some(clsid) => {
            let _ = GJI_CLSID.set(clsid);
            log::info!("[tip-detect] GJI CLSID discovered: {}", fmt_guid(&clsid));
            save_gji_clsid_to_cache(&clsid);
        }
        None => {
            log::info!("[tip-detect] GJI not found in EnumProfiles(JA)");
        }
    }
}

fn find_gji_clsid(
    mgr: &ITfInputProcessorProfileMgr,
    profiles: &ITfInputProcessorProfiles,
) -> Option<GUID> {
    unsafe {
        let enumerator = mgr
            .EnumProfiles(0x0411 /* Japanese */)
            .map_err(|e| log::warn!("[tip-detect] EnumProfiles(JA) failed: {e}"))
            .ok()?;
        loop {
            let mut prof = TF_INPUTPROCESSORPROFILE::default();
            let mut fetched: u32 = 0;
            let res =
                enumerator.Next(std::slice::from_mut(&mut prof), &mut fetched as *mut u32);
            if res.is_err() || fetched == 0 {
                break;
            }
            if prof.dwProfileType != TF_PROFILETYPE_INPUTPROCESSOR {
                continue;
            }
            if let Ok(bstr) =
                profiles.GetLanguageProfileDescription(&prof.clsid, prof.langid, &prof.guidProfile)
            {
                if bstr.to_string().contains("Google") {
                    return Some(prof.clsid);
                }
            }
        }
        None
    }
}

// ── アクティブ IME 種別クエリ ──────────────────────────────────────────────

/// 現在アクティブな TIP の CLSID から IME 種別を返す。
///
/// - キャッシュ済み GJI CLSID と一致 → `GoogleJapaneseInput`
/// - それ以外の TIP または IMM32 HKL → `MicrosoftIme`
/// - 取得失敗 → `None`（呼び出し元はフォールバック値を使う）
pub(super) fn query_active_kind(mgr: &ITfInputProcessorProfileMgr) -> Option<ActiveImeKind> {
    unsafe {
        let mut prof = TF_INPUTPROCESSORPROFILE::default();
        mgr.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &mut prof)
            .map_err(|e| log::debug!("[tip-detect] GetActiveProfile failed: {e}"))
            .ok()?;

        if prof.dwProfileType != TF_PROFILETYPE_INPUTPROCESSOR {
            // IMM32 ベースの HKL → MS-IME 系とみなす
            return Some(ActiveImeKind::MicrosoftIme);
        }

        if let Some(gji_clsid) = GJI_CLSID.get() {
            if prof.clsid == *gji_clsid {
                return Some(ActiveImeKind::GoogleJapaneseInput);
            }
        }
        Some(ActiveImeKind::MicrosoftIme)
    }
}

// ── GUID キャッシュ (cache.toml) ──────────────────────────────────────────

const CACHE_FILENAME: &str = "cache.toml";
const CACHE_SECTION: &str = "tip_clsid";

/// `cache.toml` の `[tip_clsid]` セクションから GJI CLSID を読み込む。
fn try_load_gji_clsid_from_cache() -> Option<GUID> {
    let dir = CACHE_BASE_DIR.get()?;
    let path = dir.join(CACHE_FILENAME);
    let content = std::fs::read_to_string(&path).ok()?;
    let table: toml::Table = content.parse().ok()?;
    let section = table.get(CACHE_SECTION)?.as_table()?;
    let s = section.get("gji")?.as_str()?;
    let clsid = parse_guid(s)?;
    log::debug!("[tip-detect] cache hit: gji={s}");
    Some(clsid)
}

/// 発見した GJI CLSID を `cache.toml` の `[tip_clsid]` セクションに保存する。
fn save_gji_clsid_to_cache(clsid: &GUID) {
    let Some(dir) = CACHE_BASE_DIR.get() else { return };
    let path = dir.join(CACHE_FILENAME);
    let mut root: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or_default();
    let mut section = toml::Table::new();
    section.insert("gji".to_string(), toml::Value::String(fmt_guid(clsid)));
    root.insert(CACHE_SECTION.to_string(), toml::Value::Table(section));
    let content = toml::to_string_pretty(&root).unwrap_or_default();
    if let Err(e) = std::fs::write(&path, &content) {
        log::warn!("[tip-detect] cache 保存失敗: {e}");
    } else {
        log::info!("[tip-detect] GJI CLSID saved to cache.toml");
    }
}

/// GUID 文字列 `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` を `GUID` に変換する。
fn parse_guid(s: &str) -> Option<GUID> {
    let s = s.trim().trim_start_matches('{').trim_end_matches('}');
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 { return None; }
    let d1 = u32::from_str_radix(parts[0], 16).ok()?;
    let d2 = u16::from_str_radix(parts[1], 16).ok()?;
    let d3 = u16::from_str_radix(parts[2], 16).ok()?;
    let d4_hex = format!("{}{}", parts[3], parts[4]);
    if d4_hex.len() != 16 { return None; }
    let mut d4 = [0u8; 8];
    for i in 0..8 {
        d4[i] = u8::from_str_radix(&d4_hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(GUID { data1: d1, data2: d2, data3: d3, data4: d4 })
}

// ── 診断ダンプ ─────────────────────────────────────────────────────────────

/// 起動時診断: 日本語 TIP を全列挙して CLSID・名称をログ出力する（info レベル）。
///
/// 新しい IME 環境での CLSID 確認に使用する。GJI の CLSID は `discover_and_cache_gji_clsid`
/// が自動識別するが、このダンプで目視確認もできる。
pub(super) fn dump_profiles(
    mgr: &ITfInputProcessorProfileMgr,
    profiles: &ITfInputProcessorProfiles,
) {
    log::info!("[tip-detect] ── EnumProfiles(JA) start ──");
    unsafe {
        let Ok(enumerator) = mgr.EnumProfiles(0x0411) else {
            log::warn!("[tip-detect] EnumProfiles(JA) failed");
            return;
        };
        loop {
            let mut prof = TF_INPUTPROCESSORPROFILE::default();
            let mut fetched: u32 = 0;
            let res =
                enumerator.Next(std::slice::from_mut(&mut prof), &mut fetched as *mut u32);
            if res.is_err() || fetched == 0 {
                break;
            }
            let kind = if prof.dwProfileType == TF_PROFILETYPE_INPUTPROCESSOR {
                "TIP"
            } else {
                "HKL"
            };
            let desc = profiles
                .GetLanguageProfileDescription(&prof.clsid, prof.langid, &prof.guidProfile)
                .ok()
                .map(|b| b.to_string())
                .unwrap_or_default();
            log::info!(
                "[tip-detect] {kind} clsid={clsid} profile={pguid} lang={lang:04x} \
                 desc={desc:?}",
                clsid = fmt_guid(&prof.clsid),
                pguid = fmt_guid(&prof.guidProfile),
                lang = prof.langid,
            );
        }
    }
    log::info!("[tip-detect] ── EnumProfiles(JA) end ──");
}

// ── ユーティリティ ──────────────────────────────────────────────────────────

fn fmt_guid(g: &GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7],
    )
}
