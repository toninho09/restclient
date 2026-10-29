//! Convert `curl` and `grpcurl` commands to requests (JetBrains HTTP Client syntax).
//! The LSP uses this module for pasted commands and for the "Convert" code action.
//! Nothing here touches the file system.

use crate::parser::parse_request_line;
use serde::de::IgnoredAny;
use std::iter::Peekable;
use std::str::CharIndices;

/// Boundary of the generated multipart bodies (the JetBrains default)
const MULTIPART_BOUNDARY: &str = "WebAppBoundary";

/// Request separator of .http files
const REQUEST_SEPARATOR: &str = "###";

/// The program of a command
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Curl,
    Grpcurl,
}

/// An edit that replaces whole lines with the converted requests.
/// The values are LSP positions: the edit starts at character 0 of `start_line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub start_line: u32,
    pub end_line: u32,
    /// UTF-16 length of `end_line` (LSP positions count UTF-16 code units)
    pub end_character: u32,
    pub new_text: String,
    /// The program of the first command
    pub kind: CommandKind,
}

// ---------------------------------------------------------------------------
// Shell words
// ---------------------------------------------------------------------------

/// Split shell text into commands, and each command into words.
/// Return an error for a quote without end.
fn shell_commands(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut commands = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let (words, end) = read_command(rest)?;
        if !words.is_empty() {
            commands.push(words);
        }
        rest = rest.get(end + 1..).unwrap_or("");
    }
    Ok(commands)
}

/// Read the first command of `text`. Return its words and the byte offset of the line break
/// that ends it (or the length of `text`).
/// - An unquoted line break ends the command. `\` before a line break continues the command.
/// - Supports '...', "...", $'...' (ANSI-C quoting) and `\` escapes.
fn read_command(text: &str) -> Result<(Vec<String>, usize), String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = text.char_indices().peekable();

    while let Some((idx, c)) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some((_, '\n')) | None => {}
                Some((_, '\r')) if chars.peek().is_some_and(|&(_, n)| n == '\n') => {
                    chars.next();
                }
                Some((_, next)) => {
                    word.push(next);
                    in_word = true;
                }
            },
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some((_, '\'')) => break,
                        Some((_, ch)) => word.push(ch),
                        None => return Err("Missing closing single quote".to_string()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some((_, '"')) => break,
                        Some((_, '\\')) => match chars.next() {
                            Some((_, ch @ ('"' | '\\' | '$' | '`'))) => word.push(ch),
                            Some((_, '\n')) => {}
                            Some((_, ch)) => {
                                word.push('\\');
                                word.push(ch);
                            }
                            None => return Err("Missing closing double quote".to_string()),
                        },
                        Some((_, ch)) => word.push(ch),
                        None => return Err("Missing closing double quote".to_string()),
                    }
                }
            }
            '$' if chars.peek().is_some_and(|&(_, n)| n == '\'') => {
                chars.next();
                in_word = true;
                read_ansi_c_quoted(&mut chars, &mut word)?;
            }
            '\n' => {
                if in_word {
                    words.push(word);
                }
                return Ok((words, idx));
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                word.push(c);
                in_word = true;
            }
        }
    }

    if in_word {
        words.push(word);
    }
    Ok((words, text.len()))
}

/// Read the rest of a `$'...'` word. Chrome "Copy as cURL" uses this quoting.
fn read_ansi_c_quoted(chars: &mut Peekable<CharIndices>, word: &mut String) -> Result<(), String> {
    // `\xHH` escapes are bytes, so a UTF-8 character can use more than one escape
    let mut bytes = Vec::new();
    loop {
        let Some((_, c)) = chars.next() else {
            return Err("Missing closing single quote".to_string());
        };
        match c {
            '\'' => break,
            '\\' => {
                let Some((_, esc)) = chars.next() else {
                    return Err("Missing closing single quote".to_string());
                };
                let simple = match esc {
                    'n' => Some(b'\n'),
                    't' => Some(b'\t'),
                    'r' => Some(b'\r'),
                    'a' => Some(0x07),
                    'b' => Some(0x08),
                    'e' | 'E' => Some(0x1b),
                    'f' => Some(0x0c),
                    'v' => Some(0x0b),
                    _ => None,
                };
                if let Some(byte) = simple {
                    bytes.push(byte);
                    continue;
                }
                match esc {
                    'x' => bytes.push(read_hex(chars, 2) as u8),
                    'u' | 'U' => {
                        let max = if esc == 'u' { 4 } else { 8 };
                        let ch = char::from_u32(read_hex(chars, max)).unwrap_or('\u{fffd}');
                        bytes.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                    }
                    '\\' | '\'' | '"' | '?' => bytes.push(esc as u8),
                    other => {
                        bytes.push(b'\\');
                        bytes.extend_from_slice(other.encode_utf8(&mut [0; 4]).as_bytes());
                    }
                }
            }
            c => bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
        }
    }
    word.push_str(&String::from_utf8_lossy(&bytes));
    Ok(())
}

/// Read up to `max` hexadecimal digits
fn read_hex(chars: &mut Peekable<CharIndices>, max: usize) -> u32 {
    let mut value = 0;
    for _ in 0..max {
        match chars.peek().and_then(|&(_, c)| c.to_digit(16)) {
            Some(digit) => {
                value = value * 16 + digit;
                chars.next();
            }
            None => break,
        }
    }
    value
}

/// Return the kind of a program name: `curl`, `curl.exe`, `/usr/local/bin/grpcurl`
fn program_kind(program: &str) -> Option<CommandKind> {
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let name = name.strip_suffix(".exe").unwrap_or(name);
    match name {
        "curl" => Some(CommandKind::Curl),
        "grpcurl" => Some(CommandKind::Grpcurl),
        _ => None,
    }
}

/// Return the kind of the command at the start of `text`: `curl ...`, `$ grpcurl ...`.
/// The command must have an argument after the program.
fn command_kind(text: &str) -> Option<CommandKind> {
    let text = text.trim_start();
    // A shell prompt copied with the command
    let text = text.strip_prefix("$ ").unwrap_or(text).trim_start();
    let program_end = text.find(char::is_whitespace)?;
    if text[program_end..].trim().is_empty() {
        return None;
    }
    program_kind(&text[..program_end])
}

// ---------------------------------------------------------------------------
// Conversion
// ---------------------------------------------------------------------------

