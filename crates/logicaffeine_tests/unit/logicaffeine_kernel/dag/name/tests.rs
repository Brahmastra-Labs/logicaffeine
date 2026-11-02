use super::*;

/// The spelling is the Lean importer's, which writes each component with `serde_json`: the
/// dependency-free writer agrees with it on every control character, quotes, backslashes, `/`,
/// DEL and non-ASCII text, so a name the core spells is the name the importer registered.
#[test]
fn a_component_is_spelled_exactly_as_serde_json_writes_it() {
    let mut components: Vec<String> = (0u32..0x20)
        .map(|c| format!("a{}b", char::from_u32(c).unwrap()))
        .collect();
    components.extend(
        [
            "",
            "Nat",
            "has \"quotes\"",
            "back\\slash",
            "a/b",
            "del\u{7f}",
            "α",
            "∀x",
            "😀",
            "\u{2028}\u{2029}",
            "_private",
            "123",
        ]
        .map(str::to_string),
    );
    for component in &components {
        let mut written = String::new();
        push_json_string(&mut written, component);
        assert_eq!(
            written,
            serde_json::to_string(component).unwrap(),
            "{component:?}"
        );
    }
}

/// The core's spelling and the Lean reference translator's are written independently (the
/// translator shares no code with the kernel's readers) and name every declaration the same.
#[test]
fn the_cores_spelling_is_the_reference_translators() {
    let components: Vec<String> = (0u32..0x20)
        .map(|c| format!("a{}b", char::from_u32(c).unwrap()))
        .chain(
            [
                "",
                "Nat",
                "zero",
                "succ",
                "add",
                "Quot",
                "mk",
                "lift",
                "ind",
                "sound",
                "Natural",
                "has \"quotes\"",
                "back\\slash",
                "a/b",
                "del\u{7f}",
                "α",
                "😀",
                "123",
            ]
            .map(str::to_string),
        )
        .collect();
    let paths: Vec<Vec<&str>> = components
        .iter()
        .map(|c| vec![c.as_str()])
        .chain(components.iter().map(|c| vec!["Nat", c.as_str()]))
        .chain(components.iter().map(|c| vec!["Quot", c.as_str()]))
        .chain(components.iter().map(|c| vec![c.as_str(), "x", c.as_str()]))
        .collect();
    for path in &paths {
        assert_eq!(
            kernel_name_of_components(path),
            crate::interface::lean::reference::kernel_name_of_components(path),
            "{path:?}"
        );
    }
}

/// The kernel's primitive identities: Lean's `Nat`, `Nat.zero`, `Nat.succ` and the quotient's
/// four are the kernel's own names, every other `Nat` member is spelled under `Nat/`, and anything
/// else keeps its raw hierarchical spelling.
#[test]
fn the_primitive_identities_are_the_kernels_own_names() {
    for (components, spelled) in [
        (&["Nat"][..], "Nat"),
        (&["Nat", "zero"], "Zero"),
        (&["Nat", "succ"], "Succ"),
        (&["Nat", "add"], "Nat/s:\"add\""),
        (&["Quot"], "kernel.quot.type"),
        (&["Quot", "mk"], "kernel.quot.mk"),
        (&["Quot", "lift"], "kernel.quot.lift"),
        (&["Quot", "ind"], "kernel.quot.ind"),
        (&["Quot", "sound"], "/s:\"Quot\"/s:\"sound\""),
        (&["Natural"], "/s:\"Natural\""),
        (
            &["α", "has \"quotes\""],
            "/s:\"α\"/s:\"has \\\"quotes\\\"\"",
        ),
    ] {
        assert_eq!(
            kernel_name_of_components(components),
            spelled,
            "{components:?}"
        );
    }
}
