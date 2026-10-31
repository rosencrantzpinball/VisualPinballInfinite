// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026, the Visual Pinball Infinite contributors
//! The link between the player and `vpi-panel`, the process that shows the app's screens in the
//! headset (CEF off-screen, `engine/panel-host`): one block of shared memory, which the player makes
//! under a name and the panel opens. In it:
//!
//! - the panel's state, and the size and scale the player wants its page drawn at;
//! - its pictures: three frames of BGRA, CEF's order, at most `WIDTH_MAX` by `HEIGHT_MAX`, handed over
//!   as a triple buffer: the panel draws into one, the player copies another, and the third holds the
//!   newest whole one, so neither waits for the other and neither sees half a frame;
//! - two rings of fixed records, one writer and one reader each: the player's input to the panel (the
//!   pointer, its buttons and wheel, keys and text, the page shown or hidden) and the panel's
//!   requests to the player (play a table, close the panel, quit).
//!
//! Neither side blocks: the panel takes its input every few milliseconds, the player its requests and
//! pictures each frame. The player unlinks the name as it closes its end. Linux only (the Steam
//! Frame), with POSIX shared memory declared from glibc's headers; elsewhere `create` and `open` say so.
use std::cell::UnsafeCell;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

/// The largest page, in pixels.
pub const WIDTH_MAX: u32 = 2048;
pub const HEIGHT_MAX: u32 = 1280;
const FRAME_BYTES: usize = WIDTH_MAX as usize * HEIGHT_MAX as usize * 4;
const MAGIC: u32 = u32::from_le_bytes(*b"VPIP");
const VERSION: u32 = 1;
/// Ring sizes, powers of two: the counters wrap.
const INPUTS: usize = 512;
const REQUESTS: usize = 8;
/// A request's text (a table's path), in bytes of UTF-8.
const TEXT_MAX: usize = 1016;
/// Where the frames start in the block, past the header.
const FRAMES_AT: usize = 64 * 1024;
const BLOCK_BYTES: usize = FRAMES_AT + 3 * FRAME_BYTES;
/// The triple buffer's middle holds a frame's number (0 to 2), with this bit while the player hasn't
/// taken it yet.
const FRESH: u32 = 4;

/// The panel's state, as it tells the player.
pub mod state {
    pub const STARTING: u32 = 0;
    /// Its page drawn once: frames come from here on.
    pub const READY: u32 = 1;
    pub const FAILED: u32 = 2;
    pub const ENDED: u32 = 3;
}

/// The player's input to the panel. Points are in the page's pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    /// The pointer over the page.
    Move { x: f32, y: f32 },
    /// The pointer off the page.
    Leave,
    /// A button pressed or let go at a point (0 left, 1 middle, 2 right).
    Button { button: u32, down: bool, x: f32, y: f32 },
    /// The wheel turned at a point, in pixels (positive: up and left, as CEF counts).
    Wheel { x: f32, y: f32, dx: f32, dy: f32 },
    /// A key pressed or let go: Windows' virtual key code, as CEF takes it, and CEF's modifier flags.
    Key { code: u32, down: bool, modifiers: u32 },
    /// A character typed (UTF-16 code unit).
    Char { character: u32, modifiers: u32 },
    /// The page shown in the headset, or not: hidden, it draws at a frame a second.
    Visible(bool),
}

/// The panel's requests to the player.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Play this table: the player starts again with it.
    Play(PathBuf),
    /// Close the panel, back to the table (or to nothing, with none playing).
    Close,
    /// Quit the player.
    Quit,
}

/// A frame the player took: valid until it takes the next.
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    /// Counted up by the panel for each frame it draws.
    pub number: u32,
    /// `width * height` pixels of BGRA, rows packed.
    pub pixels: &'a [u8],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputRecord {
    kind: u32,
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: u32,
    f: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RequestRecord {
    kind: u32,
    len: u32,
    text: [u8; TEXT_MAX],
}

/// A ring of records, one writer and one reader: counters that only grow (wrapping), the slot a counter
/// names its value modulo the ring's size.
#[repr(C)]
struct Ring<T: Copy, const N: usize> {
    head: AtomicU32,
    tail: AtomicU32,
    slots: [UnsafeCell<T>; N],
}