/// Convert one or more `curl` or `grpcurl` commands to requests, separated by `###`.
/// Return `None` if the text has other content or a command is not valid.
pub fn convert_commands(text: &str) -> Option<String> {
    let commands = shell_commands(text).ok()?;
    if commands.is_empty() {
        return None;
    }
    let requests = commands
        .iter()
        .map(|words| convert_command(words))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(requests.join(&format!("\n\n{}\n", REQUEST_SEPARATOR)))
}

/// Convert the words of one command
fn convert_command(words: &[String]) -> Result<String, String> {
    let words = match words.first() {
        Some(prompt) if prompt == "$" => &words[1..],
        _ => words,
    };
    let (program, args) = words.split_first().ok_or("Empty command")?;
    match program_kind(program) {
        Some(CommandKind::Curl) => curl_to_http(args),
        Some(CommandKind::Grpcurl) => grpcurl_to_grpc(args),
        None => Err(format!("Not a curl or grpcurl command: {}", program)),
    }
}

/// Format a request: directives, request line, headers and body
fn format_request(
    directives: &[(String, Option<String>)],
    request_line: &str,
    headers: &[(String, String)],
    body: Option<&str>,
) -> String {
    let mut out = String::new();
    for (name, value) in directives {
        match value {
            Some(value) => out.push_str(&format!("# @{} {}\n", name, value)),
            None => out.push_str(&format!("# @{}\n", name)),
        }
    }
    out.push_str(request_line);
    for (name, value) in headers {
        out.push('\n');
        out.push_str(format!("{}: {}", name, value).trim_end());
    }
    if let Some(body) = body {
        out.push_str("\n\n");
        out.push_str(body);
    }
    out
}

/// Format a file path for a `< path` body. A relative path gets `./` (as in JetBrains).
fn file_body(path: &str) -> String {
    let is_explicit = path.starts_with('/')
        || path.starts_with('~')
        || path.starts_with("./")
        || path.starts_with("../")
        || path.get(1..3) == Some(":\\");
    if is_explicit {
        format!("< {}", path)
    } else {
        format!("< ./{}", path)
    }
}

/// Indent a JSON body. Return `None` if the text is not one or more JSON objects or arrays.
fn format_json(text: &str) -> Option<String> {
    let text = text.trim();
    if !text.starts_with(['{', '[']) {
        return None;
    }
    let mut values = serde_json::Deserializer::from_str(text).into_iter::<IgnoredAny>();
    if !values.all(|v| v.is_ok()) {
        return None;
    }
    Some(indent_json(text))
}

/// Indent valid JSON text with two spaces. A stream of values gets one value per block.
/// The function copies strings and numbers as-is, so large numbers do not change.
fn indent_json(text: &str) -> String {
    fn newline(out: &mut String, depth: usize) {
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
    }

    let mut out = String::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    // A value at depth 0 is complete: the next value starts on a new line
    let mut top_value_done = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
                top_value_done = depth == 0;
            }
            continue;
        }
        if c.is_whitespace() {
            if depth == 0 && !out.is_empty() {
                top_value_done = true;
            }
            continue;
        }
        if depth == 0 && top_value_done {
            out.push('\n');
            top_value_done = false;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '{' | '[' => {
                out.push(c);
                let close = if c == '{' { '}' } else { ']' };
                while chars.peek().is_some_and(|n| n.is_whitespace()) {
                    chars.next();
                }
                // Keep empty objects and arrays on one line
                if chars.peek() == Some(&close) {
                    chars.next();
                    out.push(close);
                    top_value_done = depth == 0;
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
                top_value_done = depth == 0;
            }
            ',' => {
                out.push(c);
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c => out.push(c),
        }
    }
    out
}

/// Add a header if the headers do not have it (names are not case-sensitive)
fn add_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    if !headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
        headers.push((name.to_string(), value.to_string()));
    }
}

/// Parse a `Name: value` header. Return `None` for other forms.
fn parse_header(value: &str) -> Option<(String, String)> {
    let (name, value) = value.split_once(':')?;
    let name = name.trim();
    (!name.is_empty() && !name.contains(char::is_whitespace))
        .then(|| (name.to_string(), value.trim().to_string()))
}

// ---------------------------------------------------------------------------
// curl
// ---------------------------------------------------------------------------

/// What a curl option does to the request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CurlOption {
    Request,
    Header,
    Data(DataKind),
    Json,
    Form,
    FormString,
    User,
    Bearer,
    UserAgent,
    Referer,
    Cookie,
    Url,
    Get,
    Head,
    UploadFile,
    /// An option with a value and no effect on the request (`-o file`, `--max-time 5`)
    IgnoreValue,
    /// An option without a value and with no effect on the request (`-s`, `-k`, `--compressed`)
    IgnoreFlag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataKind {
    /// `-d`, `--data`, `--data-ascii`, `--data-binary`: `@file` reads a file
    Plain,
    /// `--data-raw`: `@` is a usual character
    Raw,
    /// `--data-urlencode`
    UrlEncode,
}

impl CurlOption {
    fn takes_value(self) -> bool {
        !matches!(
            self,
            CurlOption::Get | CurlOption::Head | CurlOption::IgnoreFlag
        )
    }
}

/// Short curl options with a value and no effect on the request
const CURL_SHORT_IGNORED_VALUES: &str = "cCDEKmoPQrtUwxYyz";

/// Long curl options with a value and no effect on the request
const CURL_LONG_IGNORED_VALUES: &[&str] = &[
    "abstract-unix-socket",
    "aws-sigv4",
    "cacert",
    "capath",
    "cert",
    "cert-type",
    "ciphers",
    "config",
    "connect-timeout",
    "connect-to",
    "continue-at",
    "cookie-jar",
    "crlfile",
    "dns-servers",
    "doh-url",
    "dump-header",
    "expect100-timeout",
    "ftp-port",
    "happy-eyeballs-timeout-ms",
    "interface",
    "keepalive-time",
    "key",
    "key-type",
    "limit-rate",
    "local-port",
    "max-filesize",
    "max-redirs",
    "max-time",
    "netrc-file",
    "noproxy",
    "output",
    "output-dir",
    "pass",
    "pinnedpubkey",
    "preproxy",
    "proto",
    "proto-default",
    "proto-redir",
    "proxy",
    "proxy-cacert",
    "proxy-cert",
    "proxy-header",
    "proxy-key",
    "proxy-user",
    "quote",
    "range",
    "request-target",
    "resolve",
    "retry",
    "retry-delay",
    "retry-max-time",
    "sasl-authzid",
    "service-name",
    "socks4",
    "socks4a",
    "socks5",
    "socks5-hostname",
    "speed-limit",
    "speed-time",
    "stderr",
    "telnet-option",
    "time-cond",
    "tls-max",
    "trace",
    "trace-ascii",
    "unix-socket",
    "url-query",
    "variable",
    "write-out",
];

