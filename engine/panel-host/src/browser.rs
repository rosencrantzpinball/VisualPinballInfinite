//! CEF's side of the panel: the program's start (CEF's own processes end in `execute_process`), CEF set
//! up for drawing off-screen, the page's browser and what it's told, its pictures handed to the player,
//! and the player's input handed to it.
//!
//! CEF runs its own UI thread (`multi_threaded_message_loop`); this program's main thread takes the
//! player's input from the link every few milliseconds and posts it there, where the browser lives. The
//! page draws into `on_paint`, its `<select>` lists in a popup of their own drawn over it here, and
//! each picture goes into the link's triple buffer. Chromium's switches (`switches`) are the codec runs'
//! findings (PLAN decision 20). The program ends when the player asks it to, closes the link, or ends.
use std::cell::RefCell;
use std::os::raw::c_int;
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use cef::*;
use panel_link::{Input, Link, state};

use crate::Options;
use crate::catalog::Catalog;
use crate::commands::Commands;
use crate::serve;

/// The page's background while it loads: the app's dark.
const BACKGROUND: u32 = 0xff_10_12_16;
/// CEF's flags for buttons held during a move (`cef_event_flags_t`).
const LEFT_HELD: u32 = 1 << 4;
const MIDDLE_HELD: u32 = 1 << 5;
const RIGHT_HELD: u32 = 1 << 6;

/// What the handlers and the main thread share.
pub struct Panel {
    pub options: Options,
    pub link: Option<Arc<Link>>,
    pub commands: Commands,
    /// The page's size in pixels and its scale (percent of CSS pixels to them).
    size: Mutex<(u32, u32, u32)>,
    pictures: Mutex<Pictures>,
    drawn: AtomicU32,
    closed: AtomicBool,
}

/// The page's latest picture, and its popup's (an open `<select>`), drawn together.
#[derive(Default)]
struct Pictures {
    width: u32,
    height: u32,
    view: Vec<u8>,
    popup: Option<Popup>,
    /// Both together, as handed over.
    shown: Vec<u8>,
}

struct Popup {
    rect: Rect,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

static PANEL: OnceLock<Panel> = OnceLock::new();

pub fn panel() -> &'static Panel {
    PANEL.get().expect("the panel set up before CEF starts")
}

/// A CEF string handed back as Rust's.
pub fn string(s: &CefStringUserfree) -> String {
    CefString::from(s).to_string()
}

thread_local! {
    /// The page's browser, on CEF's UI thread, where it's made and used.
    static BROWSER: RefCell<Option<Browser>> = const { RefCell::new(None) };
    /// The buttons held, for moves that drag.
    static HELD: RefCell<u32> = const { RefCell::new(0) };
}

