//! ADR-279 案 P の段階 P0(観測だけ): 明示 OFF の意図が残っている間の IMM の読みが、「追随すべきだった」形かを数える判断の核。
//!
//! 明示 OFF の意図があると IME の読み直し(refresh)は止まる(`runtime/mod.rs::reschedule_ime_refresh`)。段階 0 の測定
//! (`docs/tasks/adr279-stage0-result-2026-10-23.md` S0-1)では、この間に実 IME が開いたまま(BUG-110 型)でも awase は観測を
//! 1 件も得なかった。P0 は、専用のタイマーで IMM の読みだけを取り(`runtime/drift_follow_would.rs`)、ここで次を判定する:
//!
//! - **「`ime_on=Some(true)`(日本語入力)が、書き込みの柵のあとも `follow_after_ms` 続いた」= 追随すべきだった(`Crossed`)**。
//! - IME へは何も書かず、belief・Engine・意図も変えない。判定は journal と awase.log に残すだけ。
//!
//! # 読みの分類(半角英数の扱い、設計メモ §2.2)
//!
//! `open=1` でも conv の NATIVE ビットが無い読み(半角英数。MS-IME × Chrome で未確定の文字があるときの Ctrl+無変換、
//! スパイク M1 (c) の 38/40 で `conv=0x10`)は、**開の扱いに含めない**。Engine が OFF のままなのは英数の軸では実状態と
//! そろっており、開の軸だけで追随すると Engine が ON になって英字がローマ字→かなになる(BUG-185 の形)。数え分けるために、
//! 別の連続(`RunClass::Alnum`)として記録し、`would_follow` には数えない。conv が読めない開(`OpenConvUnknown`)は
//! どちらとも決められないので連続を切る。
//!
//! # 柵(`settle_ms`)
//!
//! 読みの**開始**が、awase の最後の操作(全キー + ImmCross の書き込みの発行。`hook::last_operation_tick_ms`、ADR-268 R3' と
//! 同じ基準)から `settle_ms`(`KEY_EFFECT_SETTLE_MS`=170ms)未満なら、IME が処理する前の古い状態を見うるので**数えない**
//! (連続を伸ばしも切りもしない)。基準は読みの完了時に取る(読みの途中の打鍵も柵に入れる)。
//!
//! # 窓
//!
//! 連続は窓(`F`、殻ではフォーカスの柵)ごと。読みの開始と完了で窓が違う、または連続を始めた窓と違うときは連続を捨てる。
//!
//! 時刻は引数で受ける(`hook` に依存しない)。

use crate::state::external_change_watch::IME_CMODE_NATIVE;

/// 1 回の IMM の読みの分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadClass {
    /// 開・日本語入力(conv に NATIVE ビットあり)。明示 OFF の意図への反証になる唯一の読み。
    OpenNative,
    /// 開・半角英数(conv に NATIVE ビットなし。`open=1, conv=0x10` 等)。開の扱いに含めない。
    OpenAlnum,
    /// 開だが conv が読めない。日本語か英数か決められない。
    OpenConvUnknown,
    /// 閉。
    Closed,
    /// 開閉が読めない(`None`・時間切れ)。
    Unknown,
}

impl ReadClass {
    /// 記録用の名前。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenNative => "open-native",
            Self::OpenAlnum => "open-alnum",
            Self::OpenConvUnknown => "open-conv-unknown",
            Self::Closed => "closed",
            Self::Unknown => "unknown",
        }
    }

    /// 連続を作る読みなら、その種類。
    const fn run_class(self) -> Option<RunClass> {
        match self {
            Self::OpenNative => Some(RunClass::Native),
            Self::OpenAlnum => Some(RunClass::Alnum),
            Self::OpenConvUnknown | Self::Closed | Self::Unknown => None,
        }
    }
}

