use anyhow::{Context, Result};
use std::path::Path;

use awase::config::AppConfig;
use awase::engine::{Engine, InputContext, InputModeState, NicolaFsm, SpecialKeyCombos};
use awase::scanmap::KeyboardModel;
use awase::yab::YabLayout;

use awase_linux::hook::EvdevInput;
use awase_linux::output::UinputOutput;
use awase_linux::vk::key_name_to_evdev;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("awase-linux starting");

    // 1. Load config
    let config_path = Path::new("config.toml");
    let config = if config_path.exists() {
        AppConfig::load(config_path)?
    } else {
        log::warn!("config.toml not found, using defaults");
        let toml_str = "[general]";
        toml::from_str(toml_str).context("Failed to create default config")?
    };
    let (config, warnings) = config.validate();
    for w in &warnings {
        log::warn!("Config: {w}");
    }

    // 2. Resolve key names to evdev keycodes
    let left_thumb = key_name_to_evdev(&config.general.left_thumb_key)
        .with_context(|| format!("Unknown left thumb key: {}", config.general.left_thumb_key))?;
    let right_thumb = key_name_to_evdev(&config.general.right_thumb_key).with_context(|| {
        format!(
            "Unknown right thumb key: {}",
            config.general.right_thumb_key
        )
    })?;

    // 3. Set thumb keycodes for hook classification
    awase_linux::hook::set_thumb_keycodes(left_thumb, right_thumb);

    // 4. Load layout
    // .yab は JIS 物理位置ベースのため Jis 固定（keyboard_model 設定は 2026-07-06 撤去）
    let keyboard_model = KeyboardModel::Jis;

    let layout_path = Path::new(&config.general.layouts_dir).join(&config.general.default_layout);
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

    // 5. Build Engine
    let fsm = NicolaFsm::new(
        layout,
        left_thumb,
        right_thumb,
        config.general.simultaneous_threshold_ms,
        // 確定モードは NgramPredictive 固定（n-gram モデル未指定時は TwoPhase に
        // 自動フォールバック）
        awase::config::ConfirmMode::NgramPredictive,
        config.general.speculative_delay_ms,
    );
    let mut engine = Engine::new(
        fsm,
        SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
        },
    );

    // 6. Open evdev device (using config)
    log::info!("Input backend: {}", config.general.linux_input_backend);
    if config.general.linux_input_backend != "evdev" {
        anyhow::bail!(
            "Backend \"{}\" is not yet implemented. Currently only \"evdev\" is supported.",
            config.general.linux_input_backend
        );
    }
    let mut evdev = if let Some(ref dev_path) = config.general.linux_evdev_device {
        log::info!("Using configured evdev device: {dev_path}");
        EvdevInput::open(Path::new(dev_path))?
    } else {
        log::info!("Auto-detecting keyboard device");
        EvdevInput::open_auto()?
    };
    log::info!("Keyboard device opened");

    // 7. Grab device (exclusive access)
    evdev.grab()?;
    log::info!("Device grabbed (exclusive access)");

    // 8. Create output
    let mut output = UinputOutput::new()?;
    log::info!("Virtual keyboard created");

    // 9. Run blocking event loop
    log::info!("awase-linux running. Press Ctrl+C to exit.");

    evdev.run_blocking(|event| {
        let vk = event.vk_code;
        let event_type = event.event_type;

        let ctx = InputContext {
            ime_on: true, // Assume IME ON for now
            input_mode: InputModeState::ObservedRomaji,
            is_japanese_ime: true,
            modifiers: awase::engine::ModifierState {
                ctrl: false,
                alt: false,
                shift: false,
                win: false,
            },
            left_thumb_down: None,
            right_thumb_down: None,
        };
        let decision = engine.on_input(event, &ctx);

        output.execute_decision(&decision, vk, event_type);

        true // continue loop
    })?;

    Ok(())
}
