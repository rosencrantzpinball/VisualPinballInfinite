//! A table made smaller to fetch, for the browser (`tools/web/package.sh --slim`): its pictures as WebP and its
//! uncompressed sounds as Ogg Vorbis, written as a new table file; the original stays as it is.
//!
//!   vpx-slim <table.vpx> <out.vpx> [--quality 90] [--ssim 0.95] [--vorbis 0.5] [--unchanged] [--list]
//!
//! Each PNG, BMP or JPEG picture becomes the smallest of: lossy WebP at `--quality` (or 95, when that falls short)
//! if it still matches the original by SSIM (at least `--ssim`, measured on what shows: colour times alpha);
//! lossless WebP (not for a JPEG); or its own file. Lossy WebP keeps transparency exactly, and the colour under a
//! clear texel as well (libwebp's `exact`), which filtering blends into a part's edges. A picture without
//! transparency goes without an alpha channel, so a decoder that goes by the file's header (the browser's, in the
//! player's page) knows it's opaque. Other pictures (OpenEXR, Radiance, TGA, Photoshop, WebP already) keep their
//! files. A WAV sound of 8- or 16-bit samples becomes Ogg Vorbis at libvorbis's quality `--vorbis` (-0.1 to 1),
//! a format the player and VPX already play: unlike MP3's and AAC's, its sounds start without added silence.

use std::num::{NonZeroU32, NonZeroU8};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use md5::{Digest, Md5};
use vpin::vpx;

struct Options {
    quality: f32,
    ssim: f64,
    vorbis: f32,
    /// Written as read, to check the table library's writing.
    unchanged: bool,
    /// Each picture's outcome printed (`--list`).
    list: bool,
}

/// What became of a picture.
#[derive(Clone, Copy)]
enum Outcome {
    Lossy(f32),
    Lossless,
    Kept(&'static str),
}

fn main() {
    if let Err(e) = run() {
        eprintln!("vpx-slim: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut o = Options { quality: 90.0, ssim: 0.95, vorbis: 0.5, unchanged: false, list: false };
    let mut files = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut number = || args.next().and_then(|v| v.parse::<f64>().ok()).ok_or_else(|| format!("{a} wants a number"));
        match a.as_str() {
            "--quality" => o.quality = number()? as f32,
            "--ssim" => o.ssim = number()?,
            "--vorbis" => o.vorbis = number()? as f32,
            "--unchanged" => o.unchanged = true,
            "--list" => o.list = true,
            _ => files.push(a),
        }
    }
    let [input, output] = files.as_slice() else {
        return Err("vpx-slim <table.vpx> <out.vpx> [--quality 90] [--ssim 0.95] [--vorbis 0.5] [--unchanged] [--list]".into());
    };
    let mut table = vpx::read(Path::new(input)).map_err(|e| format!("{input}: {e}"))?;
    if o.unchanged {
        return vpx::write(Path::new(output), &table).map_err(|e| format!("{output}: {e}"));
    }

    // the pictures, on every thread
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |t| t.get()).min(16);
    let mut slimmed: Vec<(usize, Option<Vec<u8>>, Outcome)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(image) = table.images.get(i) else { break };
                        let (bytes, outcome) = slim_picture(image, &o);
                        done.push((i, bytes, outcome));
                    }
                    done
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().expect("a picture's thread")).collect()
    });
    slimmed.sort_by_key(|(i, ..)| *i);
    let (mut before, mut after, mut lossy, mut lossless, mut kept) = (0usize, 0usize, [0usize; 2], 0usize, 0usize);
    for (i, bytes, outcome) in slimmed {
        let image = &mut table.images[i];
        let Some(file) = image.jpeg.as_mut() else { continue };
        before += file.data.len();
        match outcome {
            Outcome::Lossy(q) => lossy[usize::from(q > o.quality)] += 1,
            Outcome::Lossless => lossless += 1,
            Outcome::Kept(_) => kept += 1,
        }
        if o.list {
            let what = match outcome {
                Outcome::Lossy(q) => format!("lossy {q}"),
                Outcome::Lossless => "lossless".into(),
                Outcome::Kept(why) => format!("kept: {why}"),
            };
            println!("  {:>8.1} KB -> {:>8.1} KB  {what}  {}", file.data.len() as f64 / 1024.0, bytes.as_ref().map_or(file.data.len(), Vec::len) as f64 / 1024.0, image.name);
        }
        if let Some(bytes) = bytes {
            image.md5_hash = Some(Md5::digest(&bytes).into());
            file.data = bytes;
            file.path = with_extension(&file.path, "webp");
            image.path = with_extension(&image.path, "webp");
        }
        after += file.data.len();
    }
    let mb = |n: usize| n as f64 / 1048576.0;
    println!(
        "pictures: {} lossy WebP ({} at quality {}, {} at 95), {lossless} lossless WebP, {kept} kept: {:.1} MB -> {:.1} MB",
        lossy[0] + lossy[1],
        lossy[0],
        o.quality,
        lossy[1],
        mb(before),
        mb(after)
    );

    let (mut sounds, mut sound_before, mut sound_after) = (0, 0, 0);
    for sound in &mut table.sounds {
        if let Some((b, a)) = slim_sound(sound, o.vorbis) {
            sounds += 1;
            sound_before += b;
            sound_after += a;
        }
    }
    println!("sounds: {sounds} WAV of {} to Ogg Vorbis: {:.1} MB -> {:.1} MB", table.sounds.len(), mb(sound_before), mb(sound_after));

    vpx::write(Path::new(output), &table).map_err(|e| format!("{output}: {e}"))?;
    let size = |p: &str| std::fs::metadata(p).map_or(0, |m| m.len() as usize);
    println!("table: {:.1} MB -> {:.1} MB", mb(size(input)), mb(size(output)));
    Ok(())
}

