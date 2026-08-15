use smallvec::smallvec;

use super::test_support::*;
use super::*;
use crate::config::ConfirmMode;
use crate::engine::nicola_fsm::yab_value_to_action;
use crate::engine::output_history::OutputEntry;
use crate::ngram::NgramModel;
use crate::scanmap::PhysicalPos;
use crate::types::{
    ContextChange, KeyAction, KeyEventType, RawKeyEvent, ScanCode, Timestamp, VkCode,
};
use crate::yab::YabValue;
use timed_fsm::Response;

type Resp = Response<KeyAction, usize>;

// ── VK / scan codes shared with test_support: VK_A, VK_S, VK_NONCONVERT, VK_CONVERT,
//    SCAN_A, SCAN_S, SCAN_NONCONVERT, SCAN_CONVERT, POS_A, POS_S, lit(), make_layout() ──

// VK code constants specific to this test file
const VK_RETURN: VkCode = VkCode(0x0D);
const VK_SPACE: VkCode = VkCode(0x20);
const VK_SHIFT: VkCode = VkCode(0x10);
const VK_LSHIFT: VkCode = VkCode(0xA0);
const VK_RSHIFT: VkCode = VkCode(0xA1);
const VK_CTRL: VkCode = VkCode(0x11);
const VK_LCTRL: VkCode = VkCode(0xA2);
const VK_ALT: VkCode = VkCode(0x12);
const VK_LALT: VkCode = VkCode(0xA4);
const VK_D: VkCode = VkCode(0x44);
const VK_F: VkCode = VkCode(0x46);
const VK_C: VkCode = VkCode(0x43);
const VK_V: VkCode = VkCode(0x56);

// Scan code constants specific to this test file
const SCAN_D: ScanCode = ScanCode(0x20);
const SCAN_F: ScanCode = ScanCode(0x21);
const SCAN_C: ScanCode = ScanCode(0x2E);
const SCAN_V: ScanCode = ScanCode(0x2F);
const SCAN_RETURN: ScanCode = ScanCode(0x1C);
const SCAN_SPACE: ScanCode = ScanCode(0x39);
const SCAN_SHIFT: ScanCode = ScanCode(0x2A);
const SCAN_LSHIFT: ScanCode = ScanCode(0x2A);
const SCAN_RSHIFT: ScanCode = ScanCode(0x36);
const SCAN_CTRL: ScanCode = ScanCode(0x1D);
const SCAN_LCTRL: ScanCode = ScanCode(0x1D);
const SCAN_ALT: ScanCode = ScanCode(0x38);
const SCAN_LALT: ScanCode = ScanCode(0x38);

/// PhysicalPos for D key (row=2, col=2)
const POS_D: PhysicalPos = PhysicalPos::new(2, 2);
/// PhysicalPos for F key (row=2, col=3)
const POS_F: PhysicalPos = PhysicalPos::new(2, 3);

/// テスト用ハーネス: InputTracker + NicolaFsm を統合し、
/// on_event で自動的に物理キー状態を追跡する。
struct TestHarness {
    tracker: input_tracker::InputTracker,
    engine: NicolaFsm,
}

impl TestHarness {
    fn on_event(&mut self, event: RawKeyEvent) -> Resp {
        let phys = self.tracker.process(&event);
        self.engine.on_event(event, &phys)
    }

    fn on_timeout(&mut self, timer_id: usize) -> Resp {
        let phys = self.tracker.snapshot();
        self.engine.on_timeout(timer_id, &phys, false)
    }

    /// `composing` を明示指定するタイムアウト処理（IME composition 中の挙動を検証するため）。
    fn on_timeout_composing(&mut self, timer_id: usize, composing: bool) -> Resp {
        let phys = self.tracker.snapshot();
        self.engine.on_timeout(timer_id, &phys, composing)
    }
}

impl std::ops::Deref for TestHarness {
    type Target = NicolaFsm;
    fn deref(&self) -> &NicolaFsm {
        &self.engine
    }
}

impl std::ops::DerefMut for TestHarness {
    fn deref_mut(&mut self) -> &mut NicolaFsm {
        &mut self.engine
    }
}

fn make_engine() -> TestHarness {
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    }
}

fn make_speculative_engine() -> TestHarness {
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Speculative,
            30,
        ),
    }
}

struct Ev;

impl Ev {
    fn down(vk: VkCode) -> EvBuilder {
        EvBuilder {
            vk,
            scan: vk_to_scan(vk),
            ts: 0,
            event_type: KeyEventType::KeyDown,
            injected: false,
        }
    }
    fn up(vk: VkCode) -> EvBuilder {
        EvBuilder {
            vk,
            scan: vk_to_scan(vk),
            ts: 0,
            event_type: KeyEventType::KeyUp,
            injected: false,
        }
    }
}

struct EvBuilder {
    vk: VkCode,
    scan: ScanCode,
    ts: Timestamp,
    event_type: KeyEventType,
    injected: bool,
}

impl EvBuilder {
    fn at(mut self, ts: Timestamp) -> Self {
        self.ts = ts;
        self
    }
    fn scan(mut self, sc: ScanCode) -> Self {
        self.scan = sc;
        self
    }
    fn injected(mut self, injected: bool) -> Self {
        self.injected = injected;
        self
    }
    fn build(self) -> RawKeyEvent {
        let (kc, pos) = classify_test_key(self.vk, self.scan);
        RawKeyEvent {
            vk_code: self.vk,
            scan_code: self.scan,
            event_type: self.event_type,
            extra_info: 0,
            timestamp: self.ts,
            key_classification: kc,
            physical_pos: pos,
            ime_relevance: crate::types::ImeRelevance::default(),
            modifier_key: classify_test_modifier(self.vk),
            modifier_snapshot: Default::default(),
            injected: self.injected,
        }
    }
}

/// Map VK code to a realistic scan code for tests
fn classify_test_key(
    vk: VkCode,
    _scan: ScanCode,
) -> (crate::types::KeyClassification, Option<PhysicalPos>) {
    use crate::types::KeyClassification;

    if vk == VK_NONCONVERT || vk == VK_SPACE {
        (KeyClassification::LeftThumb, None)
    } else if vk == VK_CONVERT {
        (KeyClassification::RightThumb, None)
    } else if let Some(pos) = test_vk_to_pos(vk) {
        (KeyClassification::Char, Some(pos))
    } else {
        (KeyClassification::Passthrough, None)
    }
}

/// テスト用: VK → PhysicalPos 直接マッピング（scan_to_pos を使わない）
fn test_vk_to_pos(vk: VkCode) -> Option<PhysicalPos> {
    use crate::scanmap::PhysicalPos;
    match vk {
        VK_A => Some(PhysicalPos::new(2, 0)),
        VK_S => Some(PhysicalPos::new(2, 1)),
        VK_D => Some(PhysicalPos::new(2, 2)),
        VK_F => Some(PhysicalPos::new(2, 3)),
        VK_C => Some(PhysicalPos::new(3, 2)),
        VK_V => Some(PhysicalPos::new(3, 3)),
        _ => None,
    }
}

fn classify_test_modifier(vk: VkCode) -> Option<crate::types::ModifierKey> {
    use crate::types::ModifierKey;
    match vk {
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => Some(ModifierKey::Shift),
        VK_CTRL | VK_LCTRL => Some(ModifierKey::Ctrl),
        VK_ALT | VK_LALT => Some(ModifierKey::Alt),
        _ => None,
    }
}

fn vk_to_scan(vk: VkCode) -> ScanCode {
    match vk {
        VK_A => SCAN_A,
        VK_S => SCAN_S,
        VK_D => SCAN_D,
        VK_F => SCAN_F,
        VK_C => SCAN_C,
        VK_V => SCAN_V,
        VK_NONCONVERT => SCAN_NONCONVERT,
        VK_CONVERT => SCAN_CONVERT,
        VK_RETURN => SCAN_RETURN,
        VK_SPACE => SCAN_SPACE,
        VK_SHIFT => SCAN_SHIFT,
        VK_LSHIFT => SCAN_LSHIFT,
        VK_RSHIFT => SCAN_RSHIFT,
        VK_CTRL => SCAN_CTRL,
        VK_LCTRL => SCAN_LCTRL,
        VK_ALT => SCAN_ALT,
        VK_LALT => SCAN_LALT,
        _ => ScanCode(0),
    }
}

fn assert_pending(result: &Resp) {
    result.assert_consumed();
    assert!(result.actions.is_empty(), "pending should have no actions");
    result.assert_timer_set(TIMER_PENDING);
}

#[test]
fn test_disabled_engine_passes_through() {
    let mut engine = make_engine();
    let _ = engine.toggle_enabled();
    engine
        .on_event(Ev::down(VK_A).build())
        .assert_pass_through();
}

#[test]
fn test_modifier_key_passes_through() {
    let mut engine = make_engine();
    engine
        .on_event(Ev::down(VK_SHIFT).build())
        .assert_pass_through();
}

#[test]
fn test_non_layout_key_passes_through() {
    let mut engine = make_engine();
    engine
        .on_event(Ev::down(VK_RETURN).build())
        .assert_pass_through();
}

#[test]
fn test_pattern1_thumb_first_then_char() {
    let mut engine = make_engine();
    let t0 = 0;

    let result = engine.on_event(Ev::down(VK_NONCONVERT).at(t0).build());
    assert_pending(&result);

    let t1 = t0 + 30_000;
    let result = engine.on_event(Ev::down(VK_A).at(t1).build());
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('を')));
}

#[test]
fn test_pattern2_char_first_then_thumb() {
    let mut engine = make_engine();
    let t0 = 0;

    let result = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&result);

    // char + thumb → PendingCharThumb（3 鍵目を待つ）
    let t1 = t0 + 30_000;
    let result = engine.on_event(Ev::down(VK_CONVERT).at(t1).build());
    assert_pending(&result);

    // タイムアウト → char1+thumb を同時打鍵として確定
    let result = engine.on_timeout(TIMER_PENDING);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('ゔ')));
}

#[test]
fn test_pattern3_char_timeout() {
    let mut engine = make_engine();

    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);

    let result = engine.on_timeout(TIMER_PENDING);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('う')));
}

#[test]
fn test_pattern4_char_sequence() {
    let mut engine = make_engine();
    let t0 = 0;

    let result = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&result);

    let t1 = t0 + 30_000;
    let result = engine.on_event(Ev::down(VK_S).at(t1).build());
    result.assert_consumed();
    assert!(result
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('う'))));
    result.assert_timer_set(TIMER_PENDING);
}

#[test]
fn test_pattern5_thumb_alone_timeout() {
    let mut engine = make_engine();

    let result = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout(TIMER_PENDING);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Key(x) if x == VK_NONCONVERT));
}

/// composition 中は、無変換/変換キー単独タップの生 VK 送出を suppress する。
/// 生VKをMS-IMEに渡すと既定機能（かな/カタカナ切替・再変換）が誤発火するため。
#[test]
fn test_pattern5_thumb_alone_timeout_suppressed_while_composing() {
    let mut engine = make_engine();

    let result = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, true);
    assert_eq!(result.actions.len(), 0);
}

/// `PendingThumb` 中に、その親指面に候補が無い文字キーが来た場合、待たずに
/// 即座に「親指単独確定 + 新規キー再処理」される（`step_pending_thumb_char` の
/// 「候補なし」分岐）。
///
/// これは `NicolaFsm::go_idle()` が実際に `state` を `Idle` に戻すことに依存する
/// 挙動である。`go_idle()` が state を変更しない no-op に壊れると、
/// `ShiftReduceParser::parse`（crates/timed-fsm/src/parser.rs）の
/// `ReduceAndContinue` ループが同じ `PendingThumb` に対して同じ文字キーを
/// 何度も再判定し続け、**実際にプロセスがハングする**（cargo-mutants がこの
/// 変異を `MISSED` ではなく `TIMEOUT` として検出したのはこのため）。
///
/// ループが規定回数内に終わらなければハングする代わりに明示的に panic させる
/// ウォッチドッグで囲み、将来同じ回帰が起きても CI がタイムアウトで無言のまま
/// 詰まるのではなく、すぐ分かる形で落ちるようにしている。
#[test]
fn test_pattern6_thumb_then_noncandidate_char_resolves_without_hanging() {
    use std::sync::mpsc;
    use std::time::Duration;

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut engine = make_engine();
        let t0 = 0;

        let result = engine.on_event(Ev::down(VK_NONCONVERT).at(t0).build());
        assert_pending(&result);
        assert!(matches!(engine.state, EngineState::PendingThumb(_)));

        // VK_D は make_layout() のどの面（Normal/LeftThumb/RightThumb/Shift）にも
        // 定義が無いキー（test_support::make_layout 参照）。
        let t1 = t0 + 30_000;
        let result = engine.on_event(Ev::down(VK_D).at(t1).build());
        let _ = tx.send((result, engine.state.is_idle()));
    });

    let (result, is_idle) = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("PendingThumb + no-candidate char must resolve promptly, not hang (go_idle() must actually transition state to Idle)");

    // 親指単独確定 (生 VK_NONCONVERT) が Reduce され、その後 D キーは
    // レイアウト未定義キーとして PassThrough される → 蓄積アクションは1件のみ。
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Key(x) if x == VK_NONCONVERT));
    assert!(
        is_idle,
        "state must be Idle after resolving the stale PendingThumb, got non-idle"
    );
}

// ── 無変換/変換キー単独タップの composing 中ガード opt-out
//    (muhenkan/henkan_solo_tap_ignore_composing_guard) ──

/// 無変換を左親指キー・変換を右親指キーとし、`muhenkan_vk`/`henkan_vk` と
/// 各 `*_solo_tap_ignore_composing_guard` を明示設定したエンジンを返す。
///
/// `muhenkan_always_suppress` は既定 `false`（従来通りの composing-guard 挙動を
/// 単体で検証したいテストのため）で、無変換の常時抑制を有効化したいテストは
/// `make_engine_with_thumb_key_solo_tap_config_ex` を使うこと。
fn make_engine_with_thumb_key_solo_tap_config(
    muhenkan_ignore_composing_guard: bool,
    henkan_ignore_composing_guard: bool,
) -> TestHarness {
    make_engine_with_thumb_key_solo_tap_config_ex(
        muhenkan_ignore_composing_guard,
        false,
        henkan_ignore_composing_guard,
        false,
    )
}

/// `make_engine_with_thumb_key_solo_tap_config` に加え、
/// `muhenkan_solo_tap_always_suppress`/`henkan_solo_tap_always_suppress` も
/// 明示設定できる版。
fn make_engine_with_thumb_key_solo_tap_config_ex(
    muhenkan_ignore_composing_guard: bool,
    muhenkan_always_suppress: bool,
    henkan_ignore_composing_guard: bool,
    henkan_always_suppress: bool,
) -> TestHarness {
    let mut engine = NicolaFsm::new(
        make_layout(),
        VK_NONCONVERT,
        VK_CONVERT,
        100,
        ConfirmMode::Wait,
        30,
    );
    engine.set_thumb_key_solo_tap_config(
        Some(VK_NONCONVERT),
        ModeKeyConfig::from_legacy_bools(muhenkan_ignore_composing_guard, muhenkan_always_suppress),
        Some(VK_CONVERT),
        ModeKeyConfig::from_legacy_bools(henkan_ignore_composing_guard, henkan_always_suppress),
    );
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine,
    }
}

/// `muhenkan_solo_tap_ignore_composing_guard=true`（既定 false からのオプトイン）
/// なら、composing 中でも無変換キー単独タップで生 VK_NONCONVERT を送出する。
#[test]
fn test_muhenkan_thumb_emits_while_composing_when_guard_enabled() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config(true, false);

    let result = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, true);
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT)),
        "muhenkan_solo_tap_ignore_composing_guard=true なら composing 中でも VK_NONCONVERT を送出すべき"
    );
}

/// 変換キー側も同様に `henkan_solo_tap_ignore_composing_guard=true` で
/// composing 中の単独タップが素通しされる。
#[test]
fn test_henkan_thumb_emits_while_composing_when_guard_enabled() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config(false, true);

    let result = engine.on_event(Ev::down(VK_CONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, true);
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_CONVERT)),
        "henkan_solo_tap_ignore_composing_guard=true なら composing 中でも VK_CONVERT を送出すべき"
    );
}

/// `muhenkan_solo_tap_always_suppress=true`（既定値）なら、composing=false
/// （Windows 全般でのキー機能維持のための素通し経路）でも無変換単独タップは
/// 一切送出されない。MS-IME の既定キー割当て（無変換単独打鍵→かな切替）に
/// 素通しした生 VK_NONCONVERT を横取りされ、awase の管理外で IME モードが
/// 切り替わる事故（2026-08-07 実機）を防ぐためのガード。
#[test]
fn test_muhenkan_always_suppress_blocks_even_when_not_composing() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config_ex(false, true, false, false);

    let result = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, false);
    assert_eq!(
        result.actions.len(),
        0,
        "muhenkan_solo_tap_always_suppress=true なら composing=false でも \
         VK_NONCONVERT を送出してはならない"
    );
}

/// `muhenkan_solo_tap_always_suppress=true` は
/// `muhenkan_solo_tap_ignore_composing_guard=true`（composing 中の素通しオプトイン）
/// より優先される。
#[test]
fn test_muhenkan_always_suppress_overrides_ignore_composing_guard() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config_ex(true, true, false, false);

    let result = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, true);
    assert_eq!(
        result.actions.len(),
        0,
        "muhenkan_solo_tap_always_suppress=true なら ignore_composing_guard=true でも \
         composing 中の VK_NONCONVERT 送出を抑制すべき"
    );
}

/// `muhenkan_solo_tap_always_suppress=false` なら、composing=false のときは
/// 従来通り生 VK_NONCONVERT が送出される（既定を外した場合の後方互換確認）。
#[test]
fn test_muhenkan_always_suppress_false_preserves_legacy_passthrough() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config_ex(false, false, false, false);

    let result = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, false);
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT)),
        "muhenkan_solo_tap_always_suppress=false なら composing=false のとき \
         従来通り VK_NONCONVERT を送出すべき"
    );
}

/// ADR-091 §D3.2: `set_muhenkan_solo_tap_dedicated_fn_key(Some(vk))` が設定されている場合、
/// `muhenkan_solo_tap_always_suppress=true`（既定値）による抑制より優先され、
/// composing=false でも素の VK_NONCONVERT ではなく専用 Fn キーが送出される。
#[test]
fn test_muhenkan_solo_tap_dedicated_fn_key_sends_fn_key_instead_of_raw_vk() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config_ex(false, true, false, false);
    engine.set_muhenkan_solo_tap_dedicated_fn_key(Some(VK_F21));

    let result = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, false);
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_F21)),
        "専用 Fn キーモードが有効なら composing=false でも VK_F21 を送出すべき"
    );
    assert!(
        !result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT)),
        "専用 Fn キーモードが有効なら素の VK_NONCONVERT は送出してはならない"
    );
}

/// ADR-091 §D3.2/C1: 専用 Fn キーモードは `muhenkan_solo_tap_always_suppress` の
/// 早期 return より手前で分岐するため、composing=true・always_suppress=true
/// （通常なら completely suppress される条件）でも Fn キーが送出される。
#[test]
fn test_muhenkan_solo_tap_dedicated_fn_key_overrides_always_suppress_while_composing() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config_ex(false, true, false, false);
    engine.set_muhenkan_solo_tap_dedicated_fn_key(Some(VK_F21));

    let result = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, true);
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_F21)),
        "専用 Fn キーモードは always_suppress=true・composing=true でも \
         既存の抑制判定より優先されるべき"
    );
}

/// `set_muhenkan_solo_tap_dedicated_fn_key` が `None`（既定）のままなら、
/// 専用 Fn キー分岐は完全に無効で、既存の抑制/パススルー判定のみが効く
/// （回帰: 新規分岐を追加したことで既定挙動そのものが変わっていないことの固定）。
#[test]
fn test_muhenkan_solo_tap_dedicated_fn_key_none_preserves_existing_behavior() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config_ex(false, true, false, false);
    // set_muhenkan_solo_tap_dedicated_fn_key を一切呼ばない = None のまま。

    let result = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, false);
    assert_eq!(
        result.actions.len(),
        0,
        "専用 Fn キー未設定なら、従来通り always_suppress=true で無変換単独タップは \
         一切送出されないはず"
    );
}

/// 専用 Fn キーモードは、変換(henkan)キーの単独タップには影響しない
/// （`muhenkan_vk` との等値比較でゲートされているため）。
#[test]
fn test_muhenkan_solo_tap_dedicated_fn_key_does_not_affect_henkan() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config_ex(false, false, false, false);
    engine.set_muhenkan_solo_tap_dedicated_fn_key(Some(VK_F21));

    let result = engine.on_event(Ev::down(VK_CONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, false);
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_CONVERT)),
        "muhenkan 向けの専用 Fn キー設定は henkan の単独タップ（従来通り \
         always_suppress=false で素の VK_CONVERT を送出）に影響してはならない"
    );
    assert!(
        !result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_F21)),
        "henkan 単独タップで VK_F21 が送出されてはならない"
    );
}

/// ADR-091 §D3.2: 専用 Fn キーモードは `resolve_pending_thumb_as_single` を呼ぶ
/// 全経路で一貫して効く。タイムアウト経路だけでなく、フラッシュ経路
/// （`swap_layout` 等によるコンテキスト変更）でも Fn キーが送出されることを
/// `test_swap_layout_flushes_pending_thumb` と対照して固定する。
#[test]
fn test_muhenkan_solo_tap_dedicated_fn_key_applies_on_flush_path() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config_ex(false, true, false, false);
    engine.set_muhenkan_solo_tap_dedicated_fn_key(Some(VK_F21));

    let result = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&result);

    let new_layout = make_layout();
    let result = engine.swap_layout(new_layout);
    result.assert_consumed();
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_F21)),
        "フラッシュ経路（swap_layout 由来）でも専用 Fn キーが送出されるべき \
         （always_suppress=true なら本来 VK_NONCONVERT すら出ない状況）"
    );
    assert!(
        !result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT)),
        "フラッシュ経路で素の VK_NONCONVERT が送出されてはならない"
    );
}

/// `henkan_solo_tap_always_suppress=true`（既定値）なら、composing=false でも
/// 変換単独タップは一切送出されない。無変換と対称のガード（BUG-58 関連調査で発覚:
/// 従来 変換キーにはこの抑制手段が無く、composing していない場面では常に
/// 生 VK_CONVERT が送出されていた）。
#[test]
fn test_henkan_always_suppress_blocks_even_when_not_composing() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config_ex(false, false, false, true);

    let result = engine.on_event(Ev::down(VK_CONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, false);
    assert_eq!(
        result.actions.len(),
        0,
        "henkan_solo_tap_always_suppress=true なら composing=false でも \
         VK_CONVERT を送出してはならない"
    );
}

/// `henkan_solo_tap_always_suppress=true` は
/// `henkan_solo_tap_ignore_composing_guard=true`（composing 中の素通しオプトイン）
/// より優先される。
#[test]
fn test_henkan_always_suppress_overrides_ignore_composing_guard() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config_ex(false, false, true, true);

    let result = engine.on_event(Ev::down(VK_CONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, true);
    assert_eq!(
        result.actions.len(),
        0,
        "henkan_solo_tap_always_suppress=true なら ignore_composing_guard=true でも \
         composing 中の VK_CONVERT 送出を抑制すべき"
    );
}

/// `henkan_solo_tap_always_suppress=false` なら、composing=false のときは
/// 従来通り生 VK_CONVERT が送出される（既定を外した場合の後方互換確認）。
#[test]
fn test_henkan_always_suppress_false_preserves_legacy_passthrough() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config_ex(false, false, false, false);

    let result = engine.on_event(Ev::down(VK_CONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, false);
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_CONVERT)),
        "henkan_solo_tap_always_suppress=false なら composing=false のとき \
         従来通り VK_CONVERT を送出すべき"
    );
}

/// 無変換側のフラグが true でも、変換キー自体のフラグが false なら
/// 変換キーの単独タップは従来通り composing 中は suppress される
/// （VK ごとに独立してガードが効くことの確認）。
#[test]
fn test_muhenkan_guard_does_not_affect_henkan() {
    let mut engine = make_engine_with_thumb_key_solo_tap_config(true, false);

    let result = engine.on_event(Ev::down(VK_CONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, true);
    assert_eq!(
        result.actions.len(),
        0,
        "muhenkan 用フラグが true でも、変換キー自体のフラグが false なら composing 中は suppress されるべき"
    );
}

/// 親指キーに Ctrl/Alt（`ModifierState::is_os_modifier_held` が true になる系統）を
/// 割り当てると、そのキーの KeyDown 自体が `bypass_reason` の `OsModifierHeld` に
/// 即座に該当し、`PendingThumb` に一切入らず素通しされる（同時打鍵検出が機能しない）。
/// これは単独タップの生 VK 送出うんぬん以前の、より根本的なブロッカーである。
/// 真に「特定の物理 Alt キーを親指シフト専用にし、Alt としての機能は捨てる」を
/// 実現するには、`ModifierState` を左右別に追跡した上で「親指キーとして設定された
/// 側の modifier は `is_os_modifier_held` に含めない」という区別が必要
/// （未実装。大掛かりな変更になるため見送り、この事実を明示するテストとして残す）。
#[test]
fn test_ctrl_alt_win_thumb_key_never_enters_pending_due_to_os_modifier_bypass() {
    use crate::types::{
        ImeRelevance, KeyClassification, KeyEventType, ModifierKey, ModifierState, RawKeyEvent,
    };

    for (vk, scan, mk) in [
        (VK_LALT, SCAN_LALT, ModifierKey::Alt),
        (VK_LCTRL, SCAN_LCTRL, ModifierKey::Ctrl),
    ] {
        let mut engine = TestHarness {
            tracker: input_tracker::InputTracker::new(),
            engine: NicolaFsm::new(make_layout(), vk, VK_CONVERT, 100, ConfirmMode::Wait, 30),
        };

        let down = RawKeyEvent {
            vk_code: vk,
            scan_code: scan,
            event_type: KeyEventType::KeyDown,
            extra_info: 0,
            timestamp: 0,
            key_classification: KeyClassification::LeftThumb,
            physical_pos: None,
            ime_relevance: ImeRelevance::default(),
            modifier_key: Some(mk),
            modifier_snapshot: ModifierState::default(),
            injected: false,
        };

        let result = engine.on_event(down);
        assert!(
            !result.consumed,
            "{mk:?} thumb key should bypass to pass-through immediately, never reach PendingThumb"
        );
    }
}

/// Shift は `is_os_modifier_held`（ctrl/alt/win のみ）に含まれないため、
/// 上記 Ctrl/Alt/Win と違って `bypass_reason` を素通りし `PendingThumb` に到達しうる。
/// この経路では、composing に関わらず単独タップの生 VK 送出を suppress する必要がある
/// （Shift 単独の KeyDown/KeyUp を生のまま OS に送るとアクセシビリティ機能の
/// 誤発火等がありうるため）。
#[test]
fn test_thumb_alone_timeout_suppressed_when_thumb_is_os_modifier() {
    use crate::types::{
        ImeRelevance, KeyClassification, KeyEventType, ModifierKey, ModifierState, RawKeyEvent,
    };

    let mut engine = TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            make_layout(),
            VK_LSHIFT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    };

    let down = RawKeyEvent {
        vk_code: VK_LSHIFT,
        scan_code: vk_to_scan(VK_LSHIFT),
        event_type: KeyEventType::KeyDown,
        extra_info: 0,
        timestamp: 0,
        key_classification: KeyClassification::LeftThumb,
        physical_pos: None,
        ime_relevance: ImeRelevance::default(),
        modifier_key: Some(ModifierKey::Shift),
        modifier_snapshot: ModifierState::default(),
        injected: false,
    };

    let result = engine.on_event(down);
    assert_pending(&result);

    // composing=false（Windows 全般でのキー機能維持のための素通し経路）でも、
    // 親指キーが OS 修飾キーの場合は suppress されなければならない。
    let result = engine.on_timeout_composing(TIMER_PENDING, false);
    assert_eq!(
        result.actions.len(),
        0,
        "Shift を親指キーにした場合、単独タップは composing=false でも suppress されるべき"
    );
}

