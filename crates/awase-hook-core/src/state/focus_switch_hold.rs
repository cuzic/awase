//! 窓切替(フォーカス変更)の直後、IME の refresh が新しい窓の状態を読み終える前に届いた打鍵を保持し、
//! refresh の完了(または期限・上限)で順序どおり再生する状態機械(PR #809 の M1 の修正)。
//!
//! 純粋: 時刻・フォーカスの世代は引数で受け、保持する打鍵 `T` は呼び出し側の型(殻では `RawKeyEvent`)。殻は
//! `runtime/focus_switch_hold.rs`。
//!
//! ## なぜ要るか
//!
//! フォーカス変更(`EVENT_OBJECT_FOCUS`)から、debounce 後の refresh が新しい窓の分類・belief・Engine の活性を
//! 反映するまで、CI で中央 67ms・最大 228ms かかる(`tuning::FOCUS_SWITCH_HOLD_MAX_MS` の根拠)。その間の打鍵は
//! 前の窓の状態(Engine の活性・NonText・belief)で処理される。#809 で TsfGate を撤去した後、0〜30ms の最初の
//! 1 打鍵がほぼ全件これで崩れ、読めない窓(Windows Terminal・WezTerm)× GJI では素通しの変換キーが GJI を閉じて
//! 以後ずっとローマ字のままになった(docs/tasks/mech-ecrs-2026-10-21/verify-809-unconfirmed-result.md)。
//!
//! ## 旧 TsfGate との違い
//!
//! - 窓の種類で分けない(TSF の確定・Probing・Ready の状態を持たない)。全窓で同じ。
//! - 保持を**始める**のは「フォーカス変更から `max_ms` 以内」かつ「refresh がまだ終わっていない」間に届いた物理の
//!   KeyDown のうち、文字・親指キーで Ctrl/Alt/Win なしのもの、または IME のモードキー(親指でない変換・無変換・
//!   半角/全角・ひらがな・英数など。修飾を問わない)だけ([`starts_hold`])。Alt+Tab の Tab・ショートカット・
//!   修飾キー・KeyUp・リピート・外部注入では始めない(TsfGate は全キーを保持した)。
//! - 期限は保持の開始から `max_ms`(TsfGate はフォーカス変更から 500ms)。最初の打鍵の遅れはこれを超えない。
//! - 解放の契機は「保持より後のフォーカス変更を見た refresh の完了」(世代で照合。フォーカス変更より前に
//!   spawn された refresh の完了では解放しない)。
//!
//! ## 契機の表
//!
//! - フォーカス変更: 待機(`Armed`)に入る。保持中なら保持を続け、照合する世代だけ新しくする(連続するフォーカス
//!   イベントで保持を捨てない)。
//! - 待機中の打鍵: [`starts_hold`] が真なら保持を始める(`Started`)。それ以外は通す。待機が `max_ms` を
//!   過ぎていたら待機を終えて通す。
//! - 保持中の打鍵: 保持の中の Down に対応する Up・リピートは足す。保持の外で既に届いた Down の Up は通す(Alt+Tab
//!   の Alt の解放を止めない)。**ただし修飾キーの Up は、保持中の打鍵のどれかがその修飾つきで押されていれば足す**
//!   (再生は VK だけを送るので、Shift の離しが先に OS へ届くと保持した文字の Shift が落ちる。閉ループ検証の V1)。
//!   新しい Down は(始める条件を問わず)足す。上限・期限を超えたら、その打鍵を末尾に含めて全て解放する(追い越させない)。
//! - refresh の完了(`on_refresh_settled(seq)`): `seq` が待機・保持の世代以上なら終える(保持していれば再生)。
//! - 期限(`on_deadline`)・強制の解放(`release_all`): 保持していれば再生する。

use crate::state::half_width_resume_hold::KeyFacts;
use crate::state::TickMs;

/// 修飾の印(`HoldKey::mods_at_press`・`HoldKey::modifier`)。
pub const MOD_SHIFT: u8 = 1;
/// Ctrl の印。
pub const MOD_CTRL: u8 = 2;
/// Alt の印。
pub const MOD_ALT: u8 = 4;
/// Win の印。
pub const MOD_WIN: u8 = 8;

