#![feature(rustc_private)]
#![warn(unused_extern_crates)]

extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_middle;
extern crate rustc_span;

use rustc_errors::DiagDecorator;
use rustc_hir::intravisit::{self, FnKind, Visitor};
use rustc_hir::{Body, Expr, ExprKind, FnDecl, QPath};
use rustc_middle::ty::{Ty, TyKind};
use rustc_span::def_id::LocalDefId;
use rustc_span::Span;

dylint_linting::declare_late_lint! {
    /// ### What it does
    ///
    /// Flags construction of `ImeEvent::InputModeObserved { source: ObservationSource::X, .. }`
    /// where `X` is a source that either (a) must never be paired with
    /// `InputModeObserved` at all (`ImmGetOpenStatus` — that source means "the
    /// ON/OFF API was called", not "input mode was observed"), or (b) is only
    /// legitimate from its one designated call site (`ConvBitsInference` — the
    /// idle-conv-check completion function that turns the (possibly async-offloaded)
    /// conv bits read into a belief update).
    ///
    /// ### Why is this bad?
    ///
    /// `InputModeObserved.source` is supposed to say *what was actually observed*.
    /// A 2026-07-05 bug disguised an internal correction as
    /// `source: ObservationSource::ImmGetOpenStatus` even though no such API call
    /// happened — this let a fabricated observation through the confidence gate as
    /// if it were a real `ImmGetOpenStatus` success. `ConvBitsInference` was
    /// introduced specifically to give conv-bit-derived guesses an honest name
    /// instead of reusing an unrelated API's label; it should stay confined to the
    /// one function that actually performs that read.
    ///
    /// If you are writing an honest, internally-generated correction (not a real
    /// observation), use `ImeEvent::InputModeApplied { strategy, result, .. }`
    /// instead — see `.claude/rules/ime-belief-architecture.md` 禁止パターン2.
    ///
    /// ### Example
    ///
    /// ```rust,ignore
    /// // BAD: no ImmGetOpenStatus call happened here; this fakes the observation.
    /// self.dispatch_event(
    ///     ImeEvent::InputModeObserved {
    ///         mode: InputModeState::AssumedRomaji { .. },
    ///         source: ObservationSource::ImmGetOpenStatus,
    ///         confidence: ObservationConfidence::High,
    ///         at: tick_ms,
    ///     },
    ///     tick_ms,
    /// );
    /// ```
    ///
    /// Use instead:
    ///
    /// ```rust,ignore
    /// // GOOD: an honest, typed record of an internal correction.
    /// self.dispatch_event(
    ///     ImeEvent::InputModeApplied {
    ///         mode: InputModeState::AssumedRomaji { .. },
    ///         strategy: InputModeApplyStrategy::PostSetOpenEisuReset,
    ///         result: InputModeApplyResult::Applied,
    ///         at: tick_ms,
    ///     },
    ///     tick_ms,
    /// );
    /// ```
    pub RESTRICTED_INPUT_MODE_OBSERVATION_SOURCE,
    Warn,
    "ImeEvent::InputModeObserved constructed with a disguised or out-of-place ObservationSource"
}

/// `(source_variant, allowed_fn_names)`. An empty allow-list means the pairing is
/// never legitimate anywhere — `InputModeObserved` never actually calls
/// `ImmGetOpenStatus` (that API reports open/close, not conversion mode), so any
/// occurrence is necessarily a disguise.
const RESTRICTED_SOURCES: &[(&str, &[&str])] = &[
    ("ImmGetOpenStatus", &[]),
    ("ConvBitsInference", &["apply_idle_conv_check"]),
    // GJI I/O 活動からの ObservedEisu 矛盾訂正。GJI I/O タイムスタンプという実観測に
    // 基づく designated 経路は Blacklist observe stage のみ（source を名乗れるのは
    // observe_gji_after_focus の結果を dispatch する箇所に限る）。
    ("GjiIoInference", &["ir_stage_observe"]),
];

fn allowed_fns_for(source_variant: &str) -> Option<&'static [&'static str]> {
    RESTRICTED_SOURCES
        .iter()
        .find(|(name, _)| *name == source_variant)
        .map(|(_, allowed)| *allowed)
}

