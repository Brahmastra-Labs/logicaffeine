use super::*;

/// A module with a parameterless procedure, a function with parameters, and a `private` one of each —
/// the header forms every assembly pass must read the same way.
const MODULE: &str = "# Util\n\n## To private announce:\n    Show \"u\".\n\n## To private twice (n: Int) -> Int:\n    Return n times 2.\n\n## To hello:\n    Call announce.\n\n## To double (n: Int) -> Int:\n    Return twice(n).\n\n## To native private clock:\n";

#[test]
fn a_header_defines_the_function_it_names() {
    let rows: &[(&str, &str)] = &[
        ("## To hello:", "hello"),
        ("## To double (n: Int) -> Int:", "double"),
        ("## To private announce:", "announce"),
        ("## To private twice (n: Int) -> Int:", "twice"),
        ("## To native private clock:", "clock"),
        ("## To private native clock:", "clock"),
    ];
    for (header, name) in rows {
        assert_eq!(
            header_def(header),
            Some((name.to_string(), DefKind::Function)),
            "`{header}`"
        );
    }
}

#[test]
fn every_function_a_module_defines_is_found_by_its_name() {
    let names = defined_function_names(MODULE);
    let want: HashSet<String> = ["announce", "twice", "hello", "double", "clock"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(names, want);
}

#[test]
fn a_private_function_is_private_with_or_without_parameters() {
    let private = module_private_functions(MODULE);
    let want: HashSet<String> = ["announce", "twice", "clock"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(private, want);
}
