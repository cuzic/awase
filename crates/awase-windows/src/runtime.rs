use awase::engine::{AssumedReason, Engine, EngineCommand, InputContext, InputModeState, TIMER_PENDING, TIMER_SPECULATIVE};
use awase::platform::PlatformRuntime;
use awase::types::{ContextChange, FocusKind, RawKeyEvent, ShadowImeAction, VkCode};

use crate::focus::cache::DetectionSource;
use crate::Preconditions;

/// Config の force_text / force_bypass オーバーライドをチェックする。
/// マッチした場合は強制される FocusKind を返す。
fn check_app_override(
    overrides: &awase::config::AppOverrides,
    process_id: u32,
    class_name: &str,
) -> Option<FocusKind> {
    if overrides.force_text.is_empty() && overrides.force_bypass.is_empty() {
        return None;
    }
    let process_name = crate::focus::classify::get_process_name(process_id);
    for entry in &overrides.force_text {
        if entry.process.eq_ignore_ascii_case(&process_name)
            && entry.class.eq_ignore_ascii_case(class_name)
        {
            return Some(FocusKind::TextInput);
        }
    }
    for entry in &overrides.force_bypass {
        if entry.process.eq_ignore_ascii_case(&process_name)
            && entry.class.eq_ignore_ascii_case(class_name)
        {
            return Some(FocusKind::NonText);
        }
    }
    None
}

/// Config の `force_vk` オーバーライドに現在のフォーカス先がマッチするか判定する。
///
/// `force_vk` が空なら Win32 API を呼ばずに即 false を返す (fast path)。
/// マッチは `process` と `class` の両方が大文字小文字を無視して一致したとき。
pub fn is_force_vk(
    overrides: &awase::config::AppOverrides,
    process_id: u32,
    class_name: &str,
) -> bool {
    if overrides.force_vk.is_empty() {
        return false;
    }
    let process_name = crate::focus::classify::get_process_name(process_id);
    overrides.force_vk.iter().any(|entry| {
        entry.process.eq_ignore_ascii_case(&process_name)
            && entry.class.eq_ignore_ascii_case(class_name)
    })
}

/// Config の `force_tsf` オーバーライドに現在のフォーカス先がマッチするか判定する。
///
/// `force_tsf` が空なら Win32 API を呼ばずに即 false を返す (fast path)。
/// マッチは `process` と `class` の両方が大文字小文字を無視して一致したとき。
///
/// `Windows.UI.Input.InputSite.WindowClass` がフォーカスを持つ場合（WezTerm 等の
/// TSF ネイティブ子ウィンドウ）、`GetForegroundWindow()` でトップレベルクラスを
/// 取得して再マッチを試みる。これにより force_tsf 設定が InputSite フォーカス時にも
/// 正しく動作する。
pub fn is_force_tsf(
    overrides: &awase::config::AppOverrides,
    process_id: u32,
    class_name: &str,
) -> bool {
    if overrides.force_tsf.is_empty() {
        return false;
    }
    let process_name = crate::focus::classify::get_process_name(process_id);
    if overrides.force_tsf.iter().any(|entry| {
        entry.process.eq_ignore_ascii_case(&process_name)
            && entry.class.eq_ignore_ascii_case(class_name)
    }) {
        return true;
    }
    // InputSite は WezTerm 等の TSF ネイティブ子ウィンドウ。GetForegroundWindow()
    // はトップレベルウィンドウ（org.wezfurlong.wezterm 等）を返すため、そのクラスで
    // 再マッチすることで force_tsf 設定が InputSite フォーカス時にも機能する。
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

/// `Preconditions` から `InputContext` を構築する。
///
/// 修飾キー判定は `GetAsyncKeyState` で取得した OS 実状態のみ使用する。
pub fn build_input_context(preconditions: &Preconditions) -> InputContext {
    let raw = unsafe { crate::observer::focus_observer::read_os_modifiers() };
    let modifiers = awase::engine::ModifierState {
        ctrl: raw.ctrl,
        alt: raw.alt,
        shift: raw.shift,
        win: raw.win,
    };
    InputContext {
        ime_on: preconditions.ime_on,
        input_mode: preconditions.input_mode,
        is_japanese_ime: preconditions.is_japanese_ime,
        modifiers,
        left_thumb_down: None,
        right_thumb_down: None,
    }
}
use awase::yab::YabLayout;

use crate::executor::DecisionExecutor;
use crate::hook::CallbackResult;

// ── LayoutEntry（名前付きレイアウトエントリ）──

/// レイアウト設定一式を保持する構造体
#[derive(Debug)]
pub struct LayoutEntry {
    pub name: String,
    pub layout: YabLayout,
}

// ── AppKindClassifier（フォーカス検出状態）──

/// IMM ブリッジの検出結果（class_name ごとにキャッシュ）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImmCapability {
    /// IMM ブリッジが動作する（ImmGetOpenStatus が信頼できる値を返す）
    /// → Unicode 直接入力で OK
    Works,
    /// IMM ブリッジが動作しない（独自 TSF text store を持つアプリ）
    /// → PerKey (VK injection) が必要
    Broken,
}

/// IMM 能力キャッシュファイル名（config.toml と同じディレクトリ）
const IMM_CACHE_FILENAME: &str = "imm_cache.toml";

