# Data table — sort, filter, paginate

An employee table: sort by id or salary (ascending, then descending on a
second click), filter by department, and paginate — with `keyed by` so a
row's own element (and anything a visitor was doing to it) follows its
record rather than being rewritten in place.

```bash
largo test           # runs the ## Test block against the tree-walker and the VM
largo serve          # http://localhost:8080
largo build --static # dist/ - upload it anywhere
```

## What it shows

**The table is one derived field.** `visibleRows` is `Let ... always be` the
employees filtered by department, sorted by the chosen column and sliced to the
current page. Each action sets only the field it owns (the department, the sort
key and direction, the page), and the table follows.

**`pageStart`/`pageEnd`/`clampPage` from `ui.lg`**, used exactly as
documented, to slice the filtered-and-sorted rows into pages.

**A hand-rolled insertion sort.** `LOGOS_QUICKGUIDE.md` marks list sorting
`(proposed)`, not a builtin, so `sortRows` writes one directly with
`Repeat`/index writes, sorting by whichever key the last click chose.

## Limits

**Two `Text` values cannot be compared with `is less than`** ("Cannot
compare Text and Text," on every execution tier, not a WASM-only gap) — consistent
with `LOGOS_QUICKGUIDE.md` marking general list sorting `(proposed)`. This
app sorts by the two `Int` columns (`id`, `salaryCents`) rather than
alphabetically by name.