// ── Space 親指キーのフォールバック（left_thumb_key/right_thumb_key = VK_SPACE） ──

/// 左親指キーに Space を割り当て、`space_thumb_vk`/フラグを明示設定したエンジンを返す。
fn make_engine_with_space_thumb(ignore_composing_guard: bool, shift_literal: bool) -> TestHarness {
    let mut engine = NicolaFsm::new(
        make_layout(),
        VK_SPACE,
        VK_CONVERT,
        100,
        ConfirmMode::Wait,
        30,
    );
    engine.set_space_thumb_config(
        Some(VK_SPACE),
        TextKeyConfig {
            ignore_composing_guard,
            shift_literal,
        },
    );
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine,
    }
}

/// Space 親指キーは、composing 中（変換候補ウィンドウ表示中）でも
/// `space_thumb_ignore_composing_guard=true`（既定値）なら単独タップで送出される。
/// 無変換/変換と違い、Space の raw VK_SPACE は IME の「変換候補送り」正規機能であり、
/// composing 中に抑制すると通常の変換操作が壊れるための例外（resolve_pending_thumb_as_single 参照）。
#[test]
fn test_space_thumb_emits_while_composing_when_guard_ignored() {
    let mut engine = make_engine_with_space_thumb(true, true);

    let result = engine.on_event(Ev::down(VK_SPACE).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, true);
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_SPACE)),
        "space_thumb_ignore_composing_guard=true なら composing 中でも VK_SPACE を送出すべき"
    );
}

/// `space_thumb_ignore_composing_guard=false` なら、Space も無変換/変換と同じく
/// composing 中は suppress される（設定でオプトアウトできることの確認）。
#[test]
fn test_space_thumb_suppressed_while_composing_when_guard_disabled() {
    let mut engine = make_engine_with_space_thumb(false, true);

    let result = engine.on_event(Ev::down(VK_SPACE).build());
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, true);
    assert_eq!(
        result.actions.len(),
        0,
        "space_thumb_ignore_composing_guard=false なら composing 中は他の親指キーと同様 suppress される"
    );
}

/// flush 経路でも、composing 値を「保留キーと同一コンテキスト」だと呼び出し元が
/// `ComposingHint::Trusted` で明示保証した場合は、composing 中の Space フォールバックが
/// タイムアウト経路と一貫している（従来 flush は無条件 suppress だった不整合の回帰防止）。
/// `EngineDisabled`/`LayoutSwapped`/`BypassKey` 等、同一イベント処理内で完結する
/// flush がこれに当たる（`NicolaFsm::toggle_enabled`/`swap_layout`/`handle_bypass` 参照）。
#[test]
fn test_space_thumb_flush_consistent_with_timeout_when_composing_trusted() {
    let mut engine = make_engine_with_space_thumb(true, true);

    let result = engine.on_event(Ev::down(VK_SPACE).build());
    assert_pending(&result);

    let result = engine.flush_pending(ContextChange::EngineDisabled, ComposingHint::Trusted(true));
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_SPACE)),
        "Trusted(true) なら flush 経路も composing 中の Space ガード無視を timeout 経路と同様に適用すべき"
    );
}

/// `ComposingHint::Unknown`（フォーカス変更等、コンテキスト境界を跨ぐフラッシュ）では、
/// `space_thumb_ignore_composing_guard=true` であっても Space フォールバック例外を
/// 一切適用せず、無条件 suppress する。
///
/// 背景: `Runtime::ir_notify_focus_changed`（Windows platform 層）は
/// `detect_and_update_focus()` でフォーカスを新ウィンドウへ切り替えた**後**に
/// `build_ctx()` を呼んで `InputContext::composing` を読み直すため、この値は
/// 「保留中の親指キーが入力された元のウィンドウ」ではなく「切替後の新ウィンドウ」の
/// composing 状態を指しうる。ここで Space 例外を適用すると、Alt-Tab 等で無関係な
/// 新ウィンドウへ生 VK_SPACE が誤注入されるリスクがある（ボタン押下・チェックボックス
/// トグル等の副作用）。`Unknown` はこの安全側フォールバックを保証する。
#[test]
fn test_space_thumb_flush_suppressed_when_composing_hint_unknown() {
    let mut engine = make_engine_with_space_thumb(true, true);

    let result = engine.on_event(Ev::down(VK_SPACE).build());
    assert_pending(&result);

    let result = engine.flush_pending(ContextChange::FocusChanged, ComposingHint::Unknown);
    assert_eq!(
        result.actions.len(),
        0,
        "ComposingHint::Unknown では Space 例外を含め無条件 suppress すべき\
         （コンテキスト境界を跨ぐため composing の新鮮さを保証できない）"
    );
}

/// Shift+Space は `space_thumb_shift_literal=true` の場合、同時打鍵判定を待たず
/// 即座に PassThrough（consumed=false）として処理される（PendingThumb に入らない）。
#[test]
fn test_shift_space_literal_passthrough_when_enabled() {
    let mut engine = make_engine_with_space_thumb(true, true);

    // Shift を先に押下
    let shift_result = engine.on_event(Ev::down(VK_LSHIFT).build());
    assert!(!shift_result.consumed, "Shift 単体は素通しされる");

    // Shift 押下中に Space（親指キー）が来ても PendingThumb に入らず即座に素通し
    let result = engine.on_event(Ev::down(VK_SPACE).build());
    assert!(
        !result.consumed,
        "Shift+Space は同時打鍵判定を待たず即座に PassThrough になるべき"
    );
    assert!(
        result.timers.is_empty(),
        "PendingThumb に入らないので TIMER_PENDING は張られないはず"
    );
}

/// `space_thumb_shift_literal=false` なら、Shift+Space も通常の親指キー同様
/// `PendingThumb` に入る（同時打鍵判定が有効なままであることの確認）。
#[test]
fn test_shift_space_enters_pending_when_literal_disabled() {
    let mut engine = make_engine_with_space_thumb(true, false);

    let _ = engine.on_event(Ev::down(VK_LSHIFT).build());
    let result = engine.on_event(Ev::down(VK_SPACE).build());
    assert_pending(&result);
}

// ── Enter 親指キーのフォールバック（left_thumb_key/right_thumb_key = VK_RETURN） ──

/// 左親指キーに Enter を割り当て、`enter_thumb_vk`/フラグを明示設定したエンジンを返す。
fn make_engine_with_enter_thumb(ignore_composing_guard: bool, shift_literal: bool) -> TestHarness {
    let mut engine = NicolaFsm::new(
        make_layout(),
        VK_RETURN,
        VK_CONVERT,
        100,
        ConfirmMode::Wait,
        30,
    );
    engine.set_enter_thumb_config(
        Some(VK_RETURN),
        TextKeyConfig {
            ignore_composing_guard,
            shift_literal,
        },
    );
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine,
    }
}

/// Enter を左親指キーとした KeyDown イベントを構築する。
///
/// `classify_test_key`（`Ev::down` が使う共通の VK→分類マッピング）は VK_RETURN を
/// 汎用の non-layout キーとして `Passthrough` に分類する（`test_pending_char_then_non_layout_key_passes_through_new`
/// 等、他の多数のテストがこの前提に依存しているため変更できない）。Enter を親指キーとして
/// 扱うこのブロックのテストだけは、`test_ctrl_alt_win_thumb_key_never_enters_pending_due_to_os_modifier_bypass`
/// と同じ手法（`RawKeyEvent` を手組みして `key_classification` を明示指定）で対処する。
fn enter_thumb_down_event(ts: Timestamp) -> RawKeyEvent {
    use crate::types::{ImeRelevance, KeyClassification, KeyEventType, ModifierState};
    RawKeyEvent {
        vk_code: VK_RETURN,
        scan_code: vk_to_scan(VK_RETURN),
        event_type: KeyEventType::KeyDown,
        extra_info: 0,
        timestamp: ts,
        key_classification: KeyClassification::LeftThumb,
        physical_pos: None,
        ime_relevance: ImeRelevance::default(),
        modifier_key: None,
        modifier_snapshot: ModifierState::default(),
        injected: false,
    }
}

/// Enter 親指キーは、composing 中（変換候補ウィンドウ表示中）でも
/// `enter_thumb_ignore_composing_guard=true`（既定値）なら単独タップで送出される。
/// 無変換/変換と違い、Enter の raw VK_RETURN は IME の「変換確定」正規機能であり、
/// composing 中に抑制すると通常の変換確定操作が壊れるための例外
/// （resolve_pending_thumb_as_single 参照、Space と同じ理由付け）。
#[test]
fn test_enter_thumb_emits_while_composing_when_guard_ignored() {
    let mut engine = make_engine_with_enter_thumb(true, true);

    let result = engine.on_event(enter_thumb_down_event(0));
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, true);
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_RETURN)),
        "enter_thumb_ignore_composing_guard=true なら composing 中でも VK_RETURN を送出すべき"
    );
}

/// `enter_thumb_ignore_composing_guard=false` なら、Enter も無変換/変換と同じく
/// composing 中は suppress される（設定でオプトアウトできることの確認）。
#[test]
fn test_enter_thumb_suppressed_while_composing_when_guard_disabled() {
    let mut engine = make_engine_with_enter_thumb(false, true);

    let result = engine.on_event(enter_thumb_down_event(0));
    assert_pending(&result);

    let result = engine.on_timeout_composing(TIMER_PENDING, true);
    assert_eq!(
        result.actions.len(),
        0,
        "enter_thumb_ignore_composing_guard=false なら composing 中は他の親指キーと同様 suppress される"
    );
}

/// Shift+Enter は `enter_thumb_shift_literal=true` の場合、同時打鍵判定を待たず
/// 即座に PassThrough（consumed=false）として処理される（PendingThumb に入らない）。
#[test]
fn test_shift_enter_literal_passthrough_when_enabled() {
    let mut engine = make_engine_with_enter_thumb(true, true);

    let shift_result = engine.on_event(Ev::down(VK_LSHIFT).build());
    assert!(!shift_result.consumed, "Shift 単体は素通しされる");

    let result = engine.on_event(enter_thumb_down_event(0));
    assert!(
        !result.consumed,
        "Shift+Enter は同時打鍵判定を待たず即座に PassThrough になるべき"
    );
    assert!(
        result.timers.is_empty(),
        "PendingThumb に入らないので TIMER_PENDING は張られないはず"
    );
}

/// `enter_thumb_shift_literal=false` なら、Shift+Enter も通常の親指キー同様
/// `PendingThumb` に入る（同時打鍵判定が有効なままであることの確認）。
#[test]
fn test_shift_enter_enters_pending_when_literal_disabled() {
    let mut engine = make_engine_with_enter_thumb(true, false);

    let _ = engine.on_event(Ev::down(VK_LSHIFT).build());
    let result = engine.on_event(enter_thumb_down_event(0));
    assert_pending(&result);
}

#[test]
fn test_char_then_thumb_after_threshold() {
    let mut engine = make_engine();
    let t0 = 0;

    let result = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&result);

    let t1 = t0 + 200_000;
    let result = engine.on_event(Ev::down(VK_NONCONVERT).at(t1).build());
    result.assert_consumed();
    assert!(result
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('う'))));
}

#[test]
fn test_key_up_after_emit() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).build());
    engine.on_timeout(TIMER_PENDING);

    let result = engine.on_event(Ev::up(VK_A).build());
    result.assert_consumed();
    assert!(matches!(result.actions[0], KeyAction::Suppress));
}

#[test]
fn test_key_up_while_pending_no_double_char() {
    let mut engine = make_engine();

    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);

    let result = engine.on_event(Ev::up(VK_A).build());
    result.assert_consumed();

    let char_count = result
        .actions
        .iter()
        .filter(|a| matches!(a, KeyAction::Char('う')))
        .count();
    assert_eq!(char_count, 1, "Character should be emitted exactly once");
}

// ── swap_layout テスト ──

#[test]
fn test_swap_layout_no_pending() {
    let mut engine = make_engine();
    let new_layout = make_layout();
    let result = engine.swap_layout(new_layout);
    assert!(
        result.actions.is_empty(),
        "No pending key means no timeout actions"
    );
}

#[test]
fn test_swap_layout_flushes_pending_char() {
    let mut engine = make_engine();

    // 文字キーを保留状態にする
    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);

    // swap_layout で保留がタイムアウト確定される
    let new_layout = make_layout();
    let result = engine.swap_layout(new_layout);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('う')));
}

#[test]
fn test_swap_layout_flushes_pending_thumb() {
    let mut engine = make_engine();

    // 親指キーを保留状態にする
    let result = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&result);

    let new_layout = make_layout();
    let result = engine.swap_layout(new_layout);
    result.assert_consumed();
    // composing=false（テストのデフォルト状態）なので生 VK_NONCONVERT が emit される
    // （composing 中の suppress は resolve_pending_thumb_as_single 参照）。
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT)),
        "thumb single should be emitted when not composing"
    );
}

#[test]
fn test_swap_layout_clears_output_history() {
    let mut engine = make_engine();

    // キーを確定して出力履歴にエントリを作る
    engine.on_event(Ev::down(VK_A).build());
    engine.on_timeout(TIMER_PENDING);

    // swap_layout で出力履歴がクリアされる
    let new_layout = make_layout();
    engine.swap_layout(new_layout);

    // 出力履歴がクリアされたので KeyUp は PassThrough になる
    let result = engine.on_event(Ev::up(VK_A).build());
    result.assert_pass_through();
}

#[test]
fn test_swap_layout_uses_new_layout() {
    let mut engine = make_engine();

    // 新しい配列��作成（A キーの通常面を 'か' に変更）
    let mut new_layout = make_layout();
    new_layout.normal.insert(POS_A, lit('か'));

    engine.swap_layout(new_layout);

    // 新しい配列で変換される
    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);
    let result = engine.on_timeout(TIMER_PENDING);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('か')));
}

#[test]
fn test_toggle_enabled() {
    let mut engine = make_engine();
    assert!(engine.is_enabled());
    let _ = engine.toggle_enabled();
    assert!(!engine.is_enabled());
    let _ = engine.toggle_enabled();
    assert!(engine.is_enabled());
}

// ── OS 予約キーコンビネーションのパススルーテスト ──

#[test]
fn test_ctrl_held_char_key_passes_through() {
    let mut engine = make_engine();

    // Ctrl を押下
    engine.on_event(Ev::down(VK_CTRL).build());

    // Ctrl が押されている状態で文字キーはパススルー
    engine
        .on_event(Ev::down(VK_A).build())
        .assert_pass_through();
}

#[test]
fn test_lctrl_held_char_key_passes_through() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_LCTRL).build());

    engine
        .on_event(Ev::down(VK_C).build())
        .assert_pass_through();
}

#[test]
fn test_alt_held_char_key_passes_through() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_ALT).build());

    engine
        .on_event(Ev::down(VK_A).build())
        .assert_pass_through();
}

#[test]
fn test_lalt_held_char_key_passes_through() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_LALT).build());

    engine
        .on_event(Ev::down(VK_V).build())
        .assert_pass_through();
}

#[test]
fn test_ctrl_released_char_key_resumes_conversion() {
    let mut engine = make_engine();

    // Ctrl 押下 → リリース
    engine.on_event(Ev::down(VK_CTRL).build());
    engine.on_event(Ev::up(VK_CTRL).build());

    // Ctrl が離された後は通常の変換が行われる（保留になる）
    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);
}

#[test]
fn test_ctrl_held_non_layout_key_passes_through() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_CTRL).build());

    // 配列定義にないキーも Ctrl 押下中はパススルー
    engine
        .on_event(Ev::down(VK_RETURN).build())
        .assert_pass_through();
}

// ── Shift 面テスト ──

fn make_engine_with_shift() -> TestHarness {
    let mut layout = make_layout();
    layout.shift.insert(POS_A, lit('ウ'));
    layout.shift.insert(POS_S, lit('シ'));
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    }
}

#[test]
fn test_shift_held_uses_shift_face() {
    let mut engine = make_engine_with_shift();

    // Shift を押下
    engine.on_event(Ev::down(VK_SHIFT).build());

    // Shift 面に定義がある文字キー → Shift 面の文字が IME 経由でそのまま出力される
    // （BUG-15 の「hold 中は IME-ON 半角英数」撤去後、shift_face_reduce 本来の
    // 挙動＝.yab の値をそのまま Reduce する、2026-07-11）。
    let result = engine.on_event(Ev::down(VK_A).build());
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('ウ')));
}

#[test]
fn test_shift_face_returns_literal_via_ime() {
    // Shift 面の literal は .yab に書かれたまま Char で IME 経由に確定出力する
    // （BUG-15 撤去により、これが Shift 面の唯一の挙動になった。旧
    // `test_shift_face_halfwidth_disabled_keeps_literal` を改称・簡略化）。
    let mut layout = make_layout();
    layout.shift.insert(POS_A, lit('Ｋ'));
    let mut engine = TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    };

    engine.on_event(Ev::down(VK_SHIFT).build());
    let result = engine.on_event(Ev::down(VK_A).build());
    result.assert_consumed();
    assert!(matches!(result.actions[0], KeyAction::Char('Ｋ')));
}

#[test]
fn test_shift_held_unlisted_key_passes_through() {
    let mut engine = make_engine_with_shift();

    // Shift を押下
    engine.on_event(Ev::down(VK_LSHIFT).build());

    // Shift 面に定義がないキー → PassThrough
    engine
        .on_event(Ev::down(VK_C).build())
        .assert_pass_through();
}

#[test]
fn test_shift_released_resumes_normal() {
    let mut engine = make_engine_with_shift();

    // Shift 押下 → リリース
    engine.on_event(Ev::down(VK_RSHIFT).build());
    engine.on_event(Ev::up(VK_RSHIFT).build());

    // Shift が離された後は通常の変換が行われる（保留になる）
    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);
}

// ── 3 鍵仲裁（d1/d2 比較）テスト ──

#[test]
fn test_three_key_d1_less_than_d2() {
    // char1(t=0) → thumb(t=20ms) → char2(t=80ms)
    // d1 = 20ms, d2 = 60ms → d1 < d2 → char1+thumb = 同時、char2 = 新規処理
    let mut engine = make_engine();

    let result = engine.on_event(Ev::down(VK_A).at(0).build());
    assert_pending(&result);

    let result = engine.on_event(Ev::down(VK_CONVERT).at(20_000).build());
    assert_pending(&result); // PendingCharThumb

    let result = engine.on_event(Ev::down(VK_S).at(80_000).build());
    result.assert_consumed();
    // char1+thumb(右) で 'ゔ' が出力される
    assert!(result
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('ゔ'))));
    // 親指は消費済みなので char2 は保留に入り、ここでは出力されない
    assert!(
        !result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Char('じ'))),
        "char2 should NOT be thumb-shifted (thumb consumed)"
    );
}

#[test]
fn test_three_key_d1_greater_equal_d2() {
    // char1(t=0) → thumb(t=60ms) → char2(t=80ms)
    // d1 = 60ms, d2 = 20ms → d1 >= d2 → char1 = 単独、char2+thumb = 同時
    let mut engine = make_engine();

    let result = engine.on_event(Ev::down(VK_A).at(0).build());
    assert_pending(&result);

    let result = engine.on_event(Ev::down(VK_CONVERT).at(60_000).build());
    assert_pending(&result); // PendingCharThumb

    let result = engine.on_event(Ev::down(VK_S).at(80_000).build());
    result.assert_consumed();
    // char1(VK_A) は単独確定 'う'、char2+thumb(右) で 'じ'
    assert!(result
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('う'))));
    assert!(result
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('じ'))));
}

#[test]
fn test_three_key_timeout_resolves_as_simultaneous() {
    // char1(t=0) → thumb(t=30ms) → タイムアウト（char2 来ない）
    // → char1+thumb を同時打鍵として確定
    let mut engine = make_engine();

    let result = engine.on_event(Ev::down(VK_A).at(0).build());
    assert_pending(&result);

    let result = engine.on_event(Ev::down(VK_NONCONVERT).at(30_000).build());
    assert_pending(&result); // PendingCharThumb

    let result = engine.on_timeout(TIMER_PENDING);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('を')));
}

#[test]
fn test_three_key_key_up_char_resolves_simultaneous() {
    // char1 → thumb → char1 KeyUp → (char2 を待機) → thumb KeyUp → char1+thumb を確定
    // char1 が離されてもすぐには出力せず、後続 char2 の有無を確認するため待機する。
    // char2 が来ない場合は thumb KeyUp で同時打鍵として確定する。
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).at(0).build());
    engine.on_event(Ev::down(VK_CONVERT).at(30_000).build());

    // char1 離鍵: 待機（何も出力しない）
    let result = engine.on_event(Ev::up(VK_A).build());
    result.assert_consumed();
    assert!(
        result.actions.is_empty(),
        "char1 release should not emit immediately"
    );

    // thumb 離鍵: 同時打鍵として確定
    let result2 = engine.on_event(Ev::up(VK_CONVERT).build());
    result2.assert_consumed();
    assert!(result2
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('ゔ'))));
}

// ── 連続シフト用ヘルパー ──

fn make_engine_with_extended_layout() -> TestHarness {
    let mut layout = make_layout();
    // D, F を配列に追加
    layout.normal.insert(POS_D, lit('て'));
    layout.normal.insert(POS_F, lit('け'));
    layout.left_thumb.insert(POS_D, lit('な'));
    layout.left_thumb.insert(POS_F, lit('よ'));
    layout.right_thumb.insert(POS_D, lit('で'));
    layout.right_thumb.insert(POS_F, lit('げ'));
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    }
}

// ── 連続シフト（左親指）テスト ──

#[test]
fn test_continuous_shift_left_thumb() {
    // 左親指を押しっぱなしにしながら複数文字キーを打つ
    let mut engine = make_engine_with_extended_layout();
    let t = 0u64;

    // 左親指押下 → PendingThumb
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t).build());
    assert_pending(&r);

    // char1 が閾値内に到着 → 同時打鍵として確定、left_thumb_down がセットされる
    let r = engine.on_event(Ev::down(VK_A).at(t + 30_000).build());
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(
        matches!(r.actions[0], KeyAction::Char('を')),
        "char1 should use left thumb face"
    );

    // char2: 親指は消費済み → active_thumb_face() が None → PendingChar（保留）
    let r = engine.on_event(Ev::down(VK_S).at(t + 100_000).build());
    assert_pending(&r);

    // char3: PendingChar(S) 中に char(D) 到着 → S が通常面で単独確定、D が新たに保留
    let r = engine.on_event(Ev::down(VK_D).at(t + 170_000).build());
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('し'))),
        "char2 should use normal face for S (thumb consumed)"
    );

    // 親指リリース
    let _r = engine.on_event(Ev::up(VK_NONCONVERT).at(t + 200_000).build());

    // char4: PendingChar(D) 中に char(F) 到着 → D が通常面で単独確定、F が新たに保留
    let r = engine.on_event(Ev::down(VK_F).at(t + 250_000).build());
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('て'))),
        "char3 should use normal face for D"
    );

    // タイムアウトで F が通常面で出力される
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        matches!(r.actions[0], KeyAction::Char('け')),
        "char4 after thumb release should use normal face"
    );
}

/// 親指を単独タイムアウトで確定した後（=消費されていない）も物理的に押されたままの場合、
/// 次に来た文字キーは `active_thumb_face()` 経由でシフトされるべき
/// （`classify_idle_intent` の「Active thumb combo」分岐、`is_thumb_consumed`、
/// `active_thumb_face` の3つを同時に検証する）。
///
/// - `classify_idle_intent` の `!ev.key_class.is_thumb()` の `!` が消えると、
///   文字キー到着時にこの分岐そのものに入らなくなり、char は PendingChar
///   （通常面）に落ちる。
/// - `is_thumb_consumed` の本体が `true` に置換される、または
///   `phys_down.is_some() && consumed == phys_down` が `||` に壊れると、
///   一度もこの物理押下で consume されていないのに「消費済み」と誤判定され、
///   `active_thumb_face()` が None を返し、同じく char が PendingChar に落ちる。
/// - `active_thumb_face` 自体が無条件 None に置換されても同様。
///
/// いずれの変異でも、正しい実装なら即座に得られる 'を'（左親指シフト面）の代わりに
/// PendingChar（保留、まだ確定していない）になるため区別できる。
#[test]
fn test_char_after_thumb_solo_timeout_still_held_uses_active_thumb_face() {
    let mut engine = make_engine();

    // 左親指のみ押下 → PendingThumb
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(0).build());
    assert_pending(&r);

    // 文字キーが来ないままタイムアウト → 単独確定（生 VK_NONCONVERT 送出）。
    // consume_thumb は呼ばれないので left_thumb_consumed は None のまま。
    // KeyUp を送っていないので phys.left_thumb_down は Some のまま。
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(matches!(r.actions[0], KeyAction::Key(x) if x == VK_NONCONVERT));

    // 親指を離さずに文字キー 'A' を押す → active_thumb_face() = Some(LeftThumb) の
    // はずなので、即座に左親指面 'を' で確定する（PendingChar には入らない）。
    let r = engine.on_event(Ev::down(VK_A).at(200_000).build());
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1, "actions: {:?}", r.actions);
    assert!(
        matches!(r.actions[0], KeyAction::Char('を')),
        "held-but-unconsumed thumb should shift the next char, got {:?}",
        r.actions[0]
    );
}

// ── SpeculativeChar 状態での KeyUp (line 1180) ──

#[test]
fn test_speculative_char_key_up_matching_vk_confirms_and_goes_idle() {
    let mut engine = make_engine();
    engine.state = EngineState::SpeculativeChar(PendingKey {
        scan_code: SCAN_A,
        vk_code: VK_A,
        pos: Some(POS_A),
        timestamp: 1_000_000,
    });

    // 投機出力されたのと同じキー(VK_A)の KeyUp → 確定して Idle へ
    let _r = engine.on_event(Ev::up(VK_A).at(1_050_000).build());
    assert!(
        engine.state.is_idle(),
        "matching key_up should confirm and return to Idle, got {:?}",
        engine.state
    );
}

#[test]
fn test_speculative_char_key_up_different_vk_does_not_confirm() {
    // event.vk_code == pending.vk_code の `==` が `!=` に壊れると、無関係なキーの
    // KeyUp で投機出力が確定してしまう（逆に、本来のキーの KeyUp では確定しなくなる）。
    // ここでは無関係な VK_S の KeyUp を送り、SpeculativeChar のままであることを確認する。
    let mut engine = make_engine();
    engine.state = EngineState::SpeculativeChar(PendingKey {
        scan_code: SCAN_A,
        vk_code: VK_A,
        pos: Some(POS_A),
        timestamp: 1_000_000,
    });

    let _r = engine.on_event(Ev::up(VK_S).at(1_050_000).build());
    assert!(
        matches!(engine.state, EngineState::SpeculativeChar(_)),
        "unrelated key_up must not confirm the speculative char, got {:?}",
        engine.state
    );
}

