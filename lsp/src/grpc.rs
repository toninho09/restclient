//! gRPC support for `GRPC` requests (JetBrains HTTP Client syntax).
//! This module parses targets, builds `grpcurl` arguments and parses `grpcurl` output.
//! Only `find_grpcurl` and `request_data` touch the file system. Nothing here runs a process.

use crate::parser::{HttpRequest, RequestBody};
use crate::variables::SslConfiguration;
use serde_json::Value;
use std::env;
use std::path::{Path, PathBuf};

pub const DEFAULT_TLS_PORT: u16 = 443;
pub const DEFAULT_PLAINTEXT_PORT: u16 = 80;

/// Environment variable that overrides the grpcurl location
pub const GRPCURL_PATH_ENV: &str = "GRPCURL_PATH";

pub const GRPCURL_INSTALL_HINT: &str = "Install grpcurl: `brew install grpcurl` or \
`go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest`. \
Set GRPCURL_PATH to use a binary that is not on PATH.";

/// gRPC status code names (index = code)
const STATUS_NAMES: [&str; 17] = [
    "OK",
    "CANCELLED",
    "UNKNOWN",
    "INVALID_ARGUMENT",
    "DEADLINE_EXCEEDED",
    "NOT_FOUND",
    "ALREADY_EXISTS",
    "PERMISSION_DENIED",
    "RESOURCE_EXHAUSTED",
    "FAILED_PRECONDITION",
    "ABORTED",
    "OUT_OF_RANGE",
    "UNIMPLEMENTED",
    "INTERNAL",
    "UNAVAILABLE",
    "DATA_LOSS",
    "UNAUTHENTICATED",
];

/// grpcurl exits with `64 + status code` when the RPC returns a non-OK status
const GRPCURL_STATUS_EXIT_BASE: i32 = 64;

/// The request line of a `GRPC` request: `GRPC [grpc://|grpcs://]host[:port][/pkg.Service[/Method]]`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcTarget {
    pub tls: bool,
    /// `host:port`, with the default port added when the URL has no port
    pub authority: String,
    pub service: Option<String>,
    pub method: Option<String>,
}

impl GrpcTarget {
    /// Method symbol for invocation: `pkg.Service/Method`
    pub fn method_symbol(&self) -> Option<String> {
        Some(format!(
            "{}/{}",
            self.service.as_ref()?,
            self.method.as_ref()?
        ))
    }

    /// Method symbol for `describe`: `pkg.Service.Method`
    pub fn describe_symbol(&self) -> Option<String> {
        Some(format!(
            "{}.{}",
            self.service.as_ref()?,
            self.method.as_ref()?
        ))
    }
}

/// Parse the URL part of a `GRPC` request line.
/// No scheme, `grpc://` or `http://` means plaintext. `grpcs://` or `https://` means TLS.
pub fn parse_target(url: &str) -> Result<GrpcTarget, String> {
    let url = url.trim();
    let (tls, rest) = split_scheme(url);

    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx + 1..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err(format!("Missing host in gRPC target: {}", url));
    }

    let authority = if has_port(authority) {
        authority.to_string()
    } else {
        let port = if tls {
            DEFAULT_TLS_PORT
        } else {
            DEFAULT_PLAINTEXT_PORT
        };
        format!("{}:{}", authority, port)
    };

    let segments: Vec<&str> = path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let (service, method) = match segments.as_slice() {
        [] => (None, None),
        [service] => (Some(service.to_string()), None),
        [service, method] => (Some(service.to_string()), Some(method.to_string())),
        _ => {
            return Err(format!(
                "Invalid gRPC path in {}: expected /package.Service/Method",
                url
            ))
        }
    };

    Ok(GrpcTarget {
        tls,
        authority,
        service,
        method,
    })
}

/// Split the scheme from a target. Return `(tls, rest)`.
fn split_scheme(url: &str) -> (bool, &str) {
    for (scheme, tls) in [
        ("grpcs://", true),
        ("https://", true),
        ("grpc://", false),
        ("http://", false),
    ] {
        if url.len() >= scheme.len() && url[..scheme.len()].eq_ignore_ascii_case(scheme) {
            return (tls, &url[scheme.len()..]);
        }
    }
    (false, url)
}

/// Return true if `host:port` has a port. Supports IPv6 in brackets: `[::1]:50051`.
fn has_port(authority: &str) -> bool {
    if authority.starts_with('[') {
        authority.contains("]:")
    } else {
        authority.contains(':')
    }
}

/// grpcurl options from request directives and the environment `SSLConfiguration`
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GrpcOptions {
    pub import_paths: Vec<PathBuf>,
    pub protos: Vec<PathBuf>,
    pub protosets: Vec<PathBuf>,
    pub insecure: bool,
    pub cacert: Option<PathBuf>,
    pub cert: Option<PathBuf>,
    pub key: Option<PathBuf>,
    pub authority: Option<String>,
    /// Seconds (`@timeout`)
    pub max_time: Option<f64>,
    /// Seconds (`@connection-timeout`)
    pub connect_timeout: Option<f64>,
    pub emit_defaults: bool,
}