/// IMM 能力キャッシュをファイルから読み込む。
/// ファイルが存在しない場合は空の HashMap を返す。
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

/// フォーカス切り替え時の IME 状態スナップショット（per-HWND キャッシュ用）
#[derive(Debug, Clone, Copy)]
pub struct HwndImeSnapshot {
    pub ime_on: bool,
    pub input_mode: InputModeState,
    /// 記録時刻（GetTickCount64 ミリ秒）
    pub recorded_ms: u64,
}

/// フォーカス検出に関するシングルスレッド状態を集約する構造体
#[allow(missing_debug_implementations)]
pub struct AppKindClassifier {
    pub cache: crate::focus::cache::FocusCache,
    pub overrides: awase::config::AppOverrides,
    pub last_focus_info: Option<(u32, String)>,
    pub uia_sender: Option<std::sync::mpsc::Sender<crate::focus::uia::SendableHwnd>>,
    /// class_name ごとの IMM ブリッジ能力キャッシュ。
    /// 検出成功/失敗の実績に基づいて学習し、AppKind 判定に使う。
    /// ファイルに永続化される（起動時ロード、学習時セーブ）。
    pub imm_capability_cache: std::collections::HashMap<String, ImmCapability>,
    /// per-HWND IME 状態キャッシュ。
    ///
    /// キー: `(process_id, class_name)` — HWND は再利用されるため class_name を合わせる。
    /// 値: フォーカスが離れた時点の IME 状態スナップショット。
    /// フォーカスが戻ったとき preconditions を即座に復元し、stale 窓をゼロにする。
    /// probe / poll が成功すれば自動的に上書き補正される。
    pub hwnd_ime_cache: std::collections::HashMap<(u32, String), HwndImeSnapshot>,
    /// キャッシュファイルの格納ディレクトリ（実行ファイルと同じ場所）
    base_dir: std::path::PathBuf,
}

impl AppKindClassifier {
    pub fn new(overrides: awase::config::AppOverrides) -> Self {
        let base_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let imm_capability_cache = load_imm_cache(&base_dir);
        Self {
            cache: crate::focus::cache::FocusCache::new(),
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
        sender: std::sync::mpsc::Sender<crate::focus::uia::SendableHwnd>,
    ) {
        self.uia_sender = Some(sender);
    }
}

/// アプリケーションランタイム。
///
/// Engine (判断) と DecisionExecutor (実行) を保持し、配線する。
/// OS イベントの受け取り → Observer → Engine → Executor のパイプラインを駆動する。
///
/// 注意: 判断ロジックを追加しないこと。判断は Engine が担う。
#[allow(missing_debug_implementations)]
pub struct Runtime {
    pub engine: Engine,
    pub executor: DecisionExecutor,
    pub layouts: Vec<LayoutEntry>,
    /// IME 同期キー（イベント事前分類用）
    pub sync_toggle_keys: Vec<VkCode>,
    pub sync_on_keys: Vec<VkCode>,
    pub sync_off_keys: Vec<VkCode>,
    /// Platform 層の全状態
    pub platform_state: crate::PlatformState,
}

impl Runtime {
    fn build_ctx(&self) -> InputContext {
        build_input_context(&self.platform_state.preconditions)
    }

    /// IME 関連の事前分類情報を sync key 設定で補完する
    pub fn enrich_ime_relevance(&self, event: &mut RawKeyEvent) {
        let vk = event.vk_code;
        let rel = &mut event.ime_relevance;

        if self.sync_toggle_keys.contains(&vk) {
            rel.is_sync_key = true;
            rel.sync_direction = Some(ShadowImeAction::Toggle);
            rel.may_change_ime = true;
        } else if self.sync_on_keys.contains(&vk) {
            rel.is_sync_key = true;
            rel.sync_direction = Some(ShadowImeAction::TurnOn);
            rel.may_change_ime = true;
        } else if self.sync_off_keys.contains(&vk) {
            rel.is_sync_key = true;
            rel.sync_direction = Some(ShadowImeAction::TurnOff);
            rel.may_change_ime = true;
        }
    }

    /// Decision の副作用を実行する（メッセージループ用）。
    pub fn execute_decision(&mut self, decision: awase::engine::Decision) -> CallbackResult {
        self.executor.execute_from_loop(decision)
    }

    /// エンジンの有効/無効を切り替え、Decision を実行する
    pub fn toggle_engine(&mut self) {
        let ctx = self.build_ctx();
        let decision = self.engine.on_command(EngineCommand::ToggleEngine, &ctx);
        self.executor.execute_from_loop(decision);
    }

    /// 外部コンテキスト喪失時にエンジンの保留状態を安全にフラッシュする。
    pub fn invalidate_engine_context(&mut self, reason: ContextChange) {
        let ctx = self.build_ctx();
        let decision = self
            .engine
            .on_command(EngineCommand::InvalidateContext(reason), &ctx);
        self.executor.execute_from_loop(decision);
    }

