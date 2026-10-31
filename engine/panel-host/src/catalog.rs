//! The tables the panel shows: the tables folder read as the app reads a library (`engine/library`:
//! each table's facts, what it holds beside it, its front ends' media), and the marks the user puts on
//! them in the headset (favourites, ratings, hidden), kept in the panel's own folder. Each table goes
//! to the screens in the shape of the app's `TableView` (`app/src/library.rs`): the fields the
//! headset's screens read filled in, the rest empty, so the app's own screens show them. The Frame has
//! none of the app's records (VPS's database, its ratings and playlists): names, makers and years come
//! from the tables' own files and names, pictures from their front ends' media.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use library::installed::{self, Holds, Media, OnVps, Read, Remembered};
use library::plan::Layout;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A table found, read once.
pub struct Table {
    pub id: String,
    pub path: PathBuf,
    pub stem: String,
    pub read: Read,
    pub holds: Holds,
    /// When it came onto the machine, in ms since 1970.
    pub added: i64,
}

/// The user's marks, by each table's key (its path in lower case, as the app's own lists keep them).
#[derive(Default, Serialize, Deserialize)]
struct Marks {
    #[serde(default)]
    favourites: BTreeSet<String>,
    #[serde(default)]
    ratings: BTreeMap<String, u8>,
    #[serde(default)]
    hidden: BTreeSet<String>,
    /// The screens' own preferences (their `app.prefs`), kept as they hand them over.
    #[serde(default)]
    prefs: Value,
}

/// How reading the library goes, as the app's `Status` tells the screens.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub phase: &'static str,
    pub done: usize,
    pub of: usize,
    pub current: Option<String>,
    pub problem: Option<String>,
}

pub struct Catalog {
    pub layout: Layout,
    pub tables: Vec<Table>,
    pub status: Status,
    marks: Marks,
    marks_file: PathBuf,
}

/// Each kind of media and its words, as the app names them.
const MEDIA_KINDS: [(&str, &str); 15] = [
    ("playfield", "Playfield"),
    ("playfield-video", "Playfield video"),
    ("backglass", "Backglass"),
    ("backglass-video", "Backglass video"),
    ("dmd", "Score display"),
    ("dmd-video", "Score display video"),
    ("topper", "Topper"),
    ("topper-video", "Topper video"),
    ("wheel", "Wheel"),
    ("gameinfo", "Flyer"),
    ("gamehelp", "Instruction card"),
    ("loading", "Loading"),
    ("menu", "Menu"),
    ("gameselect", "Game select"),
    ("other", "Other"),
];

