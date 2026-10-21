//! フォーカス層の学習ストア: IMM 能力(`ImmCapabilityStore`)と注入モード(`InjectionModeStore`)。
//!
//! `cache.toml` の `[imm_capability]` / `[injection_mode]` セクションを読み書きするだけの純粋なコードで、
//! Win32 を呼ばない。以前は `classifier.rs`(`#[cfg(windows)]`)にあったため、BUG-56/107/108 の回帰テストが
//! Linux のテストバイナリに存在しなかった。ここへ分けて Linux でも走らせる(ADR-272 / ECRS F3)。
//! 従来の `crate::focus::classifier::{ImmCapability, ImmCapabilityStore, InjectionModeStore}` の経路は
//! `classifier.rs` の再エクスポートが保つ。

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

// ── ImmCapabilityStore ──

/// IMM 能力の学習・永続化を担う構造体。
///
/// `base_dir` を外から隠蔽し、`learn()` 一発でキャッシュ更新とファイル保存を行う。
#[derive(Debug)]
pub struct ImmCapabilityStore {
    cache: ImmCapabilityCache,
    base_dir: std::path::PathBuf,
    /// `ImmGetDefaultIMEWnd`=NULL の連続観測回数（process_name/class_name ごと、未確定分のみ）。
    /// ディスクへは永続化しない — セッションをまたいで引き継ぐ必要はなく、
    /// 再起動のたびに空から積み直せば十分（BUG-56対策）。
    pending_unavailable: PendingImmUnavailable,
}

type ImmCapabilityCache =
    std::collections::HashMap<String, std::collections::HashMap<String, ImmCapability>>;
type PendingImmUnavailable =
    std::collections::HashMap<String, std::collections::HashMap<String, u32>>;

