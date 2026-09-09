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
    // いる（`spike/syn-xtask-prototype`ブランチの`crates/xtask-spike`のsynスキャンで
    // 実測済み。このブランチには存在しない）。この呼び出し元だけを
    // 許可する（ADR-158 TA2）。
    ("set_ime_open", &["set_ime_open_ordered"]),
    // send_input_safe（win32.rs）: ADR-159段階0の送信側主要対象。SendInput経由の
    // 唯一のチョークポイント。2026-09-09時点の実測で20箇所・19の異なる呼び出し元
    // 関数名（output/mod.rsの1関数が2箇所から呼ぶため19）を確認した（ADR-158 TB0）。
    (
        "send_input_safe",
        &[
            "inject_alt_menu_mask",
            "reinject",
            "transmit",
            "send_keymap_target",
            "post_kanji_toggle_to_focused",
            "send_ime_mode_key",
            "send_ime_mode_key_with_shift_release_prefix",
            "toggle_caps_lock",
            "send_chrome_gji_reinit_and_poll",
            "send_key",
            "send_ctrl_chord",
            "send_unicode_char",
            "send_vk_pair",
            "send_vk_run_batch",
            "flush_raw_tsf_literal_backspaces",
            "kp_restore_kana_from_half_width",
            "send_unicode_cold_warmup_keys",
            "send_eager_warmup_vk_pair",
            "send_all_modifier_key_ups",
        ],
    ),
    // send_ime_control（imm.rs）: ADR-159段階0のもう一方の送信側対象。
    // `SendMessageTimeoutW`の唯一のチョークポイント。関数名だけではactuation
    // （cmd=IMC_SETOPENSTATUS/IMC_SETCONVERSIONMODE、7件中2件のみ:
    // set_ime_open_for_target・modify_conv_mode）とprobe（cmd=IMC_GET*、残り）を
    // 区別できないため（ADR-159 TB0 MF2、dylintでの(関数,cmd)粒度は未実装）、
    // このリストは両方を含む「既知の正当な呼び出し元」全件（7つ）を宣言する運用
    // 規約で代替する——probe専用の呼び出し元がここに含まれることはSSOTの希釈だが、
    // 「無宣言の新規呼び出し元」を防ぐという本lintの主目的（RC4対策）は両方に対して
    // 変わらず機能する。2026-09-09実測。
    (
        "send_ime_control",
        &[
            "capture_imc",
            "set_ime_open_for_target",
            "get_ime_conversion_mode_for_hwnd",
            "modify_conv_mode",
            "detect_ime_open_for_hwnd",
            "detect_ime_conversion_for_hwnd",
            "read_ime_state_fast",
        ],
    ),
    // apply_ime_open_with_view: ADR-159段階0のもう1つの合流点。fix-requires-evidence.mdの
    // 「IME actuation合流点」表が挙げる4箇所（2026-09-09実測、ADR-158 TB1）。
    // `apply_ime_open_with_belief`からの内部委譲1件を含む。
    (
        "apply_ime_open_with_view",
        &[
            "dispatch_ime_set_open",
            "force_on_and_correct_romaji",
            "reassert_explicit_physical_key",
            "apply_ime_open_with_belief",
        ],
    ),
    // apply_ime_open_with_belief: 同表の2箇所（2026-09-09実測、ADR-158 TB1）。
    (
        "apply_ime_open_with_belief",
        &["kp_apply_conv_engine_sync", "ir_apply_drift_correction"],
    ),
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
        // 2026-09-09（opus code review S1で追加）: `walk_expr`は`ExprKind::Closure`の
        // パラメータ等は辿るが、本体（別の`Body`として`BodyId`経由で参照される）は
        // `Visitor::nested_filter`のデフォルト（no-op）のため辿らない。このリポジトリの
        // actuation呼び出しの多くが`spawn_local(async move { ... })`の中にあり、
        // `async move {}`もHIR上は`ExprKind::Closure`へ脱糖されるため、この穴を放置すると
        // クロージャ・asyncブロック内の呼び出しが構造的に検出対象から漏れる
        // （2026-09-09時点で実害ゼロと実測済みだが、`send_input_safe`/`send_ime_control`
        // の主要呼び出し経路がまさにこの形のため、次の追加がここに落ちる確率が高い）。
        // `nested_filter`を設定する代わりに、ここで明示的にクロージャ本体を取得して
        // 同じVisitorで再帰する（Visitorのトレイト境界を変えずに済む、最小の修正）。
        if let ExprKind::Closure(closure) = expr.kind {
            let nested_body = self.cx.tcx.hir_body(closure.body);
            self.visit_expr(nested_body.value);
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
