/// フォーカス検出・注入モード決定に関する型定義モジュール。
///
/// 以前は `runtime::mod` に置かれていたが、focus 層に移動した（逆依存解消）。
/// `runtime` は `pub use crate::focus::classifier::*` で後方互換性を維持する。

use awase::config::{AppOverrideEntry, AppOverrides};

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

// ── override-entry helpers ──

/// `entries` の中に `(process_name, class_name)` にマッチするものがあるか。
fn matches_override_entry(entries: &[AppOverrideEntry], process_name: &str, class_name: &str) -> bool {
    entries.iter().any(|entry| {
        entry.process.eq_ignore_ascii_case(process_name)
            && entry.class.eq_ignore_ascii_case(class_name)
    })
}

/// `Windows.UI.Input.InputSite.WindowClass` フォーカス時に前景ウィンドウのクラスを使って
/// フォールバック判定する。マッチすれば `true`。
unsafe fn input_site_fallback_matches(
    entries: &[AppOverrideEntry],
    class_name: &str,
    process_name: &str,
) -> bool {
    if !class_name.eq_ignore_ascii_case("Windows.UI.Input.InputSite.WindowClass") {
        return false;
    }
    let fg_class = crate::ime::get_foreground_window_class();
    if fg_class.is_empty() || fg_class.eq_ignore_ascii_case(class_name) {
        return false;
    }
    let matched = matches_override_entry(entries, process_name, &fg_class);
    log::debug!(
        "[force-tsf] InputSite fallback: fg_class={fg_class:?} process={process_name:?} → matched={matched}"
    );
    matched
}

// ── ForceOverrides ──

/// アプリごとの注入モード・フォーカス種別オーバーライド設定を保持し、
/// 判断ロジックを提供する構造体。
///
/// `AppOverrides` をラップし、injection_hint/check_app_override を
/// メソッドとして集約することで呼び出し側 API を統一する。
pub struct ForceOverrides {
    inner: AppOverrides,
}

impl ForceOverrides {
    pub fn new(overrides: AppOverrides) -> Self {
        Self { inner: overrides }
    }

    /// `force_text` / `force_bypass` オーバーライドをチェックする。
    pub(crate) fn check_app_override(
        &self,
        process_id: u32,
        class_name: &str,
    ) -> Option<awase::types::FocusKind> {
        if self.inner.force_text.is_empty() && self.inner.force_bypass.is_empty() {
            return None;
        }
        let process_name = super::classify::get_process_name(process_id);
        for entry in &self.inner.force_text {
            if entry.process.eq_ignore_ascii_case(&process_name)
                && entry.class.eq_ignore_ascii_case(class_name)
            {
                return Some(awase::types::FocusKind::TextInput);
            }
        }
        for entry in &self.inner.force_bypass {
            if entry.process.eq_ignore_ascii_case(&process_name)
                && entry.class.eq_ignore_ascii_case(class_name)
            {
                return Some(awase::types::FocusKind::NonText);
            }
        }
        None
    }

    /// 注入ヒントを返す（ForceTsf / ForceVk / Default）。
    ///
    /// `process_name` の取得を1回にまとめ、ヘルパー関数経由で判定する。
    pub(crate) fn injection_hint(&self, process_id: u32, class_name: &str) -> InjectionHint {
        if self.inner.force_tsf.is_empty() && self.inner.force_vk.is_empty() {
            return InjectionHint::Default;
        }
        let process_name = super::classify::get_process_name(process_id);
        // force_tsf チェック（InputSite フォールバック含む）
        if !self.inner.force_tsf.is_empty() {
            if matches_override_entry(&self.inner.force_tsf, &process_name, class_name) {
                return InjectionHint::ForceTsf;
            }
            if unsafe { input_site_fallback_matches(&self.inner.force_tsf, class_name, &process_name) } {
                return InjectionHint::ForceTsf;
            }
        }
        // force_vk チェック
        if matches_override_entry(&self.inner.force_vk, &process_name, class_name) {
            return InjectionHint::ForceVk;
        }
        InjectionHint::Default
    }
}

// ── ImmCapabilityStore ──

/// IMM 能力の学習・永続化を担う構造体。
///
/// `base_dir` を外から隠蔽し、`learn()` 一発でキャッシュ更新とファイル保存を行う。
pub struct ImmCapabilityStore {
    cache: std::collections::HashMap<String, ImmCapability>,
    base_dir: std::path::PathBuf,
}

impl ImmCapabilityStore {
    pub(crate) fn new(base_dir: std::path::PathBuf) -> Self {
        let cache = load_imm_cache(&base_dir);
        Self { cache, base_dir }
    }

    pub(crate) fn get(&self, class_name: &str) -> Option<ImmCapability> {
        self.cache.get(class_name).copied()
    }

    pub(crate) fn contains_key(&self, class_name: &str) -> bool {
        self.cache.contains_key(class_name)
    }

    pub(crate) fn learn(&mut self, class_name: String, cap: ImmCapability) {
        self.cache.insert(class_name, cap);
        save_imm_cache(&self.base_dir, &self.cache);
    }

    /// キャッシュをメモリとファイルの両方からクリアする。
    pub(crate) fn clear(&mut self) -> usize {
        let count = self.cache.len();
        self.cache.clear();
        let path = self.base_dir.join(IMM_CACHE_FILENAME);
        if let Err(e) = std::fs::remove_file(&path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("Failed to remove IMM cache file {}: {e}", path.display());
            }
        }
        count
    }
}

// ── AppKindClassifier ──

/// フォーカス検出に関するシングルスレッド状態を集約する構造体
#[allow(missing_debug_implementations)]
pub struct AppKindClassifier {
    pub cache: super::cache::FocusCache,
    pub overrides: ForceOverrides,
    pub last_focus_info: Option<(u32, String)>,
    pub uia_sender: Option<std::sync::mpsc::Sender<super::uia::SendableHwnd>>,
    /// IMM 能力の学習・永続化ストア。
    pub imm_learning: ImmCapabilityStore,
    /// per-HWND IME 状態キャッシュ。
    pub hwnd_ime_cache: std::collections::HashMap<(u32, String), super::hwnd_cache::HwndImeSnapshot>,
}

impl AppKindClassifier {
    pub fn new(overrides: AppOverrides) -> Self {
        let base_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        Self {
            cache: super::cache::FocusCache::new(),
            overrides: ForceOverrides::new(overrides),
            last_focus_info: None,
            uia_sender: None,
            imm_learning: ImmCapabilityStore::new(base_dir),
            hwnd_ime_cache: std::collections::HashMap::new(),
        }
    }

    /// IMM 能力キャッシュに学習結果を追加し、ファイルに永続化する。
    pub fn learn_imm_capability(&mut self, class_name: String, cap: ImmCapability) {
        self.imm_learning.learn(class_name, cap);
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
        self.overrides.injection_hint(*pid, class)
    }
}