fn curl_short_option(c: char) -> CurlOption {
    match c {
        'X' => CurlOption::Request,
        'H' => CurlOption::Header,
        'd' => CurlOption::Data(DataKind::Plain),
        'F' => CurlOption::Form,
        'u' => CurlOption::User,
        'A' => CurlOption::UserAgent,
        'e' => CurlOption::Referer,
        'b' => CurlOption::Cookie,
        'G' => CurlOption::Get,
        'I' => CurlOption::Head,
        'T' => CurlOption::UploadFile,
        c if CURL_SHORT_IGNORED_VALUES.contains(c) => CurlOption::IgnoreValue,
        _ => CurlOption::IgnoreFlag,
    }
}

fn curl_long_option(name: &str) -> CurlOption {
    match name {
        "request" => CurlOption::Request,
        "header" => CurlOption::Header,
        "data" | "data-ascii" | "data-binary" => CurlOption::Data(DataKind::Plain),
        "data-raw" => CurlOption::Data(DataKind::Raw),
        "data-urlencode" => CurlOption::Data(DataKind::UrlEncode),
        "json" => CurlOption::Json,
        "form" => CurlOption::Form,
        "form-string" => CurlOption::FormString,
        "user" => CurlOption::User,
        "oauth2-bearer" => CurlOption::Bearer,
        "user-agent" => CurlOption::UserAgent,
        "referer" => CurlOption::Referer,
        "cookie" => CurlOption::Cookie,
        "url" => CurlOption::Url,
        "get" => CurlOption::Get,
        "head" => CurlOption::Head,
        "upload-file" => CurlOption::UploadFile,
        n if CURL_LONG_IGNORED_VALUES.contains(&n) => CurlOption::IgnoreValue,
        _ => CurlOption::IgnoreFlag,
    }
}

/// Part of the request data (`-d`)
#[derive(Debug, Clone, PartialEq, Eq)]
enum DataPart {
    Text(String),
    File(String),
}

/// Part of a multipart body (`-F`)
#[derive(Debug, Clone, PartialEq, Eq)]
struct FormPart {
    name: String,
    value: FormValue,
    content_type: Option<String>,
    filename: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FormValue {
    Text(String),
    /// `name=@path`: a file upload
    File(String),
    /// `name=<path`: a text part with the content of a file
    FileContent(String),
}

/// The request parts of a curl command
#[derive(Debug, Default)]
struct CurlRequest {
    method: Option<String>,
    urls: Vec<String>,
    headers: Vec<(String, String)>,
    /// Headers from `-u`, `-A`, `-e`, `--oauth2-bearer`. They do not replace a `-H` header.
    implied_headers: Vec<(String, String)>,
    cookies: Vec<String>,
    data: Vec<DataPart>,
    json: bool,
    form: Vec<FormPart>,
    get: bool,
    head: bool,
    upload_file: Option<String>,
}

impl CurlRequest {
    fn apply(&mut self, option: CurlOption, value: Option<String>) -> Result<(), String> {
        let value = value.unwrap_or_default();
        match option {
            CurlOption::Request => self.method = Some(value.to_uppercase()),
            CurlOption::Header => {
                // `Name;` sends an empty header. `Name:` removes a header, `@file` reads a file.
                if let Some(name) = value.strip_suffix(';').filter(|n| !n.contains(':')) {
                    self.headers.push((name.trim().to_string(), String::new()));
                } else if let Some((name, value)) = parse_header(&value) {
                    if !value.is_empty() {
                        self.headers.push((name, value));
                    }
                }
            }
            CurlOption::Data(kind) => self.data.push(data_part(kind, &value)?),
            CurlOption::Json => {
                self.json = true;
                self.data.push(data_part(DataKind::Plain, &value)?);
            }
            CurlOption::Form => self.form.push(form_part(&value, true)?),
            CurlOption::FormString => self.form.push(form_part(&value, false)?),
            CurlOption::User => {
                let credentials = if value.contains(':') {
                    value
                } else {
                    format!("{}:", value)
                };
                self.implied_headers.push((
                    "Authorization".to_string(),
                    format!("Basic {}", base64(credentials.as_bytes())),
                ));
            }
            CurlOption::Bearer => self
                .implied_headers
                .push(("Authorization".to_string(), format!("Bearer {}", value))),
            CurlOption::UserAgent => self.implied_headers.push(("User-Agent".to_string(), value)),
            CurlOption::Referer => {
                // `-e "url;auto"`: `;auto` is a curl setting, not part of the header
                let referer = value.strip_suffix(";auto").unwrap_or(&value).to_string();
                if !referer.is_empty() {
                    self.implied_headers.push(("Referer".to_string(), referer));
                }
            }
            // A value without `=` is a cookie file
            CurlOption::Cookie if value.contains('=') => self.cookies.push(value),
            CurlOption::Url => self.urls.push(value),
            CurlOption::Get => self.get = true,
            CurlOption::Head => self.head = true,
            CurlOption::UploadFile => self.upload_file = Some(value),
            CurlOption::Cookie | CurlOption::IgnoreValue | CurlOption::IgnoreFlag => {}
        }
        Ok(())
    }

