# logicaffeine-lsp

[![Crates.io](https://img.shields.io/crates/v/logicaffeine-lsp.svg)](https://crates.io/crates/logicaffeine-lsp)
[![Documentation](https://docs.rs/logicaffeine-lsp/badge.svg)](https://docs.rs/logicaffeine-lsp)
[![License: BUSL-1.1](https://img.shields.io/badge/License-BUSL--1.1-blue.svg)](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md)

Language Server Protocol implementation for [LogicAffeine](https://logicaffeine.com), providing rich IDE integration with diagnostics, completion, hover documentation, refactoring, and more.

## Features

The `logicaffeine-lsp` server implements 14 LSP features organized into four categories:

### Code Intelligence

- **Diagnostics** - Real-time syntax and semantic error detection with actionable error messages
- **Hover** - Type information, documentation, and definition context on hover
- **Semantic Tokens** - Syntax highlighting based on semantic meaning (variables, keywords, types)

### Navigation

- **Go to Definition** - Jump to where a variable, function, or type is defined
- **Find References** - Find all usages of a symbol across the codebase
- **Document Symbols** - Outline view showing all declarations in the file
- **Code Lens** - Inline reference counts and navigation actions

### Refactoring

- **Rename** - Safely rename symbols across all references with preview
- **Code Actions** - Quick fixes and refactoring suggestions (e.g., "Extract to function")

### Editing

- **Completion** - Context-aware autocomplete for keywords, variables, and functions
- **Signature Help** - Parameter hints for function calls
- **Inlay Hints** - Inline type annotations and parameter names

### Display

- **Folding Ranges** - Collapsible code regions for functions, blocks, and comments
- **Formatting** - Automatic code formatting with consistent style

## Installation

### From crates.io (Recommended)

```bash
cargo install logicaffeine-lsp
```

The binary will be installed to `~/.cargo/bin/logicaffeine-lsp`.

### Build from Source

```bash
git clone https://github.com/Brahmastra-Labs/logicaffeine.git
cd logicaffeine
cargo build --release -p logicaffeine-lsp
```

The binary will be at `target/release/logicaffeine-lsp`.

### Binary Downloads

Pre-built binaries are available for multiple platforms from [GitHub Releases](https://github.com/Brahmastra-Labs/logicaffeine/releases):

- Linux x86_64
- Linux ARM64
- macOS x86_64 (Intel)
- macOS ARM64 (Apple Silicon)
- Windows x86_64

## Editor Integration

### VSCode

The official VSCode extension is available at `editors/vscode/logicaffeine/` in the repository.

**Manual configuration** in `settings.json`:

```json
{
  "logicaffeine.lsp.serverPath": "/path/to/logicaffeine-lsp",
  "logicaffeine.lsp.trace.server": "verbose"
}
```

If you installed via `cargo install`, the path is typically:
- **Linux/macOS**: `~/.cargo/bin/logicaffeine-lsp`
- **Windows**: `%USERPROFILE%\.cargo\bin\logicaffeine-lsp.exe`

### Neovim

Using [`nvim-lspconfig`](https://github.com/neovim/nvim-lspconfig):

```lua
local lspconfig = require('lspconfig')
local configs = require('lspconfig.configs')

-- Register logicaffeine-lsp if not already registered
if not configs.logicaffeine_lsp then
  configs.logicaffeine_lsp = {
    default_config = {
      cmd = { 'logicaffeine-lsp' },
      filetypes = { 'logos' },
      root_dir = lspconfig.util.root_pattern('.git', 'Project.toml'),
      settings = {},
    },
  }
end

-- Enable for .logos files
lspconfig.logicaffeine_lsp.setup{
  on_attach = function(client, bufnr)
    -- Optional: Add custom keybindings here
  end,
}
```

Add to your Neovim configuration (e.g., `~/.config/nvim/lua/lsp.lua`).

### Emacs (lsp-mode)

Add to your Emacs configuration:

```elisp
(require 'lsp-mode)

;; Register .logos files
(add-to-list 'lsp-language-id-configuration '(logos-mode . "logos"))

;; Register logicaffeine-lsp client
(lsp-register-client
 (make-lsp-client :new-connection (lsp-stdio-connection "logicaffeine-lsp")
                  :major-modes '(logos-mode)
                  :server-id 'logicaffeine-lsp))

;; Enable for .logos files
(add-hook 'logos-mode-hook #'lsp)
```

### Emacs (eglot)

Add to your Emacs configuration:

```elisp
(require 'eglot)

;; Register logicaffeine-lsp for .logos files
(add-to-list 'eglot-server-programs '(logos-mode . ("logicaffeine-lsp")))

;; Enable for .logos files
(add-hook 'logos-mode-hook #'eglot-ensure)
```

### Sublime Text

1. Install the [LSP package](https://packagecontrol.io/packages/LSP) via Package Control
2. Open **Preferences → Package Settings → LSP → Settings**
3. Add the logicaffeine-lsp configuration:

```json
{
  "clients": {
    "logicaffeine-lsp": {
      "enabled": true,
      "command": ["logicaffeine-lsp"],
      "selector": "source.logos",
      "schemes": ["file"]
    }
  }
}
```

4. Create a syntax definition for `.logos` files with the scope `source.logos`

## Architecture

The language server integrates LogicAffeine's compilation pipeline with the LSP protocol:

```text
LSP Client (Editor)
     │
     ▼
┌─────────────────────────────────────────┐
│     Language Server (tower-lsp)         │
│  ┌─────────────────────────────────┐   │
│  │   Document State Management      │   │
│  │   (incremental sync, indexing)   │   │
│  └─────────────────────────────────┘   │
└──────────────┬──────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────┐
│     LogicAffeine Pipeline               │
│  ┌──────┐  ┌────────┐  ┌──────────┐   │
│  │Lexer │→ │ Parser │→ │ Analysis │   │
│  └──────┘  └────────┘  └──────────┘   │
└─────────────────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────┐
│          LSP Features                    │
│  Diagnostics, Hover, Completion, etc.   │
└─────────────────────────────────────────┘
```

### Key Components

1. **Server** (`server.rs`) - Tower-LSP server handling client communication via JSON-RPC
2. **State** (`state.rs`) - Workspace and document state management with incremental updates
3. **Pipeline** (`pipeline.rs`) - Integration with LogicAffeine's lexer, parser, and semantic analysis
4. **Index** (`index.rs`) - Symbol indexing for cross-file references and workspace-wide queries
5. **Feature Modules** - Individual LSP capabilities (diagnostics, hover, completion, etc.)

### Performance Optimizations

- **Incremental sync**: Only re-analyzes changed documents, not the entire workspace
- **Parallel analysis**: Uses `DashMap` for concurrent document processing
- **Lazy indexing**: Symbol index built on-demand for referenced files only
- **REPL-optimized**: Workspace state persists across document changes for fast iteration

## Development

### Running Tests

The crate includes 179 comprehensive tests covering all LSP features:

```bash
cargo test -p logicaffeine-lsp --lib
```

### Building the Binary

```bash
cargo build --release -p logicaffeine-lsp
```

### Running with Debug Logging

```bash
RUST_LOG=debug logicaffeine-lsp
```

The server communicates via stdin/stdout, so logs are written to stderr. Most editors capture stderr for debugging.

### Testing in VSCode

1. Build the debug binary: `cargo build -p logicaffeine-lsp`
2. Update VSCode settings to use the debug build: `"logicaffeine.lsp.serverPath": "/path/to/target/debug/logicaffeine-lsp"`
3. Open a `.logos` file and check the **Output → LogicAffeine Language Server** panel for logs

## Related Crates

- [`logicaffeine-language`](https://crates.io/crates/logicaffeine-language) - Core LOGOS language implementation (lexer, parser, AST)
- [`logicaffeine-compile`](https://crates.io/crates/logicaffeine-compile) - Code generation and compilation
- [`logicaffeine-proof`](https://crates.io/crates/logicaffeine-proof) - Proof assistant integration
- [`logicaffeine-cli`](https://crates.io/crates/logicaffeine-cli) - Command-line interface (`largo`)

## Dependencies

### Internal

- `logicaffeine-base` - Arena allocation, string interning, source spans
- `logicaffeine-language` - Lexer, parser, AST
- `logicaffeine-compile` - Semantic analysis and codegen
- `logicaffeine-proof` - Proof verification

### External

- `tower-lsp` - LSP server framework
- `tokio` - Async runtime
- `dashmap` - Concurrent document storage
- `serde`/`serde_json` - LSP message serialization
- `log`/`env_logger` - Logging infrastructure

## Contributing

Contributions are welcome! Please see the main [LogicAffeine repository](https://github.com/Brahmastra-Labs/logicaffeine) for contribution guidelines.

### Adding New LSP Features

1. Create a new module in `src/` (e.g., `src/new_feature.rs`)
2. Implement the LSP capability using the pipeline integration
3. Add tests in the module following existing patterns
4. Register the feature in `server.rs` capabilities
5. Update this README and `lib.rs` documentation

## License

Business Source License 1.1 (BUSL-1.1)

- **Free** for individuals and organizations with <25 employees
- **Commercial license** required for organizations with 25+ employees offering Logic Services
- **Converts to MIT** on December 24, 2029

See [LICENSE](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md) for full terms.

## Links

- [LogicAffeine Website](https://logicaffeine.com)
- [Documentation](https://docs.rs/logicaffeine-lsp)
- [GitHub Repository](https://github.com/Brahmastra-Labs/logicaffeine)
- [Issue Tracker](https://github.com/Brahmastra-Labs/logicaffeine/issues)
- [Crates.io](https://crates.io/crates/logicaffeine-lsp)
