#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod catalog;
mod peers;
mod snapshots;

use catalog::{Game, Settings};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

struct App {
    settings: Settings,
    settings_path: PathBuf,
    config_error: Option<String>,
    games: Vec<Game>,
    search: String,
    selected: Option<String>,
    save_path: String,
    candidates: Vec<PathBuf>,
    shared_path: String,
    world_code: String,
    versions: Vec<(String, PathBuf)>,
    version: Option<PathBuf>,
    restore_confirmation: Option<(PathBuf, PathBuf)>,
    job: Option<Receiver<Result<String, String>>>,
    network: Option<peers::Network>,
    last_refresh: Instant,
    new_game_name: String,
    new_game_dir: String,
    status: String,
}

impl App {
    fn new() -> Self {
        let settings_path = catalog::settings_path();
        let (settings, config_error) = match Settings::load(&settings_path) {
            Ok(settings) => (settings, None),
            Err(error) => (Settings::default(), Some(error)),
        };
        let games = catalog::discover(&settings.custom_games);
        let shared_path = settings.shared_path.to_string_lossy().into_owned();
        let network = settings_path
            .parent()
            .and_then(|path| peers::Network::start(path.to_path_buf()).ok());
        let app = Self {
            settings,
            settings_path,
            config_error,
            games,
            search: String::new(),
            selected: None,
            save_path: String::new(),
            candidates: Vec::new(),
            shared_path,
            world_code: String::new(),
            versions: Vec::new(),
            version: None,
            restore_confirmation: None,
            job: None,
            network,
            last_refresh: Instant::now(),
            new_game_name: String::new(),
            new_game_dir: String::new(),
            status: String::new(),
        };
        app.configure_network();
        app
    }

    fn select(&mut self, id: String) {
        self.save_path = self
            .settings
            .save_paths
            .get(&id)
            .map_or_else(String::new, |path| path.to_string_lossy().into_owned());
        let game = self.games.iter().find(|game| game.id == id);
        self.candidates = game.map(catalog::save_candidates).unwrap_or_default();
        self.world_code = self
            .settings
            .world_codes
            .get(&id)
            .cloned()
            .unwrap_or_else(|| {
                game.map_or_else(String::new, |game| {
                    game.name
                        .split(|c: char| !c.is_ascii_alphanumeric())
                        .filter(|word| !word.is_empty())
                        .collect::<Vec<_>>()
                        .join("-")
                        .to_ascii_lowercase()
                })
            });
        self.selected = Some(id);
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
                store: "Добавлена вручную".into(),
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
        self.select(id);
        self.status = "Игра добавлена.".into();
    }

