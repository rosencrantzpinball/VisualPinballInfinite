# One table in a web browser

The player can be built for the browser: one table, played by the player's own code (its physics, the table's
VBScript in libwinevbs, its renderer and shaders, its sound mixer, its input mapping and pause menu), drawn with
WebGPU. A table's author can put it on any web host that serves static files: the page, the program and the table,
nothing to install, nothing run on the server.

What plays in the browser:

- **The table as on the desktop:** its desktop view, VPX's look (the static prepass, bloom, ambient occlusion, motion
  blur, mirrors where WebGPU has clip distances), its script with VPX's shared scripts (`core.vbs` and the rest), FlexDMD
  and the score display drawn in the view, its sound and music. Sounds longer than the site's setting (2 s for Symphony
  IX's site) are kept as their files and decoded as they play, music above all; the shorter ones, a flipper's or a
  bumper's, are decoded as the table loads and start at once.
- **Keyboard, mouse and gamepads:** the keys as on the desktop (Shift flips, Enter plunges, 1 starts, 5 coins, Z, / and
  Space nudge, Esc the pause menu); any gamepad the browser knows as a standard one, with VPX's gamepad layout (the
  triggers flip, the staged flippers past two thirds, A launches, B starts, Y coins, Back the menu, the left stick
  nudges, the right stick is the plunger) and rumble where the browser has it. The settings' `[Input]` mappings count
  as on the desktop.
- **Touch, on a phone or tablet:** the left and right halves of the screen hold the flippers; Launch (bottom right)
  pulls the plunger back while held and lets it go; the bar along the top has Menu, Coin, Start and Nudge. With the
  pause menu up, a tap chooses, a drag up or down scrolls, a drag sideways moves a bar. Held upright, the desktop view
  fills the phone's height, on one and a half of the screen's pixels to each of the page's.
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
3. `tools/web/package.sh <table folder or .vpx> <out folder> [--title "Name"] [--byline "by ..."] [--with-backglass]
   [--slim] [--stream-sounds seconds]` builds the program (`engine/web`, the `web` profile, the build machine's folders
   left out of it) and lays the site out: `index.html`, `vpi.js`, `gpu.js`, `vpi.wasm`, `manifest.json`, `scripts/`
   (VPX's shared scripts), `table/` (the table's folder, without its `.directb2s` backglass unless asked; a file over
   24 MiB in pieces of 24 MiB, which the page fetches three at a time and joins), `source/` (the program's source as
   built, unless `--no-source`) and `.nojekyll` (GitHub Pages serves the files as they are). A file that gzip shrinks
   by a twentieth or more is gzipped, and the page unzips it.
   - `--slim` puts a smaller copy of the table in the site (`tools/web/slim`, built and run in the image): each PNG,
     BMP or JPEG picture as lossy WebP at quality 90 (95 when that falls short) where it still matches the original by
     SSIM (0.95, on what shows: colour times alpha), lossless WebP otherwise; transparency kept exactly; its
     uncompressed sounds as Ogg Vorbis (quality 0.5), a format VPX plays too, whose sounds start without the added
     silence MP3's and AAC's carry. The table's own file is left as it is.
   - `--stream-sounds 2` has the page keep sounds longer than 2 s as their files, decoded as they play (the player's
     `audio::stream_sounds_over`, mixing the very samples they give decoded whole); 0, the default, decodes them all as
     the table loads.

To try it here: `python3 tools/web/serve.py <out folder>`, then open <http://localhost:8765>.

For testing, the page takes options in its address (several together, joined by `&`):