    /// IME 状態とフォーカス状態を一括で再観測し、Engine に通知する。
    ///
    /// フォーカスデバウンス後・500ms ポーリング・may_change_ime 後など、
    /// 全ての IME/フォーカス更新がこのメソッドに集約される（ADR 028）。
    ///
    /// 処理フロー:
    /// 1. 現在のフォーカス先を取得・分類（focus_kind, app_kind 更新）
    /// 2. 前面プロセスが変わった場合は Engine に FocusChanged（flush あり）
    /// 3. IME 状態を再取得して Preconditions を更新
    /// 4. Engine に RefreshState（active 状態の遷移検知）
    /// 5. 次回ポーリングを自動スケジュール
    ///
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub fn refresh_ime_state_cache(&mut self) {
        // ── Phase 1: フォーカス先の検出・分類 ──
        let focus_changed = unsafe { self.detect_and_update_focus() };

        // ── Phase 2.5: IMM ブリッジ非対応クラスの判定 ──
        //
        // Chrome / UWP / Electron 等はクロスプロセス IMM 問い合わせ（WM_IME_CONTROL）が
        // 動作しないか、無期限ブロックする恐れがある。既知のクラス名なら事前にスキップし、
        // シャドウ状態（hook から追跡）のみで IME 状態を管理する。
        //
        // この分岐の場合、言語バーのマウス操作等による OS 側の IME 変更は検知不能だが、
        // ハードウェアキー押下（半角/全角等）は hook でシャドウが更新されるため実用上問題ない。
        //
        // Phase 2 の FocusChanged より前に計算する必要がある。
        // FocusChanged で build_ctx() が呼ばれる際、input_mode が stale な ObservedKana だと
        // engine が inactive になってしまうため、先に補正する。
        let skip_imm_query = self
            .executor
            .platform
            .focus
            .last_focus_info
            .as_ref()
            .map_or(false, |(_, class_name)| {
                crate::focus::classify::is_imm_bridge_broken(class_name)
            });

        // ── Phase 2: プロセス変更時は Engine に FocusChanged（flush あり）──
        if focus_changed {
            // IMM broken アプリ（Chrome 等）に切り替わった際に input_mode が
            // 前ウィンドウの stale な ObservedKana を引き継いでいると、FocusChanged の ctx で
            // engine が inactive になる。broken アプリでは入力モードを検出できないため、
            // ime_on=true のとき AssumedRomaji と仮定して補正する。
            if skip_imm_query
                && self.platform_state.preconditions.ime_on
                && !self.platform_state.preconditions.input_mode.is_romaji_capable()
            {
                log::info!(
                    "FocusChanged: input_mode assumed romaji (IMM broken, stale kana from prev window)"
                );
                self.platform_state.preconditions.input_mode =
                    InputModeState::AssumedRomaji { reason: AssumedReason::ImmBridgeBroken };
            }
            let ctx = self.build_ctx();
            let decision = self.engine.on_command(EngineCommand::FocusChanged, &ctx);
            self.executor.execute_from_loop(decision);
        }

        // ── Phase 2.7: 入力中ガード ──
        //
        // 最後のキー活動（物理キー押下 または VK/TSF 出力）から TYPING_IDLE_MS 以内は
        // IMM との SendMessage を一切行わない。
        //
        // 【目的 A：Win32 通常アプリ】
        //   typing 中に IME 状態を読んでも意味が薄い。緊急の IME 変化は
        //   may_change_ime で即時リフレッシュされるので、アイドル後の
        //   定期読み取りで十分。
        //
        // 【目的 B：Chrome/Edge 系】
        //   `set_ime_open` が VK バッチ（T↓E↓T↑E↑）の処理途中に割り込むと
        //   Chrome の composition がリセットされ「て→tえ」の母音落ちが起きる。
        //   typing 中は IME を閉じることもないのでポーリング不要。
        //
        // `last_hook_activity_ms` は物理キーで hook から、VK/TSF 出力後は
        // `Output::mark_vk_output()` で同期的に更新される。
        // shadow は hook 経由で常時更新されているので、1 cycle の遅延は実害なし。
        let idle_ms = crate::hook::current_tick_ms()
            .saturating_sub(self.platform_state.last_hook_activity_ms);
        let is_typing = idle_ms < crate::timing::TYPING_IDLE_MS;

        if is_typing {
            log::debug!("Skipping observer/SSOT write: typing active (idle={idle_ms}ms)");
        } else if skip_imm_query {
            // ── ブラックリストクラス: OS 読み取りをスキップ ──
            // preconditions.ime_on はシャドウ更新 (hook 経由) が直接書き換える。
            // miss_count はインクリメントしない（既知の失敗なので「検出失敗」ではない）。
            //
            // 書き込みは「shadow が ON のときだけ」に限定する (ADR 029 の force-ON 原則)。
            // shadow=OFF のときに書き込むと、ユーザーが OS 経由 (言語バー、OS ショートカット等)
            // で意図的に OFF にした瞬間を awase が毎サイクル上書きしてしまう。
            // 「言語バー ON → awase 意図の OFF に戻す」ユースケースは諦める代わりに、
            // ユーザーの明示的な OFF が絶対に効くことを優先する。
            log::debug!("Skipping IMM query for known-broken class (shadow state SSOT)");
            if self.engine.is_user_enabled()
                && self.platform_state.preconditions.is_japanese_ime
                && self.platform_state.preconditions.ime_on
            {
                let _success = self.executor.platform.set_ime_open(true);
                log::trace!("Blacklist SSOT write: ime_on=true (force-ON only)");
                // input_mode も SSOT として維持: IMM broken アプリでは検出不能のため
                // stale な ObservedKana が engine を無効化しないよう AssumedRomaji に補正する。
                if !self.platform_state.preconditions.input_mode.is_romaji_capable() {
                    log::info!("Blacklist SSOT: input_mode → AssumedRomaji (IMM broken, ime_on=true)");
                    self.platform_state.preconditions.input_mode =
                        InputModeState::AssumedRomaji { reason: AssumedReason::ImmBridgeBroken };
                }
            }
        } else {
            // ── Phase 3: IME 状態の再取得 ──
            let miss_before = self.platform_state.preconditions.ime_detect_miss_count;
            // [診断] observe 前のスナップショット（差分検出用）
            let ime_on_before_poll = self.platform_state.preconditions.ime_on;
            let input_mode_before_poll = self.platform_state.preconditions.input_mode;
            unsafe {
                crate::observer::ime_observer::observe(
                    &mut self.platform_state.preconditions,
                    &mut self.platform_state.ime_observations,
                );
            }
            let miss_after = self.platform_state.preconditions.ime_detect_miss_count;
            // observe() の生の結果を os_ime_on に記録（miss なし＝成功時のみ更新）
            // observer_poll から読む（resolve 前の生値）
            if miss_after == 0 {
                if let Some(obs) = &self.platform_state.ime_observations.observer_poll {
                    self.platform_state.os_ime_on =
                        Some(obs.value && self.platform_state.preconditions.is_japanese_ime);
                }
            }
            // observer_poll → preconditions.ime_on に優先度付き解決
            self.platform_state.apply_ime_observations(self.engine.is_user_enabled());

            // [診断] フォーカス変更から 10 秒以内で状態が変わった場合にログ出力。
            // - ime_on / input_mode のどちらが stale だったか
            // - フォーカス変更からどれだけ後に正しい値に戻ったか
            // これが安定して速ければアプローチ A（即時再プローブ）で十分。
            // None が多い / 遅い場合はアプローチ B（per-HWND キャッシュ）を検討。
            let age_ms = crate::hook::current_tick_ms()
                .saturating_sub(self.platform_state.last_focus_change_ms);
            if age_ms < 10_000 {
                let ime_on_after = self.platform_state.preconditions.ime_on;
                let input_mode_after = self.platform_state.preconditions.input_mode;
                let ime_changed = ime_on_before_poll != ime_on_after;
                let mode_changed = input_mode_before_poll != input_mode_after;
                if ime_changed || mode_changed {
                    log::info!(
                        "ObserverPoll +{}ms since focus: {}{}",
                        age_ms,
                        if ime_changed {
                            format!(
                                "ime_on {} → {}({:?}) ",
                                ime_on_before_poll,
                                ime_on_after,
                                self.platform_state.preconditions.ime_on_source,
                            )
                        } else {
                            String::new()
                        },
                        if mode_changed {
                            format!("mode {:?} → {:?}", input_mode_before_poll, input_mode_after)
                        } else {
                            String::new()
                        },
                    );
                } else if miss_after > 0 {
                    log::debug!(
                        "ObserverPoll +{}ms since focus: detection failed (miss={}), stale ime_on={} mode={:?}",
                        age_ms,
                        miss_after,
                        ime_on_before_poll,
                        input_mode_before_poll,
                    );
                }
            }

            // ── Phase 3.1: IMM 能力の学習 ──
            // 検出結果に基づいて class_name ごとの IMM 能力をキャッシュ。
            // 検出成功 (miss_count がリセット) → IMM Works
            // 検出連続失敗 (閾値到達) → IMM Broken → 次回から Chrome 扱い
            if let Some((_, class_name)) = self.executor.platform.focus.last_focus_info.as_ref() {
                let class_name = class_name.clone();
                if miss_after == 0 && miss_before > 0 {
                    // 検出成功: IMM ブリッジが動作している
                    let prev = self.executor.platform.focus.imm_capability_cache.get(&class_name);
                    if prev != Some(&ImmCapability::Works) {
                        log::info!("IMM capability learned: {class_name} → Works (detection succeeded)");
                        self.executor.platform.focus
                            .learn_imm_capability(class_name, ImmCapability::Works);
                    }
                } else if miss_after >= crate::IME_DETECT_MISS_THRESHOLD
                    && miss_before < crate::IME_DETECT_MISS_THRESHOLD
                {
                    // 閾値到達: IMM ブリッジが壊れている
                    let prev = self.executor.platform.focus.imm_capability_cache.get(&class_name);
                    if prev != Some(&ImmCapability::Broken) {
                        log::info!(
                            "IMM capability learned: {class_name} → Broken (detection failed {} times)",
                            miss_after
                        );
                        self.executor.platform.focus
                            .learn_imm_capability(class_name.clone(), ImmCapability::Broken);
                    }
                }
            }

            // ── Phase 3.5: 未知 IMM-broken アプリ向け一時 force-ON（初回ブートストラップ）──
            //
            // ここに来るのは「既知でも TSF-native でもないアプリで detect が連続失敗した」
            // 場合だけ。Chrome・WezTerm・Windows Terminal 等の既知アプリでは
            // この分岐に入らない（skip_imm_query または is_tsf_native が先に処理する）。
            //
            // shadow=ON なら「ユーザーは日本語入力したい」と解釈して SetOpen(true) を呼び、
            // engine を active のまま維持する (ADR 029)。shadow=OFF のときは書き込まない
            // （ユーザーの明示的な OFF を上書きしないため）。
            //
            // force-ON 後は ime_force_on_guard を立てて次の observe() による上書きを1回防ぐ。
            // 閾値到達時に ImmCapability::Broken と学習されるため、以降は skip_imm_query
            // 経由の Blacklist SSOT パスに移行し、この分岐には来なくなる（一過性の処理）。
            if self.platform_state.preconditions.ime_detect_miss_count
                >= crate::IME_DETECT_MISS_THRESHOLD
                && self.engine.is_user_enabled()
                && self.platform_state.preconditions.is_japanese_ime
                && self.platform_state.preconditions.ime_on
                && !self.platform_state.preconditions.ime_force_on_guard
            {
                log::warn!(
                    "IME detection failed {} times, forcing OS ime_on=true (shadow=ON)",
                    self.platform_state.preconditions.ime_detect_miss_count
                );
                let success = self.executor.platform.set_ime_open(true);
                if success {
                    self.platform_state.preconditions.ime_force_on_guard = true;
                    // miss_count はリセットしない。ガードが検出成功まで保護する。
                }
            }
        }

        // ── Phase 3.7: 診断スナップショット ──
        //
        // フォーカス変更が確定した直後の IME 状態を 1 行ログに吐き出す。
        // ウィンドウ切替直後の cold-start 不具合を解析するための観測点。
        if focus_changed {
            crate::ime_diagnostic::ImeDiagnosticSnapshot::capture("focus_changed").log();
            // フォーカス変更時は VK/TSF いずれも composition context が無効化される。
            log::debug!("[composition] focus change → marking cold");
            // shadow_ime_on を最新の IME 状態に同期してから warmup 判定を行う。
            self.executor.platform.output.notify_ime_open(self.platform_state.preconditions.ime_on);
            self.executor.platform.output.mark_composition_cold(crate::output::ColdReason::FocusChange);

            // TSF モード（WezTerm 等）かつ IME ON の場合、FocusChange 直後に F2 pre-warmup を送信する。
            // send_eager_tsf_warmup() は shadow_ime_on && is_tsf_mode を内部チェックする。
            self.executor.platform.output.send_eager_tsf_warmup();
            log::debug!("[composition] FocusChange: send_eager_tsf_warmup called (guarded by shadow_ime_on)");
        }

        // ── Phase 4: Engine に RefreshState（active 遷移検知）──
        let ctx = self.build_ctx();
        let decision = self.engine.on_command(EngineCommand::RefreshState, &ctx);
        self.executor.execute_from_loop(decision);

        // ── Phase 5: 次回ポーリングを自動スケジュール ──
        self.schedule_ime_refresh(u64::from(self.platform_state.ime_poll_interval_ms));
    }

