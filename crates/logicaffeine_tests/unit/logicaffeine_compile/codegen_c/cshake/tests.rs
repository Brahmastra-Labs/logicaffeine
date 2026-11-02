use super::*;

#[test]
fn splits_functions_typedefs_globals_and_strings_with_braces() {
    let src = "\
typedef struct { int64_t *data; size_t len; } Seq_i64;
static Seq_i64 seq_new(void) { return (Seq_i64){0,0}; }
static char *braces(void) { return strdup(\"{\"); }
static int g_flag;
static void use_it(void) { seq_new(); braces(); }
";
    let units = parse_c_units(src);
    let names: Vec<&str> = units
        .iter()
        .flat_map(|u| u.defined.iter().map(|s| s.as_str()))
        .collect();
    assert!(names.contains(&"Seq_i64"), "typedef name: {names:?}");
    assert!(names.contains(&"seq_new"), "func name: {names:?}");
    assert!(names.contains(&"braces"), "brace-in-string func: {names:?}");
    assert!(names.contains(&"g_flag"), "global name: {names:?}");
    assert!(names.contains(&"use_it"), "func name: {names:?}");
    // The `"{"` string must not have swallowed the following units.
    assert_eq!(
        units.iter().filter(|u| !u.defined.is_empty()).count(),
        5,
        "unit count"
    );
}

#[test]
fn attribute_prefixed_functions_get_their_real_name() {
    // `__attribute__((unused)) static … name(…)` — the FIRST `(` is the attribute's, not the
    // parameter list's. The real function name must still be extracted (else it is unreachable).
    let src = "\
__attribute__((unused)) static char *lint_to_string(LInt a) { return 0; }
static void show_lint(LInt a) { char *s = lint_to_string(a); }
";
    let units = parse_c_units(src);
    let names: Vec<&str> = units
        .iter()
        .flat_map(|u| u.defined.iter().map(|s| s.as_str()))
        .collect();
    assert!(
        names.contains(&"lint_to_string"),
        "attribute-prefixed name: {names:?}"
    );
    assert!(
        !names.contains(&"__attribute__"),
        "must not mis-name as __attribute__: {names:?}"
    );
    // And it must be reachable from a caller.
    let roots: std::collections::BTreeSet<String> = ["show_lint".to_string()].into_iter().collect();
    let keep: BTreeSet<&str> = reachable_units(&units, &roots)
        .into_iter()
        .flat_map(|i| units[i].defined.iter().map(|s| s.as_str()))
        .collect();
    assert!(
        keep.contains("lint_to_string"),
        "attribute-prefixed callee reachable: {keep:?}"
    );
}

#[test]
fn a_struct_defined_apart_from_its_typedef_defines_its_tag() {
    // The scheduler core's shape: the typedef names an incomplete struct, and the struct is defined later.
    // Its fields must not be read as the definition (`state` is a field, not a name the unit defines).
    let src = "\
typedef struct logos_task logos_task;
struct logos_task {
    int state;
    long long resume;
};
struct logos_task *logos_make(void) { return 0; }
static int uses(logos_task *t) { return t->state; }
";
    let units = parse_c_units(src);
    let names: Vec<&str> = units
        .iter()
        .flat_map(|u| u.defined.iter().map(|s| s.as_str()))
        .collect();
    assert_eq!(
        names,
        ["logos_task", "logos_task", "logos_make", "uses"],
        "each unit's definition: {names:?}"
    );
    assert_eq!(
        units[1].kind,
        UnitKind::Typedef,
        "a struct definition is a type"
    );
    assert_eq!(
        units[2].kind,
        UnitKind::Function,
        "a function returning a struct pointer stays a function"
    );
}

#[test]
fn a_directive_continued_by_backslashes_is_one_unit() {
    // A directive ends at the first newline no backslash escapes: a macro continued over lines is one
    // preprocessor unit, and the definitions after it keep their own units and names.
    let src = "\
#define RESERVE(s, more)          \\
    do {                          \\
        (s)->cap += (more);       \\
    } while (0)
typedef struct { long cap; } Holder;
static void grow(Holder *h) { RESERVE(h, 4); }
";
    let units = parse_c_units(src);
    assert_eq!(
        units.len(),
        3,
        "the macro, the typedef and the function: {units:#?}"
    );
    assert_eq!(units[0].kind, UnitKind::Preproc);
    assert!(
        units[0].text.ends_with("    } while (0)\n"),
        "the macro carries every continued line: {:?}",
        units[0].text
    );
    let names: Vec<&str> = units
        .iter()
        .flat_map(|u| u.defined.iter().map(|s| s.as_str()))
        .collect();
    assert_eq!(
        names,
        ["Holder", "grow"],
        "each definition after the macro: {names:?}"
    );
    assert!(
        !units[1].text.contains("while"),
        "the typedef took none of the macro: {:?}",
        units[1].text
    );
}

#[test]
fn reachability_pulls_transitive_deps_only() {
    let src = "\
typedef struct { int64_t *data; } Seq_i64;
static Seq_i64 seq_new(void) { return (Seq_i64){0}; }
static void unused_fn(void) { seq_new(); }
static int64_t main_thing(void) { Seq_i64 s = seq_new(); return 0; }
";
    let units = parse_c_units(src);
    let roots: BTreeSet<String> = ["main_thing".to_string()].into_iter().collect();
    let keep: BTreeSet<&str> = reachable_units(&units, &roots)
        .into_iter()
        .flat_map(|i| units[i].defined.iter().map(|s| s.as_str()))
        .collect();
    assert!(keep.contains("main_thing"));
    assert!(keep.contains("seq_new"));
    assert!(keep.contains("Seq_i64"), "typedef dep pulled in");
    assert!(
        !keep.contains("unused_fn"),
        "unused function tree-shaken out"
    );
}

#[test]
fn a_macro_names_what_its_replacement_mentions_and_not_its_parameters() {
    let src = "\
#define RESERVE(s, more)          \\
    do {                          \\
        (s)->data = grow((s)->data, (more)); \\
    } while (0)
#define LIMIT (CAP * 2)
#ifdef LIMIT
#endif
";
    let units = parse_c_units(src);
    let macros: Vec<(String, BTreeSet<String>)> =
        units.iter().filter_map(macro_references).collect();
    let names: Vec<&str> = macros.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["RESERVE", "LIMIT"], "only the `#define`s: {units:#?}");
    let (_, reserve) = &macros[0];
    assert!(
        reserve.contains("grow") && reserve.contains("data"),
        "RESERVE reaches what it calls: {reserve:?}"
    );
    assert!(
        !reserve.contains("s") && !reserve.contains("more") && !reserve.contains("RESERVE"),
        "its parameters and its own name are not dependencies: {reserve:?}"
    );
    let (_, limit) = &macros[1];
    assert!(limit.contains("CAP"), "an object-like macro reaches its replacement: {limit:?}");
}

/// A `#define` behind the comments that document it is still a macro, and what the comments mention is not what
/// it reaches.
#[test]
fn a_documented_macro_is_a_macro() {
    let src = "\
/* Reserve room for `more` elements, growing
   geometrically; calls free() on nothing. */
#define RESERVE(s, more) ((s)->data = grow((s)->data, (more)))
// The limit, as shrink() would see it.
// Two lines of it.
#define LIMIT (CAP * 2)
/* a directive that defines nothing */
#ifdef LIMIT
#endif
";
    let units = parse_c_units(src);
    let macros: Vec<(String, BTreeSet<String>)> =
        units.iter().filter_map(macro_references).collect();
    let names: Vec<&str> = macros.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["RESERVE", "LIMIT"], "every documented `#define`: {units:#?}");
    let (_, reserve) = &macros[0];
    assert!(reserve.contains("grow"), "RESERVE reaches what it calls: {reserve:?}");
    assert!(
        !reserve.contains("free") && !reserve.contains("geometrically"),
        "a word of its comment is not a dependency: {reserve:?}"
    );
    let (_, limit) = &macros[1];
    assert!(
        limit.contains("CAP") && !limit.contains("shrink"),
        "an object-like macro reaches its replacement, not its comment: {limit:?}"
    );
}
