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
/// `paths::resolve_relative_to_exe`（exe 隣接 → ワークスペースルート）に加え、
/// .app バンドル配置（`Contents/MacOS/awase` → `Contents/Resources/`）を試す。
/// どこにも無ければカレントディレクトリ基準の相対パスをそのまま返す。
fn resolve_resource(path: &str) -> PathBuf {
    let resolved = awase::paths::resolve_relative_to_exe(path);
    if resolved.exists() {
        return resolved;
    }
    if let Some(candidate) = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("../Resources").join(path)))
    {
        if candidate.exists() {
            return candidate;
        }
    }
    resolved
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
    let engine = Engine::new(
        fsm,
        SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
            ime_toggle: vec![],
        },
    );

    // 7. Run platform event loop
    run_event_loop(engine, &config.general.default_layout)
}

#[cfg(target_os = "macos")]
fn run_event_loop(engine: Engine, layout_name: &str) -> Result<()> {
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
    event_loop.run(app)
}

#[cfg(not(target_os = "macos"))]
fn run_event_loop(_engine: Engine, _layout_name: &str) -> Result<()> {
    log::warn!("awase-macos event loop is only available on macOS");
    Ok(())
}

#[cfg(target_os = "macos")]
mod app {
    use std::time::Instant;

    use awase::engine::{
        Decision, Effect, Engine, EngineCommand, InputContext, InputEffect, InputModeState,
        ModifierState, TimerEffect, UiEffect,
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
            }
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
                        self.output.send_keys(actions);
                    }
                    Effect::Input(InputEffect::ReinjectKey(ev)) => {
                        self.output.reinject(ev.vk_code, ev.event_type);
                    }
                    Effect::Timer(TimerEffect::Set { id, duration }) => {
                        self.timers.set(*id, *duration);
                    }
                    Effect::Timer(TimerEffect::Kill(id)) => self.timers.kill(*id),
                    Effect::Ime(e) => log::debug!("IME effect not implemented on macOS: {e:?}"),
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
            // 自分自身の注入イベントは Engine に通さず素通しする
            if event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == INJECT_MARKER
            {
                return TapAction::Pass;
            }

            let keycode =
                u16::try_from(event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE))
                    .unwrap_or(u16::MAX);

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
            let decision = self.engine.on_input(raw, &ctx);
            self.apply_decision(decision)
        }

        fn on_timer_fired(&mut self, id: usize) {
            self.timers.fired(id);
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
    }
}