    /// 現在のフォーカス先を検出し、focus_kind / app_kind を更新する。
    ///
    /// 前面プロセスが前回と異なる場合は `true` を返す（flush が必要）。
    /// 同一プロセス内のフォーカス移動では `false` を返す（flush 不要）。
    ///
    /// # Safety
    /// Win32 API を呼び出す。メインスレッドから呼ぶこと。
    unsafe fn detect_and_update_focus(&mut self) -> bool {
        use crate::focus::classify;

        // フォーカス検出全体をワーカースレッドでタイムアウト付き実行する。
        // 詳細は focus::probe::run_focus_probe() を参照。
        let probe = unsafe { crate::focus::probe::run_focus_probe() };

        let Some(probe) = probe else {
            log::warn!("Focus probe timed out — skipping update this cycle");
            return false;
        };

        if probe.process_id == 0 {
            return false;
        }
        let hwnd = probe.hwnd();
        let process_id = probe.process_id;
        let class_name = probe.class_name;

        // app_kind を更新
        let new_app_kind = crate::observer::focus_observer::detect_app_kind(&class_name);

        // IMM 能力キャッシュの初期学習（AppKind は変更しない）。
        // "IMM Broken" = IMM 状態クエリが信頼できない。VK 合成が必要とは限らない。
        // WezTerm 等は ImmGetDefaultIMEWnd=NULL でも WM_CHAR (Unicode) を正しく処理する。
        if new_app_kind == awase::types::AppKind::Win32 {
            if self.executor.platform.focus.imm_capability_cache.get(&class_name).is_none() {
                use windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
                let ime_wnd = unsafe { ImmGetDefaultIMEWnd(hwnd) };
                if ime_wnd.0.is_null() {
                    log::info!(
                        "IMM capability: ImmGetDefaultIMEWnd=NULL, learning Broken (class={class_name})"
                    );
                    self.executor.platform.focus
                        .learn_imm_capability(class_name.clone(), ImmCapability::Broken);
                }
            }
        }

        if self.platform_state.app_kind != new_app_kind {
            log::info!("AppKind changed: {:?} → {:?} (class={class_name})", self.platform_state.app_kind, new_app_kind);
            self.platform_state.app_kind = new_app_kind;
        }

        // focus_kind を分類
        // Config オーバーライドをチェック
        let (kind, reason, overridden) = if let Some(kind) = check_app_override(&self.executor.platform.focus.overrides, process_id, &class_name) {
            (kind, "config override".to_string(), true)
        } else if let Some(cached) = self.executor.platform.focus.cache.get(process_id, &class_name) {
            (cached, "cache hit".to_string(), false)
        } else {
            // classify_focus は ImmGetContext / GetWindowLongW / MSAA 等の
            // ブロッキング API を呼び出すため、ワーカースレッドで実行する。
            // HWND は *mut c_void なので、usize に変換してスレッド間転送する。
            //
            // エンジンタイマーが動作中（ユーザー入力中）は classify_focus をスキップする。
            // UIA/MSAA の SendMessage が WezTerm 等の TSF コンポジションを破壊し、
            // VK バッチの直後に「ｋあ」のような出力化けが起きるのを防ぐ。
            let engine_timer_active = {
                let timer = &self.executor.platform.timer;
                timer.is_active(TIMER_PENDING) || timer.is_active(TIMER_SPECULATIVE)
            };
            if engine_timer_active {
                log::debug!("classify_focus skipped: engine timer active (user typing)");
                (FocusKind::Undetermined, "skipped (engine active)".to_string(), false)
            } else {
                let hwnd_addr = hwnd.0 as usize;
                let classify_result = crate::win32::run_with_timeout(
                    std::time::Duration::from_millis(300),
                    move || {
                        let hwnd = windows::Win32::Foundation::HWND(hwnd_addr as *mut _);
                        classify::classify_focus(hwnd)
                    },
                );
                match classify_result {
                    Some(result) => (result.kind, format!("{}", result.reason), false),
                    None => {
                        log::warn!("classify_focus timed out for hwnd={:?}", hwnd);
                        (FocusKind::Undetermined, "classify timeout".to_string(), false)
                    }
                }
            }
        };

        // focus_kind を更新
        if self.platform_state.focus_kind != kind {
            log::debug!("Focus kind changed: {:?} → {kind:?} (reason={reason})", self.platform_state.focus_kind);
            self.platform_state.focus_kind = kind;
        }

        // キャッシュ格納（オーバーライドでない場合のみ）
        if !overridden {
            self.executor.platform.focus.cache.insert(
                process_id,
                class_name.clone(),
                kind,
                DetectionSource::Automatic,
            );
        }

        // 前面プロセスが変わったかチェック
        let last_pid = self.executor.platform.focus.last_focus_info.as_ref().map(|(pid, _)| *pid);
        let process_changed = last_pid.is_some_and(|last| last != process_id);

        // フォーカス離脱: 現在の preconditions を per-HWND キャッシュに保存
        // （last_focus_info 更新前に行う — 更新後は古い HWND の情報が消える）
        if process_changed {
            if let Some((old_pid, old_class)) = &self.executor.platform.focus.last_focus_info {
                let snapshot = HwndImeSnapshot {
                    ime_on: self.platform_state.preconditions.ime_on,
                    input_mode: self.platform_state.preconditions.input_mode,
                    recorded_ms: crate::hook::current_tick_ms(),
                };
                log::debug!(
                    "HwndCache: save [{} {}] ime_on={} mode={:?}",
                    old_pid, old_class, snapshot.ime_on, snapshot.input_mode,
                );
                let cache = &mut self.executor.platform.focus.hwnd_ime_cache;
                let now_ms = snapshot.recorded_ms;
                cache.retain(|_, v| now_ms.saturating_sub(v.recorded_ms) <= crate::timing::HWND_CACHE_MAX_AGE_MS);
                cache.insert((*old_pid, old_class.clone()), snapshot);
            }
        }

        // last_focus_info を更新
        self.executor.platform.focus.last_focus_info = Some((process_id, class_name.clone()));

        // prev_conversion_mode をリセット（異なるウィンドウの conversion_mode を比較しない）
        self.platform_state.preconditions.prev_conversion_mode = None;

        if process_changed {
            // [診断] フォーカス切り替え時点の stale スナップショットを記録する。
            // この値がどれだけ早く正しい値に置き換わるか（FocusProbe / ObserverPoll のタイミング）
            // を観測することで、アプローチ A（即時再プローブ）か B（per-HWND キャッシュ）かを判断できる。
            {
                let pc = &self.platform_state.preconditions;
                log::info!(
                    "FocusChange [{}→{}] {}: stale ime_on={}({:?}) mode={:?} japanese={}",
                    last_pid.map_or_else(|| "?".to_string(), |p| p.to_string()),
                    process_id,
                    class_name,
                    pc.ime_on,
                    pc.ime_on_source,
                    pc.input_mode,
                    pc.is_japanese_ime,
                );
            }

            // 診断用: フォアグラウンドプロセス変更時刻を記録
            self.platform_state.last_focus_change_ms = crate::hook::current_tick_ms();
            // composition_warm を epoch ベースで自動無効化（前ウィンドウの warm 状態を引き継がない）
            self.executor.platform.output.on_focus_changed();
            // 前ウィンドウの IME 観測値をクリア（新しいウィンドウは独自の状態を持つ）
            self.platform_state.ime_observations.clear_on_focus_change();

            // per-HWND キャッシュから新しいウィンドウの既知状態を即座に復元する。
            // - キャッシュヒット: stale 窓がゼロになる（FocusProbe / ObserverPoll で確認・補正）
            // - キャッシュミス: 今まで通り stale のまま probe を待つ
            let cache_key = (process_id, class_name.clone());
            if let Some(&snapshot) = self.executor.platform.focus.hwnd_ime_cache.get(&cache_key) {
                let age_ms = crate::hook::current_tick_ms()
                    .saturating_sub(snapshot.recorded_ms);
                // キャッシュが古すぎる場合（ウィンドウの IME 状態が変わった可能性が高い）は
                // キャッシュミスと同様に扱い、FocusProbe の結果を待つ。
                // 即座に古い値で ime_on を更新するとエンジンが誤って無効化される。
                if age_ms <= crate::timing::HWND_CACHE_MAX_AGE_MS {
                    self.platform_state.preconditions.set_ime_on(
                        snapshot.ime_on,
                        crate::ShadowSource::HwndCache,
                    );
                    self.platform_state.preconditions.input_mode = snapshot.input_mode;
                    log::info!(
                        "HwndCache: restore [{} {}] ime_on={} mode={:?} ({}ms ago)",
                        process_id, class_name, snapshot.ime_on, snapshot.input_mode, age_ms,
                    );
                } else {
                    log::info!(
                        "HwndCache: stale [{} {}] ime_on={} mode={:?} ({}ms ago > {}ms) → FocusProbe 待ち",
                        process_id, class_name, snapshot.ime_on, snapshot.input_mode,
                        age_ms, crate::timing::HWND_CACHE_MAX_AGE_MS,
                    );
                }
            } else {
                log::debug!(
                    "HwndCache: no entry for [{} {}], stale until FocusProbe",
                    process_id, class_name,
                );
            }

            // フォーカス変更時は IME 強制書き込みガードをリセットする。
            // 新しいウィンドウは独自の IME 状態を持つ可能性があるため、
            // 前のウィンドウで設定した状態を引きずらず、再検出から始める。
            // miss_count もリセットして、新しいウィンドウで十分な猶予を与える。
            if self.platform_state.preconditions.ime_force_on_guard
                || self.platform_state.preconditions.ime_detect_miss_count > 0
            {
                log::debug!(
                    "Focus changed: clearing ime_force_on_guard and detect_miss_count \
                     (new window may have different IME state)"
                );
                self.platform_state.preconditions.ime_force_on_guard = false;
                self.platform_state.preconditions.ime_detect_miss_count = 0;
            }

            // UIA 非同期判定が必要かチェック（Undetermined の場合）
            let needs_uia = kind == FocusKind::Undetermined;
            if needs_uia {
                if let Some(sender) = &self.executor.platform.focus.uia_sender {
                    let _ = sender.send(crate::focus::uia::SendableHwnd(hwnd));
                }
            }

            true
        } else {
            // 同一プロセ���内: UIA 判定は必要に応じ���
            if kind == FocusKind::Undetermined {
                if let Some(sender) = &self.executor.platform.focus.uia_sender {
                    let _ = sender.send(crate::focus::uia::SendableHwnd(hwnd));
                }
            }
            false
        }
    }