// SAFETY: one writer and one reader; a slot is written only while the reader can't reach it, and read
// only after the writer released it through `head`
unsafe impl<T: Copy, const N: usize> Sync for Ring<T, N> {}

impl<T: Copy, const N: usize> Ring<T, N> {
    fn push(&self, value: T) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) as usize >= N {
            return false;
        }
        // SAFETY: the slot isn't the reader's until `head` moves past it
        unsafe { *self.slots[head as usize % N].get() = value };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    fn pop(&self) -> Option<T> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        // SAFETY: the writer released the slot through `head`, and doesn't reuse it until `tail` moves
        let value = unsafe { *self.slots[tail as usize % N].get() };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(value)
    }
}

#[repr(C)]
struct FrameHeader {
    width: AtomicU32,
    height: AtomicU32,
    number: AtomicU32,
}

/// The block's header; the frames follow at `FRAMES_AT`.
#[repr(C)]
struct Shared {
    magic: AtomicU32,
    version: AtomicU32,
    state: AtomicU32,
    want_width: AtomicU32,
    want_height: AtomicU32,
    /// The page's scale, in percent of CSS pixels to the frame's.
    want_scale: AtomicU32,
    middle: AtomicU32,
    stop: AtomicU32,
    headers: [FrameHeader; 3],
    to_panel: Ring<InputRecord, INPUTS>,
    to_player: Ring<RequestRecord, REQUESTS>,
}

const _: () = assert!(size_of::<Shared>() <= FRAMES_AT);
const _: () = assert!(INPUTS.is_power_of_two() && REQUESTS.is_power_of_two());

/// One end of the link.
pub struct Link {
    name: String,
    view: *mut u8,
    owner: bool,
    /// The panel's frame being drawn, or the player's frame being read: the triple buffer's ends.
    own_frame: AtomicU32,
}

// SAFETY: the block is shared memory made of atomics, rings with one writer and one reader each, and
// frames handed over by the triple buffer; nothing here is tied to the thread that made it
unsafe impl Send for Link {}
unsafe impl Sync for Link {}

impl Link {
    /// The player's end: a new link of that name (`vpi-panel-<pid>`, say), for a page `width` by
    /// `height` pixels at `scale` percent.
    pub fn create(name: &str, width: u32, height: u32, scale: u32) -> io::Result<Link> {
        let view = sys::create(name, BLOCK_BYTES)?;
        let link = Link { name: name.to_string(), view, owner: true, own_frame: AtomicU32::new(1) };
        let s = link.shared();
        s.state.store(state::STARTING, Ordering::Relaxed);
        s.want_width.store(width.clamp(64, WIDTH_MAX), Ordering::Relaxed);
        s.want_height.store(height.clamp(64, HEIGHT_MAX), Ordering::Relaxed);
        s.want_scale.store(scale.clamp(50, 400), Ordering::Relaxed);
        // frame 2 in the middle, the panel drawing 0 first, the player holding 1
        s.middle.store(2, Ordering::Relaxed);
        s.version.store(VERSION, Ordering::Relaxed);
        s.magic.store(MAGIC, Ordering::Release);
        Ok(link)
    }

