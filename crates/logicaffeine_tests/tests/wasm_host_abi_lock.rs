//! ★ LOCK — the host ABI is APPEND-ONLY, and it is a SHIM rather than a framework. ★
//!
//! LOGOS runs a web page as WebAssembly. WebAssembly cannot touch the DOM, so every DOM operation
//! must arrive through a function imported from the host — that much the platform forces. What it
//! does NOT force is for the host to make decisions: diffing, scheduling, routing and state are
//! framework behaviour, and framework behaviour belongs in compiled LOGOS where every tier can run
//! it and every ordinary lock can test it. An import is a place where that line can be crossed
//! without anyone noticing, so the whole surface is written down here.
//!
//! Three rules:
//!
//! 1. **Append-only.** A name in `FROZEN` may never disappear or be spelled differently. Every
//!    emitted module and every host that serves one — `assets/web/runtime.js`, the `MockDom` in
//!    `wasm_dom`, the `FocusDom` in `dom_reconciliation_lock` — agrees on these names by
//!    hand. Renaming one silently breaks the browser while every Rust test keeps passing, because
//!    nothing in Rust references the string.
//!
//! 2. **New imports are declared here, deliberately.** Adding one is allowed and sometimes right
//!    (`dom_apply_words` was), but it must be a decision someone wrote down, not a side effect.
//!    The count is DERIVED from `FROZEN`, never a second literal to drift — that mistake broke a
//!    full test pass the night this lock was written (`HOST_FNS: [HostFn; 188]` grew a 189th
//!    entry and the annotation did not follow), which is exactly the failure this file exists to
//!    make impossible for names.
//!
//! 3. **An import leaves only by retirement, written down.** Taking one out of the ABI is a
//!    decision like adding one: the name moves from `FROZEN` to `RETIRED` with the reason, and in
//!    the same change it leaves the compiler's table and every host. A retired name stays retired —
//!    a host that answered it again would serve a surface the language no longer has.
//!
//! ★ WHAT ADDING ONE ACTUALLY TOUCHES — written here because the two entries nothing checks are
//! the two that bite, and both fail LATE and confusingly:
//!
//!   * `module.rs` ×3 — the `host_fns!` declaration, the used-walk `note(HostFn::X)`, and the
//!     `lower_dom_call(… HostFn::X …)` arm. ★ THE LAST TWO MUST NAME THE SAME `HostFn`. A
//!     copy-paste that notes one import and lowers another is invisible to every lock in this
//!     repo and calls the wrong host function at run time.
//!   * `vm/wasm/kind.rs` — ★ AN EFFECT IMPORT MUST DECLARE `Kind::Optional` (Nothing) for its
//!     destination register. Omitted, the register gets no kind, defaults to i64, and
//!     `lower_dom_call` stores the i32 `Nothing` handle into it — the module then fails wasm
//!     VALIDATION with "type mismatch: expected i64, found i32" and nothing in Rust points at the
//!     import. `dom_set_title` hit exactly this.
//!   * The three hosts by hand: `assets/web/runtime.js` (a STRICT one-line
//!     `  name: (args) => …,` — the arity parser below cannot see a malformed entry and silently
//!     skips it), the `MockDom` in `wasm_dom`, and the `FocusDom` in
//!     `dom_reconciliation_lock`.
//!   * `semantics/builtins.rs` ×7 if the import needs a new `BuiltinId`, plus
//!     `forge_builtin_compliance_ratchet`'s `BUILTIN_TOTAL_FROZEN` (with a written
//!     classification) and `guide_coverage`'s `guide_home` — both TOTAL matches, so they fail to
//!     COMPILE until the new builtin is classified, which is the good kind of reminder.

use logicaffeine_compile::vm::wasm::host_import_names;

// Needs the `wasmi` engine, which only the `wasm-jit` feature brings in — declared unconditionally
// so a pass with no wasm simply finds nothing inside it (the pattern `wasm_host_stub_lock.rs` uses).
#[cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;

