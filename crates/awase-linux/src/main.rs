use anyhow::{Context, Result};
use std::path::Path;

use awase::config::AppConfig;
use awase::engine::{
    Engine, InputContext, InputModeState, ModifierState, NicolaFsm, SpecialKeyCombos,
};
use awase::scanmap::KeyboardModel;
use awase::types::{KeyClassification, KeyEventType, Timestamp};
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
    let mut fsm = NicolaFsm::new(
        layout,
        left_thumb,
        right_thumb,
        config.general.simultaneous_threshold_ms,
        config.general.confirm_mode,
        config.general.speculative_delay_ms,
    );
    // 親指キー自体が Shift（evdev KEY_LEFTSHIFT/KEY_RIGHTSHIFT）に割り当てられて
    // いる場合、親指押下だけで Shift レベルが立つため複合面を無効化する
    // （Windows 側 `crates/awase-windows/src/app/bootstrap.rs` の
    // `thumb_shift_faces_enabled_for` と同じ判定方針。magic number を
    // `hook::classify_modifier` 呼び出しに置き換え重複を解消、2026-08-20
    // 独立レビューで指摘）。
    use awase::types::ModifierKey;
    fsm.set_thumb_shift_faces_enabled(
        awase_linux::hook::classify_modifier(u32::from(left_thumb.0)) != Some(ModifierKey::Shift)
            && awase_linux::hook::classify_modifier(u32::from(right_thumb.0))
                != Some(ModifierKey::Shift),
    );
    // /code-review指摘（PR #127、3回目）: timing_margin_percent/
    // min_overlap_margin_percentをconfig.tomlで設定可能にしたが、Windows側
    // （crates/awase-windows/src/app/bootstrap.rs）のset_timing_margins呼び出し
    // に対応する配線がこのstubには無く、値を設定しても無反応だった。
    fsm.set_timing_margins(
        config.general.timing_margin_percent,
        config.general.min_overlap_margin_percent,
    );
    let mut engine = Engine::new(
        fsm,
        SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
            ime_toggle: vec![],
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

    let mut modifiers = ModifierState::default();
    let mut left_thumb_down: Option<Timestamp> = None;
    let mut right_thumb_down: Option<Timestamp> = None;

    evdev.run_blocking(|event| {
        let vk = event.vk_code;
        let event_type = event.event_type;
        modifiers.update(&event);
        let is_down = matches!(event.event_type, KeyEventType::KeyDown);
        // auto-repeat KeyDown では最初のタイムスタンプを上書きしない
        // （Windows 実装 `crates/awase-windows/src/hook.rs` の `update_thumb`
        // クロージャと同じセマンティクス。上書きすると `left_thumb_consumed`
        // との比較で「消費済み」が auto-repeat のたびに剥がれてしまう、
        // 2026-08-20 独立レビューで発覚）。
        match event.key_classification {
            KeyClassification::LeftThumb => {
                left_thumb_down = if is_down {
                    left_thumb_down.or(Some(event.timestamp))
                } else {
                    None
                };
            }
            KeyClassification::RightThumb => {
                right_thumb_down = if is_down {
                    right_thumb_down.or(Some(event.timestamp))
                } else {
                    None
                };
            }
            KeyClassification::Char | KeyClassification::Passthrough => {}
        }

        let ctx = InputContext {
            ime_on: true, // Assume IME ON for now
            input_mode: InputModeState::ObservedRomaji,
            is_japanese_ime: true,
            composing: false, // Linux では composition 検出未実装
            modifiers,
            left_thumb_down,
            right_thumb_down,
        };
        let decision = engine.on_input(event, &ctx);

        output.execute_decision(&decision, vk, event_type);

        true // continue loop
    })?;

    Ok(())
}
