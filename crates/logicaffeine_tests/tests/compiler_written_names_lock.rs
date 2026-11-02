//! ★ LOCK — every name the LGX lowering MINTS is one `is_compiler_written` recognises.
//!
//! An LGX page lowers into functions and registries the author never sees, and the escape and
//! ownership analyses run over that LOWERED program. Anything they report about a synthesized name
//! reaches the author's editor as an error about code they did not write, at a span that is the
//! whole file (synthesized statements carry no source position, so the fallback is everything).
//! That is what `is_compiler_written` filters — so a minted name it does NOT recognise is a false
//! diagnostic waiting for the right page to trigger it.
//!
//! ★ THIS LOCK EXISTS BECAUSE THE FIRST VERSION OF THAT PREDICATE WAS ALREADY WRONG. It enumerated
//! the families it knew — `__lgx_`, `__render_`, `__act_`, `__bind_`, `__patch_`, `__dir_`,
//! `__html_`, `__route_`, `__S_`, `__M_`, `__P_`, `__SEEN_`, `__KEYS_` — and the lowering also
//! mints `__ARG_`, `__DUE_`, `__SITE_`, `__STYLED_`, `__Store_` and bare temporaries beginning no
//! family at all. An enumeration of a growing set is wrong between two commits and wrong SILENTLY.
//! The predicate is now the actual invariant (a leading `__`), and this holds the minting side to
//! it: a new construct whose temporary is spelled `tmp` fails HERE, not in someone's editor.

use logicaffeine_language::parser::lgx_lower as n;

#[path = "support/lgx_lower_source.rs"]
mod lgx_lower_source;

