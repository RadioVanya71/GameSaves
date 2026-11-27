use super::{Result, error};
use std::{fs, io::Write, path::Path};

pub fn load(path: &Path) -> Result<Option<super::Drive>> {
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error(e)),
    };
    let plaintext = protect(data, false)
        .map_err(|_| "Не удалось открыть сохранённый доступ к Google Диску.")?;
    serde_json::from_slice(&plaintext)
        .map(Some)
        .map_err(|_| "Сохранённый доступ к Google Диску повреждён.".into())
}

pub fn save(path: &Path, drive: &super::Drive) -> Result<()> {
    let data = serde_json::to_vec(drive).map_err(error)?;
    let encrypted =
        protect(data, true).map_err(|_| "Не удалось сохранить доступ к Google Диску.")?;
    let dir = path.parent().ok_or("Не найдена папка настроек.")?;
    fs::create_dir_all(dir).map_err(error)?;
    let mut pending = tempfile::NamedTempFile::new_in(dir).map_err(error)?;
    pending.write_all(&encrypted).map_err(error)?;
    pending.as_file().sync_all().map_err(error)?;
    pending.persist(path).map_err(error)?;
    Ok(())
}

#[cfg(windows)]
fn protect(mut data: Vec<u8>, encrypt: bool) -> std::io::Result<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len().try_into().map_err(std::io::Error::other)?,
        pbData: data.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    // DPAPI allocates the output on success. Release it with LocalFree after copying.
    unsafe {
        let success = if encrypt {
            CryptProtectData(
                &input,
                null(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                null_mut(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if success == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Ok(result)
    }
}

#[cfg(not(windows))]
fn protect(data: Vec<u8>, _: bool) -> std::io::Result<Vec<u8>> {
    // NamedTempFile uses private, owner-only permissions on Unix.
    Ok(data)
}
