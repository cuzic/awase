use anyhow::{Context, Result};
use std::path::Path;

use awase::config::AppConfig;
use awase::engine::{Engine, ImeSyncKeys, NicolaFsm, SpecialKeyCombos};
use awase::scanmap::KeyboardModel;
use awase::yab::YabLayout;

use awase_macos::vk::key_name_to_keycode;

fn main() -> Result<()> {
    // 1. Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("awase-macos starting");

    // 2. Load config
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
    let keyboard_model: KeyboardModel = config.general.keyboard_model.parse().unwrap_or_else(|e| {
        log::warn!("Invalid keyboard_model: {e}, using jis");
        KeyboardModel::Jis
    });

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

    // 6. Build Engine (NicolaFsm + InputTracker + empty ImeSyncKeys/SpecialKeyCombos)
    let tracker = awase::engine::input_tracker::InputTracker::new();
    let fsm = NicolaFsm::new(
        layout,
        left_thumb,
        right_thumb,
        config.general.simultaneous_threshold_ms,
        config.general.confirm_mode,
        config.general.speculative_delay_ms,
    );
    let _engine = Engine::new(
        fsm,
        tracker,
        ImeSyncKeys {
            toggle: vec![],
            on: vec![],
            off: vec![],
        },
        SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
        },
    );

    // 7. Event loop (stub)
    log::info!("awase-macos running. Press Ctrl+C to exit.");

    let mut event_loop = awase_macos::event_loop::EventLoop::new();
    event_loop.run()?;

    Ok(())
}
