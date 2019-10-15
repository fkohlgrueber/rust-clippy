//! Checks for continue statements in loops that are redundant.
//!
//! For example, the lint would catch
//!
//! ```rust
//! let mut a = 1;
//! let x = true;
//!
//! while a < 5 {
//!     a = 6;
//!     if x {
//!         // ...
//!     } else {
//!         continue;
//!     }
//!     println!("Hello, world");
//! }
//! ```
//!
//! And suggest something like this:
//!
//! ```rust
//! let mut a = 1;
//! let x = true;
//!
//! while a < 5 {
//!     a = 6;
//!     if x {
//!         // ...
//!         println!("Hello, world");
//!     }
//! }
//! ```
//!
//! This lint is **warn** by default.
use pattern::pattern;
use pattern_func_lib::{expr_or_semi, some_loop};
use rustc::lint::{EarlyContext, EarlyLintPass, LintArray, LintPass};
use rustc::{declare_lint_pass, declare_tool_lint};
use std::borrow::Cow;
use syntax::ast;
use syntax::source_map::{original_sp, DUMMY_SP};

use crate::utils::{snippet, snippet_block, span_help_and_lint, trim_multiline};

declare_clippy_lint! {
    /// **What it does:** The lint checks for `if`-statements appearing in loops
    /// that contain a `continue` statement in either their main blocks or their
    /// `else`-blocks, when omitting the `else`-block possibly with some
    /// rearrangement of code can make the code easier to understand.
    ///
    /// **Why is this bad?** Having explicit `else` blocks for `if` statements
    /// containing `continue` in their THEN branch adds unnecessary branching and
    /// nesting to the code. Having an else block containing just `continue` can
    /// also be better written by grouping the statements following the whole `if`
    /// statement within the THEN block and omitting the else block completely.
    ///
    /// **Known problems:** None
    ///
    /// **Example:**
    /// ```rust
    /// # fn condition() -> bool { false }
    /// # fn update_condition() {}
    /// # let x = false;
    /// while condition() {
    ///     update_condition();
    ///     if x {
    ///         // ...
    ///     } else {
    ///         continue;
    ///     }
    ///     println!("Hello, world");
    /// }
    /// ```
    ///
    /// Could be rewritten as
    ///
    /// ```rust
    /// # fn condition() -> bool { false }
    /// # fn update_condition() {}
    /// # let x = false;
    /// while condition() {
    ///     update_condition();
    ///     if x {
    ///         // ...
    ///         println!("Hello, world");
    ///     }
    /// }
    /// ```
    ///
    /// As another example, the following code
    ///
    /// ```rust
    /// # fn waiting() -> bool { false }
    /// loop {
    ///     if waiting() {
    ///         continue;
    ///     } else {
    ///         // Do something useful
    ///     }
    ///     # break;
    /// }
    /// ```
    /// Could be rewritten as
    ///
    /// ```rust
    /// # fn waiting() -> bool { false }
    /// loop {
    ///     if waiting() {
    ///         continue;
    ///     }
    ///     // Do something useful
    ///     # break;
    /// }
    /// ```
    pub NEEDLESS_CONTINUE,
    pedantic,
    "`continue` statements that can be replaced by a rearrangement of code"
}

declare_lint_pass!(NeedlessContinue => [NEEDLESS_CONTINUE]);

impl EarlyLintPass for NeedlessContinue {
    fn check_expr(&mut self, ctx: &EarlyContext<'_>, expr: &ast::Expr) {
        if !expr.span.from_expansion() { 
            check_and_warn_in_else_block(ctx, expr);
            check_and_warn_in_then_block(ctx, expr);
        }
    }
}

/* This lint has to mainly deal with two cases of needless continue
 * statements. */
// Case 1 [Continue inside else block]:
//
//     loop {
//         // region A
//         if cond {
//             // region B
//         } else {
//             continue;
//         }
//         // region C
//     }
//
// This code can better be written as follows:
//
//     loop {
//         // region A
//         if cond {
//             // region B
//             // region C
//         }
//     }
//
// Case 2 [Continue inside then block]:
//
//     loop {
//       // region A
//       if cond {
//           continue;
//           // potentially more code here.
//       } else {
//           // region B
//       }
//       // region C
//     }
//
//
// This snippet can be refactored to:
//
//     loop {
//       // region A
//       if !cond {
//           // region B
//           // region C
//       }
//     }
//