/// IMM の読み(`ime_on`・`conversion_mode`)を分類する。英数かどうかは NATIVE ビットだけで決める
/// (`external_change_watch::classify_direct_read` と同じ規則。ROMAN ビットは実 Chrome × GJI で当てにならない)。
#[must_use]
pub const fn classify_read(open: Option<bool>, conv: Option<u32>) -> ReadClass {
    match (open, conv) {
        (Some(true), Some(c)) => {
            if c & IME_CMODE_NATIVE != 0 {
                ReadClass::OpenNative
            } else {
                ReadClass::OpenAlnum
            }
        }
        (Some(true), None) => ReadClass::OpenConvUnknown,
        (Some(false), _) => ReadClass::Closed,
        (None, _) => ReadClass::Unknown,
    }
}

/// 開の連続の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunClass {
    /// 開・日本語入力の連続。`follow_after_ms` 続いたら「追随すべきだった」。
    Native,
    /// 開・半角英数の連続。開の扱いに含めない(数え分けるだけ)。
    Alnum,
}

impl RunClass {
    /// 記録用の名前(`ReadClass::label` と同じ表記)。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Native => "open-native",
            Self::Alnum => "open-alnum",
        }
    }
}

/// 走っている開の連続。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run<F> {
    pub class: RunClass,
    /// 連続を始めた窓。
    pub focus: F,
    /// 連続の最初の(柵の後の)読みの開始時刻。
    pub since_ms: u64,
    /// 連続の最後の(柵の後の)読みの開始時刻。
    pub last_ms: u64,
    /// `follow_after_ms` に達したことを既に記録したか(連続ごとに 1 回だけ記録する)。
    pub crossed: bool,
}

impl<F> Run<F> {
    /// 最初から最後の読みまでの長さ。
    #[must_use]
    pub const fn elapsed_ms(&self) -> u64 {
        self.last_ms.saturating_sub(self.since_ms)
    }
}

/// 連続が終わった理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// 明示 OFF の意図が無くなった(意図が外れた・ON になった)。
    IntentGone,
    /// 窓が変わった。
    FocusMoved,
    /// 閉を読んだ。
    Closed,
    /// 開閉が読めなかった。
    Unknown,
    /// 開だが conv が読めなかった。
    ConvUnknown,
    /// 開のまま日本語⇄英数が入れ替わった(新しい連続を始める)。
    ClassChanged,
}

impl EndReason {
    /// 記録用の名前。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::IntentGone => "intent-gone",
            Self::FocusMoved => "focus-moved",
            Self::Closed => "closed",
            Self::Unknown => "unknown",
            Self::ConvUnknown => "conv-unknown",
            Self::ClassChanged => "class-changed",
        }
    }
}

/// 終わった連続。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunEnd {
    pub class: RunClass,
    /// 最初から最後の(柵の後の)読みまでの長さ。
    pub elapsed_ms: u64,
    /// `follow_after_ms` に達していたか。
    pub crossed: bool,
    pub reason: EndReason,
}

/// 1 回の読みの判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// 明示 OFF の意図が無い・窓が変わった。何も数えない(連続があれば `ended` に入る)。
    Idle,
    /// 柵の中の読み。連続を伸ばしも切りもしない。
    Held,
    /// 連続を作らない読み(閉・不明・conv 不明)。
    NotOpen,
    /// 連続を始めた。
    Started(RunClass),
    /// 連続が続いている(まだ `follow_after_ms` に達していない、または既に記録済み)。
    Continuing { class: RunClass, elapsed_ms: u64 },
    /// 連続が初めて `follow_after_ms` に達した。`Native` なら「追随すべきだった」(`would_follow`)。
    Crossed { class: RunClass, elapsed_ms: u64 },
}

/// 1 回の読みの判定の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub event: Event,
    /// この読みで終わった連続(あれば)。
    pub ended: Option<RunEnd>,
}