/// A table's key: its path in lower case (the app's `key`).
pub fn key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// A table's ID for the screens: FNV-1a of its key (the app's `id_of`), so the same table has the
/// same ID in the app and in the headset.
pub fn id_of(path: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key(path).bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// A name and its maker and year from a file name, `Name (Maker Year) …` (the app's `from_file_name`).
fn from_file_name(stem: &str) -> (String, Option<String>, Option<i32>) {
    let Some(open) = stem.find(" (") else { return (stem.trim().to_string(), None, None) };
    let name = stem[..open].trim().to_string();
    let inside = stem[open + 2..].split(')').next().unwrap_or("");
    let mut words: Vec<&str> = inside.split_whitespace().collect();
    let year = words.last().and_then(|w| w.parse::<i32>().ok()).filter(|y| (1930..=2100).contains(y));
    if year.is_some() {
        words.pop();
    }
    let maker = (!words.is_empty()).then(|| words.join(" "));
    (name, maker, year)
}

/// The name without a subtitle, for buttons (the app's `short_name`).
fn short_name(name: &str) -> String {
    let cut = [" - ", ": ", " ("].iter().filter_map(|s| name.find(s)).min().unwrap_or(name.len());
    name[..cut].trim().to_string()
}

/// A front end's media's kind as the screens name it: Popper's kinds in lower case, `-video` after a
/// video's (the app's `media_kind`).
fn media_kind(m: &Media) -> String {
    let kind = m.kind.to_lowercase();
    let kind = if MEDIA_KINDS.iter().any(|(k, _)| *k == kind) { kind } else { "other".to_string() };
    if m.video { format!("{kind}-video") } else { kind }
}

fn kind_label(kind: &str) -> String {
    MEDIA_KINDS.iter().find(|(k, _)| *k == kind).map_or_else(|| "Other".to_string(), |(_, label)| (*label).to_string())
}

/// Where the screens get a table's media: by the table's ID and the media's number, never by a path
/// the screens give.
pub fn media_url(id: &str, n: usize) -> String {
    format!("/__vpi/media/{id}/{n}")
}

impl Catalog {
    /// The library under `root` (VPX's layout: `Tables`, `VPinMAME`, `PUPVideos` beside it), not read
    /// yet; the marks kept in `folder`.
    pub fn new(root: &Path, folder: &Path) -> Catalog {
        let marks_file = folder.join("marks.json");
        let marks = std::fs::read(&marks_file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Catalog { layout: Layout::under(root), tables: Vec::new(), status: Status { phase: "starting", ..Default::default() }, marks, marks_file }
    }

    /// Reads every table in `layout`'s tables folder (each file's reading remembered in `folder`,
    /// so a table read before isn't read again), telling `progress` as it goes.
    pub fn read(layout: &Layout, folder: &Path, mut progress: impl FnMut(usize, usize, &Path)) -> Result<Vec<Table>, String> {
        let remembered_file = folder.join("tables.json");
        let mut remembered = Remembered::load(&remembered_file);
        let found = installed::scan(&layout.tables, &mut remembered, |done, of, path| progress(done, of, path))?;
        if let Err(e) = remembered.save(&remembered_file) {
            eprintln!("vpi-panel: the tables' reads weren't kept: {e}");
        }
        let aliases = installed::Aliases::load(&layout.vpinmame);
        let media = installed::media_folders(layout);
        let around = installed::Around::read(layout);
        Ok(found
            .into_iter()
            .map(|(path, read)| {
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let holds = installed::holds(layout, &aliases, &media, &around, &path, &read);
                let added = std::fs::metadata(&path)
                    .and_then(|m| m.created())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(read.modified, |d| d.as_millis() as i64);
                Table { id: id_of(&path), path, stem, read, holds, added }
            })
            .collect())
    }

    pub fn find(&self, id: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.id == id)
    }

    /// The file behind a table's media of that number.
    pub fn media_file(&self, id: &str, n: usize) -> Option<&Path> {
        self.find(id)?.holds.media.get(n).map(|m| m.path.as_path())
    }

    /// The `library` command's reply: the app's `LibraryReply`.
    pub fn reply(&self) -> Value {
        json!({
            "status": self.status,
            "tables": self.tables.iter().map(|t| self.view(t)).collect::<Vec<_>>(),
            "playlists": [],
            "popper": self.tables.iter().any(|t| !t.holds.media.is_empty()),
            "ratingFronts": [],
        })
    }

    /// A table as the screens show it: the app's `TableView`.
    pub fn view(&self, t: &Table) -> Value {
        let placeholder = |n: &str| ["visual pinball demo table", "table1"].contains(&n.to_lowercase().as_str());
        let (file_name, maker, year) = from_file_name(&t.stem);
        let name = t.read.info_name.clone().filter(|n| !n.trim().is_empty() && !placeholder(n)).unwrap_or(file_name);
        let authors: Vec<String> =
            t.read.info_authors.as_deref().map(|a| a.split([',', '&']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
        let media: Vec<Value> = t
            .holds
            .media
            .iter()
            .enumerate()
            .map(|(n, m)| {
                let kind = media_kind(m);
                let from = m.from.name();
                let file = m.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                json!({
                    "kind": kind,
                    "label": kind_label(&kind),
                    "url": media_url(&t.id, n),
                    "video": m.video,
                    "from": from,
                    "contain": kind == "wheel",
                    "pick": format!("{}:{}", from.to_lowercase(), file.to_lowercase()),
                    "chosen": false,
                    "front": true,
                    "copy": null,
                    "turn": 0,
                    "sound": false,
                })
            })
            .collect();
        // each kind's first picture, and a playfield's first video
        let first = |kind: &str, video: bool| t.holds.media.iter().position(|m| m.video == video && media_kind(m).trim_end_matches("-video") == kind).map(|n| media_url(&t.id, n));
        let key = key(&t.path);
        json!({
            "id": t.id,
            "key": key,
            "path": t.path,
            "file": t.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
            "short": short_name(&name),
            "name": name,
            "maker": maker,
            "year": year,
            "badge": null,
            "kind": null,
            "players": null,
            "board": null,
            "designers": [],
            "theme": [],
            "version": t.read.info_version,
            "authors": authors,
            "features": [],
            "ipdb": null,
            "vpsLink": null,
            "onVps": OnVps::default(),
            "guess": null,
            "readError": t.read.error,
            "physicsSet": t.read.physics_set,
            "rom": t.holds.rom_set,
            "holds": t.holds,
            "other": [],
            "vpsHas": {},
            "picture": {
                "playfield": first("playfield", false),
                "backglass": first("backglass", false),
                "wheel": first("wheel", false),
                "video": first("playfield", true),
            },
            "media": media,
            "hidden": 0,
            "screenshots": 0,
            "screenshot": null,
            "size": t.read.size,
            "changed": t.read.modified,
            "added": t.added,
            "favourite": self.marks.favourites.contains(&key),
            "rating": self.marks.ratings.get(&key),
            "ratingFrom": self.marks.ratings.contains_key(&key).then_some("ours"),
            "hiddenTable": self.marks.hidden.contains(&key),
            "played": null,
            "tested": null,
            "romCheck": null,
            "altCheck": null,
            "altMade": false,
            "scriptUpdate": null,
            "pupCheck": null,
        })
    }

    pub fn prefs(&self) -> &Value {
        &self.marks.prefs
    }

    pub fn set_prefs(&mut self, prefs: Value) -> Result<(), String> {
        self.marks.prefs = prefs;
        self.save()
    }

    pub fn set_favourite(&mut self, id: &str, on: bool) -> Result<(), String> {
        let key = key(&self.find(id).ok_or("no such table")?.path);
        if on {
            self.marks.favourites.insert(key);
        } else {
            self.marks.favourites.remove(&key);
        }
        self.save()
    }

    pub fn set_rating(&mut self, id: &str, stars: Option<u8>) -> Result<(), String> {
        let key = key(&self.find(id).ok_or("no such table")?.path);
        match stars.filter(|s| (1..=5).contains(s)) {
            Some(s) => self.marks.ratings.insert(key, s),
            None => self.marks.ratings.remove(&key),
        };
        self.save()
    }

    pub fn set_hidden(&mut self, id: &str, on: bool) -> Result<(), String> {
        let key = key(&self.find(id).ok_or("no such table")?.path);
        if on {
            self.marks.hidden.insert(key);
        } else {
            self.marks.hidden.remove(&key);
        }
        self.save()
    }

    fn save(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.marks).map_err(|e| e.to_string())?;
        if let Some(dir) = self.marks_file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        // written beside, then moved over, so a crash never leaves half a file
        let part = self.marks_file.with_extension("json.part");
        std::fs::write(&part, bytes).map_err(|e| format!("{}: {e}", part.display()))?;
        std::fs::rename(&part, &self.marks_file).map_err(|e| format!("{}: {e}", self.marks_file.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_match_the_apps() {
        // FNV-1a of the lower-case path, as app/src/library.rs makes them
        assert_eq!(id_of(Path::new("/A/B.vpx")), id_of(Path::new("/a/b.VPX")));
        assert_eq!(id_of(Path::new("")), "cbf29ce484222325");
    }

    #[test]
    fn names_come_from_file_names() {
        assert_eq!(from_file_name("Medieval Madness (Williams 1997) VPW 1.2"), ("Medieval Madness".into(), Some("Williams".into()), Some(1997)));
        assert_eq!(from_file_name("Symphony IX 1.0.2"), ("Symphony IX 1.0.2".into(), None, None));
        assert_eq!(short_name("Star Wars: Episode I"), "Star Wars");
    }

    #[test]
    fn marks_are_kept() {
        let dir = std::env::temp_dir().join(format!("vpi-panel-marks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut c = Catalog::new(&dir, &dir);
        let path = dir.join("Tables").join("A (Maker 1990).vpx");
        c.tables.push(Table { id: id_of(&path), path: path.clone(), stem: "A (Maker 1990)".into(), read: Read::default(), holds: Holds::default(), added: 0 });
        let id = id_of(&path);
        c.set_favourite(&id, true).unwrap();
        c.set_rating(&id, Some(4)).unwrap();
        let again = Catalog::new(&dir, &dir);
        assert!(again.marks.favourites.contains(&key(&path)));
        assert_eq!(again.marks.ratings.get(&key(&path)), Some(&4));
        let v = c.view(&c.tables[0]);
        assert_eq!(v["name"], "A");
        assert_eq!(v["maker"], "Maker");
        assert_eq!(v["year"], 1990);
        assert_eq!(v["favourite"], true);
        assert_eq!(v["rating"], 4);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
