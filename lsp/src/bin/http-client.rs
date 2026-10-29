use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use http_lsp::grpc::{
    self, GrpcCall, GrpcEvent, GrpcOptions, GrpcStatus, GrpcTarget, VerboseParser,
};
use http_lsp::parser::{
    parse_http_file_with_resolver, request_at_line, HttpMethod, HttpRequest, RequestBody,
    ResponseHandler,
};
use http_lsp::response_handler::{execute_handler, load_globals, save_globals, ResponseContext};
use http_lsp::variables::{SslConfiguration, VariableResolver};

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const MAGENTA: &str = "\x1b[35m";

fn status_color(code: u16) -> &'static str {
    match code {
        200..=299 => GREEN,
        300..=399 => YELLOW,
        400..=499 => RED,
        500..=599 => RED,
        _ => RESET,
    }
}

fn main() {
    // Zed starts this task with its crash handler ports. grpcurl does not work with them.
    grpc::reset_exception_ports();

    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage: http-client <file.http> <line-number> [--curl | --describe]");
        process::exit(1);
    }

    let file_path = &args[1];
    let line_number: usize = args[2].parse().unwrap_or_else(|_| {
        eprintln!("Invalid line number: {}", args[2]);
        process::exit(1);
    });
    let curl_mode = args.iter().any(|a| a == "--curl");
    let describe_mode = args.iter().any(|a| a == "--describe");

    let content = fs::read_to_string(file_path).unwrap_or_else(|e| {
        eprintln!("Failed to read {}: {}", file_path, e);
        process::exit(1);
    });

    let file_parent = Path::new(file_path)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let mut globals = load_globals(file_parent);

    let mut resolver = VariableResolver::new();
    for (name, value) in &globals {
        resolver.set_global(name.clone(), value.clone());
    }

    resolver.load_environments_from_dir(file_parent);

    let requests = parse_http_file_with_resolver(&content, &mut resolver).unwrap_or_else(|e| {
        eprintln!("Failed to parse HTTP file: {}", e);
        process::exit(1);
    });

    let request = request_at_line(&requests, line_number).unwrap_or_else(|| {
        eprintln!("No HTTP request found at line {}", line_number);
        process::exit(1);
    });

    if request.method == HttpMethod::GRPC {
        let mode = if curl_mode {
            GrpcMode::Copy
        } else if describe_mode {
            GrpcMode::Describe
        } else {
            GrpcMode::Send
        };
        if mode != GrpcMode::Copy {
            print_request_header(request);
        }

        let base_dir = env::current_dir()
            .map(|cwd| cwd.join(file_parent))
            .unwrap_or_else(|_| file_parent.to_path_buf());
        let ssl = resolver.get_ssl_configuration();
        if let Err(e) = run_grpc(request, mode, &mut globals, &base_dir, ssl) {
            println!("{RED}{BOLD}✗ Request failed:{RESET} {RED}{e}{RESET}");
            process::exit(1);
        }
        return;
    }

    if describe_mode {
        eprintln!("--describe works only for GRPC requests");
        process::exit(1);
    }

    if curl_mode {
        generate_curl(request);
    } else {
        print_request_header(request);

        match execute_request(request, &mut globals, file_parent) {
            Ok(()) => {}
            Err(e) => {
                println!("{RED}{BOLD}✗ Request failed:{RESET} {RED}{e}{RESET}");
                process::exit(1);
            }
        }
    }
}

fn generate_curl(request: &HttpRequest) {
    let mut parts = vec![
        "curl".to_string(),
        "-X".to_string(),
        format!("{:?}", request.method),
        shell_quote(&request.url),
    ];

    for (name, value) in &request.headers {
        parts.push("-H".to_string());
        parts.push(shell_quote(&format!("{}: {}", name, value)));
    }

    if let Some(RequestBody::Inline(body)) = &request.body {
        parts.push("--data-raw".to_string());
        parts.push(shell_quote(body));
    }

    copy_command(&parts.join(" "));
}

fn print_request_header(request: &HttpRequest) {
    let method_str = format!("{:?}", request.method);

    println!("{BOLD}{CYAN}▶ {method_str} {}{RESET}", request.url);
    println!("{DIM}─────────────────────────────────────{RESET}");
    println!();
}