#[derive(Debug)]
struct LoadedImmCapabilityCache {
    cache: ImmCapabilityCache,
    ignored_legacy_entries: usize,
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
        let loaded = Self::load(&base_dir);
        let store = Self {
            cache: loaded.cache,
            base_dir,
            pending_unavailable: std::collections::HashMap::new(),
        };
        if loaded.ignored_legacy_entries > 0 {
            let _ = store.save();
        }
        store
    }

    pub(crate) fn get(&self, process_name: &str, class_name: &str) -> Option<ImmCapability> {
        self.cache
            .get(process_name)
            .and_then(|process_cache| process_cache.get(class_name))
            .copied()
    }

    pub(crate) fn learn(&mut self, process_name: String, class_name: String, cap: ImmCapability) {
        self.cache
            .entry(process_name)
            .or_default()
            .insert(class_name, cap);
        let _ = self.save();
    }

    /// 学習済みの IMM 能力（メモリ上のキャッシュと、`cache.toml` の `[imm_capability]`）と、
    /// 確定前の「疑い」カウントを全て捨てる。誤学習（BUG-56・BUG-107）の GUI 上の回復手段
    /// （トレイの「IME 制御の学習キャッシュをクリア」、BUG-108）。捨てた学習済みエントリ数と、
    /// `cache.toml` へ反映できたか（`false` なら再起動で旧エントリが戻る）を返す。
    ///
    /// `[injection_mode]` など `cache.toml` の他セクションと、学習表
    /// （`keymap-learn-table.json`）には触れない。ファイルが読めない・壊れている・書けない
    /// ときは `save_section` が上書きせず警告し、メモリだけが空になる（2つ目が `false`）。
    pub(crate) fn clear(&mut self) -> (usize, bool) {
        let removed = count_imm_capability_entries(&self.cache);
        self.cache.clear();
        self.pending_unavailable.clear();
        let persisted = self.save();
        (removed, persisted)
    }

    /// `ImmGetDefaultIMEWnd`=NULL の観測を記録する。閾値回連続で観測されて初めて
    /// `Unavailable` として確定・永続化する（`UNAVAILABLE_CONFIRM_THRESHOLD` 参照）。
    /// 呼び出し元（`learn_imm_capability_on_focus`）は既に学習済みの process/class を
    /// スキップ済みの前提。
    pub(crate) fn record_null_probe(&mut self, process_name: String, class_name: String) {
        let process_pending = self
            .pending_unavailable
            .entry(process_name.clone())
            .or_default();
        let count = process_pending.entry(class_name.clone()).or_insert(0);
        *count += 1;
        let confirmed = *count >= Self::UNAVAILABLE_CONFIRM_THRESHOLD;
        if confirmed {
            process_pending.remove(&class_name);
            if process_pending.is_empty() {
                self.pending_unavailable.remove(&process_name);
            }
            // 学習の経路をログに残す。もう一方の書き手（IME 検出ミス数の閾値超え）は
            // `Runtime::learn_imm_capability_from_miss` が `(miss a→b)` 付きで出す。
            // 降格ログ（`tracker.rs`）には経路が無いので、どちらの `IMM capability learned`
            // 行が先に出たかで、報告ログから経路を判別できるようにする。
            tracing::info!(
                "IMM capability learned: {process_name}/{class_name} → Unavailable \
                 (ImmGetDefaultIMEWnd=NULL を {} 回連続観測)",
                Self::UNAVAILABLE_CONFIRM_THRESHOLD
            );
            self.learn(process_name, class_name, ImmCapability::Unavailable);
        }
    }

    /// 非 NULL 観測（IMM32 が応答した）を得たら、その process/class の「疑い」カウントを
    /// クリアする。決め打ちの一時ウィンドウが NULL を返した直後に本物の入力欄が
    /// フォーカスされて non-NULL を返すケースで、疑いが誤って積み上がらないようにする。
    pub(crate) fn clear_pending_unavailable(&mut self, process_name: &str, class_name: &str) {
        let Some(process_pending) = self.pending_unavailable.get_mut(process_name) else {
            return;
        };
        process_pending.remove(class_name);
        if process_pending.is_empty() {
            self.pending_unavailable.remove(process_name);
        }
    }

    fn load(base_dir: &std::path::Path) -> LoadedImmCapabilityCache {
        let path = base_dir.join(CACHE_FILENAME);
        let Ok(content) = std::fs::read_to_string(&path) else {
            return LoadedImmCapabilityCache {
                cache: std::collections::HashMap::new(),
                ignored_legacy_entries: 0,
            };
        };
        let table: toml::Table = match content.parse() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Failed to parse {}: {e}", path.display());
                return LoadedImmCapabilityCache {
                    cache: std::collections::HashMap::new(),
                    ignored_legacy_entries: 0,
                };
            }
        };
        let mut cache = std::collections::HashMap::new();
        let mut ignored_legacy_entries = 0usize;
        if let Some(toml::Value::Table(section)) = table.get("imm_capability") {
            for (process_name, value) in section {
                match value {
                    toml::Value::Table(process_section) => {
                        for (class_name, value) in process_section {
                            if let toml::Value::String(s) = value {
                                let cap = match s.as_str() {
                                    "works" => ImmCapability::Works,
                                    "unavailable" | "broken" => ImmCapability::Unavailable,
                                    _ => continue,
                                };
                                cache
                                    .entry(process_name.clone())
                                    .or_insert_with(std::collections::HashMap::new)
                                    .insert(class_name.clone(), cap);
                            }
                        }
                    }
                    toml::Value::String(_) => {
                        ignored_legacy_entries += 1;
                    }
                    _ => {}
                }
            }
        }
        if ignored_legacy_entries > 0 {
            tracing::warn!(
                "Ignored {ignored_legacy_entries} legacy flat IMM capability cache entries from {} \
                 (expected [imm_capability.\"process.exe\"] class = value)",
                path.display()
            );
        }
        let entry_count = count_imm_capability_entries(&cache);
        if entry_count > 0 {
            tracing::info!(
                "Loaded IMM capability cache: {} entries from {}",
                entry_count,
                path.display()
            );
        }
        LoadedImmCapabilityCache {
            cache,
            ignored_legacy_entries,
        }
    }

    fn save(&self) -> bool {
        let mut section = toml::Table::new();
        for (process_name, process_cache) in &self.cache {
            let mut process_section = toml::Table::new();
            for (class_name, cap) in process_cache {
                let value = match cap {
                    ImmCapability::Works => "works",
                    ImmCapability::Unavailable => "unavailable",
                };
                process_section.insert(class_name.clone(), toml::Value::String(value.to_string()));
            }
            section.insert(process_name.clone(), toml::Value::Table(process_section));
        }
        let saved = save_section(&self.base_dir, "imm_capability", section);
        if saved {
            tracing::debug!(
                "Saved IMM capability cache: {} entries",
                count_imm_capability_entries(&self.cache)
            );
        }
        saved
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        count_imm_capability_entries(&self.cache)
    }
}

