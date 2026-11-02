# Infinite scroll — a paged feed that loads itself

Loads the first page of todos when the page loads, then the next page each
time the visitor scrolls to the bottom, until the server answers an empty page.

```
examples/infinite_scroll/
├── Largo.toml          # a frontend-only project: no backend entry
├── src/shared.lg       # the Todo record, declared once
├── pages/app.lg        # the page (Import Shared.) and its tests
├── static/style.css    # served at the site root as /style.css
├── static/theme.css    # the design tokens the page's styles read
└── README.md
```

```bash
largo test           # runs the ## Test blocks on the tree-walker and the VM
largo check          # includes the WebAssembly build a browser runs
largo serve          # http://localhost:8080
largo build --static # dist/ - upload it anywhere
```

The tests stub each page of the feed with
`Given that GET "/api/todos?page=2" answers …`; point the URLs at a real feed
to use it for real.

## What it shows

**A record imported from its own file.** `src/shared.lg` declares
`## A Todo has:`, and `pages/app.lg` names it with `Import Shared.` in the prose
above its first `##` section — where every import of a literate LOGOS file
goes. `Get … as Seq of Todo` decodes through the JSON codec derived from that
declaration, the same as if it were written in the page.

**`When the user scrolls to the bottom:`.** The trigger hears the semantic
event, not the raw one: a browser raises `scroll` many times a second, and the
compiled listener decides when the visitor has reached the end. A test causes
that event directly with `Raise "scrolledtobottom".`, the name every execution tier
delivers to this trigger.

**The page number is state, and the URL is built from it.**
`Get "/api/todos?page={pageNum}" as Seq of Todo, then loaded.` asks for the
next page on each scroll, and `loaded` appends what arrives with a copy that is
written back (`Let mutable merged be todos.` … `Set todos to merged.`).

**An empty page ends the feed, and a failure ends it too.** `exhausted` turns
on when the server answers `[]` or the request fails, and the scroll handler
does nothing after that: the third test stubs a third page and proves it is
never fetched. A failed request shows `e's message` (`503: feed offline`) as an
`alert`.

**A keyed list and a derived count.** `{for each t in todos, keyed by t's id:}`
keeps each row attached to its todo, and `Let loadedCount always be length of
todos.` recomputes after every turn that changes `todos`.
