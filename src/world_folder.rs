use std::fs::{self, File};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MAX_SIZE: u64 = 2 * 1024 * 1024 * 1024;
const ROOT: &str = "world/";

fn safe_component(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    !name.is_empty()
        && !name.ends_with(['.', ' '])
        && !name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        && !matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        && !["COM", "LPT"].iter().any(|p| {
            stem.strip_prefix(p)
                .is_some_and(|s| s.chars().count() == 1 && "0123456789¹²³".contains(s))
        })
}
pub fn write<W: Write + Seek>(zip: &mut ZipWriter<W>, root: &Path) -> Result<()> {
    let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.add_directory(ROOT, opt)?;
    let mut pending = vec![(root.to_owned(), ROOT.to_string())];
    let mut count = 0;
    let mut bytes = 0;
    let mut any_file = false;
    while let Some((dir, prefix)) = pending.pop() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "Нечитаемое имя файла мира.")?;
            if !safe_component(&name) || prefix.len() + name.len() > 2048 {
                return Err("Недопустимый путь в папке мира.".into());
            }
            let kind = entry.file_type()?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if fs::symlink_metadata(entry.path())?.file_attributes() & 0x400 != 0 {
                    return Err("В папке мира есть ссылка.".into());
                }
            }
            count += 1;
            if count > 100_000 {
                return Err("Слишком много файлов мира.".into());
            }
            let path = format!("{prefix}{name}");
            if kind.is_dir() {
                let path = format!("{path}/");
                zip.add_directory(&path, opt)?;
                pending.push((entry.path(), path));
            } else if kind.is_file() {
                zip.start_file(path, opt)?;
                bytes += io::copy(
                    &mut File::open(entry.path())?.take(MAX_SIZE - bytes + 1),
                    zip,
                )?;
                if bytes > MAX_SIZE {
                    return Err("Мир превышает 2 ГиБ.".into());
                }
                any_file = true;
            } else {
                return Err("В папке мира есть ссылка или специальный файл.".into());
            }
        }
    }
    if !any_file {
        return Err("Папка мира пуста.".into());
    }
    Ok(())
}
pub fn restore(mut archive: ZipArchive<File>, dest: &Path, parent: &Path) -> Result<PathBuf> {
    if archive.len() > 100_001 || !archive.by_name(ROOT)?.is_dir() {
        return Err("Ожидался архив папки мира.".into());
    }
    let stage = tempfile::Builder::new()
        .prefix(".gamesaves-world-")
        .tempdir_in(parent)?;
    let mut any_file = false;
    let mut bytes = 0;
    for index in 0..archive.len() {
        let mut item = archive.by_index(index)?;
        if item.name() == ROOT {
            continue;
        }
        let name = item
            .name()
            .strip_prefix(ROOT)
            .ok_or("Некорректный путь архива.")?
            .trim_end_matches('/');
        if name.len() > 2048 || !name.split('/').all(safe_component) {
            return Err("Недопустимый путь архива.".into());
        }
        let path = stage.path().join(name);
        if item.is_dir() {
            fs::create_dir_all(path)?;
            continue;
        }
        if item.compression() != zip::CompressionMethod::Stored
            || item.size() != item.compressed_size()
            || item.size() > MAX_SIZE - bytes
        {
            return Err("Некорректный размер файла мира.".into());
        }
        fs::create_dir_all(path.parent().ok_or("Недопустимый путь.")?)?;
        let mut out = File::create_new(path)?;
        let size = item.size();
        if io::copy(&mut (&mut item).take(size + 1), &mut out)? != size {
            return Err("Неполный файл мира.".into());
        }
        out.sync_all()?;
        bytes += size;
        any_file = true;
    }
    if !any_file {
        return Err("Пустой архив мира.".into());
    }
    let backup = tempfile::Builder::new()
        .prefix("savesync-backup-")
        .tempdir_in(parent)?;
    let previous = backup
        .path()
        .join(dest.file_name().ok_or("Выбери папку конкретного мира.")?);
    fs::rename(dest, &previous)?;
    let _keep = backup.keep();
    if let Err(error) = fs::rename(stage.path(), dest) {
        if fs::rename(&previous, dest).is_err() {
            return Err(format!(
                "Не удалось восстановить папку. Исходный мир сохранён в {}.",
                previous.display()
            )
            .into());
        }
        return Err(error.into());
    }
    Ok(previous)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshots;
    #[test]
    fn restores_whole_world_preserving_displaced_files() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let world = tmp.path().join("ocean");
        fs::create_dir_all(world.join("chunks"))?;
        fs::write(world.join("meta"), b"map")?;
        fs::write(world.join("chunks/a"), b"resource")?;
        let archive = snapshots::publish(&world, &tmp.path().join("archives"), "Ваня")?;
        fs::remove_file(world.join("chunks/a"))?;
        fs::write(world.join("new"), b"later")?;
        let backup = snapshots::restore(&archive, &world)?;
        assert_eq!(fs::read(world.join("chunks/a"))?, b"resource");
        assert!(!world.join("new").exists());
        assert_eq!(fs::read(backup.join("new"))?, b"later");
        Ok(())
    }
}