impl GrpcOptions {
    /// Read the directives of a request. Relative paths resolve against `base_dir`
    /// (the directory of the .http file). Directives win over `ssl`.
    pub fn from_request(
        request: &HttpRequest,
        base_dir: &Path,
        ssl: Option<&SslConfiguration>,
    ) -> Result<Self, String> {
        let meta = &request.metadata;
        // `components()` removes `.` segments: /project/./proto → /project/proto
        let path = |value: &str| base_dir.join(value).components().collect::<PathBuf>();

        let mut opts = GrpcOptions {
            import_paths: meta.directive_values("import-path").map(path).collect(),
            protos: meta.directive_values("proto").map(path).collect(),
            protosets: meta.directive_values("protoset").map(path).collect(),
            insecure: meta.has_directive("insecure"),
            cacert: meta.directive("cacert").map(path),
            cert: meta.directive("cert").map(path),
            key: meta.directive("key").map(path),
            authority: meta.directive("authority").map(|v| v.to_string()),
            max_time: meta.directive("timeout").map(parse_duration).transpose()?,
            connect_timeout: meta
                .directive("connection-timeout")
                .map(parse_duration)
                .transpose()?,
            emit_defaults: meta.has_directive("emit-defaults"),
        };

        // grpcurl needs an import path for each absolute -proto file
        for proto in &opts.protos {
            if let Some(dir) = proto.parent() {
                let dir = dir.to_path_buf();
                if !opts.import_paths.contains(&dir) {
                    opts.import_paths.push(dir);
                }
            }
        }

        if let Some(ssl) = ssl {
            if ssl.verify_host_certificate == Some(false) {
                opts.insecure = true;
            }
            if opts.cert.is_none() {
                opts.cert.clone_from(&ssl.client_certificate);
            }
            if opts.key.is_none() {
                opts.key.clone_from(&ssl.client_certificate_key);
            }
        }

        Ok(opts)
    }
}

/// Parse a duration in seconds: `10`, `1.5s`, `500ms`, `2 m`.
pub fn parse_duration(value: &str) -> Result<f64, String> {
    let value = value.trim();
    let split = value
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number: f64 = number
        .parse()
        .map_err(|_| format!("Invalid duration: {}", value))?;
    let factor = match unit.trim() {
        "" | "s" => 1.0,
        "ms" => 0.001,
        "m" => 60.0,
        other => return Err(format!("Invalid duration unit '{}' in: {}", other, value)),
    };
    Ok(number * factor)
}

/// One grpcurl call: the target, the options and the metadata (`-H` headers)
#[derive(Debug, Clone)]
pub struct GrpcCall {
    pub target: GrpcTarget,
    pub opts: GrpcOptions,
    pub headers: Vec<(String, String)>,
}

impl GrpcCall {
    /// Arguments to invoke the method of the target.
    /// `data` is the value of `-d`: `"@"` reads the messages from stdin. `None` sends no `-d`.
    /// `verbose` adds `-v` and `-format-error`, for `VerboseParser` and `parse_error_status`.
    pub fn invoke_args(&self, verbose: bool, data: Option<&str>) -> Result<Vec<String>, String> {
        let symbol = self
            .target
            .method_symbol()
            .ok_or_else(|| "The gRPC target has no /package.Service/Method path".to_string())?;

        let mut args = self.common_args();
        if self.opts.emit_defaults {
            args.push("-emit-defaults".to_string());
        }
        if verbose {
            args.push("-v".to_string());
            args.push("-format-error".to_string());
        }
        if let Some(data) = data {
            args.push("-d".to_string());
            args.push(data.to_string());
        }
        args.push(self.target.authority.clone());
        args.push(symbol);
        Ok(args)
    }

    /// Arguments to list the services
    pub fn list_args(&self) -> Vec<String> {
        let mut args = self.common_args();
        args.push(self.target.authority.clone());
        args.push("list".to_string());
        args
    }

    /// Arguments to describe a symbol. `msg_template` adds a JSON template for messages.
    pub fn describe_args(&self, symbol: &str, msg_template: bool) -> Vec<String> {
        let mut args = self.common_args();
        if msg_template {
            args.push("-msg-template".to_string());
        }
        args.push(self.target.authority.clone());
        args.push("describe".to_string());
        args.push(symbol.to_string());
        args
    }

    /// Flags common to invoke, list and describe. All flags must come before the address.
    fn common_args(&self) -> Vec<String> {
        let (target, opts) = (&self.target, &self.opts);
        let mut args = Vec::new();
        let path_arg = |args: &mut Vec<String>, flag: &str, path: &Path| {
            args.push(flag.to_string());
            args.push(path.to_string_lossy().to_string());
        };

        if target.tls {
            if opts.insecure {
                args.push("-insecure".to_string());
            }
            if let Some(cacert) = &opts.cacert {
                path_arg(&mut args, "-cacert", cacert);
            }
            if let Some(cert) = &opts.cert {
                path_arg(&mut args, "-cert", cert);
            }
            if let Some(key) = &opts.key {
                path_arg(&mut args, "-key", key);
            }
        } else {
            args.push("-plaintext".to_string());
        }

        if let Some(authority) = &opts.authority {
            args.push("-authority".to_string());
            args.push(authority.clone());
        }
        if let Some(secs) = opts.connect_timeout {
            args.push("-connect-timeout".to_string());
            args.push(secs.to_string());
        }
        if let Some(secs) = opts.max_time {
            args.push("-max-time".to_string());
            args.push(secs.to_string());
        }
        for dir in &opts.import_paths {
            path_arg(&mut args, "-import-path", dir);
        }
        for proto in &opts.protos {
            path_arg(&mut args, "-proto", proto);
        }
        for protoset in &opts.protosets {
            path_arg(&mut args, "-protoset", protoset);
        }
        // -H headers also go to reflection requests, so auth works for list/describe too
        for (name, value) in &self.headers {
            args.push("-H".to_string());
            args.push(format!("{}: {}", name, value));
        }

        args
    }
}

