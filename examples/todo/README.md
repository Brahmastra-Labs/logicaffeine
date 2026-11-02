# Todo — add, remove, toggle, filter, persist

The classic todo list: add, remove, toggle done, filter (all / active / done),
a live "N left" count, and the list survives a page reload.

```bash
largo test           # runs the ## Test blocks against the tree-walker and the VM
largo serve          # http://localhost:8080
largo build --static # dist/ - upload it anywhere
```

## What it shows

**The list persisted with two sentences.** `Save todos as "todos".` ends
`add`, `toggle` and `remove`; `Load "todos" into todos.` runs
`When the page loads:`. The `Seq of Todo` makes the round trip through the JSON
codec derived from the `Todo` record, so the page has no encoder of its own,
and a first visit (nothing saved yet) loads the empty list the field started
with. `nextId` is recomputed from the loaded list, so an item added after a
reload takes the next free id.

**Derived state, including a derived list.** `left` counts the open items, and
`visible` is the list the current filter shows:
`Let visible always be visibleTodos(todos, filter).` is recomputed whenever
`todos` or `filter` changes. The view loops over `visible` with an
`{else:}` arm that says "Nothing to do." when the filter matches nothing.

**`keyed by t's id`.** Toggling or filtering reorders and removes rows; each
row's own element (and anything a visitor was doing to it) follows its `Todo`
by id rather than being rewritten in place.

**A one-click, one-transaction turn.** `add`, `toggle` and `remove` each
change the list and save it in the same turn — a turn is one user action, and
persistence is a normal part of it, not a separate step.