impl Panel {
    pub fn size(&self) -> (u32, u32, u32) {
        *self.size.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The view's size in CSS pixels.
    fn view_size(&self) -> (i32, i32) {
        let (w, h, scale) = self.size();
        ((w * 100 / scale.max(1)) as i32, (h * 100 / scale.max(1)) as i32)
    }

    /// A picture from CEF: the view's, or its popup's, into the link with the popup over the view.
    fn painted(&self, popup: bool, width: u32, height: u32, pixels: &[u8]) {
        let mut p = self.pictures.lock().unwrap_or_else(|e| e.into_inner());
        if popup {
            match &mut p.popup {
                Some(open) => {
                    open.width = width;
                    open.height = height;
                    open.pixels.clear();
                    open.pixels.extend_from_slice(pixels);
                }
                None => return,
            }
        } else {
            p.width = width;
            p.height = height;
            p.view.clear();
            p.view.extend_from_slice(pixels);
        }
        self.hand_over(&mut p);
    }

    fn hand_over(&self, p: &mut Pictures) {
        let Pictures { width, height, view, popup, shown } = p;
        let (width, height) = (*width, *height);
        if view.len() != (width * height * 4) as usize {
            return;
        }
        shown.clear();
        shown.extend_from_slice(view);
        if let Some(pop) = popup.as_ref().filter(|o| o.pixels.len() == (o.width * o.height * 4) as usize) {
            // the popup's rectangle is in CSS pixels; its picture in the view's
            let scale = self.size().2 as f32 / 100.0;
            let (x0, y0) = ((pop.rect.x as f32 * scale) as i64, (pop.rect.y as f32 * scale) as i64);
            for row in 0..pop.height as i64 {
                let y = y0 + row;
                if y < 0 || y >= height as i64 {
                    continue;
                }
                let from_x = x0.max(0);
                let to_x = (x0 + pop.width as i64).min(width as i64);
                if from_x >= to_x {
                    continue;
                }
                let src = ((row * pop.width as i64 + (from_x - x0)) * 4) as usize;
                let dst = ((y * width as i64 + from_x) * 4) as usize;
                let n = ((to_x - from_x) * 4) as usize;
                shown[dst..dst + n].copy_from_slice(&pop.pixels[src..src + n]);
            }
        }
        let n = self.drawn.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(link) = &self.link {
            link.draw(width, height, n, |out| out.copy_from_slice(shown));
            if n == 1 {
                link.set_state(state::READY);
            }
        }
    }
}

/// Runs `job` on CEF's UI thread.
pub fn on_ui(job: impl FnOnce() + Send + 'static) {
    let job: Box<dyn FnOnce() + Send> = Box::new(job);
    let mut task = Job::new(Arc::new(Mutex::new(Some(job))));
    post_task(ThreadId::UI, Some(&mut task));
}

/// The page's browser, on CEF's UI thread: a reference of its own, so CEF may call back into the
/// handlers (closing calls `on_before_close` at once) while `f` runs.
fn with_browser(f: impl FnOnce(&Browser)) {
    let browser = BROWSER.with(|b| b.borrow().clone());
    if let Some(b) = browser {
        f(&b);
    }
}

/// Tells the page an event, as the app's engine side emits its own (`listen` gets it).
pub fn emit(name: &str, payload: serde_json::Value) {
    let js = format!("window.__VPI_EMIT__ && window.__VPI_EMIT__({}, {payload})", serde_json::Value::from(name));
    on_ui(move || {
        with_browser(|b| {
            if let Some(frame) = b.main_frame() {
                frame.execute_java_script(Some(&CefString::from(js.as_str())), Some(&CefString::from("vpi://emit")), 0);
            }
        });
    });
}

/// The player's input, on CEF's UI thread. Points come in the page's pixels, CEF takes CSS pixels.
fn deliver(input: Input) {
    with_browser(|b| {
        let Some(host) = b.host() else { return };
        let scale = panel().size().2.max(1) as f32 / 100.0;
        let held = HELD.with(|h| *h.borrow());
        let at = |x: f32, y: f32| MouseEvent { x: (x / scale) as c_int, y: (y / scale) as c_int, modifiers: held };
        match input {
            Input::Move { x, y } => host.send_mouse_move_event(Some(&at(x, y)), 0),
            Input::Leave => host.send_mouse_move_event(Some(&at(-1.0, -1.0)), 1),
            Input::Button { button, down, x, y } => {
                let (kind, flag) = match button {
                    1 => (MouseButtonType::MIDDLE, MIDDLE_HELD),
                    2 => (MouseButtonType::RIGHT, RIGHT_HELD),
                    _ => (MouseButtonType::LEFT, LEFT_HELD),
                };
                HELD.with(|h| {
                    let mut h = h.borrow_mut();
                    *h = if down { *h | flag } else { *h & !flag };
                });
                let mut e = at(x, y);
                e.modifiers = HELD.with(|h| *h.borrow());
                host.send_mouse_click_event(Some(&e), kind, (!down) as c_int, 1);
            }
            Input::Wheel { x, y, dx, dy } => host.send_mouse_wheel_event(Some(&at(x, y)), dx as c_int, dy as c_int),
            Input::Key { code, down, modifiers } => {
                let e = KeyEvent {
                    type_: if down { KeyEventType::RAWKEYDOWN } else { KeyEventType::KEYUP },
                    modifiers,
                    windows_key_code: code as c_int,
                    ..Default::default()
                };
                host.send_key_event(Some(&e));
            }
            Input::Char { character, modifiers } => {
                let e = KeyEvent {
                    type_: KeyEventType::CHAR,
                    modifiers,
                    windows_key_code: character as c_int,
                    character: character as u16,
                    unmodified_character: character as u16,
                    ..Default::default()
                };
                host.send_key_event(Some(&e));
            }
            Input::Visible(shown) => {
                host.was_hidden((!shown) as c_int);
                host.set_windowless_frame_rate(if shown { 60 } else { 1 });
            }
        }
    });
}

wrap_task! {
    struct Job {
        job: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>,
    }

    impl Task {
        fn execute(&self) {
            let job = self.job.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(job) = job {
                job();
            }
        }
    }
}

/// Chromium's switches for the panel, the codec runs' findings on the Frame (PLAN decisions 19, 20).
/// Whether Chromium draws on X11 (the game session's X server, when there is one), else on its headless
/// platform; `VPI_PANEL_PLATFORM` (x11 or headless) picks for checks.
fn on_x11() -> bool {
    match std::env::var("VPI_PANEL_PLATFORM").as_deref() {
        Ok("x11") => true,
        Ok("headless") => false,
        _ => std::env::var_os("DISPLAY").is_some(),
    }
}

fn switches(line: &CommandLine) {
    let set = |name: &str, value: &str| line.append_switch_with_value(Some(&CefString::from(name)), Some(&CefString::from(value)));
    // V4L2's hardware decoders (the Frame's iris): a feature builds without VA-API leave off
    set("enable-features", "AcceleratedVideoDecoder");
    // its videos play as they show, with no click first
    set("autoplay-policy", "no-user-gesture-required");
    if on_x11() {
        // the game session's X server (gamescope's Xwayland), ANGLE on GLX: on EGL or Vulkan Chromium
        // finds no config for an X window, composites in software and then decodes in software
        set("ozone-platform", "x11");
        set("use-angle", "gl");
    } else {
        // no X server (checks in the build image, test runs): Chromium's headless platform, ANGLE on EGL
        // (surfaceless, set before CEF starts)
        set("ozone-platform", "headless");
        set("use-angle", "gl-egl");
    }
    // checks: more of Chromium's switches (`VPI_PANEL_SWITCHES=disable-gpu-rasterization,num-raster-threads=2`)
    for switch in std::env::var("VPI_PANEL_SWITCHES").unwrap_or_default().split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match switch.split_once('=') {
            Some((name, value)) => set(name, value),
            None => line.append_switch(Some(&CefString::from(switch))),
        }
    }
}

