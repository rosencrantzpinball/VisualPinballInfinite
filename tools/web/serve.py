#!/usr/bin/env python3
"""Serves a packaged table (tools/web/package.sh's out folder) on this machine, to try it in a browser:

    python3 tools/web/serve.py <out folder> [port]

then open http://localhost:<port> (8765 by default). Nothing is cached, so a rebuilt package shows at once;
WebAssembly goes as application/wasm, which the browser needs to compile it as it arrives."""
import functools
import http.server
import sys


class NoCache(http.server.SimpleHTTPRequestHandler):
    extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map, '.wasm': 'application/wasm', '.js': 'text/javascript', '.vpx': 'application/octet-stream'}

    def end_headers(self):
        self.send_header('Cache-Control', 'no-store')
        super().end_headers()


if __name__ == '__main__':
    folder = sys.argv[1] if len(sys.argv) > 1 else '.'
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 8765
    server = http.server.ThreadingHTTPServer(('127.0.0.1', port), functools.partial(NoCache, directory=folder))
    print(f'serving {folder} at http://localhost:{port}')
    server.serve_forever()