/// Every host import, sorted. Append only; never rename, never remove.
const FROZEN: &[&str] = &[
    "args",
    "dom_add_event_listener",
    "dom_append_child",
    "dom_body",
    "dom_clear_children",
    "dom_create_element",
    "dom_create_text_node",
    "dom_event_target",
    // ★ The LAST trigger, and the decision written down:
    //
    // `When a message arrives on "chat" (m: Text):` was the one sentence NO tier delivered — the
    // `undeliverable` table's only entry, waiting on "the realtime relay". That relay was already
    // built: `logicaffeine_system::relay` carries `Subscribe`/`Publish`/`Event` frames over one
    // wire protocol shared by the native server, the native client and a browser WebSocket client.
    // What was missing was the LGX end of it, which is this call.
    //
    // ★ IT IS A SHIM BY THE RULE ABOVE. The host is handed a topic and a closure; it carries
    // bytes. WHICH topic is compiled LOGOS, whether a payload matters is compiled LOGOS, and what
    // a message does to the page is a turn like any other — the reference tier reaches the same
    // page with no socket at all, which is the test that this decides nothing.
    "dom_on_message",
    // ★ THE OUTBOUND HALF, AND THE CONSTRUCT SHIPPED WITHOUT IT. `dom_on_message` subscribes; for
    // a release nothing could SEND, so a page could hear a server and never answer, and two tabs
    // of one app could not talk. The relay had this side complete the whole time
    // (`RelayFrame::Publish` fans out to local subscribers and gossips to native nodes) — what
    // was missing was a sentence and this line.
    //
    // It is a SHIM by the rule above, and deliberately only HALF the verb. A publish also reaches
    // the publishing page's own subscribers, and that delivery is compiled LOGOS — dispatched
    // through the same table `Raise` drives. So who hears a publish and when is decided in the
    // module, the reference tier reaches the same handlers with no socket at all, and this import
    // carries only the leg that genuinely leaves the page.
    "dom_publish",
    // A keyboard event's key. ★ ADDED DELIBERATELY, and this is the decision written down:
    // `on-keydown.enter` gates on WHICH key was pressed and nothing could report one —
    // `dom_event_value` is the input's value, which for a keydown is the text so far, not the
    // keystroke. So key modifiers were refused outright and "press Enter to submit", the second
    // thing anyone writes after a click, could not reach a browser at all. It is a SHIM by the
    // rule above: the host reports `KeyboardEvent.key` and decides nothing — the comparison, the
    // case-folding and the gate are all compiled LOGOS, which is why the reference tier and the
    // dispatcher can apply the same rule without the host's help.
    "dom_event_key",
    "dom_event_value",
    "dom_fetch_abort",
    "dom_fetch_send",
    "dom_fetch_set_timeout",
    "dom_fetch_start",
    "dom_get_text",
    "dom_get_value",
    "dom_navigate",
    "dom_on_route",
    // ★ Page-level command. `docs/web.md` teaches `Set the title to "Todos".` by name, so the
    // sentence was documented and unbuildable — the browser refused the whole component. A
    // title is a single concrete write to one page property, not a decision the host takes.
    "dom_set_title",
    // A `## Page`'s `Head:` block, rendered to its `<title>`/`<meta>`/`<link>`/`<base>` markup and
    // handed to the host once at mount. MERGING it over the default head set (charset, viewport,
    // theme-color, favicon) by tag identity is the same rule `web_host.rs`'s SSR path already
    // applies — compiled once, in Rust, and ported to `runtime.js` by hand — so this decides
    // nothing the reference tier does not; it is the one write only the browser's own
    // `document.head` can make.
    "dom_set_head_html",
    // `Reload the page.` — the reference tier has always run it; a browser needs the one call
    // that can actually do it. No decision, no state, no arguments.
    "dom_reload",
    // `Go back.` — the one call that can step the browser's own history. The popstate it fires
    // is already handled by the router, so this adds a command and no new listener.
    "dom_go_back",
    // The READ half of the attribute pair. Chosen over `dom_add_class`/`dom_remove_class` on
    // purpose: with a read, deciding which class words survive is compiled LOGOS that every
    // tier runs; with those two, that rule would live in a .js file only a browser executes.
    "dom_get_attribute",
    // Going offline, coming back online, the window resizing: one import for all three, because
    // they differ only in the event's name. They fire on `window` and do not bubble, so this
    // cannot be served by the element listener that already exists.
    "dom_on_window_event",
    // The one geometry reading `When the user scrolls to the bottom:` needs. A READ: whether the
    // gap counts as the bottom is decided in compiled LOGOS, where every tier runs it.
    "dom_scroll_gap",
    // ONE interval for the page, not one per timer: the browser says how much time passed and
    // compiled LOGOS decides which timers that makes due, so every tier runs the same rule.
    "dom_set_interval",
    // The one read `When a file is dropped on …:` needs. Digging a name out of a transfer is a
    // reading; what the page does about the file is compiled LOGOS.
    "dom_event_file_name",
    // The rest of the `File` record a drop binds (`{name, size, type, bytes}`, S17): its size and
    // its MIME type, read off the same transfer. Readings again — a real size, never a literal,
    // is the whole reason they are imports and not constants.
    "dom_event_file_size",
    "dom_event_file_type",
    // The record's `bytes`: the first import that answers with a LIST. The host builds it through
    // the module's own `__logos_seq_new` (see `a_host_built_list_is_an_ordinary_sequence_the_module_
    // reads`), so the one mechanism by which a host hands a module a list is written down once.
    "dom_event_file_bytes",
    // `Focus`, `Blur`, `Hover`, `Scroll to`, `Select the text in` — ONE import for the family.
    // They differ only in which single DOM call is made on one node; five imports would have
    // said the same thing five times, and an ABI is where saying something twice is how it drifts.
    "dom_element_command",
    // ★ THREE, NOT ONE. Unlike the element commands — which differ only in which single method is
    // called on the same node — these are three CAPABILITIES with three permission models and
    // three ways to be denied. One `dom_page_command` would name three decisions with one word.
    "dom_clipboard_write",
    "dom_download_file",
    "dom_notify",
    "dom_query_selector",
    "dom_replace_route",
    "dom_response_body",
    "dom_response_error",
    "dom_response_header",
    "dom_response_status",
    "dom_response_text",
    "dom_route_path",
    "dom_apply",
    "dom_prevent_default",
    "dom_route_query",
    "dom_set_attribute",
    "dom_set_bool_attribute",
    "dom_set_checked",
    "dom_set_html",
    "dom_set_text",
    "dom_set_value",
    // Browser storage. Added deliberately: WebAssembly cannot reach `localStorage` itself, and
    // without these a `## Store … persisted as "key"` kept its state in the module's own memory
    // — which dies with the page, while the sentence said otherwise. The area is an ARGUMENT so
    // a third area later widens no boundary; `storage_get` answers handle 0 for an absent key,
    // which is how a page tells "nothing is saved" from "an empty string is saved".
    "storage_get",
    "storage_remove",
    "storage_set",
    "fmt_align_into",
    "fmt_bool_into",
    "fmt_f64_into",
    "fmt_f64_prec_into",
    "fmt_i64_into",
    "fmt_seq_bool_into",
    "fmt_seq_i64_into",
    "fmt_set_i64_into",
    "logos_rt_alloc",
    "logos_rt_bigint_add",
    "logos_rt_bigint_div",
    "logos_rt_bigint_from_i64",
    "logos_rt_bigint_mod",
    "logos_rt_bigint_mul",
    "logos_rt_bigint_pow",
    "logos_rt_bigint_sub",
    "logos_rt_bigint_to_text",
    "logos_rt_complex_abs",
    "logos_rt_complex_add",
    "logos_rt_complex_from_f64",
    "logos_rt_complex_from_i64",
    "logos_rt_complex_im_f64",
    "logos_rt_complex_mul",
    "logos_rt_complex_re_f64",
    "logos_rt_complex_sub",
    "logos_rt_complex_to_text",
    "logos_rt_date_add_span",
    "logos_rt_decimal_add",
    "logos_rt_decimal_from_i64",
    "logos_rt_decimal_from_text",
    "logos_rt_decimal_mul",
    "logos_rt_decimal_sub",
    "logos_rt_decimal_to_rational",
    "logos_rt_decimal_to_text",
    "logos_rt_dynamic_to_text",
    "logos_rt_fail",
    "logos_rt_fail_at",
    "logos_rt_format_timestamp",
    "logos_rt_in_zone",
    "logos_rt_lanes16_from_bytes",
    "logos_rt_lanes4_add",
    "logos_rt_lanes4_xor",
    "logos_rt_lanes4w64_from_words",
    "logos_rt_lanes8_from_words",
    "logos_rt_lanes_byte_add",
    "logos_rt_lanes_interleave_hi",
    "logos_rt_lanes_interleave_lo",
    "logos_rt_lanes_maddubs",
    "logos_rt_lanes_packus",
    "logos_rt_lanes_shr_bytes",
    "logos_rt_lanes_shuffle",
    "logos_rt_lanes_splat16",
    "logos_rt_lanes_splat8",
    "logos_rt_lanes_to_seq",
    "logos_rt_local_instant",
    "logos_rt_make_date",
    "logos_rt_make_moment",
    "logos_rt_modular_add",
    "logos_rt_modular_div",
    "logos_rt_modular_from_i64",
    "logos_rt_modular_mul",
    "logos_rt_modular_pow",
    "logos_rt_modular_sub",
    "logos_rt_modular_to_text",
    "logos_rt_moment_add_span",
    "logos_rt_money_add",
    "logos_rt_money_from_decimal",
    "logos_rt_money_from_i64",
    "logos_rt_money_sub",
    "logos_rt_money_to_text",
    "logos_rt_months_between",
    "logos_rt_quantity_add",
    "logos_rt_quantity_convert",
    "logos_rt_quantity_div",
    "logos_rt_quantity_mul",
    "logos_rt_quantity_of_i64",
    "logos_rt_quantity_sub",
    "logos_rt_quantity_to_text",
    "logos_rt_rational_abs",
    "logos_rt_rational_add",
    "logos_rt_rational_as_i64",
    "logos_rt_rational_bitwise",
    "logos_rt_rational_ceil",
    "logos_rt_rational_cmp",
    "logos_rt_rational_cmp_f64",
    "logos_rt_rational_div",
    "logos_rt_rational_floor",
    "logos_rt_rational_floor_quotient",
    "logos_rt_rational_from_bigint",
    "logos_rt_rational_from_i64",
    "logos_rt_rational_mul",
    "logos_rt_rational_power",
    "logos_rt_rational_quotient",
    "logos_rt_rational_remainder",
    "logos_rt_rational_round",
    "logos_rt_rational_sub",
    "logos_rt_rational_to_f64",
    "logos_rt_rational_to_text",
    "logos_rt_read_wire_program",
    "logos_rt_run_accepted",
    "logos_rt_set_rate",
    "logos_rt_set_rates_decimal",
    "logos_rt_set_rates_int",
    "logos_rt_set_rates_rational",
    "logos_rt_sha1msg1",
    "logos_rt_sha1msg2",
    "logos_rt_sha1nexte",
    "logos_rt_sha1rnds4",
    "logos_rt_to_currency",
    "logos_rt_uuid_dns",
    "logos_rt_uuid_eq",
    "logos_rt_uuid_from_ptr",
    "logos_rt_uuid_max",
    "logos_rt_uuid_nil",
    "logos_rt_uuid_oid",
    "logos_rt_uuid_parse",
    "logos_rt_uuid_to_text",
    "logos_rt_uuid_url",
    "logos_rt_uuid_version",
    "logos_rt_uuid_x500",
    "logos_rt_value_abs",
    "logos_rt_value_add",
    "logos_rt_value_apply",
    "logos_rt_value_approx_eq",
    "logos_rt_value_bit_and",
    "logos_rt_value_bit_or",
    "logos_rt_value_bit_xor",
    "logos_rt_value_bool",
    "logos_rt_value_ceil",
    "logos_rt_value_checked",
    "logos_rt_value_clone",
    "logos_rt_value_contains_i64",
    "logos_rt_value_date",
    "logos_rt_value_define",
    "logos_rt_value_div",
    "logos_rt_value_eq",
    "logos_rt_value_exact_div",
    "logos_rt_value_float",
    "logos_rt_value_floor",
    "logos_rt_value_floor_div",
    "logos_rt_value_ge",
    "logos_rt_value_gt",
    "logos_rt_value_int",
    "logos_rt_value_integer_at",
    "logos_rt_value_item",
    "logos_rt_value_item_at",
    "logos_rt_value_items",
    "logos_rt_value_le",
    "logos_rt_value_length_i64",
    "logos_rt_value_list",
    "logos_rt_value_lt",
    "logos_rt_value_max",
    "logos_rt_value_min",
    "logos_rt_value_moment",
    "logos_rt_value_mul",
    "logos_rt_value_ne",
    "logos_rt_value_not",
    "logos_rt_value_nothing",
    "logos_rt_value_of_bigint",
    "logos_rt_value_of_decimal",
    "logos_rt_value_of_rational",
    "logos_rt_value_of_text",
    "logos_rt_value_pop",
    "logos_rt_value_pow",
    "logos_rt_value_power",
    "logos_rt_value_push",
    "logos_rt_value_rem",
    "logos_rt_value_round",
    "logos_rt_value_seq_concat",
    "logos_rt_value_set_item",
    "logos_rt_value_shl",
    "logos_rt_value_shr",
    "logos_rt_value_slice",
    "logos_rt_value_span",
    "logos_rt_value_sqrt",
    "logos_rt_value_sub",
    "logos_rt_value_time",
    "logos_rt_value_to_i64",
    "logos_rt_value_to_text",
    "logos_rt_value_truth_i32",
    "logos_rt_value_tuple_of",
    "logos_rt_wire_bytes_bool",
    "logos_rt_wire_bytes_float",
    "logos_rt_wire_bytes_int",
    "logos_rt_wire_bytes_text",
    "logos_rt_years_between",
    "now",
    "obs",
    "parse_float",
    "parse_int",
    "parse_timestamp",
    "pow_ff",
    "pow_fi",
    "print_bool",
    "print_char",
    "print_date",
    "print_duration",
    "print_f64",
    "print_i64",
    "print_moment",
    "print_nothing",
    "print_rational",
    "print_seq_bool",
    "print_seq_f64",
    "print_seq_i64",
    "print_seq_text",
    "print_seq_word32",
    "print_seq_word64",
    "print_set_i64",
    "print_set_text",
    "print_span",
    "print_text",
    "print_time",
    "print_word",
    "random_int",
    "read_reals",
    "read_wire_frame",
    "temporal_component",
    "temporal_component_date",
    "text_contains",
    "today",
    "write_wire_residual",
    // `noteTurn(name, key, changes, failure)` — a lowered turn's journal entry: the action, its
    // instance key, the state it changed and the failure it stopped with. A browser keeps nothing; a
    // recording host (the Chrome recorder, the in-process hosts) reads all four.
    "dom_note_turn",
    // Whether this host records the journal's detail — true under a recording harness, false in a
    // shipped page — so a lowered turn captures before/after state only where someone reads it.
    "dom_journal_detail",
    // `.stop`/`.once`/`.capture`/`.passive`: a browser fixes a listener's phase and passivity when
    // `addEventListener` runs, so the modifier bits cross as DATA (`semantics::dom::LISTEN_*`) and
    // the host attaches with them. Which modifiers a handler carries stays compiled LOGOS.
    "dom_listen_flags",
    // `Navigate to` a path outside this module's own routes: another DOCUMENT loads
    // (`location.assign`), which no in-module router can do. The destination is the page's choice.
    "dom_assign_location",
    // `vdom.lg`'s whole patch stream in one call — the word-format twin of `dom_apply`: every
    // decision was made by the program that emitted it, and the host applies it by the ids it names.
    "dom_apply_words",
    // The page keeps its own clock and decides every timer in compiled LOGOS; the host is asked only
    // to call `__lgx_tick` when the earliest one comes due (or not at all, for a negative delay).
    "dom_wake_after",
    // A program's own `Type`/`Choose`: the browser's default action of entering the value into the
    // control, answering whether it took it. Which events follow, and who hears them, stays the
    // compiled dispatcher's.
    "dom_enter_value",
    // A driving verb the program runs on its own page: the host enters the page's dispatcher as an
    // entry of its own and contains a failure there, as it contains a visitor's. Which handler hears
    // the event, and what the failure door does, stays compiled LOGOS.
    "dom_page_entry",
    // ★ Exact integers, ADDED DELIBERATELY (Tristen 10-02: "Exact ints in direct WASM"). A LOGOS `Int`
    // is unbounded; where one can outgrow a machine word the module holds a TAGGED WORD — the integer
    // itself while it fits beside the tag, a handle to an integer the host holds when it does not. The
    // host does arithmetic on the integers it holds and nothing else: which integers are exact, every
    // operation on two small ones, the overflow checks and when a held one is cloned or given back are
    // all decided in the module. One family, one shape each: the arithmetic the reference tier does,
    // the narrowing to a word, ownership, and display.
    "exact_add",
    "exact_box",
    "exact_clone",
    "exact_cmp",
    "exact_div",
    "exact_drop",
    "exact_f64",
    "exact_floordiv",
    "exact_mod",
    "exact_mul",
    "exact_narrow",
    "exact_pow",
    "exact_sub",
    "exact_text",
    "print_exact",
    // ★ The process environment and the millisecond clock, ADDED DELIBERATELY (10-06): env.lg's
    // `get(name)` and time.lg's `now()` answered nothing on every tier but the Rust one. The VM
    // expands `get` into whether the variable is set and its text, then builds the `Option` as any
    // other, so the host answers two plain questions and decides nothing: the variable's length (`-1`
    // when unset) and its bytes, written into a buffer the module allocated. A browser has no
    // environment, so there every variable is unset.
    "env_len",
    "env_into",
    "time_now",
];