/// Return the request messages for `-d`. Return `None` when the request has no body.
/// A file reference (`< ./file.json`) resolves against `base_dir`.
pub fn request_data(request: &HttpRequest, base_dir: &Path) -> Result<Option<String>, String> {
    match &request.body {
        None => Ok(None),
        Some(RequestBody::Inline(body)) => Ok(Some(body.clone())),
        Some(RequestBody::FileReference(path)) => {
            let path = base_dir.join(path);
            std::fs::read_to_string(&path)
                .map(Some)
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))
        }
        Some(RequestBody::Multipart(_)) => {
            Err("Multipart bodies are not supported for gRPC requests".to_string())
        }
    }
}

/// Find the grpcurl binary: GRPCURL_PATH, then PATH, then usual install directories.
/// Zed can start the LSP with a short PATH, so the fallback directories are necessary.
pub fn find_grpcurl() -> Option<PathBuf> {
    if let Some(path) = env::var_os(GRPCURL_PATH_ENV) {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }

    let mut dirs: Vec<PathBuf> = env::var_os("PATH")
        .map(|p| env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = env::var_os("HOME") {
        dirs.push(Path::new(&home).join("go").join("bin"));
    }
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/local/bin"));

    dirs.into_iter()
        .map(|dir| dir.join("grpcurl"))
        .find(|candidate| candidate.is_file())
}

/// Remove the Mach exception ports that this process got from its parent (macOS only).
/// Call it at startup, before grpcurl starts. Child processes get the new ports.
///
/// Zed sets these ports for its crash handler, and child processes inherit them.
/// grpcurl (Go) recovers from memory faults: the kernel changes each fault into `SIGSEGV`.
/// With the inherited ports, the kernel sends the fault to Zed and not to grpcurl.
/// Then grpcurl stops at the fault and the request never completes.
#[cfg(target_os = "macos")]
pub fn reset_exception_ports() {
    // <mach/exception_types.h>
    const EXC_MASK_BAD_ACCESS: u32 = 1 << 1;
    const EXC_MASK_BAD_INSTRUCTION: u32 = 1 << 2;
    const EXC_MASK_ARITHMETIC: u32 = 1 << 3;
    const EXC_MASK_SOFTWARE: u32 = 1 << 5;
    const EXC_MASK_BREAKPOINT: u32 = 1 << 6;
    const EXC_MASK_CRASH: u32 = 1 << 10;
    const EXCEPTION_DEFAULT: i32 = 1;
    const MACH_PORT_NULL: u32 = 0;
    // <mach/arm/thread_status.h> and <mach/i386/thread_status.h>
    #[cfg(target_arch = "aarch64")]
    const THREAD_STATE_NONE: i32 = 5;
    #[cfg(target_arch = "x86_64")]
    const THREAD_STATE_NONE: i32 = 13;

    extern "C" {
        static mach_task_self_: u32;
        fn task_set_exception_ports(
            task: u32,
            exception_mask: u32,
            new_port: u32,
            behavior: i32,
            new_flavor: i32,
        ) -> i32;
    }

    let mask = EXC_MASK_BAD_ACCESS
        | EXC_MASK_BAD_INSTRUCTION
        | EXC_MASK_ARITHMETIC
        | EXC_MASK_SOFTWARE
        | EXC_MASK_BREAKPOINT
        | EXC_MASK_CRASH;
    // SAFETY: libSystem sets `mach_task_self_` before `main`. The call changes only this task.
    unsafe {
        task_set_exception_ports(
            mach_task_self_,
            mask,
            MACH_PORT_NULL,
            EXCEPTION_DEFAULT,
            THREAD_STATE_NONE,
        );
    }
}

#[cfg(not(target_os = "macos"))]
pub fn reset_exception_ports() {}

/// Name of a gRPC status code: `5` → `NOT_FOUND`
pub fn status_name(code: u32) -> &'static str {
    STATUS_NAMES
        .get(code as usize)
        .copied()
        .unwrap_or("UNKNOWN")
}

/// gRPC status code from the grpcurl exit code. `None` if the exit code is not a status.
pub fn status_from_exit_code(exit_code: i32) -> Option<u32> {
    let code = exit_code - GRPCURL_STATUS_EXIT_BASE;
    (1..STATUS_NAMES.len() as i32)
        .contains(&code)
        .then_some(code as u32)
}

/// A gRPC status with its message
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct GrpcStatus {
    pub code: u32,
    #[serde(default)]
    pub message: String,
}

/// Parse the status that `grpcurl -format-error` writes to stderr for a non-OK status:
/// `{ "code": 5, "message": "no such thing" }`
pub fn parse_error_status(stderr: &str) -> Option<GrpcStatus> {
    serde_json::from_str(stderr.trim()).ok()
}