/// Copy a shell command to the clipboard and show it
fn copy_command(command: &str) {
    match copy_to_clipboard(command) {
        Ok(()) => {
            println!("{GREEN}{BOLD}✓ Copied to clipboard{RESET}");
            println!();
            println!("{DIM}{command}{RESET}");
        }
        Err(e) => {
            // Fallback: print so user can copy manually
            eprintln!("{RED}{BOLD}✗ Clipboard failed:{RESET} {RED}{e}{RESET}");
            println!();
            println!("{command}");
        }
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Quote a shell argument only when it has characters that the shell interprets
fn shell_arg(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@=,+".contains(c));
    if safe {
        s.to_string()
    } else {
        shell_quote(s)
    }
}

fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let (cmd, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("pbcopy", &[])
    } else {
        ("xclip", &["-selection", "clipboard"])
    };

    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{cmd}: {e}"))?;

    child
        .stdin
        .as_mut()
        .ok_or("Failed to open stdin")?
        .write_all(text.as_bytes())
        .map_err(|e| format!("Write failed: {e}"))?;

    let output = child.wait_with_output().map_err(|e| format!("{e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("{cmd} failed: {stderr}"))
    }
}

fn execute_request(
    request: &HttpRequest,
    globals: &mut HashMap<String, String>,
    http_file_dir: &Path,
) -> Result<(), String> {
    use reqwest::blocking::Client;

    let client = Client::new();

    let method = match request.method {
        HttpMethod::GET => reqwest::Method::GET,
        HttpMethod::POST => reqwest::Method::POST,
        HttpMethod::PUT => reqwest::Method::PUT,
        HttpMethod::DELETE => reqwest::Method::DELETE,
        HttpMethod::PATCH => reqwest::Method::PATCH,
        HttpMethod::HEAD => reqwest::Method::HEAD,
        HttpMethod::OPTIONS => reqwest::Method::OPTIONS,
        _ => return Err(format!("Unsupported method: {:?}", request.method)),
    };

    let mut req_builder = client.request(method, &request.url);

    for (name, value) in &request.headers {
        req_builder = req_builder.header(name, value);
    }

    if let Some(ref body) = request.body {
        match body {
            RequestBody::Inline(s) => {
                req_builder = req_builder.body(s.clone());
            }
            _ => return Err("File references not supported".to_string()),
        }
    }

    let start = Instant::now();
    let mut spinner = Spinner::start(start, "Waiting...");

    let result = req_builder.send();
    let elapsed = start.elapsed();
    spinner.stop();

    let response = result.map_err(|e| {
        // Walk the error chain to show the root cause (DNS failure, connection refused, etc.)
        let mut msg = e.to_string();
        let mut source = std::error::Error::source(&e);
        while let Some(cause) = source {
            msg = format!("{msg}\n  → {cause}");
            source = std::error::Error::source(cause);
        }
        msg
    })?;

    let status = response.status();
    let status_code = status.as_u16();
    let status_text = status.canonical_reason().unwrap_or("");
    let headers = response.headers().clone();
    let body = response
        .text()
        .map_err(|e| format!("Failed to read body: {}", e))?;

    let color = status_color(status_code);

    let elapsed_display = format_elapsed(elapsed);
    println!(
        "{BOLD}{color}HTTP {status_code} {status_text}{RESET}  {DIM}({elapsed_display}){RESET}"
    );
    println!();

    for (name, value) in headers.iter() {
        print_header(name.as_str(), value.to_str().unwrap_or("<binary>"));
    }

    println!();

    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if content_type.contains("json") {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body) {
            print_colored_json(&parsed, 0);
            println!();
        }
    } else {
        println!("{body}");
    }

    // Execute response handler after displaying the response
    run_response_handler(request, globals, http_file_dir, || {
        let mut headers_map = HashMap::new();
        for (name, value) in &headers {
            headers_map.insert(
                name.as_str().to_string(),
                value.to_str().unwrap_or("").to_string(),
            );
        }

        ResponseContext {
            status: status_code,
            headers: headers_map,
            body: serde_json::from_str::<serde_json::Value>(&body).ok(),
            body_raw: body.clone(),
        }
    })
}

/// Run the `> {% ... %}` handler of the request and save the captured globals.
/// `context` builds the response context only when the request has a handler.
fn run_response_handler(
    request: &HttpRequest,
    globals: &mut HashMap<String, String>,
    http_file_dir: &Path,
    context: impl FnOnce() -> ResponseContext,
) -> Result<(), String> {
    let Some(ResponseHandler::Inline(script)) = &request.response_handler else {
        return Ok(());
    };

    match execute_handler(script, &context()) {
        Ok(captured) => {
            if !captured.is_empty() {
                for (k, v) in &captured {
                    globals.insert(k.clone(), v.clone());
                }

                save_globals(http_file_dir, globals)?;

                let mut names: Vec<_> = captured.keys().cloned().collect();
                names.sort();
                println!();
                println!("{DIM}─────────────────────────────────────{RESET}");
                println!("{GREEN}{BOLD}✓ Saved: {}{RESET}", names.join(", "));
            }
        }
        Err(e) => {
            eprintln!("{YELLOW}Response handler skipped: {e}{RESET}");
        }
    }

    Ok(())
}

/// Format an elapsed time: `42ms` or `3.315s`
fn format_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs_f64();
    if secs >= 1.0 {
        format!("{secs:.3}s")
    } else {
        format!("{}ms", elapsed.as_millis())
    }
}

