use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Deserialize, Serialize)]
pub struct Game {
    pub id: String,
    pub name: String,
    pub store: String,
    pub install_dir: PathBuf,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub custom_games: Vec<Game>,
    pub save_paths: BTreeMap<String, PathBuf>,
    pub shared_path: PathBuf,
    pub world_codes: BTreeMap<String, String>,
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read(path) {
            Ok(data) => serde_json::from_slice(&data).map_err(|error| error.to_string()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn write(&self, path: &Path) -> io::Result<()> {
        let data = serde_json::to_vec_pretty(self)?;
        let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        fs::create_dir_all(parent)?;
        let mut pending = tempfile::NamedTempFile::new_in(parent)?;
        std::io::Write::write_all(&mut pending, &data)?;
        pending.as_file().sync_all()?;
        pending.persist(path)?;
        Ok(())
    }
}

pub fn settings_path() -> PathBuf {
    let directory = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    directory.join("GameSaves").join("games.json")
}

pub fn discover(custom_games: &[Game]) -> Vec<Game> {
    let mut games = custom_games.to_vec();
    if let Ok(steam) = steamlocate::locate() {
        if let Ok(libraries) = steam.libraries() {
            for library in libraries.flatten() {
                for app in library.apps().flatten() {
                    let install_dir = library.resolve_app_dir(&app);
                    let Some(name) = app.name else { continue };
                    if install_dir.is_dir() && app.app_id != 228980 {
                        games.push(Game {
                            id: format!("steam:{}", app.app_id),
                            name,
                            store: "Steam".into(),
                            install_dir,
                        });
                    }
                }
            }
        }
    }
    if let Some(program_data) = env::var_os("ProgramData") {
        let manifests = PathBuf::from(program_data).join("Epic/EpicGamesLauncher/Data/Manifests");
        games.extend(epic_games(&manifests));
    }
    #[cfg(windows)]
    games.extend(gog_games());
    games.sort_by_key(|game| (game.name.to_lowercase(), game.id.clone()));
    let mut seen = std::collections::HashSet::new();
    games.retain(|game| seen.insert(game.id.clone()));
    games
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct EpicManifest {
    app_name: String,
    display_name: String,
    install_location: PathBuf,
    #[serde(default, rename = "bIsIncompleteInstall")]
    incomplete: bool,
    #[serde(default)]
    main_game_app_name: String,
}

fn epic_games(manifests: &Path) -> Vec<Game> {
    let Ok(entries) = fs::read_dir(manifests) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()?.to_str()? != "item" {
                return None;
            }
            let contents = fs::read(&path).ok()?;
            let manifest: EpicManifest = serde_json::from_slice(&contents).ok()?;
            if manifest.incomplete
                || (!manifest.main_game_app_name.is_empty()
                    && manifest.main_game_app_name != manifest.app_name)
                || !manifest.install_location.is_dir()
            {
                return None;
            }
            Some(Game {
                id: format!("epic:{}", manifest.app_name),
                name: manifest.display_name,
                store: "Epic Games".into(),
                install_dir: manifest.install_location,
            })
        })
        .collect()
}

#[cfg(windows)]
fn gog_games() -> Vec<Game> {
    use winreg::RegKey;
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    };

    let mut games = Vec::new();
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let registry = RegKey::predef(hive);
        for view in [KEY_WOW64_32KEY, KEY_WOW64_64KEY] {
            let Ok(root) =
                registry.open_subkey_with_flags(r"SOFTWARE\GOG.com\Games", KEY_READ | view)
            else {
                continue;
            };
            for id in root.enum_keys().flatten() {
                let Ok(key) = root.open_subkey_with_flags(&id, KEY_READ | view) else {
                    continue;
                };
                let (Ok(name), Ok(path)) = (
                    key.get_value::<String, _>("gameName"),
                    key.get_value::<String, _>("path"),
                ) else {
                    continue;
                };
                let install_dir = PathBuf::from(path);
                if install_dir.is_dir() {
                    games.push(Game {
                        id: format!("gog:{id}"),
                        name,
                        store: "GOG".into(),
                        install_dir,
                    });
                }
            }
        }
    }
    games
}

pub fn save_candidates(game: &Game) -> Vec<PathBuf> {
    if game.id != "steam:1962700" && !game.name.eq_ignore_ascii_case("Subnautica 2") {
        return Vec::new();
    }
    let local_app_data = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    subnautica_saves(&game.install_dir, local_app_data.as_deref())
}

fn subnautica_saves(install_dir: &Path, local_app_data: Option<&Path>) -> Vec<PathBuf> {
    let relative = Path::new("Subnautica2/Saved/SaveGames");
    let mut roots = vec![install_dir.join(relative)];
    if let Some(local_app_data) = local_app_data {
        roots.push(local_app_data.join(relative));
    }
    let mut paths = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with("savegame_") && name.ends_with(".sav") && path.is_file() {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subnautica_finds_worlds_in_both_locations() -> io::Result<()> {
        let fixture = tempfile::tempdir()?;
        let game = fixture.path().join("game");
        let local = fixture.path().join("local");
        let relative = Path::new("Subnautica2/Saved/SaveGames");
        let game_saves = game.join(relative);
        let local_saves = local.join(relative);
        fs::create_dir_all(&game_saves)?;
        fs::create_dir_all(&local_saves)?;
        let first = game_saves.join("savegame_0.sav");
        let second = local_saves.join("savegame_1.sav");
        fs::write(&first, b"save")?;
        fs::write(&second, b"save")?;
        fs::write(local_saves.join("SharedGameSettings.sav"), b"settings")?;
        assert_eq!(subnautica_saves(&game, Some(&local)), [first, second]);
        Ok(())
    }

    #[test]
    fn epic_skips_unavailable_games_and_dlc() -> io::Result<()> {
        let fixture = tempfile::tempdir()?;
        let install = fixture.path().join("game");
        fs::create_dir(&install)?;
        let manifest = |id: &str, main: &str, incomplete: bool| {
            serde_json::json!({
                "AppName": id,
                "DisplayName": "Subnautica 2",
                "InstallLocation": install,
                "MainGameAppName": main,
                "bIsIncompleteInstall": incomplete,
            })
        };
        fs::write(
            fixture.path().join("base.item"),
            manifest("base", "base", false).to_string(),
        )?;
        fs::write(
            fixture.path().join("dlc.item"),
            manifest("dlc", "base", false).to_string(),
        )?;
        fs::write(
            fixture.path().join("incomplete.item"),
            manifest("other", "other", true).to_string(),
        )?;
        let games = epic_games(fixture.path());
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].id, "epic:base");
        Ok(())
    }
}
