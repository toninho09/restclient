# Quick Setup Guide

## Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Zed IDE](https://zed.dev/)
- [grpcurl](https://github.com/fullstorydev/grpcurl) (optional, for gRPC requests): `brew install grpcurl`

## Setup Steps

### 1. Clone and Install

```bash
git clone https://github.com/anthropics/zed-restclient.git
cd zed-restclient
./install_to_zed.sh
```

### 2. Restart Zed

Completely quit Zed (`Cmd+Q` on macOS) and reopen it.

### 3. Test It

Create a file called `test.http`:

```http
### Test request
GET https://httpbin.org/get
```

You should see **▶ Send Request** and **⊕ Send in New Tab** buttons above the request. Click either one to execute.

To test gRPC (requires grpcurl), add:

```http
### Test gRPC request
GRPC grpcb.in:9000/hello.HelloService/SayHello

{
  "greeting": "Zed"
}
```

## Troubleshooting

**▶ buttons don't appear:**
- Restart Zed completely (`Cmd+Q` and reopen)
- Check LSP is running: View → Server Logs → HTTP LSP
- Reinstall: `./install_to_zed.sh`

**Syntax highlighting not working:**
- Check file extension is `.http` or `.rest`
- Restart Zed completely

**gRPC requests fail with "grpcurl not found":**
- Install grpcurl: `brew install grpcurl`
- Or set `GRPCURL_PATH` to the full path of the grpcurl binary

See the [README](../README.md) for full documentation.
