use anyhow::{Context, Result};
use std::path::Path;

use awase::config::AppConfig;
use awase::engine::{
    Decision, Effect, Engine, ImeSyncKeys, InputContext, InputEffect, NicolaFsm, SpecialKeyCombos,
    TimerEffect,
};
use awase::scanmap::KeyboardModel;
use awase::types::ImeCacheState;
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

    // 5. Build Engine
    let tracker = awase::engine::input_tracker::InputTracker::new();
    let fsm = NicolaFsm::new(
        layout,
        left_thumb,
        right_thumb,
        config.general.simultaneous_threshold_ms,
        config.general.confirm_mode,
        config.general.speculative_delay_ms,
    );
    let mut engine = Engine::new(
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
            ime_cache: ImeCacheState::On, // Assume IME ON for now
        };
        let decision = engine.on_input(event, &ctx);

        execute_decision(&decision, &mut output, vk, event_type);

        true // continue loop
    })?;

    Ok(())
}

/// `Decision` を実行し、副作用を uinput デバイスに反映する。
fn execute_decision(
    decision: &Decision,
    output: &mut UinputOutput,
    vk: awase::types::VkCode,
    event_type: awase::types::KeyEventType,
) {
    match decision {
        Decision::PassThrough => {
            // Device is grabbed, so we must re-inject the key via uinput
            reinject_key(output, vk, event_type);
        }
        Decision::PassThroughWith { effects } => {
            reinject_key(output, vk, event_type);
            execute_effects(effects, output);
        }
        Decision::Consume { effects } => {
            execute_effects(effects, output);
        }
    }
}

/// パススルーキーを uinput 経由で再注入する。
fn reinject_key(
    output: &mut UinputOutput,
    vk: awase::types::VkCode,
    event_type: awase::types::KeyEventType,
) {
    use awase::types::{KeyAction, KeyEventType};

    match event_type {
        KeyEventType::KeyDown => {
            output.send_keys(&[KeyAction::Key(vk)]);
        }
        KeyEventType::KeyUp => {
            output.send_keys(&[KeyAction::KeyUp(vk)]);
        }
    }
}

/// `Effect` リストを順に実行する。
fn execute_effects(effects: &[Effect], output: &mut UinputOutput) {
    for effect in effects {
        match effect {
            Effect::Input(InputEffect::SendKeys(actions)) => {
                output.send_keys(actions);
            }
            Effect::Input(InputEffect::ReinjectKey(raw_event)) => {
                reinject_key(output, raw_event.vk_code, raw_event.event_type);
            }
            Effect::Timer(TimerEffect::Set { id, duration }) => {
                log::debug!(
                    "Timer set request: id={id}, duration={duration:?} (not yet implemented)"
                );
            }
            Effect::Timer(TimerEffect::Kill(id)) => {
                log::debug!("Timer kill request: id={id} (not yet implemented)");
            }
            Effect::Ime(ime_effect) => {
                log::debug!("IME effect: {ime_effect:?} (not yet implemented)");
            }
            Effect::Ui(ui_effect) => {
                log::debug!("UI effect: {ui_effect:?}");
            }
            Effect::Focus(focus_effect) => {
                log::debug!("Focus effect: {focus_effect:?} (not applicable on Linux)");
            }
            Effect::ImeCache(cache_effect) => {
                log::debug!("IME cache effect: {cache_effect:?}");
            }
        }
    }
}
