# Modal, toasts and the keyboard

A settings page whose name is edited in a modal dialog, with toasts that
announce each save and leave on their own. Everything interactive is driven
by the page's `## Test`s.

```bash
largo test           # runs the ## Test blocks on the tree-walker and the VM
largo serve          # http://localhost:8080
largo build --static # dist/ - upload it anywhere
```

## What it shows

**A modal that closes the three ways a visitor expects.** The dialog sits
inside a backdrop that answers `on-click={cancel}`. The dialog itself carries
`on-click.stop={stay}`, so a click anywhere inside it is answered there and
never reaches the backdrop; its Save and Cancel buttons carry `.stop` for the
same reason. `When the user presses Escape:` closes it from anywhere on the
page, and `on-keydown.enter={save}` on the name field saves without a click.

**Toasts in a store, each on its own clock.** `## Store Toasts` holds a
`Seq of Note` records and the counter that numbers them. Saving calls
`Toasts's post`; the tray places one `Toast` per note, `keyed by` its number,
so each toast keeps its own `After 3 seconds:` timer however many arrive after
it. The timer and the toast's own dismiss button both call `Toasts's dismiss`,
which rebuilds the list without that number.

**Time is the test's.** `Advance time by 2 seconds.` moves the page clock, so
the test that posts two toasts two seconds apart checks that each leaves on
its own schedule, instantly.

## Layout

```
Largo.toml
pages/app.lg         the page, its store, its components and its tests
static/style.css     the backdrop, dialog and toast rules
```
