//! The screens' commands the headset answers (`invoke`, through the shim): the library's, the marks on
//! a table, a table's own settings, the screens' preferences, and play, which the player does. The
//! app's other commands (its tools, VPS, drops, cabinet mode, the editors) aren't in the headset: they
//! answer that they aren't, and the screens show it as any problem.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use panel_link::{Link, Request};
use serde_json::{Value, json};

use crate::catalog::Catalog;
use crate::table_settings;

pub struct Commands {
    pub catalog: Arc<Mutex<Catalog>>,
    pub link: Option<Arc<Link>>,
    /// The table the player goes back to, if any (`--back`).
    pub back: Option<PathBuf>,
    /// What each table's script told of its options, asked once per table file.
    pub asked: table_settings::Asked,
}

fn text<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args[name].as_str().ok_or_else(|| format!("no {name}"))
}

impl Commands {
    fn catalog(&self) -> MutexGuard<'_, Catalog> {
        self.catalog.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A table's file, and the game it plays when it drives PinMAME.
    fn table(&self, args: &Value) -> Result<(PathBuf, Option<String>), String> {
        let catalog = self.catalog();
        let t = catalog.find(text(args, "id")?).ok_or("That table isn't in the library any more.")?;
        Ok((t.path.clone(), t.read.game_name.clone().filter(|_| t.read.pinmame)))
    }

    /// A request for the player; without one (a check with `--shot`), nothing to do.
    fn ask(&self, request: Request) -> Result<Value, String> {
        match &self.link {
            Some(link) if !link.ask(&request) => Err("the player isn't taking requests".into()),
            _ => Ok(Value::Null),
        }
    }

    /// The command `name`'s reply to `args`, or what went wrong, in words.
    pub fn run(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "library" => Ok(self.catalog().reply()),
            "prefs" => Ok(self.catalog().prefs().clone()),
            "set_prefs" => self.catalog().set_prefs(args["prefs"].clone()).map(|()| Value::Null),
            "tasks_now" => Ok(json!([])),
            // the app's test mode: none here
            "test_mode" => Ok(Value::Null),
            "is_fullscreen" => Ok(json!(false)),
            // the headset's screens know they're there by this, and which table "Back to" goes to
            "headset" => Ok(json!({ "back": self.back })),
            // what the desktop's screens ask on the way that has no meaning here
            "set_fullscreen" | "vpidb_sync" | "settled" | "set_unsaved" | "show_player_log" => Ok(Value::Null),
            "play" => {
                let path = self.catalog().find(text(args, "id")?).map(|t| t.path.clone()).ok_or("that table isn't in the library")?;
                self.ask(Request::Play(path))
            }
            "set_favourite" => self.catalog().set_favourite(text(args, "id")?, args["on"].as_bool().unwrap_or(false)).map(|()| Value::Null),
            // the problems putting the rating in a front end's list: none, as the headset keeps its own
            "set_rating" => self.catalog().set_rating(text(args, "id")?, args["stars"].as_u64().map(|s| s as u8)).map(|()| json!([])),
            "set_hidden" => self.catalog().set_hidden(text(args, "id")?, args["on"].as_bool().unwrap_or(false)).map(|()| Value::Null),
            // a table's own settings, its page's settings window's: the options its script offers (asked
            // of it by the headless runner beside the player), its own choices in its `.ini` beside it,
            // the machine's for its game
            "table_settings" => {
                let (path, game) = self.table(args)?;
                serde_json::to_value(table_settings::load(&self.asked, &path, game)).map_err(|e| e.to_string())
            }
            "save_table_settings" => {
                let (path, _) = self.table(args)?;
                let field = |name: &str| args.get(name).cloned().unwrap_or(Value::Null);
                let options: BTreeMap<String, Option<f32>> = serde_json::from_value(field("options")).map_err(|e| e.to_string())?;
                let for_table: table_settings::ForTable = serde_json::from_value(field("forTable")).map_err(|e| e.to_string())?;
                let machine: Option<(String, table_settings::MachineChoices)> = serde_json::from_value(field("machine")).map_err(|e| e.to_string())?;
                table_settings::save(&path, &options, &for_table, machine.as_ref().map(|(game, m)| (game.as_str(), m))).map(|()| Value::Null)
            }
            "save_reach_outside" => {
                let (path, _) = self.table(args)?;
                table_settings::save_reach_outside(&path, args["value"].as_bool()).map(|()| Value::Null)
            }
            // the desktop's screen layouts, and a table's files on VPS: none in the headset
            "screen_presets" | "vps_files" => Ok(json!([])),
            "panel_close" => self.ask(Request::Close),
            "quit" => self.ask(Request::Quit),
            other => Err(format!("{other} isn't in the headset")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_commands_say_so() {
        let dir = std::env::temp_dir().join(format!("vpi-panel-commands-{}", std::process::id()));
        let c = Commands { catalog: Arc::new(Mutex::new(Catalog::new(&dir, &dir))), link: None, back: Some(PathBuf::from("/t/a.vpx")), asked: Default::default() };
        assert_eq!(c.run("headset", &Value::Null), Ok(json!({ "back": "/t/a.vpx" })));
        assert_eq!(c.run("library", &Value::Null).unwrap()["tables"], json!([]));
        assert!(c.run("vps_browser", &Value::Null).unwrap_err().contains("isn't in the headset"));
        assert!(c.run("play", &json!({ "id": "nothing" })).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
