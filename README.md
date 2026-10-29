<div align="center">
  <img src="docs/logo.svg" width="200" height="200" alt="REST Client">
</div>

<h1 align="center">REST Client Extension for Zed</h1>

<p align="center">
  <a href="https://github.com/zed-ext/restclient/actions/workflows/ci.yml"><img src="https://github.com/zed-ext/restclient/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/zed-ext/restclient/releases/tag/v0.1.0"><img src="https://img.shields.io/github/v/release/zed-ext/restclient?color=blue" alt="Release"></a>
  <a href="https://github.com/zed-ext/restclient/blob/main/LICENSE"><img src="https://img.shields.io/github/license/zed-ext/restclient" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/language-Rust-orange" alt="Rust">
  <img src="https://img.shields.io/badge/platform-wasm32--wasip2-blueviolet" alt="WASM">
  <a href="https://github.com/zed-ext/restclient/stargazers"><img src="https://img.shields.io/github/stars/zed-ext/restclient?style=social" alt="Stars"></a>
</p>

<p align="center">
  A REST client extension for <a href="https://zed.dev">Zed</a> that lets you write and execute HTTP requests directly from <code>.http</code> and <code>.rest</code> files.
</p>

## Demo

![Demo](docs/video/demo.gif)

---

## Quick Start

### 1. Install the Extension

```bash
./install_to_zed.sh
```

### 2. Restart Zed

Completely quit Zed (`Cmd+Q` on macOS) and reopen it.

### 3. Try the Examples

Open any file from the `examples/` directory:

