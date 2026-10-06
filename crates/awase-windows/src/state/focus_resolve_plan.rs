//! `resolve_focus_kind` の前段（config override → キャッシュ → engine 活性）の決定（純粋）。
//!
//! `focus/kind_classifier.rs::resolve_focus_kind` に埋め込まれていた優先順位の判断を、OS・キャッシュを
//! 読まない純粋関数として切り出したもの（FCIS F5c、ADR-229）。殻は `observe`（override → キャッシュ →
//! engine 活性の順に、**前段が `None` のときだけ**次を読む。override の確認は `get_process_name` で
//! OS を読むため、読む順序と回数は元のまま）→ [`decide_resolution`] → `execute`（`NeedsClassify` の
//! ときだけワーカースレッドで `classify_focus` をタイムアウト付きで実行）になる。
//! 判断の途中で OS を読んで次の判断を決める形ではない（`NeedsClassify` は決定の結果）ので、
//! F-D1 の handler 例外は要らない。

use crate::focus::FocusKind;

/// 前段で読んだ事実。後段のフィールドは、前段がすべて `None` / `false` のときだけ殻が読む
/// （読まなかった分は `None` / `false` のまま渡され、[`decide_resolution`] は優先順位で無視する）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolveFacts {
    /// config の `force_text` / `force_bypass` の判定
    pub config_override: Option<FocusKind>,
    /// 有効期限内のキャッシュ
    pub cached: Option<FocusKind>,
    /// engine のタイマーが活性中（ユーザー打鍵中）
    pub engine_busy: bool,
}

/// 決定の理由（ログ・journal 用の文字列は [`ResolveReason::as_str`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolveReason {
    ConfigOverride,
    CacheHit,
    EngineActive,
}

impl ResolveReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ConfigOverride => "config override",
            Self::CacheHit => "cache hit",
            Self::EngineActive => "skipped (engine active)",
        }
    }
}

/// [`decide_resolution`] の決定
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// この場で確定。`overridden` は config override 由来か（キャッシュに入れない）。
    Resolved {
        kind: FocusKind,
        reason: ResolveReason,
        overridden: bool,
    },
    /// 前段では決まらない。殻が `classify_focus` を実行する。
    NeedsClassify,
}

/// override → キャッシュ → engine 活性 の優先順位で決める。OS には触れない。
pub(crate) const fn decide_resolution(facts: ResolveFacts) -> Resolution {
    if let Some(kind) = facts.config_override {
        return Resolution::Resolved {
            kind,
            reason: ResolveReason::ConfigOverride,
            overridden: true,
        };
    }
    if let Some(kind) = facts.cached {
        return Resolution::Resolved {
            kind,
            reason: ResolveReason::CacheHit,
            overridden: false,
        };
    }
    if facts.engine_busy {
        return Resolution::Resolved {
            kind: FocusKind::Undetermined,
            reason: ResolveReason::EngineActive,
            overridden: false,
        };
    }
    Resolution::NeedsClassify
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [Option<FocusKind>; 4] = [
        None,
        Some(FocusKind::TextInput),
        Some(FocusKind::NonText),
        Some(FocusKind::Undetermined),
    ];

    /// 元の if 3 連の全入力（4 × 4 × 2 = 32 通り）を、独立に書いた期待値で固定する。
    #[test]
    fn exhaustive_priority_table() {
        for o in KINDS {
            for c in KINDS {
                for busy in [false, true] {
                    let got = decide_resolution(ResolveFacts {
                        config_override: o,
                        cached: c,
                        engine_busy: busy,
                    });
                    let want = match (o, c, busy) {
                        (Some(kind), _, _) => Resolution::Resolved {
                            kind,
                            reason: ResolveReason::ConfigOverride,
                            overridden: true,
                        },
                        (None, Some(kind), _) => Resolution::Resolved {
                            kind,
                            reason: ResolveReason::CacheHit,
                            overridden: false,
                        },
                        (None, None, true) => Resolution::Resolved {
                            kind: FocusKind::Undetermined,
                            reason: ResolveReason::EngineActive,
                            overridden: false,
                        },
                        (None, None, false) => Resolution::NeedsClassify,
                    };
                    assert_eq!(got, want, "{o:?} {c:?} {busy}");
                }
            }
        }
    }

    #[test]
    fn reason_strings_match_original() {
        assert_eq!(ResolveReason::ConfigOverride.as_str(), "config override");
        assert_eq!(ResolveReason::CacheHit.as_str(), "cache hit");
        assert_eq!(
            ResolveReason::EngineActive.as_str(),
            "skipped (engine active)"
        );
    }
}
