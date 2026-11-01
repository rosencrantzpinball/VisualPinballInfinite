//! SDL's sound output, for the page's Web Audio: one output, stereo at 48 kHz (the page's
//! AudioContext plays at the rate the player mixes at). Each frame the page asks for what it's
//! short of (`pump`); every stream's callback is asked for as much, as SDL's audio thread would
//! ask it, and what the streams put is added together for the page.

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void};

use sdl3_sys::everything::*;

/// The output, as the page plays it.
const RATE: i32 = 48_000;
const CHANNELS: usize = 2;
const DEVICE: SDL_AudioDeviceID = SDL_AudioDeviceID(2);

struct Stream {
    channels: usize,
    callback: SDL_AudioStreamCallback,
    userdata: usize,
    playing: bool,
    /// What the callback put and the page hasn't had yet, the stream's own channels interleaved.
    queued: Vec<f32>,
}

thread_local! {
    static STREAMS: RefCell<Vec<*mut Stream>> = const { RefCell::new(Vec::new()) };
    static OUT: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetAudioPlaybackDevices(count: *mut c_int) -> *mut SDL_AudioDeviceID {
    if !count.is_null() {
        // SAFETY: the caller's out-parameter
        unsafe { *count = 1 };
    }
    super::c_alloc(&[DEVICE, SDL_AudioDeviceID(0)])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetAudioDeviceName(_devid: SDL_AudioDeviceID) -> *const c_char {
    c"the browser's sound".as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetAudioDeviceFormat(_devid: SDL_AudioDeviceID, spec: *mut SDL_AudioSpec, sample_frames: *mut c_int) -> bool {
    // SAFETY: the caller's out-parameters, each may be null
    unsafe {
        if !spec.is_null() {
            *spec = SDL_AudioSpec { format: SDL_AUDIO_F32, channels: CHANNELS as c_int, freq: RATE };
        }
        if !sample_frames.is_null() {
            *sample_frames = 512;
        }
    }
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_OpenAudioDeviceStream(_devid: SDL_AudioDeviceID, spec: *const SDL_AudioSpec, callback: SDL_AudioStreamCallback, userdata: *mut c_void) -> *mut SDL_AudioStream {
    // SAFETY: the caller's spec
    let channels = unsafe { spec.as_ref() }.map_or(CHANNELS, |s| s.channels.clamp(1, 8) as usize);
    let stream = Box::into_raw(Box::new(Stream { channels, callback, userdata: userdata as usize, playing: false, queued: Vec::new() }));
    STREAMS.with(|s| s.borrow_mut().push(stream));
    stream.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_ResumeAudioStreamDevice(stream: *mut SDL_AudioStream) -> bool {
    // SAFETY: a stream of ours
    match unsafe { stream.cast::<Stream>().as_mut() } {
        Some(s) => {
            s.playing = true;
            true
        }
        None => false,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_PauseAudioStreamDevice(stream: *mut SDL_AudioStream) -> bool {
    // SAFETY: a stream of ours
    match unsafe { stream.cast::<Stream>().as_mut() } {
        Some(s) => {
            s.playing = false;
            true
        }
        None => false,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_PutAudioStreamData(stream: *mut SDL_AudioStream, buf: *const c_void, len: c_int) -> bool {
    // SAFETY: a stream of ours; the caller's samples, `len` bytes of 32-bit floats
    unsafe {
        let Some(s) = stream.cast::<Stream>().as_mut() else { return false };
        if !buf.is_null() && len > 0 {
            s.queued.extend_from_slice(std::slice::from_raw_parts(buf.cast::<f32>(), len as usize / 4));
        }
    }
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_DestroyAudioStream(stream: *mut SDL_AudioStream) {
    let stream = stream.cast::<Stream>();
    let ours = STREAMS.with(|s| {
        let mut s = s.borrow_mut();
        let before = s.len();
        s.retain(|&p| p != stream);
        s.len() != before
    });
    if ours {
        // SAFETY: ours, destroyed once; its callback is never called again
        drop(unsafe { Box::from_raw(stream) });
    }
}

/// `frames` frames of the output, stereo interleaved: every playing stream's callback asked for
/// that much, and what they put added together (a stream of more channels than two folded into
/// them: the left ones left, the right ones right). The samples stay where they are until the next
/// call.
pub fn pump(frames: usize) -> *const f32 {
    let streams = STREAMS.with(|s| s.borrow().clone());
    OUT.with(|out| {
        let mut out = out.borrow_mut();
        out.clear();
        out.resize(frames * CHANNELS, 0.0);
        for stream in streams {
            // SAFETY: a stream of ours, alive while it's in the list; the callback may put data
            // into it, nothing else
            unsafe {
                let (playing, channels, callback, userdata) = {
                    let s = &*stream;
                    (s.playing, s.channels, s.callback, s.userdata)
                };
                if !playing {
                    continue;
                }
                let have = (*stream).queued.len() / channels;
                if have < frames
                    && let Some(f) = callback
                {
                    let bytes = ((frames - have) * channels * 4) as c_int;
                    f(userdata as *mut c_void, stream.cast(), bytes, bytes);
                }
                let s = &mut *stream;
                let n = (s.queued.len() / s.channels).min(frames);
                for k in 0..n {
                    let frame = &s.queued[k * s.channels..(k + 1) * s.channels];
                    let (l, r) = match s.channels {
                        1 => (frame[0], frame[0]),
                        2 => (frame[0], frame[1]),
                        // 5.1 and 7.1: front, centre half to each, back and sides to their side
                        _ => {
                            let c = frame.get(2).copied().unwrap_or(0.0) * 0.5;
                            let (mut l, mut r) = (frame[0] + c, frame[1] + c);
                            for (j, v) in frame.iter().enumerate().skip(4) {
                                if j % 2 == 0 { l += v * 0.7 } else { r += v * 0.7 }
                            }
                            (l, r)
                        }
                    };
                    out[k * 2] += l;
                    out[k * 2 + 1] += r;
                }
                s.queued.drain(..n * s.channels);
            }
        }
        out.as_ptr()
    })
}