- `examples/basic.http` — GET, POST, PUT, PATCH, DELETE
- `examples/variables.http` — File-level variables, nested resolution
- `examples/response-capture.http` — Save values from responses
- `examples/edge-cases.http` — Error codes, slow requests, response formats
- `examples/environments.rest` — Environment variables (JetBrains-compatible)
- `examples/grpc.http` — gRPC calls: unary, streaming, TLS, metadata (requires [grpcurl](https://github.com/fullstorydev/grpcurl))

Or create your own `.http` file:

```http
### Test request
GET https://httpbin.org/get
```

### 4. Execute the Request

**Option A: Click the ▶ Send Button**

Look above each HTTP request — you'll see a clickable **▶ Send** button.
Click it to execute the request and see the response in the terminal panel.

**Option B: Send in New Tab**

Click the **⊕ Send in New Tab** button to open the response in a dedicated terminal tab (labeled with the request URL).

**Option C: Generate cURL**

Click the **📋 Generate cURL** button to copy the request as a cURL command to your clipboard.

**Option D: Using Command Palette**

1. Click anywhere in the request
2. Press `Cmd+Shift+P` (macOS) or `Ctrl+Shift+P` (Linux)
3. Type "task spawn"
4. Select "▶ Send Request", "⊕ Send in New Tab", or "📋 Generate cURL"

---

## Features

- **▶ Send Request** — Execute HTTP requests with a single click, reusing the same terminal
- **⊕ Send in New Tab** — Execute in a new terminal tab, titled with the request URL
- **📋 Generate cURL** — Copy any request as a cURL command to clipboard
- **Syntax Highlighting** — Full highlighting for `.http` and `.rest` files
- **Autocompletion** — HTTP methods, headers, header values, and variable references
- **Document Symbols** — Navigate requests via the symbol outline
- **Variable Support** — `@variable = value` declarations with `{{variable}}` substitution
- **Response Variable Capture** — Save values from responses with `client.global.set()` (JetBrains-compatible)
- **Environment Variables** — JetBrains-compatible `http-client.env.json` + `http-client.private.env.json` with parent directory search
- **Multiple Requests** — Use `###` separators for multiple requests per file
- **gRPC Requests** — `GRPC host:port/package.Service/Method` (JetBrains syntax), run with grpcurl: unary, server/client/bidirectional streaming, TLS, metadata
- **📋 Copy as grpcurl / 🔍 Describe gRPC** — Copy a gRPC request as a grpcurl command, or show the method signature and a request template
- **gRPC Completion** — Services, methods, and request message fields from server reflection or `.proto` files

---

## Writing HTTP Requests

### Basic Request

```http
### Simple GET request
GET https://httpbin.org/get
```

### POST with JSON Body

```http
### Create a user
POST https://api.example.com/users
Content-Type: application/json

{
  "name": "John Doe",
  "email": "john@example.com"
}
```

### Request with Headers

```http
### Authenticated request
GET https://api.example.com/data
Authorization: Bearer your-token-here
Accept: application/json
```

### Variables

```http
@baseUrl = https://api.example.com
@userId = 123

### Use variables with {{name}}
GET {{baseUrl}}/users/{{userId}}
```

Variables work in URLs, header values, and request bodies. Nested resolution is supported:

```http
@protocol = https
@host = api.example.com
@baseUrl = {{protocol}}://{{host}}

GET {{baseUrl}}/api/users
```

### Response Variable Capture

Save values from HTTP responses and reuse them in later requests (JetBrains HTTP Client syntax):

```http
### Login and capture the token
POST https://api.example.com/login
Content-Type: application/json

{
  "username": "demo",
  "password": "secret"
}

> {%
  client.global.set("token", response.body.access_token);
  client.global.set("user_id", response.body.user.id);
%}

###

### Use the captured token
GET https://api.example.com/users/{{user_id}}
Authorization: Bearer {{token}}
```

**Supported expressions:**

| Expression | Description |
|---|---|
| `response.body.field` | JSON field access (dot notation) |
| `response.body.data[0].id` | Array index access |
| `response.body.user.profile.name` | Nested field access |
| `response.headers.valueOf("X-Request-Id")` | Response header value |
| `response.status` | HTTP status code |
| `"string literal"` | Static string value |

Captured variables are persisted to `.http-client/.global-variables.json` and available across requests.

---

## gRPC Requests

The extension runs gRPC calls with [grpcurl](https://github.com/fullstorydev/grpcurl). The syntax is the [JetBrains HTTP Client gRPC syntax](https://www.jetbrains.com/help/idea/http-client-in-product-code-editor.html).

**Requirement:** install grpcurl (`brew install grpcurl` or `go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest`). To use a binary that is not on `PATH`, set `GRPCURL_PATH` to its full path.

### Unary Call

```http
### Say hello
GRPC grpcb.in:9000/hello.HelloService/SayHello
authorization: Bearer {{token}}

{
  "greeting": "Zed"
}
```

- Start the request line with `GRPC`, then `host:port/package.Service/Method`.
- Headers become gRPC metadata.
- The body is the request message in JSON.
- Variables work in the target, the metadata, and the body.

### TLS

| Target | Connection |
|---|---|
| `host:port/...` or `grpc://host:port/...` | Plaintext |
| `grpcs://host:port/...` | TLS |

Without a port, the extension uses 443 for TLS and 80 for plaintext.

### Streaming

| RPC type | Request body |
|---|---|
| Server streaming | One message. The responses show when they arrive. |
| Client streaming | The messages, one after another. |
| Bidirectional streaming | The messages, one after another. The client sends all of them, then reads the replies. |

```http
### Client streaming
GRPC grpcb.in:9000/hello.HelloService/LotsOfGreetings

{ "greeting": "Ada" }
{ "greeting": "Grace" }
```

### List Services and Methods

A target without a method lists the services. A target with only a service shows its methods.

```http
### List the services
GRPC grpcb.in:9000

### Show the methods of a service
GRPC grpcb.in:9000/hello.HelloService
```

### Directives

Put directives in comments before the request line. `@timeout` and `@connection-timeout` have the same names as in JetBrains.

| Directive | Effect (grpcurl flag) |
|---|---|
| `# @timeout 10` | Total time limit: `10`, `1.5s`, `500ms`, `2m` (`-max-time`) |
| `# @connection-timeout 2` | Connection time limit (`-connect-timeout`) |
| `# @proto ./api/service.proto` | Use a `.proto` file in place of server reflection (`-proto`). Repeat it for more files. |
| `# @import-path ./third_party` | Directory for `.proto` imports (`-import-path`) |
| `# @protoset ./api.protoset` | Compiled descriptor set (`-protoset`) |
| `# @insecure` | Skip TLS certificate verification (`-insecure`) |
| `# @cacert ./ca.pem` | Trusted root certificates (`-cacert`) |
| `# @cert ./client.pem` / `# @key ./client.key` | Client certificate for mutual TLS (`-cert` / `-key`) |
| `# @authority api.internal` | Value of the `:authority` header (`-authority`) |
| `# @emit-defaults` | Show fields that have default values (`-emit-defaults`) |

```http
### Call a server without reflection
# @proto ./proto/hello.proto
# @timeout 5
GRPC localhost:50051/hello.HelloService/SayHello
```

Relative paths start from the directory of the `.http` file. Directive values support `{{variables}}`.

### Client Certificates (SSLConfiguration)

As in JetBrains, an environment can have an `SSLConfiguration` object. Put it in `http-client.private.env.json`:

```json
{
  "development": {
    "SSLConfiguration": {
      "clientCertificate": "certs/client.pem",
      "clientCertificateKey": "certs/client.key",
      "verifyHostCertificate": false
    }
  }
}
```

| Key | grpcurl flag |
|---|---|
| `clientCertificate` | `-cert` |
| `clientCertificateKey` | `-key` |
| `"verifyHostCertificate": false` | `-insecure` |

Certificate paths start from the directory of the environment file. Directives in the request have priority.

### Response Capture

`client.global.set()` works with gRPC responses:

| Expression | gRPC value |
|---|---|
| `response.body.field` | Field of the response message. For a stream, the last message. |
| `response.status` | gRPC status code (`0` = OK) |
| `response.headers.valueOf("name")` | Response header or trailer |

### Gutter Buttons

| Button | Action |
|---|---|
| ▶ Send gRPC Request | Run the call in the terminal |
| ⊕ Send gRPC in New Tab | Run the call in a new terminal tab |
| 📋 Copy as grpcurl | Copy the equivalent grpcurl command to the clipboard |
| 🔍 Describe gRPC | Show the method signature, the RPC type, and a JSON template of the request |

### Completion

With server reflection, or with `# @proto` / `# @protoset`, the LSP completes:

- services after `GRPC host:port/`;
- methods after `GRPC host:port/package.Service/`, with the RPC type;
- the request message: a full JSON template on an empty body line, and the field names in the message;
- gRPC metadata keys and `# @` directives.

The LSP keeps grpcurl results for 60 seconds.

### Differences from JetBrains

- JetBrains runs unary and server-streaming calls only. This extension also runs client-streaming and bidirectional calls.
- Response handlers support the `client.global.set()` expressions above. They do not run JavaScript, so `onEachMessage` is not available.

---

## File Format

The extension supports the [JetBrains HTTP Client file format](https://www.jetbrains.com/help/idea/http-client-in-product-code-editor.html):

```http
### Comment describing the request
METHOD URL
Header-Name: Header-Value

Request Body
```

Use `###` to separate multiple requests. Lines starting with `#` or `//` are comments.
Start a request with `GRPC` for a gRPC call (see [gRPC Requests](#grpc-requests)).

> **Tip:** Use `### Section Title` for section dividers between requests. Standalone `# comment` lines
> placed after a blank line following a request may not be highlighted correctly, because the
> tree-sitter HTTP grammar can interpret them as request body content. Using `###` ensures
> consistent syntax highlighting throughout the file.

---

## Environment Variables

Environment variables follow the [JetBrains HTTP Client convention](https://www.jetbrains.com/help/idea/http-client-in-product-code-editor.html). Place environment files alongside your `.http`/`.rest` files:

```
your-project/
├── http-client.env.json           # Public environment variables (commit to git)
├── http-client.private.env.json   # Private overrides (gitignored — secrets go here)
├── .http-client/
│   └── .global-variables.json     # Persisted response capture variables
├── api.http
└── api.rest
```

**`http-client.env.json`** (public — safe to commit):

```json
{
  "activeEnv": "development",
  "development": {
    "base_url": "https://httpbin.org",
    "api_url": "https://httpbin.org",
    "timeout": "5000"
  },
  "staging": {
    "base_url": "https://staging.example.com",
    "api_url": "https://staging.example.com/api"
  },
  "production": {
    "base_url": "https://api.example.com",
    "api_url": "https://api.example.com/api"
  }
}
```

**`http-client.private.env.json`** (private — add to `.gitignore`):

```json
{
  "development": {
    "api_key": "dev-key-12345",
    "auth_token": "dev-token-abcdef"
  },
  "staging": {
    "api_key": "staging-key-67890",
    "auth_token": "staging-token-ghijkl"
  }
}
```

Private variables override public ones with the same name. The extension searches the directory containing your `.http` file first, then walks up parent directories.

The `"activeEnv"` key in `http-client.env.json` specifies which environment is active. Change it to switch environments:

```json
"activeEnv": "staging"
```

Then reference environment variables in your requests:

```http
### Uses base_url from the active environment
GET {{base_url}}/api/users
Authorization: Bearer {{auth_token}}
```

**Variable resolution order** (highest priority first):

1. File-level variables (`@variable = value`)
2. Global variables (captured from response handlers)
3. Active environment variables
4. System environment variables

See `examples/environments.rest` for a full working example.

---

## Development

### Building

```bash
# Build the WASM extension
cargo build --release --target wasm32-wasip2

# Build the LSP server and CLI
cd lsp && cargo build --release
```

### Installing

```bash
./install_to_zed.sh
```

### Project Structure

```
restclient/
├── src/
│   └── lib.rs              # WASM extension entry point
├── lsp/
│   ├── src/
│   │   ├── main.rs         # LSP server (completions, symbols, diagnostics)
│   │   ├── parser.rs       # HTTP file parser
│   │   ├── grpc.rs         # gRPC targets, grpcurl arguments, grpcurl output parser
│   │   ├── variables.rs    # Variable resolution
│   │   ├── response_handler.rs  # Response variable capture
│   │   └── bin/
│   │       └── http-client.rs  # CLI binary for executing requests
│   └── Cargo.toml
├── languages/http/
│   ├── config.toml         # Language configuration
│   ├── highlights.scm      # Syntax highlighting
│   ├── runnables.scm       # Tree-sitter runnable captures
│   └── tasks.json          # Task definitions (HTTP and gRPC buttons)
├── examples/
│   ├── http-client.env.json       # Public environment config
│   ├── http-client.private.env.json # Private environment config
│   ├── .http-client/
│   │   └── .global-variables.json # Response capture storage
│   ├── basic.http          # GET, POST, PUT, PATCH, DELETE examples
│   ├── variables.http      # Variable substitution examples
│   ├── response-capture.http  # Response variable capture examples
│   ├── edge-cases.http     # Error codes, slow requests, formats
│   ├── environments.rest   # Environment variables example
│   ├── grpc.http           # gRPC calls (requires grpcurl)
│   └── proto/hello.proto   # Schema for the @proto example
├── grammars/               # Tree-sitter grammar (fork of tree-sitter-http with the GRPC method)
├── extension.toml          # Extension manifest
└── Cargo.toml              # WASM crate config
```

---

## Troubleshooting

### ▶ Send buttons don't appear

1. Restart Zed completely (`Cmd+Q` and reopen)
2. Check LSP is running: View → Server Logs → HTTP LSP
3. Reinstall: `./install_to_zed.sh`

### Syntax highlighting not working

1. Check file extension is `.http` or `.rest`
2. Restart Zed completely
3. Reinstall: `./install_to_zed.sh`

### gRPC requests fail with "grpcurl not found"

1. Install grpcurl: `brew install grpcurl`
2. If grpcurl is not on `PATH`, set `GRPCURL_PATH` to its full path

### gRPC completion shows nothing

1. Check that the server supports reflection: `grpcurl -plaintext host:port list`
2. If the server has no reflection, add `# @proto <file.proto>` before the request line

---

## License

MIT

## Contributing

Contributions welcome! Please open an issue or PR. See [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) for details.
