//! BUG-116（Shift+物理かなキーでカタカナ変換に切り替わらない）診断スパイク限定モジュール。
//!
//! **develop へマージしない。** 目的は
//! `docs/adr/137-shift-katakana-dbe-mode-key-suppression-regression.md` の前提
//! （B-1: Shift の押下有無が本当に弁別軸になるか／B-2: `reinject()` の
//! `wScan: 0` が MS-IME/TSF にモードキーとして受理されるか）を実機で検証する
//! ための一時的な計装。結論が出たらこのモジュールごと破棄する。
//!
//! 環境変数2本で「Allow スコープ」と「scan スコープ」を直交して切り替える
//! （起動時に1回だけ読み、以後は `OnceLock` で固定——モード変更には再起動が
//! 必要）。`Off`/`Zero` が既定で、環境変数を設定しない限り本番と同じ挙動。
//!
//! # 安全上の注意（BUG-15 追補7 / BUG-61 / BUG-62）
//!
//! scan 付きの `VK_DBE_KATAKANA` 注入は、実 IME が OFF の瞬間に着弾すると
//! kbd106 のかな入力ロックをトグルする既知ハザードを持つ（BUG-15 追補7）。
//! JIS かな直接入力へ落ちると Win32 に外部から入力方式を戻す公式 API が
//! 存在せず解決不能（BUG-61）。さらに復旧に使う Alt+かな（物理的には
//! `VK_DBE_ROMAN`/`VK_DBE_NOROMAN`）は awase 自身が既定で常時 swallow する
//! ため（BUG-62、`hook.rs` 参照）、**固着したら awase を Exit してから
//! Alt+かなで復旧する**（awase 稼働中は復旧操作自体が届かない）。
//!
//! この既知ハザードを実機で踏んだことを検出したら（`observer::kana_lock`
//! が On を報告したら）、`abort_scan` でセッション中 scan 注入を恒久的に
//! 停止する。`record_scan_gate`/`scan_allowed_for` によるゲートは「安全側に
//! 倒す近似」であり保証ではない——`effective_open()` は belief であり実 IME
//! の状態と乖離しうる。

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::OnceLock;

/// `plan()` で `VK_DBE_KATAKANA` KeyDown を Allow する条件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AllowScope {
    /// 既定。本番と同じ（常に Suppress 側の判定に委ねる）。
    Off,
    /// Shift 併用時のみ Allow（ADR-137 の素案）。
    Shift,
    /// Shift の有無を問わず常に Allow（BUG-52 の直接原因を実証する目的、危険）。
    Always,
}

/// `reinject_key` で送出する `wScan` の扱い。`VK_DBE_KATAKANA` にのみ適用する
/// （0xF0/0xF3/0xF4 は BUG-15 追補7 の別ハザード対象のため対象外のまま、
/// SB-1(1)）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanScope {
    /// 既定。`wScan: 0`（現行の `reinject()` と同じ）。
    Zero,
    /// フックが受け取った実際の `event.scan_code` を付与する
    /// （`MapVirtualKeyW` の逆引きはレイアウト次第で 0 を返しうるため使わない、SM-1）。
    Real,
}

/// 実機検証で発見した副問題（GJI/TsfNativeでは物理`VK_DBE_HIRAGANA`
/// KeyDownがTSF warmup所有ロジックにより常時Suppressされるため、
/// Shift+かなでカタカナに入った後、物理かなキー(Shiftなし)では
/// ひらがなへ戻せない）への対処候補。`kp_bug116_spike_log`が
/// `send_gji_half_width_alnum_toggle(Exit, ..)`（BUG-25/ADR-107で
/// 既に実機検証済みの、scan付きVK_DBE_HIRAGANA注入+Win/Alt修飾キー
/// ガード+effective_open()ガード付きの安全な注入経路）を再利用して
/// 能動的にひらがな復元を送る条件を切り替える。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HiraganaReturnScope {
    /// 既定。復元注入を一切行わない（現状のまま、ひらがなに戻せない）。
    Off,
    /// `effective_open() && !shadow_toggled`（IMEが既にON、かつこの打鍵で
    /// ON/OFFが切り替わったわけではない）なら復元注入する。cold-start中の
    /// 誤発火リスクは未検証。
    WhenOpen,
    /// `WhenOpen`の条件に加え`is_composition_warm()`も要求する（cold-start
    /// warmup進行中への誤発火をより避ける、より保守的な条件）。
    WhenWarm,
}

