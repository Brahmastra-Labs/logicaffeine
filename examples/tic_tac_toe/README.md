# Tic tac toe — a page with no server

The smallest complete demonstration of the reactive half of LOGOS. There is no backend,
no database and no API: the whole game is one `## Page`, compiled to WebAssembly and
served as static files.

```bash
largo test           # plays the games in the `## Test` blocks
largo serve          # http://localhost:8080
largo build --static # dist/ — upload it anywhere
```

## What it shows

**State, actions and turns.** `board` and `turn` are `State:`; `play` and `reset` are the
only things that change them. One click is one turn: the action runs, the page paints
once, and if the action fails the state goes back to what it was.

**A derived value.** `Let winner always be winnerOf(board).` recomputes whenever the board
moves, and the heading reads it. Nothing subscribes to anything by hand.

**An action with an argument.** `on-click={play(i)}` — the cell that was clicked is an
argument, so nine buttons share one action instead of needing nine.

**A loop that builds the board.** `{for each cell in board, at index i:}` renders the nine
cells and gives each one an id, `#c1` through `#c9`. The index counts from 1, like every
position in LOGOS, so the `i` in a cell's id is the `i` that `item i of board` reads: the
button and the square it plays are the same number.

**Ordinary LOGOS beside the page.** `winnerOf` and `lineWinner` are plain `## To`
functions in the same file. A page is a LOGOS program that happens to draw.

**Tests that drive the page.** The `## Test` blocks at the bottom play whole games and
assert what the page shows after each move: X winning a row, O winning a column, a click on
a taken square, a click after the game is over, and `Play again`. `largo test` runs them; a
page is testable without a browser.
