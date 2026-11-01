//! The part of SDL_ttf the player uses (`player::pup::fonts`: the pause menu's text, PinUP labels'),
//! for the browser, whose program carries no FreeType or HarfBuzz: fonts read and drawn in Rust by
//! ab_glyph, in SDL_ttf's place. Sizes, ascents, heights and line skips as SDL_ttf works them out
//! from the font's own metrics; text laid out glyph after glyph with the font's kerning pairs (no
//! shaping: ligatures and the like, which the menu's Latin text doesn't need), broken at `\n` and
//! wrapped at spaces, each line aligned as asked; italic slanted where the font isn't, underline,
//! and an outline grown around the glyphs. Drawn as SDL_ttf's blended text is: the colour, with the
//! glyphs' coverage for alpha, in an RGBA surface.

use std::ffi::{CString, c_char, c_float, c_int, c_void};

use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont, point};
use sdl3_sys::everything::*;
use sdl3_ttf_sys::ttf::*;

/// A block of memory a font is read from (`SDL_IOFromConstMem`): the player's own bytes, kept.
struct Memory {
    data: &'static [u8],
}

struct Font {
    font: FontRef<'static>,
    /// Font units: per em, and the ascender, descender and line gap (`hhea`'s, as FreeType reads them).
    units: f32,
    ascender: f32,
    descender: f32,
    line_gap: f32,
    family: CString,
    style_name: CString,
    italic_font: bool,
    size: f32,
    style: TTF_FontStyleFlags,
    outline: i32,
    align: TTF_HorizontalAlignment,
}

impl Font {
    /// SDL_ttf's pixels per font unit at the size set (its points at 72 dots an inch).
    fn per_unit(&self) -> f32 {
        self.size / self.units
    }
    fn ascent(&self) -> i32 {
        (self.ascender * self.per_unit()).ceil() as i32
    }
    fn height(&self) -> i32 {
        ((self.ascender - self.descender) * self.per_unit()).ceil() as i32
    }
    fn line_skip(&self) -> i32 {
        ((self.ascender - self.descender + self.line_gap) * self.per_unit()).ceil() as i32
    }
    /// ab_glyph's scale for the size: its height (ascent less descent) in pixels.
    fn scale(&self) -> PxScale {
        let height = self.font.height_unscaled() * self.per_unit();
        PxScale::from(height.max(0.1))
    }
}

