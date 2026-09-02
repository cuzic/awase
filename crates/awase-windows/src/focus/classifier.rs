#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! フォーカス検出・注入モード決定に関する型定義モジュール。
//!
//! 以前は `runtime::mod` に置かれていたが、focus 層に移動した（逆依存解消）。
//! `runtime` は `pub use crate::focus::classifier::*` で後方互換性を維持する。

use awase::config::{AppOverrideEntry, AppOverrides};
use std::sync::{OnceLock, RwLock};

/// `app_overrides.input_relay_apps`（issue #136 / BUG-90 決定4）の
/// `ime.rs::read_ime_state_fast` 専用スナップショット。
///
/// **新しい呼び出し元を足さないこと。** `Runtime`/`FocusTracker` に到達できる
/// 呼び出し元は `FocusTracker::input_relay_apps()`（正規ルート、ロック不要）を
/// 使う（`runtime/mod.rs::on_window_focus_event` 参照）。このプロセスグローバル
/// が存在するのは `read_ime_state_fast` が `self` を持たない `pub unsafe fn`
/// であるという1点のためだけ。
///
/// `RwLock` を使う理由: このリポジトリは単一スレッド・メッセージループ駆動で
/// ロック不要が原則（CLAUDE.md「Concurrency model」）だが、
/// `read_ime_state_fast` は `read_ime_state_fast_async` 経由で
/// `offload_unsafe`（ワーカースレッド）からも直接（`open_chain.rs`、
/// メインスレッドの `spawn_local` 内）からも呼ばれる。config リロード時の
/// 書き込み（`ForceOverrides::new`）はメインスレッドから、読み取りは両方の
/// スレッドから起こりうるため、`RwLock` はこの1箇所に限り正当。
static INPUT_RELAY_APPS: OnceLock<RwLock<Vec<String>>> = OnceLock::new();

fn input_relay_apps_cell() -> &'static RwLock<Vec<String>> {
    INPUT_RELAY_APPS.get_or_init(|| RwLock::new(Vec::new()))
}

