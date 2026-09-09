#![feature(rustc_private)]
#![warn(unused_extern_crates)]

extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_span;

use rustc_errors::DiagDecorator;
use rustc_hir::intravisit::{self, FnKind, Visitor};
use rustc_hir::{Body, Expr, ExprKind, FnDecl, QPath};
use rustc_span::def_id::LocalDefId;
use rustc_span::Span;

dylint_linting::declare_late_lint! {
    /// ### What it does
    ///
    /// [ADR-161](../../docs/adr/161-single-source-spec-generation.md) D1
    /// （宣言の強制とSSOT化）の4本目のlint。指定した関数（`RESTRICTED_CALLS`）への
    /// 呼び出しが、許可リストにある関数（宣言済みの合流点）以外から行われていないかを
    /// 検知する。`.foo(...)` （メソッド呼び出し構文）と `Type::foo(...)`
    /// （完全修飾構文）の両方を同じ「foo呼び出し」として検出する——正規表現ベースの
    /// `architecture_guard.rs` が構文の違いで見落とすケースを塞ぐ狙い。
    ///
    /// ### Why is this bad?
    ///
    /// 新しい呼び出し元が無宣言で増えても、正規表現ベースのガードはテキスト
    /// パターンに一致しない構文（例: `::`経由の完全修飾呼び出し）を見落としうる。
    /// この lint は rustc の HIR を直接見るため、構文の書き方に関わらず検出できる。
    ///
    /// ### 型解決について（ADR-158 TA3の判断）
    ///
    /// `RESTRICTED_CALLS`の照合は`segment.ident.name`（呼び出し式の識別子）による
    /// **名前一致のみ**で行い、レシーバや呼び出し先の`DefId`ベースの型解決は
    /// 追加しない。これは「無関係な型・関数の同名メソッド/関数にも誤って発火し
    /// うる」という制約を持つ——実際、このリポジトリには
    /// `crates/awase-windows/tests/e2e_windows.rs`に`set_ime_open(hwnd, open)`という
    /// **無関係な自由関数**（トレイトメソッドとは無関係なe2eテストヘルパー）が
    /// 存在し、同ファイル内に18箇所の呼び出しがある。`--tests`を含めてこのlintを
    /// 実行すると、これらが誤って`RESTRICTED_ACTUATION_CALL`として検出される
    /// （2026-09-09に実測確認済み）。
    ///
    /// 型解決を追加しない理由: (1) CI（`ci.yml`）の既存の実行形は
    /// `cargo dylint --all -p awase-windows -- --target ...`であり、
    /// **`--tests`を含まない**——これは他の3本の既存lint
    /// （`ime_event_guard`等）も同様で、`--tests`を含めるとそれら既存lintも
    /// テストコード内の意図的な直接構築に対して誤検出することを2026-09-09に確認した
    /// （既存の確立した規約であり、本lintがそこから外れる理由はない）。(2) 型解決
    /// （`DefId`ベースでレシーバの型やトレイト実装を辿る）は実装コストが名前一致
    /// より大きく、対象が`set_ime_open`という単一の狭い許可リストである現時点では
    /// 過剰投資。**新しい対象を`RESTRICTED_CALLS`に追加する際は、同名の無関係な
    /// 関数が同一クレート内（特にテスト以外の場所）に存在しないか確認すること。**
    pub RESTRICTED_ACTUATION_CALL,
    Warn,
    "restricted function called from outside its designated call site"
}

/// `(呼び出し対象の関数名, 許可された呼び出し元関数名のリスト)`。
const RESTRICTED_CALLS: &[(&str, &[&str])] = &[
    // set_ime_open: ADR-090 A-1でset_ime_open_orderedへ移設済みのはずのトレイト
    // メソッド。architecture_guard.rsは`.set_ime_open(`という正規表現で「本番
    // 呼び出し0件」を主張しているが、実際にはset_ime_open_ordered自身が
    // `PlatformRuntime::set_ime_open(self, open)`という完全修飾構文で1回呼んで
    // いる（`crates/xtask-spike`のsynスキャンで実測済み）。この呼び出し元だけを
    // 許可する（ADR-158 TA2）。
    ("set_ime_open", &["set_ime_open_ordered"]),
];

fn allowed_fns_for(target: &str) -> Option<&'static [&'static str]> {
    RESTRICTED_CALLS
        .iter()
        .find(|(name, _)| *name == target)
        .map(|(_, allowed)| *allowed)
}

impl<'tcx> rustc_lint::LateLintPass<'tcx> for RestrictedActuationCall {
    fn check_fn(
        &mut self,
        cx: &rustc_lint::LateContext<'tcx>,
        kind: FnKind<'tcx>,
        _decl: &'tcx FnDecl<'tcx>,
        body: &'tcx Body<'tcx>,
        _span: Span,
        _def_id: LocalDefId,
    ) {
        let fn_name = match kind {
            FnKind::ItemFn(ident, ..) | FnKind::Method(ident, ..) => ident.name,
            FnKind::Closure => return,
        };
        let mut finder = CallFinder {
            cx,
            fn_name: fn_name.as_str(),
        };
        finder.visit_expr(body.value);
    }
}

struct CallFinder<'a, 'tcx> {
    cx: &'a rustc_lint::LateContext<'tcx>,
    fn_name: &'a str,
}

impl<'a, 'tcx> Visitor<'tcx> for CallFinder<'a, 'tcx> {
    fn visit_expr(&mut self, expr: &'tcx Expr<'tcx>) {
        let called_name = match expr.kind {
            // `.foo(...)` 形式
            ExprKind::MethodCall(segment, ..) => Some(segment.ident.name),
            // `path::to::foo(...)` 形式（完全修飾/自由関数呼び出し）
            ExprKind::Call(callee, _) => {
                if let ExprKind::Path(qpath) = callee.kind {
                    last_segment_ident(qpath)
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(name) = called_name {
            let name = name.as_str();
            if let Some(allowed) = allowed_fns_for(name) {
                if !allowed.contains(&self.fn_name) {
                    emit(self.cx, expr.span, name, self.fn_name);
                }
            }
        }
        intravisit::walk_expr(self, expr);
    }
}

fn emit(cx: &rustc_lint::LateContext<'_>, span: Span, target: &str, fn_name: &str) {
    use rustc_lint::LintContext as _;
    cx.emit_span_lint(
        RESTRICTED_ACTUATION_CALL,
        span,
        DiagDecorator(move |diag| {
            diag.primary_message(format!(
                "calling `{target}` from `{fn_name}`, which is not its designated call site \
                 — route this through the sanctioned wrapper instead"
            ));
        }),
    );
}

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
