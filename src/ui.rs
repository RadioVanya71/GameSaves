use super::{App, Game, catalog, peers};
use eframe::egui::{self, Color32, FontId, RichText, Stroke, pos2, vec2};
use std::path::PathBuf;
use std::time::Duration;

const BACKGROUND: Color32 = Color32::from_rgb(12, 18, 28);
const SIDEBAR: Color32 = Color32::from_rgb(16, 24, 37);
const PANEL: Color32 = Color32::from_rgb(20, 31, 47);
const BORDER: Color32 = Color32::from_rgb(42, 57, 77);
const TEXT: Color32 = Color32::from_rgb(236, 243, 250);
const MUTED: Color32 = Color32::from_rgb(159, 179, 199);
const MINT: Color32 = Color32::from_rgb(105, 229, 200);
const INK: Color32 = Color32::from_rgb(8, 41, 44);
const ACTIVE: Color32 = Color32::from_rgb(26, 54, 62);
const ALERT: Color32 = Color32::from_rgb(255, 183, 133);

pub fn configure(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BACKGROUND;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = BACKGROUND;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.selection.bg_fill = ACTIVE;
    style.visuals.selection.stroke = Stroke::new(1.0_f32, MINT);
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(13.0, 7.0);
    style.spacing.interact_size.y = 34.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(14.0));
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widget.corner_radius = 8.into();
        widget.bg_stroke = Stroke::new(1.0_f32, BORDER);
        widget.fg_stroke = Stroke::new(1.0_f32, TEXT);
    }
    style.visuals.widgets.inactive.bg_fill = PANEL;
    style.visuals.widgets.inactive.weak_bg_fill = PANEL;
    style.visuals.widgets.hovered.bg_fill = ACTIVE;
    style.visuals.widgets.hovered.weak_bg_fill = ACTIVE;
    style.visuals.widgets.active.bg_fill = ACTIVE;
    ctx.set_style(style);
}

fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).strong().color(INK))
        .fill(MINT)
        .stroke(Stroke::NONE)
        .corner_radius(8)
}

fn panel() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(14)
        .inner_margin(16)
}

fn muted(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    ui.label(RichText::new(text.into()).color(MUTED))
}

fn cover(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    texture: Option<&egui::TextureHandle>,
    name: &str,
    radius: u8,
) {
    if let Some(texture) = texture {
        let image_size = texture.size_vec2();
        let factor = (rect.width() / image_size.x).max(rect.height() / image_size.y);
        let uv_size = rect.size() / (image_size * factor);
        let origin = pos2((1.0 - uv_size.x) / 2.0, (1.0 - uv_size.y) / 2.0);
        egui::Image::new(texture)
            .corner_radius(radius)
            .uv(egui::Rect::from_min_size(origin, uv_size))
            .paint_at(ui, rect);
    } else {
        ui.painter().rect_filled(rect, radius, ACTIVE);
        let initial: String = name
            .split_whitespace()
            .take(2)
            .filter_map(|word| word.chars().next())
            .flat_map(char::to_uppercase)
            .collect();
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            initial,
            FontId::proportional((rect.height() * 0.42).min(46.0)),
            MINT,
        );
    }
}

fn thumbnail_row(
    ui: &mut egui::Ui,
    game: &Game,
    art: Option<&egui::TextureHandle>,
    selected: bool,
) -> egui::Response {
    let available = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(available, 58.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            &game.name,
        )
    });
    if selected || response.hovered() {
        ui.painter()
            .rect_filled(rect, 9, if selected { ACTIVE } else { PANEL });
    }
    if selected {
        let rail = egui::Rect::from_min_size(
            rect.left_top() + vec2(0.0, 10.0),
            vec2(3.0, rect.height() - 20.0),
        );
        ui.painter().rect_filled(rail, 3, MINT);
    }
    let thumb = egui::Rect::from_min_size(rect.left_top() + vec2(10.0, 10.0), vec2(55.0, 38.0));
    cover(ui, thumb, art, &game.name, 6);
    let left = thumb.right() + 10.0;
    let clip = egui::Rect::from_min_max(
        pos2(left, rect.top()),
        pos2(rect.right() - 6.0, rect.bottom()),
    );
    let mut name = egui::text::LayoutJob::simple(
        game.name.clone(),
        FontId::proportional(14.0),
        TEXT,
        clip.width(),
    );
    name.wrap.max_rows = 1;
    let painter = ui.painter().with_clip_rect(clip);
    let name = painter.layout_job(name);
    painter.galley(pos2(left, rect.top() + 11.0), name, TEXT);
    painter.text(
        pos2(left, rect.top() + 33.0),
        egui::Align2::LEFT_TOP,
        &game.store,
        FontId::proportional(12.0),
        MUTED,
    );
    response.on_hover_text(&game.name)
}

