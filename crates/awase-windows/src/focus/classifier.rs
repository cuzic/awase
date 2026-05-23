/// フォーカス検出・注入モード決定に関する型定義モジュール。
///
/// 以前は `runtime::mod` に置かれていたが、focus 層に移動した（逆依存解消）。
/// `runtime` は `pub use crate::focus::classifier::*` で後方互換性を維持する。

use awase::config::AppOverrides;

// ── IMM capability cache ──

/// IMM 能力キャッシュファイル名（config.toml と同じディレクトリ）
const IMM_CACHE_FILENAME: &str = "imm_cache.toml";

/// IMM ブリッジの検出結果（class_name ごとにキャッシュ）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImmCapability {
    /// IMM ブリッジが動作する（ImmGetOpenStatus が信頼できる値を返す）
    Works,
    /// IMM ブリッジが動作しない（独自 TSF text store を持つアプリ）
    Broken,
}

/// IMM 能力キャッシュをファイルから読み込む。
fn load_imm_cache(base_dir: &std::path::Path) -> std::collections::HashMap<String, ImmCapability> {
    let path = base_dir.join(IMM_CACHE_FILENAME);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return std::collections::HashMap::new(),
    };
    let table: toml::Table = match content.parse() {
        Ok(t) => t,
        Err(e) => {
            log::warn!("Failed to parse {}: {e}", path.display());
            return std::collections::HashMap::new();
        }
    };
    let mut cache = std::collections::HashMap::new();
    if let Some(toml::Value::Table(classes)) = table.get("classes") {
        for (class_name, value) in classes {
            if let toml::Value::String(s) = value {
                let cap = match s.as_str() {
                    "works" => ImmCapability::Works,
                    "broken" => ImmCapability::Broken,
                    _ => continue,
                };
                cache.insert(class_name.clone(), cap);
            }
        }
    }
    if !cache.is_empty() {
        log::info!("Loaded IMM capability cache: {} entries from {}", cache.len(), path.display());
    }
    cache
}

/// IMM 能力キャッシュをファイルに書き出す。
fn save_imm_cache(base_dir: &std::path::Path, cache: &std::collections::HashMap<String, ImmCapability>) {
    let path = base_dir.join(IMM_CACHE_FILENAME);
    let mut classes = toml::Table::new();
    for (class_name, cap) in cache {
        let value = match cap {
            ImmCapability::Works => "works",
            ImmCapability::Broken => "broken",
        };
        classes.insert(class_name.clone(), toml::Value::String(value.to_string()));
    }
    let mut root = toml::Table::new();
    root.insert("classes".to_string(), toml::Value::Table(classes));
    let content = toml::to_string_pretty(&root).unwrap_or_default();
    if let Err(e) = std::fs::write(&path, content) {
        log::warn!("Failed to save IMM cache to {}: {e}", path.display());
    } else {
        log::debug!("Saved IMM capability cache: {} entries to {}", cache.len(), path.display());
    }
}

// ── InjectionHint ──

/// output 層が注入モードを決定するために必要な、focus 層の公開セマンティクス。
///
/// `AppKindClassifier::injection_hint()` が返す型。
/// output 層はこの型のみを参照し、focus 内部フィールドに直接アクセスしない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionHint {
    /// config の `force_tsf` エントリにマッチ → TSF Sequential VK 注入
    ForceTsf,
    /// config の `force_vk` エントリにマッチ → VK Batched 注入
    ForceVk,
    /// オーバーライドなし → AppKind に従って output 層が最終決定する
    Default,
}

// ── AppKindClassifier ──

/// フォーカス検出に関するシングルスレッド状態を集約する構造体
#[allow(missing_debug_implementations)]
pub struct AppKindClassifier {
    pub(crate) cache: super::cache::FocusCache,
    pub(crate) overrides: AppOverrides,
    pub(crate) last_focus_info: Option<(u32, String)>,
    pub(crate) uia_sender: Option<std::sync::mpsc::Sender<super::uia::SendableHwnd>>,
    /// class_name ごとの IMM ブリッジ能力キャッシュ。
    pub(crate) imm_capability_cache: std::collections::HashMap<String, ImmCapability>,
    /// per-HWND IME 状態キャッシュ。
    pub(crate) hwnd_ime_cache: std::collections::HashMap<(u32, String), super::hwnd_cache::HwndImeSnapshot>,
    /// キャッシュファイルの格納ディレクトリ
    base_dir: std::path::PathBuf,
}

