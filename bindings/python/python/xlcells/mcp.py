"""The MCP server of xlcells, on the WebAssembly module: ``xlcells-mcp`` (or
``python -m xlcells.mcp``) with the arguments of ``xl-mcp``::

    xlcells-mcp [--log FILE]                       an agent on this machine (stdio)
    xlcells-mcp --app DIR [--port N]               the app in the browser, this machine only
    xlcells-mcp --http ADDR --workspace DIR --token SECRET [--public-url URL]

What the server does is the module's (`crates/xl-mcp`, the same code as the program
``xl-mcp``); this side reads and writes the lines of stdio and listens on the network.
"""
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from ._wasm import call


def _stdio():
    for stream in (sys.stdin, sys.stdout):
        try:
            stream.reconfigure(encoding="utf-8", newline="\n")
        except (AttributeError, ValueError):
            pass
    for line in sys.stdin:
        reply = call("mcp_line", line.rstrip("\r\n"))
        if reply is not None:
            sys.stdout.write(reply + "\n")
            sys.stdout.flush()


def _http(how):
    host, port = how["addr"].rsplit(":", 1)
    largest = how["max_body"]

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def _body(self):
            """The body: by its length, or in chunks (as some clients send it)."""
            if "chunked" in (self.headers.get("Transfer-Encoding") or "").lower():
                parts, total = [], 0
                while True:
                    size = int(self.rfile.readline().split(b";")[0].strip() or b"0", 16)
                    if size == 0:
                        while self.rfile.readline() not in (b"\r\n", b"\n", b""):
                            pass
                        return b"".join(parts)
                    total += size
                    if total > largest:
                        return None
                    parts.append(self.rfile.read(size))
                    self.rfile.readline()
            size = int(self.headers.get("Content-Length") or 0)
            if size > largest:
                return None
            return self.rfile.read(size) if size else b""

        def _answer(self):
            body = self._body()
            if body is None:
                self.send_error(413, "the body is larger than the server takes")
                return
            reply = call("mcp_http", self.command, self.path, dict(self.headers.items()), body)
            data = reply["body"]
            self.send_response(reply["status"])
            for name, value in reply["headers"]:
                self.send_header(name, value)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        do_GET = do_POST = do_PUT = do_DELETE = _answer

        def log_message(self, *args):
            pass

    # A thread for each connection: a browser keeps one open and asks on another. The
    # module takes one call at a time (its lock), whichever thread it comes from.
    server = ThreadingHTTPServer((host, int(port)), Handler)
    server.daemon_threads = True
    bound = "{0}:{1}".format(*server.server_address[:2])
    print(call("mcp_banner", bound), flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


def main(argv=None):
    args = sys.argv[1:] if argv is None else list(argv)
    try:
        how = call("mcp_launch", args)
    except ValueError as e:
        print(e, file=sys.stderr)
        return 2
    if how["mode"] == "stdio":
        _stdio()
    else:
        _http(how)
    return 0


if __name__ == "__main__":
    sys.exit(main())
