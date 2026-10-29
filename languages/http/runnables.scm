; Mark HTTP request method as runnable - creates ▶ Run button in gutter
; @run captures the method text (GET, POST, etc.)
; @url captures the target URL — becomes $ZED_CUSTOM_url for the task label/terminal title
(request
  method: (method) @run
  url: (target_url) @url
  (#not-eq? @run "GRPC")
  (#set! tag http-request))

; Mark gRPC requests (GRPC host:port/package.Service/Method) with their own tag,
; so the gutter shows the gRPC tasks and not the HTTP tasks
(request
  method: (method) @run
  url: (target_url) @url
  (#eq? @run "GRPC")
  (#set! tag grpc-request))