/// A hint for a known grpcurl error, or `None`.
pub fn error_hint(stderr: &str, target: &GrpcTarget) -> Option<String> {
    let err = stderr.to_ascii_lowercase();
    if err.contains("first record does not look like a tls handshake") {
        return Some("The server uses plaintext. Remove grpcs:// from the target.".to_string());
    }
    if !target.tls
        && err.contains("failed to dial")
        && (err.trim_end().ends_with(": eof") || err.contains("connection closed"))
    {
        return Some(format!(
            "The server can require TLS. Use GRPC grpcs://{}/...",
            target.authority
        ));
    }
    if err.contains("does not support the reflection api") {
        return Some(
            "The server has no reflection. Add `# @proto <file.proto>` or `# @protoset <file>`."
                .to_string(),
        );
    }
    if err.contains("certificate signed by unknown authority") || err.contains("x509:") {
        return Some(
            "The server certificate is not trusted. Add `# @cacert <file>` or `# @insecure`."
                .to_string(),
        );
    }
    if err.contains("connection refused")
        || (err.contains("failed to dial") && err.contains("context deadline exceeded"))
    {
        return Some(format!(
            "No server answers at {}. Make sure the server runs and the address is correct.",
            target.authority
        ));
    }
    None
}

/// A method signature from `describe` output:
/// `rpc SayHello ( .hello.HelloRequest ) returns ( stream .hello.HelloResponse );`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodDescriptor {
    pub name: String,
    /// Fully-qualified type with a leading dot: `.hello.HelloRequest`
    pub request_type: String,
    pub response_type: String,
    pub client_streaming: bool,
    pub server_streaming: bool,
}

impl MethodDescriptor {
    /// The RPC type: unary, server streaming, client streaming or bidirectional streaming
    pub fn kind(&self) -> &'static str {
        match (self.client_streaming, self.server_streaming) {
            (false, false) => "unary",
            (false, true) => "server streaming",
            (true, false) => "client streaming",
            (true, true) => "bidirectional streaming",
        }
    }

    /// Request type without the leading dot: `hello.HelloRequest`
    pub fn request_name(&self) -> &str {
        self.request_type.trim_start_matches('.')
    }

    /// `(hello.HelloRequest) returns (stream hello.HelloResponse)`
    pub fn signature(&self) -> String {
        let part = |streaming: bool, ty: &str| {
            let ty = ty.trim_start_matches('.');
            if streaming {
                format!("stream {}", ty)
            } else {
                ty.to_string()
            }
        };
        format!(
            "({}) returns ({})",
            part(self.client_streaming, &self.request_type),
            part(self.server_streaming, &self.response_type)
        )
    }
}

/// Find and parse the first `rpc ...;` line of `text`.
pub fn parse_method_descriptor(text: &str) -> Option<MethodDescriptor> {
    text.lines().find_map(parse_rpc_line)
}

/// Parse all `rpc ...;` lines of a `describe <service>` output.
pub fn parse_service_methods(text: &str) -> Vec<MethodDescriptor> {
    text.lines().filter_map(parse_rpc_line).collect()
}

fn parse_rpc_line(line: &str) -> Option<MethodDescriptor> {
    let rest = line.trim().strip_prefix("rpc ")?;
    let (name, rest) = rest.split_once('(')?;
    let (request, rest) = rest.split_once(')')?;
    let (_, rest) = rest.split_once("returns")?;
    let (_, rest) = rest.split_once('(')?;
    let (response, _) = rest.split_once(')')?;

    let parse_type = |part: &str| {
        let part = part.trim();
        match part.strip_prefix("stream ") {
            Some(t) => (t.trim().to_string(), true),
            None => (part.to_string(), false),
        }
    };
    let (request_type, client_streaming) = parse_type(request);
    let (response_type, server_streaming) = parse_type(response);

    Some(MethodDescriptor {
        name: name.trim().to_string(),
        request_type,
        response_type,
        client_streaming,
        server_streaming,
    })
}

/// Return the JSON after `Message template:` in `describe -msg-template` output.
pub fn parse_message_template(text: &str) -> Option<String> {
    let (_, template) = text.split_once("Message template:")?;
    let template = template.trim();
    (!template.is_empty()).then(|| template.to_string())
}

/// Nesting depth of `{` and `[` in JSON text that arrives in parts. Braces in strings do not count.
#[derive(Debug, Default)]
struct JsonDepth {
    depth: i32,
    in_string: bool,
    escaped: bool,
}

impl JsonDepth {
    fn feed(&mut self, text: &str) {
        for ch in text.chars() {
            if self.in_string {
                if self.escaped {
                    self.escaped = false;
                } else if ch == '\\' {
                    self.escaped = true;
                } else if ch == '"' {
                    self.in_string = false;
                }
                continue;
            }
            match ch {
                '"' => self.in_string = true,
                '{' | '[' => self.depth += 1,
                '}' | ']' => self.depth -= 1,
                _ => {}
            }
        }
    }
}

/// Nesting depth of `{` and `[` at the end of `text`. Braces in strings do not count.
pub fn json_nesting_depth(text: &str) -> i32 {
    let mut json = JsonDepth::default();
    json.feed(text);
    json.depth
}

/// Parse `list` output: one symbol for each line.
pub fn parse_list_output(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect()
}