/// 1 回の読みの事実。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadFacts<F> {
    /// 読みの完了時に、明示意図が `Some(false)` か(殻は対象の窓かの判定 [`applies`] の値を渡す)。
    pub explicit_off: bool,
    /// 読みの開始時の窓。
    pub focus_at_start: F,
    /// 読みの完了時の窓。
    pub focus_now: F,
    pub read: ReadClass,
    /// 読みの開始時刻。
    pub read_started_ms: u64,
    /// 読みの完了時に取った、awase の最後の操作の時刻。
    pub last_operation_ms: u64,
}

/// 読みの開始が、awase の最後の操作から `settle_ms` 以上あとか(柵の後の読みか)。
/// 最後の操作が読みの開始より後(読みの途中の打鍵)なら柵の中。
#[must_use]
pub const fn read_is_after_fence(
    read_started_ms: u64,
    last_operation_ms: u64,
    settle_ms: u64,
) -> bool {
    read_started_ms >= last_operation_ms && read_started_ms - last_operation_ms >= settle_ms
}

const fn end_of<F>(run: &Run<F>, reason: EndReason) -> RunEnd {
    RunEnd {
        class: run.class,
        elapsed_ms: run.elapsed_ms(),
        crossed: run.crossed,
        reason,
    }
}

/// 1 回の読みを判定し、次の連続と結果を返す(副作用なし)。
///
/// `settle_ms` は柵(`KEY_EFFECT_SETTLE_MS`)、`follow_after_ms` は「長い drift」の境界
/// (`DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS`)。
#[must_use]
pub fn step<F: Copy + PartialEq>(
    run: Option<Run<F>>,
    facts: &ReadFacts<F>,
    settle_ms: u64,
    follow_after_ms: u64,
) -> (Option<Run<F>>, Outcome) {
    // 窓の変化を先に見る(別の窓へ移って対象外になったときも、終わりの理由を「窓が変わった」にする)。
    // 読みの途中で窓が変わった、または連続を始めた窓と違う。別の窓の読みを数えない。
    let focus_moved =
        facts.focus_at_start != facts.focus_now || run.is_some_and(|r| r.focus != facts.focus_now);
    if focus_moved {
        return (
            None,
            Outcome {
                event: Event::Idle,
                ended: run.map(|r| end_of(&r, EndReason::FocusMoved)),
            },
        );
    }
    // 明示 OFF の意図が無い(または対象の窓でない)なら対象外。連続を捨てる。
    if !facts.explicit_off {
        return (
            None,
            Outcome {
                event: Event::Idle,
                ended: run.map(|r| end_of(&r, EndReason::IntentGone)),
            },
        );
    }
    // 柵の中の読みは数えない(連続はそのまま)。
    if !read_is_after_fence(facts.read_started_ms, facts.last_operation_ms, settle_ms) {
        return (
            run,
            Outcome {
                event: Event::Held,
                ended: None,
            },
        );
    }
    let Some(class) = facts.read.run_class() else {
        let reason = match facts.read {
            ReadClass::Closed => EndReason::Closed,
            ReadClass::OpenConvUnknown => EndReason::ConvUnknown,
            _ => EndReason::Unknown,
        };
        return (
            None,
            Outcome {
                event: Event::NotOpen,
                ended: run.map(|r| end_of(&r, reason)),
            },
        );
    };
    let start = |ended: Option<RunEnd>| {
        (
            Some(Run {
                class,
                focus: facts.focus_now,
                since_ms: facts.read_started_ms,
                last_ms: facts.read_started_ms,
                crossed: false,
            }),
            Outcome {
                event: Event::Started(class),
                ended,
            },
        )
    };
    match run {
        None => start(None),
        Some(r) if r.class != class => start(Some(end_of(&r, EndReason::ClassChanged))),
        Some(r) => {
            let next = Run {
                last_ms: facts.read_started_ms.max(r.last_ms),
                ..r
            };
            let elapsed_ms = next.elapsed_ms();
            if !r.crossed && elapsed_ms >= follow_after_ms {
                (
                    Some(Run {
                        crossed: true,
                        ..next
                    }),
                    Outcome {
                        event: Event::Crossed { class, elapsed_ms },
                        ended: None,
                    },
                )
            } else {
                (
                    Some(next),
                    Outcome {
                        event: Event::Continuing { class, elapsed_ms },
                        ended: None,
                    },
                )
            }
        }
    }
}

