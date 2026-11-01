#!/usr/bin/env python3
"""Serves a packaged table (tools/web/package.sh's out folder) on this machine, to try it in a browser:

    python3 tools/web/serve.py <out folder> [port] [--captures <folder>]

then open http://localhost:<port> (8765 by default). Nothing is cached, so a rebuilt package shows at once;
WebAssembly goes as application/wasm, which the browser needs to compile it as it arrives. With --captures, the
page's pictures (`vpi.capture(name)` in its console, for comparing with the player's own) are saved there."""
import functools
import http.server
import os
import sys
import urllib.parse


class NoCache(http.server.SimpleHTTPRequestHandler):
    extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map, '.wasm': 'application/wasm', '.js': 'text/javascript', '.vpx': 'application/octet-stream'}
    captures = None

    def end_headers(self):
        self.send_header('Cache-Control', 'no-store')
        super().end_headers()

    def do_POST(self):
        # a picture from the page: /capture?name=<name>.png, its PNG the body
        url = urllib.parse.urlparse(self.path)
        name = os.path.basename(urllib.parse.parse_qs(url.query).get('name', [''])[0])
        if url.path != '/capture' or not self.captures or not name.endswith(('.png', '.json')):
            self.send_error(404)
            return
        data = self.rfile.read(int(self.headers.get('Content-Length', 0)))
        with open(os.path.join(self.captures, name), 'wb') as f:
            f.write(data)
        print(f'saved {name}: {len(data)} bytes')
        self.send_response(204)
        self.end_headers()


if __name__ == '__main__':
    args = sys.argv[1:]
    if '--captures' in args:
        at = args.index('--captures')
        NoCache.captures = args[at + 1]
        os.makedirs(NoCache.captures, exist_ok=True)
        del args[at:at + 2]
    folder = args[0] if args else '.'
    port = int(args[1]) if len(args) > 1 else 8765
    server = http.server.ThreadingHTTPServer(('127.0.0.1', port), functools.partial(NoCache, directory=folder))
    print(f'serving {folder} at http://localhost:{port}')
    server.serve_forever()