fn font<'a>(f: *const TTF_Font) -> Option<&'a mut Font> {
    // SAFETY: the player passes back the fonts we opened, until it closes them
    unsafe { f.cast::<Font>().cast_mut().as_mut() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_Init() -> bool {
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_IOFromConstMem(mem: *const c_void, size: usize) -> *mut SDL_IOStream {
    if mem.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: the caller's bytes, which outlive the font made from them (`fonts.rs` keeps them)
    let data = unsafe { std::slice::from_raw_parts(mem.cast::<u8>(), size) };
    Box::into_raw(Box::new(Memory { data })).cast()
}

/// The text of a name record (1 the family, 2 the style), as SDL_ttf reads it.
fn name(face: &ttf_parser::Face<'_>, id: u16) -> CString {
    let text = face.names().into_iter().filter(|n| n.name_id == id && n.is_unicode()).find_map(|n| n.to_string()).unwrap_or_default();
    CString::new(text.replace('\0', " ")).unwrap_or_default()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_OpenFontIO(src: *mut SDL_IOStream, closeio: bool, ptsize: c_float) -> *mut TTF_Font {
    if src.is_null() {
        return std::ptr::null_mut();
    }
    let memory = src.cast::<Memory>();
    // SAFETY: a memory block of ours, freed when the font takes it
    let data = unsafe { (*memory).data };
    if closeio {
        // SAFETY: ours, handed over once
        drop(unsafe { Box::from_raw(memory) });
    }
    let (Ok(face), Ok(font)) = (ttf_parser::Face::parse(data, 0), FontRef::try_from_slice(data)) else {
        super::set_error("not a font SDL_ttf's stand-in can read");
        return std::ptr::null_mut();
    };
    let f = Font {
        units: f32::from(face.units_per_em()),
        ascender: f32::from(face.ascender()),
        descender: f32::from(face.descender()),
        line_gap: f32::from(face.line_gap()),
        family: name(&face, 1),
        style_name: name(&face, 2),
        italic_font: face.is_italic(),
        font,
        size: ptsize.max(1.0),
        style: TTF_STYLE_NORMAL,
        outline: 0,
        align: TTF_HORIZONTAL_ALIGN_LEFT,
    };
    Box::into_raw(Box::new(f)).cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_CloseFont(font: *mut TTF_Font) {
    if !font.is_null() {
        // SAFETY: ours, closed once
        drop(unsafe { Box::from_raw(font.cast::<Font>()) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_SetFontSize(f: *mut TTF_Font, ptsize: c_float) -> bool {
    font(f).map(|f| f.size = ptsize.max(1.0)).is_some()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_SetFontStyle(f: *mut TTF_Font, style: TTF_FontStyleFlags) {
    if let Some(f) = font(f) {
        f.style = style;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_SetFontOutline(f: *mut TTF_Font, outline: c_int) -> bool {
    font(f).map(|f| f.outline = outline.clamp(0, 64)).is_some()
}

/// Hinting: none, ab_glyph's outlines as they are.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_SetFontHinting(_f: *mut TTF_Font, _hinting: TTF_HintingFlags) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_SetFontWrapAlignment(f: *mut TTF_Font, align: TTF_HorizontalAlignment) {
    if let Some(f) = font(f) {
        f.align = align;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_GetFontHeight(f: *const TTF_Font) -> c_int {
    font(f).map_or(0, |f| f.height())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_GetFontAscent(f: *const TTF_Font) -> c_int {
    font(f).map_or(0, |f| f.ascent())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_GetFontLineSkip(f: *const TTF_Font) -> c_int {
    font(f).map_or(0, |f| f.line_skip())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_GetFontFamilyName(f: *const TTF_Font) -> *const c_char {
    font(f).map_or(c"".as_ptr(), |f| f.family.as_ptr())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_GetFontStyleName(f: *const TTF_Font) -> *const c_char {
    font(f).map_or(c"".as_ptr(), |f| f.style_name.as_ptr())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_FontHasGlyph(f: *mut TTF_Font, ch: Uint32) -> bool {
    font(f).is_some_and(|f| char::from_u32(ch).is_some_and(|c| f.font.glyph_id(c).0 != 0))
}

/// A line's width in pixels, glyph after glyph with the kerning between them.
fn width(f: &Font, line: &str) -> f32 {
    let scaled = f.font.as_scaled(f.scale());
    let mut x = 0.0;
    let mut prev = None;
    for c in line.chars() {
        let id = f.font.glyph_id(c);
        if let Some(p) = prev {
            x += scaled.kern(p, id);
        }
        x += scaled.h_advance(id);
        prev = Some(id);
    }
    x
}

/// The text's lines: broken at `\n`, and where `wrap` is more than 0, at the last space before a
/// line grows wider than it (a word wider by itself broken where it must), as SDL_ttf wraps.
fn lines(f: &Font, text: &str, wrap: f32) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n').map(|p| p.trim_end_matches('\r')) {
        if wrap <= 0.0 || width(f, paragraph) <= wrap {
            out.push(paragraph.to_string());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if width(f, &candidate) <= wrap || line.is_empty() && width(f, word) <= wrap {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            // a word wider than the line: as many of its letters as fit, then the rest
            let mut rest = word.to_string();
            while width(f, &rest) > wrap && rest.chars().count() > 1 {
                let mut cut = String::new();
                for c in rest.chars() {
                    cut.push(c);
                    if width(f, &cut) > wrap {
                        cut.pop();
                        break;
                    }
                }
                if cut.is_empty() {
                    cut = rest.chars().next().map(String::from).unwrap_or_default();
                }
                rest = rest[cut.len()..].to_string();
                out.push(cut);
            }
            line = rest;
        }
        out.push(line);
    }
    out
}

/// A surface of ours: RGBA bytes in its pixels, as `SDL_PIXELFORMAT_RGBA32` keeps them.
fn surface(w: usize, h: usize, pixels: Vec<u8>) -> *mut SDL_Surface {
    let pixels = Box::into_raw(pixels.into_boxed_slice()).cast::<u8>();
    // SAFETY: plain data, every field set
    let mut s: SDL_Surface = unsafe { std::mem::zeroed() };
    s.format = SDL_PIXELFORMAT_RGBA32;
    (s.w, s.h, s.pitch) = (w as c_int, h as c_int, (w * 4) as c_int);
    s.pixels = pixels.cast();
    s.refcount = 1;
    Box::into_raw(Box::new(s))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn TTF_RenderText_Blended_Wrapped(f: *mut TTF_Font, text: *const c_char, length: usize, fg: SDL_Color, wrap_width: c_int) -> *mut SDL_Surface {
    let Some(f) = font(f) else { return std::ptr::null_mut() };
    if text.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: the caller's text, `length` bytes (0: up to its NUL)
    let bytes = unsafe {
        let n = if length == 0 { std::ffi::CStr::from_ptr(text).to_bytes().len() } else { length };
        std::slice::from_raw_parts(text.cast::<u8>(), n)
    };
    let text = String::from_utf8_lossy(bytes);
    let lines = lines(f, &text, wrap_width as f32);
    let scale = f.scale();
    let scaled = f.font.as_scaled(scale);
    let italic = (f.style & TTF_STYLE_ITALIC) == TTF_STYLE_ITALIC && !f.italic_font;
    let underline = (f.style & TTF_STYLE_UNDERLINE) == TTF_STYLE_UNDERLINE;
    let outline = f.outline.max(0) as usize;
    // a slant of 12 degrees, as FreeType's synthetic italic
    const SLANT: f32 = 0.207;
    let ascent = f.ascent() as f32;
    let slant_room = if italic { (ascent * SLANT).ceil() as usize } else { 0 };
    let widths: Vec<f32> = lines.iter().map(|l| width(f, l)).collect();
    let text_w = widths.iter().copied().fold(0.0f32, f32::max).ceil() as usize;
    let w = (text_w + slant_room + 2 * outline).max(1);
    let skip = f.line_skip().max(1) as usize;
    let h = (f.height().max(1) as usize + skip * lines.len().saturating_sub(1) + 2 * outline).max(1);
    let mut coverage = vec![0f32; w * h];
    for (k, (line, lw)) in lines.iter().zip(&widths).enumerate() {
        let start = if f.align == TTF_HORIZONTAL_ALIGN_CENTER {
            (text_w as f32 - lw) / 2.0
        } else if f.align == TTF_HORIZONTAL_ALIGN_RIGHT {
            text_w as f32 - lw
        } else {
            0.0
        } + outline as f32;
        let baseline = outline as f32 + ascent + (k * skip) as f32;
        let mut x = start;
        let mut prev = None;
        for c in line.chars() {
            let id = f.font.glyph_id(c);
            if let Some(p) = prev {
                x += scaled.kern(p, id);
            }
            let glyph = id.with_scale_and_position(scale, point(x, baseline));
            if let Some(o) = f.font.outline_glyph(glyph) {
                let b = o.px_bounds();
                o.draw(|gx, gy, c| {
                    let py = b.min.y as i32 + gy as i32;
                    let shift = if italic { ((baseline - py as f32) * SLANT).round() as i32 } else { 0 };
                    let px = b.min.x as i32 + gx as i32 + shift;
                    if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                        let at = py as usize * w + px as usize;
                        coverage[at] = (coverage[at] + c).min(1.0);
                    }
                });
            }
            x += scaled.h_advance(id);
            prev = Some(id);
        }
        if underline {
            let thickness = (f.size / 14.0).round().max(1.0) as usize;
            let top = (baseline + (f.size / 10.0).round()) as usize;
            for y in top..(top + thickness).min(h) {
                for x in (start as usize)..((start + lw).ceil() as usize).min(w) {
                    coverage[y * w + x] = 1.0;
                }
            }
        }
    }
    // the outline: the glyphs grown by its width all round, as a stroke around them shows
    if outline > 0 {
        let r = outline as i32;
        let grown: Vec<f32> = (0..w * h)
            .map(|at| {
                let (x, y) = ((at % w) as i32, (at / w) as i32);
                let mut m = 0f32;
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx * dx + dy * dy > r * r + r {
                            continue;
                        }
                        let (sx, sy) = (x + dx, y + dy);
                        if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
                            m = m.max(coverage[sy as usize * w + sx as usize]);
                        }
                    }
                }
                m
            })
            .collect();
        coverage = grown;
    }
    let mut pixels = vec![0u8; w * h * 4];
    for (p, &c) in pixels.chunks_exact_mut(4).zip(&coverage) {
        p.copy_from_slice(&[fg.r, fg.g, fg.b, (c * f32::from(fg.a)).round() as u8]);
    }
    surface(w, h, pixels)
}

/// A copy, in RGBA (ours are already).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_ConvertSurface(s: *mut SDL_Surface, _format: SDL_PixelFormat) -> *mut SDL_Surface {
    // SAFETY: a surface of ours
    let Some(s) = (unsafe { s.as_ref() }) else { return std::ptr::null_mut() };
    let (w, h) = (s.w.max(0) as usize, s.h.max(0) as usize);
    // SAFETY: its pixels, a row of `pitch` bytes for each of its rows
    let pixels = unsafe { std::slice::from_raw_parts(s.pixels.cast::<u8>(), w * h * 4) }.to_vec();
    surface(w, h, pixels)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_LockSurface(_surface: *mut SDL_Surface) -> bool {
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_UnlockSurface(_surface: *mut SDL_Surface) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_DestroySurface(s: *mut SDL_Surface) {
    if s.is_null() {
        return;
    }
    // SAFETY: a surface of ours, destroyed once: its pixels and itself
    unsafe {
        let b = Box::from_raw(s);
        let len = (b.w.max(0) * b.h.max(0) * 4) as usize;
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(b.pixels.cast::<u8>(), len)));
    }
}