    /// Format one request for each URL
    fn build(mut self) -> Result<String, String> {
        if self.urls.is_empty() {
            return Err("The curl command has no URL".to_string());
        }
        if !self.cookies.is_empty() {
            let cookies = self.cookies.join("; ");
            self.implied_headers.push(("Cookie".to_string(), cookies));
        }

        let separator = if self.json { "" } else { "&" };
        let has_data = !self.data.is_empty();
        let has_file_data = self.data.iter().any(|d| matches!(d, DataPart::File(_)));
        if has_file_data && self.data.len() > 1 {
            return Err("Cannot join file data (`-d @file`) with other data".to_string());
        }
        let data_text = || {
            self.data
                .iter()
                .filter_map(|d| match d {
                    DataPart::Text(text) => Some(text.as_str()),
                    DataPart::File(_) => None,
                })
                .collect::<Vec<_>>()
                .join(separator)
        };

        let method = match &self.method {
            Some(method) => method.clone(),
            None if self.head => "HEAD".to_string(),
            None if self.upload_file.is_some() => "PUT".to_string(),
            None if (has_data && !self.get) || !self.form.is_empty() => "POST".to_string(),
            None => "GET".to_string(),
        };

        let mut headers = self.headers.clone();
        for (name, value) in &self.implied_headers {
            add_header(&mut headers, name, value);
        }

        let body = if !self.form.is_empty() {
            headers.retain(|(n, _)| !n.eq_ignore_ascii_case("content-type"));
            headers.push((
                "Content-Type".to_string(),
                format!("multipart/form-data; boundary={}", MULTIPART_BOUNDARY),
            ));
            Some(multipart_text(&self.form))
        } else if let Some(file) = &self.upload_file {
            Some(file_body(file))
        } else if has_data && !self.get {
            if self.json {
                add_header(&mut headers, "Content-Type", "application/json");
                add_header(&mut headers, "Accept", "application/json");
            } else {
                add_header(
                    &mut headers,
                    "Content-Type",
                    "application/x-www-form-urlencoded",
                );
            }
            match self.data.first() {
                Some(DataPart::File(path)) => Some(file_body(path)),
                _ => {
                    let text = data_text();
                    Some(format_json(&text).unwrap_or(text))
                }
            }
        } else {
            None
        };

        if self.get && has_file_data {
            return Err("Cannot put file data (`-d @file`) in the URL (`-G`)".to_string());
        }
        let query = if self.get && has_data {
            Some(data_text())
        } else {
            None
        };

        let requests: Vec<String> = self
            .urls
            .iter()
            .map(|url| {
                let mut url = if url.contains("://") {
                    url.clone()
                } else {
                    // curl uses http:// for a URL without a scheme
                    format!("http://{}", url)
                };
                if let Some(query) = query.as_deref().filter(|q| !q.is_empty()) {
                    url.push(if url.contains('?') { '&' } else { '?' });
                    url.push_str(query);
                }
                format_request(
                    &[],
                    &format!("{} {}", method, url),
                    &headers,
                    body.as_deref(),
                )
            })
            .collect();
        Ok(requests.join(&format!("\n\n{}\n", REQUEST_SEPARATOR)))
    }
}

/// Parse the value of a data option
fn data_part(kind: DataKind, value: &str) -> Result<DataPart, String> {
    match kind {
        DataKind::Raw => Ok(DataPart::Text(value.to_string())),
        DataKind::Plain => match value.strip_prefix('@') {
            Some("-") => Err("Data from stdin (`@-`) is not supported".to_string()),
            Some(path) => Ok(DataPart::File(path.to_string())),
            None => Ok(DataPart::Text(value.to_string())),
        },
        // Forms: `content`, `=content`, `name=content`, `@file`, `name@file`
        DataKind::UrlEncode => {
            if let Some(content) = value.strip_prefix('=') {
                Ok(DataPart::Text(percent_encode(content)))
            } else if let Some((name, content)) = value.split_once('=') {
                Ok(DataPart::Text(format!(
                    "{}={}",
                    name,
                    percent_encode(content)
                )))
            } else if value.contains('@') {
                Err("`--data-urlencode` with a file is not supported".to_string())
            } else {
                Ok(DataPart::Text(percent_encode(value)))
            }
        }
    }
}

/// Parse a `-F` value: `name=value`, `name=@path`, `name=<path`, with `;type=` and
/// `;filename=` settings. `parse` is false for `--form-string` (a literal value).
fn form_part(value: &str, parse: bool) -> Result<FormPart, String> {
    let (name, mut rest) = value
        .split_once('=')
        .ok_or_else(|| format!("Invalid form field: {}", value))?;

    let mut content_type = None;
    let mut filename = None;
    if parse {
        // Read the settings from the end: `value;type=text/plain;filename=a.txt`
        while let Some((head, setting)) = rest.rsplit_once(';') {
            let setting = setting.trim();
            if let Some(t) = setting.strip_prefix("type=") {
                content_type = Some(t.to_string());
            } else if let Some(f) = setting.strip_prefix("filename=") {
                filename = Some(f.trim_matches('"').to_string());
            } else if !setting.starts_with("headers=") {
                break;
            }
            rest = head;
        }
    }

    let value = match (parse, rest.strip_prefix('@'), rest.strip_prefix('<')) {
        (true, Some(path), _) => FormValue::File(path.to_string()),
        (true, _, Some(path)) => FormValue::FileContent(path.to_string()),
        _ => FormValue::Text(rest.to_string()),
    };
    Ok(FormPart {
        name: name.to_string(),
        value,
        content_type,
        filename,
    })
}

/// Format the multipart body of `-F` fields (JetBrains syntax)
fn multipart_text(parts: &[FormPart]) -> String {
    let mut out = String::new();
    for part in parts {
        out.push_str(&format!("--{}\n", MULTIPART_BOUNDARY));
        let mut disposition = format!("Content-Disposition: form-data; name=\"{}\"", part.name);
        let (content_type, content) = match &part.value {
            FormValue::Text(text) => (part.content_type.clone(), text.clone()),
            FormValue::File(path) => {
                let filename = part
                    .filename
                    .clone()
                    .unwrap_or_else(|| path.rsplit(['/', '\\']).next().unwrap_or(path).to_string());
                disposition.push_str(&format!("; filename=\"{}\"", filename));
                let content_type = part
                    .content_type
                    .clone()
                    .unwrap_or_else(|| content_type_for(path).to_string());
                (Some(content_type), file_body(path))
            }
            FormValue::FileContent(path) => (part.content_type.clone(), file_body(path)),
        };
        out.push_str(&disposition);
        out.push('\n');
        if let Some(content_type) = content_type {
            out.push_str(&format!("Content-Type: {}\n", content_type));
        }
        out.push('\n');
        out.push_str(&content);
        out.push('\n');
    }
    out.push_str(&format!("--{}--", MULTIPART_BOUNDARY));
    out
}

/// Return the content type of a file from its extension
fn content_type_for(path: &str) -> &'static str {
    let extension = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

/// Percent-encode a value (RFC 3986 unreserved characters stay as-is)
fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

/// Standard base64 with padding
fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | ((b as u32) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Convert the arguments of a curl command to an HTTP request
fn curl_to_http(args: &[String]) -> Result<String, String> {
    let mut curl = CurlRequest::default();
    let mut args_iter = args.iter();
    let mut only_urls = false;

    let next_value = |args_iter: &mut std::slice::Iter<String>, option: &str| {
        args_iter
            .next()
            .cloned()
            .ok_or_else(|| format!("Missing value for {}", option))
    };

    while let Some(arg) = args_iter.next() {
        if only_urls || !arg.starts_with('-') || arg == "-" {
            curl.urls.push(arg.clone());
        } else if arg == "--" {
            only_urls = true;
        } else if let Some(long) = arg.strip_prefix("--") {
            let (name, inline_value) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (long, None),
            };
            let option = curl_long_option(name);
            let value = match (option.takes_value(), inline_value) {
                (false, _) => None,
                (true, Some(value)) => Some(value),
                (true, None) => Some(next_value(&mut args_iter, arg)?),
            };
            curl.apply(option, value)?;
        } else {
            // Short options can be in a group: `-sSL`, `-XPOST`, `-HAccept:*/*`
            let group = &arg[1..];
            for (pos, c) in group.char_indices() {
                let option = curl_short_option(c);
                if option.takes_value() {
                    let rest = &group[pos + c.len_utf8()..];
                    let value = if rest.is_empty() {
                        next_value(&mut args_iter, arg)?
                    } else {
                        rest.to_string()
                    };
                    curl.apply(option, Some(value))?;
                    break;
                }
                curl.apply(option, None)?;
            }
        }
    }

    curl.build()
}

// ---------------------------------------------------------------------------
// grpcurl
// ---------------------------------------------------------------------------

/// grpcurl flags with a value
const GRPCURL_VALUE_FLAGS: &[&str] = &[
    "alts-handshaker-service",
    "alts-target-service-account",
    "authority",
    "cacert",
    "cert",
    "connect-timeout",
    "d",
    "format",
    "H",
    "import-path",
    "keepalive-time",
    "key",
    "max-msg-sz",
    "max-time",
    "proto",
    "proto-out-dir",
    "protoset",
    "protoset-out",
    "reflect-header",
    "rpc-header",
    "servername",
    "user-agent",
];

/// grpcurl flags without a value (`-flag` or `-flag=true`)
const GRPCURL_BOOL_FLAGS: &[&str] = &[
    "allow-unknown-fields",
    "alts",
    "emit-defaults",
    "expand-headers",
    "format-error",
    "help",
    "insecure",
    "msg-template",
    "plaintext",
    "unix",
    "use-reflection",
    "v",
    "version",
    "veryverbose",
    "vv",
];

/// grpcurl flags with the same name as a request directive
const GRPCURL_DIRECTIVE_FLAGS: &[&str] = &[
    "proto",
    "import-path",
    "protoset",
    "cacert",
    "cert",
    "key",
    "authority",
];

/// Parse a Go boolean flag value
fn parse_go_bool(value: &str) -> Result<bool, String> {
    match value {
        "1" | "t" | "T" | "true" | "TRUE" | "True" => Ok(true),
        "0" | "f" | "F" | "false" | "FALSE" | "False" => Ok(false),
        other => Err(format!("Invalid boolean value: {}", other)),
    }
}

/// Convert the arguments of a grpcurl command to a `GRPC` request
fn grpcurl_to_grpc(args: &[String]) -> Result<String, String> {
    let mut plaintext = false;
    let mut directives: Vec<(String, Option<String>)> = Vec::new();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut data: Option<String> = None;

    // Go flags: `-name`, `--name`, `-name=value`, `-name value`. The first argument that is
    // not a flag ends the flags.
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            i += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        i += 1;

        let flag = arg.strip_prefix("--").unwrap_or(&arg[1..]);
        let (name, inline_value) = match flag.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (flag, None),
        };

