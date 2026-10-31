//! The page's requests, answered here with no network, under `http://tauri.localhost/`, where the
//! desktop app serves its screens on Windows: the screens' files (`ui/`, the shim put before their
//! scripts in `index.html`), the commands (`/__vpi/ipc/<command>`, answered on a thread of their own),
//! and the tables' media (`/__vpi/media/<table>/<number>`, by what the catalog names, never by a path
//! the page gives; byte ranges, which videos seek with).
use std::fs::File;
use std::io::{Read as _, Seek, SeekFrom};
use std::os::raw::c_int;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use cef::*;
use serde_json::Value;

use crate::browser::{panel, string};

const SHIM: &str = include_str!("shim.js");
const HOST: &str = "tauri.localhost";

/// The page's hosts, answered here from now on.
pub fn register() {
    let mut factory = Factory::new();
    register_scheme_handler_factory(Some(&CefString::from("http")), Some(&CefString::from(HOST)), Some(&mut factory));
}

/// What a request asked for, taken as it came in.
struct Asked {
    url: String,
    body: Vec<u8>,
    range: Option<String>,
}

enum Body {
    Bytes { bytes: Vec<u8>, at: usize },
    File { file: File, left: u64 },
}

struct Answer {
    status: c_int,
    mime: String,
    headers: Vec<(&'static str, String)>,
    length: u64,
    body: Body,
}

impl Answer {
    fn bytes(status: c_int, mime: &str, bytes: Vec<u8>) -> Answer {
        Answer { status, mime: mime.to_string(), headers: vec![("Cache-Control", "no-store".into())], length: bytes.len() as u64, body: Body::Bytes { bytes, at: 0 } }
    }

    fn not_found() -> Answer {
        Answer::bytes(404, "text/plain", b"not here".to_vec())
    }

    /// A file, or the part of it a `Range` asks for.
    fn file(path: &Path, range: Option<&str>) -> Answer {
        let Ok(mut file) = File::open(path) else { return Answer::not_found() };
        let total = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mime = mime_of(path).to_string();
        let mut headers = vec![("Accept-Ranges", "bytes".to_string())];
        if let Some((start, end)) = range.and_then(|r| byte_range(r, total)) {
            if file.seek(SeekFrom::Start(start)).is_err() {
                return Answer::not_found();
            }
            headers.push(("Content-Range", format!("bytes {start}-{end}/{total}")));
            let length = end - start + 1;
            return Answer { status: 206, mime, headers, length, body: Body::File { file, left: length } };
        }
        Answer { status: 200, mime, headers, length: total, body: Body::File { file, left: total } }
    }

    fn read_into(&mut self, out: &mut [u8]) -> usize {
        match &mut self.body {
            Body::Bytes { bytes, at } => {
                let n = out.len().min(bytes.len() - *at);
                out[..n].copy_from_slice(&bytes[*at..*at + n]);
                *at += n;
                n
            }
            Body::File { file, left } => {
                let want = out.len().min(usize::try_from(*left).unwrap_or(usize::MAX));
                let n = file.read(&mut out[..want]).unwrap_or(0);
                *left -= n as u64;
                n
            }
        }
    }

    fn skip(&mut self, n: u64) -> u64 {
        match &mut self.body {
            Body::Bytes { bytes, at } => {
                let k = (n as usize).min(bytes.len() - *at);
                *at += k;
                k as u64
            }
            Body::File { file, left } => {
                let k = n.min(*left);
                if file.seek(SeekFrom::Current(k as i64)).is_err() {
                    return 0;
                }
                *left -= k;
                k
            }
        }
    }
}

/// `bytes=start-end`, `bytes=start-` or `bytes=-suffix` within `total` bytes, as first and last byte.
fn byte_range(header: &str, total: u64) -> Option<(u64, u64)> {
    let spec = header.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    if total == 0 {
        return None;
    }
    let (start, end) = match (a.trim(), b.trim()) {
        ("", suffix) => {
            let n: u64 = suffix.parse().ok()?;
            (total.saturating_sub(n), total - 1)
        }
        (s, "") => (s.parse().ok()?, total - 1),
        (s, e) => (s.parse().ok()?, e.parse::<u64>().ok()?.min(total - 1)),
    };
    (start <= end && start < total).then_some((start, end))
}

fn mime_of(path: &Path) -> &'static str {
    match path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("html" | "htm") => "text/html",
        Some("js" | "mjs") => "text/javascript",
        Some("css") => "text/css",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png" | "apng") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("mp4" | "m4v" | "f4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        Some("mov") => "video/quicktime",
        Some("mp3") => "audio/mpeg",
        Some("m4a" | "aac") => "audio/mp4",
        Some("ogg") => "audio/ogg",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    }
}