/// Every host import taken out of the ABI, with the decision that retired it (rule 3).
const RETIRED: &[(&str, &str)] = &[
    (
        "dom_begin_paint",
        "the host-side staging pass it opened is gone: a page reconciles in `vdom.lg` and hands the \
         host one committed stream per paint (`dom_apply_words`), so a paint's boundary is the commit",
    ),
    (
        "dom_move_children",
        "a `{children}` slot's nodes are a range of the retained tree's tape, which the commit places \
         before it emits the stream, so no host reparents a holder's children",
    ),
];

/// The names the compiler can actually emit, sorted and deduplicated.
fn live() -> Vec<String> {
    let mut v: Vec<String> = host_import_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    v.sort();
    v.dedup();
    v
}

/// ★ EVERY DOM IMPORT THE COMPILER CAN EMIT IS IMPLEMENTED BY THE BROWSER HOST.
///
/// A wasm module importing a function its host does not define does not fail to compile and does
/// not fail gracefully — it fails to INSTANTIATE, so the page never loads, with no error anywhere
/// near the feature that caused it. `dom_prevent_default` was declared in the ABI, emitted by the
/// `.prevent` lowering, and implemented by NOBODY: a page with `on-submit.prevent` — the modifier
/// that stops a handled form posting itself twice — was a blank page in every real browser.
///
/// The check is deliberately narrow to the `dom_`/`storage_` surface `runtime.js` owns: the
/// numeric, text and wire imports are the compiled runtime's own and are not the browser shim's
/// job. Narrow and enforced beats broad and excused.
#[test]
fn the_browser_runtime_implements_every_dom_import() {
    const RUNTIME_JS: &str = include_str!("../../logicaffeine_compile/assets/web/runtime.js");
    let missing: Vec<String> = live()
        .into_iter()
        .filter(|n| n.starts_with("dom_") || n.starts_with("storage_"))
        // The shim declares each import as an object property: `name: (args) => …`.
        .filter(|n| !RUNTIME_JS.contains(&format!("{n}:")))
        .collect();
    assert!(
        missing.is_empty(),
        "these imports are emitted by the compiler and implemented by no host, so any page \
         reaching them fails to INSTANTIATE: {missing:?}"
    );
}