        if GRPCURL_BOOL_FLAGS.contains(&name) {
            let enabled = inline_value.map(parse_go_bool).transpose()?.unwrap_or(true);
            match name {
                "plaintext" => plaintext = enabled,
                "insecure" | "emit-defaults" if enabled => {
                    directives.push((name.to_string(), None));
                }
                "unix" if enabled => {
                    return Err("Unix socket addresses (-unix) are not supported".to_string())
                }
                _ => {}
            }
            continue;
        }

        if !GRPCURL_VALUE_FLAGS.contains(&name) {
            return Err(format!("Unknown grpcurl flag: {}", arg));
        }
        let value = match inline_value {
            Some(value) => value.to_string(),
            None => {
                let value = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| format!("Missing value for {}", arg))?;
                i += 1;
                value
            }
        };

        match name {
            "d" => data = Some(value),
            "H" | "rpc-header" | "reflect-header" => {
                if let Some(header) = parse_header(&value) {
                    if !headers.contains(&header) {
                        headers.push(header);
                    }
                }
            }
            "max-time" => directives.push(("timeout".to_string(), Some(value))),
            "connect-timeout" => {
                directives.push(("connection-timeout".to_string(), Some(value)));
            }
            n if GRPCURL_DIRECTIVE_FLAGS.contains(&n) => {
                directives.push((n.to_string(), Some(value)));
            }
            _ => {}
        }
    }

    let (address, rest) = args[i..]
        .split_first()
        .ok_or("The grpcurl command has no address")?;
    let path = match rest {
        [] => None,
        [verb] if verb == "list" || verb == "describe" => None,
        [verb, symbol] if verb == "list" || verb == "describe" => Some(symbol.clone()),
        [method] => Some(method_path(method)?),
        _ => return Err("Too many grpcurl arguments".to_string()),
    };

    // grpcurl uses TLS without -plaintext. The GRPC target is plaintext without a scheme.
    let mut target = if plaintext {
        address.clone()
    } else {
        format!("grpcs://{}", address)
    };
    if let Some(path) = path {
        target.push('/');
        target.push_str(&path);
    }

    let body = data.filter(|d| d != "@" && !d.trim().is_empty()).map(|d| {
        let d = d.trim();
        format_json(d).unwrap_or_else(|| d.to_string())
    });
    Ok(format_request(
        &directives,
        &format!("GRPC {}", target),
        &headers,
        body.as_deref(),
    ))
}

/// Convert a grpcurl method symbol to a target path: `pkg.Service/Method` or
/// `pkg.Service.Method` → `pkg.Service/Method`
fn method_path(symbol: &str) -> Result<String, String> {
    let symbol = symbol.trim_start_matches('/');
    if symbol.contains('/') {
        return Ok(symbol.to_string());
    }
    let (service, method) = symbol
        .rsplit_once('.')
        .ok_or_else(|| format!("Invalid gRPC method: {}", symbol))?;
    Ok(format!("{}/{}", service, method))
}