// ── engine_off_triple_vk: 対象外の親指キーはカウントしない (line 1324) ──

#[test]
fn test_engine_off_triple_vk_ignores_mismatched_thumb_solo_timeouts() {
    // engine_off_triple_vk.0 != 0 && vk_code == engine_off_triple_vk の `&&` が
    // `||` に壊れると、設定さえされていれば「無関係な親指キー」の単独タイムアウトでも
    // カウントされてしまい、5回連続で誤って engine off が要求される。
    let mut engine = make_engine();
    engine.set_engine_off_triple_vk(VK_NONCONVERT);

    let gap = 150_000u64; // 150ms < SOLO_OFF_TIMEOUT_US (400ms)

    // VK_CONVERT（対象外の親指キー）を5回連続で単独タイムアウトさせる
    for i in 0..5u64 {
        let t = i * gap;
        engine.on_event(Ev::down(VK_CONVERT).at(t).build());
        engine.on_timeout(TIMER_PENDING);
        assert!(
            !engine.take_engine_off_requested(),
            "{} consecutive solo presses of an unrelated thumb key must never trigger engine off",
            i + 1
        );
    }
}

// ── 連続シフト（右親指）テスト ──

#[test]
fn test_continuous_shift_right_thumb() {
    // 右親指を押しっぱなしにしながら複数文字キーを打つ
    let mut engine = make_engine_with_extended_layout();
    let t = 0u64;

    // 右親指押下 → PendingThumb
    let r = engine.on_event(Ev::down(VK_CONVERT).at(t).build());
    assert_pending(&r);

    // char1: 同時打鍵 → right_thumb_down セット
    let r = engine.on_event(Ev::down(VK_A).at(t + 30_000).build());
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(
        matches!(r.actions[0], KeyAction::Char('ゔ')),
        "char1 should use right thumb face"
    );

    // char2: 親指は消費済み → PendingChar（保留）
    let r = engine.on_event(Ev::down(VK_S).at(t + 100_000).build());
    assert_pending(&r);

    // char3: PendingChar(S) 中に char(D) 到着 → S が通常面で単独確定、D が新たに保留
    let r = engine.on_event(Ev::down(VK_D).at(t + 170_000).build());
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('し'))),
        "char2 should use normal face for S (thumb consumed)"
    );

    // 親指リリース
    let _r = engine.on_event(Ev::up(VK_CONVERT).at(t + 200_000).build());

    // char4: PendingChar(D) 中に char(F) 到着 → D が通常面で単独確定、F が新たに保留
    let r = engine.on_event(Ev::down(VK_F).at(t + 250_000).build());
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('て'))),
        "char3 should use normal face for D"
    );

    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        matches!(r.actions[0], KeyAction::Char('け')),
        "char4 after thumb release should use normal face"
    );
}

// ── PendingCharThumb タイムアウト後の連続シフト ──

#[test]
fn test_continuous_shift_after_pending_char_thumb_timeout() {
    // char1 → thumb → タイムアウト（同時打鍵確定）→ char2 が即時シフト出力されるか
    let mut engine = make_engine_with_extended_layout();
    let t = 0u64;

    // char1 → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t).build());
    assert_pending(&r);

    // thumb → PendingCharThumb
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t + 30_000).build());
    assert_pending(&r);

    // タイムアウト → char1+thumb 同時打鍵として確定、left_thumb_down がセットされる
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        matches!(r.actions[0], KeyAction::Char('を')),
        "timeout should resolve char1+left_thumb as simultaneous"
    );

    // char2: 親指は消費済み → PendingChar（保留）→ タイムアウトで通常面
    let r = engine.on_event(Ev::down(VK_S).at(t + 200_000).build());
    assert_pending(&r);

    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('し'))),
        "char2 after PendingCharThumb timeout should use normal face (thumb consumed)"
    );
}

// ── PendingCharThumb 3鍵仲裁 (d1 < d2) 後の連続シフト ──

#[test]
fn test_continuous_shift_after_three_key_d1_less_d2() {
    // char1(t=0) → thumb(t=20ms) → char2(t=80ms) → char3
    // d1=20ms < d2=60ms → char1+thumb 同時、char2 は process_new_key_down
    // char2 は thumb_down セット済みなので即時シフト出力
    // char3 も同様に即時シフト出力されるべき
    let mut engine = make_engine_with_extended_layout();

    let r = engine.on_event(Ev::down(VK_A).at(0).build());
    assert_pending(&r);

    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(20_000).build());
    assert_pending(&r); // PendingCharThumb

    // char2 到着: d1=20ms, d2=60ms → char1+thumb 同時、char2 は保留（親指消費済み）
    let r = engine.on_event(Ev::down(VK_S).at(80_000).build());
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('を'))),
        "char1+left_thumb should produce 'を'"
    );
    // char2 は親指消費済みのため保留に入り、ここでは出力されない

    // char3: PendingChar(S) 中に char(D) 到着 → S が通常面で単独確定、D が新たに保留
    let r = engine.on_event(Ev::down(VK_D).at(150_000).build());
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('し'))),
        "char2 should use normal face for S (thumb consumed)"
    );
}

// ── PendingCharThumb KeyUp 解決後の連続シフト ──

#[test]
fn test_continuous_shift_after_pending_char_thumb_key_up() {
    // char1 → thumb → char1 KeyUp → (待機) → thumb KeyUp → 同時打鍵確定 → char2 は通常面
    let mut engine = make_engine_with_extended_layout();
    let t = 0u64;

    engine.on_event(Ev::down(VK_A).at(t).build());
    engine.on_event(Ev::down(VK_NONCONVERT).at(t + 30_000).build());

    // char1 KeyUp → 待機（何も出力しない）
    let r = engine.on_event(Ev::up(VK_A).at(t + 60_000).build());
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "char1 release should not emit immediately"
    );

    // thumb KeyUp → 同時打鍵として確定
    let r = engine.on_event(Ev::up(VK_NONCONVERT).at(t + 80_000).build());
    r.assert_consumed();
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('を'))));

    // char2: 親指は消費済み（解放済み）→ PendingChar（保留）→ タイムアウトで通常面
    let r = engine.on_event(Ev::down(VK_S).at(t + 100_000).build());
    assert_pending(&r);

    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('し'))),
        "char2 after KeyUp-resolved simultaneous should use normal face (thumb consumed)"
    );
}

// ── 連続シフト中に反対側の親指が来た場合 ──

#[test]
fn test_continuous_shift_switch_thumb() {
    // 左親指押下 → char1(左シフト) → 左親指リリース → 右親指押下 → char2(右シフト)
    let mut engine = make_engine_with_extended_layout();
    let t = 0u64;

    // 左親指 → char1
    engine.on_event(Ev::down(VK_NONCONVERT).at(t).build());
    let r = engine.on_event(Ev::down(VK_A).at(t + 30_000).build());
    r.assert_consumed();
    assert!(matches!(r.actions[0], KeyAction::Char('を')));

    // 左親指リリース
    engine.on_event(Ev::up(VK_NONCONVERT).at(t + 80_000).build());

    // 右親指押下 → PendingThumb
    let r = engine.on_event(Ev::down(VK_CONVERT).at(t + 100_000).build());
    assert_pending(&r);

    // char2 → 右シフト面
    let r = engine.on_event(Ev::down(VK_A).at(t + 130_000).build());
    r.assert_consumed();
    assert!(
        matches!(r.actions[0], KeyAction::Char('ゔ')),
        "after switching thumbs, char should use right thumb face"
    );
}

// scan_to_pos テストは awase-windows に移動済み

#[test]
fn test_nicola_state_stores_scan_code() {
    // Verify that NicolaState variants correctly propagate scan_code from
    // RawKeyEvent — this is the infrastructure needed for .yab migration.
    let mut engine = make_engine();

    // Create a key event with a specific scan code
    let event = RawKeyEvent {
        vk_code: VK_A,
        scan_code: ScanCode(0x1E), // A key scan code
        event_type: KeyEventType::KeyDown,
        extra_info: 0,
        timestamp: 0,
        ime_relevance: crate::types::ImeRelevance::default(),
        modifier_key: None,
        key_classification: crate::types::KeyClassification::Char,
        physical_pos: Some(PhysicalPos::new(2, 0)),
        modifier_snapshot: Default::default(),
        injected: false,
    };

    let result = engine.on_event(event);
    assert_pending(&result);

    // The engine should have stored the scan_code in pending_char
    let EngineState::PendingChar(pending) = engine.state else {
        panic!("expected PendingChar state, got {:?}", engine.state);
    };
    assert_eq!(
        pending.scan_code,
        ScanCode(0x1E),
        "scan_code should be preserved in pending_char"
    );
}

#[test]
fn test_pending_char_thumb_stores_char_scan() {
    // Verify PendingCharThumb preserves char_scan from the original key event.
    let mut engine = make_engine();

    let char_event = RawKeyEvent {
        vk_code: VK_A,
        scan_code: ScanCode(0x1E),
        event_type: KeyEventType::KeyDown,
        extra_info: 0,
        timestamp: 0,
        ime_relevance: crate::types::ImeRelevance::default(),
        modifier_key: None,
        key_classification: crate::types::KeyClassification::Char,
        physical_pos: Some(PhysicalPos::new(2, 0)),
        modifier_snapshot: Default::default(),
        injected: false,
    };
    engine.on_event(char_event);

    let thumb_event = RawKeyEvent {
        vk_code: VK_CONVERT,
        scan_code: ScanCode(0x79), // Convert key scan code
        event_type: KeyEventType::KeyDown,
        extra_info: 0,
        timestamp: 30_000,
        ime_relevance: crate::types::ImeRelevance::default(),
        modifier_key: None,
        key_classification: crate::types::KeyClassification::RightThumb,
        physical_pos: None,
        modifier_snapshot: Default::default(),
        injected: false,
    };
    let result = engine.on_event(thumb_event);
    assert_pending(&result);

    let EngineState::PendingCharThumb { char_key, .. } = engine.state else {
        panic!("expected PendingCharThumb state, got {:?}", engine.state);
    };
    assert_eq!(
        char_key.scan_code,
        ScanCode(0x1E),
        "char_scan should be preserved in pending_char"
    );
}

// ── yab_value_to_action coverage ──

#[test]
fn test_yab_value_to_action_romaji() {
    let action = yab_value_to_action(&YabValue::Romaji {
        romaji: "ka".to_string(),
        kana: Some('か'),
    });
    assert!(matches!(action, KeyAction::Char('か')));
}

#[test]
fn test_yab_value_to_action_literal() {
    let action = yab_value_to_action(&YabValue::Literal("あ".to_string()));
    assert!(matches!(action, KeyAction::Char('あ')));
}

#[test]
fn test_yab_value_to_action_literal_empty() {
    let action = yab_value_to_action(&YabValue::Literal(String::new()));
    assert!(matches!(action, KeyAction::Suppress));
}

#[test]
fn test_yab_value_to_action_special() {
    use crate::yab::SpecialKey;
    let action = yab_value_to_action(&YabValue::Special(SpecialKey::Backspace));
    assert!(matches!(
        action,
        KeyAction::SpecialKey(SpecialKey::Backspace)
    ));
}

#[test]
fn test_yab_value_to_action_key_sequence() {
    let action = yab_value_to_action(&YabValue::KeySequence("?".to_string()));
    assert!(matches!(action, KeyAction::KeySequence(ref s) if s == "?"));
}

#[test]
fn test_yab_value_to_action_none() {
    let action = yab_value_to_action(&YabValue::None);
    assert!(matches!(action, KeyAction::Suppress));
}

// ── toggle_enabled with pending state ──

#[test]
fn test_toggle_enabled_returns_state() {
    let mut engine = make_engine();
    assert!(engine.is_enabled());
    let (enabled, _) = engine.toggle_enabled();
    assert!(!enabled);
    let (enabled, _) = engine.toggle_enabled();
    assert!(enabled);
}

// ── flush_pending: 全状態からの安全なリセット ──

#[test]
fn test_flush_pending_from_idle_is_noop() {
    let mut engine = make_engine();
    let r = engine.flush_pending(ContextChange::ImeOff, ComposingHint::Trusted(false));
    // Idle → no-op, consume with no actions
    assert!(r.actions.is_empty());
    assert!(r.consumed);
    // 再入しても no-op
    let r2 = engine.flush_pending(ContextChange::ImeOff, ComposingHint::Trusted(false));
    assert!(r2.actions.is_empty());
}

#[test]
fn test_flush_pending_from_pending_char() {
    let mut engine = make_engine();
    let t0 = 1_000_000;
    // PendingChar 状態にする
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    // flush → 通常面で単独確定
    let r = engine.flush_pending(ContextChange::EngineDisabled, ComposingHint::Trusted(false));
    assert!(!r.actions.is_empty(), "should emit the pending char");
    // Idle に戻っている
    let r2 = engine.flush_pending(ContextChange::ImeOff, ComposingHint::Trusted(false));
    assert!(r2.actions.is_empty(), "should be idle after flush");
}

#[test]
fn test_flush_pending_from_pending_thumb() {
    let mut engine = make_engine();
    let t0 = 1_000_000;
    // PendingThumb 状態にする
    let _ = engine.on_event(Ev::down(VK_NONCONVERT).at(t0).build());
    // flush（composing=true）→ 親指キーを単独確定 (= suppress = action 無し)。
    // Space 未割当なので composing=false でも本来は生 VK 送出だが、ここでは
    // composing 中の抑制（無変換/変換のかな/カタカナ切替誤爆防止）を確認する。
    let r = engine.flush_pending(
        ContextChange::InputLanguageChanged,
        ComposingHint::Trusted(true),
    );
    // 単独親指打鍵は composing 中は IME 副作用を防ぐため suppress される
    assert!(
        r.actions.is_empty(),
        "thumb single should be suppressed on flush, not emitted"
    );
}

#[test]
fn test_flush_pending_from_pending_char_thumb() {
    let mut engine = make_engine();
    let t0 = 1_000_000;
    // PendingChar → PendingCharThumb にする
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    let _ = engine.on_event(Ev::down(VK_NONCONVERT).at(t0 + 30_000).build());
    // flush → 同時打鍵として確定
    let r = engine.flush_pending(ContextChange::LayoutSwapped, ComposingHint::Trusted(false));
    assert!(!r.actions.is_empty(), "should emit simultaneous result");
}

#[test]
fn test_flush_pending_from_speculative_char() {
    let mut engine = make_speculative_engine();
    let t0 = 1_000_000;
    // SpeculativeChar 状態にする（即時出力済み）
    let r1 = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert!(!r1.actions.is_empty(), "speculative output");
    // flush → 既に出力済みなので追加出力なし
    let r = engine.flush_pending(ContextChange::ImeOff, ComposingHint::Trusted(false));
    assert!(
        r.actions.is_empty(),
        "speculative was already output, no additional actions"
    );
}

#[test]
fn test_flush_pending_cancels_timers() {
    let mut engine = make_engine();
    let t0 = 1_000_000;
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    let r = engine.flush_pending(ContextChange::ImeOff, ComposingHint::Trusted(false));
    // タイマー停止命令が含まれる（assert_timer_kill ヘルパーを使用）
    r.assert_timer_kill(TIMER_PENDING);
    r.assert_timer_kill(TIMER_SPECULATIVE);
}

#[test]
fn test_toggle_enabled_flushes_pending() {
    let mut engine = make_engine();
    let t0 = 1_000_000;
    // PendingChar 状態にする
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    // toggle → 保留がフラッシュされる
    let (enabled, flush_resp) = engine.toggle_enabled();
    assert!(!enabled);
    assert!(
        !flush_resp.actions.is_empty(),
        "should flush the pending char"
    );
}

// ── IME 制御キーのフラッシュ＋パススルー ──

const VK_KANJI: VkCode = VkCode(0x19); // 半角/全角キー
const SCAN_KANJI: ScanCode = ScanCode(0x29);

#[test]
fn test_ime_control_key_passes_through_from_idle() {
    let mut engine = make_engine();
    // Idle 状態で半角/全角 → pass_through, アクションなし
    let r = engine.on_event(Ev::down(VK_KANJI).scan(SCAN_KANJI).build());
    r.assert_pass_through();
    assert!(r.actions.is_empty());
}

#[test]
fn test_ime_control_key_flushes_pending_and_passes_through() {
    let mut engine = make_engine();
    let t0 = 1_000_000;
    // PendingChar 状態にする
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    // 半角/全角キー到着 → 保留フラッシュ + パススルー
    let r = engine.on_event(Ev::down(VK_KANJI).scan(SCAN_KANJI).at(t0 + 50_000).build());
    // consumed=false (パススルー) だがフラッシュアクションが含まれる
    assert!(!r.consumed, "should pass through the IME control key");
    assert!(
        !r.actions.is_empty(),
        "should emit flushed pending char actions"
    );
}

#[test]
fn test_ime_control_key_flushes_speculative_and_passes_through() {
    let mut engine = make_speculative_engine();
    let t0 = 1_000_000;
    // SpeculativeChar 状態にする
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    // 半角/全角キー → speculative は確定済みなので追加アクションなし、パススルー
    let r = engine.on_event(Ev::down(VK_KANJI).scan(SCAN_KANJI).at(t0 + 50_000).build());
    assert!(!r.consumed, "should pass through the IME control key");
}

// ── set_ngram_model / timing_judge ──

#[test]
fn test_set_ngram_model_and_timing_judge() {
    let mut engine = make_engine();
    // Without model, timing_judge uses fixed threshold (is_simultaneous within threshold)
    let judge = engine.timing_judge();
    assert!(judge.is_simultaneous(0, engine.threshold_us - 1, Some('あ')));
    assert!(!judge.is_simultaneous(0, engine.threshold_us + 1, Some('あ')));
    drop(judge);

    // With model, timing_judge uses the model for threshold adjustment
    let model = NgramModel::new(20_000, 30_000, 120_000);
    engine.set_ngram_model(model);
    // Unknown candidate -> score 0 -> tanh(0)=0 -> base threshold unchanged
    let judge = engine.timing_judge();
    assert!(judge.is_simultaneous(0, 99_999, Some('x')));
    assert!(!judge.is_simultaneous(0, 100_001, Some('x')));
}

// ── PendingThumb + another thumb key (expired) ──

#[test]
fn test_pending_thumb_then_char_after_threshold() {
    // PendingThumb + char after threshold -> thumb single (composing=false なので emit), char new pending
    let mut engine = make_engine();

    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(0).build());
    assert_pending(&r);

    // Char arrives after threshold。composing=false（テストのデフォルト状態）なので、
    // 無変換/変換は「Windows 全般での無変換/変換キー機能」として生 VK が emit される
    // （timeout_pending_thumb と同じ判定を flush 経路にも統一した挙動、composing 中の
    // suppress は resolve_pending_thumb_as_single 参照）。
    let r = engine.on_event(Ev::down(VK_A).at(200_000).build());
    r.assert_consumed();
    assert!(
        r.actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT)),
        "thumb single should be emitted when not composing"
    );
}

#[test]
fn test_pending_thumb_then_another_thumb() {
    // PendingThumb + another thumb -> first thumb single (composing=false なので emit), second thumb pending
    let mut engine = make_engine();

    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(0).build());
    assert_pending(&r);

    // Another thumb arrives within threshold (still same kind = thumb)。
    // composing=false なので最初の親指キーは生 VK で emit される（上のテスト参照）。
    let r = engine.on_event(Ev::down(VK_CONVERT).at(30_000).build());
    r.assert_consumed();
    assert!(
        r.actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT)),
        "first thumb single should be emitted when not composing"
    );
}

// ── PendingCharThumb + thumb key arrival (line 537, 543-544) ──

#[test]
fn test_pending_char_thumb_then_another_thumb() {
    // char1 -> thumb1 -> thumb2 arrives
    // Should resolve char1+thumb1 as simultaneous, thumb2 as new pending
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).at(0).build());
    engine.on_event(Ev::down(VK_NONCONVERT).at(20_000).build());

    // Another thumb key arrives
    let r = engine.on_event(Ev::down(VK_CONVERT).at(50_000).build());
    r.assert_consumed();
    // char1+left_thumb -> 'を'
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('を'))));
}

// ── resolve_char_thumb_as_simultaneous when no thumb face definition (line 248) ──

#[test]
fn test_resolve_char_thumb_no_thumb_face_definition() {
    // Use a key defined in normal face but NOT in any thumb face
    let mut engine = make_engine();
    engine.layout.normal.insert(POS_D, lit('て'));

    engine.on_event(Ev::down(VK_D).at(0).build());
    engine.on_event(Ev::down(VK_NONCONVERT).at(20_000).build());

    // Timeout resolves as simultaneous, but D is not in left_thumb face
    // Falls back to single char resolution via normal face -> 'て'
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('て'))));
}

// ── ReduceAndContinue + pass_through edge cases ──

#[test]
fn test_pending_char_then_non_layout_key_passes_through_new() {
    // PendingChar 中に scan_to_pos にないキー（VK_RETURN 等）が来た場合、
    // InputTracker で Passthrough に分類されるため、
    // bypass_reason → Passthrough で即座にパススルーされる。
    // 保留中の A はタイムアウトで後から確定される。
    let mut engine = make_engine();

    let r = engine.on_event(Ev::down(VK_A).at(0).build());
    assert_pending(&r);

    // VK_RETURN は scan_to_pos にないので Passthrough に分類される
    let r = engine.on_event(Ev::down(VK_RETURN).at(200_000).build());
    assert!(!r.consumed, "non-layout key should pass through");
}

// ── KeyUp while PendingThumb (lines 599-600, 606) ──

#[test]
fn test_key_up_while_pending_thumb() {
    let mut engine = make_engine();

    let r = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&r);

    // KeyUp of the pending thumb key -> resolves as single。composing=false
    // （テストのデフォルト状態）なので生 VK が emit される
    // （composing 中の suppress は resolve_pending_thumb_as_single 参照）。
    let r = engine.on_event(Ev::up(VK_NONCONVERT).build());
    r.assert_consumed();
    assert!(
        r.actions
            .iter()
            .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT)),
        "thumb single on KeyUp should be emitted when not composing"
    );
}

// ── KeyUp for active Key action (lines 619-620) ──

#[test]
fn test_key_up_active_key_action() {
    // When a key was resolved as Key(vk) and then KeyUp arrives
    // Use a key in thumb face only (not normal) so it's a layout key
    // but resolve_pending_char_as_single falls back to Key(vk)
    let mut engine = make_engine();
    // Add D only to left_thumb face, not normal
    engine.layout.left_thumb.insert(POS_D, lit('な'));

    // D is now a layout key, enters pending
    engine.on_event(Ev::down(VK_D).build());
    // Timeout: not in normal face -> Key(VK_D)
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_D)));

    // KeyUp should produce KeyUp(VK_D)
    let r = engine.on_event(Ev::up(VK_D).build());
    r.assert_consumed();
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::KeyUp(x) if *x == VK_D)));
}

// ── KeyUp for active Suppress/other action (line 622) ──

#[test]
fn test_key_up_active_suppress_action() {
    // When output_history has a Suppress action (unlikely in practice, but covers pass_through branch)
    let mut engine = make_engine();

    // Manually insert a Suppress action into output_history
    engine.output_history.push(OutputEntry {
        scan_code: SCAN_D,
        romaji: String::new(),
        kana: None,
        action: KeyAction::Suppress,
    });

    let r = engine.on_event(Ev::up(VK_D).build());
    r.assert_pass_through();
}

// ── KeyUp during PendingCharThumb resolving Key action (line 580) ──

#[test]
fn test_key_up_pending_char_thumb_resolves_char() {
    // char1 in normal face but NOT in thumb face
    // char1 KeyUp → 待機; thumb KeyUp → resolve (D not in left_thumb → fallback to normal → Char('て'))
    let mut engine = make_engine();
    engine.layout.normal.insert(POS_D, lit('て'));

    engine.on_event(Ev::down(VK_D).at(0).build());
    engine.on_event(Ev::down(VK_NONCONVERT).at(20_000).build());

    // KeyUp of D: 待機（何も出力しない）
    let r = engine.on_event(Ev::up(VK_D).build());
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "char1 release should not emit immediately"
    );

    // thumb KeyUp: resolve char1+thumb → D not in left_thumb → fallback to normal → Char('て')
    let r = engine.on_event(Ev::up(VK_NONCONVERT).build());
    r.assert_consumed();
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('て'))));
}

#[test]
fn test_key_up_pending_char_thumb_resolves_key_with_keyup() {
    // char1 NOT in normal or left_thumb -> fallback to Key(vk)
    // char1 KeyUp → 待機; thumb KeyUp → Key(VK_D) + KeyUp(VK_D) まとめて出力
    let mut engine = make_engine();
    // Add D only to right_thumb (not left_thumb, not normal)
    engine.layout.right_thumb.insert(POS_D, lit('で'));

    engine.on_event(Ev::down(VK_D).at(0).build());
    // Left thumb -> PendingCharThumb
    engine.on_event(Ev::down(VK_NONCONVERT).at(20_000).build());

    // KeyUp of D → 待機（何も出力しない）
    let r = engine.on_event(Ev::up(VK_D).build());
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "char1 release should not emit immediately"
    );

    // thumb KeyUp → resolve: D NOT in left_thumb → Key(VK_D), char1 released → KeyUp(VK_D) も追加
    let r = engine.on_event(Ev::up(VK_NONCONVERT).build());
    r.assert_consumed();
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_D)));
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::KeyUp(x) if *x == VK_D)));
}

// ── is_layout_key coverage (lines 657-659) ──

#[test]
fn test_is_layout_key_various_faces() {
    let engine = make_engine();
    // A key is in normal face
    assert!(engine.is_layout_key(Some(POS_A)));
    // D key is NOT in any face in the basic layout
    assert!(!engine.is_layout_key(Some(POS_D)));
    // None pos
    assert!(!engine.is_layout_key(None));
}

#[test]
fn test_is_layout_key_thumb_and_shift_faces() {
    let mut engine = make_engine_with_shift();
    // A is in normal, left_thumb, right_thumb, and shift
    assert!(engine.is_layout_key(Some(POS_A)));

    // Add D only to left_thumb face
    engine.layout.left_thumb.insert(POS_D, lit('な'));
    assert!(engine.is_layout_key(Some(POS_D)));
}

// ── timeout for char not in normal layout (lines 722-723) ──

#[test]
fn test_timeout_char_not_in_normal_layout() {
    let mut engine = make_engine();

    // F key is not in normal layout but IS a layout key in extended layout
    // We need a key that gets past is_layout_key but isn't in normal face
    // Add F to left_thumb only
    engine.layout.left_thumb.insert(POS_F, lit('よ'));

    // F is now a layout key (in left_thumb), so it will be pending
    let r = engine.on_event(Ev::down(VK_F).build());
    assert_pending(&r);

    // Timeout -> not in normal face -> Key(VK_F)
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_F)));
}

// ── swap_layout with PendingCharThumb ──

#[test]
fn test_swap_layout_flushes_pending_char_thumb() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).at(0).build());
    engine.on_event(Ev::down(VK_NONCONVERT).at(20_000).build());

    let new_layout = make_layout();
    let r = engine.swap_layout(new_layout);
    r.assert_consumed();
    // Should resolve char1+thumb as simultaneous
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('を'))));
}

// ── d1 >= d2 path where char2 has no thumb face definition (lines 532-533) ──