#[test]
fn no_host_import_is_ever_renamed_or_removed() {
    let live = live();
    let missing: Vec<&&str> = FROZEN
        .iter()
        .filter(|f| !live.contains(&f.to_string()))
        .collect();
    assert!(
        missing.is_empty(),
        "host import(s) {missing:?} disappeared or were renamed. Every host that serves an emitted \
         module — assets/web/runtime.js, the MockDom in wasm_dom, the FocusDom in \
         dom_reconciliation_lock — names these by HAND, so nothing in Rust goes red when one \
         drifts and the page simply stops working. Restore the name, or rename it in every host \
         and here in the same change — or retire it: move it to RETIRED with the decision."
    );
}

/// The retired names a host still answers: `runtime.js`'s and the node shim's own import keys.
fn retired_names_answered(hosts: &[(&str, &str)]) -> Vec<String> {
    let mut found = Vec::new();
    for (host, source) in hosts {
        let keys = host_arities_of(source);
        for (name, _) in RETIRED {
            if keys.contains_key(*name) {
                found.push(format!("{name}: answered by {host}"));
            }
        }
    }
    found
}

#[test]
fn a_retired_import_is_gone_from_the_table_and_every_host() {
    let live = live();
    for (name, why) in RETIRED {
        assert!(
            !why.is_empty(),
            "`{name}` is retired with its reason written down"
        );
        assert!(
            !FROZEN.contains(name),
            "`{name}` is retired and frozen at once"
        );
        assert!(
            !live.contains(&name.to_string()),
            "the compiler still emits the retired import `{name}` ({why})"
        );
    }
    let shim = logicaffeine_compile::web_host::wasm_host_shim("x");
    let answered = retired_names_answered(&[("runtime.js", RUNTIME_JS), ("the node shim", &shim)]);
    assert!(
        answered.is_empty(),
        "a host still answers a retired import:\n{}",
        answered.join("\n")
    );
}

#[test]
fn the_retired_census_sees_a_host_that_answers_a_retired_name() {
    let planted = "const env = {\n  dom_begin_paint: () => {},\n  dom_body: () => 0n,\n};\n";
    assert_eq!(
        retired_names_answered(&[("planted", planted)]),
        ["dom_begin_paint: answered by planted"],
        "the census reads a host's import keys"
    );
}

/// The names a Rust test host links into `env`: each `func_wrap("env", "<name>", …)` in `source`,
/// on one line or across several.
fn env_links(source: &str) -> Vec<&str> {
    source
        .split("\"env\",")
        .skip(1)
        .filter_map(|rest| rest.trim_start().strip_prefix('"')?.split('"').next())
        .collect()
}

/// The retired names a Rust test host still links.
fn retired_names_linked(host: &str, source: &str) -> Vec<String> {
    let linked = env_links(source);
    RETIRED
        .iter()
        .filter(|(name, _)| linked.contains(name))
        .map(|(name, _)| format!("{name}: linked by {host}"))
        .collect()
}

/// Every test source under `tests/` that links host imports into a wasmi `Linker`, but this one.
fn test_hosts() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, String)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("the tests directory")
            .flatten()
            .collect();
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|x| x == "rs")
                && path
                    .file_name()
                    .is_some_and(|n| n != "wasm_host_abi_lock.rs")
            {
                let source = std::fs::read_to_string(&path).expect("a test source");
                if !env_links(&source).is_empty() {
                    out.push((
                        path.strip_prefix(root).unwrap().display().to_string(),
                        source,
                    ));
                }
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out
}

#[test]
fn no_test_host_links_a_retired_import() {
    // The in-process hosts model the browser's `env` by hand (`MockDom`, `FocusDom`, the gauntlet's
    // recording host, `compile_common::wasm_host`). A retired name one of them still links models a
    // surface the language no longer has, which no module can call.
    let hosts = test_hosts();
    let paths: Vec<&str> = hosts.iter().map(|(p, _)| p.as_str()).collect();
    for expected in [
        "wasm_dom.rs",
        "dom_reconciliation_lock.rs",
        "compile_common/wasm_host.rs",
    ] {
        assert!(
            paths.contains(&expected),
            "the census reads {expected}: {paths:?}"
        );
    }
    let linked: Vec<String> = hosts
        .iter()
        .flat_map(|(host, source)| retired_names_linked(host, source))
        .collect();
    assert!(
        linked.is_empty(),
        "a test host still links a retired import:\n{}",
        linked.join("\n")
    );
}

#[test]
fn the_test_host_census_sees_a_retired_link() {
    let one_line = "l.func_wrap(\"env\", \"dom_begin_paint\", |_c: C<'_>| {}).unwrap();\n";
    assert_eq!(
        retired_names_linked("planted", one_line),
        ["dom_begin_paint: linked by planted"]
    );
    let wrapped = "l.func_wrap(\n    \"env\",\n    \"dom_move_children\",\n    |_c: C<'_>, _f: i64, _t: i64| {},\n)\n";
    assert_eq!(
        retired_names_linked("planted", wrapped),
        ["dom_move_children: linked by planted"]
    );
    assert_eq!(
        env_links("l.func_wrap(\"env\", \"dom_body\", f)"),
        ["dom_body"],
        "a live link is read, not flagged"
    );
    assert!(retired_names_linked("planted", "l.func_wrap(\"env\", \"dom_body\", f)").is_empty());
}

#[test]
fn the_dom_surface_is_a_shim_and_not_a_framework() {
    // Every DOM import must be a single concrete operation on ONE node — create it, set something
    // on it, put it somewhere, read it back, listen on it, or mark a paint. None of them may take
    // a decision the framework should be taking. The test is deliberately coarse: it cannot tell
    // a good implementation from a bad one, but it CAN tell when a new import's NAME announces
    // that JavaScript has started doing the thinking.
    const DECIDING: &[&str] = &[
        "diff",
        "reconcile",
        "patch",
        "render",
        "schedule",
        "settle",
        "component",
        "signal_",
        "effect",
        "watch",
        "commit",
        "vdom",
        "keyed",
    ];
    let offenders: Vec<String> = live()
        .into_iter()
        .filter(|n| n.starts_with("dom_"))
        .filter(|n| DECIDING.iter().any(|d| n.contains(d)))
        .collect();
    assert!(
        offenders.is_empty(),
        "host import(s) {offenders:?} name an act of DECIDING, not an act of doing. LOGOS runs the \
         page as WebAssembly; the host's job is to perform DOM operations it is told to perform. \
         Diffing, scheduling and reconciliation are framework logic and belong in compiled LOGOS, \
         where every tier runs them and every ordinary lock tests them — not in a .js file that \
         only a browser executes and only a node-driven test ever sees."
    );
}

