#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod artwork;
mod catalog;
mod demo;
mod drive;
mod peers;
mod snapshots;
mod ui;
mod world_folder;

use catalog::{Game, Settings};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

struct App {
    nickname_input: String,
    drive: Option<drive::Drive>,
    connect_drive: Option<Receiver<drive::Result<drive::Drive>>>,
    cloud_list: Option<Receiver<drive::Result<Vec<(String, PathBuf)>>>>,
    cloud_checked: Option<Instant>,
    cloud_error: String,
    jump_to_history: bool,
    demo: demo::Demo,
    demo_mode: bool,
    friend_window: Option<std::process::Child>,
    demo_level: String,
    settings: Settings,
    settings_path: PathBuf,
    config_error: Option<String>,
    games: Vec<Game>,
    artwork: artwork::Artwork,
    show_setup: bool,
    show_add: bool,
    search: String,
    selected: Option<String>,
    save_path: String,
    candidates: Vec<PathBuf>,
    versions: Vec<(String, PathBuf)>,
    version: Option<PathBuf>,
    restore_confirmation: Option<(PathBuf, PathBuf)>,
    job: Option<Receiver<Result<String, String>>>,
    network: Option<peers::Network>,
    last_refresh: Instant,
    saves_checked: Instant,
    new_game_name: String,
    new_game_dir: String,
    status: String,
}

impl App {
    fn new(context: &egui::Context) -> Self {
        let demo = demo::Demo::new(
            catalog::settings_path().with_file_name("demo"),
            std::env::args_os().any(|arg| arg == "--friend"),
        );
        let demo_mode = std::env::args_os().any(|a| a == "--demo" || a == "--friend");
        let setup_error = demo_mode
            .then(|| demo.prepare().err())
            .flatten()
            .map(|e| format!("Не удалось подготовить демоигру. {e}"));
        let settings_path = if demo_mode {
            demo.settings_path()
        } else {
            catalog::settings_path()
        };
        let (mut settings, config_error) = match Settings::load(&settings_path) {
            Ok(settings) => (settings, None),
            Err(error) => (Settings::default(), Some(error)),
        };
        let config_error = setup_error.or(config_error);
        let games = catalog::discover(&settings.custom_games);
        let network = demo_mode
            .then(|| {
                settings_path
                    .parent()
                    .and_then(|path| peers::Network::start(path.to_path_buf()).ok())
            })
            .flatten();
        let artwork = artwork::Artwork::new(
            context,
            settings_path.parent().unwrap_or(std::path::Path::new(".")),
        );
        if !snapshots::valid_nickname(&settings.nickname) {
            settings.nickname = if demo.profile == "friend" {
                "Друг"
            } else {
                "Игрок"
            }
            .into();
        }
        let (drive, drive_error) = if demo_mode {
            (None, String::new())
        } else {
            match drive::load(&settings_path.with_file_name("google-drive.dat")) {
                Ok(drive) => (drive, String::new()),
                Err(e) => (None, e),
            }
        };
        let mut app = Self {
            nickname_input: settings.nickname.clone(),
            drive,
            connect_drive: None,
            cloud_list: None,
            cloud_checked: None,
            cloud_error: drive_error,
            jump_to_history: false,
            demo,
            demo_mode,
            friend_window: None,
            demo_level: String::new(),
            settings,
            settings_path,
            config_error,
            games,
            artwork,
            show_setup: false,
            show_add: false,
            search: String::new(),
            selected: None,
            save_path: String::new(),
            candidates: Vec::new(),
            versions: Vec::new(),
            version: None,
            restore_confirmation: None,
            job: None,
            network,
            last_refresh: Instant::now(),
            saves_checked: Instant::now(),
            new_game_name: String::new(),
            new_game_dir: String::new(),
            status: String::new(),
        };
        app.configure_network();
        let initial = app
            .games
            .iter()
            .find(|game| {
                if demo_mode {
                    game.id == demo::GAME_ID
                } else {
                    app.settings.save_paths.contains_key(&game.id)
                }
            })
            .or_else(|| app.games.first())
            .map(|game| game.id.clone());
        if let Some(id) = initial {
            app.select(id);
        }
        app
    }

    fn select(&mut self, id: String) {
        self.save_path = self
            .settings
            .save_paths
            .get(&id)
            .map_or_else(String::new, |path| path.to_string_lossy().into_owned());
        self.show_setup = false;
        self.cloud_list = None;
        self.cloud_checked = None;
        self.cloud_error.clear();
        self.versions.clear();
        self.version = None;
        self.selected = Some(id);
        self.scan_saves();
        self.status.clear();
        self.refresh_versions();
    }

