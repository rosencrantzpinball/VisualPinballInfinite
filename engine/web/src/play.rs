//! A table's play in the browser: what the player's `play` and its window's loop do
//! (`engine/player/src/main.rs`), for the one window there is, the page's canvas, in the desktop
//! view. The game runs on the page's thread, not one of its own: each frame the page asks for
//! (`frame`), the input first, then the physics up to the moment, VPX's frame preparation, and
//! the picture of what the game shows now.
//!
//! Left out, the browser having no use for them: VR, the test mode's pictures and videos, the
//! backglass's, score display's and topper's own windows, PinMAME's machines, PinUP's labels' text,
//! cabinet devices (DOF, Open Pinball Device boards).

use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sdl3_sys::everything::*;

use player::audio::Audio;
use player::camera::{self, Camera, TableExtent, View, ViewSetup};
use player::dmd_overlay::Overlay;
use player::game::{Game, GameSetup, Snapshot};
use player::input::{self, InputEvent, InputSink};
use player::math::{Mat4, Vec3};
use player::render::{FrameInput, FrameResult, LookMode, PostAa, Presentation, Renderer, Sharpen, Text};
use player::scene::Scene;
use player::waker::Waker;

/// How long a frame may make the physics catch up at most: a frame the browser held back longer
/// (a page busy elsewhere, not hidden) goes on from there rather than stalling the next ones.
const CATCH_UP_US: u64 = 100_000;

/// The table's play, from its start to its end.
pub struct Play {
    renderer: Renderer,
    scene: Scene,
    game: Game,
    inputs: Receiver<InputEvent>,
    to_game: Sender<InputEvent>,
    sink: Arc<InputSink>,
    window: *mut SDL_Window,
    _audio: Option<(Option<Audio>, Option<Audio>)>,
    view_setup: ViewSetup,
    extent: TableExtent,
    legacy_bounds: Vec<Vec3>,
    mirror_planes: Vec<[f32; 4]>,
    camera: Option<(Camera, (u32, u32))>,
    transforms: Vec<Mat4>,
    lights: Vec<player::scene::LightNow>,
    last: Snapshot,
    applied_grade: Option<String>,
    applied_bloom: Option<f32>,
    grade_images: std::collections::HashMap<String, usize>,
    menu_shown: Option<(player::pause_menu::View, (u32, u32))>,
    menu_layout: Option<player::pause_menu::Layout>,
    menu_turn: (u32, (u32, u32)),
    menu_mouse: MenuMouse,
    esc_opens_menu: bool,
    table_extras: Vec<player::pause_menu::Extra>,
    focus_lost: bool,
    readout: bool,
    max_fps: f64,
    nudge_strength: f32,
    volume_keys: player::volume::Keys,
    steps: Steps,
    fps: Fps,
    /// When the last frame went to the screen, and how long making it took, for the balls' last-moment move.
    last_present_ns: u64,
    last_frame_ns: u64,
    frame_ns: u64,
    ended: bool,
}

/// The physics' steps counted each second, for the readout.
#[derive(Default)]
struct Steps {
    this_second: u32,
    per_second: u32,
    worst_this_second: f32,
    worst_us: f32,
    second_start: u64,
    physics_at_ns: u64,
}

struct Fps {
    frames: u32,
    fps: f64,
    since: Instant,
}

fn now_ns() -> u64 {
    // SAFETY: SDL's clock (ours)
    unsafe { SDL_GetTicksNS() }
}