#[test]
fn every_live_import_is_written_down() {
    let live = live();
    let undeclared: Vec<&String> = live
        .iter()
        .filter(|n| !FROZEN.contains(&n.as_str()))
        .collect();
    assert!(
        undeclared.is_empty(),
        "host import(s) {undeclared:?} exist but are not in FROZEN. Adding an import is allowed \
         and sometimes right, but it widens the LOGOS↔host boundary and has to be a decision \
         someone wrote down. Add the name to FROZEN — and implement it in EVERY host: \
         assets/web/runtime.js, the MockDom in wasm_dom, and the FocusDom in \
         dom_reconciliation_lock. A module that imports a function its host does not define fails \
         to instantiate, with no compile error anywhere."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ THE OTHER HALF OF THE ABI: THE SHAPE.
//
// `FROZEN` above pins the NAMES, and a rename is the loud half — nothing in Rust
// references the string, so a rename breaks the browser while every Rust test
// stays green, which is exactly why that list exists.
//
// The quiet half is arity. How many arguments an import takes was written down in
// the emitted module and, separately, by hand in `runtime.js` — and NO TEST READ
// EITHER. Change one host's parameters and the two disagree; WebAssembly then
// refuses to instantiate the module, in the browser, with no diagnostic that
// points anywhere near the edit. There is no compile error anywhere for this.
//
// So: every import the module declares must be implemented by `runtime.js` with
// exactly the parameters the module says it has.
// ─────────────────────────────────────────────────────────────────────────────

const RUNTIME_JS: &str = include_str!("../../logicaffeine_compile/assets/web/runtime.js");

/// `  name: (a, b, c) => …` — the shape every host function in `runtime.js` (and the node shim,
/// which is written in the same style) is written in. Returns (name, parameter count) for each —
/// reading the actual OBJECT KEY, unlike a raw substring search, which a name mentioned in an
/// error message or a doc comment elsewhere in the file can satisfy without the host implementing
/// anything (`temporal_component`'s own "unknown component selector" message is exactly such a
/// string; a plain `.contains("temporal_component:")` would have called that an implementation).
fn host_arities_of(js: &str) -> std::collections::BTreeMap<String, usize> {
    let mut out = std::collections::BTreeMap::new();
    for line in js.lines() {
        let body = line.trim_start();
        if body.len() == line.len() {
            continue; // top-level: not a member of the host object
        }
        let Some((name, rest)) = body.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            continue;
        }
        let rest = rest.trim_start();
        let Some(args) = rest.strip_prefix('(').and_then(|r| r.split_once(')')) else {
            continue;
        };
        if !args.1.trim_start().starts_with("=>") {
            continue;
        }
        let n = args.0.split(',').filter(|a| !a.trim().is_empty()).count();
        out.insert(name.to_string(), n);
    }
    out
}

#[test]
fn every_dom_import_is_implemented_by_the_browser_host() {
    let js = host_arities_of(RUNTIME_JS);
    let missing: Vec<&str> = logicaffeine_compile::vm::wasm::host_import_signatures()
        .into_iter()
        .map(|(n, _, _)| n)
        .filter(|n| n.starts_with("dom_") && !js.contains_key(*n))
        .collect();
    assert!(
        missing.is_empty(),
        "these DOM imports are declared by the emitted module and implemented by NOTHING in \
         runtime.js, so any page that reaches one fails to instantiate in the browser: {missing:?}"
    );
}

#[test]
fn the_module_and_the_browser_host_agree_on_arity() {
    let js = host_arities_of(RUNTIME_JS);
    let mut wrong = Vec::new();
    for (name, params, _) in logicaffeine_compile::vm::wasm::host_import_signatures() {
        if let Some(&n) = js.get(name) {
            if n != params.len() {
                wrong.push(format!(
                    "  {name}: the module imports it with {} parameter(s), runtime.js implements \
                     it with {n}",
                    params.len()
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "the emitted module and its browser host disagree about how to call these — WebAssembly \
         refuses to instantiate on a signature mismatch, so this is a blank page, not a bad \
         result:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn the_arity_check_can_actually_fail() {
    // ★ PROVE THE TEETH. A parity test whose two sides are both derived from the same
    // place is the failure mode this campaign keeps finding: a check that cannot go red.
    // Both sides here are read independently — the module's own table, and the text of
    // runtime.js — so perturbing either must be visible. This asserts that it is.
    let js = host_arities_of(RUNTIME_JS);
    assert!(
        js.len() >= 60,
        "parsed only {} host fns out of runtime.js — the parser has \
        stopped seeing the object, so the arity test above would pass vacuously",
        js.len()
    );
    let sigs = logicaffeine_compile::vm::wasm::host_import_signatures();
    let overlap = sigs.iter().filter(|(n, _, _)| js.contains_key(*n)).count();
    assert!(
        overlap >= 60,
        "only {overlap} imports are checked against runtime.js — the two sides have stopped \
         meeting, so `the_module_and_the_browser_host_agree_on_arity` proves nothing"
    );
    // And the comparison itself distinguishes: a deliberately wrong arity is seen.
    let (name, params, _) = sigs.iter().find(|(n, _, _)| js.contains_key(*n)).unwrap();
    assert_ne!(
        js[*name],
        params.len() + 1,
        "sanity: {name} cannot have both its real arity and one more"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ THE WHOLE-TABLE CENSUS: every `HostFn` row, classified, and every REQUIRED host checked.
//
// The `dom_`/`storage_` census above proves the browser answers every DOM import; it says nothing
// about the other three-quarters of the table. `temporal_component`/`temporal_component_date`
// (and `parse_timestamp`, found in the same reproduction) were standalone, non-DOM imports that
// NO real host implemented — only a test's own hand-rolled `wasmi` linker did. That was one
// instance of a class: "an import some host lacks", and the reproduction predicted there would be
// more. There were: 24 more standalone rows were missing from `runtime.js`, several from the node
// shim, and 15 from `tests/common/wasm_host.rs` — plus `print_date`/`print_moment` in the node
// shim were WRONG (pushing the raw i32/i64 number instead of the formatted date), a bug this
// census's SOURCE-level check cannot see (both names are present) but its sibling execution test
// (`wasm_temporal_host_lock.rs`, holding output to the reference tier) already would have.
//
// Every row falls into exactly one bucket, derived by reading `vm/wasm/module.rs`'s ACTUAL
// used-import scan rather than doc comments (which are unreliable here: `read_wire_frame` carries
// no "linker only" annotation but IS gated `if linked` — confirmed by grep, not assumed):
//   - DOM/storage (prefix `dom_`/`storage_`): a per-test decision by this file's own design
//     (`tests/common/wasm_host.rs`'s header: "a test supplying its own is the test doing its
//     job"). Required host: the browser (`runtime.js`) only — already covered above.
//   - LINKED_ONLY: every `note(HostFn::X, ...)` call site for X is gated by `if linked`, or by a
//     `Kind` (`BigInt`/`Complex`/`Modular`/`Decimal`/`Money`/`Quantity`/`Uuid`/`LanesV`/`Dynamic`/
//     `Tower`) that can only exist in a linked program. Resolved at LINK time against the prebuilt runtime
//     object; never an `env` import of a self-contained module. No host required.
//   - STANDALONE: everything else. Required hosts: ALL THREE — `runtime.js`, `web_host::wasm_host_shim`
//     (the node shim), and `tests/common/wasm_host.rs` (this file's own doc: "no test may hand-
//     write one of these again" — the standalone family is exactly what that means).
// ─────────────────────────────────────────────────────────────────────────────

fn is_dom_shaped(name: &str) -> bool {
    name.starts_with("dom_") || name.starts_with("storage_")
}

/// Every `HostFn` whose ONLY `note(...)` call sites in `vm/wasm/module.rs`'s used-import scan are
/// gated `if linked` (directly, or transitively via a linker-only `Kind`) — verified by reading
/// every such call site, not assumed from a doc comment. `assemble_program_impl`'s LINKER MODE
/// resolves these against the prebuilt base runtime object at link time; a self-contained module
/// never imports them, so no host needs to answer them.
const LINKED_ONLY: &[&str] = &[
    // BigInt (9 kinds doc, `kind_is_handle`): overflow-promoted integer arithmetic.
    "logos_rt_bigint_from_i64",
    "logos_rt_bigint_pow",
    "logos_rt_bigint_to_text",
    "logos_rt_bigint_mul",
    "logos_rt_bigint_add",
    "logos_rt_bigint_sub",
    "logos_rt_bigint_div",
    "logos_rt_bigint_mod",
    // Complex.
    "logos_rt_complex_from_i64",
    "logos_rt_complex_add",
    "logos_rt_complex_sub",
    "logos_rt_complex_mul",
    "logos_rt_complex_to_text",
    "logos_rt_complex_abs",
    "logos_rt_complex_from_f64",
    "logos_rt_complex_re_f64",
    "logos_rt_complex_im_f64",
    // Modular (ℤ/nℤ).
    "logos_rt_modular_from_i64",
    "logos_rt_modular_add",
    "logos_rt_modular_sub",
    "logos_rt_modular_mul",
    "logos_rt_modular_div",
    "logos_rt_modular_pow",
    "logos_rt_modular_to_text",
    // Decimal (exact base-10).
    "logos_rt_decimal_from_text",
    "logos_rt_decimal_from_i64",
    "logos_rt_decimal_add",
    "logos_rt_decimal_sub",
    "logos_rt_decimal_mul",
    "logos_rt_decimal_to_text",
    "logos_rt_decimal_to_rational",
    // Money (currency-typed Decimal + exchange rates).
    "logos_rt_money_from_decimal",
    "logos_rt_money_from_i64",
    "logos_rt_money_add",
    "logos_rt_money_sub",
    "logos_rt_money_to_text",
    "logos_rt_set_rate",
    "logos_rt_to_currency",
    "logos_rt_set_rates_int",
    "logos_rt_set_rates_rational",
    "logos_rt_set_rates_decimal",
    // Quantity (unit-typed values).
    "logos_rt_quantity_of_i64",
    "logos_rt_quantity_convert",
    "logos_rt_quantity_add",
    "logos_rt_quantity_sub",
    "logos_rt_quantity_mul",
    "logos_rt_quantity_div",
    "logos_rt_quantity_to_text",
    // Rational (exact BigInt-backed fractions — NOT the plain (num,den) `print_rational` path,
    // which is standalone; only the handle-based exact arithmetic is linked).
    "logos_rt_rational_from_i64",
    "logos_rt_rational_from_bigint",
    "logos_rt_rational_add",
    "logos_rt_rational_sub",
    "logos_rt_rational_mul",
    "logos_rt_rational_div",
    "logos_rt_rational_to_text",
    "logos_rt_rational_floor",
    "logos_rt_rational_ceil",
    "logos_rt_rational_round",
    "logos_rt_rational_abs",
    "logos_rt_rational_cmp",
    // The VM's rules for an exact number: a whole rational takes the integer's `/`, `%`, `//`, `**` and bitwise
    // operators, a fraction the rational's; a real beside one meets its double.
    "logos_rt_rational_quotient",
    "logos_rt_rational_floor_quotient",
    "logos_rt_rational_remainder",
    "logos_rt_rational_power",
    "logos_rt_rational_bitwise",
    "logos_rt_rational_cmp_f64",
    "logos_rt_rational_to_f64",
    "logos_rt_rational_as_i64",
    // Uuid.
    "logos_rt_uuid_parse",
    "logos_rt_uuid_nil",
    "logos_rt_uuid_max",
    "logos_rt_uuid_dns",
    "logos_rt_uuid_url",
    "logos_rt_uuid_oid",
    "logos_rt_uuid_x500",
    "logos_rt_uuid_version",
    "logos_rt_uuid_eq",
    "logos_rt_uuid_to_text",
    "logos_rt_uuid_from_ptr",
    // `Moment/Date ± Span` calendar arithmetic (linker only — the doc comment on the note() call
    // site says so directly: "linker").
    "logos_rt_moment_add_span",
    "logos_rt_date_add_span",
    // The other linked-only temporal family (`if linked` at every note() site).
    "logos_rt_format_timestamp",
    "logos_rt_months_between",
    "logos_rt_years_between",
    "logos_rt_make_date",
    "logos_rt_make_moment",
    "logos_rt_in_zone",
    "logos_rt_local_instant",
    // LanesV (SIMD-lane values) — `lanes_v_host_fn`'s whole table is behind `if linked`.
    "logos_rt_lanes16_from_bytes",
    "logos_rt_lanes8_from_words",
    "logos_rt_lanes4w64_from_words",
    "logos_rt_lanes_splat16",
    "logos_rt_lanes_splat8",
    "logos_rt_lanes_to_seq",
    "logos_rt_lanes_shuffle",
    "logos_rt_lanes_interleave_lo",
    "logos_rt_lanes_interleave_hi",
    "logos_rt_lanes_byte_add",
    "logos_rt_lanes_maddubs",
    "logos_rt_lanes_packus",
    "logos_rt_lanes_shr_bytes",
    "logos_rt_lanes4_add",
    "logos_rt_lanes4_xor",
    // The runtime allocator (seeds the emitter's bump allocator from a linked slab).
    "logos_rt_alloc",
    // The wire-program protocol's linked half (`readWireProgram`/`WireBytes` conversions).
    "logos_rt_wire_bytes_int",
    "logos_rt_wire_bytes_bool",
    "logos_rt_wire_bytes_float",
    "logos_rt_wire_bytes_text",
    "read_wire_frame",
    "logos_rt_read_wire_program",
    "logos_rt_run_accepted",
    // SHA-1 round functions (crypto acceleration intrinsics).
    "logos_rt_sha1rnds4",
    "logos_rt_sha1msg1",
    "logos_rt_sha1msg2",
    "logos_rt_sha1nexte",
    // Dynamic (a wire-decoded value with no static kind).
    "logos_rt_dynamic_to_text",
    // The exact extension's value runtime (`logicaffeine_data::abi`) and its adapters: noted only for a program
    // the extension reaches, which a self-contained module refuses by name.
    "logos_rt_value_int",
    "logos_rt_value_float",
    "logos_rt_value_bool",
    "logos_rt_value_nothing",
    "logos_rt_value_date",
    "logos_rt_value_moment",
    "logos_rt_value_time",
    "logos_rt_value_span",
    "logos_rt_value_list",
    "logos_rt_value_clone",
    "logos_rt_value_push",
    "logos_rt_value_set_item",
    "logos_rt_value_pop",
    "logos_rt_value_items",
    "logos_rt_value_item_at",
    "logos_rt_value_slice",
    "logos_rt_value_add",
    "logos_rt_value_sub",
    "logos_rt_value_mul",
    "logos_rt_value_div",
    "logos_rt_value_exact_div",
    "logos_rt_value_floor_div",
    "logos_rt_value_rem",
    "logos_rt_value_pow",
    "logos_rt_value_approx_eq",
    "logos_rt_value_seq_concat",
    "logos_rt_value_bit_and",
    "logos_rt_value_bit_or",
    "logos_rt_value_bit_xor",
    "logos_rt_value_shl",
    "logos_rt_value_shr",
    "logos_rt_value_min",
    "logos_rt_value_max",
    "logos_rt_value_power",
    "logos_rt_value_item",
    "logos_rt_value_eq",
    "logos_rt_value_ne",
    "logos_rt_value_lt",
    "logos_rt_value_gt",
    "logos_rt_value_le",
    "logos_rt_value_ge",
    "logos_rt_value_not",
    "logos_rt_value_abs",
    "logos_rt_value_sqrt",
    "logos_rt_value_floor",
    "logos_rt_value_ceil",
    "logos_rt_value_round",
    "logos_rt_value_of_bigint",
    "logos_rt_value_of_rational",
    "logos_rt_value_of_decimal",
    "logos_rt_value_tuple_of",
    "logos_rt_value_apply",
    "logos_rt_value_define",
    "logos_rt_value_checked",
    "logos_rt_value_to_text",
    "logos_rt_value_of_text",
    "logos_rt_value_length_i64",
    "logos_rt_value_truth_i32",
    "logos_rt_value_contains_i64",
    "logos_rt_value_to_i64",
    "logos_rt_value_integer_at",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HostReq {
    /// Browser only (`runtime.js`) — a per-test decision everywhere else, by design.
    Dom,
    /// Resolved at link time; no `env` import of a self-contained module ever names it.
    Linked,
    /// One correct answer everywhere: `runtime.js`, the node shim, and the shared test host.
    Standalone,
}

fn classify(name: &str) -> HostReq {
    if is_dom_shaped(name) {
        HostReq::Dom
    } else if LINKED_ONLY.contains(&name) {
        HostReq::Linked
    } else {
        HostReq::Standalone
    }
}

#[test]
fn every_live_import_is_classified_and_the_classes_partition_correctly() {
    // The three buckets must be disjoint by construction (DOM is a prefix test, LINKED_ONLY is a
    // fixed list checked against it) and every name checked against `LINKED_ONLY` must be a REAL,
    // exact `live()` entry — a stale or misspelled row here would silently classify nothing.
    let live_set = live();
    let mut stale = Vec::new();
    for n in LINKED_ONLY {
        assert!(
            !is_dom_shaped(n),
            "{n} is in LINKED_ONLY but is DOM-shaped — pick one bucket"
        );
        if !live_set.contains(&n.to_string()) {
            stale.push(*n);
        }
    }
    assert!(
        stale.is_empty(),
        "LINKED_ONLY names no longer live (renamed or removed?): {stale:?}"
    );
    let dom_count = live_set.iter().filter(|n| is_dom_shaped(n)).count();
    let standalone_count = live_set
        .iter()
        .filter(|n| classify(n) == HostReq::Standalone)
        .count();
    assert_eq!(
        dom_count + LINKED_ONLY.len() + standalone_count,
        live_set.len(),
        "the three buckets must exactly partition every live import"
    );
}

#[test]
fn every_standalone_import_is_implemented_by_all_three_hosts() {
    // `args` is standalone by classification (the browser and the node shim each have exactly one
    // right answer: an empty `Seq of Text` for a page with no argv, real `process.argv` for a CLI
    // program) but NOT by the shared Rust test host: a test reading argv needs to inject SPECIFIC
    // values (`wasm_aot_args.rs`), so — like a DOM import — it is legitimately a per-test decision
    // there, not an opinion-free formatter. `tests/common/wasm_host.rs`'s own `link_value_imports`
    // doc comment carries the matching exemption and reasoning.
    const TEST_HOST_EXEMPT: &[&str] = &["args"];

    let shim = logicaffeine_compile::web_host::wasm_host_shim("x");
    let runtime_keys = host_arities_of(RUNTIME_JS);
    let shim_keys = host_arities_of(&shim);
    let common_src = include_str!("compile_common/wasm_host.rs");
    let mut missing = Vec::new();
    for n in live() {
        if classify(&n) != HostReq::Standalone {
            continue;
        }
        if !runtime_keys.contains_key(&n) {
            missing.push(format!("{n}: missing from assets/web/runtime.js"));
        }
        if !shim_keys.contains_key(&n) {
            missing.push(format!(
                "{n}: missing from web_host::wasm_host_shim (the node shim)"
            ));
        }
        if TEST_HOST_EXEMPT.contains(&n.as_str()) {
            continue;
        }
        // `tests/common/wasm_host.rs` registers some names through a literal `func_wrap("env",
        // "name", ...)` call and others (the `print_seq_*`/`print_set_*` family) through a loop
        // over an array of string literals — a quoted-name search catches both shapes; `host_
        // arities_of`'s stricter object-key shape does not apply to Rust source at all.
        if !common_src.contains(&format!("\"{n}\"")) {
            missing.push(format!("{n}: missing from tests/common/wasm_host.rs"));
        }
    }
    assert!(
        missing.is_empty(),
        "a standalone (non-DOM, non-linked) import is missing from a host that must answer it, \
         so a page or a headless program using it fails to instantiate there:\n{}",
        missing.join("\n")
    );
}

#[test]
fn no_runtime_js_or_shim_key_is_dead_code() {
    // The other half of the census: a key a host implements that names NO live import at all —
    // renamed-but-not-deleted cruft, or a typo that silently never fires because nothing ever
    // calls it. `host_arities_of` already excludes non-`env`-shaped lines (top-level `const`s,
    // object methods without `=>`), so a survivor here is a real orphaned import name.
    let live_set = live();
    let shim = logicaffeine_compile::web_host::wasm_host_shim("x");
    let mut dead = Vec::new();
    for (host, keys) in [
        ("runtime.js", host_arities_of(RUNTIME_JS)),
        ("the node shim", host_arities_of(&shim)),
    ] {
        for k in keys.keys() {
            if !live_set.contains(k) {
                dead.push(format!(
                    "{k}: implemented by {host} but names no live import"
                ));
            }
        }
    }
    assert!(
        dead.is_empty(),
        "dead host code, naming an import the compiler can no longer emit:\n{}",
        dead.join("\n")
    );
}

#[test]
fn the_whole_table_census_can_actually_fail() {
    // ★ PROVE THE TEETH. `host_arities_of` reads the OBJECT KEY, not any substring: a bare mention
    // with no `name: (args) =>` shape (exactly what an error message or a doc comment is — this
    // file's own runtime.js carries the literal text `` `temporal_component: unknown component
    // selector ${which}` `` inside a thrown `Error`) must NOT read as an implementation, or this
    // census would have missed the original gap.
    let prose_only = "  temporal_component: unknown component selector, not a real object key\n";
    assert!(
        !host_arities_of(prose_only).contains_key("temporal_component"),
        "a name mentioned in prose with no arrow-function shape must not read as an implementation"
    );
    // And the classifier distinguishes: a DOM name is never STANDALONE, a LINKED_ONLY name is
    // never STANDALONE, and a real standalone name (`temporal_component`) is neither DOM nor
    // LINKED — the three arms are all reachable, not a wall that always answers one way.
    assert_eq!(classify("dom_body"), HostReq::Dom);
    assert_eq!(classify("logos_rt_bigint_add"), HostReq::Linked);
    assert_eq!(classify("temporal_component"), HostReq::Standalone);
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ `noteTurn(name, key, changes, failure)` — a lowered turn's journal entry, all four parts.
//
// Every lowered `__act_` reports the action's name, the instance key it ran on, the state it changed
// (flattened `[field, before, after]` display strings) and the failure it stopped with. The browser
// keeps nothing; a recording host — the shared `InteractivePage`, the Chrome recorder — reads all four,
// so each must arrive intact: a Text, a Text, a `Seq of Text` and a Text.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "wasm-jit")]
#[test]
fn note_turn_lowers_to_wasm_and_the_test_host_records_every_part() {
    let src = "## Main\nLet changes be a new Seq of Text.\nPush \"n\" to changes.\nPush \"0\" to changes.\nPush \"1\" to changes.\n\
               Call noteTurn with \"clicked\" and \"row-7\" and changes and \"\".\n";
    let page = crate::common::wasm_host::mount_interactive(src);
    assert_eq!(
        page.journal(),
        vec![crate::common::wasm_host::JournalCall {
            name: "clicked".to_string(),
            key: "row-7".to_string(),
            changes: vec!["n".to_string(), "0".to_string(), "1".to_string()],
            failure: String::new(),
        }],
        "a noteTurn call reaching a real module must hand the host its name, key, changes and failure"
    );
}

#[cfg(feature = "wasm-jit")]
#[test]
fn note_turn_records_every_call_in_order() {
    let src = "## Main\nLet none be a new Seq of Text.\nCall noteTurn with \"first\" and \"\" and none and \"\".\n\
               Call noteTurn with \"second\" and \"k\" and none and \"Index 9 out of bounds\".\n";
    let page = crate::common::wasm_host::mount_interactive(src);
    assert_eq!(
        page.turns(),
        vec!["first".to_string(), "second".to_string()]
    );
    assert_eq!(
        page.journal()[1].failure,
        "Index 9 out of bounds",
        "the failure a turn stopped with reaches the host"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ `setHeadHtml(fragment)` — a `## Page`'s `Head:` block, applied on the lowered tier.
//
// The wasmi test host has no real `document.head` to merge into (that half is
// `runtime.js`'s `mergeIntoHead`, proven for real by `browser_tier_lock`'s Chrome harness); this
// only proves the call reaches a real module and carries the exact fragment text.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "wasm-jit")]
#[test]
fn set_head_html_lowers_to_wasm_and_the_test_host_records_it() {
    let src = "## Main\nCall setHeadHtml with \"<title>Todos</title>\".\n";
    let page = crate::common::wasm_host::mount_interactive(src);
    assert_eq!(
        page.head_html(),
        vec!["<title>Todos</title>".to_string()],
        "a setHeadHtml call reaching a real module must record its fragment on the host"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ THE ONE WAY A HOST HANDS THE MODULE A LIST.
//
// A Text the host answers with (`dom_get_text`, a response body, the route) is written into the
// module's own heap through `__logos_text_new`, so it is an ordinary Text the moment it arrives. A
// LIST the host answers with (a dropped file's `bytes`) takes the same road through
// `__logos_seq_new(len, handles)`: the module allocates an ordinary `Seq` header — its class, and on
// a module that counts holders its true count — and the host only fills the slots. Every import
// that answers this way spells its result `HOST_SEQ` in the host-import table.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "wasm-jit")]
#[test]
fn a_host_built_list_is_an_ordinary_sequence_the_module_reads() {
    let spelled = logicaffeine_compile::vm::wasm::host_seq_import_names();
    assert_eq!(
        spelled,
        vec!["dom_event_file_bytes"],
        "the census of imports that answer with a host-built list"
    );
    for name in &spelled {
        assert!(
            FROZEN.contains(name),
            "`{name}` answers with a list and is not written down in FROZEN"
        );
    }

    let src = "## Main\nLet b be eventFileBytes(0).\nShow length of b.\nShow item 2 of b.\nPush 4 to b.\nShow length of b.\nShow item 4 of b.\n";
    let module =
        logicaffeine_compile::compile::compile_to_wasm(src).unwrap_or_else(|e| panic!("{e:?}"));
    let engine = wasmi::Engine::default();
    let parsed =
        wasmi::Module::new(&engine, &module[..]).unwrap_or_else(|e| panic!("invalid module: {e}"));
    assert!(
        parsed.exports().any(|e| e.name() == "__logos_seq_new"),
        "a module that takes a host-built list exports the allocator the host builds it through"
    );
    let run = crate::common::wasm_host::run_outcome_hosted(src, None, |l| {
        l.func_wrap(
            "env",
            "dom_event_file_bytes",
            |mut c: wasmi::Caller<'_, _>, _ev: i64| -> i32 {
                crate::common::wasm_host::alloc_seq_i64(&mut c, &[7, 8, 9])
            },
        )
        .expect("dom_event_file_bytes");
    })
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(run.failure, None);
    assert_eq!(
        run.output, "3\n8\n4\n4",
        "the host's three bytes read back as a Seq of Int the module can index and grow"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ `listenFlags(el, event, flags)` and `assignLocation(url)` — two effects whose ARGUMENTS are the
// whole contract: the host acts on exactly what arrives (which node, which event, which modifier
// bits; which URL) and decides nothing. A module that passed the flags as a Text, or dropped one
// argument, would instantiate and call the right name with the wrong data — so the test reads the
// values the host actually received.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "wasm-jit")]
#[test]
fn listen_flags_and_assign_location_reach_the_host_with_their_arguments() {
    use std::sync::{Arc, Mutex};
    let listens: Arc<Mutex<Vec<(i64, String, i64)>>> = Arc::default();
    let locations: Arc<Mutex<Vec<String>>> = Arc::default();
    let src = "## Main\nLet a be listenFlags(7, \"click\", 10).\nLet b be assignLocation(\"https://example.com/away\").\nShow \"done\".\n";
    let (l_rec, loc_rec) = (listens.clone(), locations.clone());
    let run = crate::common::wasm_host::run_outcome_hosted(src, None, move |l| {
        l.func_wrap(
            "env",
            "dom_listen_flags",
            move |c: wasmi::Caller<'_, _>, el: i64, event: i32, flags: i64| {
                let event = crate::common::wasm_host::read_text(&c, event);
                l_rec.lock().unwrap().push((el, event, flags));
            },
        )
        .expect("dom_listen_flags");
        l.func_wrap(
            "env",
            "dom_assign_location",
            move |c: wasmi::Caller<'_, _>, url: i32| {
                loc_rec
                    .lock()
                    .unwrap()
                    .push(crate::common::wasm_host::read_text(&c, url));
            },
        )
        .expect("dom_assign_location");
    })
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(run.failure, None);
    assert_eq!(run.output, "done");
    assert_eq!(
        *listens.lock().unwrap(),
        vec![(7, "click".to_string(), 10)],
        "the node, the event name and the modifier bits (stop 2 | capture 8) reach the host as written"
    );
    assert_eq!(
        *locations.lock().unwrap(),
        vec!["https://example.com/away".to_string()]
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ `randomInt(lo, hi)` — randomness is the host's to supply; the bounds are the program's.
// ─────────────────────────────────────────────────────────────────────────────

/// A wasm program calling `randomInt(1, 6)` compiles, and every draw of many lands in `1..=6` — and
/// across the draws every face appears, so the host is not answering a constant.
#[cfg(feature = "wasm-jit")]
#[test]
fn random_int_reaches_the_host_and_every_draw_stays_in_range() {
    let src = "## Main\nLet mutable i be 0.\nWhile i < 600:\n    Show randomInt(1, 6).\n    Set i to i + 1.\n";
    let run = crate::common::wasm_host::run_outcome(src).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(run.failure, None);
    let draws: Vec<i64> = run
        .output
        .lines()
        .map(|l| l.parse().expect("each draw shows an Int"))
        .collect();
    assert_eq!(draws.len(), 600);
    assert!(
        draws.iter().all(|d| (1..=6).contains(d)),
        "a draw left 1..=6: {draws:?}"
    );
    for face in 1..=6 {
        assert!(
            draws.contains(&face),
            "600 draws of 1..=6 never produced {face}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ THE FIELDS A VISITOR TYPES INTO ARE ONE LIST, KEPT THREE TIMES.
// ─────────────────────────────────────────────────────────────────────────────

/// The first array literal of quoted strings after `anchor` in `source`, as its members — a type
/// annotation's brackets (`[&str; 8]`) are passed over.
fn string_array_after(source: &str, anchor: &str) -> Vec<String> {
    let at = source
        .find(anchor)
        .unwrap_or_else(|| panic!("`{anchor}` is not in the source any more"));
    let rest = &source[at..];
    let open = rest
        .match_indices('[')
        .map(|(i, _)| i)
        .find(|&i| rest[i + 1..].trim_start().starts_with(['"', '\'']))
        .expect("an array of quoted strings follows the anchor");
    let close = open + rest[open..].find(']').expect("the array closes");
    rest[open + 1..close]
        .split(',')
        .map(|m| m.trim())
        .filter(|m| !m.is_empty())
        .map(|m| m.trim_matches(|c| c == '"' || c == '\'').to_string())
        .collect()
}

/// `semantics::dom` decides a `Type` for every in-process tier (`answers_by_itself`), `runtime.js`
/// performs a page's own (`dom_enter_value`), and the Chrome recorder performs a test's
/// (`focusForTyping`). Each spells the `<input>` types that take typed text, and each must spell the
/// same ones — a type in one list and not another is a field one tier types into and another refuses.
/// The browser reports an `<input>` with no `type` as `"text"`, which the recorder's document spells
/// `""`, so the JavaScript lists carry every type but that one. The words that bar a control are the
/// same in each: disabled, and read-only for typing.
#[test]
fn every_host_types_into_the_same_fields() {
    const DOM_RS: &str = include_str!("../../logicaffeine_compile/src/semantics/dom.rs");
    const RECORDER_JS: &str =
        include_str!("../../../apps/logicaffeine_cli/src/browser/recorder.js");
    let mut reference = string_array_after(DOM_RS, "const TEXT_ENTRY_TYPES");
    reference.retain(|t| !t.is_empty());
    let entry_at = RUNTIME_JS
        .find("dom_enter_value:")
        .expect("runtime.js performs a page's own Type");
    let entry = &RUNTIME_JS[entry_at
        ..entry_at
            + RUNTIME_JS[entry_at..]
                .find("\n  },")
                .expect("the entry closes")];
    let hosts = [
        (
            "runtime.js dom_enter_value",
            string_array_after(entry, "n.localName === 'input' &&"),
        ),
        (
            "recorder.js TEXT_ENTRY_TYPES",
            string_array_after(RECORDER_JS, "const TEXT_ENTRY_TYPES"),
        ),
    ];
    for (host, types) in &hosts {
        assert_eq!(
            types, &reference,
            "{host} types into other fields than semantics::dom's TEXT_ENTRY_TYPES"
        );
    }
    let barred = &DOM_RS[DOM_RS
        .find("fn barred")
        .expect("semantics::dom bars controls")..];
    assert!(
        barred.contains("has(\"disabled\")") && barred.contains("has(\"readonly\")"),
        "semantics::dom bars a disabled or read-only control"
    );
    for (host, text) in [
        ("runtime.js dom_enter_value", entry),
        ("recorder.js", RECORDER_JS),
    ] {
        assert!(
            text.contains("':disabled'") && text.contains("':read-write'"),
            "{host} must bar a disabled control and type only into a `:read-write` one, as semantics::dom does"
        );
    }
}