/// 保持を始めうる打鍵の種類(殻が `RawKeyEvent` から分類する)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartKind {
    /// 文字・親指キー(`KeyClassification::{Char, LeftThumb, RightThumb}`)。
    CharOrThumb,
    /// IME のモードキー(`vk::is_ime_mode_key_for_ime`: 変換・無変換・半角/全角・かな・ひらがな・英数など)。
    /// 親指として設定されていない変換キー・半角/全角が古い窓の文脈で素通しになると、効きの予測が前の窓の belief に
    /// 付き(`advance_focus_tracking` が refresh の時点の belief を前の窓のキャッシュへ保存する)、読めない窓の実 IME と
    /// 食い違ったまま残る(閉ループ検証の V2)。
    ImeKey,
    /// それ以外(修飾キー・Tab・矢印など)。
    Other,
}

/// 保持の判断に使う打鍵の事実。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldKey {
    /// 物理キーの同一性と Down/Up・リピート。
    pub facts: KeyFacts,
    /// 押下時の修飾の状態(`MOD_*` の和。`RawKeyEvent::modifier_snapshot`)。
    pub mods_at_press: u8,
    /// この打鍵が修飾キーなら、その印(`MOD_*` の 1 つ)。修飾キーでなければ 0。
    pub modifier: u8,
    /// 他プロセスの注入か(`RawKeyEvent::injected`)。
    pub injected: bool,
    /// 保持を始めうる種類。
    pub kind: StartKind,
}

/// 待機中にこの打鍵から保持を始めるか(純粋)。
///
/// 物理の新しい KeyDown(リピート・外部注入でない)で、文字・親指キーなら Ctrl/Alt/Win なし
/// (`RawKeyEvent::starts_focus_resync` と同じ。ショートカットと Alt+Tab の Tab を遅らせない)、IME のモードキーなら
/// 修飾を問わない(Ctrl+無変換 なども IME の操作で、古い窓の文脈で処理されると効きの予測が前の窓に付く)。
#[must_use]
pub const fn starts_hold(key: HoldKey) -> bool {
    if !key.facts.is_down || key.facts.is_repeat || key.injected {
        return false;
    }
    match key.kind {
        StartKind::CharOrThumb => key.mods_at_press & (MOD_CTRL | MOD_ALT | MOD_WIN) == 0,
        StartKind::ImeKey => true,
        StartKind::Other => false,
    }
}

/// 保持する Down の上限。CI の打鍵間隔 40ms・保持 ~70ms では 2 件前後で解放される。
pub const FOCUS_SWITCH_HOLD_CAP: usize = 8;

/// 打鍵の扱い。
#[derive(Debug, PartialEq, Eq)]
pub enum OnKey<T> {
    /// 保持しない(呼び出し側が通常どおり処理する。打鍵を返す)。
    Pass(T),
    /// 保持を始めた(呼び出し側は期限のタイマーを張り、`Consumed` を返す)。
    Started,
    /// 保持に足した(呼び出し側は `Consumed` を返す)。
    Held,
    /// 上限・期限に達した。この打鍵を末尾に含めた列を再生する(呼び出し側は `Consumed` を返す)。
    Release(Vec<T>),
}

#[derive(Debug)]
struct Held<T> {
    /// 解放に要る refresh の世代(保持中にフォーカス変更が来たら新しくする)。
    seq: u64,
    started_at: TickMs,
    items: Vec<(HoldKey, T)>,
}

#[derive(Debug)]
enum Phase<T> {
    Idle,
    Armed { at: TickMs, seq: u64 },
    Holding(Held<T>),
}

/// 窓切替直後の打鍵の保持。同時に 1 つだけ。
#[derive(Debug)]
pub struct FocusSwitchHold<T> {
    phase: Phase<T>,
    max_ms: u64,
}

impl<T> FocusSwitchHold<T> {
    /// `max_ms` は待機の長さと保持の期限(殻は `tuning::FOCUS_SWITCH_HOLD_MAX_MS` を渡す)。
    #[must_use]
    pub const fn new(max_ms: u64) -> Self {
        Self {
            phase: Phase::Idle,
            max_ms,
        }
    }

