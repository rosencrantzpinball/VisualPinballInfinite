//! `vpi-panel`: the app's screens in the headset (PLAN decision 20). The CEF built for the Frame
//! (`tools/steam-frame/cef`) draws the app's own Svelte screens (`app/ui`, built into `ui/` beside this
//! program, in their headset layout) off-screen; the player, which starts this program, shows the
//! pictures as a panel in the room and passes its pointer and keys back, all through the link it makes
//! (`panel-link`). The screens' commands are answered here, from the tables folder (`catalog`), but
//! play, which the player does.
//!
//! ```text
//! vpi-panel --link <name> [--library <folder>] [--back <table>] [--ui <folder>] [--data <folder>] [--url <url>]
//! vpi-panel --shot <picture.png> [--at <ms>[,<ms>...]] [--moves|--clicks <ms>:<x>,<y>[;...]] [--size <w>x<h>] [--scale <percent>] [...]
//! ```
//!
//! `--library` is VPX's layout's folder (`Tables` in it; the Frame's `~/Documents/VPinballX`), `--data`
//! where the panel keeps what it keeps (CEF's cache, the user's marks; the player's data folder's
//! `panel`), `--back` the table the player goes back to, which the bar's "Back to" names (none: no such
//! button). `--shot` is for checks without a player: the page drawn alone, its picture saved after
//! `--at` ms (3000; at several times, each as `<name>-<ms>.png`), the pointer moved to the page's pixels
//! given at the times given (`--moves`), or clicked there (`--clicks`), then the program ends. CEF's own processes start this program again, and end in
//! `cef::execute_process` before any of this is read.
// a table's view, the app's `TableView`, is one `json!` deeper than the default allows
#![recursion_limit = "256"]

mod catalog;
mod commands;
mod screens;
/// A table's own settings, as the app's table page keeps them: the app's own module, so the
/// headset's settings window reads and writes them as the desktop's does.
#[path = "../../../app/src/table_settings.rs"]
#[allow(dead_code)]
mod table_settings;

#[cfg(all(target_os = "linux", feature = "cef"))]
mod browser;
#[cfg(all(target_os = "linux", feature = "cef"))]
mod serve;

use std::path::PathBuf;
use std::process::ExitCode;

/// The page the panel opens: the app's screens as the desktop app serves them on Windows, in their
/// headset layout.
pub const START_URL: &str = "http://tauri.localhost/index.html#headset";

#[derive(Clone, Debug)]
pub struct Options {
    pub link: Option<String>,
    pub library: PathBuf,
    /// The table the player goes back to (`Back to …`), if any.
    pub back: Option<PathBuf>,
    pub ui: PathBuf,
    pub data: PathBuf,
    pub url: String,
    pub shot: Option<PathBuf>,
    /// When the pictures are taken (`--shot`), ms from the start.
    pub shot_at_ms: Vec<u64>,
    /// The pointer moved to these points of the page's (its pixels) at these times, ms, and there
    /// clicked or not (`--shot`'s checks).
    pub moves: Vec<(u64, f32, f32, bool)>,
    /// The page's size and scale when there's no link to say them.
    pub size: (u32, u32),
    pub scale: u32,
}

impl Options {
    pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
        let here = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)).unwrap_or_default();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let mut o = Options {
            link: None,
            library: home.join("Documents").join("VPinballX"),
            back: None,
            ui: here.join("ui"),
            data: vpi_settings::data_folder().unwrap_or_else(|| home.join(".local/share/VisualPinballInfinite")).join("panel"),
            url: START_URL.to_string(),
            shot: None,
            shot_at_ms: vec![3000],
            moves: Vec::new(),
            size: (1600, 1000),
            scale: 125,
        };
        let value = |args: &mut dyn Iterator<Item = String>, name: &str| args.next().ok_or(format!("{name} needs a value"));
        while let Some(a) = args.next() {
            match a.as_str() {
                "--link" => o.link = Some(value(&mut args, &a)?),
                "--library" => o.library = PathBuf::from(value(&mut args, &a)?),
                "--back" => o.back = Some(PathBuf::from(value(&mut args, &a)?)),
                "--ui" => o.ui = PathBuf::from(value(&mut args, &a)?),
                "--data" => o.data = PathBuf::from(value(&mut args, &a)?),
                "--url" => o.url = value(&mut args, &a)?,
                "--shot" => o.shot = Some(PathBuf::from(value(&mut args, &a)?)),
                "--at" => {
                    let v = value(&mut args, &a)?;
                    o.shot_at_ms = v.split(',').map(|t| t.trim().parse()).collect::<Result<_, _>>().map_err(|_| "--at takes milliseconds")?;
                    o.shot_at_ms.sort_unstable();
                }
                "--moves" | "--clicks" => {
                    let v = value(&mut args, &a)?;
                    let click = a == "--clicks";
                    let point = |m: &str| -> Option<(u64, f32, f32, bool)> {
                        let (at, xy) = m.split_once(':')?;
                        let (x, y) = xy.split_once(',')?;
                        Some((at.trim().parse().ok()?, x.trim().parse().ok()?, y.trim().parse().ok()?, click))
                    };
                    let points: Vec<_> = v.split(';').map(|m| point(m).ok_or(format!("{a} takes <ms>:<x>,<y>;..."))).collect::<Result<_, _>>()?;
                    o.moves.extend(points);
                    o.moves.sort_by_key(|m| m.0);
                }
                "--size" => {
                    let v = value(&mut args, &a)?;
                    let (w, h) = v.split_once('x').ok_or("--size takes <w>x<h>")?;
                    o.size = (w.parse().map_err(|_| "--size's width")?, h.parse().map_err(|_| "--size's height")?);
                }
                "--scale" => o.scale = value(&mut args, &a)?.parse().map_err(|_| "--scale takes a percentage")?,
                // what CEF adds or reads itself
                s if s.starts_with("--") => {}
                other => return Err(format!("{other}: not an option of vpi-panel's")),
            }
        }
        if o.link.is_none() && o.shot.is_none() {
            return Err("vpi-panel --link <name> (the player starts it), or --shot <picture.png> for a check".into());
        }
        Ok(o)
    }
}

fn main() -> ExitCode {
    #[cfg(all(target_os = "linux", feature = "cef"))]
    {
        browser::main()
    }
    #[cfg(not(all(target_os = "linux", feature = "cef")))]
    {
        eprintln!("vpi-panel: built without CEF (the panel-host crate's cef feature, on Linux)");
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn options() {
        let o = parse(&["--link", "vpi-panel-7", "--library", "/t", "--back", "/t/Tables/a.vpx", "--size", "1280x800", "--scale", "100"]).unwrap();
        assert_eq!(o.link.as_deref(), Some("vpi-panel-7"));
        assert_eq!(o.library, PathBuf::from("/t"));
        assert_eq!(o.back, Some(PathBuf::from("/t/Tables/a.vpx")));
        assert_eq!((o.size, o.scale), ((1280, 800), 100));
        let o = parse(&["--shot", "/p.png", "--at", "9000,3000", "--moves", "2000:10,20;1000:5.5,6", "--clicks", "1500:1,2"]).unwrap();
        assert_eq!(o.shot_at_ms, vec![3000, 9000]);
        assert_eq!(o.moves, vec![(1000, 5.5, 6.0, false), (1500, 1.0, 2.0, true), (2000, 10.0, 20.0, false)]);
        assert!(parse(&[]).is_err());
        assert!(parse(&["stray"]).is_err());
        // CEF's own switches pass
        assert!(parse(&["--shot", "/p.png", "--enable-logging"]).is_ok());
    }
}
