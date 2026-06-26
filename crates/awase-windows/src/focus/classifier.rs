//! フォーカス検出・注入モード決定に関する型定義モジュール。
//!
//! 以前は `runtime::mod` に置かれていたが、focus 層に移動した（逆依存解消）。
//! `runtime` は `pub use crate::focus::classifier::*` で後方互換性を維持する。

use awase::config::{AppOverrideEntry, AppOverrides};

// ── IMM capability cache ──

/// 学習済みキャッシュファイル名（exe と同じディレクトリ）
const CACHE_FILENAME: &str = "cache.toml";
/// 旧バージョンのキャッシュファイル名（移行時に削除対象）
const LEGACY_CACHE_FILENAME: &str = "imm_cache.toml";

/// IMM32 クロスプロセス制御能力の検出結果（class_name ごとにキャッシュ）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImmCapability {
    /// IMM32 クロスプロセス制御が動作する（`ImmGetOpenStatus` が信頼できる値を返す）
    Works,
    /// IMM32 クロスプロセス制御が使えない（独自 TSF text store を持つアプリ等）
    Unavailable,
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
fn matches_override_entry(
    entries: &[AppOverrideEntry],
    process_name: &str,
    class_name: &str,
) -> bool {
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
#[derive(Debug)]
pub struct ForceOverrides {
    inner: AppOverrides,
}

impl ForceOverrides {
    #[must_use]
    pub const fn new(overrides: AppOverrides) -> Self {
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
            if unsafe {
                input_site_fallback_matches(&self.inner.force_tsf, class_name, &process_name)
            } {
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
#[derive(Debug)]
pub struct ImmCapabilityStore {
    cache: std::collections::HashMap<String, ImmCapability>,
    base_dir: std::path::PathBuf,
}

impl ImmCapabilityStore {
    pub(crate) fn new(base_dir: std::path::PathBuf) -> Self {
        let cache = Self::load(&base_dir);
        Self { cache, base_dir }
    }

    pub(crate) fn get(&self, class_name: &str) -> Option<ImmCapability> {
        self.cache.get(class_name).copied()
    }

    pub(crate) fn learn(&mut self, class_name: String, cap: ImmCapability) {
        self.cache.insert(class_name, cap);
        self.save();
    }

    fn load(base_dir: &std::path::Path) -> std::collections::HashMap<String, ImmCapability> {
        let path = base_dir.join(CACHE_FILENAME);
        let Ok(content) = std::fs::read_to_string(&path) else {
            return std::collections::HashMap::new();
        };
        let table: toml::Table = match content.parse() {
            Ok(t) => t,
            Err(e) => {
                log::warn!("Failed to parse {}: {e}", path.display());
                return std::collections::HashMap::new();
            }
        };
        let mut cache = std::collections::HashMap::new();
        if let Some(toml::Value::Table(section)) = table.get("imm_capability") {
            for (class_name, value) in section {
                if let toml::Value::String(s) = value {
                    let cap = match s.as_str() {
                        "works" => ImmCapability::Works,
                        "unavailable" | "broken" => ImmCapability::Unavailable,
                        _ => continue,
                    };
                    cache.insert(class_name.clone(), cap);
                }
            }
        }
        if !cache.is_empty() {
            log::info!(
                "Loaded IMM capability cache: {} entries from {}",
                cache.len(),
                path.display()
            );
        }
        cache
    }

    fn save(&self) {
        let mut section = toml::Table::new();
        for (class_name, cap) in &self.cache {
            let value = match cap {
                ImmCapability::Works => "works",
                ImmCapability::Unavailable => "unavailable",
            };
            section.insert(class_name.clone(), toml::Value::String(value.to_string()));
        }
        save_section(&self.base_dir, "imm_capability", section);
        log::debug!("Saved IMM capability cache: {} entries", self.cache.len());
    }

    /// キャッシュをメモリとファイルの両方からクリアする。
    /// `cache.toml` 全体を削除するため `InjectionModeStore` のデータも失われる。
    pub(crate) fn clear(&mut self) -> usize {
        let count = self.cache.len();
        self.cache.clear();
        for filename in [CACHE_FILENAME, LEGACY_CACHE_FILENAME] {
            let path = self.base_dir.join(filename);
            if let Err(e) = std::fs::remove_file(&path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("Failed to remove cache file {}: {e}", path.display());
                }
            }
        }
        count
    }
}

// ── キャッシュファイル共通 write ヘルパー ──────────────────────────────────────

/// `cache.toml` の指定セクションだけを更新し、他のセクションを保持して上書き保存する。
fn save_section(base_dir: &std::path::Path, section_name: &str, section: toml::Table) {
    let path = base_dir.join(CACHE_FILENAME);
    let mut root: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or_default();
    root.insert(section_name.to_string(), toml::Value::Table(section));
    let content = toml::to_string_pretty(&root).unwrap_or_default();
    if let Err(e) = std::fs::write(&path, &content) {
        log::warn!("Failed to save cache to {}: {e}", path.display());
    }
}

// ── InjectionModeStore ────────────────────────────────────────────────────────

/// 事後昇格学習ストア：GJI write 観測によって「Tsf モードが必要」と判明した class_name を永続化する。
///
/// `cache.toml` の `[injection_mode]` セクションに `class_name = "tsf"` の形式で保存する。
/// `ForceOverrides::injection_hint()` が未マッチのとき、このストアを参照して `ForceTsf` を返す。
#[derive(Debug)]
pub struct InjectionModeStore {
    tsf_classes: std::collections::HashSet<String>,
    base_dir: std::path::PathBuf,
}

impl InjectionModeStore {
    pub(crate) fn new(base_dir: std::path::PathBuf) -> Self {
        let tsf_classes = Self::load(&base_dir);
        Self { tsf_classes, base_dir }
    }

    /// class_name が Tsf モード必要と学習済みかどうか。
    pub(crate) fn has_tsf(&self, class_name: &str) -> bool {
        self.tsf_classes.contains(class_name)
    }

    /// class_name を Tsf 必要としてキャッシュに登録し永続化する（冪等）。
    pub(crate) fn learn_tsf(&mut self, class_name: String) {
        if self.tsf_classes.insert(class_name) {
            self.save();
        }
    }

    fn load(base_dir: &std::path::Path) -> std::collections::HashSet<String> {
        let path = base_dir.join(CACHE_FILENAME);
        let Ok(content) = std::fs::read_to_string(&path) else {
            return std::collections::HashSet::new();
        };
        let table: toml::Table = match content.parse() {
            Ok(t) => t,
            Err(_) => return std::collections::HashSet::new(),
        };
        let mut classes = std::collections::HashSet::new();
        if let Some(toml::Value::Table(section)) = table.get("injection_mode") {
            for (class_name, value) in section {
                if matches!(value, toml::Value::String(s) if s == "tsf") {
                    classes.insert(class_name.clone());
                }
            }
        }
        if !classes.is_empty() {
            log::info!(
                "Loaded injection mode cache: {} TSF classes from {}",
                classes.len(),
                path.display()
            );
        }
        classes
    }

    fn save(&self) {
        let mut section = toml::Table::new();
        for class_name in &self.tsf_classes {
            section.insert(class_name.clone(), toml::Value::String("tsf".to_string()));
        }
        save_section(&self.base_dir, "injection_mode", section);
        log::debug!("Saved injection mode cache: {} TSF classes", self.tsf_classes.len());
    }
}
