mod oauth;
mod vault;

pub use oauth::Client;
pub use vault::{load, save};

use crate::{catalog::Game, snapshots};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use url::Url;

pub type Result<T> = std::result::Result<T, String>;
const FILES: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
const MAX_ARCHIVE: u64 = 3 * 1024 * 1024 * 1024;
const FOLDER: &str = "application/vnd.google-apps.folder";

#[derive(Clone, Deserialize, Serialize)]
pub struct Drive {
    client: Client,
    refresh: String,
    pub folder: Folder,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    selected_archives: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    #[serde(skip)]
    access: Arc<Mutex<Option<Access>>>,
}
struct Access {
    token: String,
    until: Instant,
}
impl Access {
    fn new(token: String, expires_in: u64) -> Self {
        Self {
            token,
            until: Instant::now()
                + Duration::from_secs(expires_in.clamp(1, 7200).saturating_sub(30)),
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Folder {
    id: String,
    pub name: String,
}
impl Folder {
    pub fn link(&self) -> String {
        format!("https://drive.google.com/drive/folders/{}", self.id)
    }
}

impl Drive {
    pub fn connect(
        client: Client,
        requested_folder: Option<&str>,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<Self> {
        let (tokens, mut ids) = oauth::authorize(
            &client,
            oauth::Selection::Folder(requested_folder),
            cancelled,
        )?;
        let folder_id = ids.pop().ok_or("Выбери папку.")?;
        let refresh = tokens
            .refresh_token
            .ok_or("Google не выдал долгосрочный доступ. Повтори подключение.")?;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Metadata {
            name: String,
            mime_type: String,
            trashed: bool,
            capabilities: Capabilities,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Capabilities {
            can_add_children: bool,
        }
        oauth::check_cancelled(cancelled)?;
        let metadata: Metadata = json(http(30)
            .get(format!("{FILES}/{folder_id}?supportsAllDrives=true&fields=name,mimeType,trashed,capabilities(canAddChildren)"))
            .header("Authorization", bearer(&tokens.access_token))
            .call().map_err(|e| format!("Проверка папки на www.googleapis.com. {}", request_error(e)))?)
            .map_err(|e| format!("Проверка папки. {e}"))?;
        if metadata.mime_type != FOLDER
            || metadata.trashed
            || !metadata.capabilities.can_add_children
        {
            return Err("Выбери папку, в которую можешь загружать файлы.".into());
        }
        Ok(Self {
            client,
            refresh,
            folder: Folder {
                id: folder_id,
                name: metadata.name,
            },
            selected_archives: Default::default(),
            access: Arc::new(Mutex::new(Some(Access::new(
                tokens.access_token,
                tokens.expires_in,
            )))),
        })
    }

    pub fn client(&self) -> Client {
        self.client.clone()
    }

    fn token(&self) -> Result<String> {
        let mut cached = self
            .access
            .lock()
            .map_err(|_| "Не удалось обновить доступ.")?;
        if let Some(token) = cached
            .as_ref()
            .filter(|access| Instant::now() < access.until)
        {
            return Ok(token.token.clone());
        }
        let tokens = self.client.refresh(&self.refresh)?;
        let access = Access::new(tokens.access_token, tokens.expires_in);
        let token = access.token.clone();
        *cached = Some(access);
        Ok(token)
    }

    pub fn send(&self, game: &str, source: &Path, author: &str) -> Result<String> {
        let temp = tempfile::tempdir().map_err(error)?;
        let path = snapshots::publish(source, temp.path(), author).map_err(error)?;
        let archive = File::open(&path).map_err(error)?;
        let size = archive.metadata().map_err(error)?.len();
        if size > MAX_ARCHIVE {
            return Err("Архив превышает лимит GameSaves в 3 ГиБ.".into());
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Не удалось назвать архив.")?;
        let metadata = serde_json::json!({
            "name": name,
            "parents": [self.folder.id],
            "mimeType": "application/zip",
            "appProperties": {"gamesaves": game, "author": author},
        });
        let token = self.token()?;
        let agent = http(600);
        let session = agent
            .post(format!(
                "{UPLOAD}?uploadType=resumable&supportsAllDrives=true&fields=id"
            ))
            .header("Authorization", bearer(&token))
            .header("Content-Type", "application/json; charset=UTF-8")
            .header("X-Upload-Content-Type", "application/zip")
            .header("X-Upload-Content-Length", size.to_string())
            .send(metadata.to_string().as_bytes())
            .map_err(request_error)?;
        let session = session
            .headers()
            .get("Location")
            .and_then(|v| v.to_str().ok())
            .and_then(upload_session)
            .ok_or("Google не выдал адрес загрузки.")?;
        #[derive(Deserialize)]
        struct Uploaded {
            id: String,
        }
        let uploaded: Uploaded = json(
            agent
                .put(session.as_str())
                .header("Authorization", bearer(&token))
                .header("Content-Type", "application/zip")
                .header("Content-Length", size.to_string())
                .send(archive)
                .map_err(request_error)?,
        )?;
        if !valid_id(&uploaded.id) {
            return Err("Google не подтвердил загрузку архива.".into());
        }
        Ok("Сохранение загружено на Google Диск.".into())
    }

    pub fn add_archives(
        mut self,
        game: &str,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(Self, usize)> {
        let current = self.token()?;
        let (tokens, ids) = oauth::authorize(&self.client, oauth::Selection::Archives, cancelled)?;
        oauth::check_cancelled(cancelled)?;
        let agent = http(30);
        let account = |token: &str| -> Result<String> {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct User {
                permission_id: String,
            }
            #[derive(Deserialize)]
            struct About {
                user: User,
            }
            let info: About = json(
                agent
                    .get("https://www.googleapis.com/drive/v3/about?fields=user(permissionId)")
                    .header("Authorization", bearer(token))
                    .call()
                    .map_err(request_error)?,
            )?;
            Ok(info.user.permission_id)
        };
        if account(&current)? != account(&tokens.access_token)? {
            return Err("Выбери тот же Google-аккаунт, которым подключена папка.".into());
        }
        let mut selected = std::collections::BTreeSet::new();
        for id in ids {
            oauth::check_cancelled(cancelled)?;
            let remote: RemoteFile = json(agent.get(format!("{FILES}/{id}?supportsAllDrives=true&fields=id,name,parents,appProperties,trashed"))
                .header("Authorization", bearer(&tokens.access_token)).call().map_err(request_error)?)?;
            if !remote.trashed
                && remote.id == id
                && remote.parents.contains(&self.folder.id)
                && remote
                    .app_properties
                    .get("gamesaves")
                    .is_none_or(|tag| tag == game)
                && snapshots::version_timestamp(&remote.name).is_some()
            {
                selected.insert(remote.id);
            }
        }
        if selected.is_empty() {
            return Err("Выбери ZIP-архивы GameSaves для этой игры из подключённой папки.".into());
        }
        let count = selected.len();
        self.selected_archives
            .entry(game.into())
            .or_default()
            .extend(selected);
        if let Some(refresh) = tokens.refresh_token {
            self.refresh = refresh;
        }
        self.access = Arc::new(Mutex::new(Some(Access::new(
            tokens.access_token,
            tokens.expires_in,
        ))));
        Ok((self, count))
    }

    pub fn list(&self, game: &str) -> Result<Vec<(String, PathBuf)>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Page {
            #[serde(default)]
            files: Vec<RemoteFile>,
            next_page_token: Option<String>,
        }
        let token = self.token()?;
        let agent = http(30);
        let mut page_token: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        let mut rows = Vec::new();
        let explicit = self
            .selected_archives
            .get(game)
            .filter(|ids| !ids.is_empty());
        loop {
            let mut url = list_url(&self.folder.id, explicit.is_none().then_some(game))?;
            if let Some(next) = &page_token {
                url.query_pairs_mut().append_pair("pageToken", next);
            }
            let response = agent
                .get(url.as_str())
                .header("Authorization", bearer(&token))
                .call()
                .map_err(request_error)?;
            let page: Page = json(response)?;
            rows.extend(
                page.files
                    .into_iter()
                    .filter(|file| file.matches_game(game, explicit))
                    .filter_map(RemoteFile::row),
            );
            match page.next_page_token {
                Some(next) if !next.is_empty() && seen.insert(next.clone()) => {
                    page_token = Some(next)
                }
                Some(_) => return Err("Google прислал повторяющуюся страницу истории.".into()),
                None => break,
            }
        }
        rows.sort_unstable_by(|a, b| {
            b.1.file_name()
                .cmp(&a.1.file_name())
                .then_with(|| a.1.cmp(&b.1))
        });
        Ok(rows)
    }

    pub fn restore(&self, version: &Path, destination: &Path) -> Result<PathBuf> {
        let id = version
            .parent()
            .and_then(Path::to_str)
            .filter(|id| valid_id(id))
            .ok_or("Не найдена версия на Диске.")?;
        let mut response = http(600)
            .get(format!("{FILES}/{id}?alt=media&supportsAllDrives=true"))
            .header("Authorization", bearer(&self.token()?))
            .call()
            .map_err(request_error)?;
        if response.status() != 200 {
            return Err("Google не вернул архив.".into());
        }
        let mut tmp = tempfile::NamedTempFile::new().map_err(error)?;
        let size = std::io::copy(
            &mut response.body_mut().as_reader().take(MAX_ARCHIVE + 1),
            tmp.as_file_mut(),
        )
        .map_err(error)?;
        if size > MAX_ARCHIVE {
            return Err("Архив на Диске превышает лимит GameSaves.".into());
        }
        if destination.is_file() {
            let mut zip = zip::ZipArchive::new(tmp.reopen().map_err(error)?).map_err(error)?;
            let item = zip.by_name("save").map_err(error)?;
            if item.compression() != zip::CompressionMethod::Stored
                || item.size() != item.compressed_size()
                || item.size() > MAX_ARCHIVE
            {
                return Err("Некорректный архив сохранения.".into());
            }
        }
        snapshots::restore(tmp.path(), destination).map_err(error)
    }
}

fn list_url(folder: &str, game: Option<&str>) -> Result<Url> {
    if !valid_id(folder) || game.is_some_and(|key| !valid_id(key)) {
        return Err("Некорректный идентификатор папки или игры.".into());
    }
    let mut query = format!("'{folder}' in parents and trashed = false");
    if let Some(game) = game {
        query.push_str(&format!(
            " and appProperties has {{ key='gamesaves' and value='{game}' }}"
        ));
    }
    let mut url = Url::parse(FILES).unwrap();
    url.query_pairs_mut()
        .append_pair("q", &query)
        .append_pair("fields", "nextPageToken,files(id,name,appProperties)")
        .append_pair("pageSize", "100")
        .append_pair("supportsAllDrives", "true")
        .append_pair("includeItemsFromAllDrives", "true");
    Ok(url)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteFile {
    id: String,
    name: String,
    #[serde(default)]
    parents: Vec<String>,
    #[serde(default)]
    trashed: bool,
    #[serde(default)]
    app_properties: std::collections::BTreeMap<String, String>,
}
impl RemoteFile {
    fn matches_game(
        &self,
        game: &str,
        explicit: Option<&std::collections::BTreeSet<String>>,
    ) -> bool {
        match self.app_properties.get("gamesaves") {
            Some(tag) => tag == game,
            None => explicit.is_some_and(|ids| ids.contains(&self.id)),
        }
    }

    fn row(self) -> Option<(String, PathBuf)> {
        if !valid_id(&self.id) {
            return None;
        }
        let timestamp = snapshots::version_timestamp(&self.name)?;
        let author = self.app_properties.get("author").map(String::as_str);
        Some((
            snapshots::version_label(timestamp, author),
            PathBuf::from(self.id).join(self.name),
        ))
    }
}

pub fn game_key(game: &Game) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let name = match game.id.as_str() {
        "steam:1962700" => "subnautica 2".into(),
        "steam:892970" => "valheim".into(),
        _ => game
            .name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase(),
    };
    URL_SAFE_NO_PAD.encode(ring::digest::digest(&ring::digest::SHA256, name.as_bytes()).as_ref())
}
fn upload_session(input: &str) -> Option<Url> {
    let url = Url::parse(input).ok()?;
    (url.scheme() == "https"
        && url.host_str() == Some("www.googleapis.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.path() == "/upload/drive/v3/files")
        .then_some(url)
}
pub fn folder_from_link(input: &str) -> Option<String> {
    let url = Url::parse(input.trim()).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("drive.google.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let segments: Vec<_> = url.path().trim_end_matches('/').split('/').collect();
    let id = match segments.as_slice() {
        ["", "drive", "folders", id] | ["", "folders", id] => *id,
        ["", "drive", "u", account, "folders", id]
            if !account.is_empty() && account.bytes().all(|b| b.is_ascii_digit()) =>
        {
            *id
        }
        ["", "open"] => {
            return url
                .query_pairs()
                .find(|(key, id)| key == "id" && valid_id(id))
                .map(|(_, id)| id.into_owned());
        }
        _ => return None,
    };
    valid_id(id).then(|| id.to_owned())
}

fn valid_id(id: &str) -> bool {
    (1..=200).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}
fn http(seconds: u64) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .https_only(true)
            .tls_config(tls)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(seconds)))
            .build(),
    )
}
fn json<T: DeserializeOwned>(mut response: ureq::http::Response<ureq::Body>) -> Result<T> {
    if !response.status().is_success() {
        return Err("Google не подтвердил запрос.".into());
    }
    let data = response
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_vec()
        .map_err(|_| "Не удалось прочитать ответ Google.")?;
    serde_json::from_slice(&data).map_err(|_| "Google вернул неполный ответ.".into())
}
fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn request_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(401) => "Доступ Google истёк. Подключи Диск заново.".into(),
        ureq::Error::StatusCode(403) => {
            "Google отклонил запрос. Проверь право записи, свободное место и настройки Drive API."
                .into()
        }
        ureq::Error::StatusCode(404) => "Папка или архив не найдены на Google Диске.".into(),
        ureq::Error::StatusCode(code) => format!("Google Диск вернул ошибку HTTP {code}."),
        ureq::Error::HostNotFound => "Не удалось определить адрес Google (DNS).".into(),
        ureq::Error::Timeout(_) => "Google не ответил вовремя.".into(),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => {
            "Не удалось установить защищённое соединение с Google (TLS).".into()
        }
        ureq::Error::InvalidProxyUrl | ureq::Error::ConnectProxyFailed(_) => {
            "Не удалось соединиться с Google через прокси.".into()
        }
        ureq::Error::Io(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            "Не удалось проверить защищённое соединение с Google (TLS).".into()
        }
        ureq::Error::Io(e) => match e.raw_os_error() {
            Some(code) => format!("Соединение с Google прервалось (код системы {code})."),
            None => format!("Соединение с Google прервалось ({}).", e.kind()),
        },
        ureq::Error::Protocol(_) => "Получен некорректный сетевой ответ от Google.".into(),
        _ => "Не удалось установить соединение с Google.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_links_extract_only_supported_drive_addresses() {
        for link in [
            "https://drive.google.com/drive/folders/folder-id?usp=sharing",
            "https://drive.google.com/drive/u/0/folders/folder-id/",
            "https://drive.google.com/open?id=folder-id",
        ] {
            assert_eq!(folder_from_link(link).as_deref(), Some("folder-id"));
        }
        for link in [
            "https://drive.google.com.evil.net/drive/folders/folder-id",
            "http://drive.google.com/drive/folders/folder-id",
            "https://drive.google.com/file/d/file-id",
            "https://drive.google.com/drive/folders/a%2Fb",
        ] {
            assert!(folder_from_link(link).is_none());
        }
    }

    #[test]
    fn connection_errors_identify_the_cause() {
        assert!(request_error(ureq::Error::HostNotFound).contains("DNS"));
        assert!(request_error(ureq::Error::Tls("certificate")).contains("TLS"));
        assert!(
            request_error(ureq::Error::Io(std::io::Error::from_raw_os_error(10060)))
                .contains("10060")
        );
        assert!(request_error(ureq::Error::InvalidProxyUrl).contains("прокси"));
    }

    #[test]
    fn folder_connection_survives_a_restart() -> Result<()> {
        let registration = serde_json::json!({
            "client": {"client_id":"fixture.apps.googleusercontent.com", "client_secret":"fixture"},
            "refresh":"refresh-token", "folder":{"id":"folder-id","name":"Запасные сейвы"}
        });
        let drive: Drive = serde_json::from_value(registration.clone()).map_err(error)?;
        let dir = tempfile::tempdir().map_err(error)?;
        let credentials = dir.path().join("google-drive.dat");
        assert!(load(&credentials)?.is_none());
        save(&credentials, &drive)?;
        let restarted = load(&credentials)?.expect("drive connection");
        assert_eq!(
            restarted.folder.link(),
            "https://drive.google.com/drive/folders/folder-id"
        );
        assert_eq!(
            serde_json::to_value(restarted).map_err(error)?,
            registration
        );
        #[cfg(windows)]
        assert!(
            !std::fs::read(&credentials)
                .map_err(error)?
                .windows(13)
                .any(|s| s == b"refresh-token")
        );
        Ok(())
    }

    #[test]
    fn upload_is_pinned_to_google() {
        assert!(
            upload_session("https://www.googleapis.com/upload/drive/v3/files?upload_id=1")
                .is_some()
        );
        for url in [
            "http://www.googleapis.com/upload/drive/v3/files",
            "https://other.example/upload/drive/v3/files",
            "https://www.googleapis.com@other.example/upload/drive/v3/files",
            "https://www.googleapis.com/upload/elsewhere",
        ] {
            assert!(upload_session(url).is_none());
        }
    }
    #[test]
    fn history_request_and_rows_use_safe_ids() -> Result<()> {
        assert!(list_url("../../outside", Some("game")).is_err());
        let url = list_url("folder", Some("game"))?;
        assert!(url.query_pairs().any(|(key, value)| key == "q"
            && value.contains("'folder' in parents")
            && value.contains("value='game'")));
        let row = RemoteFile {
            id: "download-id".into(),
            name: "save-001234567890123-ver.zip".into(),
            parents: vec![],
            trashed: false,
            app_properties: [("author".into(), "Ваня".into())].into(),
        }
        .row()
        .unwrap();
        assert!(row.0.ends_with("Ваня"));
        assert_eq!(
            row.1,
            PathBuf::from("download-id").join("save-001234567890123-ver.zip")
        );
        Ok(())
    }
    #[test]
    fn manually_selected_archive_can_join_only_the_intended_history() {
        let mut file: RemoteFile = serde_json::from_value(serde_json::json!({
            "id":"archive1", "name":"save-001234567890123-ver.zip"
        }))
        .unwrap();
        assert!(!file.matches_game("playing", None));
        let selected = ["archive1".into()].into();
        assert!(file.matches_game("playing", Some(&selected)));
        file.app_properties
            .insert("gamesaves".into(), "different".into());
        assert!(!file.matches_game("playing", Some(&selected)));
    }
}