/// `%xx` in a URL's path as the bytes they stand for.
fn unescape(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A URL's path on our host, its query and fragment left off; none for another host.
fn path_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://")?.strip_prefix(HOST)?;
    let path = rest.split(['?', '#']).next().unwrap_or("");
    Some(unescape(if path.is_empty() { "/" } else { path }))
}

/// A file of the screens', within `ui/` and nowhere else; `index.html` with the shim before its scripts.
fn ui_file(ui: &Path, path: &str) -> Answer {
    let relative = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
    let within = Path::new(relative);
    if within.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Answer::not_found();
    }
    let file: PathBuf = ui.join(within);
    if relative != "index.html" {
        return Answer::file(&file, None);
    }
    let Ok(html) = std::fs::read_to_string(&file) else {
        return Answer::bytes(404, "text/plain", format!("the screens aren't built: no {}", file.display()).into_bytes());
    };
    let tag = "<script src=\"/__vpi/shim.js\"></script>";
    let html = match html.find("<head>") {
        Some(at) => format!("{}{tag}{}", &html[..at + 6], &html[at + 6..]),
        None => format!("{tag}{html}"),
    };
    Answer::bytes(200, "text/html", html.into_bytes())
}

/// A command's answer: its reply as JSON, or what went wrong in words (which `invoke` rejects with).
fn command(name: &str, body: &[u8]) -> Answer {
    let args: Value = if body.is_empty() { Value::Null } else { serde_json::from_slice(body).unwrap_or(Value::Null) };
    match panel().commands.run(name, &args) {
        Ok(reply) => Answer::bytes(200, "application/json", serde_json::to_vec(&reply).unwrap_or_default()),
        Err(e) => Answer::bytes(400, "text/plain", e.into_bytes()),
    }
}

fn answer(asked: &Asked) -> Answer {
    let Some(path) = path_of(&asked.url) else { return Answer::not_found() };
    if let Some(name) = path.strip_prefix("/__vpi/ipc/") {
        return command(name, &asked.body);
    }
    if path == "/__vpi/shim.js" {
        return Answer::bytes(200, "text/javascript", SHIM.as_bytes().to_vec());
    }
    if let Some(rest) = path.strip_prefix("/__vpi/media/") {
        let file = rest.split_once('/').and_then(|(id, n)| {
            let catalog = panel().commands.catalog.lock().unwrap_or_else(|e| e.into_inner());
            catalog.media_file(id, n.parse().ok()?).map(Path::to_path_buf)
        });
        return file.map_or_else(Answer::not_found, |f| Answer::file(&f, asked.range.as_deref()));
    }
    ui_file(&panel().options.ui, &path)
}

/// A request and its answer, once it has one.
struct Exchange {
    asked: Asked,
    answer: Option<Answer>,
}

/// CEF's callback, handed to the thread that answers a command. CEF's callbacks may be run on any
/// thread (`cef_callback_t`).
struct Later(Callback);
// SAFETY: CEF's reference-counted objects count across threads, and a callback may run on any
unsafe impl Send for Later {}