/// One item of `grpcurl -v` output
#[derive(Debug, Clone, PartialEq)]
pub enum GrpcEvent {
    MethodDescriptor(MethodDescriptor),
    ResponseHeaders(Vec<(String, String)>),
    Message(Value),
    Trailers(Vec<(String, String)>),
    /// `Sent 1 request and received 1 response`
    Summary(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Section {
    /// Lines to skip, for example the request metadata (a copy of the request headers)
    #[default]
    None,
    Descriptor,
    ResponseHeaders,
    Contents,
    Trailers,
}

/// Incremental parser for `grpcurl -v` stdout. Feed it one line at a time.
/// It emits each message when the message is complete, so streams show live.
#[derive(Debug, Default)]
pub struct VerboseParser {
    section: Section,
    pairs: Vec<(String, String)>,
    message: String,
    json: JsonDepth,
}

impl VerboseParser {
    pub fn feed_line(&mut self, line: &str) -> Vec<GrpcEvent> {
        let mut events = Vec::new();

        // Section titles only count between messages
        if self.message.is_empty() {
            let next = match line.trim_end() {
                "Resolved method descriptor:" => Some(Section::Descriptor),
                "Request metadata to send:" => Some(Section::None),
                "Response headers received:" => Some(Section::ResponseHeaders),
                "Response contents:" => Some(Section::Contents),
                "Response trailers received:" => Some(Section::Trailers),
                _ => None,
            };
            if let Some(next) = next {
                self.flush_pairs(&mut events);
                self.section = next;
                return events;
            }
            if line.starts_with("Sent ") && line.contains(" and received ") {
                self.flush_pairs(&mut events);
                self.section = Section::None;
                events.push(GrpcEvent::Summary(line.trim().to_string()));
                return events;
            }
        }

        match self.section {
            Section::None => {}
            Section::Descriptor => {
                if let Some(descriptor) = parse_method_descriptor(line) {
                    events.push(GrpcEvent::MethodDescriptor(descriptor));
                }
            }
            Section::ResponseHeaders | Section::Trailers => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    self.flush_pairs(&mut events);
                    self.section = Section::None;
                } else if let Some((name, value)) = trimmed.split_once(':') {
                    self.pairs
                        .push((name.trim().to_string(), value.trim().to_string()));
                }
            }
            Section::Contents => self.feed_message_line(line, &mut events),
        }

        events
    }

    /// Emit what is still pending at the end of the output.
    pub fn finish(&mut self) -> Vec<GrpcEvent> {
        let mut events = Vec::new();
        self.flush_pairs(&mut events);
        let raw = std::mem::take(&mut self.message);
        if !raw.trim().is_empty() {
            events.push(GrpcEvent::Message(parse_message(&raw)));
        }
        self.json = JsonDepth::default();
        events
    }

    fn feed_message_line(&mut self, line: &str, events: &mut Vec<GrpcEvent>) {
        if self.message.is_empty() {
            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                return;
            }
            // A scalar message (for example a wrapper type) fits on one line
            if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
                events.push(GrpcEvent::Message(parse_message(line)));
                return;
            }
        }

        if !self.message.is_empty() {
            self.message.push('\n');
        }
        self.message.push_str(line);
        self.json.feed(line);

        if self.json.depth <= 0 {
            let raw = std::mem::take(&mut self.message);
            self.json = JsonDepth::default();
            events.push(GrpcEvent::Message(parse_message(&raw)));
        }
    }

    fn flush_pairs(&mut self, events: &mut Vec<GrpcEvent>) {
        let pairs = std::mem::take(&mut self.pairs);
        match self.section {
            Section::ResponseHeaders => events.push(GrpcEvent::ResponseHeaders(pairs)),
            Section::Trailers => events.push(GrpcEvent::Trailers(pairs)),
            _ => return,
        }
        self.section = Section::None;
    }
}

/// Parse a message as JSON. Text that is not JSON becomes a JSON string.
pub fn parse_message(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.trim().to_string()))
}

/// Completion context on a `GRPC` request line:
/// `GRPC host:port/pkg.Ser|` completes a service, `GRPC host:port/pkg.Service/Me|` completes a method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcLineContext {
    /// The target without the path: `grpc://host:port`
    pub target: String,
    /// The service of the method to complete. `None` completes a service.
    pub service: Option<String>,
    /// Byte offset where the service or the method starts
    pub replace_start: usize,
}