    #[must_use]
    pub const fn is_holding(&self) -> bool {
        matches!(self.phase, Phase::Holding(_))
    }

    #[must_use]
    pub const fn is_armed(&self) -> bool {
        matches!(self.phase, Phase::Armed { .. })
    }

    /// 保持の開始時刻(ログの hold_ms 用)。
    #[must_use]
    pub const fn started_at(&self) -> Option<TickMs> {
        match &self.phase {
            Phase::Holding(held) => Some(held.started_at),
            _ => None,
        }
    }

    /// 保持中の件数(ログ用)。
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.phase {
            Phase::Holding(held) => held.items.len(),
            _ => 0,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// フォーカス変更を見た。`seq` はこの変更の世代(単調増加)。
    pub fn on_focus_event(&mut self, now: TickMs, seq: u64) {
        if let Phase::Holding(held) = &mut self.phase {
            held.seq = seq;
        } else {
            self.phase = Phase::Armed { at: now, seq };
        }
    }

    /// 打鍵が来た。
    pub fn on_key(&mut self, now: TickMs, key: HoldKey, item: T) -> OnKey<T> {
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle => OnKey::Pass(item),
            Phase::Armed { at, seq } => self.on_key_armed(at, seq, now, key, item),
            Phase::Holding(held) => self.on_key_holding(held, now, key, item),
        }
    }

    /// 待機中の打鍵。呼ばれた時点で `phase` は `Idle`(続けるなら戻す)。
    fn on_key_armed(
        &mut self,
        at: TickMs,
        seq: u64,
        now: TickMs,
        key: HoldKey,
        item: T,
    ) -> OnKey<T> {
        if now.0.saturating_sub(at.0) >= self.max_ms {
            // 待機が切れた(refresh の完了を取りこぼした等)。以後は通す。
            return OnKey::Pass(item);
        }
        if starts_hold(key) {
            self.phase = Phase::Holding(Held {
                seq,
                started_at: now,
                items: vec![(key, item)],
            });
            return OnKey::Started;
        }
        self.phase = Phase::Armed { at, seq };
        OnKey::Pass(item)
    }

    /// 保持中の打鍵。呼ばれた時点で `phase` は `Idle`(続けるなら戻す)。
    fn on_key_holding(
        &mut self,
        mut held: Held<T>,
        now: TickMs,
        key: HoldKey,
        item: T,
    ) -> OnKey<T> {
        if !Self::joins_hold(&held, key) {
            // 保持の外で既に届いた Down の Up・リピートは通す(押されたまま残さない。Alt+Tab の Alt の解放)。
            self.phase = Phase::Holding(held);
            return OnKey::Pass(item);
        }
        held.items.push((key, item));
        let downs = held
            .items
            .iter()
            .filter(|(k, _)| k.facts.is_down && !k.facts.is_repeat)
            .count();
        let expired = now.0.saturating_sub(held.started_at.0) >= self.max_ms;
        if downs > FOCUS_SWITCH_HOLD_CAP || expired {
            return OnKey::Release(Self::items_of(held.items));
        }
        self.phase = Phase::Holding(held);
        OnKey::Held
    }