/// A picture's new file, None to keep its own, and what became of it.
fn slim_picture(image: &vpx::image::ImageData, o: &Options) -> (Option<Vec<u8>>, Outcome) {
    let Some(file) = image.jpeg.as_ref().map(|j| j.data.as_slice()) else { return (None, Outcome::Kept("no file")) };
    let (format, jpeg) = if file.starts_with(b"\x89PNG\r\n\x1a\n") {
        (image::ImageFormat::Png, false)
    } else if file.starts_with(&[0xFF, 0xD8, 0xFF]) {
        (image::ImageFormat::Jpeg, true)
    } else if file.starts_with(b"BM") {
        (image::ImageFormat::Bmp, false)
    } else {
        return (None, Outcome::Kept("another format"));
    };
    let Ok(decoded) = image::load_from_memory_with_format(file, format) else { return (None, Outcome::Kept("undecodable")) };
    // the player keeps every picture but one of light past white in 8 bits a channel (`Image::from_decoded`, as
    // VPX does), a 16-bit one's texels made 8-bit as here: only floats keep their files
    if matches!(decoded.color(), image::ColorType::Rgb32F | image::ColorType::Rgba32F) {
        return (None, Outcome::Kept("floats"));
    }
    let rgba = decoded.into_rgba8();
    let (w, h) = rgba.dimensions();
    let alpha = rgba.pixels().any(|p| p[3] < 255);
    let rgb: Vec<u8>;
    let texels: &[u8] = if alpha {
        rgba.as_raw()
    } else {
        rgb = rgba.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
        &rgb
    };
    let mut best: (Option<Vec<u8>>, usize, Outcome) = (None, file.len(), Outcome::Kept(if jpeg { "lossy WebP not close enough" } else { "smallest as it is" }));
    if !jpeg {
        let l = lossless(texels, alpha, w, h);
        if l.len() < best.1 {
            best = (Some(l.clone()), l.len(), Outcome::Lossless);
        }
    }
    // SSIM looks at 8 × 8 windows: a smaller picture goes lossless or stays
    if w >= 8 && h >= 8 {
        for q in [o.quality, o.quality.max(95.0)] {
            let bytes = lossy(texels, alpha, w, h, q);
            let Some(back) = decode(&bytes) else { break };
            if ssim(rgba.as_raw(), &back, w as usize, h as usize) >= o.ssim {
                if bytes.len() < best.1 {
                    best = (Some(bytes.clone()), bytes.len(), Outcome::Lossy(q));
                }
                break;
            }
            if q >= 95.0 {
                break;
            }
        }
    }
    (best.0, best.2)
}

fn encoder(texels: &[u8], alpha: bool, w: u32, h: u32) -> webp::Encoder<'_> {
    if alpha { webp::Encoder::from_rgba(texels, w, h) } else { webp::Encoder::from_rgb(texels, w, h) }
}

fn lossless(texels: &[u8], alpha: bool, w: u32, h: u32) -> Vec<u8> {
    let mut c = webp::WebPConfig::new().expect("libwebp's settings");
    // SAFETY: libwebp's own settings, filled in by libwebp
    unsafe { libwebp_sys::WebPConfigLosslessPreset(&mut c, 6) };
    c.exact = 1;
    encoder(texels, alpha, w, h).encode_advanced(&c).expect("libwebp encodes").to_vec()
}

fn lossy(texels: &[u8], alpha: bool, w: u32, h: u32, quality: f32) -> Vec<u8> {
    let mut c = webp::WebPConfig::new().expect("libwebp's settings");
    c.lossless = 0;
    c.quality = quality;
    c.method = 6;
    c.exact = 1;
    c.use_sharp_yuv = 1;
    c.alpha_compression = 1;
    c.alpha_quality = 100;
    encoder(texels, alpha, w, h).encode_advanced(&c).expect("libwebp encodes").to_vec()
}

