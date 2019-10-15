extern crate proc_macro;

use pattern_func::pattern_func;

pattern_func!{
    fn expr_or_semi($expr) {
        Expr($expr) | Semi($expr)
    }
}
