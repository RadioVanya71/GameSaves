use crate::catalog::{Game, Settings};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const GAME_ID: &str = "demo:orbit";
pub const WORLD_CODE: &str = "orbit-demo";

pub struct Demo {
    pub profile: &'static str,
    pub root: PathBuf,
}

impl Demo {
    pub fn new(root: PathBuf, friend: bool) -> Self {
        Self {
            root,
            profile: if friend { "friend" } else { "player" },
        }
    }

    pub fn settings_path(&self) -> PathBuf {
        self.root.join(self.profile).join("games.json")
    }

    fn game_dir(&self) -> PathBuf {
        self.root.join(self.profile).join("Orbit")
    }

    fn save(&self) -> PathBuf {
        self.game_dir().join("orbit.sav")
    }

    pub fn prepare(&self) -> io::Result<()> {
        fs::create_dir_all(self.game_dir())?;
        let shared = self.root.join("shared");
        fs::create_dir_all(&shared)?;
        self.seed(&self.save(), b"1\n")?;
        self.seed(
            &self.game_dir().join("cover.png"),
            include_bytes!("demo-cover.png"),
        )?;
        let config = self.settings_path();
        if !config.try_exists()? {
            let mut settings = Settings {
                shared_path: shared,
                ..Default::default()
            };
            settings.custom_games.push(Game {
                id: GAME_ID.into(),
                name: "Орбитальная станция".into(),
                store: "Демоигра".into(),
                install_dir: self.game_dir(),
            });
            settings.save_paths.insert(GAME_ID.into(), self.save());
            settings
                .world_codes
                .insert(GAME_ID.into(), WORLD_CODE.into());
            settings.write(&config)?;
        }
        Ok(())
    }

    fn seed(&self, dest: &Path, bytes: &[u8]) -> io::Result<()> {
        if dest.try_exists()? {
            return Ok(());
        }
        let mut pending = tempfile::NamedTempFile::new_in(self.game_dir())?;
        pending.write_all(bytes)?;
        match pending.persist_noclobber(dest) {
            Ok(_) => Ok(()),
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(e) => Err(e.error),
        }
    }

    pub fn level(&self) -> io::Result<u32> {
        fs::read_to_string(self.save())?
            .trim()
            .parse()
            .map_err(io::Error::other)
    }

    pub fn advance(&self) -> io::Result<()> {
        let next = self
            .level()?
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Достигнут последний уровень"))?;
        let mut pending = tempfile::NamedTempFile::new_in(self.game_dir())?;
        writeln!(pending, "{next}")?;
        pending.persist(self.save())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{peers, snapshots};

    #[test]
    fn two_demo_profiles_share_versions_and_restore_independent_progress() {
        let root = tempfile::tempdir().unwrap();
        let player = Demo::new(root.path().into(), false);
        let friend = Demo::new(root.path().into(), true);
        player.prepare().unwrap();
        friend.prepare().unwrap();
        player.advance().unwrap();
        player.prepare().unwrap();
        assert_eq!(player.level().unwrap(), 2);
        assert_eq!(friend.level().unwrap(), 1);
        let settings = Settings::load(&player.settings_path()).unwrap();
        let shared = snapshots::world_directory(&settings.shared_path, WORLD_CODE).unwrap();
        let world = peers::World::new(shared, player.settings_path().parent().unwrap()).unwrap();
        peers::deliver(None, &world, &player.save()).unwrap();
        let archive = snapshots::versions(&world.shared).unwrap().pop().unwrap().1;
        let backup = snapshots::restore(&archive, &friend.save()).unwrap();
        assert_eq!(friend.level().unwrap(), 2);
        assert_eq!(fs::read_to_string(backup).unwrap(), "1\n");
    }
}