impl App {
    fn library(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (logo, _) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
            ui.painter().rect_filled(logo, 10, MINT);
            let front =
                egui::Rect::from_center_size(logo.center() + vec2(-3.0, 3.0), vec2(15.0, 12.0));
            let back = front.translate(vec2(6.0, -6.0));
            ui.painter()
                .rect_stroke(back, 3, Stroke::new(1.7_f32, INK), egui::StrokeKind::Inside);
            ui.painter().rect_filled(front, 3, MINT);
            ui.painter().rect_stroke(
                front,
                3,
                Stroke::new(1.7_f32, INK),
                egui::StrokeKind::Inside,
            );
            ui.label(RichText::new("GameSaves").size(23.0).strong());
        });
        ui.add_space(22.0);
        ui.label(RichText::new("Мои игры").size(16.0).strong());
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Найти игру…")
                .margin(egui::Margin::symmetric(10, 7))
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            if ui.button("Добавить игру").clicked() {
                self.show_add = true;
                self.status.clear();
            }
            if ui
                .button("Обновить")
                .on_hover_text("Найти установленные игры заново")
                .clicked()
            {
                self.games = catalog::discover(&self.settings.custom_games);
                self.artwork.refresh();
                if let Some(game) = self
                    .games
                    .iter()
                    .find(|game| Some(&game.id) == self.selected.as_ref())
                {
                    self.candidates = catalog::save_candidates(game);
                } else if let Some(game) = self.games.first() {
                    self.select(game.id.clone());
                }
            }
        });
        ui.add_space(9.0);
        let query = self.search.trim().to_lowercase();
        let matches: Vec<_> = self
            .games
            .iter()
            .filter(|game| game.name.to_lowercase().contains(&query))
            .collect();
        if matches.is_empty() {
            muted(
                ui,
                if query.is_empty() {
                    "Добавь игру или обнови список."
                } else {
                    "Ничего не найдено."
                },
            );
        }
        let mut selection = None;
        ui.spacing_mut().item_spacing.y = 4.0;
        egui::ScrollArea::vertical()
            .id_salt("library")
            .auto_shrink([false, false])
            .show_rows(ui, 58.0, matches.len(), |ui, range| {
                for index in range {
                    let game = matches[index];
                    let art = self.artwork.get(
                        game,
                        self.settings.covers.get(&game.id).map(PathBuf::as_path),
                    );
                    if thumbnail_row(
                        ui,
                        game,
                        art.as_ref(),
                        self.selected.as_ref() == Some(&game.id),
                    )
                    .clicked()
                    {
                        selection = Some(game.id.clone());
                    }
                }
            });
        if let Some(game) = selection {
            self.select(game);
        }
    }

    fn game_header(&mut self, ui: &mut egui::Ui, game: &Game) {
        let art = self.artwork.get(
            game,
            self.settings.covers.get(&game.id).map(PathBuf::as_path),
        );
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, 204.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 16, PANEL);
        let art_width = (width * 0.50).clamp(215.0, 418.0);
        let art_rect = egui::Rect::from_min_max(
            pos2(rect.right() - art_width, rect.top()),
            rect.right_bottom(),
        );
        cover(ui, art_rect, art.as_ref(), &game.name, 15);
        if art.is_some() {
            ui.painter()
                .rect_filled(art_rect, 15, Color32::from_black_alpha(45));
        }
        let title_area = egui::Rect::from_min_max(
            rect.left_top() + vec2(21.0, 20.0),
            pos2(art_rect.left() - 12.0, rect.bottom() - 16.0),
        );
        let mut title = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(title_area)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        title.set_clip_rect(title_area);
        title.label(RichText::new(&game.store).size(13.0).color(MINT));
        title.add_space(5.0);
        let mut job = egui::text::LayoutJob::simple(
            game.name.clone(),
            FontId::proportional(31.0),
            TEXT,
            title_area.width(),
        );
        job.wrap.max_rows = 2;
        title
            .label(job)
            .on_hover_text(game.install_dir.display().to_string());
        title.add_space(7.0);
        if title
            .button(if self.show_setup {
                "Свернуть настройки"
            } else {
                "Настройки игры"
            })
            .clicked()
        {
            self.show_setup = !self.show_setup;
        }
    }

    fn send_and_restore(&mut self, ui: &mut egui::Ui, game: &Game) {
        let ready_save = self
            .settings
            .save_paths
            .get(&game.id)
            .filter(|path| path.is_file())
            .cloned();
        let world = self.world();
        let ready = world.is_some() && ready_save.is_some();
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new("Обмен сохранениями").size(20.0).strong());
            if ready {
                muted(ui, "Закрой игру перед отправкой или восстановлением.");
            } else {
                muted(
                    ui,
                    "Выбери файл сохранения и подключи общую папку в настройках игры.",
                );
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(ready, primary("Отправить сохранение"))
                    .clicked()
                {
                    if let (Some(world), Some(source)) = (world, ready_save.clone()) {
                        let sender = self.network.as_ref().map(peers::Network::sender);
                        self.start_job(ui.ctx(), move || {
                            peers::deliver(sender.as_ref(), &world, &source)
                        });
                    }
                }
                let restore = ui.add_enabled(
                    ready_save.is_some() && self.version.is_some(),
                    egui::Button::new("Восстановить версию"),
                );
                if restore.clicked() {
                    if let (Some(version), Some(destination)) = (self.version.clone(), ready_save) {
                        self.restore_confirmation = Some((version, destination));
                    }
                }
            });
        });
    }

    fn history(&mut self, ui: &mut egui::Ui) {
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new("История версий").size(18.0).strong());
            if self.versions.is_empty() {
                muted(ui, "Полученные и отправленные версии появятся здесь.");
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("versions")
                .max_height(225.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (index, (label, path)) in self.versions.iter().enumerate() {
                        let selected = Some(path) == self.version.as_ref();
                        let display = if index == 0 {
                            format!("{label}  ·  Последняя")
                        } else {
                            label.clone()
                        };
                        let button = egui::Button::new(RichText::new(display).color(if selected {
                            MINT
                        } else {
                            TEXT
                        }))
                        .min_size(vec2(ui.available_width(), 36.0))
                        .selected(selected);
                        if ui
                            .add(button)
                            .on_hover_text(
                                path.file_name()
                                    .unwrap_or(path.as_os_str())
                                    .to_string_lossy(),
                            )
                            .clicked()
                        {
                            self.version = Some(path.clone());
                        }
                    }
                });
        });
    }

    fn setup(&mut self, ui: &mut egui::Ui, game: &Game) {
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new("Настройки игры").size(18.0).strong());
            ui.label("Файл сохранения");
            if !self.candidates.is_empty() {
                egui::ComboBox::from_id_salt("candidates")
                    .selected_text("Найденные сохранения")
                    .show_ui(ui, |ui| {
                        for candidate in &self.candidates {
                            let path = candidate.to_string_lossy();
                            let name = candidate
                                .file_name()
                                .unwrap_or(candidate.as_os_str())
                                .to_string_lossy();
                            if ui
                                .selectable_label(self.save_path == path, name)
                                .on_hover_text(path.as_ref())
                                .clicked()
                            {
                                self.save_path = path.into_owned();
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
                if !path.is_file() {
                    muted(ui, "Для обмена нужен отдельный файл сохранения.");
                }
            }
            ui.separator();
            ui.label("Общая папка");
            path_field(ui, &mut self.shared_path, false);
            ui.horizontal_wrapped(|ui| {
                ui.label("Код мира");
                ui.add(egui::TextEdit::singleline(&mut self.world_code).desired_width(135.0));
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
            muted(
                ui,
                "На обоих ПК выбери одну общую папку и одинаковый код мира.",
            );
            #[cfg(windows)]
            self.cover_settings(ui, game);
        });
    }

    #[cfg(windows)]
    fn cover_settings(&mut self, ui: &mut egui::Ui, game: &Game) {
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label("Обложка");
            if ui
                .add_enabled(
                    self.config_error.is_none(),
                    egui::Button::new("Выбрать картинку"),
                )
                .clicked()
            {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Изображения", &["jpg", "jpeg", "png", "webp"])
                    .pick_file()
                {
                    let directory = self.settings_path.with_file_name("custom-covers");
                    match crate::artwork::set_custom_cover(&path, &directory) {
                        Ok(saved) => self.update_cover(&game.id, Some(saved)),
                        Err(error) => self.status = error.to_string(),
                    }
                }
            }
            if self.settings.covers.contains_key(&game.id)
                && ui.button("Вернуть автообложку").clicked()
            {
                self.update_cover(&game.id, None);
            }
        });
    }

    #[cfg(windows)]
    fn update_cover(&mut self, game_id: &str, replacement: Option<PathBuf>) {
        let mut updated = self.settings.clone();
        let previous = match &replacement {
            Some(path) => updated.covers.insert(game_id.to_owned(), path.clone()),
            None => updated.covers.remove(game_id),
        };
        match updated.write(&self.settings_path) {
            Ok(()) => {
                self.settings = updated;
                self.artwork.refresh();
                if let Some(previous) = previous {
                    if previous.parent()
                        == Some(self.settings_path.with_file_name("custom-covers").as_path())
                    {
                        let _ = std::fs::remove_file(previous);
                    }
                }
                self.status = "Обложка обновлена.".into();
            }
            Err(error) => {
                if let Some(path) = replacement {
                    let _ = std::fs::remove_file(path);
                }
                self.status = format!("Не удалось сохранить обложку. {error}");
            }
        }
    }

    fn add_modal(&mut self, ctx: &egui::Context) {
        if !self.show_add {
            return;
        }
        let modal = egui::Modal::new(egui::Id::new("add-game")).show(ctx, |ui| {
            ui.set_width(390.0);
            ui.heading("Добавить игру вручную");
            ui.label("Название");
            ui.add(
                egui::TextEdit::singleline(&mut self.new_game_name).desired_width(f32::INFINITY),
            );
            ui.label("Папка установки");
            path_field(ui, &mut self.new_game_dir, false);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.config_error.is_none(), primary("Добавить"))
                    .clicked()
                {
                    self.add_game();
                }
                if ui.button("Отмена").clicked() {
                    self.show_add = false;
                }
            });
            if self.show_add && !self.status.is_empty() {
                ui.label(RichText::new(&self.status).color(ALERT));
            }
        });
        if modal.should_close() {
            self.show_add = false;
        }
    }
}