    /// 統合 IME リフレッシュタイマーをスケジュール（リセット）する。
    ///
    /// 既存のタイマーをキャンセルして `delay_ms` 後に再設定する。
    /// フォーカス変更(50ms) / ポーリング(500ms) / 即時(0ms) を統一的に扱う。
    pub fn schedule_ime_refresh(&mut self, delay_ms: u64) {
        self.executor.platform.timer.set(
            crate::TIMER_IME_REFRESH,
            std::time::Duration::from_millis(delay_ms),
        );
    }

    /// 配列を動的に切り替える
    pub fn switch_layout(&mut self, index: usize) {
        let Some(entry) = self.layouts.get(index) else {
            log::warn!("Layout index {index} out of range");
            return;
        };

        let name = entry.name.clone();
        let decision = self
            .engine
            .on_command(EngineCommand::SwapLayout(entry.layout.clone()), &self.build_ctx());
        self.executor.execute_from_loop(decision);

        self.executor.platform.tray.set_layout_name(&name);

        log::info!("Switched layout to: {name}");
    }

    /// 手動アプリオーバーライドのトグル処理
    pub fn toggle_app_override(&mut self) {
        let current = self.platform_state.focus_kind;
        let new_kind = if current == FocusKind::TextInput {
            FocusKind::NonText
        } else {
            FocusKind::TextInput
        };

        self.platform_state.focus_kind = new_kind;

        // Update learning cache
        if let Some((pid, cls)) = self.executor.platform.focus.last_focus_info.as_ref() {
            self.executor.platform.focus.cache.insert(
                *pid,
                cls.clone(),
                new_kind,
                DetectionSource::UserOverride,
            );
        }

        // If demoted to NonText, flush engine pending
        if new_kind == FocusKind::NonText {
            self.invalidate_engine_context(ContextChange::FocusChanged);
        }

        // バルーン通知を表示
        self.executor.platform.tray.show_balloon(
            "awase",
            if new_kind == FocusKind::TextInput {
                "テキスト入力モードに切り替えました"
            } else {
                "バイパスモードに切り替えました"
            },
        );

        let mode_str = if new_kind == FocusKind::TextInput {
            "TextInput (engine enabled)"
        } else {
            "NonText (engine bypassed)"
        };
        log::info!("Manual focus override: → {mode_str}");
    }