    fn add_game(&mut self) {
        let name = self.new_game_name.trim();
        let directory = PathBuf::from(self.new_game_dir.trim());
        if name.is_empty() || !directory.is_absolute() || !directory.is_dir() {
            self.status = "Введите название и выберите существующую папку игры.".into();
            return;
        }
        let id = format!("manual:{name}:{}", directory.display());
        if !self.settings.custom_games.iter().any(|game| game.id == id) {
            self.settings.custom_games.push(Game {
                id: id.clone(),
                name: name.into(),
                store: "Вручную".into(),
                install_dir: directory,
            });
            if let Err(error) = self.settings.write(&self.settings_path) {
                self.settings.custom_games.pop();
                self.status = format!("Не удалось добавить игру. {error}");
                return;
            }
            self.games = catalog::discover(&self.settings.custom_games);
        }
        self.search.clear();
        self.new_game_name.clear();
        self.new_game_dir.clear();
        self.show_add = false;
        self.select(id);
        self.status = "Игра добавлена.".into();
    }

    fn scan_saves(&mut self) {
        self.saves_checked = Instant::now();
        self.candidates = self
            .selected
            .as_ref()
            .and_then(|id| self.games.iter().find(|game| &game.id == id))
            .map(catalog::save_candidates)
            .unwrap_or_default();
    }

    fn active_save(&self, game_id: &str) -> Option<&std::path::Path> {
        catalog::active_save(
            self.settings.save_paths.get(game_id).map(PathBuf::as_path),
            &self.candidates,
        )
    }

    fn remember_path(&mut self, id: &str) {
        let path = PathBuf::from(self.save_path.trim());
        if !catalog::usable_save(&path) {
            self.status = "Выберите существующий файл или папку сохранений.".into();
            return;
        }
        let previous = self.settings.save_paths.insert(id.to_owned(), path);
        match self.settings.write(&self.settings_path) {
            Ok(()) => self.status = "Путь сохранён.".into(),
            Err(error) => {
                if let Some(path) = previous {
                    self.settings.save_paths.insert(id.to_owned(), path);
                } else {
                    self.settings.save_paths.remove(id);
                }
                self.status = format!("Не удалось сохранить путь. {error}");
            }
        }
    }

    fn world_with_code(&self, code: &str) -> Option<peers::World> {
        if !self.demo_mode {
            return None;
        }
        let shared = snapshots::world_directory(&self.demo.root.join("shared"), code).ok()?;
        peers::World::new(shared, self.settings_path.parent()?)
    }

    fn world(&self) -> Option<peers::World> {
        (self.selected.as_deref() == Some(demo::GAME_ID))
            .then(|| self.world_with_code(demo::WORLD_CODE))
            .flatten()
    }

    fn configure_network(&self) {
        if let Some(network) = &self.network {
            network.configure(self.world_with_code(demo::WORLD_CODE).into_iter().collect());
        }
    }