    fn remember_path(&mut self, id: &str) {
        let path = PathBuf::from(self.save_path.trim());
        if !path.is_absolute() || !(path.is_file() || path.is_dir()) {
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

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.heading("Мои игры");
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Поиск"));
        if ui.button("Обновить список").clicked() {
            self.games = catalog::discover(&self.settings.custom_games);
            if let Some(game) = self
                .games
                .iter()
                .find(|game| Some(&game.id) == self.selected.as_ref())
            {
                self.candidates = catalog::save_candidates(game);
            }
        }
        ui.separator();
        let query = self.search.trim().to_lowercase();
        let mut selection = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut matches = 0;
            for game in &self.games {
                if game.name.to_lowercase().contains(&query) {
                    matches += 1;
                    let selected = self.selected.as_ref() == Some(&game.id);
                    let label = format!("{} · {}", game.name, game.store);
                    if ui.selectable_label(selected, label).clicked() {
                        selection = Some(game.id.clone());
                    }
                }
            }
            if matches == 0 {
                ui.label("Игры не найдены.");
            }
        });
        if let Some(id) = selection {
            self.select(id);
        }
    }

    fn manual_game(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Добавить игру вручную", |ui| {
            ui.label("Название");
            ui.text_edit_singleline(&mut self.new_game_name);
            ui.label("Папка установки");
            path_field(ui, &mut self.new_game_dir, false);
            if ui
                .add_enabled(self.config_error.is_none(), egui::Button::new("Добавить"))
                .clicked()
            {
                self.add_game();
            }
        });
    }

    fn game_details(&mut self, ui: &mut egui::Ui, game: &Game) {
        ui.heading(&game.name);
        ui.label(&game.store);
        ui.label(format!("Папка установки: {}", game.install_dir.display()));
        ui.add_space(16.0);
        ui.heading("Сохранения");
        if self.candidates.is_empty() {
            ui.label("Укажи файл или папку сохранений игры.");
        } else {
            egui::ComboBox::from_id_salt("save-candidates")
                .selected_text("Найденные сохранения")
                .show_ui(ui, |ui| {
                    for candidate in &self.candidates {
                        let path = candidate.to_string_lossy().into_owned();
                        if ui.selectable_label(self.save_path == path, &path).clicked() {
                            self.save_path = path;
                        }
                    }
                });
        }
        path_field(ui, &mut self.save_path, true);
        if ui
            .add_enabled(
                self.config_error.is_none(),
                egui::Button::new("Запомнить путь"),
            )
            .clicked()
        {
            self.remember_path(&game.id);
        }
        if let Some(path) = self.settings.save_paths.get(&game.id) {
            ui.label(format!("Сохранённый путь: {}", path.display()));
            if !path.exists() {
                ui.label("Этот путь больше недоступен. Выбери новый.");
            }
        }
    }

    fn world_with_code(&self, code: &str) -> Option<peers::World> {
        let shared = snapshots::world_directory(&self.settings.shared_path, code).ok()?;
        peers::World::new(shared, self.settings_path.parent()?)
    }

    fn world(&self) -> Option<peers::World> {
        let code = self.settings.world_codes.get(self.selected.as_ref()?)?;
        self.world_with_code(code)
    }

    fn configure_network(&self) {
        if let Some(network) = &self.network {
            network.configure(
                self.settings
                    .world_codes
                    .values()
                    .filter_map(|code| self.world_with_code(code))
                    .collect(),
            );
        }
    }

    fn refresh_versions(&mut self) {
        self.last_refresh = Instant::now();
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

    fn sync_settings(&mut self, id: &str) {
        let shared = PathBuf::from(self.shared_path.trim());
        let code = self.world_code.trim();
        if !shared.is_dir() {
            self.status = "Выбери существующую общую папку.".into();
            return;
        }
        if let Err(error) = snapshots::world_directory(&shared, code) {
            self.status = error.to_string();
            return;
        }
        let mut updated = self.settings.clone();
        updated.shared_path = shared;
        updated.world_codes.insert(id.to_owned(), code.to_owned());
        if let Err(error) = updated.write(&self.settings_path) {
            self.status = format!("Не удалось запомнить общую папку. {error}");
            return;
        }
        self.settings = updated;
        self.configure_network();
        self.status.clear();
        self.refresh_versions();
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
        self.refresh_versions();
    }

    fn sync_panel(&mut self, ui: &mut egui::Ui, game: &Game) {
        ui.add_space(16.0);
        ui.heading("Обмен сохранениями");
        let world = self.world();
        egui::CollapsingHeader::new("Настройка обмена")
            .id_salt(&game.id)
            .default_open(world.is_none())
            .show(ui, |ui| {
                ui.label("Выберите одну общую папку и одинаковый код мира на обоих ПК.");
                path_field(ui, &mut self.shared_path, false);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Код мира");
                    ui.text_edit_singleline(&mut self.world_code);
                    if ui
                        .add_enabled(
                            self.config_error.is_none(),
                            egui::Button::new("Подключить папку"),
                        )
                        .clicked()
                    {
                        self.sync_settings(&game.id);
                    }
                });
            });

        let Some(world) = world else {
            return;
        };
        let chosen = self
            .settings
            .save_paths
            .get(&game.id)
            .filter(|path| path.is_file())
            .cloned();
        if chosen.is_none() {
            ui.label("Для обмена выбери и запомни конкретный файл сохранения. Папки можно сохранять в каталоге игр.");
        }
        ui.label("Закрой игру перед отправкой или восстановлением.");
        if ui
            .add_enabled(chosen.is_some(), egui::Button::new("Отправить сохранение"))
            .clicked()
        {
            if let Some(source) = chosen.clone() {
                let sender = self.network.as_ref().map(peers::Network::sender);
                self.start_job(ui.ctx(), move || {
                    peers::deliver(sender.as_ref(), &world, &source)
                });
            }
        }
        if self.versions.is_empty() {
            ui.label("Пока нет версий сохранения.");
            return;
        }
        ui.horizontal_wrapped(|ui| {
            let label = self
                .versions
                .iter()
                .find(|(_, path)| Some(path) == self.version.as_ref())
                .map_or("Выбери версию", |(label, _)| label);
            egui::ComboBox::from_id_salt("shared-versions")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (label, path) in &self.versions {
                        if ui
                            .selectable_label(Some(path) == self.version.as_ref(), label)
                            .clicked()
                        {
                            self.version = Some(path.clone());
                        }
                    }
                });
            if let (Some(version), Some(destination)) = (self.version.clone(), chosen) {
                if ui.button("Восстановить версию").clicked() {
                    self.restore_confirmation = Some((version, destination));
                }
            }
        });
    }

    fn confirm_restore(&mut self, ctx: &egui::Context) {
        let Some((version, destination)) = self.restore_confirmation.clone() else {
            return;
        };
        egui::Modal::new(egui::Id::new("confirm-restore")).show(ctx, |ui| {
            ui.heading("Восстановить сохранение?");
            ui.label(format!(
                "Будет заменён файл {}. Его резервная копия останется рядом.",
                destination.display()
            ));
            ui.horizontal(|ui| {
                if ui.button("Восстановить").clicked() {
                    self.restore_confirmation = None;
                    self.start_job(ctx, move || {
                        snapshots::restore(&version, &destination)
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

fn path_field(ui: &mut egui::Ui, path: &mut String, allow_file: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(path)
                .desired_width(340.0)
                .hint_text("Полный путь"),
        );
        #[cfg(windows)]
        {
            if ui.button("Выбрать папку").clicked() {
                if let Some(directory) = rfd::FileDialog::new().pick_folder() {
                    *path = directory.to_string_lossy().into_owned();
                }
            }
            if allow_file && ui.button("Выбрать файл").clicked() {
                if let Some(file) = rfd::FileDialog::new().pick_file() {
                    *path = file.to_string_lossy().into_owned();
                }
            }
        }
        #[cfg(not(windows))]
        let _ = allow_file;
    });
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.check_job();
        let idle = self.job.is_none();
        if self.selected.is_some() && idle {
            let interval = Duration::from_secs(2);
            if self.last_refresh.elapsed() >= interval {
                self.refresh_versions();
            }
            ctx.request_repaint_after(interval.saturating_sub(self.last_refresh.elapsed()));
        }
        egui::SidePanel::left("game-list")
            .default_width(245.0)
            .show(ctx, |ui| {
                ui.add_enabled_ui(idle, |ui| self.sidebar(ui));
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                ui.add_enabled_ui(idle, |ui| self.manual_game(ui));
                ui.separator();
                if let Some(error) = &self.config_error {
                    ui.label(format!(
                        "Не удалось открыть настройки {}. {}",
                        self.settings_path.display(),
                        error
                    ));
                }
                let game = self
                    .selected
                    .as_ref()
                    .and_then(|id| self.games.iter().find(|game| game.id == *id))
                    .cloned();
                if let Some(game) = game {
                    ui.add_enabled_ui(idle, |ui| {
                        self.game_details(ui, &game);
                        self.sync_panel(ui, &game);
                    });
                } else {
                    ui.heading("Выбери игру из списка");
                    ui.label("Если игры нет в списке, добавь её вручную.");
                }
                if !self.status.is_empty() {
                    ui.add_space(8.0);
                    ui.label(&self.status);
                }
            });
        });
        self.confirm_restore(ctx);
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([940.0, 720.0])
            .with_min_inner_size([715.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Игровые сохранения",
        options,
        Box::new(|_| Ok(Box::new(App::new()))),
    )
}
