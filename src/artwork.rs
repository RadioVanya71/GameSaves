use crate::catalog::Game;
use eframe::egui;
use std::collections::HashMap;
use std::fs::{self, File};
#[cfg(any(windows, test))]
use std::io;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

const LIMIT: u64 = 12 * 1024 * 1024;

struct Request {
    game: Game,
    cover: Option<PathBuf>,
    generation: u64,
}

struct Response {
    id: String,
    generation: u64,
    image: Option<egui::ColorImage>,
}

pub struct Artwork {
    requests: mpsc::Sender<Request>,
    responses: mpsc::Receiver<Response>,
    entries: HashMap<String, Option<egui::TextureHandle>>,
    generation: u64,
}

impl Artwork {
    pub fn new(context: &egui::Context, directory: &Path) -> Self {
        let (requests, queue) = mpsc::channel::<Request>();
        let (output, responses) = mpsc::channel();
        let queue = Arc::new(Mutex::new(queue));
        let steam_cache = steamlocate::locate()
            .ok()
            .map(|steam| steam.path().join("appcache/librarycache"));
        let cache = directory.join("covers");
        let config = ureq::Agent::config_builder()
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(5)))
            .build();
        let http = ureq::Agent::new_with_config(config);
        for _ in 0..3 {
            let (queue, output, context) = (queue.clone(), output.clone(), context.clone());
            let (cache, steam_cache, http) = (cache.clone(), steam_cache.clone(), http.clone());
            std::thread::spawn(move || {
                loop {
                    let request = match queue.lock().expect("artwork queue").recv() {
                        Ok(request) => request,
                        Err(_) => return,
                    };
                    let image = find(
                        &request.game,
                        request.cover.as_deref(),
                        &cache,
                        steam_cache.as_deref(),
                        &http,
                    )
                    .map(|image| {
                        let rgba = fit_image(image).into_rgba8();
                        egui::ColorImage::from_rgba_unmultiplied(
                            [rgba.width() as usize, rgba.height() as usize],
                            rgba.as_raw(),
                        )
                    });
                    if output
                        .send(Response {
                            id: request.game.id,
                            generation: request.generation,
                            image,
                        })
                        .is_err()
                    {
                        return;
                    }
                    context.request_repaint();
                }
            });
        }
        Self {
            requests,
            responses,
            entries: HashMap::new(),
            generation: 0,
        }
    }

    pub fn poll(&mut self, context: &egui::Context) {
        while let Ok(response) = self.responses.try_recv() {
            if response.generation == self.generation {
                let texture = response.image.map(|image| {
                    context.load_texture(&response.id, image, egui::TextureOptions::LINEAR)
                });
                self.entries.insert(response.id, texture);
            }
        }
    }

    pub fn get(&mut self, game: &Game, cover: Option<&Path>) -> Option<egui::TextureHandle> {
        self.entries
            .entry(game.id.clone())
            .or_insert_with(|| {
                let _ = self.requests.send(Request {
                    game: game.clone(),
                    cover: cover.map(Path::to_path_buf),
                    generation: self.generation,
                });
                None
            })
            .clone()
    }

    pub fn rescan(&mut self, games: &[Game]) {
        self.generation += 1;
        let ids: std::collections::HashSet<_> = games.iter().map(|game| game.id.as_str()).collect();
        self.entries
            .retain(|id, texture| texture.is_some() && ids.contains(id.as_str()));
    }

    #[cfg(windows)]
    pub fn refresh(&mut self) {
        self.generation += 1;
        self.entries.clear();
    }
}

fn decode(bytes: &[u8]) -> Option<image::DynamicImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_height = Some(4096);
    limits.max_image_width = Some(4096);
    limits.max_alloc = Some(80 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().ok()
}

fn fit_image(image: image::DynamicImage) -> image::DynamicImage {
    if image.width() > 1100 || image.height() > 700 {
        image.thumbnail(1100, 700)
    } else {
        image
    }
}

fn read_cover(path: &Path) -> Option<image::DynamicImage> {
    let file = File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > LIMIT {
        return None;
    }
    decode(&bytes)
}

#[cfg(any(windows, test))]
pub fn set_custom_cover(source: &Path, directory: &Path) -> io::Result<PathBuf> {
    let image = read_cover(source).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Выбери изображение JPG, PNG или WebP.",
        )
    })?;
    fs::create_dir_all(directory)?;
    let mut pending = tempfile::Builder::new()
        .prefix("cover-")
        .suffix(".png")
        .tempfile_in(directory)?;
    fit_image(image)
        .write_to(pending.as_file_mut(), image::ImageFormat::Png)
        .map_err(io::Error::other)?;
    pending.as_file().sync_all()?;
    let (_, saved) = pending.keep()?;
    Ok(saved)
}

fn steam_id(game: &Game) -> Option<u32> {
    game.id
        .strip_prefix("steam:")
        .and_then(|id| id.parse().ok())
        .or_else(|| {
            game.name
                .eq_ignore_ascii_case("Subnautica 2")
                .then_some(1962700)
        })
}