/// Color-cycling braille spinner with elapsed time. It runs on its own thread.
struct Spinner {
    /// `stop` drops the sender. This wakes the thread, and the thread ends.
    thread: Option<(mpsc::Sender<()>, std::thread::JoinHandle<()>)>,
}

impl Spinner {
    fn start(start: Instant, label: &'static str) -> Self {
        let (sender, receiver) = mpsc::channel::<()>();

        let handle = std::thread::spawn(move || {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let colors = [
                "\x1b[36m", // cyan
                "\x1b[96m", // bright cyan
                "\x1b[34m", // blue
                "\x1b[94m", // bright blue
                "\x1b[35m", // magenta
                "\x1b[95m", // bright magenta
                "\x1b[33m", // yellow
                "\x1b[93m", // bright yellow
                "\x1b[32m", // green
                "\x1b[92m", // bright green
            ];
            let mut i = 0;
            loop {
                let ms = start.elapsed().as_millis();
                let display = if ms >= 1000 {
                    format!("{:.1}s", ms as f64 / 1000.0)
                } else {
                    format!("{}ms", ms)
                };
                let color = colors[i % colors.len()];
                print!(
                    "\r{color}{} {label} {display}{RESET}    ",
                    frames[i % frames.len()],
                );
                let _ = std::io::stdout().flush();
                i += 1;
                let wait = receiver.recv_timeout(Duration::from_millis(80));
                if wait != Err(mpsc::RecvTimeoutError::Timeout) {
                    break;
                }
            }
            print!("\r\x1b[2K");
            let _ = std::io::stdout().flush();
        });

        Self {
            thread: Some((sender, handle)),
        }
    }

