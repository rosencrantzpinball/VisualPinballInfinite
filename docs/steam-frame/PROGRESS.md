# Visual Pinball Infinite on the Steam Frame: progress

The plan and its decisions: [PLAN.md](PLAN.md).

## Where it stands

| Milestone | State |
|---|---|
| M0 plan | Done 2026-10-29 |
| M1 flat on Linux | Done 2026-10-29: builds for arm64, plays Symphony IX on Xvfb/lavapipe, picture within 0.62 % of VPX's |
| M2 VR | Done 2026-10-29: both eyes at VPX's placement through OpenXR, controllers, passthrough; under Monado and on the Frame |
| M3 on the Frame | Done 2026-10-29: installed as its own devkit game, started by Steam or by the test harness, VR and headless runs pass |
| M4 quality and speed | Done 2026-10-29: eye-tracked foveation, dynamic resolution (a change of size now costs nothing), specular AA, per-eye culling, per-table `[PlayerVR]`, the perf line; GPU time at or under the fork's (Symphony IX 1920 px, FXAA: 14.4–14.9 ms this morning, 8.6 now, the fork 8.7–8.8; STAR FOX 1728 px: 29.8 → 20.3 ms, the fork 21.2). The gaze following is the owner's to check in the headset |
| M5 beyond Symphony IX | In progress: the Frame's other tables play, chosen from the pause menu in the headset; PinMAME's host and libpinmame built for Linux and running on the Frame (its 2904 games listed). A ROM game waits for ROMs: the Frame's `pinmame/roms` is empty |

## What works

- `vpi-player` builds for `aarch64-unknown-linux-gnu` in the image `vpi-steamrt4-arm64`: SDL_GPU on Vulkan, every shader
  compiled to SPIR-V by DXC at build time, libwinevbs built for Linux from VPI's pin and patches.
- Symphony IX 1.0.2 plays in a window on Xvfb, drawn by lavapipe: its script runs without errors (VPX 5436's
  `core.vbs`/`controller.vbs`, FlexDMD's fonts), scripted keys start a game, 27 fps at 1280×720 on the Mac's CPU.
- The physics reference tests run on arm64 (see "Facts checked").
- VR through OpenXR (`--vr`): each eye drawn through the renderer at VPX 10.8.1's table placement (`[PlayerVR]`
  `Orientation`, `TableX/Y/Z`, `[Player]` `LockbarWidth/Height`), at the table's `[PlayerVR]` `ResFactor` (else the
  runtime's recommended size); opaque, or with passthrough when asked (`[PlayerVR] Passthrough = 1`): the cabinet
  alone, the table's room left out, the real room around it; the controllers bound
  through the Frame's own profile, else Touch, Index, simple; the runtime's GPU time and a perf line every 5 s.