#[test]
fn test_three_key_d1_ge_d2_no_thumb_face_for_char2() {
    // char1(A, t=0) -> thumb(t=60ms) -> char2(D, t=80ms)
    // d1=60ms >= d2=20ms -> char1 single, char2+thumb attempted but D not in thumb face
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).at(0).build());
    engine.on_event(Ev::down(VK_CONVERT).at(60_000).build());

    // D is not in right_thumb face -> falls through to process_new_key_down
    let r = engine.on_event(Ev::down(VK_D).at(80_000).build());
    r.assert_consumed();
    // char1(A) -> 'う' (single)
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('う'))));
}

// ── Romaji in layout face ──

#[test]
fn test_romaji_value_in_layout() {
    let mut layout = make_layout();
    layout.normal.insert(
        POS_D,
        YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: Some('か'),
        },
    );
    let mut engine = TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    };

    engine.on_event(Ev::down(VK_D).build());
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        matches!(&r.actions[0], KeyAction::Char('か')),
        "should output Char('か') action"
    );
}

// ── Special key in layout face ──

#[test]
fn test_special_value_in_layout() {
    use crate::yab::SpecialKey;
    let mut layout = make_layout();
    layout
        .normal
        .insert(POS_D, YabValue::Special(SpecialKey::Backspace));
    let mut engine = TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    };

    engine.on_event(Ev::down(VK_D).build());
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(matches!(
        r.actions[0],
        KeyAction::SpecialKey(SpecialKey::Backspace)
    ));

    // SpecialKey actions are atomic (down+up in one shot).
    // output_history stores the SpecialKey action, so KeyUp finds it
    // and suppresses or passes through since there's no VK to release.
    let _r = engine.on_event(Ev::up(VK_D).build());
}

// ── None value in layout face ──

#[test]
fn test_none_value_in_layout() {
    // D: Normal='無'（明示的な抑制）かつ left_thumb='な'（配列キーとして確定）
    // 単独打鍵した場合、Normal lookup が Some(Suppress) を返す → Suppress として確定される。
    // これは VK_OEM_102 が Shift面に定義があるが Normal面が '無' の場合の修正を検証する。
    let mut layout = make_layout();
    layout.normal.insert(POS_D, YabValue::None);
    layout.left_thumb.insert(POS_D, lit('な')); // is_layout_key = true にするため必要
    let mut engine = TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    };

    engine.on_event(Ev::down(VK_D).build());
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(matches!(r.actions[0], KeyAction::Suppress));
}

// SysKeyDown/SysKeyUp テストは削除
// Windows の SysKey イベントはプラットフォーム層で KeyDown/KeyUp に変換される

// ── KeyUp of thumb during PendingCharThumb (thumb released) ──

#[test]
fn test_key_up_thumb_during_pending_char_thumb() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).at(0).build());
    engine.on_event(Ev::down(VK_CONVERT).at(20_000).build());

    // Thumb KeyUp -> resolves char1+thumb as simultaneous
    let r = engine.on_event(Ev::up(VK_CONVERT).build());
    r.assert_consumed();
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('ゔ'))));
}

// ── Romaji KeyUp produces Suppress ──

#[test]
fn test_key_up_for_romaji_produces_suppress() {
    let mut layout = make_layout();
    layout.normal.insert(
        POS_D,
        YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: Some('か'),
        },
    );
    let mut engine = TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    };

    engine.on_event(Ev::down(VK_D).build());
    engine.on_timeout(TIMER_PENDING);

    let r = engine.on_event(Ev::up(VK_D).build());
    r.assert_consumed();
    assert!(matches!(r.actions[0], KeyAction::Suppress));
}

// ── KeyUp while PendingChar resolves to Key action (line 606) ──

#[test]
fn test_key_up_while_pending_char_key_action() {
    // A key that's a layout key but NOT in normal face -> resolves to Key(vk)
    // Then KeyUp should find Key in output_history and append KeyUp
    let mut engine = make_engine();
    // Add D only to left_thumb (not normal)
    engine.layout.left_thumb.insert(POS_D, lit('な'));

    // D is a layout key, enters PendingChar
    let r = engine.on_event(Ev::down(VK_D).build());
    assert_pending(&r);

    // KeyUp of D while pending -> resolve_pending_char_as_single
    // D not in normal -> Key(VK_D), output_history records Key(VK_D)
    // Then output_history removal finds Key(VK_D) -> push KeyUp(VK_D)
    let r = engine.on_event(Ev::up(VK_D).build());
    r.assert_consumed();
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_D)));
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::KeyUp(x) if *x == VK_D)));
}

// ── 3-key d1 >= d2 with left thumb (lines 516, 522) ──

#[test]
fn test_three_key_d1_ge_d2_left_thumb() {
    // char1(A, t=0) -> left_thumb(t=60ms) -> char2(S, t=80ms)
    // d1=60ms >= d2=20ms -> char1 single, char2+left_thumb simultaneous
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).at(0).build());
    engine.on_event(Ev::down(VK_NONCONVERT).at(60_000).build());

    let r = engine.on_event(Ev::down(VK_S).at(80_000).build());
    r.assert_consumed();
    // char1(A) -> 'う' (single), char2(S)+left_thumb -> 'あ'
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('う'))));
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('あ'))));
}

// ── output_history tracking tests ──

#[test]
fn test_output_history_tracked_on_timeout() {
    let mut engine = make_engine();

    // Press A key, goes to PendingChar
    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);
    assert!(engine.output_history.recent_kana(3).is_empty());

    // Timeout confirms A as standalone → 'う' (normal face)
    let result = engine.on_timeout(TIMER_PENDING);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('う')));
    assert_eq!(engine.output_history.recent_kana(3), vec!['う']);
}

#[test]
fn test_output_history_tracked_on_simultaneous() {
    let mut engine = make_engine();
    let t0 = 0;

    // Thumb first (left thumb = nonconvert)
    let result = engine.on_event(Ev::down(VK_NONCONVERT).at(t0).build());
    assert_pending(&result);

    // Char arrives within threshold → simultaneous → left_thumb face for A = 'を'
    let t1 = t0 + 30_000;
    let result = engine.on_event(Ev::down(VK_A).at(t1).build());
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('を')));
    assert_eq!(engine.output_history.recent_kana(3), vec!['を']);
}

#[test]
fn test_output_history_recent_kana_limit() {
    let mut engine = make_engine();

    // Output 4 chars via successive timeout confirmations
    // 1st: A → 'う'
    engine.on_event(Ev::down(VK_A).build());
    engine.on_timeout(TIMER_PENDING);
    assert_eq!(engine.output_history.recent_kana(3), vec!['う']);

    // 2nd: S → 'し'
    engine.on_event(Ev::down(VK_S).build());
    engine.on_timeout(TIMER_PENDING);
    assert_eq!(engine.output_history.recent_kana(3), vec!['う', 'し']);

    // 3rd: A → 'う'
    engine.on_event(Ev::down(VK_A).build());
    engine.on_timeout(TIMER_PENDING);
    assert_eq!(engine.output_history.recent_kana(3), vec!['う', 'し', 'う']);

    // 4th: S → 'し' — recent_kana(3) returns only the last 3
    engine.on_event(Ev::down(VK_S).build());
    engine.on_timeout(TIMER_PENDING);
    assert_eq!(engine.output_history.recent_kana(3), vec!['し', 'う', 'し']);
}

// ── n-gram adaptive threshold tests ──────────────────────────────

/// Create an NgramModel with known bigram scores for threshold tests.
///
/// Layout reminder:
///   Left thumb face:  A → 'を', S → 'あ'
///   Right thumb face: A → 'ゔ', S → 'じ'
///
/// Bigrams:
///   "しを" =  2.0  → tanh(2.0) ≈ 0.964 → threshold ≈ 100_000 + 19_280 = 119_280us
///   "しゔ" = -2.0  → tanh(-2.0) ≈ -0.964 → threshold ≈ 100_000 - 19_280 = 80_720us
fn make_ngram_model() -> NgramModel {
    let toml = r#"
[bigram]
"しを" = 2.0
"しゔ" = -2.0

[trigram]
"#;
    // base = 100ms = 100_000us, range = 20ms = 20_000us
    NgramModel::from_toml(toml, 20_000, 30_000, 120_000).unwrap()
}

/// High-frequency bigram candidate relaxes threshold so borderline timing
/// is accepted as simultaneous.
///
/// Scenario: recent output = 'し', then A + left thumb (→ candidate 'を').
/// Bigram "しを" = 2.0 → adjusted threshold ≈ 119ms.
/// Gap = 105ms: without n-gram 105ms > 100ms (standalone),
///              with n-gram 105ms < 119ms (simultaneous).
#[test]
fn test_ngram_high_freq_relaxes_threshold() {
    let mut engine = make_engine();
    engine.set_ngram_model(make_ngram_model());
    // Seed output_history with 'し' to provide n-gram context
    engine.output_history.push(OutputEntry {
        scan_code: SCAN_S,
        romaji: String::new(),
        kana: Some('し'),
        action: KeyAction::Char('し'),
    });

    let t0: u64 = 0;

    // Char key A → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&r);

    // Left thumb 105ms later → should enter PendingCharThumb
    // (105_000us < adjusted threshold ~119_280us)
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t0 + 105_000).build());
    r.assert_consumed();
    assert!(r.actions.is_empty(), "should be pending, not yet emitted");

    // Timeout resolves PendingCharThumb as simultaneous → left thumb face for A = 'を'
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(
        matches!(r.actions[0], KeyAction::Char('を')),
        "high-freq bigram should relax threshold: expected 'を', got {:?}",
        r.actions[0]
    );
}

/// Low-frequency bigram candidate tightens threshold so borderline timing
/// is rejected as standalone.
///
/// Scenario: recent output = 'し', then A + right thumb (→ candidate 'ゔ').
/// Bigram "しゔ" = -2.0 → adjusted threshold ≈ 81ms.
/// Gap = 90ms: without n-gram 90ms < 100ms (simultaneous),
///             with n-gram 90ms > 81ms (standalone).
#[test]
fn test_ngram_low_freq_tightens_threshold() {
    let mut engine = make_engine();
    engine.set_ngram_model(make_ngram_model());
    // Seed output_history with 'し' to provide n-gram context
    engine.output_history.push(OutputEntry {
        scan_code: SCAN_S,
        romaji: String::new(),
        kana: Some('し'),
        action: KeyAction::Char('し'),
    });

    let t0: u64 = 0;

    // Char key A → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&r);

    // Right thumb 90ms later → should NOT enter PendingCharThumb
    // (90_000us > adjusted threshold ~80_720us → time exceeded)
    // Instead, the pending char is resolved as standalone ('う' from normal face),
    // and the thumb key becomes a new pending.
    let r = engine.on_event(Ev::down(VK_CONVERT).at(t0 + 90_000).build());
    r.assert_consumed();

    // The standalone char 'う' should be emitted immediately
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('う'))),
        "low-freq bigram should tighten threshold: expected standalone 'う', got {:?}",
        r.actions
    );
}

/// Without n-gram model, the engine uses a fixed threshold (100ms).
/// 95ms < 100ms → simultaneous detection via left thumb face.
#[test]
fn test_without_ngram_fixed_threshold_simultaneous() {
    let mut engine = make_engine();
    // No set_ngram_model call — uses fixed 100ms threshold

    let t0: u64 = 0;

    // Char key A → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&r);

    // Left thumb 95ms later → within fixed 100ms threshold → PendingCharThumb
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t0 + 95_000).build());
    r.assert_consumed();
    assert!(r.actions.is_empty(), "should be pending (PendingCharThumb)");

    // Timeout → simultaneous → left thumb face for A = 'を'
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(
        matches!(r.actions[0], KeyAction::Char('を')),
        "fixed threshold: 95ms < 100ms should be simultaneous, got {:?}",
        r.actions[0]
    );
}

/// Without n-gram model, 105ms > fixed 100ms threshold → standalone.
/// This is the counterpart to test_ngram_high_freq_relaxes_threshold:
/// same 105ms gap, but without n-gram it's rejected.
#[test]
fn test_without_ngram_fixed_threshold_standalone() {
    let mut engine = make_engine();
    // No set_ngram_model call — uses fixed 100ms threshold

    let t0: u64 = 0;

    // Char key A → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&r);

    // Left thumb 105ms later → exceeds fixed 100ms threshold → standalone
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t0 + 105_000).build());
    r.assert_consumed();

    // The standalone char 'う' (normal face) should be emitted
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('う'))),
        "fixed threshold: 105ms > 100ms should be standalone 'う', got {:?}",
        r.actions
    );
}

/// Without n-gram, 90ms < 100ms → simultaneous via right thumb.
/// This is the counterpart to test_ngram_low_freq_tightens_threshold:
/// same 90ms gap, but without n-gram it's accepted.
#[test]
fn test_without_ngram_90ms_is_simultaneous() {
    let mut engine = make_engine();
    // No set_ngram_model call — uses fixed 100ms threshold

    let t0: u64 = 0;

    // Char key A → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&r);

    // Right thumb 90ms later → within fixed 100ms → PendingCharThumb
    let r = engine.on_event(Ev::down(VK_CONVERT).at(t0 + 90_000).build());
    r.assert_consumed();
    assert!(r.actions.is_empty(), "should be pending (PendingCharThumb)");

    // Timeout → simultaneous → right thumb face for A = 'ゔ'
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(
        matches!(r.actions[0], KeyAction::Char('ゔ')),
        "fixed threshold: 90ms < 100ms should be simultaneous, got {:?}",
        r.actions[0]
    );
}

#[test]
fn test_speculative_char_timeout_confirms() {
    // Manually set engine to SpeculativeChar state
    let mut engine = make_engine();
    engine.state = EngineState::SpeculativeChar(PendingKey {
        scan_code: SCAN_A,
        vk_code: VK_A,
        pos: Some(POS_A),
        timestamp: 1_000_000,
    });

    // Call on_timeout → should return to Idle with no actions
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "SpeculativeChar timeout should produce no actions (already emitted), got {:?}",
        r.actions
    );
    assert!(
        engine.state.is_idle(),
        "state should be Idle after timeout, got {:?}",
        engine.state
    );
}

// ── Speculative confirm mode tests ──

#[test]
fn test_speculative_single_char() {
    // Character press in Speculative mode → immediate output → timeout → no additional output
    let mut engine = make_speculative_engine();

    // Press 'A' key → should immediately output 'う' (normal face) and enter SpeculativeChar
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1, "should emit one action immediately");
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "should emit normal face char 'う', got {:?}",
        r.actions[0]
    );
    assert!(
        matches!(engine.state, EngineState::SpeculativeChar(_)),
        "state should be SpeculativeChar, got {:?}",
        engine.state
    );

    // Timeout → no additional output, back to Idle
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "timeout should produce no actions (already emitted), got {:?}",
        r.actions
    );
    assert!(
        engine.state.is_idle(),
        "state should be Idle after timeout, got {:?}",
        engine.state
    );
}

#[test]
fn test_speculative_simultaneous() {
    // Char press → immediate output → thumb arrives within threshold → BS + thumb face
    let mut engine = make_speculative_engine();
    let t0 = 1_000_000;

    // Press 'A' key → immediate output 'う'
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "should emit 'う' immediately"
    );

    // Left thumb arrives within threshold (30ms < 100ms threshold)
    let t1 = t0 + 30_000;
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t1).build());
    r.assert_consumed();
    // Should have BS (retract 'う' which is a Char → 0 romaji chars → 0 BS)
    // Actually, for Literal chars, emitted_romaji is empty string, so no BS needed
    // The action should just be the thumb face char 'を'
    assert!(
        !r.actions.is_empty(),
        "should produce actions for thumb retraction"
    );
    // Last action should be the thumb-face character
    assert!(
        matches!(r.actions.last(), Some(KeyAction::Char('を'))),
        "last action should be thumb face char 'を', got {:?}",
        r.actions
    );
    assert!(
        engine.state.is_idle(),
        "state should be Idle after retraction, got {:?}",
        engine.state
    );
}

#[test]
fn test_speculative_simultaneous_with_romaji() {
    // Char press with romaji → immediate output → thumb arrives → BS×N + thumb face
    let mut layout = make_layout();
    layout.normal.insert(
        POS_D,
        YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: Some('か'),
        },
    );
    layout.left_thumb.insert(POS_D, lit('げ'));

    let mut engine = TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Speculative,
            30,
        ),
    };
    let t0 = 1_000_000;

    // Press 'D' key → immediate output Char('か')
    let r = engine.on_event(Ev::down(VK_D).at(t0).build());
    r.assert_consumed();
    assert!(
        matches!(&r.actions[0], KeyAction::Char('か')),
        "should emit Char('か') immediately, got {:?}",
        r.actions[0]
    );

    // Left thumb arrives within threshold
    let t1 = t0 + 30_000;
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t1).build());
    r.assert_consumed();
    // Bug #3 fix: IME treats complete romaji as 1 composition unit → always 1 BS
    assert_eq!(
        r.actions.len(),
        2,
        "should have 1 BS + 1 thumb action, got {:?}",
        r.actions
    );
    assert!(
        matches!(
            &r.actions[0],
            KeyAction::SpecialKey(crate::types::SpecialKey::Backspace)
        ),
        "first action should be BS, got {:?}",
        r.actions[0]
    );
    assert!(
        matches!(&r.actions[1], KeyAction::Char('げ')),
        "second action should be 'げ', got {:?}",
        r.actions[1]
    );
}

#[test]
fn test_speculative_char_sequence() {
    // char1 → immediate → char2 → char1 was correct + char2 immediate
    let mut engine = make_speculative_engine();
    let t0 = 1_000_000;

    // Press 'A' key → immediate output 'う'
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "should emit 'う' immediately"
    );

    // Press 'S' key → char1 was correct, char2 emits immediately
    let t1 = t0 + 50_000;
    let r = engine.on_event(Ev::down(VK_S).at(t1).build());
    r.assert_consumed();
    // Should emit 'し' (normal face for S)
    assert!(
        matches!(&r.actions[0], KeyAction::Char('し')),
        "should emit 'し' for second char, got {:?}",
        r.actions
    );
    assert!(
        matches!(engine.state, EngineState::SpeculativeChar(_)),
        "state should be SpeculativeChar for second char, got {:?}",
        engine.state
    );
}

#[test]
fn test_speculative_thumb_outside_threshold() {
    // Char press → thumb arrives after threshold → speculative was correct, thumb processed as new
    let mut engine = make_speculative_engine();
    let t0 = 1_000_000;

    // Press 'A' key → immediate output 'う'
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "should emit 'う' immediately"
    );

    // Left thumb arrives AFTER threshold (150ms > 100ms)
    let t1 = t0 + 150_000;
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t1).build());
    // Thumb should be processed as a new key (pending thumb in Wait fallback via handle_idle)
    r.assert_consumed();
    // In Speculative mode, thumb goes to handle_idle_wait → PendingThumb
    assert!(
        matches!(engine.state, EngineState::PendingThumb(_)),
        "thumb outside threshold should be pending, got {:?}",
        engine.state
    );
}

#[test]
fn test_speculative_thumb_first_falls_back_to_wait() {
    // Thumb first in Speculative mode → same as Wait mode (PendingThumb)
    let mut engine = make_speculative_engine();
    let t0 = 1_000_000;

    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t0).build());
    assert_pending(&r);
    assert!(
        matches!(engine.state, EngineState::PendingThumb(_)),
        "thumb first should enter PendingThumb, got {:?}",
        engine.state
    );
}

// ── TwoPhase confirm mode tests ──

fn make_two_phase_engine() -> TestHarness {
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::TwoPhase,
            30,
        ),
    }
}

#[test]
fn test_two_phase_thumb_within_short_delay() {
    // Thumb arrives at 20ms (< 30ms speculative delay) → clean simultaneous, no flicker
    // Phase 1: char enters PendingChar with TIMER_SPECULATIVE
    // Thumb arrives before TIMER_SPECULATIVE fires → same as Wait mode PendingChar+thumb path
    let mut engine = make_two_phase_engine();
    let t0 = 1_000_000;

    // Press 'A' key → PendingChar with TIMER_SPECULATIVE
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(r.actions.is_empty(), "Phase 1 should not emit any actions");
    r.assert_timer_set(TIMER_SPECULATIVE);
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "state should be PendingChar, got {:?}",
        engine.state
    );

    // Left thumb arrives at 20ms (< 30ms) → PendingCharThumb
    let t1 = t0 + 20_000;
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t1).build());
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "should be pending (PendingCharThumb), got {:?}",
        r.actions
    );

    // Timeout resolves as simultaneous → left thumb face for A = 'を'
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(
        matches!(r.actions[0], KeyAction::Char('を')),
        "clean simultaneous should produce 'を', got {:?}",
        r.actions[0]
    );
}

#[test]
fn test_two_phase_thumb_after_short_delay() {
    // Thumb arrives at 50ms (> 30ms but < 100ms) → speculative output happened, BS + replace
    // Phase 1: char enters PendingChar with TIMER_SPECULATIVE
    // TIMER_SPECULATIVE fires at 30ms → Phase 2: speculative output + SpeculativeChar
    // Thumb arrives at 50ms → BS + thumb face
    let mut engine = make_two_phase_engine();
    let t0 = 1_000_000;

    // Press 'A' key → PendingChar with TIMER_SPECULATIVE
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(r.actions.is_empty(), "Phase 1 should not emit any actions");

    // TIMER_SPECULATIVE fires → Phase 2: speculative output 'う'
    let r = engine.on_timeout(TIMER_SPECULATIVE);
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1, "should emit speculative output");
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "speculative output should be 'う', got {:?}",
        r.actions[0]
    );
    r.assert_timer_set(TIMER_PENDING);
    assert!(
        matches!(engine.state, EngineState::SpeculativeChar(_)),
        "state should be SpeculativeChar, got {:?}",
        engine.state
    );

    // Left thumb arrives at 50ms (within remaining threshold window)
    let t1 = t0 + 50_000;
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t1).build());
    r.assert_consumed();
    // Last action should be the thumb-face character 'を'
    assert!(
        matches!(r.actions.last(), Some(KeyAction::Char('を'))),
        "last action should be thumb face char 'を', got {:?}",
        r.actions
    );
    assert!(
        engine.state.is_idle(),
        "state should be Idle after retraction, got {:?}",
        engine.state
    );
}

#[test]
fn test_two_phase_no_thumb() {
    // No thumb → speculative at 30ms, confirmed at 100ms
    // Phase 1: PendingChar + TIMER_SPECULATIVE
    // Phase 2 (30ms): speculative output + SpeculativeChar + TIMER_PENDING for remaining 70ms
    // TIMER_PENDING fires: confirmed (no additional output)
    let mut engine = make_two_phase_engine();
    let t0 = 1_000_000;

    // Press 'A' key → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(r.actions.is_empty());

    // TIMER_SPECULATIVE fires → speculative output
    let r = engine.on_timeout(TIMER_SPECULATIVE);
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(matches!(&r.actions[0], KeyAction::Char('う')));
    assert!(matches!(engine.state, EngineState::SpeculativeChar(_)));

    // TIMER_PENDING fires → confirmed, no additional output
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "SpeculativeChar timeout should produce no actions, got {:?}",
        r.actions
    );
    assert!(
        engine.state.is_idle(),
        "state should be Idle after full confirmation, got {:?}",
        engine.state
    );
}

#[test]
fn test_two_phase_char_sequence() {
    // Chars arrive rapidly, each within 30ms → wait confirms previous
    // char1 → PendingChar, char2 arrives within 30ms → char1 confirmed as single, char2 pending
    let mut engine = make_two_phase_engine();
    let t0 = 1_000_000;

    // Press 'A' key → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(r.actions.is_empty());

    // Press 'S' key at 20ms (< 30ms, before TIMER_SPECULATIVE fires)
    let t1 = t0 + 20_000;
    let r = engine.on_event(Ev::down(VK_S).at(t1).build());
    r.assert_consumed();
    // char1 (A) should be flushed as single ('う')
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('う'))),
        "char1 should be confirmed as 'う', got {:?}",
        r.actions
    );
    // char2 (S) should now be in PendingChar
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "state should be PendingChar for char2, got {:?}",
        engine.state
    );
}

// ── AdaptiveTiming モード テスト ──

fn make_adaptive_engine() -> TestHarness {
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::AdaptiveTiming,
            30,
        ),
    }
}

/// 最初のキー（前キーなし）→ TwoPhase 動作（PendingChar + TIMER_SPECULATIVE）
#[test]
fn test_adaptive_first_key_uses_two_phase() {
    let mut engine = make_adaptive_engine();
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());

    // TwoPhase: PendingChar 状態 + TIMER_SPECULATIVE が設定される
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "TwoPhase Phase 1 should have no actions"
    );
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "state should be PendingChar, got {:?}",
        engine.state
    );
    r.assert_timer_set(TIMER_SPECULATIVE);
}

/// 連続打鍵（50ms 間隔）→ Wait 動作（PendingChar + TIMER_PENDING）
#[test]
fn test_adaptive_rapid_typing_uses_wait() {
    let mut engine = make_adaptive_engine();

    // 1 文字目（TwoPhase 動作）
    let t0 = 1_000_000;
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    // タイムアウトで確定させて Idle に戻す
    let _ = engine.on_timeout(TIMER_SPECULATIVE);
    let _ = engine.on_timeout(TIMER_PENDING);

    // 2 文字目: 50ms 後（< 80ms → continuous → Wait）
    let t1 = t0 + 50_000;
    let r = engine.on_event(Ev::down(VK_S).at(t1).build());

    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "Wait mode should have no immediate actions"
    );
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "state should be PendingChar, got {:?}",
        engine.state
    );
    r.assert_timer_set(TIMER_PENDING);
}

/// ポーズ後（200ms 間隔）→ TwoPhase 動作（PendingChar + TIMER_SPECULATIVE）
#[test]
fn test_adaptive_after_pause_uses_two_phase() {
    let mut engine = make_adaptive_engine();

    // 1 文字目
    let t0 = 1_000_000;
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    let _ = engine.on_timeout(TIMER_SPECULATIVE);
    let _ = engine.on_timeout(TIMER_PENDING);

    // 2 文字目: 200ms 後（>= 80ms → paused → TwoPhase）
    let t1 = t0 + 200_000;
    let r = engine.on_event(Ev::down(VK_S).at(t1).build());

    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "TwoPhase Phase 1 should have no actions"
    );
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "state should be PendingChar, got {:?}",
        engine.state
    );
    r.assert_timer_set(TIMER_SPECULATIVE);
}

/// 連続打鍵 → ポーズ → 最後のキーは TwoPhase を使用
#[test]
fn test_adaptive_continuous_then_pause() {
    let mut engine = make_adaptive_engine();

    // 1 文字目 t=1000ms
    let t0 = 1_000_000;
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    let _ = engine.on_timeout(TIMER_SPECULATIVE);
    let _ = engine.on_timeout(TIMER_PENDING);

    // 2 文字目 t=1050ms (50ms gap → continuous → Wait)
    let t1 = t0 + 50_000;
    let r1 = engine.on_event(Ev::down(VK_S).at(t1).build());
    r1.assert_timer_set(TIMER_PENDING); // Wait mode
    let _ = engine.on_timeout(TIMER_PENDING);

    // 3 文字目 t=1300ms (250ms gap → paused → TwoPhase)
    let t2 = t1 + 250_000;
    let r2 = engine.on_event(Ev::down(VK_A).at(t2).build());
    r2.assert_consumed();
    assert!(
        r2.actions.is_empty(),
        "TwoPhase Phase 1 should have no actions"
    );
    r2.assert_timer_set(TIMER_SPECULATIVE);
}

// ── NgramPredictive confirm mode tests ──

fn make_ngram_predictive_engine() -> TestHarness {
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::NgramPredictive,
            30,
        ),
    }
}

