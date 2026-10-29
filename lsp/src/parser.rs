use crate::variables::VariableResolver;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
    HEAD,
    OPTIONS,
    CONNECT,
    TRACE,
    GRPC,
}

impl std::str::FromStr for HttpMethod {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "GET" => Ok(HttpMethod::GET),
            "POST" => Ok(HttpMethod::POST),
            "PUT" => Ok(HttpMethod::PUT),
            "DELETE" => Ok(HttpMethod::DELETE),
            "PATCH" => Ok(HttpMethod::PATCH),
            "HEAD" => Ok(HttpMethod::HEAD),
            "OPTIONS" => Ok(HttpMethod::OPTIONS),
            "CONNECT" => Ok(HttpMethod::CONNECT),
            "TRACE" => Ok(HttpMethod::TRACE),
            "GRPC" => Ok(HttpMethod::GRPC),
            _ => Err(format!("Unknown HTTP method: {}", s)),
        }
    }
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::GET => "GET",
            HttpMethod::POST => "POST",
            HttpMethod::PUT => "PUT",
            HttpMethod::DELETE => "DELETE",
            HttpMethod::PATCH => "PATCH",
            HttpMethod::HEAD => "HEAD",
            HttpMethod::OPTIONS => "OPTIONS",
            HttpMethod::CONNECT => "CONNECT",
            HttpMethod::TRACE => "TRACE",
            HttpMethod::GRPC => "GRPC",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RequestBody {
    Inline(String),
    FileReference(PathBuf),
    Multipart(Vec<MultipartPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultipartPart {
    pub name: String,
    pub headers: Vec<(String, String)>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResponseHandler {
    File(PathBuf),
    Inline(String), // JavaScript code
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMetadata {
    pub start_line: usize,
    pub end_line: usize,
    pub block_start_line: usize,
    pub name: Option<String>,
    /// Comment directives before the request line: `# @name value` or `// @name value`.
    /// The value is `None` for flag directives such as `# @insecure`.
    pub directives: Vec<(String, Option<String>)>,
}

impl RequestMetadata {
    /// Return all values of a directive, in the order of the file.
    pub fn directive_values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.directives
            .iter()
            .filter(move |(n, _)| n == name)
            .filter_map(|(_, v)| v.as_deref())
    }

    /// Return the last value of a directive.
    pub fn directive(&self, name: &str) -> Option<&str> {
        self.directives
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    /// Return true if the directive is present, with or without a value.
    pub fn has_directive(&self, name: &str) -> bool {
        self.directives.iter().any(|(n, _)| n == name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub http_version: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Option<RequestBody>,
    pub response_handler: Option<ResponseHandler>,
    pub metadata: RequestMetadata,
}

struct RequestBlock {
    content: String,
    start_line: usize,
    end_line: usize,
}

/// Parse an .http file content into a list of HTTP requests
pub fn parse_http_file(content: &str) -> Result<Vec<HttpRequest>, String> {
    parse_http_file_with_globals(content, &std::collections::HashMap::new())
}

pub fn parse_http_file_with_globals(
    content: &str,
    globals: &std::collections::HashMap<String, String>,
) -> Result<Vec<HttpRequest>, String> {
    let mut resolver = VariableResolver::new();
    for (name, value) in globals {
        resolver.set_global(name.clone(), value.clone());
    }
    parse_http_file_with_resolver(content, &mut resolver)
}

/// Parse an .http file using a pre-configured VariableResolver.
/// The resolver may already have environments and globals loaded.
pub fn parse_http_file_with_resolver(
    content: &str,
    resolver: &mut VariableResolver,
) -> Result<Vec<HttpRequest>, String> {
    let mut requests = Vec::new();

    // Parse file-level @variable = value declarations into the resolver
    resolver.parse_file_variables(content);

    // Split by request separator (###) while tracking line numbers
    let blocks = split_by_separator_with_lines(content);

    for block in blocks {
        let trimmed = block.content.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Skip blocks that only contain comments and variable declarations
        let has_non_comment_content = trimmed.lines().any(|line| {
            let line = line.trim();
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with("//")
                && !line.starts_with('@')
        });

        if !has_non_comment_content {
            continue;
        }

        match parse_request_block(&block.content, block.start_line, block.end_line) {
            Ok(mut request) => {
                // Resolve variables in the request
                resolve_request_variables(&mut request, resolver);
                requests.push(request);
            }
            Err(_e) => {}
        }
    }

    Ok(requests)
}

/// Split content by ### separator while tracking line numbers
fn split_by_separator_with_lines(content: &str) -> Vec<RequestBlock> {
    let mut blocks = Vec::new();
    let mut current_block = String::new();
    let mut block_start_line = 1usize;
    let mut current_line = 1usize;

    for line in content.lines() {
        if line.trim().starts_with("###") {
            // Save previous block if it has content
            if !current_block.trim().is_empty() {
                blocks.push(RequestBlock {
                    content: current_block.clone(),
                    start_line: block_start_line,
                    end_line: current_line - 1,
                });
                current_block.clear();
            }
            // Start new block at the NEXT line (after the separator)
            // This ensures code lens appears at the HTTP method line, not the ### line
            block_start_line = current_line + 1;
        } else {
            current_block.push_str(line);
            current_block.push('\n');
        }
        current_line += 1;
    }

    // Push final block
    if !current_block.trim().is_empty() {
        blocks.push(RequestBlock {
            content: current_block,
            start_line: block_start_line,
            end_line: current_line - 1,
        });
    }

    blocks
}

/// Parse a single request block
fn parse_request_block(
    block: &str,
    start_line: usize,
    end_line: usize,
) -> Result<HttpRequest, String> {
    let lines: Vec<&str> = block.lines().collect();
    let mut line_idx = 0;

    // Skip comments, empty lines, and variable declarations; extract directives
    let mut request_name: Option<String> = None;
    let mut directives: Vec<(String, Option<String>)> = Vec::new();
    while line_idx < lines.len() {
        let line = lines[line_idx].trim();
        if line.is_empty() {
            line_idx += 1;
            continue;
        }

        // Skip variable declarations: @variable = value
        if line.starts_with("@") {
            line_idx += 1;
            continue;
        }

        // Check for directives: # @name RequestName, // @timeout 10, # @insecure
        if let Some((name, value)) = parse_directive(line) {
            if name == "name" {
                if let Some(first) = value.as_deref().and_then(|v| v.split_whitespace().next()) {
                    request_name = Some(first.to_string());
                }
            }
            directives.push((name, value));
            line_idx += 1;
            continue;
        }

        // Skip comment lines
        if line.starts_with("#") || line.starts_with("//") {
            line_idx += 1;
            continue;
        }

        // Found non-comment line, break
        break;
    }

    if line_idx >= lines.len() {
        return Err("No request line found".to_string());
    }

    // Calculate the ACTUAL line number where the HTTP method appears
    let actual_request_line = start_line + line_idx;

    // Parse request line
    let (method, url, http_version) = parse_request_line(lines[line_idx])?;
    line_idx += 1;

    // Parse headers
    let mut headers = Vec::new();
    while line_idx < lines.len() {
        let line = lines[line_idx].trim();

        // Empty line marks end of headers
        if line.is_empty() {
            line_idx += 1;
            break;
        }

        // Parse header
        if let Some(colon_idx) = line.find(':') {
            let name = line[..colon_idx].trim().to_string();
            let value = line[colon_idx + 1..].trim().to_string();
            headers.push((name, value));
        }

        line_idx += 1;
    }

    // Parse body (everything after headers until response handler)
    let mut body_lines = Vec::new();
    let mut response_handler = None;

    while line_idx < lines.len() {
        let line = lines[line_idx];

        // Check for response handler
        if line.trim().starts_with(">") {
            let handler_content = line.trim()[1..].trim();
            if handler_content.starts_with("{%") {
                // Inline handler
                let mut handler_code = String::new();
                // Collect until %}
                let first_line = handler_content.trim_start_matches("{%").trim();
                if first_line.ends_with("%}") {
                    handler_code = first_line.trim_end_matches("%}").to_string();
                } else {
                    handler_code.push_str(first_line);
                    line_idx += 1;
                    while line_idx < lines.len() {
                        let next_line = lines[line_idx];
                        if next_line.trim().ends_with("%}") {
                            handler_code.push('\n');
                            handler_code.push_str(next_line.trim().trim_end_matches("%}"));
                            break;
                        }
                        handler_code.push('\n');
                        handler_code.push_str(next_line);
                        line_idx += 1;
                    }
                }
                response_handler = Some(ResponseHandler::Inline(handler_code));
            } else {
                // File reference
                response_handler = Some(ResponseHandler::File(PathBuf::from(handler_content)));
            }
            break;
        }

        body_lines.push(line);
        line_idx += 1;
    }

    // Process body
    let body = if body_lines.is_empty() || body_lines.iter().all(|l| l.trim().is_empty()) {
        None
    } else {
        let body_text = body_lines.join("\n").trim().to_string();

        // Check for file reference
        if let Some(stripped) = body_text.strip_prefix("<") {
            let file_path = stripped.trim();
            Some(RequestBody::FileReference(PathBuf::from(file_path)))
        } else {
            Some(RequestBody::Inline(body_text))
        }
    };

    Ok(HttpRequest {
        method,
        url,
        http_version,
        headers,
        body,
        response_handler,
        metadata: RequestMetadata {
            start_line: actual_request_line,
            end_line,
            block_start_line: start_line,
            name: request_name,
            directives,
        },
    })
}

/// Parse a comment directive: `# @name value`, `// @name value` or `# @name=value`.
/// Return `None` if the line is not a directive comment.
fn parse_directive(line: &str) -> Option<(String, Option<String>)> {
    let rest = if let Some(r) = line.strip_prefix("//") {
        r
    } else {
        line.trim_start_matches('#')
    };
    if rest.len() == line.len() {
        return None;
    }

    let rest = rest.trim_start().strip_prefix('@')?;
    let name_end = rest
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(rest.len());
    let name = &rest[..name_end];
    if name.is_empty() {
        return None;
    }

    let value = rest[name_end..].trim_start_matches('=').trim();
    let value = (!value.is_empty()).then(|| value.to_string());
    Some((name.to_string(), value))
}

/// Find the request at the 1-based `line`. A line between blocks (for example a `###`
/// separator) selects the next request.
pub fn request_at_line(requests: &[HttpRequest], line: usize) -> Option<&HttpRequest> {
    requests
        .iter()
        .find(|r| line >= r.metadata.block_start_line && line <= r.metadata.end_line)
        .or_else(|| {
            requests
                .iter()
                .filter(|r| r.metadata.block_start_line >= line)
                .min_by_key(|r| r.metadata.block_start_line)
        })
}

/// Parse the request line: METHOD URL [HTTP-version]
pub fn parse_request_line(line: &str) -> Result<(HttpMethod, String, Option<String>), String> {
    let parts: Vec<&str> = line.split_whitespace().collect();

    if parts.len() < 2 {
        return Err(format!("Invalid request line: {}", line));
    }

    let method = parts[0].parse::<HttpMethod>()?;
    let url = parts[1].to_string();
    let http_version = if parts.len() > 2 {
        Some(parts[2].to_string())
    } else {
        None
    };

    Ok((method, url, http_version))
}

/// Resolve all variables in a parsed HTTP request
fn resolve_request_variables(request: &mut HttpRequest, resolver: &VariableResolver) {
    // Resolve URL
    request.url = resolver.resolve(&request.url);

    // Resolve header values
    for (_, value) in &mut request.headers {
        *value = resolver.resolve(value);
    }

    // Resolve body if present
    if let Some(RequestBody::Inline(body)) = &mut request.body {
        *body = resolver.resolve(body);
    }

    // Resolve directive values
    for (_, value) in &mut request.metadata.directives {
        if let Some(v) = value {
            *v = resolver.resolve(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variable_substitution_in_url() {
        let content = r#"
@baseUrl = https://api.example.com
@userId = 123

GET {{baseUrl}}/users/{{userId}}
"#;
        let requests = parse_http_file(content).unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://api.example.com/users/123");
    }

    #[test]
    fn test_variable_substitution_in_headers() {
        let content = r#"
@token = secret123
@contentType = application/json

GET https://api.example.com
Authorization: Bearer {{token}}
Content-Type: {{contentType}}
"#;
        let requests = parse_http_file(content).unwrap();
        assert_eq!(requests.len(), 1);

        // Find headers by name
        let auth_header = requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "Authorization")
            .map(|(_, value)| value);
        assert_eq!(auth_header, Some(&"Bearer secret123".to_string()));

        let content_header = requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "Content-Type")
            .map(|(_, value)| value);
        assert_eq!(content_header, Some(&"application/json".to_string()));
    }

    #[test]
    fn test_variable_substitution_in_body() {
        let content = r#"
@username = john_doe
@email = john@example.com

POST https://api.example.com/users
Content-Type: application/json

{
  "username": "{{username}}",
  "email": "{{email}}"
}
"#;
        let requests = parse_http_file(content).unwrap();
        assert_eq!(requests.len(), 1);

        if let Some(RequestBody::Inline(body)) = &requests[0].body {
            assert!(body.contains(r#""username": "john_doe""#));
            assert!(body.contains(r#""email": "john@example.com""#));
        } else {
            panic!("Expected inline body");
        }
    }

    #[test]
    fn test_nested_variable_resolution() {
        let content = r#"
@protocol = https
@host = api.example.com
@baseUrl = {{protocol}}://{{host}}
@version = v1
@endpoint = users

GET {{baseUrl}}/{{version}}/{{endpoint}}
"#;
        let requests = parse_http_file(content).unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://api.example.com/v1/users");
    }

    #[test]
    fn test_undefined_variable_left_as_is() {
        let content = r#"
@defined = value

GET https://{{undefined}}/api/{{defined}}
Authorization: Bearer {{missingToken}}
"#;
        let requests = parse_http_file(content).unwrap();
        assert_eq!(requests.len(), 1);

        // Undefined variables should remain as-is
        assert_eq!(requests[0].url, "https://{{undefined}}/api/value");

        let auth_header = requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "Authorization")
            .map(|(_, value)| value);
        assert_eq!(auth_header, Some(&"Bearer {{missingToken}}".to_string()));
    }

    #[test]
    fn test_line_numbers_minimal() {
        let content = r#"GET https://httpbin.org/get

###

POST https://httpbin.org/post
"#;
        let requests = parse_http_file(content).unwrap();
        assert_eq!(requests.len(), 2);

        // GET is at line 1 (file line 1)
        assert_eq!(
            requests[0].metadata.start_line, 1,
            "GET should be at line 1"
        );

        // POST is at line 5 (file line 5)
        assert_eq!(
            requests[1].metadata.start_line, 5,
            "POST should be at line 5"
        );
    }

    #[test]
    fn test_separator_line_excluded_from_blocks() {
        let content = "### 200 OK\nGET https://httpbin.org/status/200\n\n### demo auto complete\nGET https://httpbin.org/get\nAccept: application/json\n";
        let requests = parse_http_file(content).unwrap();
        assert_eq!(requests.len(), 2);

        // Block 1: GET /status/200 — starts at line 2 (line after first ###)
        assert_eq!(requests[0].metadata.block_start_line, 2);
        assert_eq!(requests[0].metadata.start_line, 2);

        // Block 2: GET /get — starts at line 5 (line after second ###)
        assert_eq!(requests[1].metadata.block_start_line, 5);
        assert_eq!(requests[1].metadata.start_line, 5);

        // Line 4 (### separator) is NOT in any block range
        let line_4_in_block = requests
            .iter()
            .any(|r| 4 >= r.metadata.block_start_line && 4 <= r.metadata.end_line);
        assert!(
            !line_4_in_block,
            "### separator line should not be in any block"
        );

        // Line 1 (first ### separator) is also NOT in any block range
        let line_1_in_block = requests
            .iter()
            .any(|r| 1 >= r.metadata.block_start_line && 1 <= r.metadata.end_line);
        assert!(
            !line_1_in_block,
            "First ### separator line should not be in any block"
        );
    }

    #[test]
    fn test_block_ranges_with_comments_before_first_separator() {
        let content = "# Comment header\n# Another comment\n\n### First request\nGET https://example.com/api\n";
        let requests = parse_http_file(content).unwrap();
        assert_eq!(requests.len(), 1);

        // GET is at line 5, block starts at line 5
        assert_eq!(requests[0].metadata.block_start_line, 5);
        assert_eq!(requests[0].metadata.start_line, 5);
    }

    #[test]
    fn test_parse_http_file_with_globals() {
        let content = r#"
GET https://api.example.com/users/{{token}}
"#;

        let mut globals = std::collections::HashMap::new();
        globals.insert("token".to_string(), "abc123".to_string());

        let requests =
            parse_http_file_with_globals(content, &globals).expect("parse should succeed");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://api.example.com/users/abc123");
    }

    #[test]
    fn test_parse_grpc_request() {
        let content = r#"
@host = localhost:50051

### Unary call
GRPC {{host}}/hello.HelloService/SayHello
authorization: Bearer abc

{
  "greeting": "zed"
}
"#;
        let requests = parse_http_file(content).unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::GRPC);
        assert_eq!(request.method.as_str(), "GRPC");
        assert_eq!(request.url, "localhost:50051/hello.HelloService/SayHello");
        assert_eq!(
            request.headers,
            vec![("authorization".to_string(), "Bearer abc".to_string())]
        );
        assert_eq!(request.metadata.start_line, 5);
        match &request.body {
            Some(RequestBody::Inline(body)) => assert!(body.contains(r#""greeting": "zed""#)),
            other => panic!("Expected inline body, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_grpc_host_only() {
        let content = "GRPC grpcs://grpcb.in:9001\n";
        let requests = parse_http_file(content).unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, HttpMethod::GRPC);
        assert_eq!(requests[0].url, "grpcs://grpcb.in:9001");
        assert!(requests[0].body.is_none());
    }

    #[test]
    fn test_parse_grpc_many_messages_body() {
        let content = r#"GRPC localhost:9000/hello.HelloService/LotsOfGreetings

{ "greeting": "a" }
{ "greeting": "b" }
"#;
        let requests = parse_http_file(content).unwrap();
        match &requests[0].body {
            Some(RequestBody::Inline(body)) => {
                assert_eq!(body, "{ \"greeting\": \"a\" }\n{ \"greeting\": \"b\" }")
            }
            other => panic!("Expected inline body, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_directives() {
        let content = r#"
@protoDir = ./proto

### With directives
# @name sayHello
# @timeout 10
// @connection-timeout 2 s
# @proto {{protoDir}}/hello.proto
# @proto=other.proto
# @insecure
# A plain comment with an @mention
GRPC localhost:50051/hello.HelloService/SayHello
"#;
        let requests = parse_http_file(content).unwrap();
        assert_eq!(requests.len(), 1);
        let meta = &requests[0].metadata;
        assert_eq!(meta.name.as_deref(), Some("sayHello"));
        assert_eq!(meta.directive("name"), Some("sayHello"));
        assert_eq!(meta.directive("timeout"), Some("10"));
        assert_eq!(meta.directive("connection-timeout"), Some("2 s"));
        assert_eq!(
            meta.directive_values("proto").collect::<Vec<_>>(),
            vec!["./proto/hello.proto", "other.proto"]
        );
        assert!(meta.has_directive("insecure"));
        assert_eq!(meta.directive("insecure"), None);
        assert!(!meta.has_directive("mention"));
    }

    #[test]
    fn test_parse_directive_helper() {
        assert_eq!(
            parse_directive("# @name foo"),
            Some(("name".to_string(), Some("foo".to_string())))
        );
        assert_eq!(
            parse_directive("// @no-log"),
            Some(("no-log".to_string(), None))
        );
        assert_eq!(
            parse_directive("## @timeout=5"),
            Some(("timeout".to_string(), Some("5".to_string())))
        );
        assert_eq!(parse_directive("# plain comment"), None);
        assert_eq!(parse_directive("# @"), None);
        assert_eq!(parse_directive("@var = value"), None);
    }
}