impl<'tcx> rustc_lint::LateLintPass<'tcx> for RestrictedInputModeObservationSource {
    fn check_fn(
        &mut self,
        cx: &rustc_lint::LateContext<'tcx>,
        kind: FnKind<'tcx>,
        _decl: &'tcx FnDecl<'tcx>,
        body: &'tcx Body<'tcx>,
        _span: Span,
        _def_id: LocalDefId,
    ) {
        // クロージャ/コルーチン展開等の匿名項目は独立した「名前」を持たず、その中身は
        // 外側の名前付き関数の走査で `walk_expr` が再帰的に辿るためここでは無視する。
        // 違反は常に一番近い名前付き関数に帰属させる（ime_event_guard と同じ方針）。
        let fn_name = match kind {
            FnKind::ItemFn(ident, ..) | FnKind::Method(ident, ..) => ident.name,
            FnKind::Closure => return,
        };
        let mut finder = SourceFieldFinder {
            cx,
            fn_name: fn_name.as_str(),
        };
        finder.visit_expr(body.value);
    }
}

struct SourceFieldFinder<'a, 'tcx> {
    cx: &'a rustc_lint::LateContext<'tcx>,
    fn_name: &'a str,
}

impl<'a, 'tcx> Visitor<'tcx> for SourceFieldFinder<'a, 'tcx> {
    fn visit_expr(&mut self, expr: &'tcx Expr<'tcx>) {
        if let ExprKind::Struct(qpath, fields, _) = expr.kind {
            if last_segment_ident(*qpath).is_some_and(|s| s.as_str() == "InputModeObserved")
                && is_ime_event(self.cx, self.cx.typeck_results().expr_ty(expr))
            {
                for field in fields {
                    if field.ident.as_str() != "source" {
                        continue;
                    }
                    let Some(source_variant) = path_expr_ident(field.expr) else {
                        continue;
                    };
                    let source_variant = source_variant.as_str();
                    if let Some(allowed) = allowed_fns_for(source_variant) {
                        if !allowed.contains(&self.fn_name) {
                            emit(self.cx, field.expr.span, source_variant, self.fn_name);
                        }
                    }
                }
            }
        }
        intravisit::walk_expr(self, expr);
    }
}

fn emit(cx: &rustc_lint::LateContext<'_>, span: Span, source_variant: &str, fn_name: &str) {
    use rustc_lint::LintContext as _;
    cx.emit_span_lint(
        RESTRICTED_INPUT_MODE_OBSERVATION_SOURCE,
        span,
        DiagDecorator(move |diag| {
            diag.primary_message(format!(
                "constructing `InputModeObserved` with `source: ObservationSource::{source_variant}` \
                 outside its designated function (found in `{fn_name}`) — this source must \
                 reflect an API call that actually happened here; use `InputModeApplied` for \
                 internal corrections instead"
            ));
        }),
    );
}

/// Returns `true` if the type path ends with `ime_event::ImeEvent`.
fn is_ime_event<'tcx>(cx: &rustc_lint::LateContext<'tcx>, ty: Ty<'tcx>) -> bool {
    if let TyKind::Adt(adt_def, _) = ty.kind() {
        cx.tcx.def_path_str(adt_def.did()).ends_with("ime_event::ImeEvent")
    } else {
        false
    }
}

/// Returns the last path segment's identifier for a resolved struct-literal qpath
/// (e.g. `ImeEvent::InputModeObserved { .. }` → `"InputModeObserved"`).
fn last_segment_ident(qpath: QPath<'_>) -> Option<rustc_span::symbol::Symbol> {
    match qpath {
        QPath::Resolved(_, path) => path.segments.last().map(|s| s.ident.name),
        QPath::TypeRelative(_, segment) => Some(segment.ident.name),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// Returns the last path segment's identifier for a plain path expression
/// (e.g. the unit-variant value `ObservationSource::ConvBitsInference` →
/// `"ConvBitsInference"`).
fn path_expr_ident(expr: &Expr<'_>) -> Option<rustc_span::symbol::Symbol> {
    if let ExprKind::Path(qpath) = expr.kind {
        last_segment_ident(qpath)
    } else {
        None
    }
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}