    /// Sync key 後に遅延されたキーを再処理する。
    ///
    /// sync key で guard が起動された後、KeyUp で OS が IME を切り替えてから呼ばれる。
    /// guard 解除 → IME 状態 refresh → バッファキー再処理。
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub fn process_deferred_keys(&mut self) {
        // Guard を解除
        if self.platform_state.ime_guard.active {
            self.platform_state.ime_guard.active = false;
            log::debug!("IME guard OFF (process_deferred_keys)");
        }

        // Refresh IME state (Observer → ImeObservations → Preconditions)
        unsafe {
            crate::observer::ime_observer::observe(
                &mut self.platform_state.preconditions,
                &mut self.platform_state.ime_observations,
            );
        }
        self.platform_state.apply_ime_observations(self.engine.is_user_enabled());

        // Drain deferred keys from Platform guard
        let keys: Vec<_> = self.platform_state.ime_guard.deferred_keys.drain(..).collect();
        if keys.is_empty() {
            return;
        }

        log::debug!("Processing {} deferred key(s) after IME toggle", keys.len());

        for (event, _phys) in keys {
            // Build fresh context with updated preconditions
            let ctx = self.build_ctx();
            let decision = self.engine.on_input(event, &ctx);
            self.executor.execute_from_loop(decision);
        }
    }

