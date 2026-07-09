# Gleam Language Tour

Source: <https://tour.gleam.run/>

## Structure

A single progressive tour covering "all aspects of the language": basics & types, functions,
flow control / case expressions, custom types & records, the standard-library essentials, and
the modern conveniences (labelled arguments, `use`, pipelines). Three navigation modes:
**Next** (sequential), **Contents** (jump), and an **everything-on-one-page** view.

## Standout pedagogical / QOL features

- **Compiles-and-evaluates as you type.** No Run button even needed — edit the code and output /
  type errors / warnings update live. Implemented by compiling Gleam → JavaScript and running it
  *entirely in the browser* (no server round-trip).
  - Contrast with LOGOS: our examples run on the WASM interpreter via a Run button (also
    server-free) — same spirit; live-as-you-type would be the next increment.
- **"Everything on one page" view** — lets experienced readers Ctrl-F the whole language fast.
- **Assumes prior programming experience** and says so up front — frees it to skip basics and move
  quickly. (LOGOS targets *both* novices and experts; cf. its "How to Read This Guide" split.)
- **Community link (Discord) presented as part of the learning loop**, not a footer afterthought.
  (LOGOS already ends every section with a Discord CTA.)