fn count_imm_capability_entries(cache: &ImmCapabilityCache) -> usize {
    cache.values().map(std::collections::HashMap::len).sum()
}

// ── キャッシュファイル共通 write ヘルパー ──────────────────────────────────────

/// `cache.toml` の指定セクションだけを更新し、他のセクションを保持して上書き保存する。
fn save_section(base_dir: &std::path::Path, section_name: &str, section: toml::Table) -> bool {
    let path = base_dir.join(CACHE_FILENAME);
    // 既存ファイルがあるのに読めない・パースできない場合は、他セクションを消さないよう上書きしない。
    let mut root: toml::Table = match std::fs::read_to_string(&path) {
        Ok(c) => match c.parse() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    "Skip saving [{section_name}]: {} is not valid TOML: {e}",
                    path.display()
                );
                return false;
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
        Err(e) => {
            tracing::warn!(
                "Skip saving [{section_name}]: cannot read {}: {e}",
                path.display()
            );
            return false;
        }
    };
    root.insert(section_name.to_string(), toml::Value::Table(section));
    let content = match toml::to_string_pretty(&root) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to serialize cache for {}: {e}", path.display());
            return false;
        }
    };
    // 書き込み途中の中断で cache.toml が壊れ、次回保存で他セクションが消えるのを防ぐ。
    if let Err(e) = awase::fs_atomic::write_atomic(&path, content.as_bytes()) {
        tracing::warn!("Failed to save cache to {}: {e}", path.display());
        return false;
    }
    true
}

// ── InjectionModeStore ────────────────────────────────────────────────────────

/// 事後昇格学習ストア（読み取り専用）：`cache.toml` の `[injection_mode]` セクション
/// （`class_name = "tsf"` の形式）に既に学習済みの class_name を読む。
/// 事後昇格の書き込み側は ADR-276 で撤去した（既存の利用者の学習結果を壊さないため読み取りは残す）。
/// `ForceOverrides::injection_hint()` が未マッチのとき、このストアを参照して `ForceTsf` を返す。
#[derive(Debug)]
pub struct InjectionModeStore {
    tsf_classes: std::collections::HashSet<String>,
}

impl InjectionModeStore {
    pub(crate) fn new(base_dir: std::path::PathBuf) -> Self {
        let tsf_classes = Self::load(&base_dir);
        Self { tsf_classes }
    }

    /// class_name が Tsf モード必要と学習済みかどうか。
    pub(crate) fn has_tsf(&self, class_name: &str) -> bool {
        self.tsf_classes.contains(class_name)
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
            tracing::info!(
                "Loaded injection mode cache: {} TSF classes from {}",
                classes.len(),
                path.display()
            );
        }
        classes
    }

    }
}