wrap_resource_handler! {
    struct Reply {
        exchange: Arc<Mutex<Exchange>>,
    }

    impl ResourceHandler {
        fn open(&self, _request: Option<&mut Request>, handle_request: Option<&mut c_int>, callback: Option<&mut Callback>) -> c_int {
            let Some(handle) = handle_request else { return 0 };
            let is_command = self.exchange.lock().unwrap_or_else(|e| e.into_inner()).asked.url.contains("/__vpi/ipc/");
            if is_command && let Some(callback) = callback {
                // a command may read files: answered on a thread of its own, CEF told when
                *handle = 0;
                let exchange = self.exchange.clone();
                let later = Later(callback.clone());
                std::thread::spawn(move || {
                    let later = later;
                    let mut x = exchange.lock().unwrap_or_else(|e| e.into_inner());
                    x.answer = Some(answer(&x.asked));
                    drop(x);
                    later.0.cont();
                });
                return 1;
            }
            let mut x = self.exchange.lock().unwrap_or_else(|e| e.into_inner());
            x.answer = Some(answer(&x.asked));
            *handle = 1;
            1
        }

        fn response_headers(&self, response: Option<&mut Response>, response_length: Option<&mut i64>, _redirect_url: Option<&mut CefString>) {
            let x = self.exchange.lock().unwrap_or_else(|e| e.into_inner());
            let Some(a) = &x.answer else { return };
            if let Some(r) = response {
                r.set_status(a.status);
                r.set_mime_type(Some(&CefString::from(a.mime.as_str())));
                for (name, value) in &a.headers {
                    r.set_header_by_name(Some(&CefString::from(*name)), Some(&CefString::from(value.as_str())), 1);
                }
            }
            if let Some(length) = response_length {
                *length = a.length as i64;
            }
        }

        fn read(&self, data_out: *mut u8, bytes_to_read: c_int, bytes_read: Option<&mut c_int>, _callback: Option<&mut ResourceReadCallback>) -> c_int {
            let mut x = self.exchange.lock().unwrap_or_else(|e| e.into_inner());
            let n = match (&mut x.answer, data_out.is_null(), bytes_to_read > 0) {
                (Some(a), false, true) => {
                    // SAFETY: CEF's buffer of `bytes_to_read` bytes, ours until we return
                    let out = unsafe { std::slice::from_raw_parts_mut(data_out, bytes_to_read as usize) };
                    a.read_into(out)
                }
                _ => 0,
            };
            if let Some(read) = bytes_read {
                *read = n as c_int;
            }
            (n > 0) as c_int
        }

        fn skip(&self, bytes_to_skip: i64, bytes_skipped: Option<&mut i64>, _callback: Option<&mut ResourceSkipCallback>) -> c_int {
            let mut x = self.exchange.lock().unwrap_or_else(|e| e.into_inner());
            let n = x.answer.as_mut().map_or(0, |a| a.skip(bytes_to_skip.max(0) as u64));
            if let Some(skipped) = bytes_skipped {
                *skipped = n as i64;
            }
            (n > 0) as c_int
        }

        fn cancel(&self) {}
    }
}

/// A request's body, as it came.
fn body_of(request: &Request) -> Vec<u8> {
    let Some(data) = request.post_data() else { return Vec::new() };
    let mut elements: Vec<Option<PostDataElement>> = (0..data.element_count()).map(|_| None).collect();
    data.elements(Some(&mut elements));
    let mut body = Vec::new();
    for element in elements.iter().flatten() {
        let n = element.bytes_count();
        let at = body.len();
        body.resize(at + n, 0);
        let got = element.bytes(n, body[at..].as_mut_ptr());
        body.truncate(at + got);
    }
    body
}

wrap_scheme_handler_factory! {
    struct Factory;

    impl SchemeHandlerFactory {
        fn create(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _scheme_name: Option<&CefString>, request: Option<&mut Request>) -> Option<ResourceHandler> {
            let request = request?;
            let range = string(&request.header_by_name(Some(&CefString::from("Range"))));
            let asked = Asked { url: string(&request.url()), body: body_of(request), range: (!range.is_empty()).then_some(range) };
            Some(Reply::new(Arc::new(Mutex::new(Exchange { asked, answer: None }))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(byte_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(byte_range("bytes=900-", 1000), Some((900, 999)));
        assert_eq!(byte_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(byte_range("bytes=500-5000", 1000), Some((500, 999)));
        assert_eq!(byte_range("bytes=1000-", 1000), None);
        assert_eq!(byte_range("items=0-1", 1000), None);
    }

    #[test]
    fn paths() {
        assert_eq!(path_of("http://tauri.localhost/index.html#headset").as_deref(), Some("/index.html"));
        assert_eq!(path_of("http://tauri.localhost").as_deref(), Some("/"));
        assert_eq!(path_of("http://tauri.localhost/__vpi/ipc/set%5Frating?x=1").as_deref(), Some("/__vpi/ipc/set_rating"));
        assert_eq!(path_of("http://example.com/"), None);
    }

    #[test]
    fn the_screens_stay_in_their_folder() {
        let ui = Path::new("/nowhere");
        assert_eq!(ui_file(ui, "/../../etc/passwd").status, 404);
    }
}