    /// The panel's end: the link the player made.
    pub fn open(name: &str) -> io::Result<Link> {
        let view = sys::open(name, BLOCK_BYTES)?;
        let link = Link { name: name.to_string(), view, owner: false, own_frame: AtomicU32::new(0) };
        let s = link.shared();
        if s.magic.load(Ordering::Acquire) != MAGIC || s.version.load(Ordering::Relaxed) != VERSION {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{name} isn't a panel link of this version")));
        }
        Ok(link)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn shared(&self) -> &Shared {
        // SAFETY: the block starts with the header, mapped for the link's life
        unsafe { &*(self.view as *const Shared) }
    }

    fn frame_bytes(&self, index: u32) -> *mut u8 {
        // SAFETY: three frames follow the header within the block
        unsafe { self.view.add(FRAMES_AT + index as usize * FRAME_BYTES) }
    }

    pub fn state(&self) -> u32 {
        self.shared().state.load(Ordering::Acquire)
    }

    pub fn set_state(&self, state: u32) {
        self.shared().state.store(state, Ordering::Release);
    }

    /// The page's size in pixels and its scale in percent, as the player wants it now.
    pub fn wanted(&self) -> (u32, u32, u32) {
        let s = self.shared();
        (s.want_width.load(Ordering::Relaxed), s.want_height.load(Ordering::Relaxed), s.want_scale.load(Ordering::Relaxed))
    }

    pub fn want(&self, width: u32, height: u32, scale: u32) {
        let s = self.shared();
        s.want_width.store(width.clamp(64, WIDTH_MAX), Ordering::Relaxed);
        s.want_height.store(height.clamp(64, HEIGHT_MAX), Ordering::Relaxed);
        s.want_scale.store(scale.clamp(50, 400), Ordering::Relaxed);
    }

    pub fn request_stop(&self) {
        self.shared().stop.store(1, Ordering::Release);
    }

    pub fn stop_requested(&self) -> bool {
        self.shared().stop.load(Ordering::Acquire) != 0
    }

    /// The panel: draws a frame into the triple buffer's free one with `fill` (given `width * height`
    /// BGRA pixels to write), then hands it over as the newest.
    pub fn draw(&self, width: u32, height: u32, number: u32, fill: impl FnOnce(&mut [u8])) {
        let (width, height) = (width.min(WIDTH_MAX), height.min(HEIGHT_MAX));
        let back = self.own_frame.load(Ordering::Relaxed);
        // SAFETY: the triple buffer leaves `back` to the panel alone until it hands it over
        let pixels = unsafe { std::slice::from_raw_parts_mut(self.frame_bytes(back), width as usize * height as usize * 4) };
        fill(pixels);
        let h = &self.shared().headers[back as usize];
        h.width.store(width, Ordering::Relaxed);
        h.height.store(height, Ordering::Relaxed);
        h.number.store(number, Ordering::Relaxed);
        let was = self.shared().middle.swap(back | FRESH, Ordering::AcqRel);
        self.own_frame.store(was & 3, Ordering::Relaxed);
    }

    /// The player: the newest frame, if the panel drew one since it last took one.
    pub fn take_frame(&self) -> Option<Frame<'_>> {
        let s = self.shared();
        if s.middle.load(Ordering::Acquire) & FRESH == 0 {
            return None;
        }
        let front = s.middle.swap(self.own_frame.load(Ordering::Relaxed), Ordering::AcqRel) & 3;
        self.own_frame.store(front, Ordering::Relaxed);
        let h = &s.headers[front as usize];
        let (width, height) = (h.width.load(Ordering::Relaxed), h.height.load(Ordering::Relaxed));
        // SAFETY: the triple buffer leaves `front` to the player alone until it takes the next
        let pixels = unsafe { std::slice::from_raw_parts(self.frame_bytes(front), width as usize * height as usize * 4) };
        Some(Frame { width, height, number: h.number.load(Ordering::Relaxed), pixels })
    }

    /// The player: input for the panel (false: its ring is full, the input dropped).
    pub fn send(&self, input: Input) -> bool {
        let r = match input {
            Input::Move { x, y } => InputRecord { kind: 1, a: x, b: y, ..Default::default() },
            Input::Leave => InputRecord { kind: 2, ..Default::default() },
            Input::Button { button, down, x, y } => InputRecord { kind: 3, a: x, b: y, e: button, f: down as u32, ..Default::default() },
            Input::Wheel { x, y, dx, dy } => InputRecord { kind: 4, a: x, b: y, c: dx, d: dy, ..Default::default() },
            Input::Key { code, down, modifiers } => InputRecord { kind: 5, e: code, f: down as u32 | modifiers << 1, ..Default::default() },
            Input::Char { character, modifiers } => InputRecord { kind: 6, e: character, f: modifiers, ..Default::default() },
            Input::Visible(on) => InputRecord { kind: 7, e: on as u32, ..Default::default() },
        };
        self.shared().to_panel.push(r)
    }