/// A WebP file's texels as RGBA.
fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
    let image = webp::Decoder::new(bytes).decode()?;
    Some(if image.is_alpha() { image.to_vec() } else { image.chunks_exact(3).flat_map(|t| [t[0], t[1], t[2], 255]).collect() })
}

/// SSIM of what two RGBA pictures show (colour times alpha): red, green and blue each over 8 × 8 windows 4 texels
/// apart, averaged.
fn ssim(a: &[u8], b: &[u8], w: usize, h: usize) -> f64 {
    let shown = |p: &[u8]| -> Vec<[f32; 3]> {
        p.chunks_exact(4)
            .map(|t| {
                let k = f32::from(t[3]) / 255.0;
                [f32::from(t[0]) * k, f32::from(t[1]) * k, f32::from(t[2]) * k]
            })
            .collect()
    };
    let (x, y) = (shown(a), shown(b));
    let (c1, c2) = (6.5025f64, 58.5225f64);
    let (mut sum, mut windows) = (0f64, 0usize);
    for wy in (0..=h - 8).step_by(4) {
        for wx in (0..=w - 8).step_by(4) {
            for ch in 0..3 {
                let (mut sx, mut sy, mut sxx, mut syy, mut sxy) = (0f64, 0f64, 0f64, 0f64, 0f64);
                for yy in wy..wy + 8 {
                    for xx in wx..wx + 8 {
                        let (p, q) = (f64::from(x[yy * w + xx][ch]), f64::from(y[yy * w + xx][ch]));
                        sx += p;
                        sy += q;
                        sxx += p * p;
                        syy += q * q;
                        sxy += p * q;
                    }
                }
                let (mx, my) = (sx / 64.0, sy / 64.0);
                let (vx, vy, cov) = (sxx / 64.0 - mx * mx, syy / 64.0 - my * my, sxy / 64.0 - mx * my);
                sum += ((2.0 * mx * my + c1) * (2.0 * cov + c2)) / ((mx * mx + my * my + c1) * (vx + vy + c2));
                windows += 1;
            }
        }
    }
    sum / windows as f64
}

/// A WAV sound of 8- or 16-bit samples as Ogg Vorbis, when that's smaller: its size before and after.
fn slim_sound(sound: &mut vpx::sound::SoundData, quality: f32) -> Option<(usize, usize)> {
    let name = &sound.path[sound.path.rfind(['/', '\\']).map_or(0, |i| i + 1)..];
    let wav = name.rfind('.').is_none_or(|d| name[d + 1..].eq_ignore_ascii_case("wav"));
    let f = &sound.wave_form;
    if !wav || f.format_tag != 1 || !(f.bits_per_sample == 8 || f.bits_per_sample == 16) {
        return None;
    }
    let channels = usize::from(f.channels);
    let samples: Vec<f32> = if f.bits_per_sample == 16 {
        sound.data.chunks_exact(2).map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0).collect()
    } else {
        sound.data.iter().map(|&v| (f32::from(v) - 128.0) / 128.0).collect()
    };
    let frames = samples.len() / channels.max(1);
    if frames == 0 {
        return None;
    }
    let planes: Vec<Vec<f32>> = (0..channels).map(|c| (0..frames).map(|k| samples[k * channels + c]).collect()).collect();
    let mut builder = vorbis_rs::VorbisEncoderBuilder::new_with_serial(NonZeroU32::new(f.samples_per_sec)?, NonZeroU8::new(u8::try_from(channels).ok()?)?, Vec::new(), 1);
    builder.bitrate_management_strategy(vorbis_rs::VorbisBitrateManagementStrategy::QualityVbr { target_quality: quality });
    let mut encoder = builder.build().ok()?;
    for start in (0..frames).step_by(1024) {
        let end = (start + 1024).min(frames);
        let block: Vec<&[f32]> = planes.iter().map(|p| &p[start..end]).collect();
        encoder.encode_audio_block(&block).ok()?;
    }
    let ogg = encoder.finish().ok()?;
    if ogg.len() >= sound.data.len() {
        return None;
    }
    let before = sound.data.len();
    sound.data = ogg;
    sound.path = with_extension(&sound.path, "ogg");
    Some((before, sound.data.len()))
}

/// `path` with its file's extension `ext` (Windows' separators too, as tables carry them).
fn with_extension(path: &str, ext: &str) -> String {
    let name = path.rfind(['/', '\\']).map_or(0, |i| i + 1);
    match path[name..].rfind('.') {
        Some(dot) => format!("{}.{ext}", &path[..name + dot]),
        None => format!("{path}.{ext}"),
    }
}