static ALLOW: OnceLock<AllowScope> = OnceLock::new();
static SCAN: OnceLock<ScanScope> = OnceLock::new();
static HIRAGANA_RETURN: OnceLock<HiraganaReturnScope> = OnceLock::new();

pub(crate) fn hiragana_return_scope() -> HiraganaReturnScope {
    *HIRAGANA_RETURN.get_or_init(|| {
        match std::env::var("AWASE_BUG116_HIRAGANA_RETURN").as_deref() {
            Ok("open") => HiraganaReturnScope::WhenOpen,
            Ok("warm") => HiraganaReturnScope::WhenWarm,
            _ => HiraganaReturnScope::Off,
        }
    })
}

/// SB-1: かなロック検出でセッション中の scan 注入を恒久停止する（一度立てたら戻さない）。
static SCAN_ABORTED: AtomicBool = AtomicBool::new(false);

pub(crate) fn allow_scope() -> AllowScope {
    *ALLOW.get_or_init(|| match std::env::var("AWASE_BUG116_ALLOW").as_deref() {
        Ok("shift") => AllowScope::Shift,
        Ok("always") => AllowScope::Always,
        _ => AllowScope::Off,
    })
}

pub(crate) fn scan_scope() -> ScanScope {
    *SCAN.get_or_init(|| match std::env::var("AWASE_BUG116_SCAN").as_deref() {
        Ok("real") => ScanScope::Real,
        _ => ScanScope::Zero,
    })
}

/// SB-1(3): かなロック検出時、以後のセッション全体で scan 注入を止める。
/// 一度だけ `log::warn!` する（毎キー出すとログが溢れ、出さないと事後に
/// 「なぜ途中から scan が付かなくなったか」が分からなくなる、premortem 指摘）。
pub(crate) fn abort_scan(reason: &str) {
    if !SCAN_ABORTED.swap(true, Ordering::Relaxed) {
        log::warn!("[bug116] ABORT: scan 注入をセッション中恒久停止 — {reason}");
    }
}

fn scan_aborted() -> bool {
    SCAN_ABORTED.load(Ordering::Relaxed)
}

/// `key_pipeline.rs` 側（`self.platform_state.ime`/`observer::kana_lock` に
/// フルアクセスできる箇所）で計算した「scan を付けてよいか」を、`reinject_key`
/// 側（`WindowsPlatform` の一メソッドで `platform_state` に直接触れない）に
/// 受け渡すための単発ラッチ。
///
/// 対応する `(vk, is_keyup)` が一致する場合のみ有効とみなす近似——defer 経由で
/// 実送出が後のティックにずれても、無関係な別イベントの結果を誤って使うことは
/// ない。ただし ime 状態自体は判定時点からのスナップショットであり、defer 窓で
/// 状態が変わる残存リスクはある（B-4/SB-1 と同種の「保証ではなく安全側の近似」）。
static GATE_VK: AtomicU16 = AtomicU16::new(0);
static GATE_IS_KEYUP: AtomicBool = AtomicBool::new(false);
static GATE_OK: AtomicBool = AtomicBool::new(false);

pub(crate) fn record_scan_gate(vk: u16, is_keyup: bool, ok: bool) {
    GATE_VK.store(vk, Ordering::Relaxed);
    GATE_IS_KEYUP.store(is_keyup, Ordering::Relaxed);
    GATE_OK.store(ok, Ordering::Relaxed);
}

/// scan 注入が許可されているか（かなロック abort ラッチ・記録済みゲートの両方を見る）。
pub(crate) fn scan_allowed_for(vk: u16, is_keyup: bool) -> bool {
    !scan_aborted()
        && GATE_VK.load(Ordering::Relaxed) == vk
        && GATE_IS_KEYUP.load(Ordering::Relaxed) == is_keyup
        && GATE_OK.load(Ordering::Relaxed)
}

/// 起動時に1回、現在のモードをログへ残す（`app/bootstrap.rs` から呼ぶ）。
pub(crate) fn log_mode_on_startup() {
    let allow = allow_scope();
    let scan = scan_scope();
    let hiragana_return = hiragana_return_scope();
    if allow != AllowScope::Off
        || scan != ScanScope::Zero
        || hiragana_return != HiraganaReturnScope::Off
    {
        log::warn!(
            "[bug116] spike mode: allow={allow:?} scan={scan:?} hiragana_return={hiragana_return:?} \
             （develop 非マージの診断ビルド。BUG-15追補7/BUG-61/BUG-62のハザードに注意）"
        );
    }
}