    /// Stop the spinner and clear its line. Calling it again does nothing.
    fn stop(&mut self) {
        if let Some((sender, handle)) = self.thread.take() {
            drop(sender);
            let _ = handle.join();
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop();
    }
}

fn print_colored_json(value: &serde_json::Value, indent: usize) {
    const BLUE: &str = "\x1b[34m";
    const STR_GREEN: &str = "\x1b[32m";
    const NUM_CYAN: &str = "\x1b[36m";
    const BOOL_YELLOW: &str = "\x1b[33m";
    const NULL_RED: &str = "\x1b[31m";
    const KEY_MAGENTA: &str = "\x1b[35m";
    const BRACE: &str = "\x1b[1m";

    let pad = "  ".repeat(indent);
    let inner_pad = "  ".repeat(indent + 1);

    match value {
        serde_json::Value::Null => print!("{NULL_RED}null{RESET}"),
        serde_json::Value::Bool(b) => print!("{BOOL_YELLOW}{b}{RESET}"),
        serde_json::Value::Number(n) => print!("{NUM_CYAN}{n}{RESET}"),
        serde_json::Value::String(s) => {
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
            print!("{STR_GREEN}\"{escaped}\"{RESET}");
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                print!("{BRACE}[]{RESET}");
                return;
            }
            println!("{BRACE}[{RESET}");
            for (i, item) in arr.iter().enumerate() {
                print!("{inner_pad}");
                print_colored_json(item, indent + 1);
                if i < arr.len() - 1 {
                    println!(",");
                } else {
                    println!();
                }
            }
            print!("{pad}{BRACE}]{RESET}");
        }
        serde_json::Value::Object(obj) => {
            if obj.is_empty() {
                print!("{BRACE}{{}}{RESET}");
                return;
            }
            println!("{BRACE}{{{RESET}");
            let len = obj.len();
            for (i, (key, val)) in obj.iter().enumerate() {
                print!("{inner_pad}{KEY_MAGENTA}\"{key}\"{RESET}{BLUE}:{RESET} ");
                print_colored_json(val, indent + 1);
                if i < len - 1 {
                    println!(",");
                } else {
                    println!();
                }
            }
            print!("{pad}{BRACE}}}{RESET}");
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrpcMode {
    /// Invoke the method, or list the services when the target has no method
    Send,
    /// Copy the grpcurl command to the clipboard
    Copy,
    /// Show the services, methods and message templates
    Describe,
}

fn run_grpc(
    request: &HttpRequest,
    mode: GrpcMode,
    globals: &mut HashMap<String, String>,
    base_dir: &Path,
    ssl: Option<&SslConfiguration>,
) -> Result<(), String> {
    let call = GrpcCall {
        target: grpc::parse_target(&request.url)?,
        opts: GrpcOptions::from_request(request, base_dir, ssl)?,
        headers: request.headers.clone(),
    };

    if mode == GrpcMode::Copy {
        return generate_grpcurl(request, &call, base_dir);
    }

    let grpcurl = grpc::find_grpcurl()
        .ok_or_else(|| format!("grpcurl not found.\n  → {}", grpc::GRPCURL_INSTALL_HINT))?;

    if mode == GrpcMode::Describe || call.target.method.is_none() {
        describe_grpc(&grpcurl, &call)
    } else {
        execute_grpc_request(&grpcurl, request, &call, globals, base_dir)
    }
}

fn grpc_status_color(code: u32) -> &'static str {
    match code {
        0 => GREEN,
        // CANCELLED, DEADLINE_EXCEEDED
        1 | 4 => YELLOW,
        _ => RED,
    }
}

/// Add the hint for a known grpcurl error to the error text
fn grpc_error(stderr: &str, target: &GrpcTarget) -> String {
    let mut msg = stderr.trim().to_string();
    if msg.is_empty() {
        msg = "grpcurl failed with no error output".to_string();
    }
    if let Some(hint) = grpc::error_hint(stderr, target) {
        msg = format!("{msg}\n  → {hint}");
    }
    msg
}

fn print_header(name: &str, value: &str) {
    println!("{MAGENTA}{name}{RESET}{DIM}:{RESET} {value}");
}

fn print_metadata(pairs: &[(String, String)]) {
    for (name, value) in pairs {
        print_header(name, value);
    }
}

fn print_grpc_message(message: &serde_json::Value) {
    match message {
        serde_json::Value::String(raw) => println!("{raw}"),
        _ => {
            print_colored_json(message, 0);
            println!();
        }
    }
}

/// The response of one gRPC call
#[derive(Default)]
struct GrpcResponse {
    server_streaming: bool,
    headers: Vec<(String, String)>,
    messages: Vec<serde_json::Value>,
    trailers: Vec<(String, String)>,
    summary: Option<String>,
}

impl GrpcResponse {
    /// Return true when a server stream already showed its headers and messages
    fn streamed(&self) -> bool {
        self.server_streaming && !self.messages.is_empty()
    }
}

fn execute_grpc_request(
    grpcurl: &Path,
    request: &HttpRequest,
    call: &GrpcCall,
    globals: &mut HashMap<String, String>,
    base_dir: &Path,
) -> Result<(), String> {
    let data = grpc::request_data(request, base_dir)?;
    let args = call.invoke_args(true, data.as_ref().map(|_| "@"))?;

    let mut child = Command::new(grpcurl)
        .args(&args)
        .stdin(if data.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start {}: {e}", grpcurl.display()))?;

    // Write and read on separate threads: a bidi stream can fill a pipe on either side
    let stdin_writer = match (child.stdin.take(), data) {
        (Some(mut stdin), Some(data)) => Some(std::thread::spawn(move || {
            let _ = stdin.write_all(data.as_bytes());
        })),
        _ => None,
    };
    let mut stderr = child.stderr.take().ok_or("Failed to open grpcurl stderr")?;
    let stderr_reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut stderr, &mut text);
        text
    });
    let stdout = child.stdout.take().ok_or("Failed to open grpcurl stdout")?;

