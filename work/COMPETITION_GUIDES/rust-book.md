# The Rust Programming Language ("the Book")

Source: <https://doc.rust-lang.org/book/> · Interactive (Brown U.) variant: <https://rust-book.cs.brown.edu>

## Structure (chapter arc)

1. Getting Started — install, `cargo new`, anatomy of a program
2. **Programming a Guessing Game** — a *complete, real program* in chapter 2, before any reference
3. Common Programming Concepts — variables, types, functions, comments, control flow
4. **Understanding Ownership** — a dedicated chapter (ownership, references/borrowing, slices) with diagrams
5. Structs
6. Enums and Pattern Matching
7. Packages, Crates, Modules
8. Common Collections — Vec, String, HashMap
9. Error Handling — `panic!`, `Result`, `?`
10. Generic Types, Traits, Lifetimes
11. Writing Automated Tests
12. **An I/O Project: building a command-line `grep`** — second end-to-end project
13. Iterators and Closures
14. Cargo and Crates.io
15. Smart Pointers
16. Fearless Concurrency
17. OOP features
18. Patterns and Matching
19. Advanced Features (unsafe, advanced traits/types, macros)
20. **Final Project: a multithreaded web server** — third end-to-end project
- Appendices: A keywords · B operators & symbols · C derivable traits · D dev tools · E editions

## Standout pedagogical / QOL features

- **Three project-based chapters** (2, 12, 20) interleaved with reference — learn by *building*, not just reading.
- **Ownership taught as its own first-class chapter** with mental-model diagrams, before structs/enums.
- **Ordering discipline:** "common concepts" → ownership → composite types → collections → errors. Each builds on the last; nothing is used before it's introduced.
- **Rich reference appendices** (every keyword, every operator/symbol, derivable traits, tooling).
- **Brown University interactive edition:** inline **quizzes** after sections, highlightable text, and **ownership/borrow visualizations** — comprehension checks baked into the read.
- Accessibility: in-guide **search** (press `S`), **theme** options, keyboard shortcuts, offline `rustup doc --book`.
- Community translations; stable + edition-pinned so examples never rot.