- `?size=1280x720`: the canvas at that many pixels, whatever the window;
- `?profile`: every render pass timed on the graphics card (WebGPU's timestamp queries, where the browser has them),
  read in the console with `vpi.gpu.profile()`;
- `?manual`: no frames of the page's own, only those `vpi.step(n)` asks for;
- `?plain`: none of WebGPU's optional features, as a browser without them has it;
- `?touch` or `?touch=0`: the touch controls, or none, whatever the device; `?dpr=2`: that many of the canvas's pixels
  to each of the page's at most (2, or 1.5 on a phone or tablet); `?view=fss` or `?view=desktop`: the table's full
  single screen view or its desktop view, whatever the settings say;
- `?stream=10`: sounds longer than that many seconds decoded as they play (0: all decoded as the table loads), whatever
  the site says; `?pictures=program`: the table's pictures decoded by the program, not the browser;
- `?autotest=<name>&seconds=4`: plays that long by itself, then saves `<name>.png` (what WebGPU drew) and `<name>.json`
  (the report below: the browser, its WebGPU, every WebGPU error, memory, frame and mixing times) through
  `serve.py --captures <folder>`;
  `vpi.capture(name)` in the console saves a picture likewise. Opening one such address in each browser gives
  pictures to compare with the player's own (`tools/steam-frame/flat.sh ... --shot`).

On a phone, where there's no console, **Test report** (before play) or **Report** (in the bar) shows the same and
more: the browser and screen, WebGPU's adapter and errors, the program's memory, the fetch and start times, the frame
times, the sound, the player's last lines. **Copy report** copies it to paste into a message. A phone's browser that
runs out of memory reloads the page without a word: the page keeps where it got to in local storage, and the next
visit says where the last one ended.

## Hosting

The out folder goes on the host as it is (on GitHub Pages: the folder's files as a branch's whole tree, Pages
set to deploy from that branch). The host needs nothing special:

- `.wasm` served as `application/wasm` (most hosts do; the page falls back to compiling it whole if not);
- files of 24 MiB at most, which the table's pieces keep to: Cloudflare Pages takes 25 MiB a file, GitHub 100 MB;
- HTTPS (or `localhost`): browsers give WebGPU only to secure pages, so a phone can't try it from a computer on the same
  network by its address;
- nothing for compression: what gzip shrinks comes gzipped (the program, 12 MB, comes as 6), and pictures and sounds
  are compressed already.
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
table's, read only, and `/data`, saved to local storage. The table's files come from the browser's cache when an
earlier visit fetched the same contents (Cache Storage, by each piece's hash in `manifest.json`).

The table's pictures are decoded by the browser (`player::render::set_host_pictures`): each one's file goes to the
page as the renderer would decode it, the page decodes it with the browser's own decoder (`createImageBitmap`, off
the page's thread, a few at a time), copies it into its texture, and makes VPX's mipmaps of it on the graphics card
(`gpu.js`'s `vpxMipmaps`: render/mips.rs's Kaiser kernel, in linear light, as VPX's own compute shader does); the
page waits for them all before the first frame. None of it passes through the program's memory. What the browser
might decode otherwise stays with the program: a PNG that may be transparent (only its texels say), a JPEG with EXIF
data, TGA, Photoshop, OpenEXR and Radiance files.

The graphics card's work is kept in short pieces: macOS stops a browser's GPU work that holds the display too
long ("Impacting Interactivity"), and Chrome then loses its WebGPU device and, after a few such losses, switches
WebGPU off until it's restarted. So in the browser the environment's irradiance is worked out in 16 passes of about
8 ms (one draw of 130 ms elsewhere), the still parts' 128 passes are drawn 4 a frame (all at once elsewhere), and the
page lets the card finish the table's start before its first frame.

Measured on an M4 Pro Mac with Symphony IX 1.0.2. In Chrome 156 at 2274 × 1293 a frame takes the graphics card about
8 ms (median; 10 ms at the 90th percentile) and the page's thread 1.3 ms, 120 frames a second on a 120 Hz screen; in
Safari 26.2, 60 frames a second and 4 ms of the page's thread. The pictures of both match the player's own on Linux
within 3% region by region. Its slim site (`--slim --stream-sounds 2`) against the table as it is, all its sounds
decoded as it loads, its pictures by the program, in Chrome, 30 s of a game:

| | as it is | slim site |
|---|---|---|
| to fetch | 160 MB | 69 MB |
| start, once fetched | 5.3 s | 1.4 s (Safari: 2.5 s) |
| the program's memory | 1,171 MB | 249 MB (Safari: 249 MB) |
| a frame, median | 1.4 ms | 1.4 ms |
| mixing, 99th percentile | 0.2 ms | 0.8 ms (a long sound starting: up to 3.6 ms) |

Its picture matches the table's as it is at 51 dB PSNR (the browser's libwebp; the program's decoder, 38 dB: libwebp
upsamples a picture's colour smoothly). With the processor four times slower, as on a phone, sounds over 2 s decoded
as they play took the start from 26 s to 18 s and never ran the sound dry; mixing then peaked at 15 ms as a long sound
started.

## Licences

The program is the player's, GPL-3.0-or-later, with libwinevbs (Wine's VBScript engine, LGPL-2.1-or-later) linked in;
a site that serves it serves GPL software, so it should point to the source (the page's footer links the project), the
build scripts here included. A table's own licence is its author's.