wrap_app! {
    struct PanelApp {
        browser_process: BrowserProcessHandler,
    }

    impl App {
        fn on_before_command_line_processing(&self, process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
            // the browser process's own line: the others take theirs from it
            let browser_process = process_type.is_none_or(|t| t.to_string().is_empty());
            if let (true, Some(line)) = (browser_process, command_line) {
                switches(line);
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(self.browser_process.clone())
        }
    }
}

wrap_browser_process_handler! {
    struct PanelBrowserProcess;

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            serve::register();
            let p = panel();
            let window_info = WindowInfo::default().set_as_windowless(0);
            let settings = BrowserSettings { windowless_frame_rate: 60, background_color: BACKGROUND, ..Default::default() };
            let mut client = PanelClient::new(PanelRender::new(), PanelLife::new(), PanelDisplay::new(), PanelLoad::new(), PanelRequests::new());
            let url = CefString::from(p.options.url.as_str());
            if browser_host_create_browser(Some(&window_info), Some(&mut client), Some(&url), Some(&settings), None, None) != 1 {
                eprintln!("vpi-panel: the page's browser couldn't be made");
                if let Some(link) = &p.link {
                    link.set_state(state::FAILED);
                }
            }
        }
    }
}

wrap_client! {
    struct PanelClient {
        render: RenderHandler,
        life: LifeSpanHandler,
        display: DisplayHandler,
        load: LoadHandler,
        requests: RequestHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(self.render.clone())
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(self.life.clone())
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(self.display.clone())
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(self.load.clone())
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(self.requests.clone())
        }
    }
}