// ---------------------------------------------------------------------------
// Edits
// ---------------------------------------------------------------------------

fn is_separator(line: &str) -> bool {
    line.trim_start().starts_with(REQUEST_SEPARATOR)
}

/// Convert the commands on the lines `start_line..=end_line` (0-based) of `content`.
/// - Add `###` before the requests if the block already has a request line before them.
/// - Add `###` after the requests if the block has more content after them.
pub fn convert_lines(content: &str, start_line: usize, end_line: usize) -> Option<Replacement> {
    let lines: Vec<&str> = content.split('\n').collect();
    if start_line > end_line || end_line >= lines.len() {
        return None;
    }

    let text = lines[start_line..=end_line].join("\n");
    let kind = command_kind(&text)?;
    let mut new_text = convert_commands(&text)?;

    let block_start = lines[..start_line]
        .iter()
        .rposition(|l| is_separator(l))
        .map_or(0, |i| i + 1);
    if lines[block_start..start_line]
        .iter()
        .any(|l| parse_request_line(l).is_ok())
    {
        new_text = format!("{}\n{}", REQUEST_SEPARATOR, new_text);
    }

    // The next lines of the block would become headers or body of the last request
    let has_content_after = lines[end_line + 1..]
        .iter()
        .take_while(|l| !is_separator(l))
        .any(|l| !l.trim().is_empty());
    if has_content_after {
        new_text.push_str(&format!("\n\n{}", REQUEST_SEPARATOR));
    }

    let last_line = lines[end_line].trim_end_matches('\r');
    Some(Replacement {
        start_line: start_line as u32,
        end_line: end_line as u32,
        end_character: last_line.encode_utf16().count() as u32,
        new_text,
        kind,
    })
}

/// Find the `curl` or `grpcurl` command that contains the 0-based `line`.
/// Return the first and the last line of the command.
pub fn command_at(content: &str, line: usize) -> Option<(usize, usize)> {
    let lines: Vec<&str> = content.split('\n').collect();
    if line >= lines.len() {
        return None;
    }

    // The command starts on the line or above it, in the same block
    let mut start = None;
    for i in (0..=line).rev() {
        if command_kind(lines[i]).is_some() {
            start = Some(i);
            break;
        }
        if is_separator(lines[i]) {
            return None;
        }
    }
    let start = start?;

    let offset: usize = lines[..start].iter().map(|l| l.len() + 1).sum();
    let (_, end) = read_command(&content[offset..]).ok()?;
    let end_line = start + content[offset..offset + end].matches('\n').count();
    (line <= end_line).then_some((start, end_line))
}

/// Find a pasted `curl` or `grpcurl` command: compare the document text before (`old`) and
/// after (`new`) one change. Return the conversion and the text that the change removed.
///
/// The inserted text must start with a command, and only white space can be before it and
/// after it on its lines. One key press inserts one character, so typing never converts.
pub fn pasted_replacement(old: &str, new: &str) -> Option<(Replacement, String)> {
    let prefix = common_prefix_len(old, new);
    let suffix = common_suffix_len(&old[prefix..], &new[prefix..]);
    let inserted = &new[prefix..new.len() - suffix];
    let removed = &old[prefix..old.len() - suffix];

    let text_start = prefix + (inserted.len() - inserted.trim_start().len());
    let text_end = prefix + inserted.trim_end().len();
    if text_start >= text_end {
        return None;
    }
    command_kind(&new[text_start..text_end])?;

    let line_start = new[..text_start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = new[text_end..]
        .find('\n')
        .map_or(new.len(), |i| text_end + i);
    if !new[line_start..text_start].trim().is_empty() || !new[text_end..line_end].trim().is_empty()
    {
        return None;
    }

    let start_line = new[..line_start].matches('\n').count();
    let end_line = start_line + new[line_start..line_end].matches('\n').count();
    let replacement = convert_lines(new, start_line, end_line)?;
    Some((replacement, removed.to_string()))
}

/// Length in bytes of the common start of `a` and `b`, at a character boundary
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or(a.len().min(b.len()), |((i, _), _)| i)
}