    /// パニックリセット: IME 関連キー連打で発動する緊急リセット。
    ///
    /// エンジン状態・IME・修飾キー・フック・キャッシュをすべて初期状態に戻す。
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub fn panic_reset(&mut self) {
        log::warn!("Panic reset triggered!");

        // 1. エンジンの保留状態をフラッシュ
        self.invalidate_engine_context(ContextChange::InputLanguageChanged);

        // 2. IME 未確定文字列をキャンセル → OFF → ON
        unsafe { cancel_ime_composition() };
        self.executor.platform.set_ime_open(false);
        self.executor.platform.set_ime_open(true);

        // 3. 全修飾キーの KeyUp を送信（スタック解消）
        send_all_modifier_key_ups();

        // 4. フック再インストール（OS に無言削除されていた場合のリカバリ）
        crate::hook::reinstall_hook();

        // 5. PlatformState を全面リセット
        self.platform_state.preconditions.input_mode = InputModeState::ObservedRomaji;
        self.platform_state.preconditions.set_ime_on(true, crate::ShadowSource::PanicReset); // 安全側: ON
        self.platform_state.preconditions.is_japanese_ime = true;
        self.platform_state.preconditions.prev_conversion_mode = None;
        self.platform_state.preconditions.ime_detect_miss_count = 0;
        // panic_reset 直後に refresh_ime_state_cache() が走ると、ここで書いた
        // ime_on=true を stale な observe() 結果が即座に上書きしてしまう。
        // force_on_guard で 1 サイクルだけ保護し、次の検出成功時に自然に解除する。
        self.platform_state.preconditions.ime_force_on_guard = true;
        self.platform_state.hook.sent_to_engine = [0u64; 4];
        self.platform_state.hook.track_only_keys = [0u64; 4];
        self.platform_state.hook.in_callback = false;
        self.platform_state.hook.suppress_ctrl_bypass = false;
        self.platform_state.ime_guard.active = false;
        self.platform_state.ime_guard.deferred_keys.clear();

        // 6. IME 状態を再取得
        self.refresh_ime_state_cache();

        // 7. バルーン通知
        self.executor
            .platform
            .tray
            .show_balloon("awase", "状態をリセットしました");
    }
}

/// 全修飾キーの KeyUp を `SendInput` で送信する。
///
/// Shift, Ctrl, Alt, Win の左右それぞれに対して KeyUp を送り、
/// スタックした修飾キー状態を解消する。
fn send_all_modifier_key_ups() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };

    // VK_SHIFT(0x10), VK_CONTROL(0x11), VK_MENU(0x12),
    // VK_LWIN(0x5B), VK_RWIN(0x5C),
    // VK_LSHIFT(0xA0), VK_RSHIFT(0xA1),
    // VK_LCONTROL(0xA2), VK_RCONTROL(0xA3),
    // VK_LMENU(0xA4), VK_RMENU(0xA5)
    const MODIFIER_VKS: [u16; 11] = [
        0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5,
    ];

    let inputs: Vec<INPUT> = MODIFIER_VKS
        .iter()
        .map(|&vk| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: crate::output::INJECTED_MARKER,
                },
            },
        })
        .collect();

    crate::win32::send_input_safe(&inputs);
    log::debug!("Sent KeyUp for all modifier keys");
}

/// IME の未確定文字列をキャンセルする。
///
/// # Safety
/// Win32 IMM API (`ImmGetContext`, `ImmNotifyIME`, `ImmReleaseContext`) を呼び出す。
unsafe fn cancel_ime_composition() {
    use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmNotifyIME, ImmReleaseContext};
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let hwnd = GetForegroundWindow();
    if hwnd.0.is_null() {
        return;
    }

    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() {
        return;
    }

    use windows::Win32::UI::Input::Ime::{NOTIFY_IME_ACTION, NOTIFY_IME_INDEX};
    // NI_COMPOSITIONSTR = 0x15, CPS_CANCEL = 0x04
    let _ = ImmNotifyIME(himc, NOTIFY_IME_ACTION(0x15), NOTIFY_IME_INDEX(0x04), 0);
    let _ = ImmReleaseContext(hwnd, himc);
    log::debug!("Cancelled IME composition");
}