wrap_render_handler! {
    struct PanelRender;

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                let (width, height) = panel().view_size();
                *rect = Rect { x: 0, y: 0, width, height };
            }
        }

        fn screen_info(&self, _browser: Option<&mut Browser>, screen_info: Option<&mut ScreenInfo>) -> c_int {
            let Some(info) = screen_info else { return 0 };
            let (width, height) = panel().view_size();
            info.device_scale_factor = panel().size().2 as f32 / 100.0;
            info.depth = 24;
            info.depth_per_component = 8;
            info.rect = Rect { x: 0, y: 0, width, height };
            info.available_rect = Rect { x: 0, y: 0, width, height };
            1
        }

        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: c_int) {
            let p = panel();
            let mut pictures = p.pictures.lock().unwrap_or_else(|e| e.into_inner());
            if show == 0 {
                pictures.popup = None;
                p.hand_over(&mut pictures);
            } else if pictures.popup.is_none() {
                pictures.popup = Some(Popup { rect: Rect::default(), width: 0, height: 0, pixels: Vec::new() });
            }
        }

        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) {
            let mut pictures = panel().pictures.lock().unwrap_or_else(|e| e.into_inner());
            if let (Some(open), Some(rect)) = (pictures.popup.as_mut(), rect) {
                open.rect = rect.clone();
            }
        }

        fn on_paint(&self, _browser: Option<&mut Browser>, type_: PaintElementType, _dirty_rects: Option<&[Rect]>, buffer: *const u8, width: c_int, height: c_int) {
            if buffer.is_null() || width <= 0 || height <= 0 {
                return;
            }
            // SAFETY: CEF's picture, `width * height` pixels of BGRA, for the length of the call
            let pixels = unsafe { std::slice::from_raw_parts(buffer, width as usize * height as usize * 4) };
            panel().painted(type_ == PaintElementType::POPUP, width as u32, height as u32, pixels);
        }
    }
}

wrap_life_span_handler! {
    struct PanelLife;

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            if let Some(b) = browser {
                BROWSER.with(|x| *x.borrow_mut() = Some(b.clone()));
                if let Some(host) = b.host() {
                    host.set_focus(1);
                }
            }
        }

        // links that would open a window of their own: nowhere in the headset
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut c_int>,
        ) -> c_int {
            eprintln!("vpi-panel: a window not opened: {}", target_url.map(|u| u.to_string()).unwrap_or_default());
            1
        }

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            BROWSER.with(|x| *x.borrow_mut() = None);
            panel().closed.store(true, Ordering::Release);
        }
    }
}

wrap_display_handler! {
    struct PanelDisplay;

    impl DisplayHandler {
        fn on_console_message(&self, _browser: Option<&mut Browser>, _level: LogSeverity, message: Option<&CefString>, source: Option<&CefString>, line: c_int) -> c_int {
            let source = source.map(|s| s.to_string()).unwrap_or_default();
            eprintln!("vpi-panel: page: {} ({}:{line})", message.map(|m| m.to_string()).unwrap_or_default(), source.rsplit('/').next().unwrap_or(""));
            1
        }
    }
}

wrap_load_handler! {
    struct PanelLoad;

    impl LoadHandler {
        fn on_load_error(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, error_code: Errorcode, error_text: Option<&CefString>, failed_url: Option<&CefString>) {
            eprintln!(
                "vpi-panel: {} didn't load: {} ({error_code:?})",
                failed_url.map(|u| u.to_string()).unwrap_or_default(),
                error_text.map(|t| t.to_string()).unwrap_or_default()
            );
        }
    }
}

wrap_request_handler! {
    struct PanelRequests;

    impl RequestHandler {
        // the page stays on the screens' host: anything else isn't shown in the headset
        fn on_before_browse(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>, _user_gesture: c_int, _is_redirect: c_int) -> c_int {
            let url = request.map(|r| string(&r.url())).unwrap_or_default();
            let ours = url.starts_with("http://tauri.localhost/") || url == "http://tauri.localhost" || url.starts_with("devtools://");
            if !ours {
                eprintln!("vpi-panel: not followed: {url}");
            }
            (!ours) as c_int
        }
    }
}

