use super::{Result, http, json};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{digest, rand};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, BufWriter, Read, Write},
    net::TcpListener,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use url::Url;

const AUTHORIZE: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

#[derive(Clone, Deserialize, Serialize)]
pub struct Client {
    client_id: String,
    client_secret: String,
}
impl Client {
    pub fn bundled() -> Option<Self> {
        let client_id = option_env!("GAMESAVES_GOOGLE_CLIENT_ID")?;
        let client_secret = option_env!("GAMESAVES_GOOGLE_CLIENT_SECRET")?;
        let client = Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
        };
        client.valid().then_some(client)
    }

    #[cfg(any(windows, test))]
    pub fn load(path: &std::path::Path) -> Result<Self> {
        #[derive(Deserialize)]
        struct Registration {
            installed: Client,
        }
        let file = std::fs::File::open(path).map_err(super::error)?;
        let mut data = Vec::new();
        file.take(16385)
            .read_to_end(&mut data)
            .map_err(super::error)?;
        if data.len() > 16384 {
            return Err("Файл регистрации слишком большой.".into());
        }
        let client = serde_json::from_slice::<Registration>(&data)
            .map_err(|_| "Нужен JSON регистрации Google типа «Приложение для компьютера».")?
            .installed;
        if !client.valid() {
            return Err("Некорректная регистрация приложения Google.".into());
        }
        Ok(client)
    }

    fn valid(&self) -> bool {
        self.client_id.ends_with(".apps.googleusercontent.com")
            && !self.client_secret.trim().is_empty()
    }

    fn token<'a>(&'a self, fields: impl IntoIterator<Item = (&'a str, &'a str)>) -> Result<Tokens> {
        let mut form = vec![
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
        ];
        form.extend(fields);
        let response = http(30).post(TOKEN).send_form(form);
        let result = match response {
            Ok(response) => json(response),
            Err(ureq::Error::StatusCode(400 | 401)) => {
                Err("Google не принял код входа. Подключи Диск заново и выбери аккаунт из списка тестировщиков.".into())
            }
            Err(e) => Err(super::request_error(e)),
        };
        result.map_err(|e| format!("Получение доступа на oauth2.googleapis.com. {e}"))
    }

    pub fn refresh(&self, secret: &str) -> Result<Tokens> {
        self.token([("grant_type", "refresh_token"), ("refresh_token", secret)])
    }
}

#[derive(Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
}

fn random_string() -> Result<String> {
    let mut bytes = [0_u8; 32];
    rand::SecureRandom::fill(&rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| "Не удалось начать безопасный вход.")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub(super) fn check_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        Err("Подключение Google Диска отменено.".into())
    } else {
        Ok(())
    }
}

pub fn authorize(client: &Client, cancelled: &AtomicBool) -> Result<(Tokens, String)> {
    check_cancelled(cancelled)?;
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(super::error)?;
    listener.set_nonblocking(true).map_err(super::error)?;
    let redirect = format!(
        "http://127.0.0.1:{}/oauth",
        listener.local_addr().map_err(super::error)?.port()
    );
    let verifier = random_string()?;
    let state = random_string()?;
    let challenge =
        URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()).as_ref());
    let mut url = Url::parse(AUTHORIZE).unwrap();
    url.query_pairs_mut()
        .append_pair("client_id", &client.client_id)
        .append_pair("redirect_uri", &redirect)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPE)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("trigger_onepick", "true")
        .append_pair("allow_folder_selection", "true")
        .append_pair("mimetypes", "application/vnd.google-apps.folder");
    check_cancelled(cancelled)?;
    webbrowser::open(url.as_str()).map_err(|_| "Не удалось открыть браузер.")?;

    let (code, folder) = wait_for_choice(listener, &redirect, &state, cancelled)?;
    check_cancelled(cancelled)?;
    let tokens = client.token([
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("code_verifier", verifier.as_str()),
        ("redirect_uri", redirect.as_str()),
    ])?;
    Ok((tokens, folder))
}