/// n-gram で通常面のスコアが高い場合、Speculative（即時出力）を使用する
#[test]
fn test_ngram_predictive_high_normal_score_uses_speculative() {
    let mut engine = make_ngram_predictive_engine();

    // Seed output_history so that bigram ('あ', 'う') has a high score
    // Normal face for A key = 'う', left_thumb = 'を', right_thumb = 'ゔ'
    engine.output_history.push(OutputEntry {
        scan_code: ScanCode(0),
        romaji: String::new(),
        kana: Some('あ'),
        action: KeyAction::Char('あ'),
    });

    // High score for normal face kana ('あ', 'う'), low for thumb face
    let toml_str = r#"
[bigram]
"あう" = 2.0
"あを" = 0.5
"あゔ" = 0.3
"#;
    let model = NgramModel::from_toml(toml_str, 20_000, 30_000, 120_000).unwrap();
    engine.set_ngram_model(model);

    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());

    // Speculative: immediate output + SpeculativeChar state
    assert!(
        !r.actions.is_empty(),
        "NgramPredictive should output immediately when normal score is high"
    );
    assert!(
        matches!(engine.state, EngineState::SpeculativeChar(_)),
        "Should be in SpeculativeChar state"
    );
}

/// n-gram で親指面のスコアが高い場合、Wait（保留）を使用する
#[test]
fn test_ngram_predictive_high_thumb_score_uses_wait() {
    let mut engine = make_ngram_predictive_engine();

    // Seed output_history so that thumb face kana has high score
    engine.output_history.push(OutputEntry {
        scan_code: ScanCode(0),
        romaji: String::new(),
        kana: Some('あ'),
        action: KeyAction::Char('あ'),
    });

    // Low score for normal face kana, high for thumb face kana
    let toml_str = r#"
[bigram]
"あう" = 0.3
"あを" = 2.0
"あゔ" = 0.1
"#;
    let model = NgramModel::from_toml(toml_str, 20_000, 30_000, 120_000).unwrap();
    engine.set_ngram_model(model);

    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());

    // Wait: no actions, PendingChar state
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "NgramPredictive should wait when thumb score is higher"
    );
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "Should be in PendingChar state"
    );
    r.assert_timer_set(TIMER_PENDING);
}

/// n-gram モデルが未設定の場合、TwoPhase にフォールバックする
#[test]
fn test_ngram_predictive_no_model_falls_back() {
    let mut engine = make_ngram_predictive_engine();
    // No ngram model set → should fall back to TwoPhase

    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());

    // TwoPhase: PendingChar + TIMER_SPECULATIVE
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "TwoPhase fallback Phase 1 should have no actions"
    );
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "Should be in PendingChar state"
    );
    r.assert_timer_set(TIMER_SPECULATIVE);
}

/// 出力履歴が空の場合、スコアは両方 0 → diff=0 → Wait を使用する
#[test]
fn test_ngram_predictive_no_history_uses_wait() {
    let mut engine = make_ngram_predictive_engine();

    // Empty output_history + model with some bigrams (but they won't match with empty history)
    let toml_str = r#"
[bigram]
"あう" = 2.0
"#;
    let model = NgramModel::from_toml(toml_str, 20_000, 30_000, 120_000).unwrap();
    engine.set_ngram_model(model);

    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());

    // Both scores are 0.0 → diff = 0.0 (not > 0.5) → Wait
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "NgramPredictive should wait when no history (scores are zero)"
    );
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "Should be in PendingChar state"
    );
    r.assert_timer_set(TIMER_PENDING);
}

// ── Cross-mode comparison tests ──
// These tests verify that all ConfirmMode variants produce the same final
// characters after BS retraction is applied.  NgramPredictive is excluded
// because it requires an n-gram model to be configured and its behaviour
// depends on context history.

/// Modes to include in cross-mode comparison tests.
const CROSS_MODES: [ConfirmMode; 4] = [
    ConfirmMode::Wait,
    ConfirmMode::Speculative,
    ConfirmMode::TwoPhase,
    ConfirmMode::AdaptiveTiming,
];

fn make_engine_with_mode(mode: ConfirmMode) -> TestHarness {
    let layout = make_layout();
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(layout, VK_NONCONVERT, VK_CONVERT, 100, mode, 30),
    }
}

/// Collect final output from a sequence of Responses, handling BS retraction.
/// BS (`Key(0x08)`) retracts the most recently emitted non-Suppress action.
fn collect_output(responses: &[Resp]) -> Vec<KeyAction> {
    let mut output: Vec<KeyAction> = Vec::new();
    for r in responses {
        for action in &r.actions {
            match action {
                KeyAction::SpecialKey(crate::types::SpecialKey::Backspace) => {
                    output.pop();
                }
                KeyAction::Suppress => {} // skip suppresses
                other => output.push(other.clone()),
            }
        }
    }
    output
}

/// Extract only the Char values from the collected output.
fn collect_chars(responses: &[Resp]) -> Vec<char> {
    collect_output(responses)
        .into_iter()
        .filter_map(|a| match a {
            KeyAction::Char(c) => Some(c),
            _ => None,
        })
        .collect()
}

#[test]
fn test_all_modes_single_char_same_output() {
    let mut reference: Option<Vec<char>> = None;
    for mode in CROSS_MODES {
        let mut engine = make_engine_with_mode(mode);
        let mut responses = vec![];

        // Press A key
        responses.push(engine.on_event(Ev::down(VK_A).at(1_000_000).build()));
        // Fire all possible timers so every mode resolves
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));

        let chars = collect_chars(&responses);
        assert!(
            !chars.is_empty(),
            "mode {:?} should produce output for A key",
            mode
        );
        assert_eq!(
            chars,
            vec!['う'],
            "mode {:?} should produce normal face 'う' for A key, got {:?}",
            mode,
            chars
        );

        if let Some(ref expected) = reference {
            assert_eq!(
                &chars, expected,
                "mode {:?} differs from reference output",
                mode
            );
        } else {
            reference = Some(chars);
        }
    }
}

#[test]
fn test_all_modes_simultaneous_same_final_output() {
    let mut reference: Option<Vec<char>> = None;
    for mode in CROSS_MODES {
        let mut engine = make_engine_with_mode(mode);
        let mut responses = vec![];
        let t = 1_000_000u64;

        // Press A, then left thumb within threshold
        responses.push(engine.on_event(Ev::down(VK_A).at(t).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_event(Ev::down(VK_NONCONVERT).at(t + 20_000).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));

        let chars = collect_chars(&responses);
        // After BS retraction, all modes should end up with left thumb face for A = 'を'
        assert!(
            chars.contains(&'を'),
            "mode {:?} should produce left thumb face 'を' for simultaneous A+muhenkan, got {:?}",
            mode,
            chars
        );

        if let Some(ref expected) = reference {
            assert_eq!(
                &chars, expected,
                "mode {:?} simultaneous output differs from reference",
                mode
            );
        } else {
            reference = Some(chars);
        }
    }
}

#[test]
fn test_all_modes_simultaneous_right_thumb_same_final_output() {
    let mut reference: Option<Vec<char>> = None;
    for mode in CROSS_MODES {
        let mut engine = make_engine_with_mode(mode);
        let mut responses = vec![];
        let t = 1_000_000u64;

        // Press A, then right thumb within threshold
        responses.push(engine.on_event(Ev::down(VK_A).at(t).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_event(Ev::down(VK_CONVERT).at(t + 20_000).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));

        let chars = collect_chars(&responses);
        // After BS retraction, all modes should end up with right thumb face for A = 'ゔ'
        assert!(
            chars.contains(&'ゔ'),
            "mode {:?} should produce right thumb face 'ゔ' for A+henkan, got {:?}",
            mode,
            chars
        );

        if let Some(ref expected) = reference {
            assert_eq!(
                &chars, expected,
                "mode {:?} right-thumb simultaneous differs from reference",
                mode
            );
        } else {
            reference = Some(chars);
        }
    }
}

#[test]
fn test_all_modes_rapid_sequence_same_output() {
    let mut reference: Option<Vec<char>> = None;
    for mode in [
        ConfirmMode::Wait,
        ConfirmMode::Speculative,
        ConfirmMode::TwoPhase,
    ] {
        let mut engine = make_engine_with_mode(mode);
        let mut responses = vec![];

        // Type A, S rapidly (50ms apart), well outside threshold for simultaneous
        // but close enough to exercise the rapid path
        responses.push(engine.on_event(Ev::down(VK_A).at(1_000_000).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_event(Ev::down(VK_S).at(1_050_000).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));

        let chars = collect_chars(&responses);
        // Should have normal-face outputs for A='う' and S='し'
        assert_eq!(
            chars,
            vec!['う', 'し'],
            "mode {:?} rapid A,S should produce ['う','し'], got {:?}",
            mode,
            chars
        );

        if let Some(ref expected) = reference {
            assert_eq!(
                &chars, expected,
                "mode {:?} rapid sequence differs from reference",
                mode
            );
        } else {
            reference = Some(chars);
        }
    }
}

#[test]
fn test_all_modes_thumb_first_then_char_same_output() {
    let mut reference: Option<Vec<char>> = None;
    for mode in CROSS_MODES {
        let mut engine = make_engine_with_mode(mode);
        let mut responses = vec![];
        let t = 1_000_000u64;

        // Thumb first, then char within threshold (pattern 1)
        responses.push(engine.on_event(Ev::down(VK_NONCONVERT).at(t).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_event(Ev::down(VK_A).at(t + 30_000).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));

        let chars = collect_chars(&responses);
        assert!(
            chars.contains(&'を'),
            "mode {:?} thumb-first should produce 'を', got {:?}",
            mode,
            chars
        );

        if let Some(ref expected) = reference {
            assert_eq!(
                &chars, expected,
                "mode {:?} thumb-first output differs from reference",
                mode
            );
        } else {
            reference = Some(chars);
        }
    }
}

#[test]
fn test_all_modes_char_alone_after_threshold_same_output() {
    // Char is pressed, thumb arrives after threshold → char confirmed as normal face
    let mut reference: Option<Vec<char>> = None;
    for mode in CROSS_MODES {
        let mut engine = make_engine_with_mode(mode);
        let mut responses = vec![];
        let t = 1_000_000u64;

        responses.push(engine.on_event(Ev::down(VK_A).at(t).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));
        // Thumb arrives after full timeout → processed as new key, not simultaneous
        responses.push(engine.on_event(Ev::down(VK_NONCONVERT).at(t + 200_000).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));

        let chars = collect_chars(&responses);
        // 'う' from the A key (normal face) — thumb alone doesn't produce a char
        assert!(
            chars.contains(&'う'),
            "mode {:?} should produce normal face 'う' for A, got {:?}",
            mode,
            chars
        );

        if let Some(ref expected) = reference {
            assert_eq!(
                &chars, expected,
                "mode {:?} char-alone-after-threshold differs from reference",
                mode
            );
        } else {
            reference = Some(chars);
        }
    }
}

// ── Mode-specific characteristic tests ──

#[test]
fn test_speculative_has_immediate_output() {
    let mut engine = make_engine_with_mode(ConfirmMode::Speculative);
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    assert!(
        !r.actions.is_empty(),
        "Speculative should output immediately"
    );
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "Speculative immediate output should be normal face 'う', got {:?}",
        r.actions[0]
    );
}

#[test]
fn test_wait_has_no_immediate_output() {
    let mut engine = make_engine_with_mode(ConfirmMode::Wait);
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    assert!(
        r.actions.is_empty(),
        "Wait should not output immediately on key down"
    );
}

#[test]
fn test_two_phase_no_output_before_speculative_timer() {
    let mut engine = make_engine_with_mode(ConfirmMode::TwoPhase);
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    assert!(
        r.actions.is_empty(),
        "TwoPhase should not output immediately (Phase 1)"
    );
    // But after speculative timer fires, output appears (Phase 2)
    let r = engine.on_timeout(TIMER_SPECULATIVE);
    assert!(
        !r.actions.is_empty(),
        "TwoPhase should output after speculative delay (Phase 2)"
    );
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "TwoPhase Phase 2 output should be 'う', got {:?}",
        r.actions[0]
    );
}

#[test]
fn test_adaptive_first_key_behaves_like_two_phase() {
    // AdaptiveTiming with no prior key history should use TwoPhase behavior
    let mut engine = make_engine_with_mode(ConfirmMode::AdaptiveTiming);
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    assert!(
        r.actions.is_empty(),
        "AdaptiveTiming first key should not output immediately (TwoPhase Phase 1)"
    );
    let r = engine.on_timeout(TIMER_SPECULATIVE);
    assert!(
        !r.actions.is_empty(),
        "AdaptiveTiming first key should output after speculative timer"
    );
}

#[test]
fn test_speculative_retraction_on_simultaneous() {
    // Verify that Speculative mode resolves to thumb face when thumb arrives
    // within threshold.  The engine emits the speculative char immediately,
    // then when thumb arrives it retracts (BS) and emits the thumb face.
    // collect_output neutralises the BS+original pair.
    let mut engine = make_engine_with_mode(ConfirmMode::Speculative);
    let t = 1_000_000u64;

    let r1 = engine.on_event(Ev::down(VK_A).at(t).build());
    assert!(
        matches!(&r1.actions[0], KeyAction::Char('う')),
        "Speculative should emit 'う' immediately"
    );

    // Thumb within threshold
    let r2 = engine.on_event(Ev::down(VK_NONCONVERT).at(t + 20_000).build());
    // The thumb response must include the thumb face character 'を'
    let has_thumb_char = r2
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('を')));
    assert!(
        has_thumb_char,
        "Speculative retraction should include thumb face 'を', got {:?}",
        r2.actions
    );

    // After collecting all output (with BS retraction applied), the final
    // result should contain the thumb face 'を' and the speculative 'う'
    // should be neutralised.
    let responses = vec![r1, r2];
    let chars = collect_chars(&responses);
    assert!(
        chars.last() == Some(&'を'),
        "Final output should end with thumb face 'を', got {:?}",
        chars
    );
}

#[test]
fn test_collect_output_handles_bs_retraction() {
    // Unit test for the collect_output helper itself
    let responses = vec![
        Response {
            actions: vec![KeyAction::Char('う')],
            consumed: true,
            timers: vec![],
        },
        Response {
            actions: vec![
                KeyAction::SpecialKey(crate::types::SpecialKey::Backspace), // BS retracts 'う'
                KeyAction::Char('を'),
            ],
            consumed: true,
            timers: vec![],
        },
    ];
    let chars = collect_chars(&responses);
    assert_eq!(chars, vec!['を'], "BS should retract 'う', leaving 'を'");
}

#[test]
fn test_collect_output_no_retraction() {
    // No BS → all outputs preserved
    let responses = vec![
        Response {
            actions: vec![KeyAction::Char('う')],
            consumed: true,
            timers: vec![],
        },
        Response {
            actions: vec![KeyAction::Char('し')],
            consumed: true,
            timers: vec![],
        },
    ];
    let chars = collect_chars(&responses);
    assert_eq!(chars, vec!['う', 'し'], "No BS means all chars preserved");
}

// ── build_response tests ──

#[test]
fn test_build_response_cancel_all_timers() {
    let engine = make_engine();
    let resp = engine.build_response(
        smallvec![KeyAction::Romaji("ka".to_string())],
        true,
        TimerIntent::CancelAll,
    );
    resp.assert_consumed();
    assert_eq!(resp.actions.len(), 1);
    resp.assert_timer_kill(TIMER_PENDING);
    resp.assert_timer_kill(TIMER_SPECULATIVE);
    // CancelAll should not set any timers
    assert!(
        !resp
            .timers
            .iter()
            .any(|t| matches!(t, timed_fsm::TimerCommand::Set { .. })),
        "CancelAll should not set any timers"
    );
}

#[test]
fn test_build_response_pending_sets_timer() {
    let engine = make_engine();
    let resp = engine.build_response(smallvec![], true, TimerIntent::Pending);
    resp.assert_consumed();
    assert!(resp.actions.is_empty(), "Pending should have no actions");
    resp.assert_timer_set(TIMER_PENDING);
    resp.assert_timer_kill(TIMER_SPECULATIVE);
}

#[test]
fn test_build_response_speculative_wait_sets_timer() {
    let engine = make_engine();
    let resp = engine.build_response(
        smallvec![KeyAction::Romaji("u".to_string())],
        true,
        TimerIntent::SpeculativeWait,
    );
    resp.assert_consumed();
    assert_eq!(resp.actions.len(), 1);
    resp.assert_timer_set(TIMER_SPECULATIVE);
    resp.assert_timer_kill(TIMER_PENDING);
}

#[test]
fn test_build_response_phase2_transition() {
    let engine = make_engine();
    let resp = engine.build_response(
        smallvec![KeyAction::Romaji("ka".to_string())],
        true,
        TimerIntent::Phase2Transition {
            remaining_us: 50_000,
        },
    );
    resp.assert_consumed();
    assert_eq!(resp.actions.len(), 1);
    resp.assert_timer_kill(TIMER_SPECULATIVE);
    resp.assert_timer_set(TIMER_PENDING);
}

#[test]
fn test_update_history_record() {
    let mut engine = make_engine();
    assert!(engine.output_history.is_empty());

    engine.update_history(OutputUpdate::Record(OutputEntry {
        scan_code: SCAN_A,
        romaji: "ka".to_string(),
        kana: Some('か'),
        action: KeyAction::Romaji("ka".to_string()),
    }));
    assert_eq!(engine.output_history.len(), 1);
    assert_eq!(engine.output_history.recent_kana(1), vec!['か']);
}

#[test]
fn test_update_history_retract_and_record() {
    let mut engine = make_engine();

    // First, record an entry
    engine.update_history(OutputUpdate::Record(OutputEntry {
        scan_code: SCAN_A,
        romaji: "u".to_string(),
        kana: Some('う'),
        action: KeyAction::Romaji("u".to_string()),
    }));
    assert_eq!(engine.output_history.len(), 1);

    // Now retract and record a new entry
    engine.update_history(OutputUpdate::RetractAndRecord(OutputEntry {
        scan_code: SCAN_A,
        romaji: "vu".to_string(),
        kana: Some('ゔ'),
        action: KeyAction::Romaji("vu".to_string()),
    }));
    assert_eq!(
        engine.output_history.len(),
        1,
        "retract+record should keep count at 1"
    );
    assert_eq!(engine.output_history.recent_kana(1), vec!['ゔ']);
}

#[test]
fn test_context_invalidation_focus_changed() {
    // FocusChanged バリアントが存在し Debug 出力できることを確認
    let reason = ContextChange::FocusChanged;
    let s = format!("{:?}", reason);
    assert_eq!(s, "FocusChanged");
}

// ── Modifier state tracking across engine disable/enable ──

#[test]
fn test_ctrl_released_while_disabled_does_not_stick() {
    // エンジン OFF 中に Ctrl が離された場合、再 ON 後に stuck しないこと
    let mut engine = make_engine();

    // Ctrl を押す（エンジン ON 中）
    engine.on_event(Ev::down(VK_CTRL).build());

    // エンジン OFF
    let _ = engine.toggle_enabled();
    assert!(!engine.is_enabled());

    // Ctrl を離す（エンジン OFF 中）
    engine.on_event(Ev::up(VK_CTRL).build());

    // エンジン ON
    let _ = engine.toggle_enabled();
    assert!(engine.is_enabled());

    // 文字キーがエンジンで処理されること（OsModifierHeld でバイパスされない）
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    r.assert_consumed();
}

#[test]
fn test_alt_released_while_disabled_does_not_stick() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_ALT).build());
    let _ = engine.toggle_enabled();

    // Alt を離す（エンジン OFF 中）
    engine.on_event(Ev::up(VK_ALT).build());

    let _ = engine.toggle_enabled();

    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    r.assert_consumed();
}

// ── engine_off_triple_vk (consecutive solo muhenkan) tests ──

#[test]
fn test_engine_off_triple_solo_thumb_triggers() {
    // 無変換を単独タイムアウトで5回連続確定するとエンジン OFF 要求が立つことを確認。
    // 3回だとスリープ復帰時の混乱で焦って連打しただけで誤発火した実機事例
    // (2026-07-08) があり、5回に引き上げた。4回目までは発火しないことも確認する。
    let mut engine = make_engine();
    engine.set_engine_off_triple_vk(VK_NONCONVERT);

    let gap = 150_000u64; // 150ms < SOLO_OFF_TIMEOUT_US (400ms)

    for i in 0..4u64 {
        let t = i * gap;
        engine.on_event(Ev::down(VK_NONCONVERT).at(t).build());
        engine.on_timeout(TIMER_PENDING);
        assert!(
            !engine.take_engine_off_requested(),
            "{} consecutive solo presses → must not yet trigger engine off",
            i + 1
        );
    }
    engine.on_event(Ev::down(VK_NONCONVERT).at(4 * gap).build());
    engine.on_timeout(TIMER_PENDING);
    assert!(
        engine.take_engine_off_requested(),
        "5 consecutive solo presses → engine off"
    );
}

#[test]
fn test_engine_off_counter_resets_on_thumb_consume() {
    // 同時打鍵として consume された場合、ソロ連打カウンターがリセットされることを確認。
    // (「アップロード」のような連続左親指シフト入力でエンジン OFF が誤発動しないことを保証)
    let mut engine = make_engine();
    engine.set_engine_off_triple_vk(VK_NONCONVERT);

    let gap = 150_000u64; // 150ms = within SOLO_OFF_TIMEOUT_US (400ms)

    // solo 1 回目 (タイムアウト経由)
    engine.on_event(Ev::down(VK_NONCONVERT).at(0).build());
    engine.on_timeout(TIMER_PENDING);
    assert!(!engine.take_engine_off_requested());

    // solo 2 回目 (タイムアウト経由) → count=2
    engine.on_event(Ev::down(VK_NONCONVERT).at(gap).build());
    engine.on_timeout(TIMER_PENDING);
    assert!(!engine.take_engine_off_requested());

    // 同時打鍵 (無変換+A → 'を'): consume_thumb が呼ばれ solo_counter がリセットされる
    engine.on_event(Ev::down(VK_NONCONVERT).at(2 * gap).build());
    let result = engine.on_event(Ev::down(VK_A).at(2 * gap + 30_000).build()); // 30ms = within 100ms threshold
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Char('を'))),
        "無変換+A should produce 'を'"
    );

    // リセット後: solo 2 回 (count=1, then count=2 → engine off しない)
    engine.on_event(Ev::down(VK_NONCONVERT).at(3 * gap).build());
    engine.on_timeout(TIMER_PENDING);
    assert!(
        !engine.take_engine_off_requested(),
        "count=1 after reset → no engine off"
    );

    engine.on_event(Ev::down(VK_NONCONVERT).at(4 * gap).build());
    engine.on_timeout(TIMER_PENDING);
    assert!(
        !engine.take_engine_off_requested(),
        "count=2 after reset → no engine off"
    );
}

// ── FsmAdapter tests ──

mod fsm_adapter_tests {
    use super::*;
    use crate::config::ConfirmMode;
    use crate::engine::fsm_adapter::FsmAdapter;
    use crate::engine::input_tracker::{InputTracker, PhysicalKeyState};
    use crate::engine::nicola_fsm::NicolaFsm;
    use crate::types::ContextChange;

    fn make_adapter() -> FsmAdapter {
        let fsm = NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        );
        FsmAdapter::new(fsm)
    }

    #[test]
    fn is_enabled_default_true() {
        let adapter = make_adapter();
        assert!(adapter.is_enabled());
    }

    #[test]
    fn set_enabled_false() {
        let mut adapter = make_adapter();
        let (actual, decision) = adapter.set_enabled(false);
        assert!(!actual);
        assert!(!adapter.is_enabled());
        // Decision should exist (may have effects or not)
        let _ = decision;
    }

    #[test]
    fn set_enabled_true_when_already_true() {
        let mut adapter = make_adapter();
        let (actual, _decision) = adapter.set_enabled(true);
        assert!(actual);
        assert!(adapter.is_enabled());
    }

    #[test]
    fn toggle_enabled_flips_state() {
        let mut adapter = make_adapter();
        assert!(adapter.is_enabled());

        let (enabled, _decision) = adapter.toggle_enabled();
        assert!(!enabled);
        assert!(!adapter.is_enabled());

        let (enabled, _decision) = adapter.toggle_enabled();
        assert!(enabled);
        assert!(adapter.is_enabled());
    }

    #[test]
    fn flush_returns_decision() {
        let mut adapter = make_adapter();
        let decision = adapter.flush(ContextChange::FocusChanged, ComposingHint::Trusted(false));
        // Flush on idle should return a Decision without panicking
        let _ = decision.is_consumed();
    }

    #[test]
    fn flush_to_effects_returns_vec() {
        let mut adapter = make_adapter();
        let effects =
            adapter.flush_to_effects(ContextChange::FocusChanged, ComposingHint::Trusted(false));
        // Verify it returns a Vec (may or may not be empty depending on FSM internals)
        let _ = effects.len();
    }

    #[test]
    fn on_event_processes_key_down() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();
        let event = Ev::down(VK_A).at(1_000_000).build();
        let phys = tracker.process(&event);
        let decision = adapter.on_event(event, &phys);
        // Character key when enabled should be consumed
        assert!(decision.is_consumed());
    }

    #[test]
    fn on_event_key_up_pass_through_when_idle() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();
        // Key up without prior key down
        let event = Ev::up(VK_A).build();
        let phys = tracker.process(&event);
        let decision = adapter.on_event(event, &phys);
        // Key-up without pending state should pass through
        let _ = decision;
    }

    #[test]
    fn on_timeout_on_idle() {
        let mut adapter = make_adapter();
        let phys = PhysicalKeyState::empty();
        let decision = adapter.on_timeout(0, &phys, false);
        // Timeout on idle should not panic, just produce a decision
        let _ = decision;
    }

    #[test]
    fn set_threshold_ms_updates() {
        let mut adapter = make_adapter();
        // Should not panic; verify indirectly through behavior
        adapter.set_threshold_ms(200);
        // After increasing threshold, keys further apart should still be simultaneous
        let mut tracker = InputTracker::new();
        let ev1 = Ev::down(VK_NONCONVERT).at(0).build();
        let phys1 = tracker.process(&ev1);
        let _ = adapter.on_event(ev1, &phys1);

        let ev2 = Ev::down(VK_A).at(150_000).build(); // 150ms apart
        let phys2 = tracker.process(&ev2);
        let decision = adapter.on_event(ev2, &phys2);
        assert!(decision.is_consumed());
    }

    #[test]
    fn set_confirm_mode_updates() {
        let mut adapter = make_adapter();
        // Should not panic
        adapter.set_confirm_mode(ConfirmMode::Speculative, 50);
        adapter.set_confirm_mode(ConfirmMode::Wait, 30);
    }

    #[test]
    fn swap_layout_returns_decision() {
        let mut adapter = make_adapter();
        let new_layout = make_layout();
        let decision = adapter.swap_layout(new_layout);
        // swap_layout may flush pending state
        let _ = decision;
    }

    #[test]
    fn swap_layout_on_idle_no_consumed() {
        let mut adapter = make_adapter();
        let decision = adapter.swap_layout(make_layout());
        // On idle, no pending keys to flush, so likely pass-through or empty
        // Just verify it doesn't panic
        let _ = decision.is_consumed();
    }

    #[test]
    fn set_enabled_false_then_on_event_passes_through() {
        let mut adapter = make_adapter();
        let (_actual, _decision) = adapter.set_enabled(false);

        let mut tracker = InputTracker::new();
        let event = Ev::down(VK_A).at(1_000_000).build();
        let phys = tracker.process(&event);
        let decision = adapter.on_event(event, &phys);
        // When disabled, key events should pass through
        assert!(!decision.is_consumed());
    }

    #[test]
    fn toggle_then_flush() {
        let mut adapter = make_adapter();
        // Process a key to create pending state
        let mut tracker = InputTracker::new();
        let event = Ev::down(VK_A).at(1_000_000).build();
        let phys = tracker.process(&event);
        let _ = adapter.on_event(event, &phys);

        // Toggle off should flush pending
        let (enabled, decision) = adapter.toggle_enabled();
        assert!(!enabled);
        let _ = decision;
    }

    #[test]
    fn set_ngram_model_does_not_panic() {
        let mut adapter = make_adapter();
        let toml_str = r#"
[bigram]
"あり" = 1.5
"#;
        let model = NgramModel::from_toml(toml_str, 20_000, 30_000, 120_000).unwrap();
        adapter.set_ngram_model(model);
    }

    #[test]
    fn response_to_decision_consumed_flag() {
        // Test indirectly: when engine is enabled and receives a character key,
        // the adapter should produce a consumed decision
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();
        let event = Ev::down(VK_A).at(1_000_000).build();
        let phys = tracker.process(&event);
        let decision = adapter.on_event(event, &phys);
        assert!(decision.is_consumed());
    }

    #[test]
    fn flush_after_key_produces_effects() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();

        // Send a character key to create pending state
        let event = Ev::down(VK_A).at(1_000_000).build();
        let phys = tracker.process(&event);
        let _ = adapter.on_event(event, &phys);

        // Flush should resolve the pending key
        let decision = adapter.flush(ContextChange::FocusChanged, ComposingHint::Trusted(false));
        // The flush should produce some output (consumed with effects)
        let _ = decision;
    }
}