    let start = Instant::now();
    let mut spinner = Spinner::start(start, "Waiting...");
    let mut parser = VerboseParser::default();
    let mut response = GrpcResponse::default();

    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| format!("Failed to read grpcurl output: {e}"))?;
        for event in parser.feed_line(&line) {
            handle_grpc_event(event, &mut response, &mut spinner, start);
        }
    }
    for event in parser.finish() {
        handle_grpc_event(event, &mut response, &mut spinner, start);
    }

    let exit = child
        .wait()
        .map_err(|e| format!("Failed to wait for grpcurl: {e}"))?;
    let elapsed = start.elapsed();
    spinner.stop();
    if let Some(writer) = stdin_writer {
        let _ = writer.join();
    }
    let stderr = stderr_reader.join().unwrap_or_default();

    let exit_code = exit.code().unwrap_or(-1);
    let status = if exit_code == 0 {
        GrpcStatus {
            code: 0,
            message: String::new(),
        }
    } else if let Some(code) = grpc::status_from_exit_code(exit_code) {
        grpc::parse_error_status(&stderr).unwrap_or(GrpcStatus {
            code,
            message: stderr.trim().to_string(),
        })
    } else {
        // grpcurl failed before the call: connection, reflection or bad input
        return Err(grpc_error(&stderr, &call.target));
    };

    print_grpc_result(&response, &status, elapsed);

    run_response_handler(request, globals, base_dir, || {
        let body = response.messages.last().cloned();
        let mut headers = HashMap::new();
        for (name, value) in response.headers.iter().chain(&response.trailers) {
            headers.insert(name.clone(), value.clone());
        }
        ResponseContext {
            status: status.code as u16,
            headers,
            body_raw: body.as_ref().map(|b| b.to_string()).unwrap_or_default(),
            body,
        }
    })
}

/// Show a server stream live: headers first, then each message when it arrives
fn handle_grpc_event(
    event: GrpcEvent,
    response: &mut GrpcResponse,
    spinner: &mut Spinner,
    start: Instant,
) {
    match event {
        GrpcEvent::MethodDescriptor(descriptor) => {
            response.server_streaming = descriptor.server_streaming;
        }
        GrpcEvent::ResponseHeaders(headers) => response.headers = headers,
        GrpcEvent::Message(message) => {
            if response.server_streaming {
                spinner.stop();
                if response.messages.is_empty() && !response.headers.is_empty() {
                    print_metadata(&response.headers);
                    println!();
                }
                let n = response.messages.len() + 1;
                let at = format_elapsed(start.elapsed());
                println!("{DIM}── message {n} (+{at}) ──{RESET}");
                print_grpc_message(&message);
                *spinner = Spinner::start(start, "Streaming...");
            }
            response.messages.push(message);
        }
        GrpcEvent::Trailers(trailers) => response.trailers = trailers,
        GrpcEvent::Summary(summary) => response.summary = Some(summary),
    }
}