impl AppKindClassifier {
    pub fn new(overrides: AppOverrides) -> Self {
        let base_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let imm_capability_cache = load_imm_cache(&base_dir);
        Self {
            cache: super::cache::FocusCache::new(),
            overrides,
            last_focus_info: None,
            uia_sender: None,
            imm_capability_cache,
            hwnd_ime_cache: std::collections::HashMap::new(),
            base_dir,
        }
    }

    /// IMM 能力キャッシュに学習結果を追加し、ファイルに永続化する。
    pub fn learn_imm_capability(&mut self, class_name: String, cap: ImmCapability) {
        self.imm_capability_cache.insert(class_name, cap);
        save_imm_cache(&self.base_dir, &self.imm_capability_cache);
    }

    pub fn set_uia_sender(
        &mut self,
        sender: std::sync::mpsc::Sender<super::uia::SendableHwnd>,
    ) {
        self.uia_sender = Some(sender);
    }

    /// 現在のフォーカス先に対する注入ヒントを返す。
    #[must_use]
    pub fn injection_hint(&self) -> InjectionHint {
        let Some((pid, class)) = self.last_focus_info.as_ref() else {
            return InjectionHint::Default;
        };
        if is_force_tsf(&self.overrides, *pid, class) {
            return InjectionHint::ForceTsf;
        }
        if is_force_vk(&self.overrides, *pid, class) {
            return InjectionHint::ForceVk;
        }
        InjectionHint::Default
    }
}

// ── Override helper fns ──

/// Config の force_text / force_bypass オーバーライドをチェックする。
pub(crate) fn check_app_override(
    overrides: &AppOverrides,
    process_id: u32,
    class_name: &str,
) -> Option<awase::types::FocusKind> {
    if overrides.force_text.is_empty() && overrides.force_bypass.is_empty() {
        return None;
    }
    let process_name = super::classify::get_process_name(process_id);
    for entry in &overrides.force_text {
        if entry.process.eq_ignore_ascii_case(&process_name)
            && entry.class.eq_ignore_ascii_case(class_name)
        {
            return Some(awase::types::FocusKind::TextInput);
        }
    }
    for entry in &overrides.force_bypass {
        if entry.process.eq_ignore_ascii_case(&process_name)
            && entry.class.eq_ignore_ascii_case(class_name)
        {
            return Some(awase::types::FocusKind::NonText);
        }
    }
    None
}

/// Config の `force_vk` オーバーライドに現在のフォーカス先がマッチするか判定する。
pub fn is_force_vk(overrides: &AppOverrides, process_id: u32, class_name: &str) -> bool {
    if overrides.force_vk.is_empty() {
        return false;
    }
    let process_name = super::classify::get_process_name(process_id);
    overrides.force_vk.iter().any(|entry| {
        entry.process.eq_ignore_ascii_case(&process_name)
            && entry.class.eq_ignore_ascii_case(class_name)
    })
}

/// Config の `force_tsf` オーバーライドに現在のフォーカス先がマッチするか判定する。
///
/// `Windows.UI.Input.InputSite.WindowClass` フォーカス時は GetForegroundWindow() で
/// トップレベルクラスを再マッチし、force_tsf 設定が InputSite でも機能するようにする。
pub fn is_force_tsf(overrides: &AppOverrides, process_id: u32, class_name: &str) -> bool {
    if overrides.force_tsf.is_empty() {
        return false;
    }
    let process_name = super::classify::get_process_name(process_id);
    if overrides.force_tsf.iter().any(|entry| {
        entry.process.eq_ignore_ascii_case(&process_name)
            && entry.class.eq_ignore_ascii_case(class_name)
    }) {
        return true;
    }
    if class_name.eq_ignore_ascii_case("Windows.UI.Input.InputSite.WindowClass") {
        let fg_class = unsafe { crate::ime::get_foreground_window_class() };
        if !fg_class.is_empty() && !fg_class.eq_ignore_ascii_case(class_name) {
            let matched = overrides.force_tsf.iter().any(|entry| {
                entry.process.eq_ignore_ascii_case(&process_name)
                    && entry.class.eq_ignore_ascii_case(&fg_class)
            });
            log::debug!(
                "[force-tsf] InputSite fallback: fg_class={fg_class:?} process={process_name:?} → matched={matched}"
            );
            return matched;
        }
    }
    false
}
