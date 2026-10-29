# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- gRPC requests with the JetBrains syntax `GRPC host:port/package.Service/Method`, executed with grpcurl
- Plaintext (`grpc://` or no scheme) and TLS (`grpcs://`) connections
- Unary, server-streaming, client-streaming, and bidirectional calls; server-stream messages show when they arrive
- gRPC metadata from headers, JSON request messages from the body or from a `< file` reference
- `GRPC host` lists the services; `GRPC host/package.Service` shows the methods
- Request directives: `@timeout`, `@connection-timeout`, `@proto`, `@import-path`, `@protoset`, `@insecure`, `@cacert`, `@cert`, `@key`, `@authority`, `@emit-defaults`
- gRPC tasks: ▶ Send gRPC Request, ⊕ Send gRPC in New Tab, 📋 Copy as grpcurl, 🔍 Describe gRPC
- gRPC completion: services, methods, request message template and fields (server reflection or `.proto` files), metadata keys
- Completion of `# @` directives
- Response capture for gRPC: `response.body` (last message), `response.status` (gRPC code), `response.headers.valueOf()` (headers and trailers)
- JetBrains `SSLConfiguration` in environment files (`clientCertificate`, `clientCertificateKey`, `verifyHostCertificate`)
- `examples/grpc.http` and `examples/proto/hello.proto`

### Changed
- Grammar: fork of tree-sitter-http with the `GRPC` method, so `GRPC` highlights as a keyword
- `install_to_zed.sh` builds the grammar at the `rev` of `extension.toml` and checks for grpcurl

### Fixed
- macOS: gRPC requests and gRPC completions stopped at "Waiting..." when Zed started them. Child processes get the Mach exception ports of Zed's crash handler, and grpcurl (Go) stops at its first memory fault. `http-client` and `http-lsp` now reset these ports at startup
- Environment files with number, boolean, or object values (for example `SSLConfiguration`) failed to load
- `install_to_zed.sh` built a `wasm32-wasip1` core module, which Zed cannot load ("attempted to parse a wasm module with a component parser"), so the LSP never started. The extension now builds for `wasm32-wasip2` (a WebAssembly component), and the script stops if the output is not a component

## [0.2.1] - 2026-03-14

### Added
- ▶ Send Request button (reuses terminal) and ⊕ Send in New Tab button (new terminal per request)
- Color-cycling braille spinner with elapsed time during requests
- Syntax-highlighted JSON response output (keys, strings, numbers, booleans, null)
- Precise elapsed time display (e.g. `3.315s`, `42ms`)
- Hidden task runner output lines for clean terminal display

### Changed
- Cleaned up codebase for open-source release
- Removed unused dependencies and dead code
- Updated documentation to reflect current architecture

## [0.2.0] - 2026-03-10

### Added
- LSP code lens for "Generate cURL" command
- HTTP autocompletion (methods, headers, header values, variable references)
- Document symbols for request navigation
- Variable support with `@var = value` and `{{var}}` substitution
- Environment file support (`.http-client/environments.json`)
- Nested variable resolution

## [0.1.0] - 2026-03-07

### Added
- Initial release
- HTTP file parser supporting JetBrains HTTP Client format
- Syntax highlighting for `.http` and `.rest` files
- Tree-sitter grammar integration