    fn refresh_versions(&mut self) {
        self.last_refresh = Instant::now();
        if self.selected.as_deref() == Some(demo::GAME_ID) {
            self.demo_level = self
                .demo
                .level()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "неизвестен".into());
        }
        if self.drive.is_some() {
            return;
        }
        let inspect_older = self
            .version
            .as_ref()
            .filter(|path| {
                self.versions
                    .first()
                    .is_some_and(|(_, latest)| *path != latest)
            })
            .cloned();
        self.versions = self
            .world()
            .map(|world| {
                let local = snapshots::versions(&world.local).unwrap_or_default();
                let shared = snapshots::versions(&world.shared).unwrap_or_default();
                let mut seen = std::collections::HashSet::new();
                let mut versions: Vec<_> = local
                    .into_iter()
                    .chain(shared)
                    .filter(|(_, path)| seen.insert(path.file_name().map(|name| name.to_owned())))
                    .collect();
                versions.sort_by(|left, right| right.1.file_name().cmp(&left.1.file_name()));
                versions
            })
            .unwrap_or_default();
        self.version = inspect_older
            .filter(|path| self.versions.iter().any(|(_, current)| current == path))
            .or_else(|| self.versions.first().map(|(_, path)| path.clone()));
    }

    fn start_drive_connection(&mut self, client: drive::Client, ctx: &egui::Context) {
        if self.connect_drive.is_some() {
            return;
        }
        self.cloud_error.clear();
        let (tx, rx) = mpsc::channel();
        self.connect_drive = Some(rx);
        let context = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(drive::Drive::connect(client));
            context.request_repaint();
        });
    }

    fn reset_drive_history(&mut self) {
        self.cloud_list = None;
        self.cloud_checked = None;
        self.versions.clear();
        self.version = None;
        self.cloud_error.clear();
        self.restore_confirmation = None;
    }

    fn poll_drive(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.connect_drive {
            match rx.try_recv() {
                Ok(Ok(new_drive)) => {
                    self.connect_drive = None;
                    match drive::save(
                        &self.settings_path.with_file_name("google-drive.dat"),
                        &new_drive,
                    ) {
                        Ok(()) => {
                            self.drive = Some(new_drive);
                            self.reset_drive_history();
                            self.status = "Google Диск подключён.".into();
                        }
                        Err(e) => self.cloud_error = e,
                    }
                }
                Ok(Err(e)) => {
                    self.connect_drive = None;
                    self.cloud_error = e;
                }
                Err(TryRecvError::Disconnected) => {
                    self.connect_drive = None;
                    self.cloud_error = "Подключение прервано.".into();
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let Some(drive) = self.drive.clone() else {
            return;
        };
        let Some(game) = self
            .selected
            .as_ref()
            .and_then(|id| self.games.iter().find(|game| &game.id == id))
        else {
            return;
        };
        if let Some(rx) = &self.cloud_list {
            match rx.try_recv() {
                Err(TryRecvError::Empty) => return,
                Ok(Ok(rows)) => {
                    let previous = self.version.take();
                    let previous_was_latest =
                        previous.as_ref() == self.versions.first().map(|(_, p)| p);
                    self.version = previous
                        .filter(|v| !previous_was_latest && rows.iter().any(|(_, p)| p == v))
                        .or_else(|| rows.first().map(|(_, p)| p.clone()));
                    self.versions = rows;
                    self.cloud_error.clear();
                }
                Ok(Err(e)) => self.cloud_error = e,
                Err(TryRecvError::Disconnected) => {
                    self.cloud_error = "Не удалось обновить историю Google Диска.".into()
                }
            }
            self.cloud_list = None;
        }
        if self
            .cloud_checked
            .is_some_and(|t| t.elapsed().as_secs() < 15)
        {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.cloud_list = Some(rx);
        self.cloud_checked = Some(Instant::now());
        let key = drive::game_key(game);
        let context = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(drive.list(&key));
            context.request_repaint();
        });
    }

    fn start_job(
        &mut self,
        context: &egui::Context,
        job: impl FnOnce() -> Result<String, String> + Send + 'static,
    ) {
        let (sender, receiver) = mpsc::channel();
        let context = context.clone();
        self.status = "Работа с сохранением…".into();
        self.job = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(job());
            context.request_repaint();
        });
    }

    fn check_job(&mut self) {
        let Some(job) = &self.job else { return };
        self.status = match job.try_recv() {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => format!("Операция прервана. {error}"),
            Err(TryRecvError::Disconnected) => "Операция завершилась с ошибкой.".into(),
            Err(TryRecvError::Empty) => return,
        };
        self.job = None;
        self.cloud_list = None;
        self.cloud_checked = None;
        self.refresh_versions();
    }

    fn confirm_restore(&mut self, ctx: &egui::Context) {
        let Some((version, destination)) = self.restore_confirmation.clone() else {
            return;
        };
        egui::Modal::new(egui::Id::new("confirm-restore"))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .inner_margin(24)
                    .corner_radius(14),
            )
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.heading("Восстановить сохранение?");
                ui.label(format!(
                    "Будет восстановлено сохранение {}. Его резервная копия останется рядом.",
                    destination.display()
                ));
                ui.horizontal(|ui| {
                    if ui.button("Восстановить").clicked() {
                        self.restore_confirmation = None;
                        let drive = self.drive.clone();
                        self.start_job(ctx, move || {
                            match drive {
                                Some(drive) => drive.restore(&version, &destination),
                                None => snapshots::restore(&version, &destination)
                                    .map_err(|e| e.to_string()),
                            }
                            .map(|backup| {
                                format!(
                                    "Сохранение восстановлено. Резервная копия: {}",
                                    backup.display()
                                )
                            })
                            .map_err(|error| error.to_string())
                        });
                    }
                    if ui.button("Отмена").clicked() {
                        self.restore_confirmation = None;
                    }
                });
            });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([840.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        if std::env::args_os().any(|arg| arg == "--friend") {
            "GameSaves Demo · Друг"
        } else if std::env::args_os().any(|arg| arg == "--demo") {
            "GameSaves Demo · Игрок"
        } else {
            "GameSaves"
        },
        options,
        Box::new(|creation| {
            ui::configure(&creation.egui_ctx);
            Ok(Box::new(App::new(&creation.egui_ctx)))
        }),
    )
}
