use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::Builder;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn world_directory(shared: &Path, code: &str) -> Result<PathBuf> {
    if !shared.is_absolute() {
        return Err("Выбери существующую общую папку.".into());
    }
    if code.is_empty()
        || code.len() > 64
        || !code.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err("В коде мира нужны латинские буквы, цифры или дефисы (до 64 символов).".into());
    }
    Ok(shared
        .join("GameSaves")
        .join(format!("world-{}", code.to_ascii_lowercase())))
}

fn version_timestamp(name: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let (timestamp, suffix) = name
        .strip_prefix("save-")?
        .strip_suffix(".zip")?
        .split_once('-')?;
    if timestamp.len() != 15
        || !timestamp.bytes().all(|c| c.is_ascii_digit())
        || suffix.is_empty()
        || suffix.len() > 32
        || !suffix.bytes().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }
    chrono::DateTime::from_timestamp_millis(timestamp.parse().ok()?)
}

pub fn valid_version_name(name: &str) -> bool {
    version_timestamp(name).is_some()
}

pub fn versions(directory: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut versions = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(date) = version_timestamp(name) else {
            continue;
        };
        if entry.file_type()?.is_file() {
            versions.push((
                date.with_timezone(&chrono::Local)
                    .format("%d.%m.%Y · %H:%M:%S")
                    .to_string(),
                path,
            ));
        }
    }
    versions.sort_by(|left, right| right.1.cmp(&left.1));
    Ok(versions)
}

pub fn publish(source: &Path, directory: &Path) -> Result<PathBuf> {
    if !source.is_file() || fs::symlink_metadata(source)?.is_symlink() {
        return Err("Для отправки выбери локальный файл сохранения.".into());
    }
    fs::create_dir_all(directory)?;
    let source = source.canonicalize()?;
    if source.starts_with(directory.canonicalize()?) {
        return Err("Выбери файл из папки игры.".into());
    }
    let mut input = File::open(source)?;
    if input.metadata()?.len() == 0 {
        return Err("Файл сохранения пуст.".into());
    }
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut pending = Builder::new()
        .prefix(&format!("save-{timestamp:015}-"))
        .rand_bytes(12)
        .suffix(".pending")
        .tempfile_in(directory)?;
    let mut zip = ZipWriter::new(pending.as_file_mut());
    zip.start_file(
        "save",
        SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(true),
    )?;
    io::copy(&mut input, &mut zip)?;
    zip.finish()?.sync_all()?;
    let destination = pending.path().with_extension("zip");
    pending.persist_noclobber(&destination)?;
    Ok(destination)
}

pub fn copy_version(version: &Path, directory: &Path) -> Result<()> {
    fs::create_dir_all(directory)?;
    let name = version.file_name().ok_or("У версии нет имени файла.")?;
    let destination = directory.join(name);
    let mut pending = Builder::new().suffix(".pending").tempfile_in(directory)?;
    io::copy(&mut File::open(version)?, &mut pending)?;
    pending.as_file().sync_all()?;
    pending.persist_noclobber(destination)?;
    Ok(())
}

pub fn restore(version: &Path, destination: &Path) -> Result<PathBuf> {
    if !destination.is_file() || fs::symlink_metadata(destination)?.is_symlink() {
        return Err(
            "Выбери существующий локальный файл сохранения. Новый слот можно создать в игре."
                .into(),
        );
    }
    let destination = destination.canonicalize()?;
    let parent = destination.parent().ok_or("Недоступна папка сохранений.")?;
    if version.canonicalize()? == destination {
        return Err("Архив и файл игры должны находиться по разным путям.".into());
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(parent.join(".savesync-restore.lock"))?;
    lock.try_lock()?;

    let mut archive = ZipArchive::new(File::open(version)?)?;
    let mut contents = archive.by_name("save")?;
    if contents.size() == 0 {
        return Err("В архиве нет данных сохранения.".into());
    }
    let mut staged = Builder::new()
        .prefix(".savesync-restore-")
        .tempfile_in(parent)?;
    io::copy(&mut contents, &mut staged)?;
    staged.as_file().sync_all()?;

    let mut backup = Builder::new()
        .prefix("savesync-backup-")
        .suffix(".bak")
        .tempfile_in(parent)?;
    io::copy(&mut File::open(&destination)?, &mut backup)?;
    backup.flush()?;
    backup.as_file().sync_all()?;
    let (_, backup_path) = backup.keep()?;
    staged.persist(&destination)?;
    Ok(backup_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_versions_restore_with_backup() -> Result<()> {
        let root = tempfile::tempdir()?;
        let shared = world_directory(root.path(), "subnautica-2")?;
        let local = root.path().join("savegame_0.sav");
        fs::write(&local, b"first world state")?;
        let first = publish(&local, &shared)?;
        fs::write(&local, b"second world state")?;
        publish(&local, &shared)?;
        assert_eq!(versions(&shared)?.len(), 2);
        let backup = restore(&first, &local)?;
        assert_eq!(fs::read(&local)?, b"first world state");
        assert_eq!(fs::read(&backup)?, b"second world state");
        Ok(())
    }

    #[test]
    fn damaged_archive_preserves_local_save() -> Result<()> {
        let root = tempfile::tempdir()?;
        let version = root.path().join("interrupted.zip");
        let local = root.path().join("savegame_0.sav");
        fs::write(&version, b"broken zip")?;
        fs::write(&local, b"original world")?;
        assert!(restore(&version, &local).is_err());
        assert_eq!(fs::read(&local)?, b"original world");
        Ok(())
    }
}