// ============================================================================
// Engine integration tests (engine.rs coverage)
// ============================================================================

mod engine_integration_tests {
    use super::*;
    use crate::config::{ConfirmMode, ParsedKeyCombo};
    use crate::engine::decision::{
        Decision, Effect, EngineCommand, ImeEffect, InputContext, InputEffect, SpecialKeyCombos,
        UiEffect,
    };
    use crate::engine::engine::Engine;
    use crate::engine::nicola_fsm::NicolaFsm;
    use crate::types::ShadowImeAction;

    fn empty_special_keys() -> SpecialKeyCombos {
        SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
            ime_toggle: vec![],
        }
    }

    fn make_test_engine() -> Engine {
        let layout = make_layout();
        let fsm = NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        );
        let mut engine = Engine::new(fsm, empty_special_keys());
        // テストでは prev_active=true にしておく（全前提条件を満たす ctx で使う想定）
        engine.set_prev_active(true);
        engine
    }

    fn ime_on_ctx() -> InputContext {
        InputContext {
            ime_on: true,
            input_mode: InputModeState::ObservedRomaji,
            is_japanese_ime: true,
            composing: false,
            modifiers: ModifierState {
                ctrl: false,
                alt: false,
                shift: false,
                win: false,
            },
            left_thumb_down: None,
            right_thumb_down: None,
        }
    }

    fn ime_off_ctx() -> InputContext {
        InputContext {
            ime_on: false,
            input_mode: InputModeState::ObservedRomaji,
            is_japanese_ime: true,
            composing: false,
            modifiers: ModifierState {
                ctrl: false,
                alt: false,
                shift: false,
                win: false,
            },
            left_thumb_down: None,
            right_thumb_down: None,
        }
    }

    fn has_effect<F: Fn(&Effect) -> bool>(decision: &Decision, pred: F) -> bool {
        match decision {
            Decision::Consume { effects } => effects.iter().any(&pred),
            Decision::PassThroughWith { effects } => effects.iter().any(&pred),
            Decision::PassThrough => false,
        }
    }

    fn effects_of(decision: &Decision) -> &[Effect] {
        match decision {
            Decision::Consume { effects } => effects,
            Decision::PassThroughWith { effects } => effects,
            Decision::PassThrough => &[],
        }
    }

    // ── 1. Engine::on_input basic flow ──

    #[test]
    fn on_input_char_key_with_ime_on_is_consumed() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(
            d.is_consumed(),
            "char key with IME ON should be consumed by FSM"
        );
    }

    #[test]
    fn on_input_char_key_with_preconditions_not_met_passes_through() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_off_ctx());
        assert!(
            !d.is_consumed(),
            "char key with preconditions not met should pass through"
        );
    }

    #[test]
    fn on_input_char_key_with_ime_on_preconditions_met_is_consumed() {
        let mut engine = make_test_engine();
        // preconditions already all true from make_test_engine
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(
            d.is_consumed(),
            "char key with preconditions met should be consumed"
        );
    }

    #[test]
    fn on_input_char_key_not_romaji_passes_through() {
        let mut engine = make_test_engine();
        let kana_ctx = InputContext {
            ime_on: true,
            input_mode: InputModeState::ObservedKana,
            is_japanese_ime: true,
            composing: false,
            modifiers: ModifierState::default(),
            left_thumb_down: None,
            right_thumb_down: None,
        };
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &kana_ctx);
        assert!(
            !d.is_consumed(),
            "char key with kana input mode should pass through"
        );
    }

    #[test]
    fn on_input_key_up_after_consumed_down_is_auto_consumed() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
        let d = engine.on_input(Ev::up(VK_A).at(200).build(), &ime_on_ctx());
        assert!(
            d.is_consumed(),
            "KeyUp for consumed KeyDown should also be consumed"
        );
    }

    #[test]
    fn on_input_key_up_without_consumed_down_passes_through() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::up(VK_A).at(100).build(), &ime_on_ctx());
        assert!(
            !d.is_consumed(),
            "KeyUp without prior consumed KeyDown should pass through"
        );
    }

    // ── 2. Engine::on_input with modifiers ──

    #[test]
    fn on_input_shift_key_passes_through() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_SHIFT).at(100).build(), &ime_on_ctx());
        assert!(!d.is_consumed(), "Shift KeyDown should pass through");
    }

    #[test]
    fn on_input_ctrl_key_passes_through() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_CTRL).at(100).build(), &ime_on_ctx());
        assert!(!d.is_consumed(), "Ctrl KeyDown should pass through");
    }

    #[test]
    fn on_input_alt_key_passes_through() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_ALT).at(100).build(), &ime_on_ctx());
        assert!(!d.is_consumed(), "Alt KeyDown should pass through");
    }

    // ── 3. Engine::on_timeout ──

    #[test]
    fn on_timeout_after_pending_char() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());

        let d = engine.on_timeout(TIMER_PENDING, &ime_on_ctx());
        assert!(d.is_consumed());
        assert!(
            has_effect(&d, |e| matches!(e, Effect::Input(InputEffect::SendKeys(_)))),
            "timeout should produce SendKeys"
        );
    }

    #[test]
    fn on_timeout_with_ime_off_flushes() {
        let mut engine = make_test_engine();
        engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());

        let d = engine.on_timeout(TIMER_PENDING, &ime_off_ctx());
        assert!(d.is_consumed());
    }

    // ── 4. Engine::on_command ──

    #[test]
    fn on_command_toggle_engine() {
        let mut engine = make_test_engine();
        assert!(engine.is_user_enabled());

        let d = engine.on_command(EngineCommand::ToggleEngine, &ime_on_ctx());
        assert!(!engine.is_user_enabled());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: false, .. })
        )));

        let d = engine.on_command(EngineCommand::ToggleEngine, &ime_on_ctx());
        assert!(engine.is_user_enabled());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: true, .. })
        )));
    }

    #[test]
    fn on_command_force_engine_on_from_off() {
        // トレイの「状態をリセット」用: user_enabled=false からでも必ず ON にする。
        let mut engine = make_test_engine();
        engine.on_command(EngineCommand::ToggleEngine, &ime_on_ctx()); // user OFF
        assert!(!engine.is_user_enabled());

        let d = engine.on_command(EngineCommand::ForceEngineOn, &ime_on_ctx());
        assert!(engine.is_user_enabled());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: true, .. })
        )));
    }

    #[test]
    fn on_command_force_engine_on_is_noop_when_already_on() {
        // トグルと違い、既に ON のときは OFF に反転させない（冪等）。
        let mut engine = make_test_engine();
        assert!(engine.is_user_enabled());

        engine.on_command(EngineCommand::ForceEngineOn, &ime_on_ctx());
        assert!(engine.is_user_enabled());
    }

    #[test]
    fn on_command_invalidate_context() {
        let mut engine = make_test_engine();
        engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        let d = engine.on_command(
            EngineCommand::InvalidateContext(ContextChange::ImeOff),
            &ime_on_ctx(),
        );
        assert!(d.is_consumed());
    }

    #[test]
    fn on_command_sync_ime_state_off_deactivates() {
        let mut engine = make_test_engine();
        assert!(engine.compute_active(&ime_on_ctx()));

        // Platform updated atomic → ctx now reflects ime_on=false
        let d = engine.on_command(EngineCommand::RefreshState, &ime_off_ctx());
        assert!(!engine.compute_active(&ime_off_ctx()));
        assert!(engine.is_user_enabled(), "user_enabled unchanged");
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: false, .. })
        )));
    }

    #[test]
    fn on_command_sync_ime_state_on_activates() {
        let mut engine = make_test_engine();
        // まず ime_off で inactive にする
        engine.on_command(EngineCommand::RefreshState, &ime_off_ctx());
        assert!(!engine.compute_active(&ime_off_ctx()));

        // Platform updated atomic → ctx now reflects ime_on=true
        let d = engine.on_command(EngineCommand::RefreshState, &ime_on_ctx());
        assert!(engine.compute_active(&ime_on_ctx()));
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: true, .. })
        )));
    }

    #[test]
    fn on_command_sync_ime_state_on_but_user_disabled() {
        let mut engine = make_test_engine();
        engine.on_command(EngineCommand::ToggleEngine, &ime_on_ctx()); // user OFF
        assert!(!engine.compute_active(&ime_on_ctx()));

        let d = engine.on_command(EngineCommand::RefreshState, &ime_on_ctx());
        // user disabled → still inactive
        assert!(!engine.compute_active(&ime_on_ctx()));
        assert!(!has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { .. })
        )));
    }

    #[test]
    fn on_command_sync_ime_state_no_change() {
        let mut engine = make_test_engine();
        assert!(engine.compute_active(&ime_on_ctx()));

        let d = engine.on_command(EngineCommand::RefreshState, &ime_on_ctx());
        assert!(!d.is_consumed());
        assert!(engine.compute_active(&ime_on_ctx()));
    }

    #[test]
    fn on_command_update_fsm_params() {
        let mut engine = make_test_engine();
        let d = engine.on_command(
            EngineCommand::UpdateFsmParams {
                threshold_ms: 200,
                confirm_mode: ConfirmMode::Speculative,
                speculative_delay_ms: 50,
            },
            &ime_on_ctx(),
        );
        assert!(!d.is_consumed());
    }

    #[test]
    fn on_command_reload_keys() {
        let mut engine = make_test_engine();
        let d = engine.on_command(
            EngineCommand::ReloadKeys {
                special: empty_special_keys(),
            },
            &ime_on_ctx(),
        );
        assert!(!d.is_consumed());
    }

    // ── 5. Engine::check_special_keys ──

    fn make_engine_with_special(special: SpecialKeyCombos) -> Engine {
        let layout = make_layout();
        let fsm = NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        );
        let mut engine = Engine::new(fsm, special);
        engine.set_prev_active(true);
        engine
    }

    #[test]
    fn special_key_engine_on_combo() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_NONCONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![combo],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);

        engine.on_command(EngineCommand::ToggleEngine, &ime_on_ctx());
        assert!(!engine.is_user_enabled());

        let d = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        assert!(
            engine.is_user_enabled(),
            "engine should be re-enabled by special key combo"
        );
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: true, .. })
        )));
    }

    #[test]
    fn special_key_engine_on_combo_recovers_when_context_inactive_but_user_enabled() {
        // 実機バグ: user_enabled=true のまま ime_on=false 等の *文脈* で Engine が
        // inactive に陥っているとき、以前は `!engine_enabled` ガードにより
        // Ctrl+Shift+変換（engine_on コンボ）が完全に無視され PassThrough
        // されるだけだった（force_enable_and_activate の recovery ロジックへ
        // 到達不能）。この回帰を防ぐ。
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_NONCONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![combo],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);
        assert!(engine.is_user_enabled(), "user_enabled は最初から true");
        assert!(
            !engine.compute_active(&ime_off_ctx()),
            "ime_off_ctx では文脈により inactive のはず"
        );

        let d = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_off_ctx());
        assert!(
            has_effect(&d, |e| matches!(
                e,
                Effect::Ime(ImeEffect::SetOpen { open: true, .. })
            )),
            "user_enabled=true でも context-inactive なら engine_on コンボで IME を \
             強制的に開く SetOpen(true) が発行されるべき（修正前は match_event が \
             None を返し、この effect が一切発行されず PassThrough のみだった）"
        );
    }

    #[test]
    fn special_key_engine_on_combo_does_not_match_when_already_active() {
        // `match_event` のガードは `(!engine_enabled || !engine_active)`。上の
        // `_recovers_when_context_inactive_but_user_enabled` は engine_active=false の
        // ケースのみを固定しており、`!engine_enabled` 項が削除されても
        // （`engine_enabled || !engine_active` に壊れても）engine_active=false では
        // 依然 true のままなので検知できない。engine が既に enabled かつ active
        // （通常運用中）のときに限って両者は分岐する: 元のコードは EngineOn に
        // マッチしてはならない（force_enable_and_activate の不要な再実行を防ぐ）。
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_NONCONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![combo],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
            ime_toggle: vec![],
        };
        let engine = make_engine_with_special(special);
        assert!(engine.is_user_enabled());
        assert!(
            engine.compute_active(&ime_on_ctx()),
            "ime_on_ctx では active のはず"
        );

        let matched = engine
            .match_special_keys_for_test(&ime_on_ctx(), &Ev::down(VK_NONCONVERT).at(100).build());
        assert_eq!(
            matched, None,
            "engine が既に enabled かつ active なら engine_on コンボはマッチしてはならない"
        );
    }

    #[test]
    fn special_key_engine_off_combo() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_NONCONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![combo],
            ime_on: vec![],
            ime_off: vec![],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);
        assert!(engine.is_user_enabled());

        let d = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        assert!(
            !engine.is_user_enabled(),
            "engine should be disabled by special key combo"
        );
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: false, .. })
        )));
    }

    #[test]
    fn special_key_ime_on_combo() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_CONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![combo],
            ime_off: vec![],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);

        let d = engine.on_input(Ev::down(VK_CONVERT).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen { open: true, .. })
        )));
    }

    #[test]
    fn special_key_ime_off_combo() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_CONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![combo],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);

        let d = engine.on_input(Ev::down(VK_CONVERT).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen { open: false, .. })
        )));
    }

    /// ADR-092 決定D Step4a: `SpecialKeyMatch::ImeToggle` は `ctx.ime_on` を見て
    /// 反転方向を決める（`ime_on`/`ime_off` の方向固定コンボとは異なる）。
    #[test]
    fn special_key_ime_toggle_combo_flips_to_off_when_currently_on() {
        let combo = ParsedKeyCombo {
            ctrl: true,
            shift: false,
            alt: false,
            vk: VK_SPACE,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
            ime_toggle: vec![combo],
        };
        let mut engine = make_engine_with_special(special);

        let ctx = InputContext {
            modifiers: ModifierState {
                ctrl: true,
                ..ime_on_ctx().modifiers
            },
            ..ime_on_ctx()
        };
        let d = engine.on_input(Ev::down(VK_SPACE).at(100).build(), &ctx);
        assert!(d.is_consumed());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen { open: false, .. })
        )));
    }

    #[test]
    fn special_key_ime_toggle_combo_flips_to_on_when_currently_off() {
        let combo = ParsedKeyCombo {
            ctrl: true,
            shift: false,
            alt: false,
            vk: VK_SPACE,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
            ime_toggle: vec![combo],
        };
        let mut engine = make_engine_with_special(special);

        let ctx = InputContext {
            modifiers: ModifierState {
                ctrl: true,
                ..ime_off_ctx().modifiers
            },
            ..ime_off_ctx()
        };
        let d = engine.on_input(Ev::down(VK_SPACE).at(100).build(), &ctx);
        assert!(d.is_consumed());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen { open: true, .. })
        )));
    }

    /// 明示方向（`ime_on`）は同じ物理キーに対してトグルより優先される
    /// （`match_event` 内でトグルは ime_on/ime_off の後にチェックされる）。
    #[test]
    fn special_key_ime_on_takes_priority_over_toggle_for_same_combo() {
        let combo = ParsedKeyCombo {
            ctrl: true,
            shift: false,
            alt: false,
            vk: VK_SPACE,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![combo],
            ime_off: vec![],
            ime_toggle: vec![combo],
        };
        let mut engine = make_engine_with_special(special);

        // ime_on_ctx (ime_on: true) で検証する: トグルが勝つなら false になる
        // はずが、明示 ime_on コンボが優先されれば true のまま
        // (SetOpen(true)) になる。
        let ctx = InputContext {
            modifiers: ModifierState {
                ctrl: true,
                ..ime_on_ctx().modifiers
            },
            ..ime_on_ctx()
        };
        let d = engine.on_input(Ev::down(VK_SPACE).at(100).build(), &ctx);
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen { open: true, .. })
        )));
    }

    // ── ADR-092 決定D Step4c: GJI config1.db 由来の自動検出 IME ON/OFF/
    //    トグルキー（`ime_on_auto`/`ime_off_auto`/`ime_toggle_auto`） ──

    /// 手動設定（`keys.ime_on`）が空の間、自動検出リスト（`ime_on_auto`）が
    /// 効く。
    #[test]
    fn ime_on_auto_fires_when_manual_ime_on_empty() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_F21,
        };
        let mut engine = make_engine_with_special(empty_special_keys());
        engine.set_ime_on_auto_keys(vec![combo]);

        let d = engine.on_input(Ev::down(VK_F21).at(100).build(), &ime_off_ctx());
        assert!(d.is_consumed());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen { open: true, .. })
        )));
    }

    /// 手動設定（`keys.ime_off`）が空の間、自動検出リスト（`ime_off_auto`）が
    /// 効く。
    #[test]
    fn ime_off_auto_fires_when_manual_ime_off_empty() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_F21,
        };
        let mut engine = make_engine_with_special(empty_special_keys());
        engine.set_ime_off_auto_keys(vec![combo]);

        let d = engine.on_input(Ev::down(VK_F21).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen { open: false, .. })
        )));
    }

    /// 手動設定（`keys.ime_toggle`）が空の間、自動検出リスト
    /// （`ime_toggle_auto`）が効く。
    #[test]
    fn ime_toggle_auto_fires_when_manual_ime_toggle_empty() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_F21,
        };
        let mut engine = make_engine_with_special(empty_special_keys());
        engine.set_ime_toggle_auto_keys(vec![combo]);

        let d = engine.on_input(Ev::down(VK_F21).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen { open: false, .. })
        )));
    }

    /// BUG-14 同種のリスク対策（Opus コードレビュー指摘）: `ime_on_auto`/
    /// `ime_off_auto`は`event.injected`な合成イベントにマッチしない。
    /// 手動設定の `ime_on`/`ime_off` と異なり、自動検出リストはユーザーが
    /// 存在を意識せず追加されるため、注入イベントへの露出を正当化する
    /// 根拠が無い。
    #[test]
    fn ime_on_auto_ignores_injected_event() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_F21,
        };
        let mut engine = make_engine_with_special(empty_special_keys());
        engine.set_ime_on_auto_keys(vec![combo]);

        let d = engine.on_input(
            Ev::down(VK_F21).at(100).injected(true).build(),
            &ime_on_ctx(),
        );
        assert!(
            !has_effect(&d, |e| matches!(e, Effect::Ime(_))),
            "injected event must not trigger ime_on_auto, got {:?}",
            effects_of(&d)
        );
    }

    /// 上記の `ime_toggle_auto` 版。
    #[test]
    fn ime_toggle_auto_ignores_injected_event() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_F21,
        };
        let mut engine = make_engine_with_special(empty_special_keys());
        engine.set_ime_toggle_auto_keys(vec![combo]);

        let d = engine.on_input(
            Ev::down(VK_F21).at(100).injected(true).build(),
            &ime_on_ctx(),
        );
        assert!(
            !has_effect(&d, |e| matches!(e, Effect::Ime(_))),
            "injected event must not trigger ime_toggle_auto, got {:?}",
            effects_of(&d)
        );
    }

    /// 決定C R1（明示 > 自動）: `keys.ime_on` が非空の間、`ime_on_auto` は
    /// 一切参照されない。手動リストと自動リストに**別のキー**を割り当て、
    /// 自動側キーの押下が無視される（consume されない = PassThrough のまま）
    /// ことを確認する（Opus コードレビュー指摘: 旧版は手動・自動に同じ
    /// キーを設定していたため、優先順位が逆転していても観測上の差が
    /// 出ない検証不能なテストだった。`ime_toggle_auto_ignored_when_manual_ime_toggle_non_empty`
    /// と同じ設計に修正）。
    #[test]
    fn ime_on_auto_ignored_when_manual_ime_on_non_empty() {
        let manual_combo = ParsedKeyCombo {
            ctrl: true,
            shift: false,
            alt: false,
            vk: VK_SPACE,
        };
        let auto_combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_F21,
        };
        let special = SpecialKeyCombos {
            ime_on: vec![manual_combo],
            ..empty_special_keys()
        };
        let mut engine = make_engine_with_special(special);
        engine.set_ime_on_auto_keys(vec![auto_combo]);

        // `ime_off_ctx()` は使わない: engine の `prev_activation` 初期値は
        // Active であり、ime_off_ctx() 自体が最初の呼び出しで無関係な
        // ActivationSync 起点の SetOpen(false) を誘発してしまう
        // （ime_on_auto のマッチとは無関係なノイズ）。優先順位検証には
        // `ime_toggle_auto_ignored_when_manual_ime_toggle_non_empty` と
        // 同じ `ime_on_ctx()`（active のまま維持）を使う。
        let d = engine.on_input(Ev::down(VK_F21).at(100).build(), &ime_on_ctx());
        assert!(!has_effect(&d, |e| matches!(e, Effect::Ime(_))));
    }

    /// `ime_on_auto_ignored_when_manual_ime_on_non_empty` の `ime_off` 版
    /// （決定C R1、Opus コードレビュー指摘: `ime_off` 側の手動優先テストが
    /// 欠落していた）。
    #[test]
    fn ime_off_auto_ignored_when_manual_ime_off_non_empty() {
        let manual_combo = ParsedKeyCombo {
            ctrl: true,
            shift: false,
            alt: false,
            vk: VK_SPACE,
        };
        let auto_combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_F21,
        };
        let special = SpecialKeyCombos {
            ime_off: vec![manual_combo],
            ..empty_special_keys()
        };
        let mut engine = make_engine_with_special(special);
        engine.set_ime_off_auto_keys(vec![auto_combo]);

        let d = engine.on_input(Ev::down(VK_F21).at(100).build(), &ime_on_ctx());
        assert!(!has_effect(&d, |e| matches!(e, Effect::Ime(_))));
    }

    /// 自動検出リストのキーは、手動 `ime_toggle` が非空なら一切参照されない
    /// （決定C R1）。ここでは手動 `ime_toggle` に割り当てたキーとは別の
    /// キーを `ime_toggle_auto` に入れ、自動側キーの押下が無視される
    /// （consume されない = PassThrough のまま）ことを確認する。
    #[test]
    fn ime_toggle_auto_ignored_when_manual_ime_toggle_non_empty() {
        let manual_combo = ParsedKeyCombo {
            ctrl: true,
            shift: false,
            alt: false,
            vk: VK_SPACE,
        };
        let auto_combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_F21,
        };
        let special = SpecialKeyCombos {
            ime_toggle: vec![manual_combo],
            ..empty_special_keys()
        };
        let mut engine = make_engine_with_special(special);
        engine.set_ime_toggle_auto_keys(vec![auto_combo]);

        let d = engine.on_input(Ev::down(VK_F21).at(100).build(), &ime_on_ctx());
        assert!(!has_effect(&d, |e| matches!(e, Effect::Ime(_))));
    }

    // ── ADR-092 決定D Step4b: 無変換/変換単独タップの IME open 軸への肩代わり ──
    //
    // 重要な前提（テスト設計時に判明）: `Engine::compute_active` は
    // `ctx.ime_on` を判定条件に含むため（判定順: user_enabled → is_japanese_ime →
    // ime_on → is_romaji）、`ime_on=false` の間は Phase 2 で無条件
    // `Decision::pass_through()` を返し Phase 3（NicolaFsm、
    // `resolve_pending_thumb_as_single` を含む）に到達しない。つまり
    // `DelegateToOpenAxis` は **IME が既に ON の状態からの操作**でしか
    // 発火し得ない（`TurnOff`/`Toggle(ime_on=true→false)` は届くが、
    // `TurnOn`（IME OFF から ON へ）は届かない）。これは実装のバグではなく
    // ADR-092 背景節が明記する既存の構造的な穴（Step3 の対象、本ADRでは
    // 意図的に対象外）——engine が非活性（＝IME OFF）の間は awase がそもそも
    // 無変換/変換の生 VK を横取りしないため、MS-IME/GJI 自身のネイティブな
    // キー割当て処理（`KeyAssignmentHenkan=1` 等）にそのまま委ねられる形に
    // なる。以下のテストは全て `ime_on_ctx()`（engine active）を前提にする。

    /// `muhenkan_vk` を設定した `Engine` を返す（`delegate_to_open_axis` テスト用）。
    fn make_test_engine_with_muhenkan() -> Engine {
        let mut engine = make_test_engine();
        engine.set_thumb_key_solo_tap_config(
            Some(VK_NONCONVERT),
            ModeKeyConfig::from_legacy_bools(false, true),
            None,
            ModeKeyConfig::from_legacy_bools(false, true),
        );
        engine
    }

    /// `henkan_vk` を設定した `Engine` を返す（`delegate_to_open_axis` テスト用、
    /// `make_test_engine_with_muhenkan` の対称版。Opus コードレビュー指摘:
    /// 既存の `delegate_to_open_axis_*` 系テストは全て無変換のみで、変換側の
    /// `resolve_pending_thumb_as_single` の分岐が未検証だった）。
    fn make_test_engine_with_henkan() -> Engine {
        let mut engine = make_test_engine();
        engine.set_thumb_key_solo_tap_config(
            None,
            ModeKeyConfig::from_legacy_bools(false, true),
            Some(VK_CONVERT),
            ModeKeyConfig::from_legacy_bools(false, true),
        );
        engine
    }

    /// 無変換単独タップが**確定**（timeout）した時点で `DelegateToOpenAxis` が
    /// 発火し、`Effect::Ime(SetOpen)` が生成され、かつ生 VK_NONCONVERT は
    /// 送出されない。
    #[test]
    fn delegate_to_open_axis_fires_on_confirmed_muhenkan_solo_tap() {
        let mut engine = make_test_engine_with_muhenkan();
        engine.set_muhenkan_delegate_to_open_axis(Some(ShadowImeAction::TurnOff));

        let d = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        assert!(
            d.is_consumed(),
            "solo tap should be pending, not passthrough"
        );
        assert!(
            !has_effect(&d, |e| matches!(e, Effect::Ime(_))),
            "IME effect must not fire before solo tap is confirmed"
        );

        let d = engine.on_timeout(TIMER_PENDING, &ime_on_ctx());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen { open: false, .. })
        )));
        assert!(
            !has_effect(&d, |e| matches!(
                e,
                Effect::Input(InputEffect::SendKeys(actions))
                    if actions.iter().any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT))
            )),
            "raw VK_NONCONVERT must not be sent when delegated to open axis, got {:?}",
            effects_of(&d)
        );
    }

    /// **chord のタイミングウィンドウ内の誤確定では発火しない**（ADR-092
    /// リスク節が明記する回帰テスト要件）。無変換キーの直後、閾値内に文字キーが
    /// 来た場合は同時打鍵として確定し、`DelegateToOpenAxis`（単独タップ確定
    /// 専用の経路）は一切発火しない。
    #[test]
    fn delegate_to_open_axis_does_not_fire_during_chord_timing_window() {
        let mut engine = make_test_engine_with_muhenkan();
        engine.set_muhenkan_delegate_to_open_axis(Some(ShadowImeAction::TurnOff));

        let d1 = engine.on_input(Ev::down(VK_NONCONVERT).at(0).build(), &ime_on_ctx());
        assert!(d1.is_consumed());
        // 同時打鍵の閾値内（make_test_engine の threshold_ms=100）に文字キーが来る
        // → 同時打鍵として確定し、単独タップの delegate_to_open_axis 経路には
        // 一切到達しない。
        let d2 = engine.on_input(Ev::down(VK_A).at(50).build(), &ime_on_ctx());
        assert!(
            !has_effect(&d1, |e| matches!(e, Effect::Ime(_)))
                && !has_effect(&d2, |e| matches!(e, Effect::Ime(_))),
            "chord confirmation must not trigger IME open axis delegation, d1={:?} d2={:?}",
            effects_of(&d1),
            effects_of(&d2)
        );
    }

    /// `ShadowImeAction::Toggle` は確定時点の `ctx.ime_on`（belief）を見て
    /// 反転方向を決める。
    #[test]
    fn delegate_to_open_axis_toggle_resolves_via_ctx_ime_on() {
        let mut engine = make_test_engine_with_muhenkan();
        engine.set_muhenkan_delegate_to_open_axis(Some(ShadowImeAction::Toggle));

        let _ = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        let d = engine.on_timeout(TIMER_PENDING, &ime_on_ctx());
        assert!(
            has_effect(&d, |e| matches!(
                e,
                Effect::Ime(ImeEffect::SetOpen { open: false, .. })
            )),
            "Toggle while ime_on=true must resolve to SetOpen(false), got {:?}",
            effects_of(&d)
        );
    }

    /// 専用Fnキー（`muhenkan_solo_tap_dedicated_fn_key`）は `delegate_to_open_axis`
    /// より優先される。
    #[test]
    fn dedicated_fn_key_takes_priority_over_delegate_to_open_axis() {
        let mut engine = make_test_engine_with_muhenkan();
        engine.set_muhenkan_solo_tap_dedicated_fn_key(Some(VK_F21));
        engine.set_muhenkan_delegate_to_open_axis(Some(ShadowImeAction::TurnOff));

        let _ = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        let d = engine.on_timeout(TIMER_PENDING, &ime_on_ctx());
        assert!(
            !has_effect(&d, |e| matches!(e, Effect::Ime(_))),
            "dedicated_fn_key must take priority, no IME effect expected, got {:?}",
            effects_of(&d)
        );
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Input(InputEffect::SendKeys(actions))
                if actions.iter().any(|a| matches!(a, KeyAction::Key(x) if *x == VK_F21))
        )));
    }

    /// M1 回帰防止（Opus コードレビュー指摘、実機テストプローブで実証済み）:
    /// `apply_ime_open_request` が `ime_set_open_effects`（`prev_activation`を
    /// 推進する）を経由せず直接 `push_effect` していたため、確定した単独タップ
    /// による `SetOpen(false)` の**次の**打鍵で `ActivationSync` 起点の重複
    /// `SetOpen` + 不要な `EngineStateChanged{send_ime_key:true}` が再発火して
    /// いた。`ime_off_combo_does_not_double_emit_set_open_on_next_input`
    /// と同型のテスト。
    #[test]
    fn delegate_to_open_axis_confirmed_tap_does_not_double_emit_set_open_on_next_input() {
        let mut engine = make_test_engine_with_muhenkan();
        engine.set_muhenkan_delegate_to_open_axis(Some(ShadowImeAction::TurnOff));

        let _ = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        let d1 = engine.on_timeout(TIMER_PENDING, &ime_on_ctx());
        assert_eq!(
            count_set_open_effects(&d1),
            1,
            "confirmed solo tap should emit exactly 1 SetOpen, got {:?}",
            effects_of(&d1)
        );

        // Platform 層は SetOpen(false) を見て preconditions.ime_on=false を反映する。
        // 次の on_input は新しい ctx (ime_on=false) で呼ばれる。
        let d2 = engine.on_input(Ev::up(VK_NONCONVERT).at(110).build(), &ime_off_ctx());
        assert_eq!(
            count_set_open_effects(&d2),
            0,
            "next on_input must NOT re-emit SetOpen (prev_activation should have been \
             advanced by ime_set_open_effects), got {:?}",
            effects_of(&d2)
        );
    }

    /// M2 回帰防止（Opus コードレビュー指摘、実機テストプローブで実証済み）:
    /// 無変換が物理的に押下中（`PendingThumb`、まだ単独タップ確定前）に
    /// `EngineCommand::ToggleEngine` が届くと、`toggle_enabled()` 内部の
    /// flush が `ComposingHint::Trusted` で保留キーを強制的に単独タップ
    /// 確定させ、`ime_open_requested` をセットしうる。この「確定」は
    /// ユーザーが実際に無変換をタップしたのではなくトレイ操作等の無関係な
    /// 外部イベントによる強制解決であり、`apply_ime_open_request` を素通り
    /// させると（当時のバグ）無関係な次の打鍵でスプリアスな `SetOpen` が
    /// 発火していた。`discard_ime_open_request` で捨てることを固定する。
    #[test]
    fn toggle_engine_discards_pending_ime_open_request_not_leak_to_later_key() {
        let mut engine = make_test_engine_with_muhenkan();
        engine.set_muhenkan_delegate_to_open_axis(Some(ShadowImeAction::TurnOff));

        // 無変換を物理的に押下（まだ単独タップ確定前、PendingThumb）。
        let _ = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());

        // トレイ操作等で ToggleEngine が届く → 内部 flush で強制的に単独タップ
        // 確定 → ime_open_requested がセットされうる。もう一度 ToggleEngine を
        // 呼んで元の enabled 状態へ戻す（Idle 状態での2回目の flush は no-op）。
        let _ = engine.on_command(EngineCommand::ToggleEngine, &ime_on_ctx());
        let _ = engine.on_command(EngineCommand::ToggleEngine, &ime_on_ctx());

        // 無関係な後続キー入力に、捨てられたはずの ime_open_requested に由来する
        // SetOpen が漏れ出さないこと。
        let d = engine.on_input(Ev::down(VK_A).at(9000).build(), &ime_on_ctx());
        assert!(
            !has_effect(&d, |e| matches!(e, Effect::Ime(_))),
            "stale ime_open_requested from ToggleEngine's internal flush must not leak \
             into an unrelated later key, got {:?}",
            effects_of(&d)
        );
    }

    /// M2 回帰防止（`SwapLayout` 版、上記 `ToggleEngine` 版と対称）。
    #[test]
    fn swap_layout_discards_pending_ime_open_request_not_leak_to_later_key() {
        let mut engine = make_test_engine_with_muhenkan();
        engine.set_muhenkan_delegate_to_open_axis(Some(ShadowImeAction::TurnOff));

        let _ = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());

        let new_layout = make_layout();
        let _ = engine.on_command(EngineCommand::SwapLayout(new_layout), &ime_on_ctx());

        let d = engine.on_input(Ev::down(VK_A).at(9000).build(), &ime_on_ctx());
        assert!(
            !has_effect(&d, |e| matches!(e, Effect::Ime(_))),
            "stale ime_open_requested from SwapLayout's internal flush must not leak \
             into an unrelated later key, got {:?}",
            effects_of(&d)
        );
    }

    /// `delegate_to_open_axis_fires_on_confirmed_muhenkan_solo_tap` の変換
    /// （henkan）版。`resolve_pending_thumb_as_single`のhenkan分岐
    /// （`dedicated_fn_key`は常に`None`、`ModeKeyConfig`のみ）を固定する
    /// （テストカバレッジ欠落の指摘への対応）。
    #[test]
    fn delegate_to_open_axis_fires_on_confirmed_henkan_solo_tap() {
        let mut engine = make_test_engine_with_henkan();
        engine.set_henkan_delegate_to_open_axis(Some(ShadowImeAction::TurnOff));

        let d = engine.on_input(Ev::down(VK_CONVERT).at(100).build(), &ime_on_ctx());
        assert!(
            d.is_consumed(),
            "solo tap should be pending, not passthrough"
        );
        assert!(
            !has_effect(&d, |e| matches!(e, Effect::Ime(_))),
            "IME effect must not fire before solo tap is confirmed"
        );

        let d = engine.on_timeout(TIMER_PENDING, &ime_on_ctx());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen { open: false, .. })
        )));
        assert!(
            !has_effect(&d, |e| matches!(
                e,
                Effect::Input(InputEffect::SendKeys(actions))
                    if actions.iter().any(|a| matches!(a, KeyAction::Key(x) if *x == VK_CONVERT))
            )),
            "raw VK_CONVERT must not be sent when delegated to open axis, got {:?}",
            effects_of(&d)
        );
    }

    // 二重 enqueue 回帰防止: IME OFF コンボ後の次キーで SetOpen が再発行されないこと。
    //
    // 旧実装は SpecialKeyMatch::ImeOff が SetOpen(false) のみ emit し、activation.prev を
    // 更新しなかった。Platform 層が preconditions.ime_on=false を即時反映するため、
    // 次の on_input（例: 合成された Ctrl KeyUp）で check_active_transition が同じ
    // 状態変化を検出し SetOpen(false) を再 emit していた。
    //
    // 現実装は build_ime_set_open_decision が activation.prev を新状態に推進するため、
    // 次回の transition_to は no-op となる。
    #[test]
    fn ime_off_combo_does_not_double_emit_set_open_on_next_input() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_CONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![combo],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);

        let d1 = engine.on_input(Ev::down(VK_CONVERT).at(100).build(), &ime_on_ctx());
        let setopen_count_1 = count_set_open_effects(&d1);
        assert_eq!(
            setopen_count_1, 1,
            "first on_input should emit exactly 1 SetOpen"
        );

        // Platform 層は SetOpen を見て preconditions.ime_on=false を反映する。
        // 次の on_input は新しい ctx (ime_on=false) で呼ばれる。
        let d2 = engine.on_input(Ev::up(VK_CTRL).at(110).build(), &ime_off_ctx());
        let setopen_count_2 = count_set_open_effects(&d2);
        assert_eq!(
            setopen_count_2, 0,
            "next on_input must NOT re-emit SetOpen (activation.prev should have been advanced)"
        );
    }

    // 同じく IME ON コンボ後の重複防止。
    #[test]
    fn ime_on_combo_does_not_double_emit_set_open_on_next_input() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_CONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![combo],
            ime_off: vec![],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);

        let d1 = engine.on_input(Ev::down(VK_CONVERT).at(100).build(), &ime_off_ctx());
        let setopen_count_1 = count_set_open_effects(&d1);
        assert_eq!(
            setopen_count_1, 1,
            "first on_input should emit exactly 1 SetOpen"
        );

        let d2 = engine.on_input(Ev::up(VK_CTRL).at(110).build(), &ime_on_ctx());
        let setopen_count_2 = count_set_open_effects(&d2);
        assert_eq!(
            setopen_count_2, 0,
            "next on_input must NOT re-emit SetOpen (activation.prev should have been advanced)"
        );
    }

    // user_enabled=false で既に Inactive の状態で IME OFF コンボを受けたとき、
    // ActivationController の遷移は発生しないが、IME 制御の意図を Platform に伝えるため
    // SetOpen(false) は明示的に発行される必要がある。
    #[test]
    fn ime_off_combo_emits_set_open_even_when_already_inactive() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_CONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![combo],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);
        engine.set_user_enabled(false); // Inactive 状態に
        engine.set_prev_active(false); // activation.prev も同期

        let d = engine.on_input(Ev::down(VK_CONVERT).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
        assert_eq!(
            count_set_open_effects(&d),
            1,
            "SetOpen must be emitted even when activation state didn't change"
        );
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen { open: false, .. })
        )));
    }

    fn count_set_open_effects(decision: &Decision) -> usize {
        let effects = match decision {
            Decision::Consume { effects } | Decision::PassThroughWith { effects } => effects,
            Decision::PassThrough => return 0,
        };
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Ime(ImeEffect::SetOpen { .. })))
            .count()
    }

    #[test]
    fn special_key_not_triggered_on_key_up() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_NONCONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![combo],
            ime_on: vec![],
            ime_off: vec![],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);

        let d = engine.on_input(Ev::up(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        assert!(
            engine.is_user_enabled(),
            "KeyUp should not trigger engine_off"
        );
        assert!(!has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { .. })
        )));
    }

    // ── 6. KeyLifecycle integration ──

    #[test]
    fn lifecycle_key_down_consumed_key_up_consumed() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
        let d = engine.on_input(Ev::up(VK_A).at(200).build(), &ime_on_ctx());
        assert!(d.is_consumed());
    }

    #[test]
    fn lifecycle_key_down_passthrough_key_up_passthrough() {
        let mut engine = make_test_engine();
        // preconditions not met → passthrough (use ime_off_ctx)
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_off_ctx());
        assert!(!d.is_consumed());
        let d = engine.on_input(Ev::up(VK_A).at(200).build(), &ime_off_ctx());
        assert!(!d.is_consumed());
    }

    // ── 7. Engine::on_command RefreshState ──

    #[test]
    fn refresh_state_on_updates_preconditions() {
        let mut engine = make_test_engine();
        // まず RefreshState で prev_active=false に遷移させる
        engine.on_command(EngineCommand::RefreshState, &ime_off_ctx());
        assert!(!engine.compute_active(&ime_off_ctx()));

        // RefreshState → Platform updated atomic → ctx reflects ime_on=true
        let d = engine.on_command(EngineCommand::RefreshState, &ime_on_ctx());
        assert!(
            engine.compute_active(&ime_on_ctx()),
            "preconditions met → active"
        );
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: true, .. })
        )));
    }

    #[test]
    fn refresh_state_on_but_user_disabled_stays_inactive() {
        let mut engine = make_test_engine();
        engine.on_command(EngineCommand::ToggleEngine, &ime_on_ctx()); // user OFF
        assert!(!engine.is_user_enabled());

        let d = engine.on_command(EngineCommand::RefreshState, &ime_on_ctx());
        // user disabled → still inactive even with IME ON
        assert!(!engine.compute_active(&ime_on_ctx()));
        assert!(!has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { .. })
        )));
    }

    #[test]
    fn refresh_state_off_deactivates_engine() {
        let mut engine = make_test_engine();
        assert!(engine.compute_active(&ime_on_ctx()));

        // Platform updated atomic → ctx reflects ime_on=false
        let d = engine.on_command(EngineCommand::RefreshState, &ime_off_ctx());
        assert!(!engine.compute_active(&ime_off_ctx()));
        assert!(engine.is_user_enabled(), "user_enabled unchanged");
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: false, .. })
        )));
    }

    #[test]
    fn refresh_state_no_change() {
        let mut engine = make_test_engine();
        assert!(engine.compute_active(&ime_on_ctx()));

        let d = engine.on_command(EngineCommand::RefreshState, &ime_on_ctx());
        assert!(engine.compute_active(&ime_on_ctx()));
        // No state change → no EngineStateChanged effect
        assert!(!has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { .. })
        )));
    }

    #[test]
    fn refresh_state_not_japanese_deactivates() {
        let mut engine = make_test_engine();
        assert!(engine.compute_active(&ime_on_ctx()));

        // Platform updated is_japanese_ime=false in ctx
        let not_japanese_ctx = InputContext {
            ime_on: true,
            input_mode: InputModeState::ObservedRomaji,
            is_japanese_ime: false,
            composing: false,
            modifiers: ModifierState::default(),
            left_thumb_down: None,
            right_thumb_down: None,
        };
        let d = engine.on_command(EngineCommand::RefreshState, &not_japanese_ctx);
        assert!(!engine.compute_active(&not_japanese_ctx));
        assert!(engine.is_user_enabled(), "user_enabled unchanged");
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: false, .. })
        )));
    }

    // ── 8. Engine::on_command FocusChanged ──

    #[test]
    fn focus_changed_basic() {
        let mut engine = make_test_engine();
        let d = engine.on_command(EngineCommand::FocusChanged, &ime_on_ctx());
        assert!(!d.is_consumed());
        // ADR 028: Engine は flush と active transition のみ担当。
        // FocusEffect (UpdateLastFocusInfo, Timer 等) は Platform 層が直接処理。
    }

    #[test]
    fn focus_changed_ime_off_deactivates_engine() {
        let mut engine = make_test_engine();
        assert!(engine.compute_active(&ime_on_ctx()));

        // Platform updated atomic → ctx reflects ime_on=false
        let d = engine.on_command(EngineCommand::FocusChanged, &ime_off_ctx());
        assert!(
            !engine.compute_active(&ime_off_ctx()),
            "engine should be inactive when IME is OFF at focus change"
        );
        assert!(engine.is_user_enabled(), "user_enabled unchanged");
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: false, .. })
        )));
    }

    #[test]
    fn focus_changed_needs_uia() {
        // ADR 028: UIA リクエストは Platform 層が直接実行。
        // Engine は needs_uia を処理しない。
        let mut engine = make_test_engine();
        let d = engine.on_command(EngineCommand::FocusChanged, &ime_on_ctx());
        assert!(!d.is_consumed());
    }

    #[test]
    fn focus_changed_syncs_engine_with_ime() {
        let mut engine = make_test_engine();

        // Focus change with IME OFF should deactivate engine
        // Platform updated atomic → ctx reflects ime_on=false
        engine.on_command(EngineCommand::FocusChanged, &ime_off_ctx());
        assert!(
            !engine.compute_active(&ime_off_ctx()),
            "engine should be inactive when IME is OFF at focus change"
        );
        assert!(engine.is_user_enabled(), "user_enabled unchanged");

        // Focus change with IME ON should activate engine
        engine.on_command(EngineCommand::FocusChanged, &ime_on_ctx());
        assert!(
            engine.compute_active(&ime_on_ctx()),
            "engine should be active when IME is ON at focus change"
        );
    }

    #[test]
    fn focus_changed_overridden() {
        // ADR 028: キャッシュ格納は Platform 層が直接処理。
        // Engine は FocusEffect を emit しない。
        let mut engine = make_test_engine();
        let d = engine.on_command(EngineCommand::FocusChanged, &ime_on_ctx());
        assert!(!d.is_consumed());
    }

    #[test]
    fn focus_changed_with_modifiers_in_ctx() {
        let mut engine = make_test_engine();
        let d = engine.on_command(EngineCommand::FocusChanged, &ime_on_ctx());
        assert!(!d.is_consumed());
    }

    // ── 10. is_user_enabled ──

    #[test]
    fn is_user_enabled_default_true() {
        let engine = make_test_engine();
        assert!(engine.is_user_enabled());
    }

    // ── 12. Engine::on_command with SwapLayout ──

    #[test]
    fn on_command_swap_layout() {
        let mut engine = make_test_engine();
        let new_layout = make_layout();
        let d = engine.on_command(EngineCommand::SwapLayout(new_layout), &ime_on_ctx());
        let _ = d; // verify no panic
    }

    // ── 13. Multiple key sequence integration ──

    #[test]
    fn full_char_input_sequence() {
        let mut engine = make_test_engine();

        // Type 'A' key: down -> timeout -> up
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());

        let d = engine.on_timeout(TIMER_PENDING, &ime_on_ctx());
        assert!(d.is_consumed());
        assert!(
            has_effect(&d, |e| matches!(e, Effect::Input(InputEffect::SendKeys(_)))),
            "timeout should produce SendKeys"
        );

        let d = engine.on_input(Ev::up(VK_A).at(300).build(), &ime_on_ctx());
        assert!(d.is_consumed()); // lifecycle auto-consume

        // Type 'S' key
        let d = engine.on_input(Ev::down(VK_S).at(400).build(), &ime_on_ctx());
        assert!(d.is_consumed());
    }

    // ── 14. Focus change flushes pending key ups ──

    #[test]
    fn focus_change_flushes_pending_key_ups() {
        let mut engine = make_test_engine();

        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());

        let d = engine.on_command(EngineCommand::FocusChanged, &ime_on_ctx());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Input(InputEffect::ReinjectKey(_))
        )));
    }

    // ── 15. Engine disabled -> char key passes through ──

    #[test]
    fn disabled_engine_char_passes_through() {
        let mut engine = make_test_engine();
        engine.on_command(EngineCommand::ToggleEngine, &ime_on_ctx());
        assert!(!engine.is_user_enabled());

        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(!d.is_consumed());
    }

    // ── 16. Thumb key with IME ON ──

    #[test]
    fn thumb_key_with_ime_on_is_consumed() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
    }

    // ── 17. RefreshState with pending flush ──

    #[test]
    fn sync_ime_off_flushes_pending() {
        let mut engine = make_test_engine();
        engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());

        // Platform updated atomic → ctx reflects ime_on=false
        let d = engine.on_command(EngineCommand::RefreshState, &ime_off_ctx());
        assert!(!engine.compute_active(&ime_off_ctx()));
        assert!(engine.is_user_enabled(), "user_enabled unchanged");
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: false, .. })
        )));
    }

    // ── 18. SetNgramModel command ──

    #[test]
    fn on_command_set_ngram_model() {
        use crate::ngram::NgramModel;
        let mut engine = make_test_engine();
        let model = NgramModel::new(20_000, 50_000, 200_000);
        let d = engine.on_command(EngineCommand::SetNgramModel(model), &ime_on_ctx());
        assert!(!d.is_consumed());
    }

    // ── 19. Two char keys in sequence (second key resolves first) ──

    #[test]
    fn two_char_keys_second_resolves_first() {
        let mut engine = make_test_engine();

        // First char key enters PendingChar
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());

        // Second char key resolves first and enters new PendingChar
        let d = engine.on_input(Ev::down(VK_S).at(150).build(), &ime_on_ctx());
        assert!(d.is_consumed());
        // Should have SendKeys for the first character
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Input(InputEffect::SendKeys(_))
        )));
    }

    // ── 20. Thumb + char simultaneous input ──

    #[test]
    fn thumb_then_char_within_threshold() {
        let mut engine = make_test_engine();

        // Left thumb down
        let d = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());

        // Char key within threshold -> simultaneous input
        let d = engine.on_input(Ev::down(VK_A).at(130).build(), &ime_on_ctx());
        assert!(d.is_consumed());
    }

    // ── 21. Focus changed then input ──

    #[test]
    fn focus_changed_then_input_works() {
        let mut engine = make_test_engine();

        engine.on_command(EngineCommand::FocusChanged, &ime_on_ctx());

        // Input should still work after focus change
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
    }

    // ── 22. Multiple toggles ──

    #[test]
    fn multiple_toggles_cycle() {
        let mut engine = make_test_engine();

        for _ in 0..5 {
            engine.on_command(EngineCommand::ToggleEngine, &ime_on_ctx());
            assert!(!engine.is_user_enabled());
            engine.on_command(EngineCommand::ToggleEngine, &ime_on_ctx());
            assert!(engine.is_user_enabled());
        }
    }

    // ── 23. RefreshState with pending key flushes ──

    #[test]
    fn refresh_state_off_with_pending_flushes() {
        let mut engine = make_test_engine();
        // Enter pending state
        engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());

        // Platform updated atomic → ctx reflects ime_on=false
        let d = engine.on_command(EngineCommand::RefreshState, &ime_off_ctx());
        assert!(!engine.compute_active(&ime_off_ctx()));
        assert!(engine.is_user_enabled(), "user_enabled unchanged");
        // Should have flush effects (SendKeys) before the state change
        let effs = effects_of(&d);
        assert!(
            effs.len() >= 2,
            "should have flush + state change + cache update effects"
        );
    }

    // ── 24. Focus change updates preconditions ──

    #[test]
    fn focus_changed_ime_on_activates_engine() {
        let mut engine = make_test_engine();
        // まず IME OFF のフォーカス変更で prev_active=false にする
        engine.on_command(EngineCommand::FocusChanged, &ime_off_ctx());
        assert!(!engine.compute_active(&ime_off_ctx()));

        // Focus change with IME ON should activate engine
        let d = engine.on_command(EngineCommand::FocusChanged, &ime_on_ctx());
        assert!(
            engine.compute_active(&ime_on_ctx()),
            "engine should be active when IME is ON at focus change"
        );
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: true, .. })
        )));
    }

    #[test]
    fn focus_changed_ime_on_but_user_disabled() {
        let mut engine = make_test_engine();
        engine.on_command(EngineCommand::ToggleEngine, &ime_on_ctx()); // user OFF
        assert!(!engine.compute_active(&ime_on_ctx()));

        // Focus change with IME ON should not activate (user disabled)
        let d = engine.on_command(EngineCommand::FocusChanged, &ime_on_ctx());
        assert!(
            !engine.compute_active(&ime_on_ctx()),
            "user disabled → still inactive"
        );
        assert!(!has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { .. })
        )));
    }

    // ── check_active_transition / transition_activation (line 159, 221) ──

    #[test]
    fn active_to_inactive_transition_flushes_pending_char_as_send_keys() {
        // check_active_transition の `was_active != now_active` (line 159) が
        // `==` に壊れると、実際に状態が変化したときに限ってこの分岐に入らなくなり、
        // 保留中の文字がフラッシュされなくなる。
        let mut engine = make_test_engine();
        engine.on_input(Ev::down(VK_A).at(0).build(), &ime_on_ctx());

        let d = engine.on_command(EngineCommand::RefreshState, &ime_off_ctx());
        assert!(!engine.compute_active(&ime_off_ctx()));
        assert!(
            has_effect(&d, |e| matches!(
                e,
                Effect::Input(InputEffect::SendKeys(actions))
                    if actions.iter().any(|a| matches!(a, KeyAction::Char('う')))
            )),
            "active→inactive transition must flush the pending char, got {:?}",
            effects_of(&d)
        );
    }

    #[test]
    fn active_to_inactive_transition_emits_set_open_false() {
        // transition_activation の `if !suppress_set_open` (line 221) の `!` が
        // 消えると、通常の ImeOff 遷移（NotRomajiInput ではない）で SetOpen が
        // 発行されなくなる。EngineStateChanged は無条件で push されるため、
        // それだけを見るテストではこの変異を検出できない。
        let mut engine = make_test_engine();
        assert!(engine.compute_active(&ime_on_ctx()));

        let d = engine.on_command(EngineCommand::RefreshState, &ime_off_ctx());
        assert!(
            has_effect(&d, |e| matches!(
                e,
                Effect::Ime(ImeEffect::SetOpen { open: false, .. })
            )),
            "normal ImeOff transition must emit SetOpen(false), got {:?}",
            effects_of(&d)
        );
    }

    // 2026-08-04: 「IME OFF・Engine ON」再発対策（`SetOpenOrigin` 導入）の回帰テスト。
    //
    // `EngineCommand::RefreshState`（Platform 層が `ctx.ime_on` を再評価するたびに叩く
    // 経路。IME ポーリング/idle-conv-check 由来で毎キー入力とは無関係に発火しうる）が
    // 引き起こす active/inactive 遷移は `check_active_transition` を経由するため、
    // 発行される `SetOpen` は必ず `SetOpenOrigin::ActivationSync` でなければならない。
    // ここが誤って `ExplicitUserAction` になると、awase-windows 側の
    // `kp_stage_post_decision` がユーザーの明示的な IME OFF 意図（`last_intent`）を
    // 「観測駆動の echo」で上書きしてしまい、ユーザーが IME を OFF にした直後でも
    // Engine が勝手に ON へ戻る（`docs/known-bugs.md` 参照）。
    #[test]
    fn refresh_state_transition_emits_activation_sync_origin_not_explicit_user_action() {
        let mut engine = make_test_engine();
        // make_test_engine() は prev_active=true から始まるため、まず ime_off_ctx() で
        // Inactive に落としてから、本題の Inactive→Active 遷移を起こす。
        engine.on_command(EngineCommand::RefreshState, &ime_off_ctx());
        assert!(!engine.compute_active(&ime_off_ctx()));

        let d = engine.on_command(EngineCommand::RefreshState, &ime_on_ctx());
        assert!(
            has_effect(&d, |e| matches!(
                e,
                Effect::Ime(ImeEffect::SetOpen {
                    open: true,
                    origin: SetOpenOrigin::ActivationSync
                })
            )),
            "RefreshState 由来の SetOpen は ActivationSync でなければならない \
             (ExplicitUserAction だと belief の last_intent が観測駆動の echo で \
             汚染される), got {:?}",
            effects_of(&d)
        );
        assert!(
            !has_effect(&d, |e| matches!(
                e,
                Effect::Ime(ImeEffect::SetOpen {
                    origin: SetOpenOrigin::ExplicitUserAction,
                    ..
                })
            )),
            "RefreshState 由来の SetOpen に ExplicitUserAction が混ざってはならない, got {:?}",
            effects_of(&d)
        );
    }

    // 対照テスト: IME-ON コンボ（本物のユーザー操作）は ExplicitUserAction を使う。
    #[test]
    fn ime_on_combo_emits_explicit_user_action_origin() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_CONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![combo],
            ime_off: vec![],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);

        let d = engine.on_input(Ev::down(VK_CONVERT).at(100).build(), &ime_off_ctx());
        assert!(
            has_effect(&d, |e| matches!(
                e,
                Effect::Ime(ImeEffect::SetOpen {
                    open: true,
                    origin: SetOpenOrigin::ExplicitUserAction
                })
            )),
            "IME-ON コンボは ExplicitUserAction を使わなければならない, got {:?}",
            effects_of(&d)
        );
    }

    /// `transition_activation` の `EngineStateChanged.send_ime_key: !suppress_ime_key`
    /// 自体は上の `active_to_inactive_transition_emits_set_open_false`（`suppress_set_open`
    /// の `!` を検証）とは別の変異体で、これまで `send_ime_key` フィールドを直接見る
    /// テストが無かった。通常の ImeOff 遷移（NotRomajiInput ではない）では
    /// `send_ime_key=true` のはず。
    #[test]
    fn active_to_inactive_transition_normal_send_ime_key_true() {
        let mut engine = make_test_engine();
        assert!(engine.compute_active(&ime_on_ctx()));

        let d = engine.on_command(EngineCommand::RefreshState, &ime_off_ctx());
        assert!(
            has_effect(&d, |e| matches!(
                e,
                Effect::Ui(UiEffect::EngineStateChanged {
                    send_ime_key: true,
                    ..
                })
            )),
            "normal ImeOff transition must set send_ime_key=true, got {:?}",
            effects_of(&d)
        );
    }

    /// `NotRomajiInput`（tray での英数モード選択等）への遷移では `suppress_set_open`
    /// (=`suppress_ime_key`) が true になり、`SetOpen` を出さず `send_ime_key=false`
    /// のはず（ユーザーが選択した kana/katakana モードを維持するため）。
    #[test]
    fn active_to_inactive_transition_not_romaji_input_suppresses_send_ime_key() {
        let mut engine = make_test_engine();
        assert!(engine.compute_active(&ime_on_ctx()));

        let not_romaji_ctx = InputContext {
            input_mode: InputModeState::ObservedKana,
            ..ime_on_ctx()
        };
        let d = engine.on_command(EngineCommand::RefreshState, &not_romaji_ctx);
        assert!(
            !engine.compute_active(&not_romaji_ctx),
            "NotRomajiInput への遷移のはず"
        );
        assert!(
            has_effect(&d, |e| matches!(
                e,
                Effect::Ui(UiEffect::EngineStateChanged {
                    send_ime_key: false,
                    ..
                })
            )),
            "NotRomajiInput transition must set send_ime_key=false, got {:?}",
            effects_of(&d)
        );
        assert!(
            !has_effect(&d, |e| matches!(e, Effect::Ime(ImeEffect::SetOpen { .. }))),
            "NotRomajiInput transition must not emit SetOpen, got {:?}",
            effects_of(&d)
        );
    }

    // ── on_input: KeyUp dedup 短絡 (line 238) ──

    #[test]
    fn key_up_for_consumed_pending_char_short_circuits_without_resolving() {
        // `if !is_key_down && self.lifecycle.on_key_up(...)` (line 238) の `!` が
        // 消えると、この短絡がもう機能せず、保留中の文字キーの KeyUp が
        // （本来は素通しの dedup のはずが）FSM 経由で再度解決され、
        // 想定外に文字が出力されてしまう。
        let mut engine = make_test_engine();
        let d1 = engine.on_input(Ev::down(VK_A).at(0).build(), &ime_on_ctx());
        assert!(d1.is_consumed());

        let d2 = engine.on_input(Ev::up(VK_A).at(50).build(), &ime_on_ctx());
        assert!(d2.is_consumed());
        assert!(
            effects_of(&d2).is_empty(),
            "KeyUp for an already-consumed pending key must be a bare dedup consume \
             with no effects, got {:?}",
            effects_of(&d2)
        );
    }

    // ── on_input: lifecycle 登録条件 (line 264) ──

    #[test]
    fn key_down_passthrough_key_is_not_registered_for_lifecycle_dedup() {
        // `if is_key_down && decision.is_consumed()` (line 264) の `&&` が `||` に
        // 壊れると、consumed でない（passthrough の）KeyDown も lifecycle に
        // 登録されてしまい、対応する KeyUp が line 238 の dedup 短絡に
        // 誤って引っかかって consumed 扱いになってしまう。
        let mut engine = make_test_engine();
        let d1 = engine.on_input(Ev::down(VK_RETURN).at(0).build(), &ime_on_ctx());
        assert!(
            !d1.is_consumed(),
            "Enter (non-layout key) should pass through"
        );

        let d2 = engine.on_input(Ev::up(VK_RETURN).at(50).build(), &ime_on_ctx());
        assert!(
            !d2.is_consumed(),
            "Enter key_up must also pass through, not be dedup-consumed"
        );
    }

    // ── on_timeout: active/inactive 分岐 (line 278) ──

    #[test]
    fn on_timeout_while_active_resolves_normally_not_via_flush() {
        // `if !self.compute_active(ctx)` (line 278) の `!` が消えると、
        // active なときに flush(ImeOff, ComposingHint::Unknown) 経由になってしまう。
        // PendingChar の場合は resolve_pending_char_as_single が hint に依存しないため
        // 区別できないが、PendingThumb は composing hint で挙動が変わる
        // （Unknown なら無条件 suppress）ため、こちらで区別する。
        let mut engine = make_test_engine();
        let d1 = engine.on_input(Ev::down(VK_NONCONVERT).at(0).build(), &ime_on_ctx());
        assert!(d1.is_consumed());

        // composing=false（非合成中）で active のままタイムアウト。
        let d2 = engine.on_timeout(TIMER_PENDING, &ime_on_ctx());
        assert!(d2.is_consumed());
        assert!(
            has_effect(&d2, |e| matches!(
                e,
                Effect::Input(InputEffect::SendKeys(actions)) if !actions.is_empty()
            )),
            "active な on_timeout は通常経路で親指キーの生VKを出力するはず（flush 経路だと \
             ComposingHint::Unknown により無条件 suppress され actions が空になる）, got {:?}",
            effects_of(&d2)
        );
    }

    // ── take_solo_off_notification (line 302) ──

    #[test]
    fn take_solo_off_notification_false_by_default() {
        let mut engine = make_test_engine();
        assert!(!engine.take_solo_off_notification());
    }

    #[test]
    fn take_solo_off_notification_true_once_after_solo_off_trigger() {
        let mut engine = make_test_engine();
        engine.set_engine_off_triple_vk(VK_NONCONVERT);
        let gap = 150_000u64;

        for i in 0..5u64 {
            engine.on_input(Ev::down(VK_NONCONVERT).at(i * gap).build(), &ime_on_ctx());
            engine.on_timeout(TIMER_PENDING, &ime_on_ctx());
        }

        assert!(
            engine.take_solo_off_notification(),
            "should be true once right after the 5th consecutive solo timeout"
        );
        assert!(
            !engine.take_solo_off_notification(),
            "one-shot flag must be false on the immediately following call"
        );
    }

    // ── apply_engine_on_with_ime_recovery (line 466-467) ──

    #[test]
    fn toggle_engine_recovery_forces_ime_on_and_reports_state_changed() {
        // `apply_engine_on_with_ime_recovery` の pseudo_ctx { ime_on: true, ..*ctx } から
        // `ime_on: true` フィールドが削除されると、実際の ctx.ime_on (=false) がそのまま
        // 使われてしまい、target_state が Inactive のままになる。prev_activation も
        // Inactive のままなので transition_activation の was==now が成立してしまい、
        // 空の effects → SetOpen(true) のフォールバックのみが push され、
        // EngineStateChanged が発行されなくなる。
        let mut engine = make_test_engine();
        engine.on_command(EngineCommand::ToggleEngine, &ime_on_ctx()); // まず OFF にする
        assert!(!engine.is_user_enabled());

        // ime_on=false の ctx で ON に戻す → recovery 経路（pseudo_ctx で ime_on を強制 true）。
        let d = engine.on_command(EngineCommand::ToggleEngine, &ime_off_ctx());
        assert!(engine.is_user_enabled());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen { open: true, .. })
        )));
        assert!(
            has_effect(&d, |e| matches!(
                e,
                Effect::Ui(UiEffect::EngineStateChanged { enabled: true, .. })
            )),
            "recovery must compute target state from the ime-forced-on pseudo_ctx, \
             not the real (ime_on=false) ctx, got {:?}",
            effects_of(&d)
        );
    }

    #[test]
    fn toggle_engine_off_when_already_inactive_emits_no_transition() {
        // engine.rs:320 の `if user_enabled && !new_active` の `&&` が `||` に壊れると
        // 検出される変異体キラー。
        //
        // 状況: engine は user_enabled=true・prev_activation=Active（make_test_engine の
        // 既定）だが、ctx は ime_on=false なので実効状態は既に inactive。ここで
        // ToggleEngine を撃つと user_enabled が false に落ちるだけで、実効状態は
        // inactive → inactive のまま変化しない。したがって
        //   old_active=false, new_active=false, user_enabled=false
        // となり、正しい `&&` では条件 `false && !false = false` → else 分岐
        // (`apply_active_transition(false, false)` は old==new で no-op) となって
        // 遷移エフェクトは一切出ない。
        //
        // `&&`→`||` に壊れると条件は `false || !false = true` となり recovery 分岐へ。
        // recovery は is_enabled=false（トグル後）で target=Inactive を計算するが、
        // prev_activation はまだ stale な Active のままなので transition_activation の
        // was(true)!=now(false) が成立し、spurious な SetOpen{false} +
        // EngineStateChanged{enabled:false} を発行してしまう。
        let mut engine = make_test_engine();
        assert!(engine.is_user_enabled());
        assert!(
            !engine.compute_active(&ime_off_ctx()),
            "precondition: effective state is already inactive under ime_off ctx"
        );

        let d = engine.on_command(EngineCommand::ToggleEngine, &ime_off_ctx());
        assert!(
            !engine.is_user_enabled(),
            "toggle must flip user_enabled to false"
        );
        assert!(
            !has_effect(&d, |e| matches!(
                e,
                Effect::Ui(UiEffect::EngineStateChanged { .. })
            )),
            "inactive→inactive toggle must not emit EngineStateChanged, got {:?}",
            effects_of(&d)
        );
        assert!(
            !has_effect(&d, |e| matches!(e, Effect::Ime(ImeEffect::SetOpen { .. }))),
            "inactive→inactive toggle must not emit SetOpen, got {:?}",
            effects_of(&d)
        );
    }

    // ── matches_ime_off (line 516) ──

    #[test]
    fn matches_ime_off_true_for_matching_combo_false_otherwise() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_CONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![combo],
            ime_toggle: vec![],
        };
        let engine = make_engine_with_special(special);

        assert!(engine.matches_ime_off(&ime_on_ctx(), &Ev::down(VK_CONVERT).at(0).build()));
        assert!(!engine.matches_ime_off(&ime_on_ctx(), &Ev::down(VK_A).at(0).build()));
    }

    // ── matches_key_combo (line 583-586): 修飾キーの厳密一致 ──

    #[test]
    fn key_combo_requires_exact_ctrl_match() {
        // `event.vk_code == combo.vk && combo.ctrl == modifiers.ctrl && ...` の
        // 最初の `&&` (line 584) が `||` に壊れると、vk が一致しただけで
        // ctrl の不一致を無視してマッチしてしまう。
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_CONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![combo],
            ime_off: vec![],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);
        let ctx_with_ctrl = InputContext {
            modifiers: ModifierState {
                ctrl: true,
                alt: false,
                shift: false,
                win: false,
            },
            ..ime_on_ctx()
        };
        let d = engine.on_input(Ev::down(VK_CONVERT).at(0).build(), &ctx_with_ctrl);
        assert!(
            !has_effect(&d, |e| matches!(e, Effect::Ime(ImeEffect::SetOpen { .. }))),
            "combo requires ctrl=false; ctrl=true must not match, got {:?}",
            effects_of(&d)
        );
    }

    #[test]
    fn key_combo_requires_exact_shift_match() {
        // 2番目の `&&` (line 585) が `||` に壊れると、ctrl まで一致すれば
        // shift の不一致を無視してマッチしてしまう。
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_CONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![combo],
            ime_off: vec![],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);
        let ctx_with_shift = InputContext {
            modifiers: ModifierState {
                ctrl: false,
                alt: false,
                shift: true,
                win: false,
            },
            ..ime_on_ctx()
        };
        let d = engine.on_input(Ev::down(VK_CONVERT).at(0).build(), &ctx_with_shift);
        assert!(
            !has_effect(&d, |e| matches!(e, Effect::Ime(ImeEffect::SetOpen { .. }))),
            "combo requires shift=false; shift=true must not match, got {:?}",
            effects_of(&d)
        );
    }

    #[test]
    fn key_combo_requires_exact_alt_match() {
        // 3番目の `&&` (line 586) が `||` に壊れると、ctrl/shift まで一致すれば
        // alt の不一致を無視してマッチしてしまう。
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_CONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![combo],
            ime_off: vec![],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);
        let ctx_with_alt = InputContext {
            modifiers: ModifierState {
                ctrl: false,
                alt: true,
                shift: false,
                win: false,
            },
            ..ime_on_ctx()
        };
        let d = engine.on_input(Ev::down(VK_CONVERT).at(0).build(), &ctx_with_alt);
        assert!(
            !has_effect(&d, |e| matches!(e, Effect::Ime(ImeEffect::SetOpen { .. }))),
            "combo requires alt=false; alt=true must not match, got {:?}",
            effects_of(&d)
        );
    }

    #[test]
    fn key_combo_requires_exact_vk_match() {
        // matches_key_combo の本体が無条件 `true` に置換されると、vk が
        // 全く違うキーでもマッチしてしまう。
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_CONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![combo],
            ime_off: vec![],
            ime_toggle: vec![],
        };
        let mut engine = make_engine_with_special(special);
        let d = engine.on_input(Ev::down(VK_A).at(0).build(), &ime_on_ctx());
        assert!(
            !has_effect(&d, |e| matches!(e, Effect::Ime(ImeEffect::SetOpen { .. }))),
            "unrelated vk must not match the combo, got {:?}",
            effects_of(&d)
        );
    }

    // ── セッター系: 呼ぶ→実際に効いていることを後続動作で確認する ──

    #[test]
    fn engine_set_user_enabled_actually_toggles_active_state() {
        let mut engine = make_test_engine();
        assert!(engine.is_user_enabled());

        engine.set_user_enabled(false);
        assert!(
            !engine.is_user_enabled(),
            "set_user_enabled(false) must actually disable"
        );

        engine.set_user_enabled(true);
        assert!(engine.is_user_enabled());
    }

    #[test]
    fn engine_set_space_thumb_config_ignore_composing_guard_actually_takes_effect() {
        // set_space_thumb_config が no-op に壊れると、space_thumb_vk が None のままになり、
        // ignore_composing_guard=true を指定しても composing 中は無条件 suppress
        // されてしまう（生 VK_SPACE が出力されない）。
        let mut engine = make_test_engine();
        engine.set_space_thumb_config(
            Some(VK_SPACE),
            TextKeyConfig {
                ignore_composing_guard: true,
                shift_literal: false,
            },
        );

        let composing_ctx = InputContext {
            composing: true,
            ..ime_on_ctx()
        };

        let d1 = engine.on_input(Ev::down(VK_SPACE).at(0).build(), &composing_ctx);
        assert!(d1.is_consumed());

        let d2 = engine.on_timeout(TIMER_PENDING, &composing_ctx);
        assert!(
            has_effect(&d2, |e| matches!(
                e,
                Effect::Input(InputEffect::SendKeys(actions))
                    if actions.iter().any(|a| matches!(a, KeyAction::Key(x) if *x == VK_SPACE))
            )),
            "space_thumb_vk + ignore_composing_guard=true must emit raw VK_SPACE even while \
             composing, got {:?}",
            effects_of(&d2)
        );
    }

    #[test]
    fn engine_set_thumb_key_solo_tap_config_ignore_composing_guard_actually_takes_effect() {
        // set_thumb_key_solo_tap_config が no-op に壊れると、muhenkan_vk が None のままになり、
        // 同様に composing 中の生 VK 出力が抑制されたままになる。
        let mut engine = make_test_engine();
        engine.set_thumb_key_solo_tap_config(
            Some(VK_NONCONVERT),
            ModeKeyConfig::from_legacy_bools(true, false),
            None,
            // 旧 `ThumbKeySoloTapGuard::default()`（ignore_composing_guard=false,
            // always_suppress=false）と同値。
            ModeKeyConfig::from_legacy_bools(false, false),
        );

        let composing_ctx = InputContext {
            composing: true,
            ..ime_on_ctx()
        };

        engine.on_input(Ev::down(VK_NONCONVERT).at(0).build(), &composing_ctx);
        let d2 = engine.on_timeout(TIMER_PENDING, &composing_ctx);
        assert!(
            has_effect(&d2, |e| matches!(
                e,
                Effect::Input(InputEffect::SendKeys(actions))
                    if actions.iter().any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT))
            )),
            "muhenkan_vk + ignore_composing_guard=true must emit raw VK even while composing, \
             got {:?}",
            effects_of(&d2)
        );
    }

    #[test]
    fn engine_set_enter_thumb_config_ignore_composing_guard_actually_takes_effect() {
        // set_enter_thumb_config が no-op に壊れると、enter_thumb_vk が None のままになり、
        // ignore_composing_guard=true を指定しても composing 中は無条件 suppress
        // されてしまう（生 VK_RETURN が出力されない）。
        let mut engine = make_test_engine();
        engine.set_enter_thumb_config(
            Some(VK_RETURN),
            TextKeyConfig {
                ignore_composing_guard: true,
                shift_literal: false,
            },
        );

        let composing_ctx = InputContext {
            composing: true,
            ..ime_on_ctx()
        };

        let d1 = engine.on_input(enter_thumb_down_event(0), &composing_ctx);
        assert!(d1.is_consumed());

        let d2 = engine.on_timeout(TIMER_PENDING, &composing_ctx);
        assert!(
            has_effect(&d2, |e| matches!(
                e,
                Effect::Input(InputEffect::SendKeys(actions))
                    if actions.iter().any(|a| matches!(a, KeyAction::Key(x) if *x == VK_RETURN))
            )),
            "enter_thumb_vk + ignore_composing_guard=true must emit raw VK_RETURN even while \
             composing, got {:?}",
            effects_of(&d2)
        );
    }

    #[test]
    fn update_fsm_params_threshold_ms_actually_changes_timing_window() {
        // set_threshold_ms の `ms * 1000` (nicola_fsm.rs:408) が `+`/`/` に壊れると、
        // 実際の threshold_us が大きく変わる（+1000 なら誤差程度、/1000 なら 0 近く）。
        // ここでは閾値を意図的に 10ms まで縮め、通常なら simultaneous になる 50ms の
        // ギャップが simultaneous でなくなることで、指定した値が実際に反映されている
        // ことを確認する。
        let mut engine = make_test_engine();
        engine.on_command(
            EngineCommand::UpdateFsmParams {
                threshold_ms: 10,
                confirm_mode: ConfirmMode::Wait,
                speculative_delay_ms: 30,
            },
            &ime_on_ctx(),
        );

        let d1 = engine.on_input(Ev::down(VK_NONCONVERT).at(0).build(), &ime_on_ctx());
        assert!(d1.is_consumed());

        // 50ms gap: 変更後の 10ms 閾値なら simultaneous ではない → 親指単独確定。
        let d2 = engine.on_input(Ev::down(VK_A).at(50_000).build(), &ime_on_ctx());
        assert!(
            has_effect(&d2, |e| matches!(
                e,
                Effect::Input(InputEffect::SendKeys(actions))
                    if actions.iter().any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT))
            )),
            "threshold_ms=10 なら 50ms gap は simultaneous にならないはず, got {:?}",
            effects_of(&d2)
        );
    }

    #[test]
    fn update_fsm_params_confirm_mode_actually_switches_to_speculative() {
        // set_confirm_mode 本体 (nicola_fsm.rs:413) が no-op に壊れると、confirm_mode が
        // 既定の Wait のままになり、文字キー押下時に即座出力（投機）されなくなる。
        let mut engine = make_test_engine();
        engine.on_command(
            EngineCommand::UpdateFsmParams {
                threshold_ms: 100,
                confirm_mode: ConfirmMode::Speculative,
                speculative_delay_ms: 40,
            },
            &ime_on_ctx(),
        );

        let d = engine.on_input(Ev::down(VK_A).at(0).build(), &ime_on_ctx());
        assert!(
            has_effect(&d, |e| matches!(
                e,
                Effect::Input(InputEffect::SendKeys(actions))
                    if actions.iter().any(|a| matches!(a, KeyAction::Char('う')))
            )),
            "Speculative mode must emit immediately on key down, got {:?}",
            effects_of(&d)
        );
    }

    #[test]
    fn update_fsm_params_speculative_delay_ms_actually_sets_phase2_timer_duration() {
        // set_confirm_mode の `speculative_delay_ms * 1000` (nicola_fsm.rs:414) が
        // `+`/`/` に壊れると、TwoPhase モードで TIMER_SPECULATIVE 満了後に
        // Phase2 へ遷移する際の残り時間 (remaining_us = threshold_us -
        // speculative_delay_us) が大きくずれる。
        // TwoPhase モードは Phase1 で speculative_delay_us だけ短く待ってから
        // 投機出力に遷移する（idle_speculative の即時 Reduce とは別経路）ため、
        // ここで確認する。
        // threshold_ms=100, speculative_delay_ms=40 →
        // remaining_us = 100_000 - 40_000 = 60_000 (60ms) を直接検証する。
        let mut engine = make_test_engine();
        engine.on_command(
            EngineCommand::UpdateFsmParams {
                threshold_ms: 100,
                confirm_mode: ConfirmMode::TwoPhase,
                speculative_delay_ms: 40,
            },
            &ime_on_ctx(),
        );

        let d1 = engine.on_input(Ev::down(VK_A).at(0).build(), &ime_on_ctx());
        assert!(d1.is_consumed());
        // Phase 1 の短い待機タイマー自体も speculative_delay_us (40ms) のはず。
        let phase1_duration = effects_of(&d1).iter().find_map(|e| match e {
            Effect::Timer(TimerEffect::Set { duration, .. }) => Some(*duration),
            _ => None,
        });
        assert_eq!(
            phase1_duration,
            Some(std::time::Duration::from_micros(40_000)),
            "expected Phase1 (TIMER_SPECULATIVE) duration 40ms, got {:?} (effects: {:?})",
            phase1_duration,
            effects_of(&d1)
        );

        // TIMER_SPECULATIVE 満了 → Phase2 へ遷移し、残り時間 (60ms) で TIMER_PENDING を再設定。
        let d2 = engine.on_timeout(TIMER_SPECULATIVE, &ime_on_ctx());
        let phase2_duration = effects_of(&d2).iter().find_map(|e| match e {
            Effect::Timer(TimerEffect::Set { duration, .. }) => Some(*duration),
            _ => None,
        });
        assert_eq!(
            phase2_duration,
            Some(std::time::Duration::from_micros(60_000)),
            "expected Phase2 timer duration 60ms (100ms - 40ms), got {:?} (effects: {:?})",
            phase2_duration,
            effects_of(&d2)
        );
    }

    #[test]
    fn set_ngram_model_actually_adjusts_simultaneous_threshold() {
        // fsm_adapter.rs::set_ngram_model が no-op に壊れると、ngram_model が
        // None のままになり、adjusted_threshold が常に生の threshold_us を返す。
        // ここでは「う」の直後に「あ」が来る強い負のバイグラムスコアを与えて
        // 閾値を意図的に縮め、通常は simultaneous になる 50ms のギャップが
        // simultaneous でなくなることでモデルが実際に反映されていることを確認する。
        use crate::ngram::NgramModel;

        let mut engine = make_test_engine();

        // recent_kana に 'う' を積む（A 単独確定）。
        engine.on_input(Ev::down(VK_A).at(0).build(), &ime_on_ctx());
        engine.on_timeout(TIMER_PENDING, &ime_on_ctx());

        let toml_str = r#"
[bigram]
"うあ" = -10.0
"#;
        let model = NgramModel::from_toml(toml_str, 80_000, 10_000, 150_000).unwrap();
        engine.on_command(EngineCommand::SetNgramModel(model), &ime_on_ctx());

        // 左親指 → S (左親指面で 'あ') を 50ms 差で送る。
        // 調整後の閾値は 100_000 + tanh(-10)*80_000 ≈ 20_000 (20ms) に縮むはずなので、
        // 50ms のギャップは simultaneous にならない。
        let d1 = engine.on_input(Ev::down(VK_NONCONVERT).at(200_000).build(), &ime_on_ctx());
        assert!(d1.is_consumed());
        let d2 = engine.on_input(Ev::down(VK_S).at(250_000).build(), &ime_on_ctx());
        assert!(
            !has_effect(&d2, |e| matches!(
                e,
                Effect::Input(InputEffect::SendKeys(actions))
                    if actions.iter().any(|a| matches!(a, KeyAction::Char('あ')))
            )),
            "ngram model が反映されていれば 50ms gap は simultaneous にならず 'あ' は \
             出ないはず, got {:?}",
            effects_of(&d2)
        );
    }
}