fn fetch(http: &ureq::Agent, url: &str, limit: u64) -> Option<Vec<u8>> {
    let mut response = http.get(url).call().ok()?;
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .ok()
}

fn trusted_art_url(raw: &str, suffix: &str) -> Option<String> {
    let url = if raw.starts_with("//") {
        format!("https:{raw}")
    } else {
        raw.to_owned()
    };
    let authority = url.strip_prefix("https://")?.split('/').next()?;
    if authority.ends_with(suffix) && !authority.contains(['@', ':', '\\', '?', '#']) {
        Some(url)
    } else {
        None
    }
}

fn steam_cover(http: &ureq::Agent, id: u32) -> Option<Vec<u8>> {
    let cdn = format!(
        "https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{id}/header.jpg"
    );
    if let Some(bytes) = fetch(http, &cdn, LIMIT) {
        return Some(bytes);
    }
    let metadata = fetch(
        http,
        &format!("https://store.steampowered.com/api/appdetails?appids={id}&filters=basic"),
        768 * 1024,
    )?;
    let details: serde_json::Value = serde_json::from_slice(&metadata).ok()?;
    let url = details[id.to_string()]["data"]["header_image"].as_str()?;
    fetch(http, &trusted_art_url(url, ".steamstatic.com")?, LIMIT)
}

fn gog_cover(http: &ureq::Agent, id: u64) -> Option<Vec<u8>> {
    let metadata = fetch(
        http,
        &format!("https://api.gog.com/products/{id}"),
        768 * 1024,
    )?;
    let product: serde_json::Value = serde_json::from_slice(&metadata).ok()?;
    ["background", "boxArt", "logo"].iter().find_map(|key| {
        let raw = product["images"][*key].as_str()?;
        fetch(http, &trusted_art_url(raw, ".gog-statics.com")?, LIMIT)
    })
}

fn cached_steam_cover(cache: &Path, id: u32) -> Option<image::DynamicImage> {
    for name in [
        "library_header",
        "header",
        "library_600x900",
        "library_capsule",
    ] {
        if let Some(image) = read_cover(&cache.join(format!("{id}_{name}.jpg"))) {
            return Some(image);
        }
        let app = cache.join(id.to_string());
        let image_name = format!("{name}.jpg");
        if let Some(image) = read_cover(&app.join(&image_name)) {
            return Some(image);
        }
        if let Ok(entries) = fs::read_dir(app) {
            for entry in entries.flatten() {
                if let Some(image) = read_cover(&entry.path().join(&image_name)) {
                    return Some(image);
                }
            }
        }
    }
    None
}

fn find(
    game: &Game,
    cover: Option<&Path>,
    cache: &Path,
    steam_cache: Option<&Path>,
    http: &ureq::Agent,
) -> Option<image::DynamicImage> {
    if let Some(image) = cover.and_then(read_cover) {
        return Some(image);
    }
    for name in [
        "cover.jpg",
        "cover.png",
        "cover.webp",
        "folder.jpg",
        "folder.png",
    ] {
        if let Some(image) = read_cover(&game.install_dir.join(name)) {
            return Some(image);
        }
    }
    let steam = steam_id(game);
    if let Some(image) = steam
        .zip(steam_cache)
        .and_then(|(id, cache)| cached_steam_cover(cache, id))
    {
        return Some(image);
    }
    let gog = game.id.strip_prefix("gog:").and_then(|id| id.parse().ok());
    let filename = match (steam, gog) {
        (Some(id), _) => format!("steam-{id}.img"),
        (_, Some(id)) => format!("gog-{id}.img"),
        _ => return None,
    };
    let path = cache.join(filename);
    if let Some(image) = read_cover(&path) {
        return Some(image);
    }
    let bytes = match (steam, gog) {
        (Some(id), _) => steam_cover(http, id),
        (_, Some(id)) => gog_cover(http, id),
        _ => None,
    }?;
    let image = decode(&bytes)?;
    if fs::create_dir_all(cache).is_ok() {
        if let Ok(mut file) = tempfile::NamedTempFile::new_in(cache) {
            if file.write_all(&bytes).is_ok() {
                let _ = file.persist(path);
            }
        }
    }
    Some(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_cover_is_loaded_and_can_be_copied() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("cover.png");
        image::RgbImage::new(40, 25)
            .save(&path)
            .map_err(io::Error::other)?;
        let game = Game {
            id: "local:1".into(),
            name: "Game".into(),
            store: "Local".into(),
            install_dir: root.path().to_path_buf(),
        };
        let cache = root.path().join("cache");
        let saved = set_custom_cover(&path, &cache)?;
        fs::remove_file(path)?;
        let image = find(
            &game,
            Some(&saved),
            &cache,
            None,
            &ureq::Agent::new_with_defaults(),
        )
        .unwrap();
        assert_eq!((image.width(), image.height()), (40, 25));
        Ok(())
    }
}
