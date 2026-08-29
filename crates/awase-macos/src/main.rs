use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use awase::config::AppConfig;
use awase::engine::{Engine, NicolaFsm, SpecialKeyCombos};
use awase::scanmap::KeyboardModel;
use awase::types::ModifierKey;
use awase::yab::YabLayout;

use awase_macos::vk::key_name_to_keycode;

/// リソース（config.toml / layout）を解決する。
///
/// .app バンドル内から実行されている場合は署名対象の `Contents/Resources` に
/// 固定し、カレントディレクトリへはフォールバックしない — Accessibility 権限を
/// 持つプロセスが、起動ディレクトリに置かれた署名対象外の config/layout を
/// 読み込むのを防ぐため（2026-08-29 セキュリティレビュー指摘1）。
///
/// バンドル外（`cargo run` / 手動配置）は従来どおり
/// `paths::resolve_relative_to_exe`（exe 隣接 → ワークスペースルート → CWD）。
fn resolve_resource(path: &str) -> PathBuf {
    let raw = Path::new(path);
    if raw.is_absolute() {
        return raw.to_path_buf();
    }
    if let Ok(exe) = std::env::current_exe() {
        let in_bundle = exe
            .to_str()
            .is_some_and(|p| p.contains(".app/Contents/MacOS/"));
        if in_bundle {
            if let Some(dir) = exe.parent() {
                return dir.join("../Resources").join(path);
            }
        }
    }
    awase::paths::resolve_relative_to_exe(path)
}

fn main() -> Result<()> {
    // 1. Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("awase-macos starting");

    // 2. Load config
    let config_path = resolve_resource("config.toml");
    let config = if config_path.exists() {
        log::info!("Loading config from: {}", config_path.display());
        AppConfig::load(&config_path)?
    } else {
        log::warn!("config.toml not found, using defaults");
        let toml_str = "[general]";
        toml::from_str(toml_str).context("Failed to create default config")?
    };
    let (config, warnings) = config.validate();
    for w in &warnings {
        log::warn!("Config: {w}");
    }

    // 3. Resolve key names to macOS keycodes
    let left_thumb = key_name_to_keycode(&config.general.left_thumb_key)
        .with_context(|| format!("Unknown left thumb key: {}", config.general.left_thumb_key))?;
    let right_thumb = key_name_to_keycode(&config.general.right_thumb_key).with_context(|| {
        format!(
            "Unknown right thumb key: {}",
            config.general.right_thumb_key
        )
    })?;

    // 4. Set thumb keycodes for hook classification
    awase_macos::hook::set_thumb_keycodes(left_thumb, right_thumb);

    // 5. Load .yab layout
    // .yab は JIS 物理位置ベースのため Jis 固定（keyboard_model 設定は 2026-07-06 撤去）
    let keyboard_model = KeyboardModel::Jis;

    let layout_rel = Path::new(&config.general.layouts_dir).join(&config.general.default_layout);
    let layout_path = resolve_resource(&layout_rel.to_string_lossy());
    let layout = if layout_path.exists() {
        let content = std::fs::read_to_string(&layout_path)?;
        YabLayout::parse(&content, keyboard_model)?.resolve_kana()
    } else {
        log::warn!(
            "Layout file not found: {}, using empty layout",
            layout_path.display()
        );
        YabLayout::parse("", keyboard_model)?
    };

    // 6. Build Engine (NicolaFsm + InputTracker + empty ImeSyncKeys/SpecialKeyCombos)
    let mut fsm = NicolaFsm::new(
        layout,
        left_thumb,
        right_thumb,
        config.general.simultaneous_threshold_ms,
        config.general.confirm_mode,
        config.general.speculative_delay_ms,
    );
    // 親指キー自体が Shift（macOS keycode 0x38/0x3C）に割り当てられている場合、
    // 親指押下だけで Shift レベルが立つため複合面を無効化する（Windows/Linux 側と
    // 同じ判定方針。magic number を `hook::classify_modifier` 呼び出しに置き換え
    // 重複を解消、2026-08-20 独立レビューで指摘）。
    fsm.set_thumb_shift_faces_enabled(
        awase_macos::hook::classify_modifier(left_thumb.0) != Some(ModifierKey::Shift)
            && awase_macos::hook::classify_modifier(right_thumb.0) != Some(ModifierKey::Shift),
    );
    // [keys] のコンボ設定を macOS keycode に解決して Engine に渡す
    let parse_combos = |keys: &[String], label: &str| {
        let parsed: Vec<_> = keys
            .iter()
            .filter_map(|s| {
                let combo = awase_macos::vk::parse_key_combo(s);
                if combo.is_none() {
                    log::warn!("keys.{label}: cannot parse combo \"{s}\" on macOS, ignoring");
                }
                combo
            })
            .collect();
        log::info!("keys.{label}: {keys:?} ({} parsed)", parsed.len());
        parsed
    };
    let engine = Engine::new(
        fsm,
        SpecialKeyCombos {
            engine_on: parse_combos(&config.keys.engine_on, "engine_on"),
            engine_off: parse_combos(&config.keys.engine_off, "engine_off"),
            ime_on: parse_combos(&config.keys.ime_on, "ime_on"),
            ime_off: parse_combos(&config.keys.ime_off, "ime_off"),
            ime_toggle: parse_combos(&config.keys.ime_toggle, "ime_toggle"),
        },
    );

    // 7. Run platform event loop
    let poll_interval =
        std::time::Duration::from_millis(u64::from(config.general.ime_poll_interval_ms.max(100)));
    run_event_loop(engine, &config.general.default_layout, poll_interval)
}

