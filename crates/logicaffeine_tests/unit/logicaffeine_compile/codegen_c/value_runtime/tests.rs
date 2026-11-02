use super::*;

#[test]
fn a_program_links_the_runtime_exactly_when_it_declares_it() {
    assert!(links(&format!("/* prelude */\n{DECLARATION}\nint main(void) {{ return 0; }}\n")));
    assert!(!links("int main(void) { return 0; }\n"));
    assert!(!links("#define LOGOS_LINKS_VALUE_RUNTIME 0\n"));
}

#[test]
fn a_program_without_the_declaration_links_nothing_and_builds_nothing() {
    assert_eq!(link_args("int main(void) { return 0; }\n"), Ok(Vec::new()));
}

#[test]
fn the_system_libraries_are_read_from_the_note_in_order() {
    assert_eq!(
        system_libs_of("note: native-static-libs: -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc"),
        Some(["-lgcc_s", "-lutil", "-lrt", "-lpthread", "-lm", "-ldl", "-lc"].map(String::from).to_vec())
    );
    assert_eq!(system_libs_of("warning: unused variable"), None);
    assert_eq!(system_libs_of("note: native-static-libs:"), Some(Vec::new()));
}

#[test]
fn an_archive_is_named_by_its_content() {
    let a = content_name(b"one archive");
    assert_eq!(a, content_name(b"one archive"));
    assert_ne!(a, content_name(b"one archivf"));
    assert_ne!(content_name(b""), content_name(b"\0"));
    assert!(a.starts_with("logos_value_runtime-") && a.ends_with(".a"), "{a}");
}

#[test]
fn the_library_is_a_static_library_over_data_s_c_abi() {
    let m = manifest("logicaffeine-data = { path = \"/x\", features = [\"c-abi\"] }");
    assert!(m.contains("crate-type = [\"staticlib\"]"), "{m}");
    assert!(m.contains("[workspace]"), "the library must not join an enclosing workspace:\n{m}");
    assert!(m.contains("features = [\"c-abi\"]"), "{m}");
    assert!(ROOT.contains("#[global_allocator]") && ROOT.contains("abi::Counting"), "{ROOT}");
    assert!(data_dependency().contains("features = [\"c-abi\"]"), "{}", data_dependency());
}

#[test]
fn an_unchanged_source_is_not_rewritten() {
    let dir = std::env::temp_dir().join(format!("value-runtime-write-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lib.rs");
    write_if_changed(&path, "one").unwrap();
    let first = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    write_if_changed(&path, "one").unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), first, "an unchanged source was rewritten");
    write_if_changed(&path, "two").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn each_target_names_its_triple_and_keeps_its_own_system_libraries() {
    assert_eq!(Target::Host.triple(), None);
    assert_eq!(Target::Riscv64.triple(), Some("riscv64gc-unknown-linux-gnu"));
    assert_ne!(Target::Host.system_libs_file(), Target::Riscv64.system_libs_file());
}

#[test]
fn a_cross_built_archive_is_read_from_its_target_s_directory() {
    let target = std::path::Path::new("/t");
    assert_eq!(built_archive(target, Target::Host), target.join("release").join(ARCHIVE));
    assert_eq!(
        built_archive(target, Target::Riscv64),
        target.join("riscv64gc-unknown-linux-gnu").join("release").join(ARCHIVE)
    );
}

#[test]
fn a_statically_linked_target_names_the_static_unwinder_where_rustc_names_the_shared_one() {
    let note = ["-lc", "-lm", "-lgcc_s", "-lutil", "-lc"].map(String::from).to_vec();
    assert_eq!(Target::Host.link_libs(note.clone()), note, "the host links as rustc says");
    assert_eq!(
        Target::Riscv64.link_libs(note),
        ["-lc", "-lm", "-lgcc_eh", "-lutil", "-lc"].map(String::from).to_vec(),
        "a static link has no libgcc_s; its unwinder is libgcc_eh"
    );
}
