use super::*;

/// A condition loses the parentheses that enclose all of it, and only those.
#[test]
fn a_condition_sheds_only_the_parentheses_around_all_of_it() {
    assert_eq!(condition("(((n < 2LL)))"), "n < 2LL");
    assert_eq!(condition("((map_i64_i64_contains(&m, n)))"), "map_i64_i64_contains(&m, n)");
    assert_eq!(condition("(a < b) && (c < d)"), "(a < b) && (c < d)", "two groups, neither around all of it");
    assert_eq!(condition("((a < b) && (c < d))"), "(a < b) && (c < d)");
    assert_eq!(condition("f(x)"), "f(x)");
    assert_eq!(condition("(x)"), "x");
}

/// A statement expression, an assignment and a literal's parentheses keep what they need.
#[test]
fn a_condition_keeps_what_its_reading_needs() {
    assert_eq!(condition("(({ int64_t t = f(); t; }))"), "({ int64_t t = f(); t; })", "a statement expression");
    assert_eq!(condition("((x = next()))"), "(x = next())", "an assignment keeps one pair");
    assert_eq!(condition("((n += 1))"), "(n += 1)");
    assert_eq!(condition("((m <<= 2))"), "(m <<= 2)");
    assert_eq!(condition("((a == b))"), "a == b", "a comparison is no assignment");
    assert_eq!(condition("((a <= b))"), "a <= b");
    assert_eq!(condition("((a >= b) && (c != d))"), "(a >= b) && (c != d)");
    assert_eq!(condition("(str_equals(s, \")\"))"), "str_equals(s, \")\")", "a parenthesis in a string is not one");
    assert_eq!(condition("((c == ')'))"), "c == ')'", "nor one in a character");
    assert_eq!(condition("(a) + (b)"), "(a) + (b)");
}

/// A call's argument loses the parentheses that enclose all of it, unless they keep it one argument or a statement
/// expression.
#[test]
fn an_argument_sheds_only_its_own_enclosing_parentheses() {
    assert_eq!(argument("(n - 1LL)"), "n - 1LL");
    assert_eq!(argument("((n - 1LL))"), "n - 1LL");
    assert_eq!(argument("(int64_t)(n)"), "(int64_t)(n)", "a cast is not an enclosing pair");
    assert_eq!(argument("(a, b)"), "(a, b)", "a comma operator stays one argument");
    assert_eq!(argument("(f(a, b))"), "f(a, b)", "a comma inside a call is the call's");
    assert_eq!(argument("(({ int64_t t = f(); t; }))"), "({ int64_t t = f(); t; })");
    assert_eq!(argument("(str_concat(s, \",\"))"), "str_concat(s, \",\")", "a comma in a string is no comma");
    assert_eq!(argument("x"), "x");
}

/// A value in parentheses that enclose all of it is enclosed already; anything else gets a pair.
#[test]
fn a_value_is_enclosed_once() {
    assert_eq!(enclosed("(n >= 1LL)"), "(n >= 1LL)");
    assert_eq!(enclosed("n"), "(n)");
    assert_eq!(enclosed("(a) + (b)"), "((a) + (b))", "two groups are not one");
    assert_eq!(enclosed("f(x)"), "(f(x))");
    assert_eq!(enclosed("({ int64_t t = f(); t; })"), "({ int64_t t = f(); t; })");
}

/// An operand loses the parentheses C's precedence makes redundant, and keeps every pair C or gcc needs.
#[test]
fn an_operand_sheds_only_the_parentheses_c_does_not_need() {
    // Redundant: a tighter operator, or the same strength on the left.
    assert_eq!(operand("(zr * zr)", "+", false), "zr * zr");
    assert_eq!(operand("(zi * zi)", "+", true), "zi * zi");
    assert_eq!(operand("((zr * zr) + (zi * zi))", ">", false), "(zr * zr) + (zi * zi)");
    assert_eq!(operand("(a - b)", "+", false), "a - b", "left-associative on the left");
    assert_eq!(operand("(a * b)", "/", false), "a * b");
    assert_eq!(operand("(n >= 1LL)", "&&", false), "n >= 1LL");
    assert_eq!(operand("(n <= 10LL)", "&&", true), "n <= 10LL");
    assert_eq!(operand("(a && b)", "&&", true), "a && b");
    assert_eq!(operand("(a == b)", "||", false), "a == b");
    assert_eq!(operand("(f(a, b))", "+", true), "f(a, b)", "a call in parentheses of its own");
    assert_eq!(operand("((int64_t)(x))", "+", true), "(int64_t)(x)", "a cast binds tighter than any operator");
    // Needed: looser children, the right side of the same strength, and what -Wparentheses asks for.
    assert_eq!(operand("(a - b)", "+", true), "(a - b)", "c + (a - b) overflows otherwise than c + a - b");
    assert_eq!(operand("(a + b)", "*", false), "(a + b)");
    assert_eq!(operand("(a * b)", "/", true), "(a * b)");
    assert_eq!(operand("(a && b)", "||", false), "(a && b)", "-Wparentheses: && within ||");
    assert_eq!(operand("(a < b)", "==", false), "(a < b)", "-Wparentheses: comparison within comparison");
    assert_eq!(operand("(a + b)", "<<", false), "(a + b)", "-Wparentheses: arithmetic within a shift");
    assert_eq!(operand("(a == b)", "&", false), "(a == b)", "-Wparentheses: comparison within a bitwise operator");
    assert_eq!(operand("(a & b)", "==", false), "(a & b)", "a bitwise operator binds looser than equality");
    assert_eq!(operand("(a ? b : c)", "+", false), "(a ? b : c)");
    assert_eq!(operand("(x = 3)", "+", false), "(x = 3)");
    assert_eq!(operand("(a, b)", "+", false), "(a, b)");
    assert_eq!(operand("(-x)", "-", true), "(-x)", "no `a - -x`");
    assert_eq!(operand("(*p)", "*", true), "(*p)", "no `a * *p`");
    assert_eq!(operand("({ int64_t t = f(); t; })", "+", false), "({ int64_t t = f(); t; })");
    assert_eq!(operand("(a) + (b)", "*", false), "(a) + (b)", "two groups are not one");
    assert_eq!(operand("x", "+", false), "x");
    assert_eq!(operand("(str_equals(s, \" + \"))", "&&", false), "str_equals(s, \" + \")", "an operator in a string is none");
}

/// The loosest operator outside every parenthesis, read as the emitter spaces operators.
#[test]
fn the_loosest_operator_is_read_outside_every_parenthesis() {
    assert_eq!(loosest_operator("a * b + c"), Some("+"));
    assert_eq!(loosest_operator("(a + b) * c"), Some("*"));
    assert_eq!(loosest_operator("a < b && c < d"), Some("&&"));
    assert_eq!(loosest_operator("a && b || c"), Some("||"));
    assert_eq!(loosest_operator("f(a + b)"), None);
    assert_eq!(loosest_operator("-x"), None);
    assert_eq!(loosest_operator("a <= b"), Some("<="));
    assert_eq!(loosest_operator("x <<= 2"), Some("<<="));
    assert_eq!(loosest_operator("a, b"), Some(","));
}