/// Length in bytes of the common end of `a` and `b`, at a character boundary
fn common_suffix_len(a: &str, b: &str) -> usize {
    a.chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len_utf8())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc::{self, GrpcOptions};
    use crate::parser::{parse_http_file, HttpMethod, RequestBody};
    use std::path::Path;

    fn words(text: &str) -> Vec<String> {
        let commands = shell_commands(text).unwrap();
        assert_eq!(commands.len(), 1, "{:?}", commands);
        commands.into_iter().next().unwrap()
    }

    fn convert(text: &str) -> String {
        convert_commands(text).unwrap_or_else(|| panic!("no conversion: {}", text))
    }

    fn inline_body(body: &Option<RequestBody>) -> &str {
        match body {
            Some(RequestBody::Inline(text)) => text,
            other => panic!("expected an inline body, got {:?}", other),
        }
    }

    #[test]
    fn test_shell_words() {
        assert_eq!(
            words(r#"curl 'a b' "c \"d\" \$e" f\ g"#),
            vec!["curl", "a b", "c \"d\" $e", "f g"]
        );
        // Line continuation and a JSON body on many lines in quotes
        assert_eq!(
            words("curl -X POST \\\n  -d '{\n  \"a\": 1\n}' \\\r\n  https://x"),
            vec!["curl", "-X", "POST", "-d", "{\n  \"a\": 1\n}", "https://x"]
        );
        // ANSI-C quoting (Chrome "Copy as cURL")
        assert_eq!(
            words(r"curl $'it\'s\n\x41é\xc3\xa9'"),
            vec!["curl", "it's\nAéé"]
        );
        assert!(shell_commands("curl 'abc").is_err());
        assert!(shell_commands("curl \"abc").is_err());
        assert!(shell_commands("curl $'abc").is_err());

        let commands = shell_commands("curl a\n\ncurl b\n").unwrap();
        assert_eq!(commands, vec![vec!["curl", "a"], vec!["curl", "b"]]);
    }

    #[test]
    fn test_command_kind() {
        assert_eq!(command_kind("curl https://x"), Some(CommandKind::Curl));
        assert_eq!(command_kind("  $ curl https://x"), Some(CommandKind::Curl));
        assert_eq!(
            command_kind("/usr/local/bin/grpcurl -plaintext x"),
            Some(CommandKind::Grpcurl)
        );
        assert_eq!(command_kind("curl.exe https://x"), Some(CommandKind::Curl));
        assert_eq!(command_kind("curl"), None);
        assert_eq!(command_kind("curl   "), None);
        assert_eq!(command_kind("curly https://x"), None);
        assert_eq!(command_kind("GET https://x"), None);
    }

    #[test]
    fn test_curl_get() {
        assert_eq!(
            convert("curl https://api.example.com/users"),
            "GET https://api.example.com/users"
        );
        assert_eq!(
            convert("curl -sSL example.com/a -H 'Accept: application/json'"),
            "GET http://example.com/a\nAccept: application/json"
        );
        assert_eq!(convert("curl -I https://x"), "HEAD https://x");
        assert_eq!(convert("curl -XDELETE https://x/1"), "DELETE https://x/1");
        assert_eq!(
            convert("curl --request patch --url https://x"),
            "PATCH https://x"
        );
    }

    #[test]
    fn test_curl_post_json() {
        let block = convert(
            r#"curl -X POST https://api.example.com/users \
  -H "Content-Type: application/json" \
  -H 'Authorization: Bearer abc' \
  -d '{"name":"Ada","id":12345678901234567890123,"tags":[],"meta":{}}'"#,
        );
        assert_eq!(
            block,
            "POST https://api.example.com/users
Content-Type: application/json
Authorization: Bearer abc

{
  \"name\": \"Ada\",
  \"id\": 12345678901234567890123,
  \"tags\": [],
  \"meta\": {}
}"
        );

        let requests = parse_http_file(&block).unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, HttpMethod::POST);
        assert_eq!(requests[0].headers.len(), 2);
        assert!(inline_body(&requests[0].body).contains("12345678901234567890123"));
    }

    #[test]
    fn test_curl_data_forms() {
        // -d without -X is a POST with a form content type
        assert_eq!(
            convert("curl -d a=1 -d b=2 https://x"),
            "POST https://x\nContent-Type: application/x-www-form-urlencoded\n\na=1&b=2"
        );
        assert_eq!(
            convert("curl --data-urlencode 'q=a b&c' https://x"),
            "POST https://x\nContent-Type: application/x-www-form-urlencoded\n\nq=a%20b%26c"
        );
        // -G puts the data in the query string
        assert_eq!(
            convert("curl -G -d a=1 -d b=2 'https://x?z=0'"),
            "GET https://x?z=0&a=1&b=2"
        );
        assert_eq!(
            convert("curl --json '{\"a\":1}' https://x"),
            "POST https://x\nContent-Type: application/json\nAccept: application/json\n\n{\n  \"a\": 1\n}"
        );
        // A file
        assert_eq!(
            convert("curl -H 'Content-Type: application/json' -d @body.json https://x"),
            "POST https://x\nContent-Type: application/json\n\n< ./body.json"
        );
        assert_eq!(
            convert("curl --data-raw '@not-a-file' https://x"),
            "POST https://x\nContent-Type: application/x-www-form-urlencoded\n\n@not-a-file"
        );
        assert_eq!(
            convert("curl -T ../data.bin https://x"),
            "PUT https://x\n\n< ../data.bin"
        );
        assert!(convert_commands("curl -d @a.json -d b=1 https://x").is_none());
        assert!(convert_commands("curl -d @- https://x").is_none());
    }

    #[test]
    fn test_curl_auth_and_implied_headers() {
        assert_eq!(
            convert("curl -u user:passwd https://x"),
            "GET https://x\nAuthorization: Basic dXNlcjpwYXNzd2Q="
        );
        // A -H header has priority
        assert_eq!(
            convert("curl -u a:b -H 'authorization: Token t' https://x"),
            "GET https://x\nauthorization: Token t"
        );
        assert_eq!(
            convert("curl --oauth2-bearer tok -A agent/1 -e 'https://ref;auto' -b 'a=1' -b 'b=2' -b jar.txt https://x"),
            "GET https://x\nAuthorization: Bearer tok\nUser-Agent: agent/1\nReferer: https://ref\nCookie: a=1; b=2"
        );
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
    }

    #[test]
    fn test_curl_ignored_options() {
        assert_eq!(
            convert("curl -k -L --compressed -o out.json -w '%{http_code}' --max-time 5 -x http://proxy:8080 https://x"),
            "GET https://x"
        );
    }

    #[test]
    fn test_curl_form() {
        let block = convert(
            "curl -F 'name=Ada' -F 'photo=@img/a.png' -F 'doc=@r.pdf;type=application/x-pdf;filename=report.pdf' -F 'notes=<notes.txt' https://x/upload",
        );
        assert_eq!(
            block,
            "POST https://x/upload
Content-Type: multipart/form-data; boundary=WebAppBoundary

--WebAppBoundary
Content-Disposition: form-data; name=\"name\"

Ada
--WebAppBoundary
Content-Disposition: form-data; name=\"photo\"; filename=\"a.png\"
Content-Type: image/png

< ./img/a.png
--WebAppBoundary
Content-Disposition: form-data; name=\"doc\"; filename=\"report.pdf\"
Content-Type: application/x-pdf

< ./r.pdf
--WebAppBoundary
Content-Disposition: form-data; name=\"notes\"

< ./notes.txt
--WebAppBoundary--"
        );
        assert_eq!(
            form_part("raw=@literal;x", false).unwrap().value,
            FormValue::Text("@literal;x".to_string())
        );
    }

    #[test]
    fn test_curl_chrome_copy() {
        let block = convert(
            r#"curl 'https://api.example.com/graphql' \
  -H 'accept: */*' \
  -H 'content-type: application/json' \
  -H $'cookie: session=abc; theme=dark' \
  --data-raw $'{"query":"{ me { name } }","note":"it\'s"}' \
  --compressed"#,
        );
        assert_eq!(
            block,
            r#"POST https://api.example.com/graphql
accept: */*
content-type: application/json
cookie: session=abc; theme=dark

{
  "query": "{ me { name } }",
  "note": "it's"
}"#
        );
    }

    #[test]
    fn test_many_commands() {
        assert_eq!(
            convert("$ curl https://a\ncurl https://b"),
            "GET https://a\n\n###\nGET https://b"
        );
        assert!(convert_commands("curl https://a\nsome text").is_none());
        assert!(convert_commands("").is_none());
    }

    #[test]
    fn test_grpcurl() {
        let block = convert(
            r#"grpcurl -plaintext -H 'authorization: Bearer t' -d '{"greeting":"Zed"}' grpcb.in:9000 hello.HelloService/SayHello"#,
        );
        assert_eq!(
            block,
            "GRPC grpcb.in:9000/hello.HelloService/SayHello\nauthorization: Bearer t\n\n{\n  \"greeting\": \"Zed\"\n}"
        );

        let requests = parse_http_file(&block).unwrap();
        assert_eq!(requests[0].method, HttpMethod::GRPC);
        let target = grpc::parse_target(&requests[0].url).unwrap();
        assert!(!target.tls);
        assert_eq!(target.method.as_deref(), Some("SayHello"));
    }

    #[test]
    fn test_grpcurl_tls_and_directives() {
        let block = convert(
            "grpcurl --proto ./api/hello.proto -import-path=./third_party -insecure -max-time 10 \
             -connect-timeout=2 -authority api.internal -emit-defaults -v -format-error \
             -d @ api.example.com:443 hello.HelloService.SayHello",
        );
        assert_eq!(
            block,
            "# @proto ./api/hello.proto
# @import-path ./third_party
# @insecure
# @timeout 10
# @connection-timeout 2
# @authority api.internal
# @emit-defaults
GRPC grpcs://api.example.com:443/hello.HelloService/SayHello"
        );

        let requests = parse_http_file(&block).unwrap();
        let opts = GrpcOptions::from_request(&requests[0], Path::new("/p"), None).unwrap();
        assert!(opts.insecure && opts.emit_defaults);
        assert_eq!(opts.max_time, Some(10.0));
        assert_eq!(opts.connect_timeout, Some(2.0));
        assert_eq!(opts.authority.as_deref(), Some("api.internal"));
        assert!(grpc::parse_target(&requests[0].url).unwrap().tls);
    }

    #[test]
    fn test_grpcurl_list_and_errors() {
        assert_eq!(
            convert("grpcurl -plaintext localhost:50051 list"),
            "GRPC localhost:50051"
        );
        assert_eq!(
            convert("grpcurl -plaintext=true localhost:50051 list hello.HelloService"),
            "GRPC localhost:50051/hello.HelloService"
        );
        // Two messages for a client stream
        assert_eq!(
            convert(r#"grpcurl -plaintext -d '{"a":1} {"a":2}' h:1 s.S/M"#),
            "GRPC h:1/s.S/M\n\n{\n  \"a\": 1\n}\n{\n  \"a\": 2\n}"
        );
        assert!(convert_commands("grpcurl -unknown h:1 s.S/M").is_none());
        assert!(convert_commands("grpcurl -unix /tmp/sock s.S/M").is_none());
        assert!(convert_commands("grpcurl -plaintext").is_none());
        assert!(convert_commands("grpcurl h:1 Method").is_none());
    }

    #[test]
    fn test_indent_json() {
        assert_eq!(
            indent_json("[1,[2,3],{}]"),
            "[\n  1,\n  [\n    2,\n    3\n  ],\n  {}\n]"
        );
        assert_eq!(
            indent_json(r#"{"a":"x,{\"y\":1}"}"#),
            "{\n  \"a\": \"x,{\\\"y\\\":1}\"\n}"
        );
        assert_eq!(format_json("\"text\""), None);
        assert_eq!(format_json("{\"a\":"), None);
    }

    #[test]
    fn test_convert_lines_separators() {
        let content = "### First\nGET https://a\n\ncurl https://b\nGET https://c";
        let r = convert_lines(content, 3, 3).unwrap();
        assert_eq!(r.new_text, "###\nGET https://b\n\n###");
        assert_eq!((r.start_line, r.end_line, r.end_character), (3, 3, 14));
        assert_eq!(r.kind, CommandKind::Curl);

        let content = "### First\n# @name b\ncurl https://b\n\n### Next";
        let r = convert_lines(content, 2, 2).unwrap();
        assert_eq!(r.new_text, "GET https://b");

        // UTF-16 length of the last line
        let r = convert_lines("curl 'https://x/é😀'", 0, 0).unwrap();
        assert_eq!(r.end_character, 20);
    }

    #[test]
    fn test_command_at() {
        let content = "### A\ncurl -X POST https://x \\\n  -d '{\n\n  \"a\": 1\n}'\nGET https://y\n### B\ncurl https://z";
        assert_eq!(command_at(content, 1), Some((1, 5)));
        assert_eq!(command_at(content, 4), Some((1, 5)));
        assert_eq!(command_at(content, 5), Some((1, 5)));
        assert_eq!(command_at(content, 6), None);
        assert_eq!(command_at(content, 7), None);
        assert_eq!(command_at(content, 8), Some((8, 8)));
        assert_eq!(command_at(content, 0), None);
    }

    #[test]
    fn test_pasted_replacement() {
        let old = "### A\nGET https://a\n\n\n### B";
        let new = "### A\nGET https://a\n\ncurl -d x=1 https://b\n\n### B";
        let (r, removed) = pasted_replacement(old, new).unwrap();
        assert_eq!(removed, "");
        assert_eq!((r.start_line, r.end_line), (3, 3));
        assert_eq!(
            r.new_text,
            "###\nPOST https://b\nContent-Type: application/x-www-form-urlencoded\n\nx=1"
        );

        // A paste with line breaks before and after the command
        let (r, _) = pasted_replacement("", "\n  curl https://b \\\n -H 'A: 1'\n").unwrap();
        assert_eq!((r.start_line, r.end_line), (1, 2));
        assert_eq!(r.new_text, "GET https://b\nA: 1");

        // Typing: one character at a time
        assert!(pasted_replacement("curl https://", "curl https://x").is_none());
        assert!(pasted_replacement("curl", "curl ").is_none());
        // Text before or after the paste on the same line
        assert!(pasted_replacement("# ", "# curl https://x").is_none());
        assert!(pasted_replacement("abc", "curl https://xabc").is_none());
        // Not a command
        assert!(pasted_replacement("", "GET https://x").is_none());
        assert!(pasted_replacement("", "curl 'https://x").is_none());
    }

    #[test]
    fn test_pasted_replacement_reports_removed_text() {
        // An undo of a conversion: the block goes away and the command comes back
        let old = "### A\nPOST https://b\n\nx=1\n";
        let new = "### A\ncurl -d x=1 https://b\n";
        let (_, removed) = pasted_replacement(old, new).unwrap();
        assert!(!removed.is_empty());
        assert!("POST https://b\n\nx=1".contains(removed.trim()));
    }
}
