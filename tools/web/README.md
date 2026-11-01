# One table in a web browser

The player can be built for the browser: one table, played by the player's own code (its physics, the table's
VBScript in libwinevbs, its renderer and shaders, its sound mixer, its input mapping and pause menu), drawn with
WebGPU. A table's author can put it on any web host that serves static files: the page, the program and the table,
nothing to install, nothing run on the server.

What plays in the browser:

- **The table as on the desktop:** its desktop view, VPX's look (the static prepass, bloom, ambient occlusion, motion
  blur, mirrors where WebGPU has clip distances), its script with VPX's shared scripts (`core.vbs` and the rest), FlexDMD
  and the score display drawn in the view, its sound and music.
- **Keyboard, mouse and gamepads:** the keys as on the desktop (Shift flips, Enter plunges, 1 starts, 5 coins, Z, / and
  Space nudge, Esc the pause menu); any gamepad the browser knows as a standard one, with VPX's gamepad layout (the
  triggers flip, the staged flippers past two thirds, A launches, B starts, Y coins, Back the menu, the left stick
  nudges, the right stick is the plunger) and rumble where the browser has it. The settings' `[Input]` mappings count
  as on the desktop.
- **The options:** the pause menu's settings and the table's own options are kept in the browser's local storage (the
  player's `Settings.ini` and the table's settings file, in its data folder `/data`), as are the scores the table saves.
  They stay with that browser on that site.

Left out: VR, PinMAME (no ROM tables: an original table, run by its script, plays), PinUP packs, the backglass and score
display windows (the score display the table draws in its own view shows), cabinet devices (DOF, plunger and nudge
boards), screenshots.

## Building and packaging a table

Everything is built from source in the build image (`tools/steam-frame/in-image.sh`), whose Rust has the
`wasm32-wasip1` target and whose clang 19 builds WebAssembly:

1. `tools/steam-frame/in-image.sh tools/web/wasi-libc.sh`: wasi-libc (its `wasi-sdk-28` release), for libwinevbs's C.
2. `tools/steam-frame/in-image.sh tools/web/libwinevbs.sh`: libwinevbs for WebAssembly, the same commit and patches as
   the desktop's (`tools/libwinevbs`), and `patches/` here (Wine's headers given a WebAssembly CPU).
3. `tools/web/package.sh <table folder or .vpx> <out folder> [--title "Name"] [--byline "by ..."] [--with-backglass]`
   builds the program (`engine/web`, the `web` profile) and lays the site out: `index.html`, `vpi.js`, `gpu.js`,
   `vpi.wasm`, `manifest.json`, `scripts/` (VPX's shared scripts) and `table/` (the table's folder, without its
   `.directb2s` backglass unless asked).

To try it here: `python3 tools/web/serve.py <out folder>`, then open <http://localhost:8765>.

For testing, the page takes options in its address (several together, joined by `&`):

- `?size=1280x720`: the canvas at that many pixels, whatever the window;
- `?profile`: every render pass timed on the graphics card (WebGPU's timestamp queries, where the browser has them),
  read in the console with `vpi.gpu.profile()`;
- `?manual`: no frames of the page's own, only those `vpi.step(n)` asks for;
- `?plain`: none of WebGPU's optional features, as a browser without them has it;
- `?autotest=<name>&seconds=4`: plays that long by itself, then saves `<name>.png` (what WebGPU drew) and `<name>.json`
  (the browser, its WebGPU features, every WebGPU error, the frame times) through `serve.py --captures <folder>`;
  `vpi.capture(name)` in the console saves a picture likewise. Opening one such address in each browser gives
  pictures to compare with the player's own (`tools/steam-frame/flat.sh ... --shot`).

## Hosting

The out folder goes on the host as it is. The host needs nothing special:

- `.wasm` served as `application/wasm` (most hosts do; the page falls back to compiling it whole if not);
- compression (gzip or Brotli) if the host offers it: the program is 12 MB, about 6 MB compressed; the table is the big
  download (Symphony IX's 147 MB, which compresses little).
- No cross-origin isolation headers: the program uses no shared memory or threads.

Browsers: those with WebGPU (Chrome and Edge 113 and later, Safari 26, Firefox where WebGPU is on). The page says so when
there's none. Safari's WebGPU has no clip distances: the table's mirrors are left out there.

## How it works

`engine/web` is the program: the player's library (`engine/player`, with its `web` feature) compiled to WebAssembly
(`wasm32-wasip1`), libwinevbs linked in, and the part of SDL the player calls written for the browser
(`engine/web/src/sdl`): the GPU's calls recorded as words and played into WebGPU by the page (`site/gpu.js`), one call a
frame; the shaders made WGSL when the player is built (`engine/player/build.rs`: DXC's SPIR-V, made WGSL by naga), each
with a header of what it binds; sound from SDL's audio callbacks, pulled each frame into an AudioWorklet; the
browser's gamepads as SDL's joysticks; SDL_ttf's part the pause menu uses drawn by ab_glyph. The game runs on the
page's thread, a frame each animation frame: the input, the physics up to the moment (1000 steps a second), VPX's frame
preparation, the picture (`engine/web/src/play.rs`). The page's files are WASI's, kept in memory (`site/vpi.js`): the
table's, read only, and `/data`, saved to local storage.

The graphics card's work is kept in short pieces: macOS stops a browser's GPU work that holds the display too
long ("Impacting Interactivity"), and Chrome then loses its WebGPU device and, after a few such losses, switches
WebGPU off until it's restarted. So in the browser the environment's irradiance is worked out in 16 passes of about
8 ms (one draw of 130 ms elsewhere), the still parts' 128 passes are drawn 4 a frame (all at once elsewhere), and the
page lets the card finish the table's start before its first frame.

Measured on an M4 Pro Mac with Symphony IX 1.0.2: the table starts in about 5 s once fetched. In Chrome 156 at
2274 × 1293 a frame takes the graphics card about 8 ms (median; 10 ms at the 90th percentile) and the page's thread
1.3 ms, 120 frames a second on a 120 Hz screen; in Safari 26.2, 60 frames a second and 4 ms of the page's thread. The
pictures of both match the player's own on Linux within 3% region by region. The program's memory is about 1.2 GB (the
table's pictures and its sounds decoded, as on the desktop).

## Licences

The program is the player's, GPL-3.0-or-later, with libwinevbs (Wine's VBScript engine, LGPL-2.1-or-later) linked in;
a site that serves it serves GPL software, so it should point to the source (the page's footer links the project), the
build scripts here included. A table's own licence is its author's.