/// 専用のタイマーで読みを続ける窓か(P0 の対象)。
///
/// - 明示意図が `Some(false)`(明示 OFF。ON の意図・意図なしは対象外)。
/// - 直接観測の窓(`direct_watch_kind` が `Some`: `Imm32Unavailable` × GJI/同定済みの MS-IME 本体)。
/// - 実効 TSF ネイティブでない(`read_ime_state_full` がクラス名で `None` を返す窓。Windows Terminal・UWP など)。
/// - 無効アプリでない。
#[must_use]
pub const fn applies(
    explicit_intent: Option<bool>,
    direct_watch: bool,
    effectively_tsf_native: bool,
    app_disabled: bool,
) -> bool {
    matches!(explicit_intent, Some(false))
        && direct_watch
        && !effectively_tsf_native
        && !app_disabled
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTLE: u64 = crate::tuning::KEY_EFFECT_SETTLE_MS;
    const AFTER: u64 = crate::tuning::DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS;

    fn facts(read: ReadClass, at: u64) -> ReadFacts<u32> {
        ReadFacts {
            explicit_off: true,
            focus_at_start: 1,
            focus_now: 1,
            read,
            read_started_ms: at,
            last_operation_ms: 0,
        }
    }

    /// 連続した読み(500ms 間隔)を流し、最後の結果と連続を返す。
    fn feed(reads: &[(ReadClass, u64)]) -> (Option<Run<u32>>, Vec<Outcome>) {
        let mut run = None;
        let mut outs = Vec::new();
        for &(r, at) in reads {
            let (next, out) = step(run, &facts(r, at), SETTLE, AFTER);
            run = next;
            outs.push(out);
        }
        (run, outs)
    }

    #[test]
    fn classify_uses_native_bit_only() {
        assert_eq!(classify_read(Some(true), Some(0x19)), ReadClass::OpenNative);
        assert_eq!(classify_read(Some(true), Some(0x09)), ReadClass::OpenNative);
        // 半角英数(スパイク M1 (c) の conv=0x10)は開の扱いに含めない。
        assert_eq!(classify_read(Some(true), Some(0x10)), ReadClass::OpenAlnum);
        assert_eq!(classify_read(Some(true), Some(0)), ReadClass::OpenAlnum);
        assert_eq!(classify_read(Some(true), None), ReadClass::OpenConvUnknown);
        assert_eq!(classify_read(Some(false), Some(0x19)), ReadClass::Closed);
        assert_eq!(classify_read(Some(false), None), ReadClass::Closed);
        assert_eq!(classify_read(None, Some(0x19)), ReadClass::Unknown);
        assert_eq!(classify_read(None, None), ReadClass::Unknown);
    }

    #[test]
    fn fence_boundary() {
        assert!(!read_is_after_fence(1_000 + SETTLE - 1, 1_000, SETTLE));
        assert!(read_is_after_fence(1_000 + SETTLE, 1_000, SETTLE));
        // 読みの途中の打鍵(最後の操作が読みの開始より後)は柵の中。
        assert!(!read_is_after_fence(1_000, 1_001, SETTLE));
    }

    /// BUG-110 型: 開・日本語入力が `AFTER` 続いたら 1 回だけ `Crossed`。境界の直前は `Continuing`。
    #[test]
    fn native_open_crosses_once_at_the_boundary() {
        let t0 = 10_000;
        let (_, outs) = feed(&[
            (ReadClass::OpenNative, t0),
            (ReadClass::OpenNative, t0 + AFTER - 1),
            (ReadClass::OpenNative, t0 + AFTER),
            (ReadClass::OpenNative, t0 + AFTER + 500),
        ]);
        assert_eq!(outs[0].event, Event::Started(RunClass::Native));
        assert_eq!(
            outs[1].event,
            Event::Continuing {
                class: RunClass::Native,
                elapsed_ms: AFTER - 1
            }
        );
        assert_eq!(
            outs[2].event,
            Event::Crossed {
                class: RunClass::Native,
                elapsed_ms: AFTER
            }
        );
        assert_eq!(
            outs[3].event,
            Event::Continuing {
                class: RunClass::Native,
                elapsed_ms: AFTER + 500
            }
        );
        assert!(outs.iter().all(|o| o.ended.is_none()));
    }

    /// 閉・不明・conv 不明の読みは連続を切る(起点を捨てる)。
    #[test]
    fn non_open_reads_break_the_run() {
        for (read, reason) in [
            (ReadClass::Closed, EndReason::Closed),
            (ReadClass::Unknown, EndReason::Unknown),
            (ReadClass::OpenConvUnknown, EndReason::ConvUnknown),
        ] {
            let t0 = 10_000;
            let (run, outs) = feed(&[
                (ReadClass::OpenNative, t0),
                (ReadClass::OpenNative, t0 + 2_000),
                (read, t0 + 2_500),
                (ReadClass::OpenNative, t0 + 3_000),
                (ReadClass::OpenNative, t0 + 3_000 + AFTER - 1),
            ]);
            assert_eq!(outs[2].event, Event::NotOpen);
            assert_eq!(
                outs[2].ended,
                Some(RunEnd {
                    class: RunClass::Native,
                    elapsed_ms: 2_000,
                    crossed: false,
                    reason
                })
            );
            // 切れた後は新しい起点から数え直す(合計では AFTER を超えても Crossed にならない)。
            assert_eq!(outs[3].event, Event::Started(RunClass::Native));
            assert!(matches!(outs[4].event, Event::Continuing { .. }));
            assert_eq!(run.map(|r| r.since_ms), Some(t0 + 3_000));
        }
    }

    /// 半角英数(open=1, conv=0x10)は開の扱いに含めない: 日本語の連続を切り、別の連続として数え分ける。
    #[test]
    fn alnum_open_is_a_separate_run_and_never_native() {
        let t0 = 10_000;
        let (_, outs) = feed(&[
            (ReadClass::OpenNative, t0),
            (ReadClass::OpenAlnum, t0 + 1_000),
            (ReadClass::OpenAlnum, t0 + 1_000 + AFTER),
        ]);
        assert_eq!(outs[1].event, Event::Started(RunClass::Alnum));
        assert_eq!(
            outs[1].ended.map(|e| e.reason),
            Some(EndReason::ClassChanged)
        );
        assert_eq!(
            outs[2].event,
            Event::Crossed {
                class: RunClass::Alnum,
                elapsed_ms: AFTER
            }
        );
        // 英数だけの連続は、どれだけ続いても Native の Crossed を出さない。
        let reads: Vec<_> = (0..20)
            .map(|i| (ReadClass::OpenAlnum, t0 + i * 500))
            .collect();
        let (_, outs) = feed(&reads);
        assert!(!outs.iter().any(|o| matches!(
            o.event,
            Event::Crossed {
                class: RunClass::Native,
                ..
            }
        )));
    }

    /// 柵の中の読みは連続を伸ばしも切りもしない。柵の中の開だけでは連続を始めない。
    #[test]
    fn held_reads_neither_extend_nor_break() {
        let t0 = 10_000;
        let run0 = None;
        // 柵の中の開(最後の操作から SETTLE 未満)は数えない。
        let held = ReadFacts {
            last_operation_ms: t0 - 1,
            ..facts(ReadClass::OpenNative, t0)
        };
        let (run1, out) = step(run0, &held, SETTLE, AFTER);
        assert_eq!(out.event, Event::Held);
        assert!(run1.is_none());
        // 連続の途中の柵の中の閉も、連続を切らない(awase の操作の直後の古い読みかもしれない)。
        let (run2, _) = step(None, &facts(ReadClass::OpenNative, t0), SETTLE, AFTER);
        let held_closed = ReadFacts {
            last_operation_ms: t0 + 1_000,
            ..facts(ReadClass::Closed, t0 + 1_050)
        };
        let (run3, out) = step(run2, &held_closed, SETTLE, AFTER);
        assert_eq!(out.event, Event::Held);
        assert_eq!(run3, run2);
    }

    /// 明示 OFF の意図が無くなった・窓が変わったら、連続を捨てて Idle。
    #[test]
    fn intent_gone_or_focus_moved_resets() {
        let t0 = 10_000;
        let (run, _) = feed(&[
            (ReadClass::OpenNative, t0),
            (ReadClass::OpenNative, t0 + AFTER),
        ]);
        let gone = ReadFacts {
            explicit_off: false,
            ..facts(ReadClass::OpenNative, t0 + AFTER + 500)
        };
        let (next, out) = step(run, &gone, SETTLE, AFTER);
        assert!(next.is_none());
        assert_eq!(out.event, Event::Idle);
        assert_eq!(
            out.ended,
            Some(RunEnd {
                class: RunClass::Native,
                elapsed_ms: AFTER,
                crossed: true,
                reason: EndReason::IntentGone
            })
        );
        // 読みの途中で窓が変わった。
        let moved_mid = ReadFacts {
            focus_now: 2,
            ..facts(ReadClass::OpenNative, t0 + AFTER + 500)
        };
        let (next, out) = step(run, &moved_mid, SETTLE, AFTER);
        assert!(next.is_none());
        assert_eq!(out.ended.map(|e| e.reason), Some(EndReason::FocusMoved));
        // 連続を始めた窓と違う窓の読み。
        let other = ReadFacts {
            focus_at_start: 2,
            focus_now: 2,
            ..facts(ReadClass::OpenNative, t0 + AFTER + 500)
        };
        let (next, out) = step(run, &other, SETTLE, AFTER);
        assert!(next.is_none());
        assert_eq!(out.event, Event::Idle);
        assert_eq!(out.ended.map(|e| e.reason), Some(EndReason::FocusMoved));
    }

    /// 明示 OFF の意図が無ければ、どんな読みでも何も数えない(連続を始めない)。
    #[test]
    fn no_explicit_off_never_starts() {
        for read in [
            ReadClass::OpenNative,
            ReadClass::OpenAlnum,
            ReadClass::OpenConvUnknown,
            ReadClass::Closed,
            ReadClass::Unknown,
        ] {
            let f = ReadFacts {
                explicit_off: false,
                ..facts(read, 10_000)
            };
            let (next, out) = step(None, &f, SETTLE, AFTER);
            assert!(next.is_none());
            assert_eq!(
                out,
                Outcome {
                    event: Event::Idle,
                    ended: None
                }
            );
        }
    }

    /// 対象の窓の全列挙: 明示 OFF × 直接観測の窓 × 非 TSF ネイティブ × 有効アプリのときだけ。
    #[test]
    fn applies_only_for_explicit_off_in_direct_watch_windows() {
        for intent in [None, Some(false), Some(true)] {
            for direct in [false, true] {
                for tsf in [false, true] {
                    for disabled in [false, true] {
                        let want = intent == Some(false) && direct && !tsf && !disabled;
                        assert_eq!(
                            applies(intent, direct, tsf, disabled),
                            want,
                            "intent={intent:?} direct={direct} tsf={tsf} disabled={disabled}"
                        );
                    }
                }
            }
        }
    }
}
