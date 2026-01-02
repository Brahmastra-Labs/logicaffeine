# Contributing to Logicaffeine

Thank you for your interest in contributing to Logicaffeine! This document provides guidelines for contributing to the project.

## Getting Started

### Prerequisites

- Rust (latest stable)
- For verification features: Z3 SMT solver

### Building

```bash
# Clone the repository
git clone https://github.com/Brahmastra-Labs/logicaffeine.git
cd logicaffeine

# Build the project
cargo build

# Run the test suite
cargo test -- --skip e2e
```

### Running Tests

```bash
# Run all tests (excluding e2e)
cargo test -- --skip e2e

# Run a specific phase
cargo test --test phase5_wh_movement

# Run a specific test
cargo test wh_embedded_clause

# Run with output
cargo test -- --nocapture
```

## Development Workflow

We follow **Test-Driven Development (TDD)**:

1. **RED** - Write a failing test first
2. **GREEN** - Write minimal code to make it pass
3. **REFACTOR** - Clean up while keeping tests green

### Important Rules

- **Never modify failing tests** - Tests define the spec. If a test fails, fix the implementation, not the test.
- **Keep changes focused** - Don't add unrelated improvements in the same PR.
- **No over-engineering** - Only add what's needed for the current task.

## Code Style

- Arena allocation for all AST nodes
- Symbol interning for all strings
- Guard pattern for parser backtracking
- Visitor pattern for AST traversal

## Pull Request Process

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Make your changes following TDD
4. Ensure all tests pass
5. Submit a pull request

### PR Guidelines

- Include a clear description of what changes were made and why
- Reference any related issues
- Keep commits focused and atomic
- Update documentation if needed

## Project Structure

```
logicaffeine/
├── assets/lexicon.json    # Vocabulary database
├── src/
│   ├── lexer.rs          # Tokenization
│   ├── parser/           # Parser modules
│   ├── ast/              # AST types
│   ├── transpile.rs      # FOL output generation
│   └── codegen.rs        # Rust code generation
├── logos_core/           # Runtime library
├── logos_verification/   # Z3 verification (optional)
└── tests/                # Phase-organized tests
```

## Test Phases

Tests are organized by linguistic phenomenon (Phases 1-43). See the README for a complete list.

## Questions?

- Open an issue for bugs or feature requests
- Check existing issues before creating new ones

## License

By contributing, you agree that your contributions will be licensed under the project's BSL-1.1 license.
