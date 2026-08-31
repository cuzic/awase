use anyhow::{Context, Result};
use std::path::Path;

use awase::config::AppConfig;
use awase::engine::{Engine, NicolaFsm, SpecialKeyCombos};
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
    // .yab は JIS 物理位置ベースのため Jis 固定（keyboard_model 設定は 2026-07-06 撤去）
    let keyboard_model = KeyboardModel::Jis;

    let layout_path = Path::new(&config.general.layouts_dir).join(&config.general.default_layout);
    let layout = if layout_path.exists() {
        let content = std::fs::read_to_string(&layout_path)?;
        let layout = YabLayout::parse(&content, keyboard_model)?.resolve_kana();
        let (layout, keystroke_warnings) = awase::yab::resolve_keystroke_syntax(
            layout,
            &config.keystroke_macro,
            config.general.keystroke_sequence,
        );
        for w in &keystroke_warnings {
            log::warn!("Layout ({}): {w}", layout_path.display());
        }
        layout
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
    use awase::types::ModifierKey;
    fsm.set_thumb_shift_faces_enabled(
        awase_macos::hook::classify_modifier(left_thumb.0) != Some(ModifierKey::Shift)
            && awase_macos::hook::classify_modifier(right_thumb.0) != Some(ModifierKey::Shift),
    );
    // /code-review指摘（PR #127、3回目）: timing_margin_percent/
    // min_overlap_margin_percentをconfig.tomlで設定可能にしたが、Windows側
    // （crates/awase-windows/src/app/bootstrap.rs）の対応する配線がこのstubには
    // 無く、値を設定しても無反応だった。
    fsm.apply_general_config(&config.general);
    let _engine = Engine::new(
        fsm,
        SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
            ime_toggle: vec![],
        },
    );

    // 7. Event loop (stub)
    log::info!("awase-macos running. Press Ctrl+C to exit.");

    let mut event_loop = awase_macos::event_loop::EventLoop::new();
    event_loop.run()?;

    Ok(())
}
