use super::*;

#[test]
fn renames_the_entry_and_drops_argv_globals() {
    let user = "\n\nint main(int argc, char **argv) {\n    _logos_argc = argc;\n    _logos_argv = argv;\n    show_i64((int64_t)42);\n    return 0;\n}\n";
    let out = lift_user_region(user);
    assert!(
        out.contains("__device__ int logos_main(int argc, char **argv) {"),
        "entry renamed + device"
    );
    assert!(
        !out.contains("int main(int argc, char **argv) {"),
        "no leftover device-side C main"
    );
    assert!(
        !out.contains("_logos_argc = argc;"),
        "host-only argv globals dropped"
    );
    assert!(
        out.contains("show_i64((int64_t)42);"),
        "the program body is preserved"
    );
}

#[test]
fn a_type_definition_stays_a_type() {
    let user = "typedef struct logos_frame_worker {\n    logos_frame_hdr __hdr;\n    int64_t ch;\n} logos_frame_worker;\n\
                static int logos_step_worker(void *fp);\n";
    let out = lift_user_region(user);
    assert_eq!(
        out.lines().next(),
        Some("typedef struct logos_frame_worker {"),
        "a stepped task's frame is a type: {out}"
    );
    assert!(
        !out.contains("__device__ typedef"),
        "no qualifier on a type definition: {out}"
    );
    assert!(
        out.contains("__device__ static int logos_step_worker(void *fp);"),
        "the step's declaration is a device function"
    );
}