    /// The panel: the player's next input.
    pub fn receive(&self) -> Option<Input> {
        loop {
            let r = self.shared().to_panel.pop()?;
            return Some(match r.kind {
                1 => Input::Move { x: r.a, y: r.b },
                2 => Input::Leave,
                3 => Input::Button { button: r.e, down: r.f != 0, x: r.a, y: r.b },
                4 => Input::Wheel { x: r.a, y: r.b, dx: r.c, dy: r.d },
                5 => Input::Key { code: r.e, down: r.f & 1 != 0, modifiers: r.f >> 1 },
                6 => Input::Char { character: r.e, modifiers: r.f },
                7 => Input::Visible(r.e != 0),
                _ => continue,
            });
        }
    }

    /// The panel: a request for the player (false: its ring is full, or a path too long for it).
    pub fn ask(&self, request: &Request) -> bool {
        let mut r = RequestRecord { kind: 0, len: 0, text: [0; TEXT_MAX] };
        match request {
            Request::Play(path) => {
                let text = path.to_string_lossy();
                if text.len() > TEXT_MAX {
                    return false;
                }
                r.kind = 1;
                r.len = text.len() as u32;
                r.text[..text.len()].copy_from_slice(text.as_bytes());
            }
            Request::Close => r.kind = 2,
            Request::Quit => r.kind = 3,
        }
        self.shared().to_player.push(r)
    }

    /// The player: the panel's next request.
    pub fn request(&self) -> Option<Request> {
        loop {
            let r = self.shared().to_player.pop()?;
            return Some(match r.kind {
                1 => Request::Play(PathBuf::from(String::from_utf8_lossy(&r.text[..(r.len as usize).min(TEXT_MAX)]).into_owned())),
                2 => Request::Close,
                3 => Request::Quit,
                _ => continue,
            });
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        sys::release(&self.name, self.view, BLOCK_BYTES, self.owner);
    }
}

#[cfg(target_os = "linux")]
mod sys {
    use std::ffi::{CString, c_char, c_int, c_uint, c_void};
    use std::io;

    const O_RDWR: c_int = 0o2;
    const O_CREAT: c_int = 0o100;
    const O_EXCL: c_int = 0o200;
    const PROT_READ: c_int = 1;
    const PROT_WRITE: c_int = 2;
    const MAP_SHARED: c_int = 1;
    const MAP_FAILED: *mut c_void = !0usize as *mut c_void;

    unsafe extern "C" {
        fn shm_open(name: *const c_char, flags: c_int, mode: c_uint) -> c_int;
        fn shm_unlink(name: *const c_char) -> c_int;
        fn ftruncate(fd: c_int, length: i64) -> c_int;
        fn close(fd: c_int) -> c_int;
        fn mmap(address: *mut c_void, length: usize, protection: c_int, flags: c_int, fd: c_int, offset: i64) -> *mut c_void;
        fn munmap(address: *mut c_void, length: usize) -> c_int;
    }

    /// POSIX's name for the link: a `/`, then the name.
    fn posix(name: &str) -> io::Result<CString> {
        CString::new(format!("/{}", name.trim_start_matches('/'))).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a link's name with a NUL"))
    }

    fn map(fd: c_int, size: usize) -> io::Result<*mut u8> {
        // SAFETY: a shared mapping of the whole object, which is `size` bytes long
        let view = unsafe { mmap(std::ptr::null_mut(), size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0) };
        let error = io::Error::last_os_error();
        // SAFETY: ours; the mapping stays once mapped
        unsafe { close(fd) };
        if view == MAP_FAILED { Err(error) } else { Ok(view.cast()) }
    }

    pub fn create(name: &str, size: usize) -> io::Result<*mut u8> {
        let n = posix(name)?;
        // SAFETY: plain POSIX calls on a NUL-terminated name; the object starts zeroed
        unsafe {
            let fd = shm_open(n.as_ptr(), O_RDWR | O_CREAT | O_EXCL, 0o600);
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            if ftruncate(fd, size as i64) != 0 {
                let e = io::Error::last_os_error();
                close(fd);
                shm_unlink(n.as_ptr());
                return Err(e);
            }
            map(fd, size).inspect_err(|_| {
                shm_unlink(n.as_ptr());
            })
        }
    }

