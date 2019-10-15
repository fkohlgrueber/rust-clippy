extern crate proc_macro;

use pattern_func::pattern_func;

pattern_func!{
    fn expr_or_semi($expr) {
        Expr($expr) | Semi($expr)
    }
}

pattern_func!{
    fn some_loop($body, $label) {
        Loop($body, $label) 
        | ForLoop(_, $body, $label) 
        | While(_, $body, $label)
    }
}