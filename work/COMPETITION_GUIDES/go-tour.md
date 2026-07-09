# A Tour of Go

Source: <https://go.dev/tour/> · Section list: <https://go.dev/tour/list>

## Structure

1. Basics — packages, imports, exported names, functions, variables, types, constants
2. Flow control — for, if/else, switch, defer
3. More types — pointers, structs, arrays, slices, maps, function values, closures
4. Methods and interfaces — methods, interfaces, type assertions, type switches, Stringer, errors, Readers
5. Generics — type parameters, generic types
6. Concurrency — goroutines, channels, buffered channels, select, sync.Mutex

## Standout pedagogical / QOL features

- **Every page is a runnable, editable playground.** Read prose on the left, edit Go on the right, hit Run, see output inline. No install. (LOGOS's `GuideCodeBlock` is the same idea — a genuine strength to keep leaning on.)
- **Integrated exercises** ("Exercise: Loops and Functions", "Exercise: Slices", "Exercise: rot13Reader", "Exercise: Equivalent Binary Trees", "Exercise: Web Crawler") — the reader *writes* code, not just runs samples. Solutions exist for self-check.
- **Strictly progressive**: procedural → types → methods/interfaces → generics → concurrency. Concurrency last, after the type system is solid.
- **Tiny, single-concept pages** — one idea per page keeps each runnable example minimal and focused.
- Self-hostable / offline (`go tool tour`).