impl Play {
    /// The table at `table`, with the settings in the data folder (`VPI_DATA`) and the table's own
    /// over them, as the player starts it.
    pub fn start(table: &Path, refresh_hz: f32) -> Result<Play, String> {
        let t0 = Instant::now();
        // the settings: the main file, then the table's own over it (kept in VPI's folder,
        // `VPI_TABLE_SETTINGS`: the table's own folder is only read)
        // the library: the table's folder, and VPX's shared scripts the page carries (core.vbs and
        // the rest, which scripts load with GetTextFile)
        player::script::api::set_library(vpi_settings::LibraryFolders {
            tables: table.parent().map_or_else(|| "/table".into(), Path::to_path_buf),
            scripts: Some("/scripts".into()),
            pinmame: None,
            pinup: None,
            pinballx: None,
            pinbally: None,
            per_game: false,
        });
        let main = player::settings::Settings::load();
        let settings = main.clone().with_table(table).with_pov(table);
        if let Some(path) = &settings.path {
            println!("settings: {}", path.display());
        }
        let view = settings.view().filter(|v| *v == View::Desktop || *v == View::SingleScreen).unwrap_or(View::Desktop);
        let options = Options::from_settings(&settings);
        player::scoreview::set_profiles(player::vpx_settings::dmd_profiles(&settings));
        player::segments::set_profiles(player::vpx_settings::segment_profiles(&settings));

        // the table and our model of it, as `play` makes them
        player::shapes::rubber::set_detail_level(player::vpx_settings::detail_level(&settings).unwrap_or(10));
        let mut vpx = vpin::vpx::read(table).map_err(|e| format!("could not read {}: {e}", table.display()))?;
        player::pictures::resolve_links(&mut vpx);
        let replaced = player::pictures::replace_from_folder(&mut vpx, table);
        if !replaced.is_empty() {
            println!("pictures: {} played from the table's pictures folder in place of its own", replaced.join(", "));
        }
        if !player::achievements::hardcore_play()
            && let Some(file) = table_physics::vpp::find(table)
            && let Ok(text) = std::fs::read_to_string(&file)
        {
            match table_physics::vpp::apply(&mut vpx, &text) {
                Ok(_) => println!("physics: as {} sets them", file.display()),
                Err(e) => println!("physics: {} left aside: {e}", file.display()),
            }
        }
        let t_read = t0.elapsed();
        let (doc, pictures) = player::model::load(&vpx)?;
        let t_meshes = t0.elapsed();
        let mut scene = Scene::build(&vpx, &doc, pictures);
        scene.use_own_plungers(&vpx);
        scene.use_own_animations(&vpx);
        drop(doc);
        let t_scene = t0.elapsed();
        let g = &vpx.gamedata;
        let backdrop_image = match view {
            View::SingleScreen => g.backglass_image_full_single_screen.clone().unwrap_or_else(|| g.backglass_image_full_desktop.clone()),
            _ => g.backglass_image_full_desktop.clone(),
        };
        let light_level = player::vpx_settings::light_level(&settings, vpx.gamedata.global_emission_scale);
        scene.set_player_look(&vpx, player::vpx_settings::exposure(&settings), player::vpx_settings::tone_mapper(&settings), light_level);
        scene.add_backdrop(&vpx, &backdrop_image, true);
        let physics = table_physics::build(&vpx, &player::vpx_settings::physics(&settings));
        let legacy_bounds: Vec<Vec3> = table_physics::view_bounds::legacy_view_bounds(&vpx).iter().map(|v| Vec3::new(v.x, v.y, v.z)).collect();
        let g = &vpx.gamedata;
        let extent = TableExtent { left: g.left, top: g.top, right: g.right, bottom: g.bottom };
        let mut view_setup = ViewSetup::from_table(&vpx, view);
        println!(
            "{}: loaded in {:.0} ms (the file {:.0}, its meshes {:.0}, pictures and parts {:.0}) — {} draws, {} images ({:.0} MB decoded), {} moving parts",
            table.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            t0.elapsed().as_secs_f64() * 1e3,
            t_read.as_secs_f64() * 1e3,
            (t_meshes - t_read).as_secs_f64() * 1e3,
            (t_scene - t_meshes).as_secs_f64() * 1e3,
            scene.draws.len(),
            scene.images.len(),
            scene.images.iter().map(player::scene::Image::bytes).sum::<usize>() as f64 / 1e6,
            scene.groups.len()
        );

        let mut sounds = player::audio::Sounds::load(&vpx);
        // the sounds decoded, their files' bytes go: in the browser the program's memory is all it
        // has, and nothing reads them again
        let freed: usize = vpx.sounds.iter_mut().map(|s| std::mem::take(&mut s.data).len()).sum();
        println!("sounds: {} decoded, their files' {:.0} MB let go", vpx.sounds.len(), freed as f64 / 1e6);
        (sounds.playfield_volume, sounds.backglass_volume) = settings.volumes();
        sounds.levels = player::audio::Levels::from_settings(&settings);
        player::achievements::set_sound(settings.int_or("VPI", "AchievementSound", 1, 0, 1) != 0);
        let menu = {
            let i = &vpx.info;
            let given = |v: &Option<String>| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
            let title = given(&i.table_name).unwrap_or_else(|| table.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
            let byline = match (given(&i.author_name), given(&i.table_version)) {
                (Some(a), Some(v)) => format!("by {a}  \u{b7}  version {v}"),
                (Some(a), None) => format!("by {a}"),
                (None, Some(v)) => format!("version {v}"),
                (None, None) => String::new(),
            };
            player::pause_menu::Setup {
                title,
                byline,
                rules: given(&i.table_rules),
                settings_file: vpi_settings::settings_path(),
                table_file: Some(table.with_extension("ini")),
                picture_only: false,
                view: None,
                tables: Vec::new(),
                playing: Some(table.to_path_buf()),
                library: false,
                headset: false,
            }
        };

        // SAFETY: our own SDL, on the page's thread
        unsafe {
            SDL_Init(SDL_INIT_VIDEO | SDL_INIT_AUDIO | SDL_INIT_GAMEPAD);
        }
        // the cabinet's screen: the browser's, its size unknown to the page (VPX's default)
        view_setup.apply_settings(&settings, view, &extent, None);
        let own_view = {
            let mut own = ViewSetup::from_table(&vpx, view);
            own.apply_settings(&main, view, &extent, None);
            own
        };
        let audio = {
            let layout = player::audio::Layout::from_setting(settings.enum_or("Player", "Sound3D", 0, 6));
            let backglass = Audio::open(&sounds, None, None);
            let playfield = Audio::open(&sounds, None, Some(layout));
            match (playfield, backglass) {
                (Ok((pf, ptx)), Ok((bg, btx))) => {
                    sounds.connect(ptx, btx);
                    Some((Some(pf), Some(bg)))
                }
                (Ok((audio, tx)), Err(e)) | (Err(e), Ok((audio, tx))) => {
                    eprintln!("vpi-web: one sound output only: {e}");
                    sounds.connect(tx.clone(), tx);
                    Some((Some(audio), None))
                }
                (Err(e), Err(_)) => {
                    eprintln!("vpi-web: no sound: {e}");
                    None
                }
            }
        };
        let colour = |section: &str| settings.section(section);
        let mut setup = GameSetup {
            physics,
            vpx: Arc::new(vpx),
            table_path: table.to_path_buf(),
            render_parts: scene.groups.iter().map(|g| g.part).collect(),
            script: true,
            sounds,
            record: None,
            view,
            vr: false,
            stop_at_ms: None,
            record_from_ms: None,
            backglass_window: false,
            show_grill: false,
            pup_windows: [None; 3],
            pup: player::pup::PupSettings::from_settings(&settings),
            machine_settings: Arc::new(player::machine_settings::MachineSettings::load(settings.path.as_deref())),
            pinmame: player::machine_settings::PinmameSettings::from_settings(&settings),
            saved_values: player::saved_values::file_for(settings.path.as_deref()),
            cabinet_input: true,
            plumb: (
                settings.get("Player", "SimulatedPlumb").and_then(player::settings::vpx_bool).unwrap_or(true),
                settings.float_or("Player", "PlumbDamping", 1.0, 0.0, 2.0, 0.0),
                settings.float_or("Player", "PlumbThresholdAngle", 1.0, 0.15, 4.0, 0.0),
            ),
            sensors: player::sensors::Sensors::default(),
            tilt_counts_nudges: settings.get("VPI", "TiltCountsNudges").and_then(player::settings::vpx_bool).unwrap_or(false),
            light_level: Some(light_level),
            plunger_retract: player::vpx_settings::plunger_retract(&settings),
            waker: None,
            table_options: settings.section("TableOption"),
            controller_settings: settings.section("Controller"),
            altsound_settings: settings.section("Plugin.AltSound"),
            serum_settings: colour("Plugin.Serum"),
            vni_settings: colour("Plugin.VNI"),
            alphadmd_settings: colour("Plugin.AlphaDMD"),
            upscale_settings: colour("Plugin.UpscaleDMD"),
            upscale_at_once: false,
            dof_settings: vec![("Enable".into(), "0".into())],
            dof_folder: None,
            record_toys: false,
            dmdutil_settings: vec![("Enable".into(), "0".into())],
            dmd_panel_check: None,
            dmd_device: None,
            menu: Some(player::pause_menu::Setup { view: Some(player::pause_menu::Viewing { which: view, now: view_setup, own: own_view }), ..menu }),
        };

        // the window, the page's canvas, and the renderer on its WebGPU
        // SAFETY: our own SDL
        let window = unsafe { SDL_CreateWindow(c"Visual Pinball Infinite".as_ptr(), 1600, 900, SDL_WINDOW_HIGH_PIXEL_DENSITY | SDL_WINDOW_RESIZABLE) };
        // SAFETY: the window is the page's canvas, there for the whole play
        let mut renderer = unsafe { Renderer::new(window, &scene, &setup.vpx.images, Presentation::Queued, options.samples, LookMode::Vpx, None)? };
        renderer.draw_still_parts_once(true);
        renderer.set_post(options.post_aa, options.sharpen);
        renderer.set_supersampling(options.aa_factor);
        renderer.set_ambient_occlusion(options.ao);
        renderer.set_motion_blur(options.motion_blur);
        renderer.set_bloom_off(options.bloom_off);
        renderer.set_anisotropic(player::vpx_settings::anisotropic(&settings))?;
        // a mirror needs the clip distances WebGPU may not have: without them, none
        let reflections: u32 = player::vpx_settings::reflection_mode(&settings).into();
        renderer.set_max_reflection_mode(if crate::sdl::gpu::has_feature(0) { reflections } else { 0 });
        renderer.set_ball_settings(&player::vpx_settings::ball_override(&settings), player::vpx_settings::ball_lighting_off(&settings), player::vpx_settings::ball_anti_stretch(&settings));
        renderer.set_reveal_windows(true);
        renderer.set_dmd_overlays(Some(Overlay::Auto), Overlay::Off);
        renderer.set_pack_first(true, true);
        println!(
            "still parts: {}",
            match (renderer.still_parts_once(), renderer.multisampled()) {
                (true, false) => "drawn once (VPX's static prepass)",
                (true, true) => "drawn once (VPX's static prepass), what moves multisampled",
                (false, true) => "drawn every frame, multisampled",
                (false, false) => "drawn every frame",
            }
        );

        // the input: the settings' mappings, else VPX's defaults; the keyboard, the mouse and the
        // browser's gamepads through SDL's event watch
        let mut input_map = player::input_map::InputMap::new(&settings);
        let table_extras = player::pause_menu::table_extras(&setup.vpx.gamedata.code.string);
        setup.sensors = player::sensors::Sensors::from_settings(&settings, &mut input_map);
        input_map.sensors_read(&setup.sensors.devices());
        let (to_game, inputs) = mpsc::channel();
        let sink = Arc::new(InputSink::new(to_game.clone(), Arc::new(Waker::new()), input_map));
        if !input::install(sink.clone()) {
            return Err("watch input".into());
        }
        player::rumble::set_mode(settings.enum_or("Player", "RumbleMode", 3, 4));
        let grade_images: std::collections::HashMap<String, usize> = setup
            .vpx
            .images
            .iter()
            .enumerate()
            .filter(|(_, im)| im.width == 256 && im.height == 16)
            .filter_map(|(i, im)| Some((im.name.to_lowercase(), scene.table_images.get(i).copied().flatten()?)))
            .collect();
        scene.release_pictures(&grade_images.values().copied().collect::<Vec<_>>());
        let esc_opens_menu = settings.get("VPI", "PauseMenuBeforeQuit").and_then(player::settings::vpx_bool).unwrap_or(true);
        // SAFETY: on SDL's thread (ours)
        player::pause_menu::set_controls(player::pause_menu::control_lines(&table_extras, &|a| unsafe { sink.names(a) }, esc_opens_menu));

        let mirror_planes = scene.mirrors.iter().map(|m| m.plane).collect();
        let transforms = vec![Mat4::IDENTITY; scene.groups.len()];
        let lights = scene.light_parts.iter().map(player::scene::LightNow::at_start).collect();
        let mut game = Game::new(setup);
        let start_us = now_ns() / 1000;
        game.start(start_us);
        let refresh = if refresh_hz > 0.0 { refresh_hz } else { 60.0 };
        println!("{}: playing ({:.0} ms to start)", renderer.present_mode, t0.elapsed().as_secs_f64() * 1e3);
        Ok(Play {
            renderer,
            scene,
            game,
            inputs,
            to_game,
            sink,
            window,
            _audio: audio,
            view_setup,
            extent,
            legacy_bounds,
            mirror_planes,
            camera: None,
            transforms,
            lights,
            last: Snapshot::default(),
            applied_grade: None,
            applied_bloom: None,
            grade_images,
            menu_shown: None,
            menu_layout: None,
            menu_turn: (0, (1, 1)),
            menu_mouse: MenuMouse::default(),
            esc_opens_menu,
            table_extras,
            focus_lost: false,
            readout: options.readout,
            max_fps: options.max_fps,
            nudge_strength: player::vpx_settings::visual_nudge_strength(&settings),
            volume_keys: player::volume::Keys::default(),
            steps: Steps { second_start: start_us, physics_at_ns: now_ns(), ..Steps::default() },
            fps: Fps { frames: 0, fps: 0.0, since: Instant::now() },
            last_present_ns: 0,
            last_frame_ns: 0,
            frame_ns: (1e9 / f64::from(refresh)) as u64,
            ended: false,
        })
    }

    /// The frame-rate limit the settings set (0: none, every refresh of the browser's).
    pub fn max_fps(&self) -> f64 {
        self.max_fps
    }

    /// The screen's refresh as the page measured it, for the balls' last-moment move.
    pub fn set_refresh(&mut self, hz: f32) {
        if hz > 0.0 {
            self.frame_ns = (1e9 / f64::from(hz)) as u64;
        }
    }

    /// One frame: the events since the last, the game up to now, and its picture. False once
    /// play has ended (the script's `QuitPlayer`, or Quit on the pause menu).
    pub fn frame(&mut self) -> Result<bool, String> {
        if self.ended {
            return Ok(false);
        }
        if !self.events() {
            return Ok(self.end());
        }
        // SAFETY: SDL's gamepads, on its thread (ours)
        unsafe { player::rumble::play_pending() };

        // the game: input first, as VPX does, then the physics up to the present
        let game = &mut self.game;
        while let Ok(ev) = self.inputs.try_recv() {
            game.input(ev);
        }
        game.poll_volume_keys(&mut self.volume_keys);
        game.menu_tick();
        game.play_chime();
        game.resume_if_due(now_ns() / 1000);
        let started = Instant::now();
        let stepping_from_ns = now_ns();
        let now_us = now_ns() / 1000;
        let target_us = now_us.min(game.next_step_time_usec() + CATCH_UP_US);
        let steps = if game.paused() { 0 } else { game.advance(target_us) };
        let s = &mut self.steps;
        if steps > 0 {
            s.worst_this_second = s.worst_this_second.max(started.elapsed().as_secs_f32() * 1e6 / steps as f32);
            s.physics_at_ns = stepping_from_ns + started.elapsed().as_nanos() as u64 * u64::from(steps - 1) / u64::from(steps);
        }
        s.this_second += steps;
        if now_us - s.second_start >= 1_000_000 {
            (s.per_second, s.worst_us) = (s.this_second, s.worst_this_second);
            (s.this_second, s.worst_this_second, s.second_start) = (0, 0.0, now_us);
        }
        if game.quit_requested() {
            return Ok(self.end());
        }
        game.prepare_frame();
        self.last = game.snapshot(s.per_second, s.worst_us, s.physics_at_ns);
        self.draw()?;
        Ok(true)
    }

    /// The events since the last frame: Esc's pause menu, the menu's mouse, gamepads coming and
    /// going, the canvas losing the keyboard; false when play should end.
    fn events(&mut self) -> bool {
        use player::pause_menu::code;
        // SAFETY: our own events
        unsafe {
            let mut event: SDL_Event = std::mem::zeroed();
            while SDL_PollEvent(&mut event) {
                let kind = event.r#type;
                if kind == SDL_EVENT_WINDOW_FOCUS_LOST.0 || kind == SDL_EVENT_WINDOW_FOCUS_GAINED.0 {
                    let lost = kind == SDL_EVENT_WINDOW_FOCUS_LOST.0;
                    if lost != self.focus_lost {
                        self.focus_lost = lost;
                        println!("{}", if lost { "the page lost the keyboard: play paused until it's back" } else { "the page has the keyboard again: play goes on" });
                        let _ = self.to_game.send(InputEvent { id: 0, time_ns: now_ns(), dik: if lost { code::FOCUS_LOST } else { code::FOCUS_BACK }, down: true });
                    }
                }
                // Esc opens the pause menu (its Quit ends play); while it shows Esc steps back
                if kind == SDL_EVENT_KEY_DOWN.0 && !event.key.repeat && event.key.scancode == SDL_SCANCODE_ESCAPE && !player::pause_menu::shown_lately() {
                    if self.esc_opens_menu {
                        let _ = self.to_game.send(InputEvent { id: 0, time_ns: now_ns(), dik: code::EXIT, down: true });
                    } else {
                        return false;
                    }
                }
                if let Some(layout) = &self.menu_layout {
                    self.menu_mouse.event(&event, layout, self.window, self.menu_turn);
                } else {
                    self.menu_mouse.hover = None;
                }
                if kind == SDL_EVENT_JOYSTICK_ADDED.0 {
                    self.sink.joystick_added(event.jdevice.which);
                    player::pause_menu::set_controls(player::pause_menu::control_lines(&self.table_extras, &|a| self.sink.names(a), self.esc_opens_menu));
                } else if kind == SDL_EVENT_JOYSTICK_REMOVED.0 {
                    self.sink.joystick_removed(event.jdevice.which);
                    player::pause_menu::set_controls(player::pause_menu::control_lines(&self.table_extras, &|a| self.sink.names(a), self.esc_opens_menu));
                }
            }
        }
        true
    }

    /// The game's picture now, as the window's loop draws it.
    fn draw(&mut self) -> Result<(), String> {
        let frame_start = now_ns();
        let s = &self.last;
        if s.color_grade != self.applied_grade {
            self.applied_grade.clone_from(&s.color_grade);
            if let Some(name) = &self.applied_grade {
                self.renderer.set_color_grade(self.grade_images.get(&name.to_lowercase()).and_then(|&i| self.scene.images.get(i)))?;
            }
        }
        if s.bloom_strength != self.applied_bloom {
            self.applied_bloom = s.bloom_strength;
            if let Some(strength) = self.applied_bloom {
                self.renderer.set_bloom_strength(strength);
            }
        }
        for (i, group) in self.scene.groups.iter().enumerate() {
            self.transforms[i] = s.parts.get(i).copied().flatten().map_or(Mat4::IDENTITY, |state| group.motion.transform(state));
        }
        // a view set on the pause menu's view page, kept for the rest of the play
        if let Some(v) = s.menu.as_deref().and_then(|m| m.camera)
            && v != self.view_setup
        {
            self.view_setup = v;
            self.camera = None;
        }
        self.renderer.set_view_changing(s.menu.as_deref().is_some_and(|m| m.view_page));
        let size = self.renderer.window_size();
        if self.camera.as_ref().is_none_or(|(_, at)| *at != size) {
            let c = camera::compute(&self.view_setup, &self.extent, size.0 as f32 / size.1 as f32, &self.legacy_bounds, &self.scene.depth_bounds, &self.mirror_planes);
            self.camera = Some((c, size));
        }
        let Some((c, _)) = &self.camera else { return Ok(()) };
        let c = *c;

        let text_turn = (self.view_setup.rotation(size.0, size.1) / 90.0).round() as u32 % 4;
        let ui = if text_turn % 2 == 1 { (size.1, size.0) } else { size };
        let mut text = Vec::new();
        if self.readout {
            text.push(Text { x: 12.0, y: 12.0, scale: 2.0, color: [1.0, 0.85, 0.3, 1.0], text: format!("Visual Pinball Infinite  {:.0} fps  WebGPU", self.fps.fps) });
            text.push(Text {
                x: 12.0,
                y: 34.0,
                scale: 2.0,
                color: [0.7, 0.85, 1.0, 1.0],
                text: format!("Physics: {} steps/s, slowest step {:.0} us, {} balls", s.steps_per_second, s.worst_step_us, s.balls.len()),
            });
        }
        let announcement = player::achievements::notice();
        if let Some(notice) = self
            .focus_lost
            .then(|| "Play paused: click the table to go on".to_string())
            .or_else(player::volume::notice)
            .or_else(|| announcement.map(|a| a.replace(" · ", " - ")))
        {
            let scale = (ui.0 as f32 * 0.92 / (notice.chars().count().max(1) as f32 * 8.0)).clamp(1.0, 3.0);
            let width = notice.chars().count() as f32 * 8.0 * scale;
            text.push(Text { x: ((ui.0 as f32 - width) / 2.0).max(0.0), y: ui.1 as f32 * 0.8, scale, color: [1.0, 1.0, 1.0, 1.0], text: notice });
        }
        // the pause menu's picture, made again when what it shows or the canvas's size changes
        let menu_same = match (&self.menu_shown, &s.menu) {
            (Some((was, at)), Some(now)) => **now == *was && *at == ui,
            (None, None) => true,
            _ => false,
        };
        if !menu_same {
            let made = s.menu.as_deref().and_then(|v| player::pause_menu::picture(v, ui));
            if let Err(e) = self.renderer.set_menu(made.as_ref().map(|(p, _)| p)) {
                eprintln!("vpi-web: the pause menu: {e}");
            }
            self.menu_layout = made.map(|(_, layout)| layout);
            self.menu_shown = s.menu.as_deref().map(|v| (v.clone(), ui));
        }
        self.menu_turn = (text_turn, size);
        if !s.part_render.is_empty() {
            for (now, l) in self.lights.iter_mut().zip(&self.scene.light_parts) {
                if let Some(r) = s.part_render.get(l.part as usize) {
                    let live = r.light_live.unwrap_or_else(|| player::scene::LightLive::of(l));
                    *now = player::scene::LightNow::from_game(r.light, r.light_color, r.light_tint, live);
                }
            }
        }
        let shake = nudge_shake(s.nudge_offset, self.view_setup.rotation(size.0, size.1), self.nudge_strength);
        let shake_matrix = Mat4::translate(shake[0], shake[1], 0.0);
        // the balls' last-moment move, as VPX means it: the way they go until the frame shows
        let now = now_ns();
        let delay_s = now.saturating_sub(s.physics_at_ns) as f32 * 1e-9 + predicted_display_delay(now, self.last_present_ns, self.frame_ns, self.last_frame_ns) + 0.001;
        let k = delay_s / 0.01;
        let balls: Vec<player::render::BallDraw> = s
            .balls
            .iter()
            .map(|b| {
                let (p, v) = (b.position, b.velocity);
                player::render::BallDraw { position: Vec3::new(p.x + v.x * k, p.y + v.y * k, p.z + v.z * k), ..*b }
            })
            .collect();
        let input = FrameInput {
            view_proj: c.view * c.projection * shake_matrix,
            view_rotation: self.view_setup.rotation(size.0, size.1),
            unshaken: c.view * c.projection,
            view: c.view,
            projection: c.projection,
            shake: shake_matrix,
            eye: c.eye,
            group_transforms: &self.transforms,
            balls: &balls,
            lights: &self.lights,
            part_render: &s.part_render,
            reels: &s.reels,
            text_boxes: &s.text_boxes,
            screen_offset: shake,
            text: &text,
            text_turn,
            dmd: s.dmd.as_deref(),
            segments: s.segments.as_deref(),
            display_hidden: s.display_hidden,
            backglass: s.backglass.as_ref(),
            pup: s.pup.as_ref(),
        };
        if let FrameResult::Submitted { .. } = self.renderer.frame(&input)? {
            let submitted = now_ns();
            (self.last_present_ns, self.last_frame_ns) = (submitted, submitted.saturating_sub(frame_start));
            self.fps.frames += 1;
        }
        let elapsed = self.fps.since.elapsed();
        if elapsed >= Duration::from_millis(500) {
            self.fps.fps = f64::from(self.fps.frames) / elapsed.as_secs_f64();
            (self.fps.frames, self.fps.since) = (0, Instant::now());
        }
        Ok(())
    }

    /// Play ends: the table's own goodbye (its scores saved).
    fn end(&mut self) -> bool {
        if !self.ended {
            self.ended = true;
            self.game.end();
            println!("play ended");
        }
        false
    }

    /// The play as it stands, for the page's console (`vpi.state()`): JSON.
    pub fn state(&self) -> String {
        let s = &self.last;
        format!(
            r#"{{"paused":{},"menu":{},"balls":{},"steps_per_second":{},"fps":{:.1},"focus_lost":{},"ended":{}}}"#,
            self.game.paused(),
            s.menu.is_some(),
            s.balls.len(),
            s.steps_per_second,
            self.fps.fps,
            self.focus_lost,
            self.ended
        )
    }
}

/// The player's options the settings set (`main.rs`'s `apply_player_settings`), as the browser
/// takes them: no HDR, tear-free at the browser's refresh.
struct Options {
    samples: u32,
    aa_factor: f32,
    post_aa: PostAa,
    sharpen: Sharpen,
    ao: player::render::AmbientOcclusion,
    motion_blur: bool,
    bloom_off: bool,
    readout: bool,
    max_fps: f64,
}

impl Options {
    fn from_settings(s: &player::settings::Settings) -> Options {
        use player::vpx_settings as vs;
        Options {
            // WebGPU multisamples 4 times or not at all
            samples: vs::samples(s).map_or(1, |n| if n > 1 { 4 } else { 1 }),
            aa_factor: vs::aa_factor(s).unwrap_or(1.0),
            post_aa: vs::post_aa(s).unwrap_or(PostAa::Off),
            sharpen: vs::sharpen(s).unwrap_or(Sharpen::Off),
            ao: vs::ambient_occlusion(s).unwrap_or(player::render::AmbientOcclusion::Dynamic),
            motion_blur: vs::motion_blur(s).unwrap_or(true),
            bloom_off: vs::bloom_off(s),
            readout: vs::readout(s).unwrap_or(false),
            max_fps: vs::max_framerate(s).map_or(0.0, |f| if f > 0.0 { f64::from(f) } else { 0.0 }),
        }
    }
}

/// VPX's nudge shake (`main.rs`).
fn nudge_shake(offset: [f32; 2], rotation_deg: f32, strength: f32) -> [f32; 2] {
    let (x, y) = (strength * offset[0] * 2.0, strength * offset[1] * 2.0);
    let (s, c) = (-rotation_deg.to_radians()).sin_cos();
    [x * c - y * s, x * s + y * c]
}

/// VPX's guess at how long the frame being made takes to reach the screen (`main.rs`).
fn predicted_display_delay(now_ns: u64, last_present_ns: u64, frame_ns: u64, last_frame_ns: u64) -> f32 {
    if frame_ns == 0 {
        return 0.0;
    }
    let mut delay = frame_ns - now_ns.saturating_sub(last_present_ns) % frame_ns;
    if delay < last_frame_ns {
        delay += frame_ns;
    }
    delay as f32 * 1e-9
}

/// The pause menu's mouse (`main.rs`'s `MenuMouse`): pointing picks a row, a click chooses it (on a
/// bar, sets it there, held to drag it), the wheel scrolls, the right button steps back.
#[derive(Default)]
struct MenuMouse {
    hover: Option<usize>,
    dragging: Option<usize>,
}

impl MenuMouse {
    /// # Safety
    /// With an event of ours, filled in.
    unsafe fn event(&mut self, event: &SDL_Event, layout: &player::pause_menu::Layout, window: *mut SDL_Window, turn: (u32, (u32, u32))) {
        use player::pause_menu::{Pointer, Spot, point};
        // SAFETY: as the caller promises
        unsafe {
            let kind = event.r#type;
            let rows = layout.rows.len();
            let at = |x: f32, y: f32| {
                let density = SDL_GetWindowPixelDensity(window).max(0.01);
                let (px, py) = (x * density, y * density);
                let (w, h) = (turn.1.0 as f32, turn.1.1 as f32);
                match turn.0 % 4 {
                    1 => (py, w - px),
                    2 => (w - px, h - py),
                    3 => (h - py, px),
                    _ => (px, py),
                }
            };
            if kind == SDL_EVENT_MOUSE_MOTION.0 {
                let (x, y) = at(event.motion.x, event.motion.y);
                if let Some(row) = self.dragging {
                    if let Some(along) = layout.along(row, x) {
                        point(Pointer::Drag(row, along, rows));
                    }
                } else {
                    let over = layout.at(x, y).map(|(k, _)| k);
                    if over != self.hover {
                        self.hover = over;
                        if let Some(k) = over {
                            point(Pointer::Over(k, rows));
                        }
                    }
                }
            } else if kind == SDL_EVENT_MOUSE_BUTTON_DOWN.0 {
                let (x, y) = at(event.button.x, event.button.y);
                match i32::from(event.button.button) {
                    SDL_BUTTON_LEFT => {
                        if let Some((k, spot)) = layout.at(x, y) {
                            point(Pointer::Click(k, spot, rows));
                            if matches!(spot, Spot::Bar(_)) {
                                self.dragging = Some(k);
                            }
                        }
                    }
                    SDL_BUTTON_RIGHT | SDL_BUTTON_X1 => point(Pointer::Back),
                    _ => {}
                }
            } else if kind == SDL_EVENT_MOUSE_BUTTON_UP.0 && i32::from(event.button.button) == SDL_BUTTON_LEFT {
                self.dragging = None;
            } else if kind == SDL_EVENT_MOUSE_WHEEL.0 {
                let notches = event.wheel.integer_y;
                if notches != 0 {
                    point(Pointer::Wheel(notches));
                }
            }
        }
    }
}