#[cfg(target_os = "macos")]
fn run_event_loop(
    engine: Engine,
    layout_name: &str,
    poll_interval: std::time::Duration,
) -> Result<()> {
    use std::cell::RefCell;
    use std::rc::Rc;

    if !awase_macos::hook::check_accessibility_permission() {
        anyhow::bail!(
            "Accessibility permission is not granted. \
             Enable this app in System Settings > Privacy & Security > Accessibility, \
             then restart."
        );
    }

    let output = awase_macos::output::Output::new()?;

    // メニューバー常駐（NSApplication 初期化後に作ること）
    awase_macos::event_loop::init_nsapp();
    let tray = awase_macos::tray::SystemTray::new();
    tray.set_layout_name(layout_name);

    let app = Rc::new(RefCell::new(app::App::new(engine, output, tray)));

    log::info!("awase-macos running (menu bar icon: あ). Quit from the menu or Ctrl+C.");
    let mut event_loop = awase_macos::event_loop::EventLoop::new();
    event_loop.run(app, poll_interval)
}

#[cfg(not(target_os = "macos"))]
fn run_event_loop(
    _engine: Engine,
    _layout_name: &str,
    _poll_interval: std::time::Duration,
) -> Result<()> {
    log::warn!("awase-macos event loop is only available on macOS");
    Ok(())
}

#[cfg(target_os = "macos")]
mod app {
    use std::time::Instant;

