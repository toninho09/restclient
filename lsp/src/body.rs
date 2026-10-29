//! Body of an HTTP request: inline text, a file (`< ./file`) or a multipart body.

use crate::parser::{file_reference, HttpRequest, RequestBody};
use std::path::Path;

/// Return the bytes of the request body. Return `None` when the request has no body.
/// Relative file paths resolve against `base_dir` (the directory of the .http file).
pub fn request_body(request: &HttpRequest, base_dir: &Path) -> Result<Option<Vec<u8>>, String> {
    match &request.body {
        None => Ok(None),
        Some(RequestBody::Inline(text)) if is_multipart(request) => {
            multipart_body(text, base_dir).map(Some)
        }
        Some(RequestBody::Inline(text)) => Ok(Some(text.clone().into_bytes())),
        Some(RequestBody::FileReference(path)) => read_file(base_dir, path).map(Some),
        Some(RequestBody::Multipart(_)) => {
            Err("Structured multipart bodies are not supported".to_string())
        }
    }
}

/// Return true if the `Content-Type` header is `multipart/...`
fn is_multipart(request: &HttpRequest) -> bool {
    request.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type")
            && value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("multipart/")
    })
}

/// Build a multipart body from its text. A `< path` line adds the bytes of the file.
/// Multipart bodies use CRLF line breaks (RFC 7578).
fn multipart_body(text: &str, base_dir: &Path) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    for line in text.lines() {
        match file_reference(line) {
            Some(path) => body.extend(read_file(base_dir, Path::new(path))?),
            None => body.extend_from_slice(line.as_bytes()),
        }
        body.extend_from_slice(b"\r\n");
    }
    Ok(body)
}

fn read_file(base_dir: &Path, path: &Path) -> Result<Vec<u8>, String> {
    let path = base_dir.join(path);
    std::fs::read(&path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_http_file;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("http-lsp-body-{}-{}", name, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn body(content: &str, base_dir: &Path) -> Result<Option<Vec<u8>>, String> {
        let requests = parse_http_file(content).unwrap();
        request_body(&requests[0], base_dir)
    }

    #[test]
    fn test_inline_and_file_body() {
        let dir = temp_dir("file");
        std::fs::write(dir.join("data.json"), b"{\"a\":1}").unwrap();

        let inline = body("POST https://x\n\n{\"b\": 2}", &dir).unwrap();
        assert_eq!(inline.as_deref(), Some(&b"{\"b\": 2}"[..]));

        let file = body("POST https://x\n\n< ./data.json", &dir).unwrap();
        assert_eq!(file.as_deref(), Some(&b"{\"a\":1}"[..]));

        let missing = body("POST https://x\n\n< ./missing.json", &dir);
        assert!(missing.unwrap_err().contains("missing.json"));

        // An XML body is not a file reference
        let xml = body("POST https://x\n\n<?xml version=\"1.0\"?><a/>", &dir).unwrap();
        assert_eq!(xml.as_deref(), Some(&b"<?xml version=\"1.0\"?><a/>"[..]));

        assert_eq!(body("GET https://x", &dir).unwrap(), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_multipart_body() {
        let dir = temp_dir("multipart");
        std::fs::write(dir.join("a.bin"), [0u8, 159, 146, 150]).unwrap();

        let content = "POST https://x
content-type: Multipart/form-data; boundary=B

--B
Content-Disposition: form-data; name=\"text\"

hello
--B
Content-Disposition: form-data; name=\"file\"; filename=\"a.bin\"

< ./a.bin
--B--";
        let bytes = body(content, &dir).unwrap().unwrap();
        let mut expected = b"--B\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\nhello\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.bin\"\r\n\r\n".to_vec();
        expected.extend_from_slice(&[0, 159, 146, 150]);
        expected.extend_from_slice(b"\r\n--B--\r\n");
        assert_eq!(bytes, expected);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
