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
    pub nickname: String,
    pub groups: BTreeMap<String, String>,
    pub custom_games: Vec<Game>,
    pub save_paths: BTreeMap<String, PathBuf>,
    pub shared_path: PathBuf,
    pub world_codes: BTreeMap<String, String>,
    pub covers: BTreeMap<String, PathBuf>,
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

pub fn active_save<'a>(manual: Option<&'a Path>, candidates: &'a [PathBuf]) -> Option<&'a Path> {
    if let Some(path) = manual.filter(|path| usable_save(path)) {
        return Some(path);
    }
    match candidates {
        [path] if usable_save(path) => Some(path),
        _ => None,
    }
}

pub fn usable_save(path: &Path) -> bool {
    path.is_absolute()
        && fs::symlink_metadata(path).is_ok_and(|meta| {
            let kind = meta.file_type();
            !is_reparse_point(&meta) && (kind.is_dir() || kind.is_file() && meta.len() > 0)
        })
}

fn is_reparse_point(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    meta.file_type().is_symlink()
}

pub fn save_candidates(game: &Game) -> Vec<PathBuf> {
    if game.id == "steam:1962700" || game.name.eq_ignore_ascii_case("Subnautica 2") {
        let local_app_data = env::var_os("LOCALAPPDATA").map(PathBuf::from);
        return subnautica_saves(&game.install_dir, local_app_data.as_deref());
    }
    if game.id != "steam:892970" && !game.name.eq_ignore_ascii_case("Valheim") {
        return Vec::new();
    }

    let mut roots = Vec::new();
    if let Some(profile) = env::var_os("USERPROFILE") {
        let local = PathBuf::from(profile).join("AppData/LocalLow/IronGate/Valheim");
        roots.extend([local.join("worlds_local"), local.join("worlds")]);
    }
    if let Ok(steam) = steamlocate::locate() {
        if let Ok(users) = fs::read_dir(steam.path().join("userdata")) {
            for user in users.flatten() {
                if user
                    .file_name()
                    .to_str()
                    .is_some_and(|id| id.bytes().all(|b| b.is_ascii_digit()))
                {
                    roots.push(user.path().join("892970/remote/worlds"));
                }
            }
        }
    }
    valheim_saves(roots)
}

fn valheim_saves(roots: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut paths: Vec<_> = roots
        .into_iter()
        .filter_map(|root| fs::read_dir(root).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            usable_save(path)
                && path.is_dir()
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("savesync-backup-"))
                && fs::read_dir(path).is_ok_and(|files| {
                    files.flatten().any(|entry| {
                        let name = entry.file_name();
                        let Some(stem) = name
                            .to_str()
                            .and_then(|name| name.strip_prefix("_main."))
                            .and_then(|name| name.strip_suffix(".fwl2"))
                        else {
                            return false;
                        };
                        let base = format!("_main.{stem}");
                        !stem.is_empty()
                            && stem.bytes().all(|b| b.is_ascii_digit())
                            && usable_save(&entry.path())
                            && path.join(format!("{base}.db2")).is_file()
                            && path.join(format!("{base}.ok")).is_file()
                    })
                })
        })
        .collect();
    unique_paths(&mut paths);
    paths
}

fn unique_paths(paths: &mut Vec<PathBuf>) {
    // Separate roots can point to the same directory. Keep the original user-friendly path.
    paths.sort();
    let mut seen = std::collections::HashSet::new();
    paths.retain(|path| seen.insert(path.canonicalize().unwrap_or_else(|_| path.clone())));
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
            if name
                .strip_prefix("savegame_")
                .and_then(|name| name.strip_suffix(".sav"))
                .is_some_and(|slot| !slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit()))
                && path.is_file()
                && usable_save(&path)
            {
                paths.push(path);
            }
        }
    }
    unique_paths(&mut paths);
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_save_requires_one_unambiguous_slot() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let first = root.path().join("savegame_0.sav");
        let second = root.path().join("savegame_1.sav");
        fs::write(&first, "one")?;
        fs::write(&second, "two")?;
        assert_eq!(active_save(None, &[]), None);
        assert_eq!(
            active_save(None, std::slice::from_ref(&first)),
            Some(first.as_path())
        );
        let slots = [first.clone(), second.clone()];
        assert_eq!(active_save(None, &slots), None);
        assert_eq!(active_save(Some(&second), &slots), Some(second.as_path()));
        fs::remove_file(&second)?;
        assert_eq!(
            active_save(Some(&second), &[first.clone()]),
            Some(first.as_path())
        );
        Ok(())
    }

    #[test]
    fn valheim_ignores_map_cache_and_incomplete_saves() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let world = root.path().join("Ocean");
        let cache = root.path().join("CacheOnly");
        let incomplete = root.path().join("Incomplete");
        for dir in [&world, &cache, &incomplete] {
            fs::create_dir(dir)?;
        }
        for name in ["_main.7.fwl2", "_main.7.db2", "_main.7.ok"] {
            fs::write(world.join(name), "world")?;
        }
        fs::write(cache.join("cacheMinimap"), "cache")?;
        fs::write(incomplete.join("_main.9.fwl2"), "metadata")?;
        assert_eq!(valheim_saves([root.path().to_path_buf()]), [world]);
        Ok(())
    }

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
