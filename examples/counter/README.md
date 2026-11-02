# Counter — the smallest LGX page

One `## Page`, one `State:` field, three `Actions:`, one `View:`. This is the
first thing to build in LGX, the LOGOS web framework: every other page in these
examples builds on this shape.

```bash
largo test           # runs the ## Test blocks against the tree-walker and the VM
largo serve          # http://localhost:8080
largo build --static # dist/ - upload it anywhere
```

## What it shows

**A turn.** Each click runs one action, recomputes nothing else (there is
nothing derived here), and repaints once. `increment`, `decrement` and
`reset` are the only three places `count` can change.

**No boilerplate.** `## Page /` mounts itself — there is no `## Main`, no
`Mount`, and no wiring between the button and the field it changes beyond
`on-click={increment}`.

## Limits

This example has no persistence, no routing and no server: it is the
smallest possible page, not a template for a larger app. See `examples/todo`
for persistence and list state, and `examples/multi_page_site` for routing.
