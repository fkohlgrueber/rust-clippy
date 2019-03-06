use crate::utils::{is_entrypoint_fn, span_lint};
use if_chain::if_chain;
use rustc::hir;
use rustc::hir::intravisit::FnKind;
use rustc::hir::{Body, Constness, FnDecl, HirId};
use rustc::lint::{in_external_macro, LateContext, LateLintPass, LintArray, LintPass};
use rustc::{declare_tool_lint, lint_array};
use rustc_mir::transform::qualify_min_const_fn::is_min_const_fn;
use syntax_pos::Span;

declare_clippy_lint! {
    /// **What it does:**
    ///
    /// Suggests the use of `const` in functions and methods where possible.
    ///
    /// **Why is this bad?**
    ///
    /// Not having the function const prevents callers of the function from being const as well.
    ///
    /// **Known problems:**
    ///
    /// Const functions are currently still being worked on, with some features only being available
    /// on nightly. This lint does not consider all edge cases currently and the suggestions may be
    /// incorrect if you are using this lint on stable.
    ///
    /// Also, the lint only runs one pass over the code. Consider these two non-const functions:
    ///
    /// ```rust
    /// fn a() -> i32 {
    ///     0
    /// }
    /// fn b() -> i32 {
    ///     a()
    /// }
    /// ```
    ///
    /// When running Clippy, the lint will only suggest to make `a` const, because `b` at this time
    /// can't be const as it calls a non-const function. Making `a` const and running Clippy again,
    /// will suggest to make `b` const, too.
    ///
    /// **Example:**
    ///
    /// ```rust
    /// fn new() -> Self {
    ///     Self { random_number: 42 }
    /// }
    /// ```
    ///
    /// Could be a const fn:
    ///
    /// ```rust
    /// const fn new() -> Self {
    ///     Self { random_number: 42 }
    /// }
    /// ```
    pub MISSING_CONST_FOR_FN,
    nursery,
    "Lint functions definitions that could be made `const fn`"
}

#[derive(Clone)]
pub struct MissingConstForFn;

impl LintPass for MissingConstForFn {
    fn get_lints(&self) -> LintArray {
        lint_array!(MISSING_CONST_FOR_FN)
    }

    fn name(&self) -> &'static str {
        "MissingConstForFn"
    }
}

impl<'a, 'tcx> LateLintPass<'a, 'tcx> for MissingConstForFn {
    fn check_fn(
        &mut self,
        cx: &LateContext<'_, '_>,
        kind: FnKind<'_>,
        _: &FnDecl,
        _: &Body,
        span: Span,
        hir_id: HirId,
    ) {
        let def_id = cx.tcx.hir().local_def_id_from_hir_id(hir_id);

        if in_external_macro(cx.tcx.sess, span) || is_entrypoint_fn(cx, def_id) {
            return;
        }

        // Perform some preliminary checks that rule out constness on the Clippy side. This way we
        // can skip the actual const check and return early.
        match kind {
            FnKind::ItemFn(_, _, header, ..) => {
                if already_const(header) {
                    return;
                }
            },
            FnKind::Method(_, sig, ..) => {
                if is_trait_method(cx, hir_id) || already_const(sig.header) {
                    return;
                }
            },
            _ => return,
        }

        let mir = cx.tcx.optimized_mir(def_id);

        if let Err((span, err)) = is_min_const_fn(cx.tcx, def_id, &mir) {
            if cx.tcx.is_min_const_fn(def_id) {
                cx.tcx.sess.span_err(span, &err);
            }
        } else {
            span_lint(cx, MISSING_CONST_FOR_FN, span, "this could be a const_fn");
        }
    }
}

fn is_trait_method(cx: &LateContext<'_, '_>, hir_id: HirId) -> bool {
    // Get the implemented trait for the current function
    let parent_impl = cx.tcx.hir().get_parent_item(hir_id);
    if_chain! {
        if parent_impl != hir::CRATE_HIR_ID;
        if let hir::Node::Item(item) = cx.tcx.hir().get_by_hir_id(parent_impl);
        if let hir::ItemKind::Impl(_, _, _, _, Some(_trait_ref), _, _) = &item.node;
        then { return true; }
    }
    false
}

// We don't have to lint on something that's already `const`
fn already_const(header: hir::FnHeader) -> bool {
    header.constness == Constness::Const
}
