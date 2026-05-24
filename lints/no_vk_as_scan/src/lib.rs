#![feature(rustc_private)]
#![warn(unused_extern_crates)]

extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_middle;
extern crate rustc_span;

use rustc_errors::DiagDecorator;
use rustc_hir::{Expr, ExprKind};
use rustc_middle::ty::{Ty, TyKind};
use rustc_span::Span;

dylint_linting::declare_late_lint! {
    /// ### What it does
    ///
    /// Detects code that extracts a raw value from `VkCode` and uses it to construct
    /// a `ScanCode`. These are entirely separate namespaces — Windows virtual key codes
    /// vs. hardware scan codes — whose numeric values coincidentally overlap for some
    /// keys (e.g., VK_CONVERT = 0x1C, Enter physical scan code = 0x1C).
    ///
    /// ### Why is this bad?
    ///
    /// Extracting the raw value from one and passing it to the other is almost certainly
    /// a bug: the number means something entirely different in each namespace.
    ///
    /// ### Example
    ///
    /// ```rust,ignore
    /// // BAD: extracts VK value and reinterprets it as a scan code
    /// let sc = ScanCode::new(u32::from(vk_code.as_u16()));
    /// ```
    ///
    /// Use instead:
    ///
    /// ```rust,ignore
    /// // GOOD: use the real scan code from the key event
    /// let sc = event.scan_code;
    /// ```
    pub NO_VK_AS_SCAN,
    Warn,
    "VkCode value used to construct a ScanCode — these are separate numeric namespaces"
}

impl<'tcx> rustc_lint::LateLintPass<'tcx> for NoVkAsScan {
    fn check_expr(&mut self, cx: &rustc_lint::LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        // Only inspect call expressions whose result type is ScanCode.
        let ExprKind::Call(func, args) = expr.kind else {
            return;
        };
        let result_ty = cx.typeck_results().expr_ty(expr);
        if !is_scan_code(cx, result_ty) {
            return;
        }

        // Warn if any argument expression contains a VkCode-typed sub-expression.
        for arg in args {
            if contains_vk_code(cx, arg) {
                emit(cx, func.span);
                return;
            }
        }
    }
}

fn emit(cx: &rustc_lint::LateContext<'_>, span: Span) {
    use rustc_lint::LintContext as _;
    cx.emit_span_lint(
        NO_VK_AS_SCAN,
        span,
        DiagDecorator(|diag| {
            diag.primary_message(
                "constructing `ScanCode` from a `VkCode` value — \
                 these are separate numeric namespaces; their values coincidentally overlap",
            );
        }),
    );
}

/// Returns `true` if the type path ends with `types::ScanCode`.
fn is_scan_code<'tcx>(cx: &rustc_lint::LateContext<'tcx>, ty: Ty<'tcx>) -> bool {
    match_type_suffix(cx, ty, "types::ScanCode")
}

/// Returns `true` if the type path ends with `types::VkCode`.
fn is_vk_code<'tcx>(cx: &rustc_lint::LateContext<'tcx>, ty: Ty<'tcx>) -> bool {
    match_type_suffix(cx, ty, "types::VkCode")
}

fn match_type_suffix<'tcx>(cx: &rustc_lint::LateContext<'tcx>, ty: Ty<'tcx>, suffix: &str) -> bool {
    if let TyKind::Adt(adt_def, _) = ty.kind() {
        cx.tcx.def_path_str(adt_def.did()).ends_with(suffix)
    } else {
        false
    }
}

/// Recursively walks an expression tree, returning `true` if any node has type `VkCode`.
fn contains_vk_code<'tcx>(cx: &rustc_lint::LateContext<'tcx>, expr: &'tcx Expr<'tcx>) -> bool {
    if is_vk_code(cx, cx.typeck_results().expr_ty(expr)) {
        return true;
    }
    match &expr.kind {
        ExprKind::Call(_, args) => args.iter().any(|a| contains_vk_code(cx, a)),
        ExprKind::MethodCall(_, receiver, args, _) => {
            contains_vk_code(cx, receiver) || args.iter().any(|a| contains_vk_code(cx, a))
        }
        ExprKind::Cast(inner, _) | ExprKind::DropTemps(inner) => contains_vk_code(cx, inner),
        ExprKind::Block(block, _) => block.expr.is_some_and(|e| contains_vk_code(cx, e)),
        _ => false,
    }
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}
