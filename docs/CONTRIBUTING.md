# Contributing to REST Client for Zed

Thank you for your interest in contributing!

## Prerequisites

- Rust (stable) via [rustup](https://rustup.rs/)
- Zed IDE
- `wasm32-wasip2` target: `rustup target add wasm32-wasip2`
- [grpcurl](https://github.com/fullstorydev/grpcurl) (optional, to run gRPC requests): `brew install grpcurl`
- [tree-sitter CLI](https://tree-sitter.github.io/tree-sitter/) (optional, to change the grammar)

## Development Setup

```bash
git clone https://github.com/anthropics/zed-restclient.git
cd zed-restclient

cargo build --release --target wasm32-wasip2

cd lsp && cargo build --release && cd ..

./install_to_zed.sh
```

Restart Zed completely after installing.

## Project Architecture

```
restclient/
├── src/
│   └── lib.rs                  # WASM extension entry point (Zed integration)
├── lsp/
│   ├── src/
│   │   ├── main.rs             # LSP server (code lens, completions, symbols, runnables)
│   │   ├── parser.rs           # HTTP file parser
│   │   ├── grpc.rs             # gRPC targets, grpcurl arguments, grpcurl output parser
│   │   ├── convert.rs          # curl/grpcurl command → request block (paste and code action)
│   │   ├── body.rs             # HTTP request body: inline, < file, multipart
│   │   ├── variables.rs        # Variable resolution engine
│   │   ├── lib.rs              # Shared library exports
│   │   └── bin/
│   │       └── http-client.rs  # CLI binary (colored output, spinner, request execution)
│   └── Cargo.toml
├── languages/http/
│   ├── config.toml             # Language configuration
│   ├── highlights.scm          # Syntax highlighting queries
│   ├── runnables.scm           # Tree-sitter runnable captures (▶ buttons)
│   └── tasks.json              # Task definitions (HTTP and gRPC buttons)
├── grammars/                   # Tree-sitter HTTP grammar (http.wasm + http.rev)
├── extension.toml              # Extension manifest
├── Cargo.toml                  # WASM crate config
└── install_to_zed.sh           # Build + install script
```

### Key Components

**WASM Extension** (`src/lib.rs`): Entry point registered with Zed. Locates and launches the LSP binary. Cannot use tokio/reqwest (WASM constraint).

**LSP Server** (`lsp/src/main.rs`): Provides code lens (Generate cURL), completions, document symbols, runnables, and code actions. It converts pasted curl/grpcurl commands with `workspace/applyEdit`. Independent workspace crate.

**Command Conversion** (`lsp/src/convert.rs`): Splits shell text into words, converts `curl` and `grpcurl` arguments to request blocks, and finds pasted commands. `did_change` compares the old and the new text. The inserted text must start with a command, so typing never converts.

**Request Body** (`lsp/src/body.rs`): Builds the bytes of an HTTP body for the CLI: inline text, `< ./file` references, and `multipart/form-data` bodies with `< ./file` parts (CRLF line breaks).

**HTTP Parser** (`lsp/src/parser.rs`): Parses `.http` files into structured request blocks. Handles methods (including `GRPC`), headers, bodies, variables, `# @directive` comments, and `###` separators.

**gRPC Module** (`lsp/src/grpc.rs`): Parses `GRPC` targets, builds grpcurl arguments from directives and `SSLConfiguration`, and parses `grpcurl -v` output. It runs no processes, so its unit tests do not need grpcurl. The CLI runs grpcurl for requests, and the LSP runs it for completion.

**Variable Resolver** (`lsp/src/variables.rs`): Resolves `{{variable}}` references with support for file-level `@var = value`, environment files, and nested resolution.

**CLI Binary** (`lsp/src/bin/http-client.rs`): Executes HTTP requests from the terminal, and gRPC requests through grpcurl. Features colored JSON output, braille spinner with elapsed time, and precise timing display.

**Grammar** (`grammars/`, `[grammars.http]` in `extension.toml`): A fork of [rest-nvim/tree-sitter-http](https://github.com/rest-nvim/tree-sitter-http) that adds `GRPC` to the `method` rule. `install_to_zed.sh` rebuilds `grammars/http.wasm` when the `rev` in `extension.toml` differs from `grammars/http.rev`. To change the grammar:

1. Edit `grammar.js` in the fork.
2. Run `tree-sitter generate --abi 14` and `tree-sitter test`.
3. Commit and push the fork, including `src/`.
4. Update `rev` in `extension.toml`, then run `./install_to_zed.sh`.

## Development Workflow

1. Make changes
2. Build and test:
   ```bash
   cd lsp && cargo test && cargo clippy -- -D warnings && cd ..
   cargo build --release --target wasm32-wasip2
   ```
3. Install and test in Zed:
   ```bash
   ./install_to_zed.sh
   # Restart Zed, open a .http file, verify changes
   ```

## Code Style

```bash
cargo fmt
cargo clippy -- -D warnings
```

Both the root WASM crate and the `lsp/` crate should pass formatting and clippy checks.

## Pull Request Process

1. Fork the repository
2. Create a feature branch
3. Make your changes with tests
4. Ensure all checks pass:
   ```bash
   cargo fmt -- --check
   cargo clippy --target wasm32-wasip2 -- -D warnings
   cd lsp && cargo fmt -- --check && cargo clippy -- -D warnings && cargo test
   ```
5. Open a PR with a clear description of what and why

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