/// `RwLock` が poison していても中身は捨てない（`/code-review` 指摘: 以前は
/// `unwrap_or_default()` で空配列にフォールバックしていたが、これだと poison
/// 後は `input_relay_apps` 設定が永続的に無視される＝ issue #136 の再発防止が
/// 静かに無効化される。`clone_from` は単純な `Vec<String>` 代入で複合的な
/// 不変条件を持たないため、poison 直前の中身をそのまま使い続けて安全）。
pub(crate) fn input_relay_apps_snapshot() -> Vec<String> {
    input_relay_apps_cell()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

// ── IMM capability cache ──

/// 学習済みキャッシュファイル名（exe と同じディレクトリ）
const CACHE_FILENAME: &str = "cache.toml";

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
    /// **副作用**: `input_relay_apps` をプロセスグローバル
    /// （`INPUT_RELAY_APPS`）へ複製する（`ime.rs::read_ime_state_fast` が
    /// `self` を持たないため、issue #136 / BUG-90 決定4）。設定リロードで
    /// 複数回呼ばれた場合は最後の呼び出しが勝つ。
    #[must_use]
    pub fn new(overrides: AppOverrides) -> Self {
        // poison していても書き込み自体は諦めない（`input_relay_apps_snapshot`
        // の doc コメント参照。`if let Ok` で握り潰すと config リロード後も
        // 古い/空の値が読まれ続けるリグレッションになる）。
        let mut apps = input_relay_apps_cell()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        apps.clone_from(&overrides.input_relay_apps);
        drop(apps);
        Self { inner: overrides }
    }

    /// `force_text` / `force_bypass` オーバーライドをチェックする。
    pub(crate) fn check_app_override(
        &self,
        process_id: u32,
        class_name: &str,
    ) -> Option<crate::focus::FocusKind> {
        if self.inner.force_text.is_empty() && self.inner.force_bypass.is_empty() {
            return None;
        }
        let process_name = super::classify::get_process_name(process_id);
        for entry in &self.inner.force_text {
            if entry.process.eq_ignore_ascii_case(&process_name)
                && entry.class.eq_ignore_ascii_case(class_name)
            {
                return Some(crate::focus::FocusKind::TextInput);
            }
        }
        for entry in &self.inner.force_bypass {
            if entry.process.eq_ignore_ascii_case(&process_name)
                && entry.class.eq_ignore_ascii_case(class_name)
            {
                return Some(crate::focus::FocusKind::NonText);
            }
        }
        None
    }

    /// `disable_apps` にマッチするプロセス名か（class 不問）。
    ///
    /// `force_bypass` と異なりウィンドウクラス名を問わない — mstsc.exe のように
    /// 複数のウィンドウクラスを取りうるアプリを丸ごと無効化するための入り口。
    #[must_use]
    pub(crate) fn is_app_disabled(&self, process_name: &str) -> bool {
        crate::state::app_suppression::matches_disabled_app(&self.inner.disable_apps, process_name)
    }

    #[must_use]
    pub(crate) fn input_relay_apps(&self) -> &[String] {
        &self.inner.input_relay_apps
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
    /// `ImmGetDefaultIMEWnd`=NULL の連続観測回数（class_name ごと、未確定分のみ）。
    /// ディスクへは永続化しない — セッションをまたいで引き継ぐ必要はなく、
    /// 再起動のたびに空から積み直せば十分（BUG-56対策）。
    pending_unavailable: std::collections::HashMap<String, u32>,
}

impl ImmCapabilityStore {
    /// `ImmGetDefaultIMEWnd`=NULL の連続観測がこの回数に達したら `Unavailable` として
    /// 確定・永続化する。Qt 等のジェネリックなウィンドウクラス名は、本物のテキスト
    /// 入力欄と無関係な一時ウィンドウ（通知アイコン等）で使い回されることがあり、
    /// 単発の NULL 観測だけで確定すると本物の入力欄まで巻き込んで誤って IMM32
    /// クロスプロセス制御を諦めてしまう（2026-08-07 実機: LINE で「でででで」
    /// 「はははは」等の文字重複コミット。`docs/known-bugs.md` BUG-56、
    /// `.claude/rules/ime-belief-architecture.md` の BUG-19 由来の 2 回連続観測
    /// デバウンスと同じ考え方）。
    const UNAVAILABLE_CONFIRM_THRESHOLD: u32 = 2;

    pub(crate) fn new(base_dir: std::path::PathBuf) -> Self {
        let cache = Self::load(&base_dir);
        Self {
            cache,
            base_dir,
            pending_unavailable: std::collections::HashMap::new(),
        }
    }

    pub(crate) fn get(&self, class_name: &str) -> Option<ImmCapability> {
        self.cache.get(class_name).copied()
    }

    pub(crate) fn learn(&mut self, class_name: String, cap: ImmCapability) {
        self.cache.insert(class_name, cap);
        self.save();
    }

    /// `ImmGetDefaultIMEWnd`=NULL の観測を記録する。閾値回連続で観測されて初めて
    /// `Unavailable` として確定・永続化する（`UNAVAILABLE_CONFIRM_THRESHOLD` 参照）。
    /// 呼び出し元（`learn_imm_capability_on_focus`）は既に学習済みの class_name を
    /// スキップ済みの前提。
    pub(crate) fn record_null_probe(&mut self, class_name: String) {
        let count = self
            .pending_unavailable
            .entry(class_name.clone())
            .or_insert(0);
        *count += 1;
        if *count >= Self::UNAVAILABLE_CONFIRM_THRESHOLD {
            self.pending_unavailable.remove(&class_name);
            self.learn(class_name, ImmCapability::Unavailable);
        }
    }

    /// 非 NULL 観測（IMM32 が応答した）を得たら、その class_name の「疑い」カウントを
    /// クリアする。決め打ちの一時ウィンドウが NULL を返した直後に本物の入力欄が
    /// フォーカスされて non-NULL を返すケースで、疑いが誤って積み上がらないようにする。
    pub(crate) fn clear_pending_unavailable(&mut self, class_name: &str) {
        self.pending_unavailable.remove(class_name);
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
        Self {
            tsf_classes,
            base_dir,
        }
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
        log::debug!(
            "Saved injection mode cache: {} TSF classes",
            self.tsf_classes.len()
        );
    }
}

#[cfg(test)]
mod imm_capability_store_tests {
    use super::{ImmCapability, ImmCapabilityStore};

    /// テストごとに衝突しない一時ディレクトリを作る。`std::fs` のみで完結するため
    /// Win32 依存なしで Linux 上でも実行できる（`journal.rs` の一時ファイル方式と同じ）。
    ///
    /// PID を含めるのは、`cargo nextest`（デフォルトでテストごとに別プロセスを
    /// 起動する）の下では COUNTER が毎回 0 から・メインスレッドの `ThreadId` も
    /// 毎回同じ値から始まるため、PID 無しだと全テストが同じパスを計算してしまい、
    /// 前のテストプロセスが書き残した `cache.toml` を後続のテストが誤って読む
    /// 事故が起きるため（実機 CI で `single_null_probe_does_not_confirm_unavailable`
    /// 等が `Some(Unavailable)` を誤って観測して発覚）。
    fn temp_store() -> ImmCapabilityStore {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "awase_imm_capability_store_test_{}_{n}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir for ImmCapabilityStore test");
        ImmCapabilityStore::new(dir)
    }

    // BUG-56: 単発の NULL 観測だけでは Unavailable を確定しない。
    #[test]
    fn single_null_probe_does_not_confirm_unavailable() {
        let mut store = temp_store();
        store.record_null_probe("Qt663QWindowIcon".to_string());
        assert_eq!(store.get("Qt663QWindowIcon"), None);
    }

    // BUG-56: 閾値回（2回）連続で NULL を観測して初めて Unavailable が確定する。
    #[test]
    fn two_consecutive_null_probes_confirm_unavailable() {
        let mut store = temp_store();
        store.record_null_probe("Qt663QWindowIcon".to_string());
        store.record_null_probe("Qt663QWindowIcon".to_string());
        assert_eq!(
            store.get("Qt663QWindowIcon"),
            Some(ImmCapability::Unavailable)
        );
    }

    // BUG-56: 途中で非 NULL 観測（本物の入力欄が応答した）が挟まると疑いカウントが
    // リセットされ、次の NULL 単発では確定しない。
    #[test]
    fn non_null_observation_resets_pending_count() {
        let mut store = temp_store();
        store.record_null_probe("Qt663QWindowIcon".to_string());
        store.clear_pending_unavailable("Qt663QWindowIcon");
        store.record_null_probe("Qt663QWindowIcon".to_string());
        assert_eq!(store.get("Qt663QWindowIcon"), None);
    }

    // 既に確定済みの class_name はカウントに影響されず安定した値を返す。
    #[test]
    fn already_confirmed_capability_is_stable() {
        let mut store = temp_store();
        store.record_null_probe("Qt663QWindowIcon".to_string());
        store.record_null_probe("Qt663QWindowIcon".to_string());
        store.record_null_probe("Qt663QWindowIcon".to_string());
        assert_eq!(
            store.get("Qt663QWindowIcon"),
            Some(ImmCapability::Unavailable)
        );
    }
}