- On the Frame: Steam starts it from the library ("Visual Pinball Infinite (Frame test)"), on SteamOS itself (Steam's
  log: `reaper SteamLaunch … -- run.sh`, no Steam Linux Runtime, whatever the devkit's settings say; the fork too);
  72 fps on Symphony IX at its resolution (1920 px square per eye) with Standard FXAA.
- Foveated rendering as the fork does it: a fragment shading rate image for each eye (full rate around the gaze, 2x2,
  4x4), through a fourth SDL addition (render passes 2 with a rate attachment); the gaze from
  `XR_EXT_eye_gaze_interaction`, else the runtime's foveation centres, else a wide disc straight ahead; `[PlayerVR]`
  `Foveation` (a table's own `.ini` over the settings), `FoveationEyeTracked`, `FoveationFlipX/Y`.
- Specular antialiasing in VR (the fork's): glossy highlights on curved parts and the balls widened by the normal's
  variation over the pixel; no measurable cost.
- PinMAME on Linux: `vpi-pinmame` and libpinmame (VPX 5436's commit, VPI's patches, `tools/pinmame/setup.sh`) beside
  the player; the ROMs from the table's `pinmame/roms`, else the tables' shared one above it (the Frame's
  `Tables/pinmame/roms`); aliases from `VPMAlias.txt` or `alias.txt` there.
- Table scripts read and write their files on Linux (`Scripting.FileSystemObject` through VPI's fence: writes in the
  table's data folder only); the player's 278 tests pass on Linux.
- Culling in VR: each pass leaves out the parts outside its own view (bounding spheres; STAR FOX: 22 % of its triangles).
- Dynamic resolution as the fork does it: each eye drawn at a scale of the table's resolution keeping the runtime's
  GPU time near 85 % of the frame period, down to 70 % (`DynamicResolution`, `DynamicResolutionTarget`,
  `DynamicResolutionMinScale`), steered each frame by small steps (the fork's steering of 2026-10-28); the top left of
  pictures made once at the full size drawn, so a change costs nothing, and that part handed to the runtime
  (`imageRect`). The perf line gives the widths an eye went through in its 5 s.
- VR's mirrors drawn only over the screen's bounds of the parts that show them (VPX's area of interest).
- The parts drawn without a hardware clip distance, which costs Turnip a third of its geometry throughput; a
  mirror's pass cuts away what's below it with an oblique near plane laid on the mirror (decision 17).

Not on Linux yet: PinUP media (FFmpeg bridge), DOF, real DMDs, HDR10, frame pacing on vertical blanks, GDI text boxes
(desktop views only), Windows' own COM objects for scripts let outside, PinMAME's AltSound packs and crash reports.
A ROM game in PinMAME hasn't run yet (no ROMs on the Frame).

Not in VR yet: VPX 10.8.1's part groups placed in cabinet or room space (a table that uses them, such as the blank
table, has its cabinet float; Symphony IX keeps its room in playfield space and is right), the pause menu as a
panel in the room (it draws flat over each eye), an analog plunger (the stick's pull is a key), controller models.

## Next

- The owner: check in the headset that the sharp spot follows the eyes (below).
- More tables: VPX 10.8.1's cabinet and room part groups (the blank table's cabinet floats).
- Heavy tables (STAR FOX: 36 fps, half rate, like the fork): single-pass stereo, as the fork draws (two instances a
  draw, each into its eye's layer), would save the per-draw work once more; a second instance costs VPI about 75 %
  of the first (below), so it would gain less than the clip distance did.
- M5 waits for ROMs: the PinMAME host on Linux and the shared ROM folder; then the backglass in VR and a table
  picker in the headset.
- The library in the headset (PLAN decision 19): CEF 154 is built from source for the Frame (`tools/steam-frame/cef/`,
  2026-10-30: some 58,000 build steps natively on arm64, H.264 and AAC by FFmpeg, HEVC by V4L2) and packaged for
  the `cef` crate (`build.sh distrib`: `CEF_PATH=$WORK/cef/dist/cef_linux_aarch64`, with `CEF_DOWNLOAD_URL`
  pointing nowhere). A test program built with `cef-dll-sys` links it and calls into it: it reports CEF's commit and
  the API hash CEF publishes for Linux. Next: its video on the Frame, then the app's screens as a panel in the room.
- The app's screens in the headset (PLAN decision 20), built and run in the image, not on the Frame yet: `vpi-panel`
  (`engine/panel-host`, CEF off-screen) shows the app's own Library through a shim of Tauri's API, its pictures to
  the player through shared memory (`engine/panel-link`), the player shows them as a quad layer and points into
  them with the controllers. `vpi-player --library` opens VR on it (run.sh does, with no table given); the menu's
  "The library" goes there (exit 76). Under Monado (`vr-monado.sh <run> --library`, `VPI_PANEL_CLICKS`): the library
  drawn and received by the player, a click on a table's PLAY ending the player with it in `frame-table.txt` (75).
  On the Frame (2026-10-30 evening): HEVC plays, decoded by iris with the second patch; the panel runs in Steam Linux
  Runtime 4 (`frame.sh panel`, headless, no OpenXR) once AT-SPI's bridge, which the runtime lacks, goes beside
  libcef.so (`panel.sh atspi`), and draws the owner's 19 tables. "Back to" names the table played last, and shows only
  when there is one. In VR on the Frame (19:25–19:33): library mode at 72 fps (GPU 0.5 ms), the panel's pictures 1 s in,
  a click on PLAY playing the table, on the headless platform and on X11; installed as the Steam game (49c657d), which
  now opens on the library; the panel draws on the GPU (zink on Turnip) there. The owner's worn try (19:34–19:36)
  found the left column garbled under the pointer, the table's pause menu stuck to the head, and no table options:
  the pause menu is now a panel in the room too, pointed at with the controllers (its Table options and The library
  with it); a table's page in the library has its Table settings (the app's own window and module, the options asked
  of the table's script by `vpi-headless`, now built and packaged for Linux); the panels are placed again as the
  headset gives the controllers and by the left stick's click, and each quad's picture is handed over once written.
  Checked unworn on the Frame (2026-10-30 night): the panel placed, the pause menu a panel with hints for the
  controllers, a table's settings read from its script there; a table's settings now saved in VPI's own folder
  (`table-settings`), never beside the tables (PLAN decision 9), read from there once saved; the library ends when
  Steam stops the game. The owner's second worn try (2026-10-31): table options changed in the pause menu were kept
  in VPI's own folder, the table's file untouched; the panel still broke into blocks of black and white under the
  pointer, though SteamVR's recording of it shows it whole. SteamVR draws quads in its lens pass, apart from what it
  records, and the image being written was the shown swapchain's; each picture also held its frame about 11 ms,
  waiting for its own copy behind Chromium's GPU work. The panel now has two swapchains: one shown while the next
  picture is written into the other, swapped once the GPU has written it, the frame waiting for nothing (up to 36
  pictures a second); pictures with blocks of solid black or white are logged and kept (`panel-checks`). Checked
  unworn and installed: b10b8b8. Next: the owner's worn try of the left column.

## Playing it on the Frame

Steam library → Non-Steam → **Visual Pinball Infinite (Frame test)**. It plays the table last chosen (at first
Symphony IX 1.0.2 from `~/Documents/VPinballX/Tables`). To change it: the pause menu (View, left) → Change the table →
pick one with the d-pad and A; the game starts again with it. Its settings are `Settings.ini` in
`~/.local/share/VisualPinballInfinite`, its log `vpi-player.log` there, the table chosen in `frame-table.txt`. Steam's dashboard may open over it at launch: close it (or Resume Game) to give the game the
controllers.

| Control (Frame controllers) | Does |
|---|---|
| Triggers | Flippers (past 60 %, a staged upper flipper too) |
| Grips | Magna save left / right |
| A / B (right) | Start / Coin |
| Right stick pulled back, or clicked | Plunger (pulled while held) |
| Bumpers | Nudge left / right |
| Left stick click | Recenter the table in front of you |
| View (left) | Pause menu: the d-pad moves, A picks |
| Menu (right) | The menu with Quit picked: A quits |

ROM tables: the ROM zips in `~/Documents/VPinballX/Tables/pinmame/roms` (where VPX on the Frame keeps them), aliases in
`pinmame/alias.txt` with LF line endings; PinMAME's NVRAM and settings go in the player's own data folder.

Passthrough: off. With `Passthrough = 1` under `[PlayerVR]` (in `Settings.ini`, or a table's own `.ini`) the cabinet
stands in your own room: the table's room is left out (the log names what) and the cameras' picture shows around it.

Foveation: the picture is sharp where you look and coarser further out, following your eyes when eye tracking is on in
the headset's settings (else sharp in the middle, coarser at the edges). `Foveation` under `[PlayerVR]` in a table's
own `.ini` (0 off, 1 low, 2 medium, 3 high), as with the VPX fork; Symphony IX's says 1. To check the eye tracking:
add `FoveationDebug = 1` under `[PlayerVR]` in `~/.local/share/VisualPinballInfinite/Settings.ini`; only a small spot
is sharp then, and it should be wherever you look. Remove the line afterwards.

## How to build and test

All from the repository, on the Mac, with OrbStack running.

```sh
# once: DXC from source (~25 min), then the image (~2 min)
tools/steam-frame/build-dxc.sh
tools/steam-frame/image.sh
# libwinevbs for Linux, into vpx-build/linux-aarch64/libwinevbs
tools/steam-frame/in-image.sh tools/libwinevbs/setup.sh
# VPX 5436's shared scripts (core.vbs, …), which a development build reads from vpx-build/vpinball-5436/scripts
git -C ../ref/vpinball-steam-frame archive af26b2d scripts | tar -x -C vpx-build/vpinball-5436/
# libpinmame for Linux, into vpx-build/linux-aarch64/pinmame-5436 (PinMAME at VPX 5436's commit, VPI's patches)
tools/steam-frame/in-image.sh tools/pinmame/setup.sh
# the player and the PinMAME host
tools/steam-frame/in-image.sh cargo build --release -p player -p pinmame-host
# a flat run on Xvfb (lavapipe), with its own settings and data folder; a picture at 3 s
tools/steam-frame/flat.sh ../runs/<name> <table.vpx> --size 1280x720 --shot ../runs/<name>/shot.png --at 3000
# the physics against VPX's own test runs
tools/steam-frame/in-image.sh cargo test --release -p physics-compat
```

Picture check against VPX, as VPI measures it (960×540, 1.5 s after start, no script; a pixel differs past 24/255):
the reference comes from the Linux VPX build the Symphony IX project's runner uses, with an empty `<table>.vbs` beside
a copy of the table so VPX runs it without its script:

```sh
python linux_vpx.py <copy>/<table>.vpx --run-dir <dir> --seconds 25 --capture --size 960x540 --ini Player.MSAASamples=0
tools/steam-frame/flat.sh <run> <table.vpx> --size 960x540 --no-script --shot <run>/shot.png --at 1500
python -I tools/steam-frame/compare.py <run>/shot.png <dir>/runtime.png <run>/diff.png
```

## Remote runs on the Frame

`tools/steam-frame/frame.sh` does it all from the Mac over ssh; each run in its own folder under `~/vpi-test/runs`
on the Frame, its own settings and data folder, collected into `runs/frame/<name>`:

- Free? Nothing launched by Steam (`reaper SteamLaunch`) and no `vpinball`/`vpi-player` of anyone else's running.
- VR, started directly: in the session's environment (`XDG_RUNTIME_DIR=/run/user/1000`, the user's D-Bus) through the
  runtime's entry point, in a process group of its own (`setsid`), so stopping it stops only it:
  `env … SteamLinuxRuntime_4-arm64/_v2-entry-point --verb=run -- ./run.sh <table> --vr-pin-eyes --vr-timings … --quit-after <ms>`.
  SteamVR's OpenXR runtime is found as for any game (its active runtime file).
- VR, as the owner starts it: `steam steam://rungameid/<shortcut id << 32 | 0x02000000>` written to
  `~/.steam/steam.pipe`, the shortcut's id read from `shortcuts.vdf` by its devkit game id; Steam starts
  `reaper SteamLaunch AppId=… -- run.sh` on SteamOS itself (not in the runtime); stopped by SIGTERM to `vpi-player`,
  which ends cleanly through SDL's quit.
- Against the fork (`tools/steam-frame/versus.sh prepare`, then `start <name> offline|worn|…` and `wait <name>`):
  copies of Symphony IX 1.0.2 and STAR FOX in `~/vpi-test/versus/tables`; each run starts VPI or the fork as Steam
  starts them (the fork's own start command less `-Launcher`, `-Play` a copy, `-PrefPath` a copy of the owner's
  settings, `VPX_XR_METRICS=1`), waits until it draws, settles, measures, stops it (the fork by SIGINT, its own
  clean close). An OpenXR API layer of ours (`xr-test-layer/`, `XR_API_LAYER_PATH`, `XR_ENABLE_API_LAYERS`) times
  every frame of both alike and, unworn, says draw and pins the eyes where VPI's test runs pin them. Each second the
  kernel's count of each one's GPU time (`drm-engine-gpu` in `/proc/<pid>/fdinfo`) and the compositor's. The owner's
  folders are listed before and after (unchanged in every run so far).
- Unworn, SteamVR runs the session to SYNCHRONIZED with `shouldRender` false: nothing is drawn, correctly. Test runs
  pin the eyes where a player stands and draw anyway (`--vr-pin-eyes`).
- Steam's dashboard at launch (seen by the fork's owner, not here: it needs the headset worn): the session is
  VISIBLE but not FOCUSED while it shows, so frames are drawn and the controllers' actions are inactive; the player
  reads them only when FOCUSED, so nothing reaches the table until the dashboard closes.
- Headless: SDL's offscreen video on the Frame's GPU, no OpenXR, `--shot` at a time.

## Facts checked

- 2026-10-29: VPI's repository was created 2026-10-24; releases 0.1.0 (2026-10-28 07:28 UTC) and 0.1.1 (17:20 UTC);
  `main` is `3cd170b`, the workspace version 0.1.1, GPL-3.0-or-later, Rust 1.98, edition 2024.
- SDL 3.4.18 (vendored) has no OpenXR support for SDL_GPU (no `SDL_openxr.h`); its Vulkan backend takes extra
  instance/device extensions and feature chains (`SDL_PROP_GPU_DEVICE_CREATE_VULKAN_OPTIONS_POINTER`) and creates its
  descriptor pools without `FREE_DESCRIPTOR_SET_BIT`.
- The Frame (2026-10-29): SteamOS (kernel 6.18, glibc 2.39), Turnip Mesa 26.3.0-devel on an Adreno 750 (Vulkan 1.4),
  SteamVR's OpenXR runtime (`/opt/steamvr/steamxr_linuxarm64.json`), Valve's FDM injection layer as an implicit OpenXR
  layer, `SteamLinuxRuntime_4-arm64` 4.0.20261021, Steam started with `-gamepadui -steamdeck -steamos3 -vrgamepadui`
  inside `gamescope --backend openvr` (Xwayland `:0` and `:1`); `/home` 82 GB free.
- Passthrough by alpha blend from scene depth is in the owner's fork (branch `steamframe-opt-overnight`:
  `VRDevice.h` prefers `XR_ENVIRONMENT_BLEND_MODE_ALPHA_BLEND` on Linux, `fs_pp_environment_alpha.sc`); SPD13's fork,
  installed on the Frame at `bb04b2a41`, asks for opaque, then additive.
- **libwinevbs on Linux speaks UTF-32**: its CMake build defines `WINE_UNICODE_NATIVE`, so `WCHAR` is the C library's
  32-bit `wchar_t`, and it calls glibc's wide functions (`wcslen`, `wcstod`, `swprintf`…), so building it with
  `-fshort-wchar` isn't an option. The player's COM strings are `OLECHAR` (u16 on Windows, u32 on Linux).
- **HLSL to SPIR-V**: FXC's language is HLSL 2018 (`-HV 2018`: a vector `?:` selects per component) and its constant
  buffer packing is `-fvk-use-dx-layout`. SDL's Vulkan backend binds texture and sampler as one combined image
  sampler per slot; VPI's SMAA sampled pictures through shared samplers, now per picture on Vulkan.
- **Picture against VPX** (2026-10-29, Symphony IX 1.0.2, VPX = the owner's July fork's Linux build on llvmpipe GL):
  VPI's method (960×540, 1.5 s, no script): **0.62 % of pixels differ** (VPI's pass mark: 0.5 %), mean difference
  2.4/255; 0.22 % outside the three pop bumpers, whose cap lights the table file saves as on: VPI draws them lit,
  that VPX build unlit (game logic, not the port). With the script, at different moments of the attract mode
  (1280×720): 1.23 %. VPX's GL picture crushes the darkest tones (banding), VPI's doesn't.
- **Physics on arm64** (2026-10-29): the engine and collision search reference runs match VPX 5436 bit for bit, and
  every drift test passes. The strict per-function tests differ in two ways only, so they are skipped on ARM (run
  them with `--ignored`): glibc's `sinf`/`cosf` differ from Microsoft's in the last bit for about 1 % of angles
  (flipper faces, gates, spinners, keyboard nudges inherit it), and an invalid result's NaN is positive on ARM,
  negative on x86. VPX's own Linux build uses the same glibc.

- **Other tables on the Frame** (2026-10-29, none needs a ROM): Beethowen (an earlier Symphony IX, its script) and STAR
  FOX – BATTLE FOR LYLAT load and play headless on the Frame's GPU. In VR, STAR FOX is bound by its geometry: 2.9 M
  triangles in each eye's parts (Symphony IX: 0.33 M), about 900 draws in the main picture and 300 in the playfield's
  mirror an eye; 35 fps (SteamVR's half rate) at any resolution: GPU 31.6 ms at 1728 px per eye, 28.7 at 1208, 28.5
  with culling; with reflections of the balls only (`[Player] PFReflection = 1`) 21.6. VPX in VR draws the same parts
  (no static prepass there either), so it can't be compared without the headset worn.
- **The first 5 s of a VR session are slow on the Frame** (2026-10-29): GPU about 14 ms a frame, then 9.3 from then on,
  whatever the build, Turnip's autotuner (`profiled` or `prefer_sysmem`) or the pictures taken. In Turnip's trace the
  early frames have 2.7 ms of gaps between the passes of one command buffer (the GPU busy with someone else's work:
  SteamVR's compositor) and slower passes; the GPU's clock (`/sys/class/devfreq/3d00000.gpu`, 231–903 MHz,
  `simple_ondemand`) stays at its floor for the first 4 s of the process. The runtime's, not the player's.

- **VPI against the fork, unworn** (2026-10-29, `versus-offline1`: both drawing through the test layer, eyes pinned,
  fixed size, FXAA, specular AA, foveation without a gaze, attract mode; the fork as installed, its build of
  2026-10-09; per frame, the runtime's GPU time, p50/p99):

  | | Symphony IX 1.0.2, 1920² | STAR FOX, 1728² |
  |---|---|---|
  | VPI | 9.34–9.48 / 10.6–10.7 ms, 72 fps, no missed frame; CPU 3.9 ms | 28.7 / 30.6 ms, 34.6 fps, 63 missed slots in 45 s |
  | Fork | 8.74–8.80 / 10.5 ms, 72 fps, no missed frame; CPU 6.0 ms | 21.2 / 29.8 ms, 36 fps steady |
  | VPI without the clip distance (`versus-oblique1`, 10:52, 903 MHz) | 8.62 / 10.3 ms, 72 fps, no missed frame | 20.3 / 23.1 ms, 36 fps steady |

  The kernel's counts agree (Symphony IX 671–682 against 630–634 ms of GPU a second). The owner's own play the same
  morning, worn: VPI 9.0–10.6 ms p50 at 1744–1920 px (Symphony IX 1.0.2); the fork about 10.3 at 1920 px on the
  Symphony IX v9 builds, and STAR FOX held at its 70 % floor (1210 px) at 21.3 ms, half rate too.
- **Where the fork's frame is cheaper** (Turnip's trace, both eyes, Symphony IX 1920² / STAR FOX 1728²): playfield
  mirror 1.57 / 5.6 ms against the fork's 0.96 / 3.4; main picture 4.04 / 23.1 against 3.69 / 14.6; bloom, tonemap
  and FXAA even (the fork copies its eyes into the swapchain after FXAA, 0.4 ms). Both draw directly (no tiling) and
  stop writing the early depth test (LRZ) at their first blended part that writes depth; sorting VPI's solid parts by
  VPX's every shader variant moved that point but not the time (STAR FOX: 11.5 → 11.2 ms an eye), so it isn't the
  gap. The fork draws both eyes in one pass, two instances a draw (`vs_basic.sc`: `gl_Layer = gl_InstanceID`).
- **Where the fork's geometry was cheaper: VPI's hardware clip distance** (2026-10-29): every part's vertex shader
  wrote `SV_ClipDistance0` for the mirrors, in every pass; the fork writes its clip distance only in its mirror
  variant, as a varying with a discard. Without it (`VPI_TEST_NO_CLIP`, mirrors then unclipped): STAR FOX 29.8 →
  20.1 ms, Symphony IX 9.33 → 8.63 (alternating runs at the full clock). Turnip loses about a third of its geometry
  throughput to it. Now the parts draw without it and a mirror's pass is cut by an oblique near plane on the
  mirror; clipping twins of the pipelines serve where that can't (the eye on a mirror's far side). Alternating
  at 903 MHz: STAR FOX 20.3 ms (twins in the mirrors 22.6, the clip distance everywhere 29.8), Symphony IX 8.62
  (9.09, 9.39); the mirror's picture as with the clip distance (0.018 % of the pixels differ, its depth's rounding).
  Ruled out on the way: instancing (each part drawn twice as instances costs 75–80 % more, so the fork's second eye
  isn't nearly free), the early depth test (above), VPX's mirror area of interest (no change on a cool Frame:
  9.39 against 9.33 ms; kept, as VPX draws).
- **The hiccups the owner saw in VPI** (2026-10-29): the log of the morning's play shows the 5 s windows in which
  dynamic resolution changed the size with their worst frames at 19–29 ms CPU and 18–34 ms GPU, the steady ones
  under 10 ms CPU; the size changed a dozen times in two minutes. Each change remade every picture (a 20–25 ms frame,
  then slower GPU frames: new pictures and render areas, Turnip's autotune measuring again). Now the pictures are
  made once at the eye's full size and the top left drawn, as the fork does: switching between 85 % and 100 % every
  2 s, 1–2 missed frames in 40 s (worst frame 11–14 ms CPU, 16–17 ms GPU) where a change had cost about two; at 85 %
  the picture frames the table as at 100 % (2.2 % of the pixels differ from the full one scaled down: the resolution
  and the attract mode's lights). `TU_AUTOTUNE_ALGO=prefer_sysmem` changes nothing there.
- **VPI against the fork, worn** (2026-10-29 11:33, `versus-worn1`, the owner moving the head side to side, the Frame
  at 93–95 °C and 800–870 MHz): Symphony IX VPI 14.3 ms p50, 68 fps, 115 missed frames in 30 s, the fork 11.2 ms,
  71.5 fps, 16; STAR FOX VPI 29.2 ms, 34 fps, 65 missed, the fork 26.8, 36 fps steady. SteamVR's compositor took
  185 and 166 ms of GPU a second with VPI, 82 and 76 with the fork (82 for both unworn): VPI asked for passthrough
  (alpha blend), the fork for an opaque picture, so the cameras' picture was blended under each of VPI's frames,
  the compositor's work, first on the GPU, delaying VPI's. VPI is now opaque unless passthrough is asked for
  (decision 18). With passthrough, the cabinet alone: Symphony IX leaves out its 32 room parts (floor, columns,
  lanterns, sky, moon, fireworks) and keeps its cabinet, legs, metal trim and topper; STAR FOX leaves out 391 parts
  and 44 flashers of its room (thin lines of its own near the cabinet stay); 71–74 % of the view see-through.
- **Looking far down at STAR FOX, worn again** (2026-10-29 15:52 and 15:55, `look-vpi-1`, `look-blend-1`): the
  flicker the owner saw at 11:33 didn't come back, neither as VPI now draws (opaque) nor with passthrough and the
  table's room drawn as then (`VPI_TEST_PASSTHROUGH_ROOM=1`), the head down to 74–76° and moving. Opaque, SteamVR held
  VPI at half rate (36 Hz) on the table, 19.6 ms of GPU a frame looking more than 40° down, and at 72 Hz looking up
  (about 10 ms). With passthrough its compositor took 161 ms of GPU a second (70 opaque) and VPI 27.3 ms a frame
  looking down (the Frame at 95 °C, its GPU at 720–770 MHz): three times the late frames, some shown for three or four
  refreshes; SteamVR reprojecting late frames didn't make it flicker. Looking down costs VPI less, not more (13.7 ms a
  frame at 70–80° down). The test layer logs each frame's head (pitch, yaw, the eyes' middle), so a moment seen worn
  can be drawn again unworn.
- **Where STAR FOX's frame goes** (2026-10-29 16:41-17:18, unworn, eyes pinned, 1728 px, 903 MHz; test switches that
  keep the draws but take work away, each alternated twice): 19.5 ms in all; with every triangle thrown away after its
  vertex shader (`VPI_TEST_OFFSCREEN`) 15.1; with a vertex shader that reads and shades nothing (`VPI_TEST_NULL_VS`) 6.7;
  with one triangle a draw (`VPI_TEST_ONE_TRIANGLE`) 4.3. So reading and shading the vertices takes 8.5 ms, assembling
  and throwing away the 5.8 M triangles of the two eyes' main and mirror passes 2.4, drawing them 4.4, the 2,000 draws'
  own cost 1.3, and bloom, tonemapping and FXAA about 3. The table has 7.1 M vertices for its 3.2 M triangles (grass
  and plants 150-200 k triangles each, its plunger 156 k), nearly three for each triangle, and only 2,995 of them
  exact copies of another: there's nothing to weld. Putting each mesh's vertices in the order its triangles use them
  gained nothing (19.65 against 19.43 and 19.48 ms) and one of its runs hung the GPU for a second (killed): removed.
  Its mirror draws 0.63 M triangles an eye; leaving out what lies wholly under the mirror gains nothing there (19.47
  against 19.55 ms: the parts under its playfield don't reflect), kept as it can't change a pixel.
- **Heat** (2026-10-29): the Frame warms to 86–95 °C within minutes of back-to-back runs and its GPU clock drops (834,
  then 720 MHz), raising Symphony IX from 9.3 to 10.3–13.7 ms and missing frames; SteamVR's compositor keeps the GPU
  at its full clock between runs. Comparisons alternate their variants and record the temperatures.
- **What Steam gives a game on the Frame** (2026-10-30, a game the owner started): `DISPLAY=:1`, gamescope's second
  Xwayland (`XDG_SESSION_TYPE=x11`, `XDG_CURRENT_DESKTOP=gamescope`); the session's gamescope runs with `--backend
  openvr`. So the headset's panel, started by the player Steam launched, draws on X11 with ANGLE on GLX, the codec
  runs' hardware-decoding setup; the harness's own runs have no `DISPLAY` (`--env DISPLAY=:1` gives them one).

## Frame test runs

All times Pacific (the Frame's clock). "Pinned" = `frame.sh vr`: the eyes pinned where a player stands, drawn while
the headset is unworn. GPU = SteamVR's own count (`/perfmetrics_meta/app/gpu_frametime`), budget 13.9 ms at 72 Hz.

| When | Build | What ran | Result |
|---|---|---|---|
| 10-29 01:58 | 92a3aea | VR pinned, 40 s, Symphony IX 1.0.2 at 1728² (runtime's recommended), no AA | Session IDLE → READY → SYNCHRONIZED, 72.2 fps held, every frame drawn, CPU 3.9 ms/frame; eye pictures right; clean exit. Frame controller profile: 21 of 26 bindings |
| 10-29 02:00 | 92a3aea | Launched through Steam (`frame.sh steam`, steam://rungameid), 30 s, unworn | Runs under `reaper SteamLaunch AppId=3073189407 -- …/run.sh` in the runtime; SYNCHRONIZED, `shouldRender` false so nothing drawn (correct unworn); SIGTERM ends it cleanly through SDL's quit |
| 10-29 02:06 | 24d9580 | VR pinned, 45 s, 1920² (the table's ResFactor 0.2344), Standard FXAA | GPU 14.4–14.9 ms p50 (over budget: 66–68 fps, SteamVR halving the rate at times); GPU 47 → 57 °C |
| 10-29 02:08 | 24d9580 | Same, 45 s, no AA (`--post-aa off`) | GPU 11.6–12.1 ms p50, 72 fps held; GPU 52 → 61 °C. FXAA costs ≈ 2.7 ms a frame here |
| 10-29 02:10 | 24d9580 | Same, no AA, `--set Player.PFReflection=3` | No change (11.6–12.1 ms): with no static prepass (VR), VPX mirrors everything for any mode from 2, and so does VPI |
| 10-29 02:12 | 24d9580 | Same, no AA, 14 s, `--env MESA_GPU_TRACES=print` (Turnip's per-pass trace) | 11.6 ms a frame; per eye: playfield mirror 0.89 ms, scene 2.85, bloom 0.42, copy of the picture for the balls' mirror 0.69, tonemap 0.55, passthrough alpha 0.40. Every pass direct (autotune: sysmem) |
| 10-29 02:33 | ba89d75 | VR pinned, 45 s, 1920², no AA; each eye's pictures swapped, not copied; alpha in the last pass | GPU 9.6–10.0 ms p50 (was 11.6–12.1); GPU 49 → 57 °C |
| 10-29 02:35 | ba89d75 | Same, Standard FXAA | GPU 11.3–12.3 ms p50, p90 13.5–13.8 (was 14.4–14.9): 72 fps held; GPU 54 → 61 °C. Eye pictures right |
| 10-29 02:37 | ba89d75 | Headless (`frame.sh headless`), 1280×720, picture at 3 s | Symphony IX drawn on the Frame's GPU with no window or OpenXR, 5 balls, still parts drawn once in 286 ms |
| 10-29 03:02 | 7dfa305 | VR pinned, 45 s, 1920², FXAA, foveation as the table says (Low), unworn so no gaze: the wide fixed disc | Shading rate images of 8 px texels in effect: GPU 9.3–9.7 ms p50 (11.3–12.3 without); 72 fps; GPU 49 → 57 °C. Centre as sharp as before, 4x4 blocks visible at the far edges of the captures |
| 10-29 03:04 | 7dfa305 | Same with `VPI_XR_FOVEA=0.20,0.38` (Medium's gaze-sized discs, held straight ahead) | GPU 8.6–8.9 ms p50: what eye-tracked Medium gives; GPU 52 → 58 °C |
| 10-29 03:09 | 8f04e51 | The installed build, 20 s, as the owner gets it (unworn: the fixed disc) | GPU 9.2–9.3 ms p50, 72 fps after the first 5 s |
| 10-29 03:11 | 8f04e51 | 30 s with `TU_AUTOTUNE_ALGO=prefer_sysmem` | Same (9.3–9.5 ms); the first 5 s as slow as with `profiled` |
| 10-29 03:13 | 8f04e51 | 30 s, no pictures taken | Same: the slow first 5 s isn't the picture at 5 s |
| 10-29 03:18 | 8f04e51 | 20 s, the GPU's clock logged | 231 MHz until 4 s after the player starts, then 903 MHz to the end |
| 10-29 03:22 | 66a67f9 (first version) | 45 s at 2400² per eye, FXAA, dynamic resolution | Down to 70 % in the slow first seconds, back up to 81 % (1944²) in four steps, 9.6–9.9 ms; each change a late frame (CPU 20–25 ms) and a few slower ones |
| 10-29 03:25 | 66a67f9 (second) | Same, the first 6 s waited out | Oscillated 82–88 %: a change's own late frames read as over the target |
| 10-29 03:27 | 66a67f9 | Same, 60 s, readings left out 1.5 s after a change | One step to 91 % (2176²) after 6 s, then steady 50 s: 11.1–11.7 ms, 72 fps |
| 10-29 03:30 | 66a67f9 | Defaults, 30 s (Symphony IX's 1920², Low foveation, dynamic resolution on) | 9.3–9.5 ms, 72 fps, the scale never moved |
| 10-29 03:33 | 5aef0ac | Headless: Beethowen, STAR FOX | Both load and play (STAR FOX: 1066 draws, 557 images, loaded in 3.6 s) |
| 10-29 03:36 | 5aef0ac | VR, STAR FOX, 45 s, defaults (no table .ini: 1728², Medium foveation) | 35 fps: GPU 31.8 ms, then 28.7 at the 70 % floor (1208²): bound by geometry, not pixels |
| 10-29 03:38 | 5aef0ac | Same, 14 s, Turnip's trace | Per eye: mirror 2.7–4.1 ms (299 draws), main picture 11.5 ms (910 draws); 31.6 ms a frame |
| 10-29 03:42 | 33e2d51 | STAR FOX at 1728², culling | 28.5 ms; 2.92 M triangles an eye, 22 % culled |
| 10-29 03:45 | 33e2d51 | Symphony IX, defaults, after STAR FOX's runs | 9.8 ms (warm GPU); 0.33 M triangles an eye, none culled (its room is one mesh) |
| 10-29 03:46 | 33e2d51 | STAR FOX, `PFReflection = 1` (balls only) | 21.6 ms, still half rate |
| 10-29 03:53 | 51cd619 | Symphony IX, defaults, specular AA on | 9.3–9.4 ms: no cost |
| 10-29 04:02 | the table chooser | Scripted menu presses: up to the table above, choose, on each table run.sh started | Symphony IX 1.0.2 → Original 2026 → STAR FOX → Beethowen (first in the list, so it stayed): four sessions, each ending cleanly |
| 10-29 04:13 | 7367d97 | `vpi-pinmame --sets`, `--rom-files mm_109c` in Steam Linux Runtime 4 | libpinmame loads and answers: 2904 sets; Medieval Madness's ROM files with their sizes and CRCs |
| 10-29 04:20 | a5abab9 | Symphony IX, defaults, 20 s (libwinevbs with the POSIX fence) | 9.1–9.2 ms, 72 fps |
| 10-29 08:23 | 63775b0, the fork | `versus-check1`: both started through the runtime's entry point | Both ended at once: the runtime's container drops `XR_API_LAYER_PATH`, and the fork needs the host's `libGLU`. Steam's log: Steam starts both on SteamOS itself, so the comparisons do too |
| 10-29 08:29 | 63775b0, the fork | `versus-check2`: Symphony IX, both through the test layer, 10 s | VPI 9.14 ms p50, the fork 8.54; no missed frame; the owner's folders unchanged |
| 10-29 08:38 | 63775b0, the fork | `versus-offline1`: Symphony IX, STAR FOX, Symphony IX again, alternating, 45 s each | The table under Facts checked; GPU 69 → 83 °C |
| 10-29 08:43 | 63775b0 | `versus-dynres-old`: Symphony IX, dynamic resolution (target 60 %, then the default) | The default steering: 4 changes, 9 missed frames, worst 25 ms CPU / 30 ms GPU; the session then stopped by the runtime |
| 10-29 08:47 | pictures made once | `versus-dynres-new`: the same | 6 changes, 5 missed frames in 50 s (most at the session's start), worst 14 ms CPU / 17 ms GPU |
| 10-29 08:52 | pictures made once | `versus-steps-new`: the size switched 85 % ↔ 100 % every 2 s (`VPI_XR_SCALE_STEPS`), with and without `prefer_sysmem`; fixed | 1 and 2 missed frames in 40 s (26 changes each); the fixed run throttled (95 °C, 720 MHz) |
| 10-29 08:56 | pictures made once | Captures at 85 % and 100 % | The same framing; reflections, bloom and glows in place |
| 10-29 09:01 | pictures made once, the fork | `versus-trace1`, `versus-trace2`: Turnip's trace of both on both tables | The pass table under Facts checked. The first STAR FOX run stopped: the owner check took a helper the fork started for one of the owner's (fixed: a process's parents are followed) |
| 10-29 09:10 | pictures made once | STAR FOX: the main pass's order of solid parts (`VPI_DEBUG_DRAW_ORDER`), then sorted by VPX's every shader variant | 3 blended parts that write depth, the first 67th; sorted by variant, 11.2 ms an eye instead of 11.5 (dropped) |
| 10-29 09:20 | the mirrors' area | Capture; `versus-aoi1`, `versus-aoi2`: the area against the whole mirror, alternating | Reflections intact. Too hot to measure well (88–95 °C): 10.27 against 10.52 ms, 759 against 781 ms of GPU a second |
| 10-29 10:14 | the mirrors' area | `versus-aoi3`: the area against the whole mirror, alternating, the Frame cool (55 °C at the start, full clock) | 9.39 / 9.39 against 9.34 / 9.32 ms: no gain |
| 10-29 10:21 | test switches | `versus-inst1`: each part drawn as two instances | STAR FOX 29.9 → 53.5 ms, Symphony IX 9.39 → 16.13: a second instance costs 75–80 % of the first |
| 10-29 10:30 | test switches | `versus-noclip1`: the parts without the hardware clip distance (mirrors unclipped) | STAR FOX 29.8 → 20.1 ms, 36 fps steady; Symphony IX 9.33 → 8.63 |
| 10-29 10:34 | clipping twins | Capture; `versus-clip1`: the twins in the mirrors against the clip distance everywhere | Reflections intact; STAR FOX 22.6 against 29.8 ms (62 missed frames → 0), Symphony IX 9.12 against 9.39 |
| 10-29 10:46 | oblique mirrors | Capture; `versus-oblique1`: the oblique near plane against the twins | 0.018 % of the pixels differ from the twins' picture; STAR FOX 20.3 against 22.6 ms, Symphony IX 8.62 against 9.09 |
| 10-29 11:00 | oblique mirrors, the fork | `versus-final1`: the head-to-head again | The Frame hot from the start (87–95 °C), its clock down to 568–780 MHz: no fair numbers; at like clocks (770–790 MHz) STAR FOX VPI 21.3 against the fork's 27.1 ms |
| 10-29 11:33 | 09b9f8a, the fork | `versus-worn1`: worn, the owner moving the head; Symphony IX, STAR FOX | Under Facts checked: VPI missed more frames; the compositor took twice the GPU time with VPI's passthrough. The owner saw parts flicker or vanish looking far down, surely in VPI's STAR FOX (34 fps, 65 frames missed; still pictures show nothing wrong; not seen again in two worn runs at 15:52 and 15:55, below) |
| 10-29 11:44 | passthrough asked for | Symphony IX by default, then with `PlayerVR.Passthrough=1` | OPAQUE by default; with it ALPHA_BLEND and the room left out, but the cabinet's metal trim (legs) and topper with it: bounding spheres too coarse, the box too low |
| 10-29 11:48 | the boxes exact | Symphony IX and STAR FOX with passthrough | The cabinets whole, their legs and topper kept; 32 and 391 room parts left out; 71–74 % see-through |
| 10-29 12:00 | opaque by default | STAR FOX, the eyes pinned looking 70° and 45° down (`VPI_XR_PIN_PITCH`), against no culling, the clipping twins in the mirrors, the whole mirror; then with the script off (`--no-script`, nothing moving) | With the script off 0.02 % of the pixels differ (the dither): culling, the oblique mirror and the mirror's area hide nothing looking down; with it on, 6.5 % (the attract mode) |
| 10-29 12:05 | opaque by default | STAR FOX looking 70° down, the room drawn, the passthrough alpha written (`VPI_XR_ENVIRONMENT_ALPHA`) | 0.003 % see-through: its floor writes depth. The 45° run stopped at the owner's word (the Frame wanted for something else) |
| 10-29 15:52 | b0e2ffa | `look-vpi-1`: worn, STAR FOX as VPI now draws (opaque), the owner looking down to 76° and moving the head; the test layer logs each frame's head | No flicker. Half rate (36 Hz) on the table, 72 Hz looking up; 19.6 ms of GPU a frame looking more than 40° down; 87–92 °C |
| 10-29 15:55 | b0e2ffa | `look-blend-1`: the same with passthrough and the table's room (`VPI_TEST_PASSTHROUGH_ROOM=1`), as at 11:33 | No flicker either (Facts checked): the compositor 161 ms of GPU a second, 245 late frames against 76; 88–95 °C |
| 10-29 16:31 | b0e2ffa, the fork | `versus-worn2`: the worn comparison again, VPI opaque | Not counted: the headset lay still for all four runs, looking up and away from the table (pitch +24-26°, no motion), SteamVR drawing for it as if worn. versus.py now waits for the head to move (91df5ad) |
| 10-29 16:41 | f5f37a6 | `geo-fox-1`: STAR FOX, against every triangle thrown away after its vertex shader (`VPI_TEST_OFFSCREEN`), twice | 19.5 against 15.1 ms |
| 10-29 16:47 | a04d467 | `mcull-fox-1`: STAR FOX, the mirror's culling by its plane against without (`VPI_TEST_NO_MIRROR_CULL`), twice | 19.47 against 19.55 ms; 0.63 M triangles an eye into the mirror both ways |
| 10-29 16:54 | d18743c | `split-fox-1`: STAR FOX, offscreen, a vertex shader reading nothing (`VPI_TEST_NULL_VS`), one triangle a draw (`VPI_TEST_ONE_TRIANGLE`), twice | 15.1, 6.7 and 4.3 ms (Facts checked) |
| 10-29 17:01 | d18743c-dirty | `fetch-on`, `weld-on`, `weld-off`, `weld-off-2`: STAR FOX's pictures, script off, with each mesh's vertices reordered (and exact copies welded) against the file's order | 7.1 M vertices, 2,995 copies; two runs of the same build differ more (0.90 % of pixels by more than 24/255, the lights' blinking) than reordered against not (0.41 %) |
| 10-29 17:10 | d18743c-dirty | `weld-on-70`, `weld-off-70`: the same looking 70° down | 0.10 % of pixels by more than 24, specks at lights' edges |
| 10-29 17:12 | d18743c-dirty | `order-fox-1`: STAR FOX, reordered against the file's order, twice | 19.65 against 19.43 and 19.48 ms; the second reordered run hung the GPU in its first second (976 ms of GPU time) and was killed: the reordering removed |
| 10-30 11:40 | 4b7848e-dirty | `cef-codecs-1`: the CEF built here on the Frame (`tools/steam-frame/cef/frame-codecs.sh`): `cefsimple` on Chromium's headless platform, a page asking what decodes and playing 6 s clips (H.264 with AAC, HEVC, AAC), ANGLE's `gl`; in `bwrap` with a `/dev` of the GPU and `video22` alone | No GPU process (ANGLE's `gl` is GLX: no X display), so all in software: H.264 1080p 118 frames in 4 s, none dropped, AAC plays, VP9 and AV1 in software, HEVC not at all. GPU 50 °C |
| 10-30 11:42 | 4b7848e-dirty | `cef-codecs-2`: the same, ANGLE on EGL (`gl-egl`, Mesa surfaceless) | The GPU: zink on Turnip, Adreno 750. Hardware reported for H.264, HEVC, HEVC Main 10 and VP9 (iris), not AV1. V4L2's decoder refused at `VIDIOC_REQBUFS`: the kernel's iris, "current session not supported(-22)"; H.264 fell back to FFmpeg, HEVC failed |
| 10-30 ~11:55 | — | A probe of ours on `/dev/video22` (Python, `S_FMT` then `REQBUFS` on the input queue, as Chromium does) | 0×0, as Chromium sets it, refused for H.264 and HEVC; 1920×1080 taken (iris pads to 1920×1088; HEVC's 2 buffers made 4). Hence `chromium-v4l2-stateful-coded-size.patch` |
| 10-30 12:02 | 4b7848e-dirty, coded-size patch | `cef-codecs-3`: in a headless gamescope of its own (X11), ANGLE on EGL | "No suitable EGL configs" for the X window: Chromium composites in software and then decodes nothing in hardware |
| 10-30 12:05 | same | `cef-codecs-4`: the same, ANGLE on GLX | Hardware for H.264, HEVC and VP9. H.264 decoded by iris through V4L2 (NV12 1920×1088, libyuv to ARGB): 124 frames in 4 s, none dropped. HEVC: Chromium 154's stateful decoder's HEVC branch is `NOTIMPLEMENTED` (hence `chromium-v4l2-stateful-hevc.patch`, not run on the Frame yet). GPU 52 → 53 °C |
| 10-30 12:06 | same | `cef-codecs-5`: the same, ANGLE on Vulkan | As on EGL: no config for the X window, software. GPU 53 → 54 °C |
| 10-30 18:53 | 91900c2 (CEF with the coded-size and HEVC patches) | `cef-codecs-6`: as `cef-codecs-4` (headless gamescope of its own, ANGLE on GLX) | H.264 and HEVC both decoded by iris through V4L2 (NV12 1920×1088, libyuv to ARGB): H.264 with AAC 124 frames in 4 s, HEVC Main 1080p 125, none dropped. Hardware reported for H.264, HEVC, HEVC Main 10 and VP9; AV1 in software. GPU 54 → 56 °C |
| 10-30 ~18:57 | 91900c2, staged in `~/vpi-test/game` | `ldd` of the panel's libraries inside Steam Linux Runtime 4 (`_v2-entry-point`) | libcef.so needs `libatk-bridge-2.0.so.0` and `libatspi.so.0`, which the runtime lacks (it has ATK alone). Built from at-spi2-core 2.56.2's source (the SDK's release, GNOME's tarball checked against its sha256) and staged beside libcef.so (~19:03): all resolve. The graphics libraries come from SteamOS through the runtime |
| 10-30 19:06 | 91900c2 + at-spi2-core | `panel-1` (`frame.sh panel`, staged): `vpi-panel` alone in the runtime, headless platform, no OpenXR, the owner's library read only, its picture at 8 s; beside the owner's own VPX session (`ALLOW_BUSY=1`, on the owner's word that headless runs may) | The app's Library with all 19 tables, read within the 8 s. GPU 93 °C, CPU 87 °C before and after: the owner's session's heat |
| 10-30 19:25 | a8d6a6a, staged | `lib-vr-1`: `frame.sh vr … library`, unworn, eyes pinned, the panel on Chromium's headless platform (no `DISPLAY`); a scripted click on Symphony IX 1.0.2's PLAY at 20 s | The library at 72 fps, GPU 0.5 ms (the eyes cleared under the panel); the click went through the panel's `play` to the table (75), then 40 s of it at 72 fps, GPU ~8 ms. The library's session got no focus (as most unworn runs, tables too). No panel captures: its page was still after loading (hence 49c657d). The Frame's controller profile took 23 of 28 bindings, the aim poses among them. GPU 64 → 70 °C |
| 10-30 19:30 | a8d6a6a + 49c657d's capture | `lib-vr-2`: the same on X11 (`--env DISPLAY=:1`, as Steam starts the game) | Focused at once; the panel's first picture 1 s into VR; the captures at 5 s and 15 s show the Library of 19 tables (no "Back to": nothing played in the run's data); the click played the table. The library's GPU 0.5 ms, with two spikes to 16 and 19 ms. GPU 60 → 67 °C |
| 10-30 19:33 | 49c657d, installed (`frame.sh install`) | `lib-steam-1`: the game started through the Steam client (`frame.sh steam`), stopped after 30 s | Library mode, the panel on X11 (`DISPLAY=:1`), its GPU process on Mesa's GLX and zink over Turnip (`libvulkan_freedreno` loaded); first picture 1 s in. Focus at ~20 s, the controllers' profile with it, and at ~25 s a `play` of STAR FOX from the panel with no click scripted: someone in the headset pointed and pulled the trigger. The player started again with STAR FOX and the check's 30 s stop ended it as it loaded. GPU 61 → 59 °C |
| 10-30 19:34–19:36 | 49c657d, installed | The owner, worn, through Steam (`~/.local/share/VisualPinballInfinite/vpi-player*.log`) | The library, Beethowen played from it; left through SteamVR's menu; started again: the library came back "Back to beethowen". Reported: pointing at the library's left column garbles it; leaving the table brings up a menu stuck to the head, close; no table options found. The head-locked one: the table's pause menu, drawn flat over each eye (the right controller's Menu is the player's "quit", which opens it at its Quit row), and its Table options are there; nothing showed the owner how to open it (the left View button) |
| 10-30 19:45 | 49c657d | `x11-windows`: `frame.sh panel` on the session's X server (`--env DISPLAY=:1`), its windows listed meanwhile (`xwininfo`) | The panel's only X window: Chromium's clipboard window, 10×10, unmapped, off-screen. Nothing of the panel's for gamescope to show. GPU 64 → 61 °C |
| 10-30 19:48, 19:50 | bb5cf93 + check-mode moves | `hover-x11`, `hover-dense`: the panel on X11, its pointer moved down the left column (`--moves`), pictures after each move and every 40 ms through the hover | Chromium's pictures clean: each move changes the pointer's ring alone, nothing elsewhere. So the garbling is after Chromium: the upload into the quad's swapchain or SteamVR's overlay of it (SteamVR shows a quad layer as an overlay, `openxr_quadlayer_0`); each quad picture is now handed over only once the GPU has written it. GPU 61 → 60 °C |
| 10-30 21:53 | 934541a, staged | `lib-vr-3`: `frame.sh vr … library` on X11 (`--env DISPLAY=:1`), unworn, a scripted click on Symphony IX 1.0.2's PLAY at 20 s | The panel's swapchain 3 × 1600×1000, placed 1.5 m before the head (0, 1.20, −1.25; the session never focused, so not placed again); its first picture 2.2 s into VR, the captures the Library of 20 tables. The click played the table: 40 s at 72 fps, GPU 8.5 ms. The library's frame timings were written over by the table's (one `--vr-timings` file for both starts). GPU 41 → 50 °C |
| 10-30 21:55 | 934541a, staged | `menu-vr-1`: Symphony IX 1.0.2, unworn, the pause menu by scripted keys: opened at 8 s (`pause`), down at 11 and 13 s, closed at 17 s, opened at 22 s by `exit` (the right controller's Menu), closed at 26 s | The pause menu's swapchain 3 × 1280×1597 as it first opened; its pictures at 8.9, 11.9, 13.8 and 22.8 s, 0.67 × 0.83 m in the room. 72 fps throughout; a frame as it opens or changes up to 22 ms (CPU) and 24 ms (GPU). Its hints named the keyboard (hence dffa4e2). GPU 44 → 54 °C |
| 10-30 21:58 | 934541a, staged | `ts-1`: `frame.sh panel`, headless, a click on Symphony IX 1.0.2's title at 8 s | Its page, the headset's buttons alone (PLAY, favourite, Table settings, achievements); but the test run's row showed, its `display: flex` outdoing `hidden` (hence e48567d). GPU 41 °C |
| 10-30 22:01 | b0590a0, staged | `ts-2`: the same, then Table settings at 10 s, pictures to 24 s | The settings window with the options Symphony IX's script offers (Color Saturation … VR Room), asked of it by `vpi-headless` on the Frame within 6 s. It said they'd be kept beside the table in the Tables folder, which the owner's brief keeps read only: no Save was pressed, nothing written there, and table settings now go to VPI's own folder (bb5f920). It also offered VPS's picker, the screens and the physics file, which the panel doesn't serve (ee00162). GPU 39 → 40 °C |
| 10-30 22:12 | ee00162, staged | `save-1`: a copy of Symphony IX 1.0.2 in the test folder (`~/vpi-test/tables`), unworn; by scripted keys the pause menu, its Table options, the first option a step on, the menu closed | Saved into the run's `table-settings` (VPI's own folder): the copy's `[PlayerVR]` lines and `Color_Saturation = 2`; the table's own file, the copy's and the owner's, the same as before (md5), nothing newer in either folder. The options page's hint names the trigger and Back. Mech_Volume and Music_Volume were saved too, at their defaults (hence fb58fb0). GPU 38 → 45 °C |
| 10-30 22:16 | fb58fb0, staged | `save-2`: the same | Only `Color_Saturation = 2` beside the copied `[PlayerVR]` lines; the tables' own files as they were. 72 fps. GPU 38 → 46 °C |
| 10-30 22:17 | fb58fb0, staged | `ts-3`: the settings window of the library's Symphony IX 1.0.2 (`frame.sh panel`), `VPI_TABLE_SETTINGS` on `save-2`'s folder, read only | Read from VPI's copy: Color Saturation "Desaturated 10%"; the window says they're kept in VPI's own folder, which VPX doesn't see; no VPS picker. GPU 42 → 41 °C |
| 10-30 22:17 | fb58fb0, staged | `lib-vr-4`: the library on X11, unworn, clicks down its left column every 2 s from 8 to 20 s | 72 fps throughout; a frame with a new panel picture (copied in and waited for before SteamVR has it) 3.4–4.5 ms of CPU, the rest 0.07 ms; GPU 0.63 ms. GPU 41 → 42 °C |
| 10-30 22:19 | 65af2ba, installed (`frame.sh install`) | `lib-steam-2`: the game started through the Steam client, stopped after 30 s | The library: the panel placed, its first picture 0.6 s into VR. The stop (SIGTERM) didn't end it: the library's loop never read SDL's quit event. Ended by hand (the player killed, its panel ending with it, Steam's launch with them); fixed in 758f107. GPU 39 → 40 °C |
| 10-30 22:22 | 758f107, staged | `term-1`: the library on X11, SIGTERM at 20 s | "asked to end" at once; the session closed and the panel ended with it, nothing left. GPU 39 → 40 °C |
| 10-30 22:24 | 758f107, installed | `lib-steam-3`: through the Steam client, stopped after 30 s | The library, its first picture 0.55 s into VR; ended on the stop with its panel and Steam's launch. These two starts rotated the owner's 19:34 logs out of the game's data folder (run.sh keeps one before its own); `frame.sh steam` keeps them aside first now. GPU 39 → 41 °C |
| 10-31 15:46–16:16 | 758f107, installed | The owner, worn, through Steam: the library (recorded 15:50:19–15:50:40 by the Frame's screen recorder), then Symphony IX v9 1.1 played from it; logs `vpi-player*.log` and SteamVR's `xrclient_vpi-player.txt` | Blocks of black and white over the panel under the pointer (the owner's words). The recording (SteamVR's view, 1920×1080 at 72 fps) shows the panel whole through 12 s of pointing down the left column, its hover highlights and clicks (Top rated, The 1990s, Originals): no frame with a flash of white. SteamVR had the session focused 15:49:55–15:50:13, 15:50:24–15:50:39 and 16:01:59–16:02:26; it makes the panel's swapchain images itself ("remote", B8G8R8A8_SRGB, 3). Focused and pointing, the player's CPU per frame 11.2 ms p50 against 0.1 idle, its wait for the headset 2.5 ms against 13.7: each frame with a new picture waited about 11 ms for its own copy (`sweep-0` below). Table options changed in the pause menu (Music_Volume, Staged_Flippers, Balls_Per_Game) kept in VPI's `table-settings`, the table's own `.ini` as it was (10-30) |
| 10-31 16:32 | 039564c's code (then 5da5de2), staged | `sweep-0`: the library on X11 (`--env DISPLAY=:1`), unworn, the pointer swept down the left column and back every 1.5 s from 8 to 48 s (`VPI_PANEL_SWEEP=8000-48000:120,150:120,520:1500`), every picture written (`VPI_PANEL_IMAGE_REST=0`, as the owner had it) | The owner's stall unworn: 52 pictures a second, a frame with one 10.5–11 ms of CPU (median 10.5 against 0.2 at rest, 1,763 of 2,448 frames over 8 ms), its wait for the headset 3.0–3.5 ms. SteamVR gave each image within 0.2 ms: the time went on waiting for our own copy, queued behind Chromium's GPU work (3.4–4.5 ms with one paint a click in `lib-vr-4`). No picture with solid blocks. CPU 54 → 61 °C |
| 10-31 16:33 | the same | `sweep-6`: the same, each image resting 6 frames | 36 pictures a second, each frame with one still 11 ms of CPU (1,205 frames over 8 ms): resting the images doesn't shorten the wait |
| 10-31 16:42–16:43 | 758f107, installed | The owner through Steam: the library, then Call of Duty Zombie Edition 2025 VR (Iceman 2025) | The library at 11.0 ms of CPU a frame (the copies' wait). The table loaded (1.0 M triangles), but its ROM, wolfman, isn't on the Frame (`Tables/pinmame/roms` is empty): PinMAME couldn't run it. The pause menu opened; quit after a minute |
| 10-31 16:52 | 039564c-dirty (b10b8b8's change), staged | `flip-1`: as `sweep-0`, the panel on two swapchains, swapped once its copy is done | 36 pictures a second, each shown the frame after it was sent; CPU a frame 0.6 ms median (0.9–1.1 at p90) against 10.5; 72 fps, the wait for the headset 13.2 ms; no picture with solid blocks. CPU 48 °C |
| 10-31 16:54 | the same | `menu-flip-1`: Symphony IX 1.0.2, the pause menu by scripted keys as in `menu-vr-1` | The menu's two swapchains of 1280×1597; its pictures at 8.9, 11.9, 13.9 and 22.9 s; a frame as it opens or changes 14–25 ms CPU, 16–28 ms GPU, as before; 72 fps otherwise |
| 10-31 16:56 | b10b8b8, installed (`frame.sh install`) | `lib-steam-4`: through the Steam client, stopped after 30 s | The library on its two swapchains, its first picture 0.5 s into VR; ended on the stop with its panel and Steam's launch; the owner's older log kept in `before/` |