/// One entry per public name-minting function, called with representative arguments.
fn every_minted_name() -> Vec<(&'static str, String)> {
    vec![
        ("render_fn_name", n::render_fn_name("App")),
        ("site_binding", n::site_binding("App", "0.1")),
        ("patch_fn_name", n::patch_fn_name("App", "add")),
        ("instance_key_param", n::instance_key_param().to_string()),
        ("state_binding", n::state_binding("App", "count")),
        ("styled_binding", n::styled_binding("App")),
        ("render_slot_param", n::render_slot_param().to_string()),
        ("block_slot_param", n::block_slot_param("content")),
        ("html_fn_name", n::html_fn_name("App")),
        ("serve_fn_name", n::serve_fn_name("App")),
        ("draw_depth_param", n::draw_depth_param().to_string()),
        ("view_html_fn_name", n::view_html_fn_name("card")),
        ("bind_fn_name", n::bind_fn_name("App", "count")),
        ("prop_binding", n::prop_binding("Row", "label")),
        (
            "callback_tag_binding",
            n::callback_tag_binding("Child", "onGo"),
        ),
        (
            "callback_key_binding",
            n::callback_key_binding("Child", "onGo"),
        ),
        (
            "callback_arg_binding",
            n::callback_arg_binding("Child", "onGo", 0),
        ),
        (
            "callback_trampoline_fn_name",
            n::callback_trampoline_fn_name("Child", "onGo"),
        ),
        (
            "outbox_count_binding",
            n::outbox_count_binding().to_string(),
        ),
        (
            "outbox_selector_binding",
            n::outbox_selector_binding().to_string(),
        ),
        (
            "outbox_event_binding",
            n::outbox_event_binding().to_string(),
        ),
        (
            "outbox_value_binding",
            n::outbox_value_binding().to_string(),
        ),
        (
            "listener_arg_binding",
            n::listener_arg_binding("App", "add", 0),
        ),
        ("listener_fn_binding", n::listener_fn_binding().to_string()),
        ("once_spent_binding", n::once_spent_binding().to_string()),
        (
            "listener_once_binding",
            n::listener_once_binding().to_string(),
        ),
        ("once_record_binding", n::once_record_binding("App")),
        (
            "listener_key_binding",
            n::listener_key_binding().to_string(),
        ),
        (
            "listener_space_binding",
            n::listener_space_binding().to_string(),
        ),
        (
            "listener_spill_binding",
            n::listener_spill_binding().to_string(),
        ),
        (
            "listener_spill_slot_binding",
            n::listener_spill_slot_binding().to_string(),
        ),
        (
            "listener_spill_count_binding",
            n::listener_spill_count_binding().to_string(),
        ),
        (
            "listener_held_binding",
            n::listener_held_binding().to_string(),
        ),
        (
            "listener_left_binding",
            n::listener_left_binding().to_string(),
        ),
        (
            "forget_listener_fn_name",
            n::forget_listener_fn_name().to_string(),
        ),
        ("render_frag_param", n::render_frag_param().to_string()),
        ("frag_owner_binding", n::frag_owner_binding().to_string()),
        ("frag_key_binding", n::frag_key_binding().to_string()),
        ("own_frag_fn_name", n::own_frag_fn_name().to_string()),
        ("gone_fn_name", n::gone_fn_name().to_string()),
        ("commit_fn_name", n::commit_fn_name().to_string()),
        (
            "drain_directives_fn_name",
            n::drain_directives_fn_name().to_string(),
        ),
        (
            "directive_queue_site_binding",
            n::directive_queue_site_binding("tooltip"),
        ),
        (
            "directive_queue_value_binding",
            n::directive_queue_value_binding("tooltip"),
        ),
        (
            "directive_queue_key_binding",
            n::directive_queue_key_binding("tooltip"),
        ),
        ("forwarded_binding", n::forwarded_binding("App", 0)),
        ("seen_binding", n::seen_binding("App")),
        ("keys_binding", n::keys_binding("App")),
        ("due_binding", n::due_binding("App", 0)),
        ("seq_binding", n::seq_binding("App", 0)),
        ("period_binding", n::period_binding("App", 0)),
        ("now_binding", n::now_binding().to_string()),
        ("timer_seq_binding", n::timer_seq_binding().to_string()),
        ("rearm_fn_name", n::rearm_fn_name().to_string()),
        ("listener_sites_binding", n::listener_sites_binding("App")),
        (
            "failing_component_binding",
            n::failing_component_binding().to_string(),
        ),
        ("open_turns_binding", n::open_turns_binding().to_string()),
        ("open_keys_binding", n::open_keys_binding().to_string()),
        (
            "watch_changes_binding",
            n::watch_changes_binding().to_string(),
        ),
        ("watching_binding", n::watching_binding().to_string()),
        ("watch_key_binding", n::watch_key_binding().to_string()),
        ("watch_depth_binding", n::watch_depth_binding().to_string()),
        ("failing_key_binding", n::failing_key_binding().to_string()),
        ("on_failure_fn_name", n::on_failure_fn_name().to_string()),
        ("tick_fn_name", n::tick_fn_name().to_string()),
        ("descends_fn_name", n::descends_fn_name().to_string()),
        ("pass_binding", n::pass_binding().to_string()),
        ("mount_binding", n::mount_binding("App")),
        ("dispatch_fn_name", n::dispatch_fn_name().to_string()),
        (
            "dispatch_node_fn_name",
            n::dispatch_node_fn_name().to_string(),
        ),
        ("answer_fn_name", n::answer_fn_name().to_string()),
        (
            "route_changed_fn_name",
            n::route_changed_fn_name().to_string(),
        ),
        ("owns_fn_name", n::owns_fn_name().to_string()),
        ("moved_binding", n::moved_binding().to_string()),
        ("cascade_binding", n::cascade_binding().to_string()),
        (
            "store_moved_binding",
            n::store_moved_binding("Session", "user"),
        ),
        ("wake_stores_fn_name", n::wake_stores_fn_name().to_string()),
        (
            "stores_dirty_binding",
            n::stores_dirty_binding().to_string(),
        ),
        (
            "render_depth_binding",
            n::render_depth_binding().to_string(),
        ),
        ("appear_who_binding", n::appear_who_binding().to_string()),
        ("appear_key_binding", n::appear_key_binding().to_string()),
        ("appear_at_binding", n::appear_at_binding().to_string()),
        ("appear_fn_name", n::appear_fn_name().to_string()),
        (
            "failure_context_binding",
            n::failure_context_binding().to_string(),
        ),
        (
            "failure_context_fn_name",
            n::failure_context_fn_name().to_string(),
        ),
        (
            "repaint_root_fn_name",
            n::repaint_root_fn_name().to_string(),
        ),
        ("shared_joined_binding", n::shared_joined_binding("Board")),
        ("shared_join_fn_name", n::shared_join_fn_name("Board")),
        ("shared_publish_fn_name", n::shared_publish_fn_name("Board")),
        ("shared_refresh_fn_name", n::shared_refresh_fn_name("Board")),
        (
            "shared_arrival_fn_name",
            n::shared_arrival_fn_name().to_string(),
        ),
        (
            "shared_clock_binding",
            n::shared_clock_binding().to_string(),
        ),
        (
            "shared_replica_binding",
            n::shared_replica_binding().to_string(),
        ),
        (
            "shared_base_binding",
            n::shared_base_binding("Board", "likes"),
        ),
        (
            "shared_ids_binding",
            n::shared_ids_binding("Board", "lines"),
        ),
        ("raise_fn_name", n::raise_fn_name().to_string()),
        ("store_type_name", n::store_type_name("Session")),
        ("directive_fn_name", n::directive_fn_name("tooltip")),
        (
            "directive_removed_fn_name",
            n::directive_removed_fn_name("watched"),
        ),
        (
            "directive_keys_binding",
            n::directive_keys_binding("watched"),
        ),
        (
            "directive_stamp_binding",
            n::directive_stamp_binding("watched"),
        ),
        (
            "directive_node_binding",
            n::directive_node_binding("watched"),
        ),
        (
            "directive_value_binding",
            n::directive_value_binding("watched"),
        ),
        ("mount_fn_name", n::mount_fn_name("App")),
        ("action_fn_name", n::action_fn_name("App", "add")),
        ("route_fn_name", n::route_fn_name("GET", "/todos")),
        ("view_type_name", n::view_type_name().to_string()),
        ("view_variant_name", n::view_variant_name("badge")),
        (
            "empty_view_variant_name",
            n::empty_view_variant_name().to_string(),
        ),
        ("view_draw_fn_name", n::view_draw_fn_name("badge")),
        ("place_view_fn_name", n::place_view_fn_name().to_string()),
        ("view_scope_param", n::view_scope_param().to_string()),
        ("view_value_param", n::view_value_param().to_string()),
    ]
}