    pub fn open(name: &str, size: usize) -> io::Result<*mut u8> {
        let n = posix(name)?;
        // SAFETY: plain POSIX calls on a NUL-terminated name
        let fd = unsafe { shm_open(n.as_ptr(), O_RDWR, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        map(fd, size)
    }

    pub fn release(name: &str, view: *mut u8, size: usize, owner: bool) {
        // SAFETY: the link's own mapping, unmapped once
        unsafe { munmap(view.cast(), size) };
        if owner && let Ok(n) = posix(name) {
            // SAFETY: a NUL-terminated name
            unsafe { shm_unlink(n.as_ptr()) };
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod sys {
    use std::io;

    fn unsupported() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "the headset's panel runs on Linux only")
    }

    pub fn create(_name: &str, _size: usize) -> io::Result<*mut u8> {
        Err(unsupported())
    }

    pub fn open(_name: &str, _size: usize) -> io::Result<*mut u8> {
        Err(unsupported())
    }

    pub fn release(_name: &str, _view: *mut u8, _size: usize, _owner: bool) {}
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn name(what: &str) -> String {
        format!("vpi-panel-test-{what}-{}", std::process::id())
    }

    #[test]
    fn input_and_requests_go_through_in_order() {
        let player = Link::create(&name("rings"), 1280, 800, 100).unwrap();
        let panel = Link::open(player.name()).unwrap();
        assert_eq!(panel.wanted(), (1280, 800, 100));
        let inputs = [
            Input::Move { x: 1.5, y: 2.0 },
            Input::Button { button: 0, down: true, x: 3.0, y: 4.0 },
            Input::Wheel { x: 5.0, y: 6.0, dx: 0.0, dy: -120.0 },
            Input::Key { code: 13, down: true, modifiers: 2 },
            Input::Char { character: 'a' as u32, modifiers: 0 },
            Input::Visible(false),
            Input::Leave,
        ];
        for i in inputs {
            assert!(player.send(i));
        }
        let got: Vec<Input> = std::iter::from_fn(|| panel.receive()).collect();
        assert_eq!(got, inputs);
        assert!(panel.ask(&Request::Play(PathBuf::from("/home/steamos/Tables/A/A.vpx"))));
        assert!(panel.ask(&Request::Close));
        assert_eq!(player.request(), Some(Request::Play(PathBuf::from("/home/steamos/Tables/A/A.vpx"))));
        assert_eq!(player.request(), Some(Request::Close));
        assert_eq!(player.request(), None);
    }

    #[test]
    fn a_full_ring_drops_what_doesnt_fit() {
        let player = Link::create(&name("full"), 640, 480, 100).unwrap();
        let _panel = Link::open(player.name()).unwrap();
        for _ in 0..INPUTS {
            assert!(player.send(Input::Leave));
        }
        assert!(!player.send(Input::Leave));
    }

    #[test]
    fn the_player_gets_the_newest_whole_frame() {
        let player = Link::create(&name("frames"), 640, 480, 100).unwrap();
        let panel = Link::open(player.name()).unwrap();
        assert!(player.take_frame().is_none());
        panel.draw(4, 2, 1, |p| p.fill(1));
        panel.draw(4, 2, 2, |p| p.fill(2));
        let f = player.take_frame().unwrap();
        assert_eq!((f.width, f.height, f.number), (4, 2, 2));
        assert!(f.pixels.iter().all(|&b| b == 2));
        assert!(player.take_frame().is_none());
        panel.draw(4, 2, 3, |p| p.fill(3));
        let f = player.take_frame().unwrap();
        assert_eq!(f.number, 3);
        assert!(f.pixels.iter().all(|&b| b == 3));
    }

    #[test]
    fn the_name_goes_with_the_player() {
        let n = name("gone");
        drop(Link::create(&n, 640, 480, 100).unwrap());
        assert!(Link::open(&n).is_err());
    }
}