#[cfg(test)]
mod imm_capability_store_tests {
    use super::{ImmCapability, ImmCapabilityStore, CACHE_FILENAME};

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
        ImmCapabilityStore::new(temp_dir())
    }

    fn temp_dir() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "awase_imm_capability_store_test_{}_{n}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir for ImmCapabilityStore test");
        dir
    }

    // BUG-56: 単発の NULL 観測だけでは Unavailable を確定しない。
    #[test]
    fn single_null_probe_does_not_confirm_unavailable() {
        let mut store = temp_store();
        store.record_null_probe("line.exe".to_string(), "Qt663QWindowIcon".to_string());
        assert_eq!(store.get("line.exe", "Qt663QWindowIcon"), None);
    }

    // BUG-56: 閾値回（2回）連続で NULL を観測して初めて Unavailable が確定する。
    #[test]
    fn two_consecutive_null_probes_confirm_unavailable() {
        let mut store = temp_store();
        store.record_null_probe("line.exe".to_string(), "Qt663QWindowIcon".to_string());
        store.record_null_probe("line.exe".to_string(), "Qt663QWindowIcon".to_string());
        assert_eq!(
            store.get("line.exe", "Qt663QWindowIcon"),
            Some(ImmCapability::Unavailable)
        );
    }

    // BUG-56: 途中で非 NULL 観測（本物の入力欄が応答した）が挟まると疑いカウントが
    // リセットされ、次の NULL 単発では確定しない。
    #[test]
    fn non_null_observation_resets_pending_count() {
        let mut store = temp_store();
        store.record_null_probe("line.exe".to_string(), "Qt663QWindowIcon".to_string());
        store.clear_pending_unavailable("line.exe", "Qt663QWindowIcon");
        store.record_null_probe("line.exe".to_string(), "Qt663QWindowIcon".to_string());
        assert_eq!(store.get("line.exe", "Qt663QWindowIcon"), None);
    }

    // 既に確定済みの class_name はカウントに影響されず安定した値を返す。
    #[test]
    fn already_confirmed_capability_is_stable() {
        let mut store = temp_store();
        store.record_null_probe("line.exe".to_string(), "Qt663QWindowIcon".to_string());
        store.record_null_probe("line.exe".to_string(), "Qt663QWindowIcon".to_string());
        store.record_null_probe("line.exe".to_string(), "Qt663QWindowIcon".to_string());
        assert_eq!(
            store.get("line.exe", "Qt663QWindowIcon"),
            Some(ImmCapability::Unavailable)
        );
    }

    // BUG-107: winit の既定クラス名など、複数プロセスで共有される class_name は
    // process_name 込みで独立して学習・参照する。
    #[test]
    fn same_class_name_is_isolated_by_process_name() {
        let mut store = temp_store();
        store.learn(
            "awase-settings.exe".to_string(),
            "Window Class".to_string(),
            ImmCapability::Works,
        );
        store.learn(
            "some-other-app.exe".to_string(),
            "Window Class".to_string(),
            ImmCapability::Unavailable,
        );

        assert_eq!(
            store.get("awase-settings.exe", "Window Class"),
            Some(ImmCapability::Works)
        );
        assert_eq!(
            store.get("some-other-app.exe", "Window Class"),
            Some(ImmCapability::Unavailable)
        );
    }

    /// BUG-108: クリアは `[imm_capability]` だけを空にし、`[injection_mode]` 等の他セクションは残す。
    /// メモリ上のキャッシュも空になり、再読み込みしても復活しない。
    #[test]
    fn clear_empties_imm_capability_section_and_keeps_other_sections() {
        let dir = temp_dir();
        std::fs::write(
            dir.join(CACHE_FILENAME),
            "[injection_mode]\n\"Some.Class\" = \"tsf\"\n",
        )
        .expect("write cache.toml");
        let mut store = ImmCapabilityStore::new(dir.clone());
        store.learn(
            "a.exe".to_string(),
            "Cls".to_string(),
            ImmCapability::Unavailable,
        );
        store.learn("b.exe".to_string(), "Cls".to_string(), ImmCapability::Works);
        store.record_null_probe("c.exe".to_string(), "Cls".to_string());
        assert_eq!(store.len(), 2);

        assert_eq!(store.clear(), (2, true));
        assert_eq!(store.len(), 0);
        assert_eq!(store.get("a.exe", "Cls"), None);
        // 確定前の疑いも消える: クリア後の1回の NULL 観測では確定しない（閾値は2回）。
        store.record_null_probe("c.exe".to_string(), "Cls".to_string());
        assert_eq!(store.get("c.exe", "Cls"), None);

        let content = std::fs::read_to_string(dir.join(CACHE_FILENAME)).expect("read cache.toml");
        let table: toml::Table = content.parse().expect("parse cache.toml");
        let imm = table.get("imm_capability").and_then(toml::Value::as_table);
        assert!(imm.is_none_or(toml::Table::is_empty), "{content}");
        assert!(
            table.contains_key("injection_mode"),
            "他セクションが消えた: {content}"
        );
        assert_eq!(
            ImmCapabilityStore::new(dir).len(),
            0,
            "再読み込みで復活した"
        );
    }

    /// cache.toml が壊れていて書き戻せないとき、クリアは「反映できなかった」を返す
    /// （メモリだけ空、ファイルは他セクションごと無傷）。
    #[test]
    fn clear_reports_failure_when_cache_toml_is_unwritable() {
        let dir = temp_dir();
        let mut store = ImmCapabilityStore::new(dir.clone());
        store.learn("a.exe".to_string(), "Cls".to_string(), ImmCapability::Works);
        std::fs::write(dir.join(CACHE_FILENAME), "this is = not [valid toml")
            .expect("corrupt cache.toml");

        assert_eq!(store.clear(), (1, false));
        assert_eq!(store.len(), 0);
        let content = std::fs::read_to_string(dir.join(CACHE_FILENAME)).expect("read cache.toml");
        assert_eq!(
            content, "this is = not [valid toml",
            "壊れたファイルを上書きした"
        );
    }

    #[test]
    fn save_and_load_nested_process_table_format() {
        let dir = temp_dir();
        let mut store = ImmCapabilityStore::new(dir.clone());
        store.learn(
            "awase-settings.exe".to_string(),
            "Window Class".to_string(),
            ImmCapability::Works,
        );
        store.learn(
            "some-other-app.exe".to_string(),
            "Window Class".to_string(),
            ImmCapability::Unavailable,
        );

        let content = std::fs::read_to_string(dir.join(CACHE_FILENAME)).expect("read cache.toml");
        assert!(content.contains("awase-settings.exe"));
        assert!(content.contains("some-other-app.exe"));

        let reloaded = ImmCapabilityStore::new(dir);
        assert_eq!(
            reloaded.get("awase-settings.exe", "Window Class"),
            Some(ImmCapability::Works)
        );
        assert_eq!(
            reloaded.get("some-other-app.exe", "Window Class"),
            Some(ImmCapability::Unavailable)
        );
    }

    #[test]
    fn load_ignores_legacy_flat_class_entries() {
        let dir = temp_dir();
        std::fs::write(
            dir.join(CACHE_FILENAME),
            r#"
[imm_capability]
"Window Class" = "unavailable"

[imm_capability."awase-settings.exe"]
"Window Class" = "works"
"#,
        )
        .expect("write cache.toml");

        let store = ImmCapabilityStore::new(dir.clone());
        assert_eq!(store.len(), 1);
        assert_eq!(store.get("Window Class", "Window Class"), None);
        assert_eq!(store.get("", "Window Class"), None);
        assert_eq!(
            store.get("awase-settings.exe", "Window Class"),
            Some(ImmCapability::Works)
        );

        let content = std::fs::read_to_string(dir.join(CACHE_FILENAME)).expect("read cache.toml");
        let table: toml::Table = content.parse().expect("parse rewritten cache.toml");
        let Some(toml::Value::Table(section)) = table.get("imm_capability") else {
            panic!("missing imm_capability section");
        };
        assert!(
            section
                .values()
                .all(|value| matches!(value, toml::Value::Table(_))),
            "legacy flat string entries should be removed on load"
        );
    }
}