    use awase::engine::{
        Decision, Effect, Engine, EngineCommand, ImeEffect, InputContext, InputEffect,
        InputModeState, ModifierState, SetOpenOrigin, TimerEffect, UiEffect,
    };
    use awase::types::{
        KeyClassification, KeyEventType, ModifierKey, RawKeyEvent, ScanCode, Timestamp, VkCode,
    };
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventType, EventField};

    use awase_macos::event_loop::{LoopHandler, MenuAction, TapAction, Timers};
    use awase_macos::hook;
    use awase_macos::ime::ImeDetector;
    use awase_macos::output::{Output, INJECT_MARKER};
    use awase_macos::tray::SystemTray;

    /// kVK_JIS_Eisu（英数）— macOS 標準の IME OFF キー
    const KEYCODE_EISU: u16 = 0x66;
    /// kVK_JIS_Kana（かな）— macOS 標準の IME ON キー
    const KEYCODE_KANA: u16 = 0x68;

    /// IME 切替待ちの出力フラッシュ用タイマー ID。
    /// Engine の TimerEffect ID（小さい連番）と衝突しない値を使う。
    const FLUSH_TIMER_ID: usize = usize::MAX;
    /// フラッシュ再確認の間隔
    const FLUSH_RETRY: std::time::Duration = std::time::Duration::from_millis(20);

    /// 起動時点からの経過マイクロ秒を返す
    fn now_timestamp() -> Timestamp {
        use std::sync::OnceLock;
        static BASELINE: OnceLock<Instant> = OnceLock::new();
        let baseline = BASELINE.get_or_init(Instant::now);
        u64::try_from(baseline.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// Engine・出力・タイマーを束ねるアプリケーション状態。
    ///
    /// CFRunLoop（単一スレッド）上で tap コールバックとタイマーの両方から
    /// `RefCell` 経由で呼ばれる。
    pub struct App {
        engine: Engine,
        output: Output,
        timers: Timers,
        ime: ImeDetector,
        tray: SystemTray,
        modifiers: ModifierState,
        left_thumb_down: Option<Timestamp>,
        right_thumb_down: Option<Timestamp>,
        /// IME 切替が観測確認されるまで保留する出力アクション。
        ///
        /// 期待値ブリッジで Engine は切替直後の打鍵も変換するが、注入した
        /// ローマ字キーストローク自体が入力ソース切替の完了とレースすると
        /// 旧ソース（ABC 等）で解釈されてリテラル "wo" などになる。切替確認
        /// まで溜めて `FLUSH_TIMER_ID` のタイマーでフラッシュする。
        deferred_keys: Vec<awase::types::KeyAction>,
    }

    impl App {
        pub fn new(engine: Engine, output: Output, tray: SystemTray) -> Self {
            Self {
                engine,
                output,
                timers: Timers::new(),
                ime: ImeDetector::new(),
                tray,
                modifiers: ModifierState::default(),
                left_thumb_down: None,
                right_thumb_down: None,
                deferred_keys: Vec::new(),
            }
        }

        /// IME 切替待ちが解消していれば保留出力をフラッシュする。
        fn maybe_flush_deferred(&mut self) {
            if self.deferred_keys.is_empty() || self.ime.is_switch_pending() {
                return;
            }
            let keys = std::mem::take(&mut self.deferred_keys);
            log::debug!("Flushing {} deferred key action(s) after IME switch", keys.len());
            self.output.send_keys(&keys);
        }

        fn make_ctx(&self) -> InputContext {
            InputContext {
                // IME 検出不能なとき（不明なレイアウト等）は ON と仮定する
                ime_on: self.ime.is_ime_on().unwrap_or(true),
                // macOS の日本語 IME はローマ字入力が既定。JIS かな入力の観測は
                // 未実装のため Linux 実装と同じく ObservedRomaji 固定とする
                input_mode: InputModeState::ObservedRomaji,
                is_japanese_ime: self.ime.is_japanese_layout(),
                composing: false, // macOS では composition 検出未実装
                modifiers: self.modifiers,
                left_thumb_down: self.left_thumb_down,
                right_thumb_down: self.right_thumb_down,
            }
        }

        fn run_effects(&mut self, effects: &[Effect]) {
            for effect in effects {
                match effect {
                    Effect::Input(InputEffect::SendKeys(actions)) => {
                        // IME 切替中は旧入力ソースで解釈されてしまうため保留する
                        // （既に保留がある場合も順序維持のため追記する）
                        if self.ime.is_switch_pending() || !self.deferred_keys.is_empty() {
                            self.deferred_keys.extend(actions.iter().cloned());
                            self.timers.set(FLUSH_TIMER_ID, FLUSH_RETRY);
                        } else {
                            self.output.send_keys(actions);
                        }
                    }
                    Effect::Input(InputEffect::ReinjectKey(ev)) => {
                        self.output.reinject(ev.vk_code, ev.event_type);
                    }
                    Effect::Timer(TimerEffect::Set { id, duration }) => {
                        self.timers.set(*id, *duration);
                    }
                    Effect::Timer(TimerEffect::Kill(id)) => self.timers.kill(*id),
                    Effect::Ime(ImeEffect::SetOpen { open, origin }) => match origin {
                        // 明示的なユーザー操作（ime_on/off/toggle コンボ等）のみ実行する
                        SetOpenOrigin::ExplicitUserAction => {
                            if self.ime.set_ime_on(*open) {
                                log::debug!("IME set_open({open}) via TISSelectInputSource");
                            } else {
                                log::warn!(
                                    "IME set_open({open}) failed: no matching input source"
                                );
                            }
                        }
                        // ActivationSync は activation 遷移の echo（SetOpenOrigin の doc
                        // 参照）。macOS では ctx.ime_on が毎イベント TIS 観測で得た
                        // 実状態そのものなので、echo を TISSelectInputSource で実行する
                        // と OS/IME 自身の切替と競合する（ATOK が OS 標準 IME に
                        // 化ける等）。観測駆動の macOS では無視するのが正しい。
                        SetOpenOrigin::ActivationSync => {
                            log::trace!("IME set_open({open}) echo (ActivationSync) ignored");
                        }
                    },
                    Effect::Ui(UiEffect::EngineStateChanged { enabled, .. }) => {
                        self.tray.set_enabled(*enabled);
                    }
                }
            }
        }

        fn apply_decision(&mut self, decision: Decision) -> TapAction {
            match decision {
                Decision::PassThrough => TapAction::Pass,
                Decision::PassThroughWith { effects } => {
                    self.run_effects(&effects);
                    TapAction::Pass
                }
                Decision::Consume { effects } => {
                    self.run_effects(&effects);
                    TapAction::Consume
                }
            }
        }

        /// 英数/かな キーが OS に届く時点で IME 状態の期待値を立てる。
        ///
        /// 入力ソース切替は非同期のため、TIS 観測が追いつく前の打鍵が
        /// 旧状態で判定される（英字モード→かな→即 k で k が素通りする）
        /// のを防ぐ。
        fn expect_ime_from_key(&self, keycode: u16) {
            match keycode {
                KEYCODE_EISU => self.ime.expect_ime_on(false),
                KEYCODE_KANA => self.ime.expect_ime_on(true),
                _ => {}
            }
        }

        /// FlagsChanged イベントから修飾キーの押下/解放を求める。
        fn flags_changed_event_type(
            keycode: u16,
            event: &CGEvent,
        ) -> Option<(ModifierKey, KeyEventType)> {
            let mk = hook::classify_modifier(keycode)?;
            let flags = event.get_flags();
            let bit = match mk {
                ModifierKey::Shift => CGEventFlags::CGEventFlagShift,
                ModifierKey::Ctrl => CGEventFlags::CGEventFlagControl,
                ModifierKey::Alt => CGEventFlags::CGEventFlagAlternate,
                ModifierKey::Meta => CGEventFlags::CGEventFlagCommand,
            };
            let event_type = if flags.contains(bit) {
                KeyEventType::KeyDown
            } else {
                KeyEventType::KeyUp
            };
            Some((mk, event_type))
        }
    }

    impl LoopHandler for App {
        fn on_cg_event(&mut self, etype: CGEventType, event: &CGEvent) -> TapAction {
            // 切替待ちの保留出力があれば、後続イベント処理の前に順序を保って流す
            self.maybe_flush_deferred();

            let keycode =
                u16::try_from(event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE))
                    .unwrap_or(u16::MAX);

            // 自分自身の注入イベントは Engine に通さず素通しする
            // （親指キー単独打鍵の再注入もここを通って OS の IME 切替に届く）
            if event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == INJECT_MARKER
            {
                if matches!(etype, CGEventType::KeyDown) {
                    self.expect_ime_from_key(keycode);
                }
                return TapAction::Pass;
            }

            let event_type = match etype {
                CGEventType::KeyDown => KeyEventType::KeyDown,
                CGEventType::KeyUp => KeyEventType::KeyUp,
                CGEventType::FlagsChanged => {
                    // 修飾キーはフラグ遷移から down/up を合成する
                    match Self::flags_changed_event_type(keycode, event) {
                        Some((_, et)) => et,
                        None => return TapAction::Pass, // CapsLock 等は素通し
                    }
                }
                _ => return TapAction::Pass,
            };

            let (key_classification, physical_pos) = hook::classify_key(keycode);
            let is_down = matches!(event_type, KeyEventType::KeyDown);

            let raw = RawKeyEvent {
                vk_code: VkCode(keycode),
                scan_code: ScanCode(u32::from(keycode)),
                event_type,
                extra_info: 0,
                timestamp: now_timestamp(),
                key_classification,
                physical_pos,
                ime_relevance: hook::classify_ime_relevance(keycode),
                modifier_key: hook::classify_modifier(keycode),
                modifier_snapshot: self.modifiers,
                // CGEventTap では他プロセス注入の確実な識別手段がないため false 固定
                injected: false,
            };

            self.modifiers.update(&raw);

            // auto-repeat KeyDown では最初のタイムスタンプを上書きしない
            // （Linux/Windows 実装と同じセマンティクス。上書きすると
            // `left_thumb_consumed` との比較で「消費済み」が剥がれる）。
            match key_classification {
                KeyClassification::LeftThumb => {
                    self.left_thumb_down = if is_down {
                        self.left_thumb_down.or(Some(raw.timestamp))
                    } else {
                        None
                    };
                }
                KeyClassification::RightThumb => {
                    self.right_thumb_down = if is_down {
                        self.right_thumb_down.or(Some(raw.timestamp))
                    } else {
                        None
                    };
                }
                KeyClassification::Char | KeyClassification::Passthrough => {}
            }

            let ctx = self.make_ctx();

            // 確定/取消キー（Enter・keypad Enter・Escape・Tab）の通過と IME OFF は
            // composition の切れ目とみなす（Output::composing_hint の doc 参照）
            if (is_down && matches!(keycode, 0x24 | 0x4C | 0x35 | 0x30)) || !ctx.ime_on {
                self.output.note_composition_break();
            }

            let decision = self.engine.on_input(raw, &ctx);
            let action = self.apply_decision(decision);

            // 物理の英数/かな キーが素通しで OS に届く場合（Engine 非活性時や
            // 親指キー以外に設定されている場合）も IME 切替の期待を立てる
            if action == TapAction::Pass && is_down {
                self.expect_ime_from_key(keycode);
            }
            action
        }

        fn on_timer_fired(&mut self, id: usize) {
            self.timers.fired(id);
            if id == FLUSH_TIMER_ID {
                if self.ime.is_switch_pending() {
                    // まだ切替中: 再確認をスケジュール（期待の猶予超過で
                    // is_switch_pending が false になるため無限には続かない）
                    self.timers.set(FLUSH_TIMER_ID, FLUSH_RETRY);
                } else {
                    self.maybe_flush_deferred();
                }
                return;
            }
            let ctx = self.make_ctx();
            let decision = self.engine.on_timeout(id, &ctx);
            // タイムアウトには「現在のイベント」が無いため Pass/Consume は無意味
            let _ = self.apply_decision(decision);
        }

        fn on_menu_action(&mut self, action: MenuAction) {
            match action {
                MenuAction::ToggleEngine => {
                    let ctx = self.make_ctx();
                    let decision = self.engine.on_command(EngineCommand::ToggleEngine, &ctx);
                    // メニュー操作にも「現在のイベント」は無い
                    let _ = self.apply_decision(decision);
                }
            }
        }

        fn on_poll(&mut self) {
            self.maybe_flush_deferred();
            // activation 遷移の検知はキーイベント経由でしか起きないため、
            // IME 切替後に打鍵が無いとトレイ表示が古いまま残る。RefreshState で
            // 遷移チェックだけを走らせ、UiEffect でトレイを追随させる。
            let ctx = self.make_ctx();
            let decision = self.engine.on_command(EngineCommand::RefreshState, &ctx);
            let _ = self.apply_decision(decision);
        }
    }
}