// pattern for Case 1
pattern!{
    needless_continue_pattern_1: Expr =
        some_loop(
            Block(
                _* // region A
                expr_or_semi(If(
                    _#if_cond, 
                    _#if_block, 
                    Block_(Block(expr_or_semi(Continue(_?#continue_label)) _*))#else_expr // else block that starts with `continue`
                )#if_expr)
                _*#region_c // region C
            ), 
            _?#loop_label
        )
}

// pattern for Case 1
pattern!{
    needless_continue_pattern_2: Expr =
        some_loop(
            Block(
                _* // region A
                expr_or_semi(If(
                    _#if_cond, 
                    Block(expr_or_semi(Continue(_?#continue_label)) _*)#if_block, // then block that starts with `continue`
                    _#else_expr
                )#if_expr)
                _*#region_c // region C
            ), 
            _?#loop_label
        )
}

const MSG_REDUNDANT_ELSE_BLOCK: &str = "This else block is redundant.\n";

const MSG_ELSE_BLOCK_NOT_NEEDED: &str = "There is no need for an explicit `else` block for this `if` \
                                         expression\n";

const DROP_ELSE_BLOCK_AND_MERGE_MSG: &str = "Consider dropping the else clause and merging the code that \
                                             follows (in the loop) with the if block, like so:\n";

const DROP_ELSE_BLOCK_MSG: &str = "Consider dropping the else clause, and moving out the code in the else \
                                   block, like so:\n";

fn check_and_warn_in_else_block<'a>(ctx: &EarlyContext<'_>, expr: &'a ast::Expr) {
    if let Some(res) = needless_continue_pattern_1(expr) {
        if compare_labels(res.loop_label, res.continue_label) {
            
            // build suggestion snippet
            
            let cond_code = snippet(ctx, res.if_cond.span, "..");
            let mut if_code = format!("if {} {{\n", cond_code);

            // Region B
            let block_code = &snippet(ctx, res.if_block.span, "..").into_owned();
            let block_code = erode_block(block_code);
            let block_code = trim_multiline(Cow::from(block_code), false);

            if_code.push_str(&block_code);

            // Region C
            // These is the code in the loop block that follows the if/else construction
            // we are complaining about. We want to pull all of this code into the
            // `then` block of the `if` statement.
            let to_annex = res.region_c
                .iter()
                .map(|stmt| original_sp(stmt.span, DUMMY_SP))
                .map(|span| snippet_block(ctx, span, "..").into_owned())
                .collect::<Vec<_>>()
                .join("\n");

            let mut suggest = String::from(DROP_ELSE_BLOCK_AND_MERGE_MSG);

            suggest.push_str(&if_code);
            suggest.push_str("\n// Merged code follows...");
            suggest.push_str(&to_annex);
            suggest.push_str("\n}\n");

            span_help_and_lint(ctx, 
                NEEDLESS_CONTINUE, 
                res.else_expr.span, 
                MSG_REDUNDANT_ELSE_BLOCK, 
                &suggest
            );
        }
    }
}

fn check_and_warn_in_then_block<'a>(ctx: &EarlyContext<'_>, expr: &'a ast::Expr) {
    if let Some(res) = needless_continue_pattern_2(expr) {
        if compare_labels(res.loop_label, res.continue_label) {
            
            // build suggestion snippet
            
            let cond_code = snippet(ctx, res.if_cond.span, "..");

            let if_code = format!("if {} {{\n    continue;\n}}\n", cond_code);
            /* ^^^^--- Four spaces of indentation. */
            // region B
            let else_code = snippet(ctx, res.else_expr.span, "..").into_owned();
            let else_code = erode_block(&else_code);
            let else_code = trim_multiline(Cow::from(else_code), false);

            let mut suggest = String::from(DROP_ELSE_BLOCK_MSG);
            suggest.push_str(&if_code);
            suggest.push_str(&else_code);
            suggest.push_str("\n...");

            // emit warning
            span_help_and_lint(
                ctx,
                NEEDLESS_CONTINUE, 
                res.if_expr.span,
                MSG_ELSE_BLOCK_NOT_NEEDED,
                &suggest
            );
        }
    }
}

/// If the `continue` has a label, check it matches the label of the loop.
fn compare_labels(loop_label: Option<&ast::Label>, continue_label: Option<&ast::Label>) -> bool {
    match (loop_label, continue_label) {
        // `loop { continue; }` or `'a loop { continue; }`
        (_, None) => true,
        // `loop { continue 'a; }`
        (None, _) => false,
        // `'a loop { continue 'a; }` or `'a loop { continue 'b; }`
        (Some(x), Some(y)) => x.ident == y.ident,
    }
}

/// Eats at `s` from the end till a closing brace `}` is encountered, and then
/// continues eating till a non-whitespace character is found.
/// e.g., the string
///
/// ```rust
/// {
///     let x = 5;
/// }
/// ```
///
/// is transformed to
///
/// ```ignore
///     {
///         let x = 5;
/// ```
///
/// NOTE: when there is no closing brace in `s`, `s` is _not_ preserved, i.e.,
/// an empty string will be returned in that case.
pub fn erode_from_back(s: &str) -> String {
    let mut ret = String::from(s);
    while ret.pop().map_or(false, |c| c != '}') {}
    while let Some(c) = ret.pop() {
        if !c.is_whitespace() {
            ret.push(c);
            break;
        }
    }
    ret
}

/// Eats at `s` from the front by first skipping all leading whitespace. Then,
/// any number of opening braces are eaten, followed by any number of newlines.
/// e.g.,  the string
///
/// ```ignore
///         {
///             something();
///             inside_a_block();
///         }
/// ```
///
/// is transformed to
///
/// ```ignore
///             something();
///             inside_a_block();
///         }
/// ```
pub fn erode_from_front(s: &str) -> String {
    s.chars()
        .skip_while(|c| c.is_whitespace())
        .skip_while(|c| *c == '{')
        .skip_while(|c| *c == '\n')
        .collect::<String>()
}

/// If `s` contains the code for a block, delimited by braces, this function
/// tries to get the contents of the block. If there is no closing brace
/// present,
/// an empty string is returned.
pub fn erode_block(s: &str) -> String {
    erode_from_back(&erode_from_front(s))
}
