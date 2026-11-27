use crate::snapshots;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

const LIMIT: u64 = 32 * 1024 * 1024;
pub type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Clone)]
pub struct Group {
    base: String,
    auth: String,
    pub device: bool,
}
#[derive(serde::Deserialize)]
pub struct Applicant {
    pub id: String,
    pub nick: String,
}
#[derive(serde::Deserialize)]
pub struct Membership {
    pub role: String,
    pub id: String,
    pub folder: String,
    pub proof: String,
    pub requests: Vec<Applicant>,
}

pub fn folder_id(input: &str) -> Option<String> {
    let url = Url::parse(input.trim()).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("drive.google.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    let id = match &parts[..] {
        ["drive", "folders", id] | ["drive", "u", _, "folders", id] => *id,
        _ => return None,
    };
    (!id.is_empty()
        && id.len() <= 200
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == '-' as u8 || b == '_' as u8))
    .then(|| id.to_string())
}
pub fn join(origin: &str, folder: String, nick: String) -> Result<String> {
    let data = serde_json::json!({"folder":folder,"nick":nick}).to_string();
    let mut r = http()
        .post(format!("{origin}/devices"))
        .header("Content-Type", "application/json")
        .send(data.as_bytes())
        .map_err(err)?;
    let reply: serde_json::Value =
        serde_json::from_str(&r.body_mut().read_to_string().map_err(err)?).map_err(err)?;
    let token = reply["token"]
        .as_str()
        .ok_or("Сервис не выдал ключ устройства.")?;
    let link = format!("{origin}/device#{token}");
    Group::parse(&link)?;
    Ok(link)
}
fn http() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(120)))
            .build(),
    )
}
impl Group {
    pub fn parse(input: &str) -> Result<Self> {
        let url = Url::parse(input.trim()).map_err(|_| "Вставь полную ссылку-приглашение.")?;
        let token = url.fragment().unwrap_or_default();
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !matches!(url.path(), "/join" | "/device")
            || url.password().is_some()
            || !url.username().is_empty()
            || url.query().is_some()
            || token.len() != 43
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err("Нужна ссылка-приглашение GameSaves из окна создания группы.".into());
        }
        Ok(Self {
            base: format!("{}/versions", url.origin().ascii_serialization()),
            auth: format!("Bearer {token}"),
            device: url.path() == "/device",
        })
    }
    pub fn origin(&self) -> &str {
        self.base.strip_suffix("/versions").unwrap()
    }
    pub fn membership(&self) -> Result<Membership> {
        let mut r = http()
            .get(format!("{}/membership", self.origin()))
            .header("Authorization", &self.auth)
            .call()
            .map_err(err)?;
        serde_json::from_str(&r.body_mut().read_to_string().map_err(err)?).map_err(err)
    }
    pub fn decide(&self, id: &str, allow: bool) -> Result<String> {
        let decision = if allow { "allow" } else { "deny" };
        http()
            .post(format!("{}/decision/{id}/{decision}", self.origin()))
            .header("Authorization", &self.auth)
            .send_empty()
            .map_err(err)?;
        Ok("Заявка обработана.".into())
    }
    pub fn list(&self) -> Result<Vec<(String, PathBuf)>> {
        let mut r = http()
            .get(format!("{}?authors=1", self.base))
            .header("Authorization", &self.auth)
            .call()
            .map_err(err)?;
        if r.status() != 200 {
            return Err("Сервис не вернул историю.".into());
        }
        let data = r
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_vec()
            .map_err(err)?;
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Entry {
            Named {
                name: String,
                author: Option<String>,
            },
            Legacy(String),
        }
        let names: Vec<Entry> = serde_json::from_slice(&data).map_err(err)?;
        let mut rows = Vec::new();
        for entry in names {
            let (name, author) = match entry {
                Entry::Named { name, author } => (name, author),
                Entry::Legacy(name) => (name, None),
            };
            let date = snapshots::version_timestamp(&name).ok_or("Некорректное имя версии.")?;
            rows.push((
                snapshots::version_label(date, author.as_deref()),
                PathBuf::from(name),
            ));
        }
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(rows)
    }
    pub fn send(&self, source: &Path, author: &str) -> Result<String> {
        let tmp = tempfile::tempdir().map_err(err)?;
        let archive = snapshots::publish(source, tmp.path(), author).map_err(err)?;
        if fs::metadata(&archive).map_err(err)?.len() > LIMIT {
            return Err("Архив превышает лимит тестовой группы в 32 МиБ.".into());
        }
        let url = format!(
            "{}/{}",
            self.base,
            archive.file_name().unwrap().to_string_lossy()
        );
        let r = http()
            .put(url)
            .header("Authorization", &self.auth)
            .send(fs::File::open(archive).map_err(err)?)
            .map_err(err)?;
        if r.status() != 201 {
            return Err("Сервис не подтвердил отправку.".into());
        }
        Ok("Сохранение загружено на Google Диск группы.".into())
    }
    pub fn restore(&self, name: &Path, target: &Path) -> Result<PathBuf> {
        let name = name
            .to_str()
            .filter(|n| snapshots::valid_version_name(n))
            .ok_or("Некорректная версия.")?;
        let mut r = http()
            .get(format!("{}/{}", self.base, name))
            .header("Authorization", &self.auth)
            .call()
            .map_err(err)?;
        if r.status() != 200 {
            return Err("Не удалось получить архив.".into());
        }
        let mut tmp = tempfile::NamedTempFile::new().map_err(err)?;
        let n = std::io::copy(
            &mut r.body_mut().as_reader().take(LIMIT + 1),
            tmp.as_file_mut(),
        )
        .map_err(err)?;
        if n > LIMIT {
            return Err("Слишком большой архив.".into());
        }
        let mut zip = zip::ZipArchive::new(tmp.reopen().map_err(err)?).map_err(err)?;
        if target.is_file() {
            let file = zip.by_name("save").map_err(err)?;
            if file.compression() != zip::CompressionMethod::Stored
                || file.size() > LIMIT
                || file.size() != file.compressed_size()
            {
                return Err("Некорректный архив группы.".into());
            }
            drop(file);
        }
        drop(zip);
        snapshots::restore(tmp.path(), target).map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invite_validates_origin_and_fragment() {
        let token = "a".repeat(43);
        assert!(Group::parse(&format!("https://example.org/join#{token}")).is_ok());
        for s in [
            "http://example.org/join#",
            "https://user@example.org/join#",
            "https://drive.google.com/folders/",
        ] {
            assert!(Group::parse(&format!("{s}{token}")).is_err());
        }
    }
}