fn wait_for_choice(
    listener: TcpListener,
    redirect: &str,
    state: &str,
    cancelled: &AtomicBool,
) -> Result<(String, String)> {
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        check_cancelled(cancelled)?;
        let mut socket = match listener.accept() {
            Ok((socket, _)) => socket,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(125));
                continue;
            }
            Err(e) => return Err(super::error(e)),
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(super::error)?;
        socket
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(super::error)?;
        let mut first_line = Vec::new();
        let mut reader = BufReader::new(&mut socket).take(16384);
        if reader.read_until(b'\n', &mut first_line).is_err() {
            continue;
        }
        // Drain HTTP headers before answering so the browser can receive the reply.
        let mut header = Vec::new();
        while let Ok(n) = reader.read_until(b'\n', &mut header) {
            if n == 0 || header == b"\r\n" {
                break;
            }
            header.clear();
        }
        let Some(result) = callback(&first_line, redirect, state) else {
            reply(
                &mut socket,
                "400 Bad Request",
                "Не удалось принять ответ Google.",
            );
            continue;
        };
        let (code, folder) = match result {
            Ok(pair) => pair,
            Err(err) => {
                reply(
                    &mut socket,
                    "200 OK",
                    "Папка не выбрана. Вернись в GameSaves.",
                );
                return Err(err);
            }
        };
        reply(
            &mut socket,
            "200 OK",
            "Выбор папки получен. Результат подключения появится в GameSaves.",
        );
        return Ok((code, folder));
    }
    Err("Выбор папки не завершён за 3 минуты. Начни подключение заново.".into())
}

fn callback(line: &[u8], expected: &str, state: &str) -> Option<Result<(String, String)>> {
    let line = std::str::from_utf8(line).ok()?;
    let mut words = line.split_ascii_whitespace();
    if words.next()? != "GET" {
        return None;
    }
    let target = words.next()?.strip_prefix("/oauth?")?;
    let url = Url::parse(&format!("{expected}?{target}")).ok()?;
    let get = |key: &str| {
        url.query_pairs()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.into_owned())
    };
    if get("state").as_deref() != Some(state) {
        return None;
    }
    let result = || {
        let code = get("code")
            .filter(|s| !s.is_empty())
            .ok_or("Вход через Google отменён.")?;
        let folder = get("picked_file_ids")
            .filter(|id| super::valid_id(id))
            .ok_or("Выбери одну папку на Google Диске.")?;
        Ok((code, folder))
    };
    Some(result())
}

fn reply(socket: &mut impl Write, status: &str, text: &str) {
    let body = format!("<!doctype html><meta charset=utf-8><title>GameSaves</title><p>{text}</p>");
    let mut socket = BufWriter::new(socket);
    let _ = write!(
        socket,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let _ = socket.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelled_folder_selection_releases_the_callback_listener() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let cancelled = AtomicBool::new(true);
        let error = wait_for_choice(listener, "", "", &cancelled).unwrap_err();
        assert!(error.contains("отменено"));
        assert!(TcpListener::bind(address).is_ok());
    }

    #[test]
    fn desktop_registration_imports_only_the_installed_client() {
        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(path.path(), br#"{"web":{"client_id":"client.apps.googleusercontent.com","client_secret":"fixture"}}"#).unwrap();
        assert!(Client::load(path.path()).is_err());
        std::fs::write(path.path(), br#"{"installed":{"client_id":"client.apps.googleusercontent.com","client_secret":"fixture"}}"#).unwrap();
        assert!(Client::load(path.path()).is_ok());
    }

    #[test]
    fn callback_requires_session_state_and_exactly_one_folder() {
        let redirect = "http://127.0.0.1:8000/oauth";
        let valid =
            b"GET /oauth?state=random&code=one%2Ftwo&picked_file_ids=folder-ab HTTP/1.1\r\n";
        assert_eq!(
            callback(valid, redirect, "random").unwrap().unwrap(),
            ("one/two".into(), "folder-ab".into())
        );
        assert!(callback(valid, redirect, "another").is_none());
        let multiple = b"GET /oauth?state=random&code=one&picked_file_ids=f,g HTTP/1.1\r\n";
        assert!(callback(multiple, redirect, "random").unwrap().is_err());
    }
}
