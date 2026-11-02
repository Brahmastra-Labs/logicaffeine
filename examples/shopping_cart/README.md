# Shopping cart — a shared, persisted cart

A small catalog, a running cart of line items shown in a side panel, a header
badge counting them, and one persisted `## Store` holding the cart, so the line
items and the count both survive a reload.

```bash
largo test           # runs the ## Test blocks against the tree-walker and the VM
largo serve          # http://localhost:8080
largo build --static # dist/ - upload it anywhere
```

## What it shows

**The cart is a `## Store`, and every reader shares it.** `## Store Cart`
holds the line items as a `Seq of CartLine`. `CartBadge` shows
`length of Cart's lines` and `CartPanel` draws the lines and their total. The
page changes the cart only through the store's own actions
(`Call Cart's addLine with name and price.` and
`Call Cart's removeLine with id.`), so the badge, the list and the total are
always the same cart.

**Persisted by its header.** `## Store Cart persisted as "cart"` keeps the
store's state in local storage through the JSON codec derived from `CartLine`,
and reads it back when the page loads: a reload shows the same lines, the same
count and the same total. A new line's id is one past the highest id in the
cart, so ids stay unique after a reload.

**An action passed down as a prop.** `## Component CartPanel (remove: fn(Int) -> Unit)`
draws each line with a "remove" button that calls `remove(line's id)`; the page
places it as `<CartPanel remove={removeLine}/>`, so what removing means is the
page's decision, not the panel's — `docs/frontend.md`'s "Passing actions down".

**One scenario per test.** Each `## Test:` starts from a freshly mounted page
with empty storage, so the empty cart, adding, removing, and the two reload
scenarios are each checked on their own.