/// Show the response like an HTTP response: status, headers, body.
/// A server stream already showed its headers and messages.
fn print_grpc_result(response: &GrpcResponse, status: &GrpcStatus, elapsed: Duration) {
    let color = grpc_status_color(status.code);
    let name = grpc::status_name(status.code);
    let elapsed_display = format_elapsed(elapsed);
    let streamed = response.streamed();

    if streamed {
        println!();
    }
    println!(
        "{BOLD}{color}gRPC {} {name}{RESET}  {DIM}({elapsed_display}){RESET}",
        status.code
    );
    if !status.message.is_empty() {
        println!("{color}{}{RESET}", status.message);
    }
    println!();

    if !streamed {
        if !response.headers.is_empty() {
            print_metadata(&response.headers);
            println!();
        }
        for message in &response.messages {
            print_grpc_message(message);
        }
    }

    if !response.trailers.is_empty() {
        if !response.messages.is_empty() {
            println!();
        }
        println!("{DIM}Trailers:{RESET}");
        print_metadata(&response.trailers);
    }

    if response.server_streaming {
        if let Some(summary) = &response.summary {
            println!();
            println!("{DIM}{summary}{RESET}");
        }
    }
}

/// Run grpcurl to completion and return stdout
fn run_grpcurl(grpcurl: &Path, args: &[String], target: &GrpcTarget) -> Result<String, String> {
    let start = Instant::now();
    let mut spinner = Spinner::start(start, "Waiting...");
    let output = Command::new(grpcurl)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Failed to start {}: {e}", grpcurl.display()));
    spinner.stop();
    let output = output?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(grpc_error(&String::from_utf8_lossy(&output.stderr), target))
    }
}

/// Show what the server offers:
/// - host only: the services;
/// - a service: its methods with their signatures;
/// - a method: its signature and a JSON template of its request message.
fn describe_grpc(grpcurl: &Path, call: &GrpcCall) -> Result<(), String> {
    let target = &call.target;
    let describe = |symbol: &str, msg_template: bool| {
        run_grpcurl(grpcurl, &call.describe_args(symbol, msg_template), target)
    };

    let Some(service) = &target.service else {
        let output = run_grpcurl(grpcurl, &call.list_args(), target)?;
        println!("{BOLD}Services at {}{RESET}", target.authority);
        println!();
        for service in grpc::parse_list_output(&output) {
            println!("  {CYAN}{service}{RESET}");
        }
        println!();
        println!("{DIM}Add /package.Service/Method to the request line to call a method.{RESET}");
        return Ok(());
    };

    let Some(symbol) = target.describe_symbol() else {
        println!("{}", describe(service, false)?.trim_end());
        return Ok(());
    };

    let output = describe(&symbol, false)?;
    println!("{}", output.trim_end());

    let Some(descriptor) = grpc::parse_method_descriptor(&output) else {
        return Ok(());
    };
    println!();
    println!("{DIM}Type: {}{RESET}", descriptor.kind());

    let output = describe(&descriptor.request_type, true)?;
    if let Some(template) = grpc::parse_message_template(&output) {
        println!();
        println!(
            "{BOLD}Request template{RESET} {DIM}({}){RESET}",
            descriptor.request_name()
        );
        print_grpc_message(&grpc::parse_message(&template));
    }

    Ok(())
}

/// Copy the request as a grpcurl command
fn generate_grpcurl(request: &HttpRequest, call: &GrpcCall, base_dir: &Path) -> Result<(), String> {
    let data = grpc::request_data(request, base_dir)?.map(|d| compact_json_messages(&d));
    let args = if call.target.method.is_some() {
        call.invoke_args(false, data.as_deref())?
    } else {
        call.list_args()
    };

    let grpcurl = grpc::find_grpcurl()
        .and_then(|p| p.file_name().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("grpcurl"));
    let mut parts = vec![grpcurl.to_string_lossy().to_string()];
    parts.extend(args.iter().map(|a| shell_arg(a)));
    copy_command(&parts.join(" "));
    Ok(())
}

/// Put each JSON message on one line. Keep the text as-is if it is not valid JSON.
fn compact_json_messages(data: &str) -> String {
    let messages: Result<Vec<serde_json::Value>, _> = serde_json::Deserializer::from_str(data)
        .into_iter()
        .collect();
    match messages {
        Ok(messages) if !messages.is_empty() => messages
            .iter()
            .map(|m| m.to_string())
            .collect::<Vec<_>>()
            .join(" "),
        _ => data.to_string(),
    }
}
