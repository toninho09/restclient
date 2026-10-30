pub const HTTP_METHODS: &[(&str, &str)] = &[
    ("GET", "Retrieve data"),
    ("POST", "Submit data"),
    ("PUT", "Update/replace data"),
    ("PATCH", "Partial update"),
    ("DELETE", "Remove data"),
    ("HEAD", "Get headers only"),
    ("OPTIONS", "Get allowed methods"),
    ("CONNECT", "Establish tunnel"),
    ("TRACE", "Loop-back test"),
    ("GRPC", "gRPC call (grpcurl)"),
];

pub const HEADER_NAMES: &[(&str, &str)] = &[
    ("Content-Type", "application/json"),
    ("Content-Encoding", "gzip"),
    ("Authorization", "Bearer "),
    ("Accept", "application/json"),
    ("Accept-Encoding", "gzip, deflate"),
    ("Accept-Language", "en-US"),
    ("Cache-Control", "no-cache"),
    ("Connection", "keep-alive"),
    ("Cookie", ""),
    ("Host", ""),
    ("Origin", ""),
    ("Referer", ""),
    ("User-Agent", "RestClient/1.0"),
    ("Transfer-Encoding", "chunked"),
    ("X-Request-ID", ""),
    ("X-API-Key", ""),
    ("X-Content-Type-Options", "nosniff"),
    ("X-Frame-Options", "DENY"),
    ("Access-Control-Allow-Origin", "*"),
    (
        "Access-Control-Allow-Methods",
        "GET, POST, PUT, DELETE, OPTIONS",
    ),
    (
        "Access-Control-Allow-Headers",
        "Content-Type, Authorization",
    ),
    ("If-None-Match", ""),
    ("If-Modified-Since", ""),
];

pub const AUTH_SCHEMES: &[&str] = &["Bearer ", "Basic ", "Token "];

/// Common gRPC metadata keys (lowercase, as gRPC sends them)
pub const GRPC_METADATA_NAMES: &[(&str, &str)] = &[
    ("authorization", "Bearer "),
    ("x-api-key", ""),
    ("x-request-id", ""),
    ("x-correlation-id", ""),
    ("x-tenant-id", ""),
    ("user-agent", "RestClient/1.0"),
    ("accept-language", "en-US"),
];

/// Comment directives (`# @name value`) and their descriptions
pub const REQUEST_DIRECTIVES: &[(&str, &str)] = &[
    ("name", "Request name"),
    ("timeout", "gRPC: total time limit (10, 1.5s, 500ms, 2m)"),
    (
        "connection-timeout",
        "gRPC: connection time limit (10, 1.5s, 500ms, 2m)",
    ),
    (
        "proto",
        "gRPC: .proto file to use in place of server reflection",
    ),
    ("import-path", "gRPC: directory for .proto imports"),
    ("protoset", "gRPC: compiled FileDescriptorSet file"),
    ("insecure", "gRPC: skip TLS certificate verification"),
    ("cacert", "gRPC: trusted root certificates file"),
    ("cert", "gRPC: client certificate file (mutual TLS)"),
    ("key", "gRPC: client private key file (mutual TLS)"),
    ("authority", "gRPC: value of the :authority header"),
    (
        "emit-defaults",
        "gRPC: show fields that have default values",
    ),
];

/// Directives that take no value
pub const FLAG_DIRECTIVES: &[&str] = &["insecure", "emit-defaults"];

pub fn header_values(name: &str) -> &'static [&'static str] {
    match name {
        "content-type" => &[
            "application/json",
            "application/xml",
            "application/x-www-form-urlencoded",
            "multipart/form-data",
            "text/plain",
            "text/html",
            "text/csv",
            "application/octet-stream",
        ],
        "accept" => &[
            "application/json",
            "application/xml",
            "text/html",
            "text/plain",
            "*/*",
        ],
        "cache-control" => &[
            "no-cache",
            "no-store",
            "max-age=0",
            "max-age=3600",
            "must-revalidate",
            "public",
            "private",
        ],
        "connection" => &["keep-alive", "close"],
        "accept-encoding" => &[
            "gzip, deflate",
            "gzip, deflate, br",
            "gzip",
            "br",
            "identity",
        ],
        "accept-language" => &[
            "en-US",
            "en-US,en;q=0.9",
            "en-GB",
            "fr-FR",
            "de-DE",
            "zh-CN",
            "ja-JP",
            "*",
        ],
        "content-encoding" => &["gzip", "deflate", "br", "identity"],
        "transfer-encoding" => &["chunked", "compress", "deflate", "gzip"],
        "x-content-type-options" => &["nosniff"],
        "x-frame-options" => &["DENY", "SAMEORIGIN"],
        "access-control-allow-origin" => &["*"],
        "access-control-allow-methods" => {
            &["GET, POST, PUT, DELETE, OPTIONS", "GET, POST, OPTIONS", "*"]
        }
        "access-control-allow-headers" => &["Content-Type, Authorization", "Content-Type", "*"],
        _ => &[],
    }
}