#[test]
fn every_minted_name_is_recognised_as_compiler_written() {
    for (who, name) in every_minted_name() {
        assert!(
            n::is_compiler_written(&name),
            "`{who}` mints `{name}`, which `is_compiler_written` does not recognise — an analysis \
             reporting on it would put an error in the author's editor about a name they have \
             never seen. Mint it with a leading `__` like every other synthesized name."
        );
    }
}

/// ★ AND THE PREDICATE MUST STILL SAY NO TO ORDINARY NAMES, or it would silence real diagnostics.
///
/// The filter's whole value is that it drops ONLY what the author cannot act on. A predicate that
/// answered yes to everything would make the editor quiet in exactly the way a broken one does.
#[test]
fn an_ordinary_name_is_not_compiler_written() {
    for ordinary in [
        "count", "todo", "x", "_private", "userName", "add", "App", "_",
    ] {
        assert!(
            !n::is_compiler_written(ordinary),
            "`{ordinary}` is a name an author could write, and a diagnostic about it must reach them"
        );
    }
}

/// The census is not allowed to shrink into meaninglessness: every public minting function in
/// the `lgx_lower` module should have a row above. A count is a blunt instrument, but it fails when
/// someone adds a family and forgets this file — which is the failure this lock is for.
#[test]
fn the_census_covers_every_public_minting_function() {
    let declared = lgx_lower_source::files()
        .iter()
        .flat_map(|(_, src)| src.lines().collect::<Vec<_>>())
        .filter(|l| {
            l.starts_with("pub fn ")
                && (l.contains("_fn_name(")
                    || l.contains("_binding(")
                    || l.contains("_param(")
                    || l.contains("_type_name(")
                    || l.contains("_variant_name("))
        })
        .count();
    assert_eq!(
        declared,
        every_minted_name().len(),
        "the `lgx_lower` module declares {declared} public name-minting functions and this census \
         lists {}. A minted name with no row here is a name nothing checks.",
        every_minted_name().len()
    );
}
