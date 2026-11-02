use super::*;

#[test]
fn a_line_is_hex_start_hex_size_then_the_symbol() {
    assert_eq!(
        format_line(0x7f00_dead_0000, 0x1a0, "logos::fib [jit]"),
        "7f00dead0000 1a0 logos::fib [jit]\n"
    );
    assert_eq!(format_line(0, 0, "x"), "0 0 x\n");
}

#[test]
fn symbols_name_the_source_function_and_the_loop_head() {
    assert_eq!(function_symbol("fib"), "logos::fib [jit]");
    assert_eq!(function_symbol(""), "logos::<anonymous> [jit]");
    assert_eq!(
        region_symbol("siftDown", 42),
        "logos::siftDown::loop@42 [jit]"
    );
    assert_eq!(region_symbol("", 7), "logos::main::loop@7 [jit]");
}

#[test]
fn the_map_lives_where_perf_reads_it() {
    assert_eq!(
        map_path(),
        std::path::PathBuf::from(format!("/tmp/perf-{}.map", std::process::id()))
    );
}