fn path_field(ui: &mut egui::Ui, path: &mut String, file: bool) {
    ui.horizontal_wrapped(|ui| {
        let response = ui.add(
            egui::TextEdit::singleline(path)
                .desired_width((ui.available_width() - 100.0).clamp(195.0, 440.0))
                .hint_text("Полный путь"),
        );
        if !path.is_empty() {
            response.on_hover_text(path.as_str());
        }
        #[cfg(windows)]
        if ui
            .button(if file {
                "Выбрать файл"
            } else {
                "Выбрать папку"
            })
            .clicked()
        {
            let dialog = rfd::FileDialog::new();
            if let Some(selected) = if file {
                dialog.pick_file()
            } else {
                dialog.pick_folder()
            } {
                *path = selected.to_string_lossy().into_owned();
            }
        }
        #[cfg(not(windows))]
        let _ = file;
    });
}

impl App {
    pub(super) fn draw(&mut self, ctx: &egui::Context) {
        self.artwork.poll(ctx);
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
            .exact_width(269.0)
            .frame(egui::Frame::new().fill(SIDEBAR).inner_margin(18))
            .show(ctx, |ui| {
                ui.add_enabled_ui(idle, |ui| self.library(ui));
            });
        if !self.status.is_empty() && !self.show_add {
            egui::TopBottomPanel::bottom("transfer-status")
                .frame(egui::Frame::new().fill(SIDEBAR).inner_margin(12))
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if !idle {
                            ui.spinner();
                        }
                        let color = if self.status.starts_with("Операция прервана")
                            || self.status.starts_with("Не удалось")
                        {
                            ALERT
                        } else {
                            MINT
                        };
                        ui.label(RichText::new(&self.status).color(color));
                    });
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BACKGROUND).inner_margin(22))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let game = self
                            .selected
                            .as_ref()
                            .and_then(|id| self.games.iter().find(|game| game.id == *id))
                            .cloned();
                        if let Some(game) = game {
                            ui.add_enabled_ui(idle, |ui| {
                                self.game_header(ui, &game);
                                ui.add_space(8.0);
                                self.send_and_restore(ui, &game);
                                ui.add_space(7.0);
                                if self.show_setup {
                                    self.setup(ui, &game);
                                    ui.add_space(7.0);
                                }
                                self.history(ui);
                            });
                        } else {
                            ui.add_space(18.0);
                            ui.heading("Подключи первую игру");
                            muted(ui, "Выбери установленную игру слева или добавь её вручную.");
                            if ui.add(primary("Добавить игру")).clicked() {
                                self.show_add = true;
                                self.status.clear();
                            }
                        }
                        if let Some(error) = &self.config_error {
                            ui.label(
                                RichText::new(format!("Не удалось прочитать настройки. {error}"))
                                    .color(ALERT),
                            );
                        }
                    });
            });
        self.add_modal(ctx);
        self.confirm_restore(ctx);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.draw(ctx);
    }
}
