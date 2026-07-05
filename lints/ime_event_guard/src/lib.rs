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
    /// Flags construction of `ImeEvent::PanicReset` or `ImeEvent::HwndCacheRestored`
    /// outside their single designated call site (`apply_panic_reset` /
    /// `apply_hwnd_cache_restore`).
    ///
    /// ### Why is this bad?
    ///
    /// These two event variants are a narrow, deliberate escape hatch: they write
    /// `desired_open` directly without going through the confidence-gated
    /// `ObserverReported` pipeline, and without setting `last_intent` (so they don't
    /// masquerade as a real user action either). That is exactly why a prior bug
    /// used `UserImeSetIntent { source: IntentSource::Recovery }` to disguise a
    /// cache-miss heuristic guess as user intent, bypassing confidence checks
    /// entirely and leaving the IME stuck off after switching windows.
    ///
    /// `IntentSource::Recovery` / `HwndCache` were removed from `UserIntentSource`
    /// to make that specific disguise impossible to construct. `PanicReset` and
    /// `HwndCacheRestored` are the honest replacement — but they are still a
    /// direct-write escape hatch, so new call sites should be rare and deliberate,
    /// not a repeat of the same "quick fix" shortcut under a different name.
    ///
    /// ### Example
    ///
    /// ```rust,ignore
    /// // BAD: a new heuristic reset disguised as a full recovery reset
    /// fn reset_stale_thing(&mut self) {
    ///     self.dispatch_event(ImeEvent::PanicReset { target: false }, tick_ms);
    /// }
    /// ```
    ///
    /// Use instead:
    ///
    /// ```rust,ignore
    /// // GOOD: an honest, confidence-tagged observation
    /// self.dispatch_event(
    ///     ImeEvent::ObserverReported {
    ///         open: false,
    ///         source: ObservationSource::HeuristicDefault,
    ///         confidence: ObservationConfidence::Low,
    ///         ..
    ///     },
    ///     tick_ms,
    /// );
    /// ```
    pub RESTRICTED_IME_EVENT_CONSTRUCTION,
    Warn,
    "ImeEvent::PanicReset/HwndCacheRestored constructed outside its designated function"
}

const ALLOWED_FNS: &[&str] = &["apply_panic_reset", "apply_hwnd_cache_restore"];
const RESTRICTED_VARIANTS: &[&str] = &["PanicReset", "HwndCacheRestored"];

impl<'tcx> rustc_lint::LateLintPass<'tcx> for RestrictedImeEventConstruction {
    fn check_fn(
        &mut self,
        cx: &rustc_lint::LateContext<'tcx>,
        kind: FnKind<'tcx>,
        _decl: &'tcx FnDecl<'tcx>,
        body: &'tcx Body<'tcx>,
        _span: Span,
        _def_id: LocalDefId,
    ) {
        // クロージャ/コルーチン展開等の匿名項目は独立した「名前」を持たず
        // (`tcx.item_name` が ICE を起こす場合がある)、その中身は外側の
        // 名前付き関数の走査で `walk_expr` が再帰的に辿るためここでは無視する。
        // 違反は常に一番近い名前付き関数に帰属させる。
        let fn_name = match kind {
            FnKind::ItemFn(ident, ..) | FnKind::Method(ident, ..) => ident.name,
            FnKind::Closure => return,
        };
        let fn_name = fn_name.as_str();
        if ALLOWED_FNS.contains(&fn_name) {
            return;
        }
        let mut finder = RestrictedConstructionFinder { cx, fn_name };
        finder.visit_expr(body.value);
    }
}

struct RestrictedConstructionFinder<'a, 'tcx> {
    cx: &'a rustc_lint::LateContext<'tcx>,
    fn_name: &'a str,
}

impl<'a, 'tcx> Visitor<'tcx> for RestrictedConstructionFinder<'a, 'tcx> {
    fn visit_expr(&mut self, expr: &'tcx Expr<'tcx>) {
        if let ExprKind::Struct(qpath, ..) = expr.kind {
            if let Some(variant_name) = last_segment_ident(*qpath) {
                if RESTRICTED_VARIANTS.contains(&variant_name.as_str())
                    && is_ime_event(self.cx, self.cx.typeck_results().expr_ty(expr))
                {
                    emit(self.cx, expr.span, variant_name.as_str(), self.fn_name);
                }
            }
        }
        intravisit::walk_expr(self, expr);
    }
}

fn emit(cx: &rustc_lint::LateContext<'_>, span: Span, variant: &str, fn_name: &str) {
    use rustc_lint::LintContext as _;
    cx.emit_span_lint(
        RESTRICTED_IME_EVENT_CONSTRUCTION,
        span,
        DiagDecorator(move |diag| {
            diag.primary_message(format!(
                "constructing `ImeEvent::{variant}` outside its designated function \
                 (found in `{fn_name}`) — this event bypasses confidence-gated \
                 observation and writes belief state directly; new call sites should \
                 be rare and deliberate"
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
/// (e.g. `ImeEvent::PanicReset { .. }` → `"PanicReset"`).
fn last_segment_ident(qpath: QPath<'_>) -> Option<rustc_span::symbol::Symbol> {
    match qpath {
        QPath::Resolved(_, path) => path.segments.last().map(|s| s.ident.name),
        QPath::TypeRelative(_, segment) => Some(segment.ident.name),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}
