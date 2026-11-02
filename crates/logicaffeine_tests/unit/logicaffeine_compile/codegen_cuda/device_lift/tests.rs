use super::*;

/// The router rewrites a libc call to its device shim and copies every other byte as written: a literal or a
/// comment that spells a character past ASCII reaches the device as the same UTF-8.
#[test]
fn routing_keeps_text_past_ascii_byte_for_byte() {
    let code = "/* ties to even — one rounding */\nchar *s = malloc(8); // café\nputs(\"naïve — ok\"); char c = 'é';\n";
    let routed = route_alloc_to_shims(code);
    assert_eq!(
        routed,
        "/* ties to even — one rounding */\nchar *s = logos_malloc(8); // café\nputs(\"naïve — ok\"); char c = 'é';\n"
    );
}

/// A name inside a literal or a comment is text, not a call.
#[test]
fn routing_leaves_names_in_literals_and_comments() {
    let code = "/* malloc */ x = \"free(p)\"; // realloc\ny = calloc(1, 2);";
    assert_eq!(
        route_alloc_to_shims(code),
        "/* malloc */ x = \"free(p)\"; // realloc\ny = logos_calloc(1, 2);"
    );
}

/// An escaped quote does not end a literal, whatever follows it.
#[test]
fn routing_reads_an_escaped_quote_as_part_of_the_literal() {
    let code = "s = \"a \\\" malloc é\"; t = malloc(1);";
    assert_eq!(
        route_alloc_to_shims(code),
        "s = \"a \\\" malloc é\"; t = logos_malloc(1);"
    );
}

/// A runtime function only a macro calls reaches the device when the program names the macro, whatever comment
/// documents the macro, and a function nothing names stays off it.
#[test]
fn a_function_only_a_documented_macro_calls_is_lifted() {
    let c = format!(
        "static void *grow_storage(void *p, size_t n) {{ return realloc(p, n); }}\n\
         static long long never_named(long long x) {{ return x + 1; }}\n\
         /* Reserve room for `n` more: grows geometrically. */\n\
         #define RESERVE(s, n) ((s)->data = grow_storage((s)->data, (n)))\n\
         {USER_CODE_SENTINEL}\n\
         int main(int argc, char **argv) {{\n    RESERVE(&xs, 4);\n    return 0;\n}}\n"
    );
    let cu = lift_program_to_device(&c).expect("the program lifts");
    assert!(
        cu.contains("grow_storage(void *p, size_t n)"),
        "the macro's callee is lifted:\n{cu}"
    );
    assert!(
        !cu.contains("never_named"),
        "a function nothing names is shaken out:\n{cu}"
    );
}