    /// refresh が終わった。`seq` は refresh を spawn した時点で見ていたフォーカスの世代。
    /// 待機・保持の世代以上なら終える。保持していれば再生する列を返す。
    pub fn on_refresh_settled(&mut self, seq: u64) -> Option<Vec<T>> {
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle => None,
            Phase::Armed { at, seq: armed } => {
                if seq < armed {
                    self.phase = Phase::Armed { at, seq: armed };
                }
                None
            }
            Phase::Holding(held) => {
                if seq < held.seq {
                    self.phase = Phase::Holding(held);
                    return None;
                }
                Some(Self::items_of(held.items))
            }
        }
    }

    /// 期限のタイマーが満了した。保持していれば再生する列を返す。保持していなければ何もしない(待機はそのまま)。
    pub fn on_deadline(&mut self) -> Option<Vec<T>> {
        if !self.is_holding() {
            return None;
        }
        self.release_all()
    }

    /// 強制の解放(panic の復旧・設定の再読み込み)。待機も終える。保持していれば再生する列を返す。
    pub fn release_all(&mut self) -> Option<Vec<T>> {
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Holding(held) => Some(Self::items_of(held.items)),
            Phase::Idle | Phase::Armed { .. } => None,
        }
    }

    /// 保持中に来た打鍵を保持に足すか。新しい Down は足す。Up・リピートは、保持の中に同じキーの Down があるか、
    /// 修飾キーで保持中の打鍵のどれかがその修飾つきで押されていれば足す(V1: Shift の離しが保持した文字を追い越さない)。
    fn joins_hold(held: &Held<T>, key: HoldKey) -> bool {
        if key.facts.is_down && !key.facts.is_repeat {
            return true;
        }
        held.items.iter().any(|(k, _)| {
            (k.facts.is_down && k.facts.identity == key.facts.identity)
                || (key.modifier != 0 && k.mods_at_press & key.modifier != 0)
        })
    }

    fn items_of(items: Vec<(HoldKey, T)>) -> Vec<T> {
        items.into_iter().map(|(_, t)| t).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: u64 = 250;
    const A: u64 = 0x11;
    const B: u64 = 0x22;
    const ALT: u64 = 0x38;

    const fn down(identity: u64) -> KeyFacts {
        KeyFacts {
            identity,
            is_down: true,
            is_repeat: false,
        }
    }
    const fn up(identity: u64) -> KeyFacts {
        KeyFacts {
            identity,
            is_down: false,
            is_repeat: false,
        }
    }
    const fn repeat(identity: u64) -> KeyFacts {
        KeyFacts {
            identity,
            is_down: true,
            is_repeat: true,
        }
    }

    /// 修飾なしの打鍵。`starts` なら文字(保持を始めうる)、そうでなければ Other。
    const fn key(facts: KeyFacts, starts: bool) -> HoldKey {
        HoldKey {
            facts,
            mods_at_press: 0,
            modifier: 0,
            injected: false,
            kind: if starts {
                StartKind::CharOrThumb
            } else {
                StartKind::Other
            },
        }
    }

    const fn with(k: HoldKey, kind: StartKind, mods_at_press: u8, modifier: u8) -> HoldKey {
        HoldKey {
            kind,
            mods_at_press,
            modifier,
            ..k
        }
    }

    fn armed(seq: u64) -> FocusSwitchHold<&'static str> {
        let mut h = FocusSwitchHold::new(MAX);
        h.on_focus_event(TickMs(1000), seq);
        h
    }

    /// フォーカス変更が無ければ何も保持しない。
    #[test]
    fn idle_passes_everything() {
        let mut h: FocusSwitchHold<&str> = FocusSwitchHold::new(MAX);
        assert_eq!(
            h.on_key(TickMs(0), key(down(A), true), "a"),
            OnKey::Pass("a")
        );
        assert_eq!(h.on_refresh_settled(5), None);
        assert_eq!(h.on_deadline(), None);
    }

    /// M1 の本体: 切替から 0〜30ms の最初の打鍵は保持され、refresh の完了で順序どおり再生される。
    #[test]
    fn first_key_before_refresh_is_held_and_replayed_in_order() {
        let mut h = armed(1);
        assert_eq!(
            h.on_key(TickMs(1000), key(down(A), true), "A↓"),
            OnKey::Started
        );
        assert_eq!(h.on_key(TickMs(1020), key(up(A), false), "A↑"), OnKey::Held);
        assert_eq!(
            h.on_key(TickMs(1040), key(down(B), true), "B↓"),
            OnKey::Held
        );
        assert_eq!(h.on_refresh_settled(1), Some(vec!["A↓", "A↑", "B↓"]));
        assert!(!h.is_holding() && !h.is_armed());
        // 解放後の B↑ は保持の外(B↓ は再生で届く)。
        assert_eq!(
            h.on_key(TickMs(1070), key(up(B), false), "B↑"),
            OnKey::Pass("B↑")
        );
    }

    /// refresh が先に終われば待機は終わり、以後の打鍵は保持しない(60ms 以降が変わらない)。
    #[test]
    fn refresh_before_first_key_disarms() {
        let mut h = armed(1);
        assert_eq!(h.on_refresh_settled(1), None);
        assert!(!h.is_armed());
        assert_eq!(
            h.on_key(TickMs(1060), key(down(A), true), "a"),
            OnKey::Pass("a")
        );
    }

    /// フォーカス変更より前に spawn された refresh(古い世代)の完了では解放しない。
    #[test]
    fn stale_refresh_does_not_release() {
        let mut h = armed(2);
        assert_eq!(
            h.on_key(TickMs(1000), key(down(A), true), "a"),
            OnKey::Started
        );
        assert_eq!(h.on_refresh_settled(1), None);
        assert!(h.is_holding());
        assert_eq!(h.on_refresh_settled(2), Some(vec!["a"]));
    }

    /// 古い世代の refresh は待機も終えない。
    #[test]
    fn stale_refresh_keeps_armed() {
        let mut h = armed(2);
        assert_eq!(h.on_refresh_settled(1), None);
        assert!(h.is_armed());
    }

    /// 保持中に次のフォーカス変更が来たら、保持を続けて新しい世代の refresh を待つ(連続フォーカスで捨てない)。
    #[test]
    fn focus_event_while_holding_keeps_items_and_waits_newer_refresh() {
        let mut h = armed(1);
        assert_eq!(
            h.on_key(TickMs(1000), key(down(A), true), "a"),
            OnKey::Started
        );
        h.on_focus_event(TickMs(1010), 2);
        assert!(h.is_holding());
        assert_eq!(h.on_refresh_settled(1), None);
        assert_eq!(h.on_refresh_settled(2), Some(vec!["a"]));
    }

    /// 保持を始めない打鍵(修飾キー・ショートカット・Passthrough の分類)は待機中も通す。待機は続く。
    #[test]
    fn non_starting_keys_pass_while_armed() {
        let mut h = armed(1);
        assert_eq!(
            h.on_key(TickMs(1000), key(down(ALT), false), "Alt↓"),
            OnKey::Pass("Alt↓")
        );
        assert!(h.is_armed());
        assert_eq!(
            h.on_key(TickMs(1005), key(down(A), true), "a"),
            OnKey::Started
        );
    }

    /// 保持を始める打鍵でも KeyUp・リピートからは始めない。
    #[test]
    fn up_or_repeat_does_not_start() {
        let mut h = armed(1);
        assert_eq!(
            h.on_key(TickMs(1000), key(up(A), true), "A↑"),
            OnKey::Pass("A↑")
        );
        assert_eq!(
            h.on_key(TickMs(1001), key(repeat(A), true), "A⟳"),
            OnKey::Pass("A⟳")
        );
        assert!(h.is_armed());
    }

    /// 保持の外で届いた Down の Up(Alt+Tab の Alt の解放など)は保持中も通す。
    #[test]
    fn up_of_key_pressed_before_hold_passes() {
        let mut h = armed(1);
        assert_eq!(
            h.on_key(TickMs(1000), key(down(A), true), "a"),
            OnKey::Started
        );
        assert_eq!(
            h.on_key(TickMs(1001), key(up(ALT), false), "Alt↑"),
            OnKey::Pass("Alt↑")
        );
        assert_eq!(
            h.on_key(TickMs(1002), key(repeat(B), false), "B⟳"),
            OnKey::Pass("B⟳")
        );
        assert_eq!(h.len(), 1);
    }

    /// 保持中の新しい Down は starts を問わず足す(修飾キーの Down も順序を保つ)。
    #[test]
    fn any_new_down_is_held_while_holding() {
        let mut h = armed(1);
        assert_eq!(
            h.on_key(TickMs(1000), key(down(A), true), "a"),
            OnKey::Started
        );
        assert_eq!(
            h.on_key(TickMs(1001), key(down(ALT), false), "Alt↓"),
            OnKey::Held
        );
        assert_eq!(
            h.on_key(TickMs(1002), key(up(ALT), false), "Alt↑"),
            OnKey::Held
        );
        assert_eq!(h.on_refresh_settled(1), Some(vec!["a", "Alt↓", "Alt↑"]));
    }

    /// 待機は max_ms で切れる(refresh の完了を取りこぼしても、ずっと保持し始めない)。
    #[test]
    fn armed_window_expires() {
        let mut h = armed(1);
        assert_eq!(
            h.on_key(TickMs(1000 + MAX), key(down(A), true), "a"),
            OnKey::Pass("a")
        );
        assert!(!h.is_armed());
        assert_eq!(
            h.on_key(TickMs(1000 + MAX + 1), key(down(B), true), "b"),
            OnKey::Pass("b")
        );
    }

    /// 保持は期限のタイマーで解放される。
    #[test]
    fn deadline_releases() {
        let mut h = armed(1);
        assert_eq!(
            h.on_key(TickMs(1000), key(down(A), true), "a"),
            OnKey::Started
        );
        assert_eq!(h.on_deadline(), Some(vec!["a"]));
        assert_eq!(h.on_deadline(), None);
        assert_eq!(h.on_refresh_settled(1), None);
    }

    /// 期限のタイマーは待機を終えない(保持していないなら何もしない)。
    #[test]
    fn deadline_without_hold_is_noop() {
        let mut h = armed(1);
        assert_eq!(h.on_deadline(), None);
        assert!(h.is_armed());
    }

    /// タイマーを取りこぼしても、保持の開始から max_ms を過ぎた打鍵で、その打鍵を末尾に含めて解放する。
    #[test]
    fn late_key_after_deadline_releases_with_it_at_the_end() {
        let mut h = armed(1);
        assert_eq!(
            h.on_key(TickMs(1000), key(down(A), true), "a"),
            OnKey::Started
        );
        assert_eq!(
            h.on_key(TickMs(1000 + MAX), key(down(B), true), "b"),
            OnKey::Release(vec!["a", "b"])
        );
        assert!(!h.is_holding());
    }

    /// 上限を超えた Down は、その打鍵を末尾に含めて全て解放する(追い越させない)。
    #[test]
    fn cap_releases_with_trigger_at_the_end() {
        let mut h: FocusSwitchHold<u64> = FocusSwitchHold::new(MAX);
        h.on_focus_event(TickMs(1000), 1);
        assert_eq!(
            h.on_key(TickMs(1000), key(down(0), true), 0),
            OnKey::Started
        );
        for i in 1..FOCUS_SWITCH_HOLD_CAP as u64 {
            assert_eq!(
                h.on_key(TickMs(1000 + i), key(down(i), true), i),
                OnKey::Held
            );
        }
        let last = FOCUS_SWITCH_HOLD_CAP as u64;
        match h.on_key(TickMs(1100), key(down(last), true), last) {
            OnKey::Release(v) => {
                assert_eq!(v.len(), FOCUS_SWITCH_HOLD_CAP + 1);
                assert_eq!(v.last(), Some(&last));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// 強制の解放は保持を再生し、待機も終える。
    #[test]
    fn release_all_replays_and_disarms() {
        let mut h = armed(1);
        assert_eq!(h.release_all(), None);
        assert!(!h.is_armed());
        let mut h = armed(1);
        assert_eq!(
            h.on_key(TickMs(1000), key(down(A), true), "a"),
            OnKey::Started
        );
        assert_eq!(h.release_all(), Some(vec!["a"]));
    }

    /// 保持を始める条件の表(V2): 文字・親指は Ctrl/Alt/Win なし、IME のモードキーは修飾を問わない。
    /// KeyUp・リピート・外部注入・その他のキーでは始めない。
    #[test]
    fn starts_hold_table() {
        let ch = key(down(A), true);
        assert!(starts_hold(ch));
        assert!(starts_hold(with(ch, StartKind::CharOrThumb, MOD_SHIFT, 0)));
        for m in [MOD_CTRL, MOD_ALT, MOD_WIN] {
            assert!(
                !starts_hold(with(ch, StartKind::CharOrThumb, m, 0)),
                "mods={m}"
            );
        }
        let ime = with(key(down(B), false), StartKind::ImeKey, 0, 0);
        assert!(starts_hold(ime));
        assert!(starts_hold(with(
            ime,
            StartKind::ImeKey,
            MOD_CTRL | MOD_SHIFT,
            0
        )));
        assert!(!starts_hold(with(
            key(up(B), false),
            StartKind::ImeKey,
            0,
            0
        )));
        assert!(!starts_hold(with(
            key(repeat(B), false),
            StartKind::ImeKey,
            0,
            0
        )));
        assert!(!starts_hold(HoldKey {
            injected: true,
            ..ime
        }));
        assert!(!starts_hold(key(down(B), false)));
    }

    /// V2: 親指でない変換キー・半角/全角は、待機中に来たら保持を始める(古い窓の文脈で処理させない)。
    #[test]
    fn ime_key_starts_hold_while_armed() {
        let mut h = armed(1);
        let conv = with(key(down(B), false), StartKind::ImeKey, 0, 0);
        assert_eq!(h.on_key(TickMs(1000), conv, "変換↓"), OnKey::Started);
        let conv_up = with(key(up(B), false), StartKind::ImeKey, 0, 0);
        assert_eq!(h.on_key(TickMs(1030), conv_up, "変換↑"), OnKey::Held);
        assert_eq!(h.on_refresh_settled(1), Some(vec!["変換↓", "変換↑"]));
    }

    /// V1: 切替の前に押した Shift の離しは、保持中の文字が Shift つきで押されていれば保持に足す(文字を追い越さない)。
    #[test]
    fn shift_up_waits_for_held_shifted_key() {
        const SHIFT: u64 = 0x2A;
        let mut h = armed(1);
        let a = with(key(down(A), true), StartKind::CharOrThumb, MOD_SHIFT, 0);
        assert_eq!(h.on_key(TickMs(1000), a, "A↓+S"), OnKey::Started);
        let shift_up = with(key(up(SHIFT), false), StartKind::Other, 0, MOD_SHIFT);
        assert_eq!(h.on_key(TickMs(1025), shift_up, "Shift↑"), OnKey::Held);
        assert_eq!(h.on_refresh_settled(1), Some(vec!["A↓+S", "Shift↑"]));
    }

    /// V1 の範囲: 保持中の打鍵が Shift なしなら、Shift の離しは従来どおり通す。Alt+Tab の Alt の離しも通す
    /// (保持中の打鍵が Alt つきで押されていない)。
    #[test]
    fn modifier_up_passes_when_no_held_key_used_it() {
        const SHIFT: u64 = 0x2A;
        let mut h = armed(1);
        assert_eq!(
            h.on_key(TickMs(1000), key(down(A), true), "a"),
            OnKey::Started
        );
        let shift_up = with(key(up(SHIFT), false), StartKind::Other, 0, MOD_SHIFT);
        assert_eq!(
            h.on_key(TickMs(1010), shift_up, "Shift↑"),
            OnKey::Pass("Shift↑")
        );
        let alt_up = with(key(up(ALT), false), StartKind::Other, 0, MOD_ALT);
        assert_eq!(h.on_key(TickMs(1020), alt_up, "Alt↑"), OnKey::Pass("Alt↑"));
        assert_eq!(h.on_refresh_settled(1), Some(vec!["a"]));
    }

    /// 修飾キーでない Up は、修飾の状態にかかわらず保持の外の Down のものなら通す。
    #[test]
    fn non_modifier_up_ignores_mods() {
        let mut h = armed(1);
        let a = with(key(down(A), true), StartKind::CharOrThumb, MOD_SHIFT, 0);
        assert_eq!(h.on_key(TickMs(1000), a, "A↓+S"), OnKey::Started);
        assert_eq!(
            h.on_key(TickMs(1010), key(up(B), false), "B↑"),
            OnKey::Pass("B↑")
        );
    }

    /// 解放後の次のフォーカス変更で、また待機に入る。
    #[test]
    fn rearm_after_release() {
        let mut h = armed(1);
        assert_eq!(
            h.on_key(TickMs(1000), key(down(A), true), "a"),
            OnKey::Started
        );
        assert_eq!(h.on_refresh_settled(1), Some(vec!["a"]));
        h.on_focus_event(TickMs(5000), 2);
        assert_eq!(
            h.on_key(TickMs(5010), key(down(B), true), "b"),
            OnKey::Started
        );
        assert_eq!(h.started_at(), Some(TickMs(5010)));
    }
}
