use super::*;

const WEB_BUILD_SCRIPT: &str = include_str!("../../../../../apps/logicaffeine_web/build.rs");

/// The byte count after `-zstack-size=` in the web client's build script.
fn linked_stack_bytes(script: &str) -> Option<usize> {
    let (_, rest) = script.split_once("-zstack-size=")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// `run_on_stack`'s wasm32 branch runs every depth the kernel admits inline on the one stack
/// `apps/logicaffeine_web/build.rs` links, so that script's `-zstack-size` must be exactly
/// `stack_bytes_for_depth(MAX_TERM_DEPTH)`: a change to `BASE_STACK_BYTES`, `BYTES_PER_RUNG` or
/// `MAX_TERM_DEPTH`, or to the script's literal, goes red here until the two agree.
#[test]
fn wasm_link_stack_matches_kernel_worst_case() {
    let worst_case = stack_bytes_for_depth(crate::type_checker::MAX_TERM_DEPTH);
    let linked = linked_stack_bytes(WEB_BUILD_SCRIPT).expect(
        "apps/logicaffeine_web/build.rs passes the linker a `-zstack-size=<bytes>` argument",
    );
    assert_eq!(
        linked, worst_case,
        "apps/logicaffeine_web/build.rs links a {linked}-byte stack; the kernel's worst case is {worst_case}"
    );
}

/// The linked stack is for the browser client only: the script reads the TARGET's architecture
/// (`CARGO_CFG_TARGET_ARCH`), not `cfg!(target_arch)`, which in a build script names the host,
/// so the native SSG server keeps the default stack and its per-call threads.
#[test]
fn wasm_link_stack_is_linked_for_wasm32_only() {
    let arg = WEB_BUILD_SCRIPT
        .find("cargo:rustc-link-arg=-zstack-size=")
        .expect("the build script emits the stack-size link argument");
    let guard = WEB_BUILD_SCRIPT[..arg]
        .rfind("CARGO_CFG_TARGET_ARCH")
        .expect("the link argument sits under a CARGO_CFG_TARGET_ARCH test");
    assert!(
        WEB_BUILD_SCRIPT[guard..arg].contains("\"wasm32\""),
        "the stack-size argument is emitted only when the target architecture is wasm32"
    );
    assert!(
        !WEB_BUILD_SCRIPT.contains("cfg!(target_arch"),
        "a build script's cfg!(target_arch) names the host, not the target"
    );
}

#[test]
fn linked_stack_bytes_reads_the_argument() {
    assert_eq!(linked_stack_bytes("x -zstack-size=4096\");"), Some(4096));
    assert_eq!(linked_stack_bytes("no argument here"), None);
    assert_eq!(linked_stack_bytes("-zstack-size=\")"), None);
}