/// A picture's name when there are several: `<name>-<ms>.<ext>`.
fn shot_at(shot: &Path, at: u64) -> std::path::PathBuf {
    let stem = shot.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = shot.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    shot.with_file_name(format!("{stem}-{at}{ext}"))
}

/// The page's picture saved as a PNG (`--shot`).
fn save_shot(path: &Path) -> Result<(), String> {
    let p = panel().pictures.lock().unwrap_or_else(|e| e.into_inner());
    if p.shown.is_empty() {
        return Err("the page drew nothing".into());
    }
    let rgba: Vec<u8> = p.shown.chunks_exact(4).flat_map(|c| [c[2], c[1], c[0], 255]).collect();
    let image = image::RgbaImage::from_raw(p.width, p.height, rgba).ok_or("the picture's size")?;
    image.save(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn main() -> ExitCode {
    // the API version CEF's headers here declare, set before any other call into CEF (its versioned
    // API: without it, CEF takes the objects handed to it as of no version and stops)
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = cef::args::Args::new();
    let mut app = PanelApp::new(PanelBrowserProcess::new());
    // CEF's own processes (the page's renderer, the GPU's, the network's) end here
    let code = execute_process(Some(args.as_main_args()), Some(&mut app), std::ptr::null_mut());
    if code >= 0 {
        return ExitCode::from(code.clamp(0, 255) as u8);
    }
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("vpi-panel: {e}");
            return ExitCode::from(2);
        }
    };
    let link = match &options.link {
        Some(name) => match Link::open(name) {
            Ok(l) => Some(Arc::new(l)),
            Err(e) => {
                eprintln!("vpi-panel: the link {name}: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    let size = link.as_ref().map_or((options.size.0, options.size.1, options.scale), |l| l.wanted());
    if let Err(e) = std::fs::create_dir_all(&options.data) {
        eprintln!("vpi-panel: {}: {e}", options.data.display());
    }
    let catalog = Arc::new(Mutex::new(Catalog::new(&options.library, &options.data)));
    let _ = PANEL.set(Panel {
        options: options.clone(),
        link: link.clone(),
        commands: Commands { catalog: catalog.clone(), link: link.clone(), back: options.back.clone(), asked: Default::default() },
        size: Mutex::new(size),
        pictures: Mutex::new(Pictures::default()),
        drawn: AtomicU32::new(0),
        closed: AtomicBool::new(false),
    });
    if !on_x11() {
        // SAFETY: before CEF or any thread of ours starts; the GPU process inherits it
        unsafe { std::env::set_var("EGL_PLATFORM", "surfaceless") };
    }

    let cache = options.data.join("cache");
    // one panel to a player, so a lock left in its profile is from one that ended without closing it,
    // which Chromium would take, on another host (a container's), for a panel still running
    for lock in ["SingletonLock", "SingletonSocket", "SingletonCookie"] {
        let _ = std::fs::remove_file(cache.join(lock));
    }
    let settings = Settings {
        no_sandbox: 1,
        windowless_rendering_enabled: 1,
        multi_threaded_message_loop: 1,
        root_cache_path: CefString::from(cache.to_string_lossy().as_ref()),
        cache_path: CefString::from(cache.join("default").to_string_lossy().as_ref()),
        log_file: CefString::from(options.data.join("cef.log").to_string_lossy().as_ref()),
        log_severity: LogSeverity::WARNING,
        background_color: BACKGROUND,
        persist_session_cookies: 0,
        ..Default::default()
    };
    if initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()) != 1 {
        eprintln!("vpi-panel: CEF didn't start");
        if let Some(link) = &link {
            link.set_state(state::FAILED);
        }
        return ExitCode::FAILURE;
    }

    // the library read beside, once CEF is there to tell the screens as it goes
    {
        let catalog = catalog.clone();
        let (layout, folder) = (catalog.lock().unwrap_or_else(|e| e.into_inner()).layout.clone(), options.data.clone());
        std::thread::spawn(move || {
            let tell = |c: &Arc<Mutex<Catalog>>, change: &dyn Fn(&mut crate::catalog::Status)| {
                let status = {
                    let mut c = c.lock().unwrap_or_else(|e| e.into_inner());
                    change(&mut c.status);
                    c.status.clone()
                };
                emit("library-status", serde_json::to_value(status).unwrap_or_default());
            };
            let mut last = Instant::now();
            let read = Catalog::read(&layout, &folder, |done, of, path| {
                if last.elapsed() > Duration::from_millis(200) {
                    last = Instant::now();
                    let current = path.file_stem().map(|s| s.to_string_lossy().into_owned());
                    tell(&catalog, &|s| {
                        s.phase = "reading";
                        s.done = done;
                        s.of = of;
                        s.current = current.clone();
                    });
                }
            });
            {
                let mut c = catalog.lock().unwrap_or_else(|e| e.into_inner());
                match read {
                    Ok(tables) => {
                        c.tables = tables;
                        c.status = crate::catalog::Status { phase: "ready", ..Default::default() };
                    }
                    Err(e) => {
                        c.status = crate::catalog::Status { phase: "ready", problem: Some(format!("The tables couldn't be read: {e}")), ..Default::default() };
                    }
                }
            }
            tell(&catalog, &|_| {});
            emit("library-changed", serde_json::Value::Null);
        });
    }

    let p = panel();
    let parent = std::os::unix::process::parent_id();
    let started = Instant::now();
    let mut ended = "the page closed";
    let (mut moved, mut taken) = (0, 0);
    loop {
        std::thread::sleep(Duration::from_millis(4));
        if p.closed.load(Ordering::Acquire) {
            break;
        }
        if let Some(link) = &link {
            while let Some(input) = link.receive() {
                on_ui(move || deliver(input));
            }
            let wanted = link.wanted();
            if wanted != p.size() {
                *p.size.lock().unwrap_or_else(|e| e.into_inner()) = wanted;
                on_ui(|| {
                    with_browser(|b| {
                        if let Some(host) = b.host() {
                            host.notify_screen_info_changed();
                            host.was_resized();
                        }
                    })
                });
            }
            if link.stop_requested() {
                ended = "the player asked it to";
                break;
            }
            // the player gone without asking: its child goes with it
            if std::os::unix::process::parent_id() != parent {
                ended = "the player ended";
                break;
            }
        }
        // checks: the pointer moved, then the pictures taken, at their times
        while let Some(&(at, x, y, click)) = options.moves.get(moved)
            && started.elapsed() >= Duration::from_millis(at)
        {
            moved += 1;
            on_ui(move || deliver(Input::Move { x, y }));
            if click {
                on_ui(move || deliver(Input::Button { button: 0, down: true, x, y }));
                on_ui(move || deliver(Input::Button { button: 0, down: false, x, y }));
            }
        }
        if let Some(shot) = &options.shot
            && let Some(&at) = options.shot_at_ms.get(taken)
            && started.elapsed() >= Duration::from_millis(at)
        {
            taken += 1;
            let path = if options.shot_at_ms.len() > 1 { shot_at(shot, at) } else { shot.clone() };
            match save_shot(&path) {
                Ok(()) => println!("vpi-panel: the page's picture: {}", path.display()),
                Err(e) => eprintln!("vpi-panel: {e}"),
            }
            if taken == options.shot_at_ms.len() {
                ended = "its pictures were taken";
                break;
            }
        }
    }
    eprintln!("vpi-panel: ending: {ended}");
    on_ui(|| with_browser(|b| b.host().into_iter().for_each(|h| h.close_browser(1))));
    let closing = Instant::now();
    while !p.closed.load(Ordering::Acquire) && closing.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(10));
    }
    shutdown();
    if let Some(link) = &link {
        link.set_state(state::ENDED);
    }
    ExitCode::SUCCESS
}