/// Detect the completion context at byte offset `col` of a `GRPC` request line.
pub fn grpc_line_context(line: &str, col: usize) -> Option<GrpcLineContext> {
    let before = line.get(..col)?;
    let indent = before.len() - before.trim_start().len();
    let after_keyword = before[indent..].strip_prefix("GRPC")?;
    let url = after_keyword.trim_start();
    if url.len() == after_keyword.len() {
        return None;
    }
    let url_start = col - url.len();

    let scheme_len = url.find("://").map(|i| i + 3).unwrap_or(0);
    let path_start = url[scheme_len..].find('/')? + scheme_len;
    let target = url[..path_start].to_string();
    let path = &url[path_start + 1..];

    let service_start = url_start + path_start + 1;
    match path.split_once('/') {
        None => Some(GrpcLineContext {
            target,
            service: None,
            replace_start: service_start,
        }),
        Some((service, method)) if !method.contains('/') => Some(GrpcLineContext {
            target,
            service: Some(service.to_string()),
            replace_start: service_start + service.len() + 1,
        }),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_http_file;

    // Real `grpcurl -v` output (grpcurl 1.9.4, grpcb.in)
    const UNARY_OUTPUT: &str = "
Resolved method descriptor:
rpc SayHello ( .hello.HelloRequest ) returns ( .hello.HelloResponse );

Request metadata to send:
x-test: abc

Response headers received:
content-type: application/grpc

Response contents:
{
  \"reply\": \"hello zed\"
}

Response trailers received:
(empty)
Sent 1 request and received 1 response
";

    const STREAM_OUTPUT: &str = "
Resolved method descriptor:
rpc LotsOfReplies ( .hello.HelloRequest ) returns ( stream .hello.HelloResponse );

Request metadata to send:
(empty)

Response headers received:
content-type: application/grpc

Response contents:
{
  \"reply\": \"hello 1\"
}

Response contents:
{}

Response contents:
{
  \"reply\": \"brace } in \\\"string\\\" {\"
}

Response trailers received:
(empty)
Sent 1 request and received 3 responses
";

    const ERROR_STDOUT: &str = "
Resolved method descriptor:
rpc SpecificError ( .grpcbin.SpecificErrorRequest ) returns ( .grpcbin.EmptyMessage );

Request metadata to send:
(empty)

Response headers received:
(empty)

Response trailers received:
content-type: application/grpc
Sent 1 request and received 0 responses
";

    // `grpcurl -format-error` stderr for a non-OK status
    const ERROR_STDERR: &str = "{
  \"code\": 5,
  \"message\": \"no such thing\"
}";

    fn parse_all(output: &str) -> Vec<GrpcEvent> {
        let mut parser = VerboseParser::default();
        let mut events = Vec::new();
        for line in output.lines() {
            events.extend(parser.feed_line(line));
        }
        events.extend(parser.finish());
        events
    }

    fn pair(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    #[test]
    fn test_parse_target() {
        let t = parse_target("localhost:50051/hello.HelloService/SayHello").unwrap();
        assert_eq!(
            t,
            GrpcTarget {
                tls: false,
                authority: "localhost:50051".to_string(),
                service: Some("hello.HelloService".to_string()),
                method: Some("SayHello".to_string()),
            }
        );
        assert_eq!(
            t.method_symbol().as_deref(),
            Some("hello.HelloService/SayHello")
        );
        assert_eq!(
            t.describe_symbol().as_deref(),
            Some("hello.HelloService.SayHello")
        );

        let t = parse_target("grpcs://grpcb.in/hello.HelloService").unwrap();
        assert!(t.tls);
        assert_eq!(t.authority, "grpcb.in:443");
        assert_eq!(t.service.as_deref(), Some("hello.HelloService"));
        assert_eq!(t.method, None);

        let t = parse_target("GRPC://example.com").unwrap();
        assert!(!t.tls);
        assert_eq!(t.authority, "example.com:80");
        assert_eq!(t.service, None);

        let t = parse_target("https://[::1]:8443/a.B/C").unwrap();
        assert!(t.tls);
        assert_eq!(t.authority, "[::1]:8443");

        assert_eq!(parse_target("[::1]").unwrap().authority, "[::1]:80");
        assert!(parse_target("grpc:///a.B/C").is_err());
        assert!(parse_target("host:1/a/b/c").is_err());
    }

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("10"), Ok(10.0));
        assert_eq!(parse_duration("1.5s"), Ok(1.5));
        assert_eq!(parse_duration("500 ms"), Ok(0.5));
        assert_eq!(parse_duration("2 m"), Ok(120.0));
        assert!(parse_duration("2 h").is_err());
        assert!(parse_duration("soon").is_err());
    }

    #[test]
    fn test_options_from_request() {
        let content = r#"
# @proto ./proto/hello.proto
# @import-path ./third_party
# @timeout 10
# @connection-timeout 500ms
# @insecure
# @emit-defaults
# @authority api.internal
GRPC grpcs://localhost:8443/hello.HelloService/SayHello
"#;
        let request = &parse_http_file(content).unwrap()[0];
        let base = Path::new("/project/api");
        let ssl = SslConfiguration {
            client_certificate: Some(PathBuf::from("/certs/client.pem")),
            client_certificate_key: Some(PathBuf::from("/certs/client.key")),
            verify_host_certificate: None,
        };
        let opts = GrpcOptions::from_request(request, base, Some(&ssl)).unwrap();

        assert_eq!(
            opts.import_paths,
            vec![
                PathBuf::from("/project/api/third_party"),
                PathBuf::from("/project/api/proto"),
            ]
        );
        assert_eq!(
            opts.protos,
            vec![PathBuf::from("/project/api/proto/hello.proto")]
        );
        assert_eq!(opts.max_time, Some(10.0));
        assert_eq!(opts.connect_timeout, Some(0.5));
        assert!(opts.insecure);
        assert!(opts.emit_defaults);
        assert_eq!(opts.authority.as_deref(), Some("api.internal"));
        assert_eq!(opts.cert, Some(PathBuf::from("/certs/client.pem")));
        assert_eq!(opts.key, Some(PathBuf::from("/certs/client.key")));
    }

    #[test]
    fn test_ssl_configuration_disables_verification() {
        let request = &parse_http_file("GRPC grpcs://h:1/a.B/C\n").unwrap()[0];
        let ssl = SslConfiguration {
            verify_host_certificate: Some(false),
            ..Default::default()
        };
        let opts = GrpcOptions::from_request(request, Path::new("/"), Some(&ssl)).unwrap();
        assert!(opts.insecure);
    }

    fn call(url: &str, opts: GrpcOptions, headers: Vec<(String, String)>) -> GrpcCall {
        GrpcCall {
            target: parse_target(url).unwrap(),
            opts,
            headers,
        }
    }

    #[test]
    fn test_invoke_args_plaintext() {
        let opts = GrpcOptions {
            insecure: true,
            max_time: Some(5.0),
            ..Default::default()
        };
        let headers = vec![pair("authorization", "Bearer abc")];
        let call = call("localhost:9000/hello.HelloService/SayHello", opts, headers);
        assert_eq!(
            call.invoke_args(true, Some("@")).unwrap(),
            vec![
                "-plaintext",
                "-max-time",
                "5",
                "-H",
                "authorization: Bearer abc",
                "-v",
                "-format-error",
                "-d",
                "@",
                "localhost:9000",
                "hello.HelloService/SayHello",
            ]
        );
    }

    #[test]
    fn test_invoke_args_tls() {
        let opts = GrpcOptions {
            insecure: true,
            cacert: Some(PathBuf::from("/ca.pem")),
            emit_defaults: true,
            ..Default::default()
        };
        let mut call = call("grpcs://api.example.com/a.B/C", opts, vec![]);
        assert_eq!(
            call.invoke_args(false, None).unwrap(),
            vec![
                "-insecure",
                "-cacert",
                "/ca.pem",
                "-emit-defaults",
                "api.example.com:443",
                "a.B/C",
            ]
        );

        call.target = parse_target("grpcs://api.example.com").unwrap();
        assert!(call.invoke_args(false, None).is_err());
    }

    #[test]
    fn test_list_and_describe_args() {
        let call = call("localhost:9000", GrpcOptions::default(), vec![]);
        assert_eq!(
            call.list_args(),
            vec!["-plaintext", "localhost:9000", "list"]
        );
        assert_eq!(
            call.describe_args(".hello.HelloRequest", true),
            vec![
                "-plaintext",
                "-msg-template",
                "localhost:9000",
                "describe",
                ".hello.HelloRequest",
            ]
        );
    }

    #[test]
    fn test_request_data() {
        let requests = parse_http_file(
            "GRPC h:1/a.B/C\n\n{ \"a\": 1 }\n\n###\nGRPC h:1/a.B/C\n\n###\nGRPC h:1/a.B/C\n\n< ./missing.json\n",
        )
        .unwrap();
        let base = Path::new("/nonexistent-dir");
        assert_eq!(
            request_data(&requests[0], base).unwrap().as_deref(),
            Some("{ \"a\": 1 }")
        );
        assert_eq!(request_data(&requests[1], base).unwrap(), None);
        assert!(request_data(&requests[2], base).is_err());
    }

    #[test]
    fn test_status_helpers() {
        assert_eq!(status_name(0), "OK");
        assert_eq!(status_name(5), "NOT_FOUND");
        assert_eq!(status_name(16), "UNAUTHENTICATED");
        assert_eq!(status_name(99), "UNKNOWN");
        assert_eq!(status_from_exit_code(69), Some(5));
        assert_eq!(status_from_exit_code(80), Some(16));
        assert_eq!(status_from_exit_code(0), None);
        assert_eq!(status_from_exit_code(1), None);
        assert_eq!(status_from_exit_code(64), None);
    }

    #[test]
    fn test_parse_error_status() {
        assert_eq!(
            parse_error_status(ERROR_STDERR),
            Some(GrpcStatus {
                code: 5,
                message: "no such thing".to_string(),
            })
        );
        assert_eq!(
            parse_error_status("{\"code\": 14}"),
            Some(GrpcStatus {
                code: 14,
                message: String::new(),
            })
        );
        assert_eq!(parse_error_status("Failed to dial target host"), None);
    }

    #[test]
    fn test_error_hint() {
        let plaintext = parse_target("grpcb.in:9001").unwrap();
        let tls = parse_target("grpcs://grpcb.in:9000").unwrap();
        assert!(error_hint(
            "Failed to dial target host \"grpcb.in:9000\": tls: first record does not look like a TLS handshake",
            &tls
        )
        .unwrap()
        .contains("plaintext"));
        assert!(error_hint(
            "Failed to dial target host \"grpcb.in:9001\": EOF",
            &plaintext
        )
        .unwrap()
        .contains("grpcs://grpcb.in:9001"));
        assert!(error_hint(
            "Failed to list services: server does not support the reflection API",
            &plaintext
        )
        .unwrap()
        .contains("@proto"));
        assert!(error_hint(
            "Failed to dial target host \"localhost:1\": context deadline exceeded",
            &plaintext
        )
        .unwrap()
        .contains("grpcb.in:9001"));
        assert_eq!(error_hint("something else", &plaintext), None);
    }

    #[test]
    fn test_parse_method_descriptor() {
        let d = parse_method_descriptor(
            "hello.HelloService.BidiHello is a method:\nrpc BidiHello ( stream .hello.HelloRequest ) returns ( stream .hello.HelloResponse );\n",
        )
        .unwrap();
        assert_eq!(
            d,
            MethodDescriptor {
                name: "BidiHello".to_string(),
                request_type: ".hello.HelloRequest".to_string(),
                response_type: ".hello.HelloResponse".to_string(),
                client_streaming: true,
                server_streaming: true,
            }
        );
        assert_eq!(d.kind(), "bidirectional streaming");
        assert_eq!(d.request_name(), "hello.HelloRequest");
        assert_eq!(
            d.signature(),
            "(stream hello.HelloRequest) returns (stream hello.HelloResponse)"
        );

        let d = parse_method_descriptor(
            "rpc SayHello ( .hello.HelloRequest ) returns ( .hello.HelloResponse );",
        )
        .unwrap();
        assert!(!d.client_streaming);
        assert!(!d.server_streaming);
        assert_eq!(d.kind(), "unary");
    }

    #[test]
    fn test_parse_service_methods() {
        let describe = "hello.HelloService is a service:
service HelloService {
  rpc LotsOfReplies ( .hello.HelloRequest ) returns ( stream .hello.HelloResponse );
  rpc SayHello ( .hello.HelloRequest ) returns ( .hello.HelloResponse );
}
";
        let methods = parse_service_methods(describe);
        assert_eq!(methods.len(), 2);
        assert_eq!(methods[0].name, "LotsOfReplies");
        assert_eq!(methods[0].kind(), "server streaming");
        assert_eq!(methods[1].name, "SayHello");
    }

    #[test]
    fn test_json_nesting_depth() {
        assert_eq!(json_nesting_depth(""), 0);
        assert_eq!(json_nesting_depth("{\n  \"a\": "), 1);
        assert_eq!(json_nesting_depth("{ \"a\": { \"b\": ["), 3);
        assert_eq!(json_nesting_depth("{ \"a\": \"} {\" }"), 0);
        assert_eq!(json_nesting_depth("{ \"a\": 1 }\n{ "), 1);
    }

    #[test]
    fn test_parse_message_template_and_list() {
        let describe = "hello.HelloRequest is a message:\nmessage HelloRequest {\n  optional string greeting = 1;\n}\n\nMessage template:\n{\n  \"greeting\": \"\"\n}\n";
        assert_eq!(
            parse_message_template(describe).as_deref(),
            Some("{\n  \"greeting\": \"\"\n}")
        );
        assert_eq!(parse_message_template("no template"), None);
        assert_eq!(
            parse_list_output("addsvc.Add\n\nhello.HelloService\n"),
            vec!["addsvc.Add", "hello.HelloService"]
        );
    }

    #[test]
    fn test_verbose_parser_unary() {
        let events = parse_all(UNARY_OUTPUT);
        assert_eq!(
            events,
            vec![
                GrpcEvent::MethodDescriptor(MethodDescriptor {
                    name: "SayHello".to_string(),
                    request_type: ".hello.HelloRequest".to_string(),
                    response_type: ".hello.HelloResponse".to_string(),
                    client_streaming: false,
                    server_streaming: false,
                }),
                GrpcEvent::ResponseHeaders(vec![pair("content-type", "application/grpc")]),
                GrpcEvent::Message(serde_json::json!({"reply": "hello zed"})),
                GrpcEvent::Trailers(vec![]),
                GrpcEvent::Summary("Sent 1 request and received 1 response".to_string()),
            ]
        );
    }

    #[test]
    fn test_verbose_parser_stream() {
        let messages: Vec<Value> = parse_all(STREAM_OUTPUT)
            .into_iter()
            .filter_map(|e| match e {
                GrpcEvent::Message(m) => Some(m),
                _ => None,
            })
            .collect();
        assert_eq!(
            messages,
            vec![
                serde_json::json!({"reply": "hello 1"}),
                serde_json::json!({}),
                serde_json::json!({"reply": "brace } in \"string\" {"}),
            ]
        );
    }

    #[test]
    fn test_verbose_parser_emits_message_before_next_section() {
        let mut parser = VerboseParser::default();
        for line in ["Response contents:", "{", "  \"a\": 1"] {
            assert!(parser.feed_line(line).is_empty());
        }
        assert_eq!(
            parser.feed_line("}"),
            vec![GrpcEvent::Message(serde_json::json!({"a": 1}))]
        );
    }

    #[test]
    fn test_verbose_parser_error_output() {
        let events = parse_all(ERROR_STDOUT);
        assert!(events.contains(&GrpcEvent::ResponseHeaders(vec![])));
        assert!(events.contains(&GrpcEvent::Trailers(vec![pair(
            "content-type",
            "application/grpc"
        )])));
        assert!(!events.iter().any(|e| matches!(e, GrpcEvent::Message(_))));
    }

    #[test]
    fn test_grpc_line_context() {
        let line = "GRPC localhost:9000/hello.Hel";
        assert_eq!(
            grpc_line_context(line, line.len()),
            Some(GrpcLineContext {
                target: "localhost:9000".to_string(),
                service: None,
                replace_start: 20,
            })
        );

        let line = "GRPC grpcs://h:1/hello.HelloService/Say";
        assert_eq!(
            grpc_line_context(line, line.len()),
            Some(GrpcLineContext {
                target: "grpcs://h:1".to_string(),
                service: Some("hello.HelloService".to_string()),
                replace_start: 36,
            })
        );

        assert_eq!(grpc_line_context("GRPC localhost:9000", 19), None);
        assert_eq!(grpc_line_context("GET http://x/y", 14), None);
        assert_eq!(grpc_line_context("GRPCX h/a", 9), None);
    }
}
