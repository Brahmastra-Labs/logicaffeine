# Modal, toast and keyboard

A modal dialog — Escape closes it, focus moves in on open and back to the
opener on close — and a toast shown and dismissed by hand.

```bash
largo test           # runs the ## Test blocks against the tree-walker and the VM
largo serve          # http://localhost:8080
largo build --static # dist/ - upload it anywhere
```

## What it shows

**A backdrop and a card as siblings, not nested.** Both are `position:
fixed`; the backdrop's own `on-click={close}` never fires for a click on the
card, because the card is not one of its DOM descendants — no event
modifier needed. See Limits for why this app does not use `.stop`.

**Focus follows the dialog, one real turn later.** `After 100ms: Focus
"#close-modal".` runs as a genuinely later turn, once the modal's own markup
has actually reached the document — see Limits for why `When this component
appears:` cannot do this directly.

**Escape is a page-level keyboard sentence**, not something the modal
component has to own — `When the user presses Escape:` on the PAGE closes it
regardless of what currently has focus, matching a native `<dialog>`.

## Limits

**Not yet landed in `examples/`.** `lowered_vm_tier_lock` shows the Modal's `close` callback prop's
`on-click` — on both the backdrop and the close button — never wiring up on the lowered VM tier (the
tier a real browser runs): `Nothing answers a click on "#backdrop"`, though the tree-walker runs it
correctly and the page compiles and checks clean. Finding **G15**; RED test
`gauntlet_findings_lock::a_callback_props_on_click_wires_up_on_the_lowered_vm_tier`. This app is
otherwise correct and stays in `work/lgx-ease/gauntlet-scratch/` until G15 lands, since rewriting it
to avoid a callback prop entirely would defeat its own point.

**`.stop` does not build for the browser.** It is real
(`crates/logicaffeine_language/src/lgx/html.rs`'s `Behaviour::Stop`) and the
obvious way to keep a click inside the modal card from bubbling to the
backdrop's own close handler, but `largo check` refuses the whole page:
"its `.stop` event modifier is not lowered for this tier yet." Confirmed
still open (the campaign's own plan already named it) and undocumented in
`docs/frontend.md`/`LOGOS_QUICKGUIDE.md` besides. Finding **G9**; RED test
`gauntlet_findings_lock::stop_event_modifier_builds_for_the_browser`. Worked
around with the sibling layout described above.

**A component cannot focus its own element from `When this component
appears:`.** The hook runs before its own markup reaches the DOM — `Focus`
on the component's OWN element fails with "found no such element," while
the identical call targeting an element OUTSIDE the newly-mounted subtree
already works. Finding **G13**; RED test
`gauntlet_findings_lock::a_component_can_focus_its_own_element_on_appearing`.
Worked around with `After 100ms:` instead — a real, later turn, after the
DOM commit.

**`Advance time by 1 millisecond.` (the English words) is refused;
`Advance time by 100ms.` (the compact literal) is not** — found only while
writing the `## Test` for the `After 100ms:` workaround above. Finding
**G14**; RED test
`gauntlet_findings_lock::advance_time_accepts_the_word_form_of_a_sub_second_duration`.
