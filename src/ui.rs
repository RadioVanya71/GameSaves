use super::{App, Game, catalog, peers};
use eframe::egui::{self, Color32, FontId, RichText, Stroke, pos2, vec2};
use std::path::PathBuf;
use std::time::Duration;

const BACKGROUND: Color32 = Color32::from_rgb(248, 250, 252);
const SIDEBAR: Color32 = Color32::WHITE;
const PANEL: Color32 = Color32::WHITE;
const BORDER: Color32 = Color32::from_rgb(190, 201, 216);
const TEXT: Color32 = Color32::from_rgb(30, 41, 59);
const MUTED: Color32 = Color32::from_rgb(71, 85, 105);
const ACCENT: Color32 = Color32::from_rgb(37, 99, 235);
const ACTIVE: Color32 = Color32::from_rgb(233, 239, 252);
const ALERT: Color32 = Color32::from_rgb(165, 37, 28);

const HERO: Color32 = Color32::from_rgb(22, 36, 66);
const ON_HERO: Color32 = Color32::from_rgb(222, 233, 255);
const ORANGE: Color32 = Color32::from_rgb(255, 168, 91);

fn semibold(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("Semibold".into()))
}

fn heading(text: &str, size: f32) -> RichText {
    RichText::new(text).font(semibold(size)).color(TEXT)
}

pub fn configure(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        (
            "Open Sans",
            &include_bytes!("fonts/OpenSans-Regular.ttf")[..],
        ),
        (
            "Semibold",
            &include_bytes!("fonts/OpenSans-Semibold.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
    }
    let regular = fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap();
    regular.insert(0, "Open Sans".into());
    let mut bold = regular.clone();
    bold.insert(0, "Semibold".into());
    fonts.families.insert(semibold(18.0).family, bold);
    ctx.set_fonts(fonts);

    // Native system-theme events arrive after the app creator has returned.
    ctx.set_theme(egui::Theme::Light);
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = BACKGROUND;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = PANEL;
    style.visuals.faint_bg_color = ACTIVE;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.weak_text_color = Some(MUTED);
    style.visuals.selection.bg_fill = ACTIVE;
    style.visuals.selection.stroke = Stroke::new(1.5_f32, ACCENT);
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.disabled_alpha = 0.68;
    style.visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.window_corner_radius = 14.into();
    style.spacing.window_margin = egui::Margin::same(22);
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(15.0, 8.0);
    style.spacing.interact_size.y = 44.0;
    for (role, font) in [
        (egui::TextStyle::Small, FontId::proportional(15.0)),
        (egui::TextStyle::Body, FontId::proportional(17.0)),
        (egui::TextStyle::Button, semibold(16.0)),
        (egui::TextStyle::Heading, semibold(28.0)),
    ] {
        style.text_styles.insert(role, font);
    }
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    for (widget, color, border) in [
        (&mut style.visuals.widgets.inactive, PANEL, BORDER),
        (&mut style.visuals.widgets.hovered, ACTIVE, ACCENT),
        (&mut style.visuals.widgets.active, ACTIVE, ACCENT),
    ] {
        widget.corner_radius = 8.into();
        widget.bg_stroke = Stroke::new(1.0_f32, border);
        widget.fg_stroke = Stroke::new(1.5_f32, TEXT);
        widget.bg_fill = color;
        widget.weak_bg_fill = color;
    }
    ctx.set_style_of(egui::Theme::Light, style.clone());
    ctx.set_style_of(egui::Theme::Dark, style);
}

fn primary(label: &str) -> impl egui::Widget + '_ {
    move |ui: &mut egui::Ui| {
        let button = egui::Button::new(
            RichText::new(label)
                .font(semibold(16.0))
                .color(Color32::WHITE),
        )
        .fill(ACCENT)
        .min_size(vec2(0.0, 44.0))
        .corner_radius(8);
        let response = ui.add(button);
        if response.has_focus() || response.hovered() {
            ui.painter().rect_stroke(
                response.rect.expand(2.0),
                10,
                Stroke::new(2.0_f32, HERO),
                egui::StrokeKind::Inside,
            );
        }
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    }
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
            ACCENT,
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
    if selected || response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, 9, if selected { ACTIVE } else { BACKGROUND });
    }
    if selected {
        let rail = egui::Rect::from_min_size(
            rect.left_top() + vec2(0.0, 10.0),
            vec2(3.0, rect.height() - 20.0),
        );
        ui.painter().rect_filled(rail, 3, ACCENT);
    }
    let thumb = egui::Rect::from_min_size(rect.left_top() + vec2(10.0, 10.0), vec2(55.0, 38.0));
    cover(ui, thumb, art, &game.name, 6);
    let left = thumb.right() + 10.0;
    let clip = egui::Rect::from_min_max(
        pos2(left, rect.top()),
        pos2(rect.right() - 6.0, rect.bottom()),
    );
    let mut name =
        egui::text::LayoutJob::simple(game.name.clone(), semibold(16.0), TEXT, clip.width());
    name.wrap.max_rows = 1;
    let painter = ui.painter().with_clip_rect(clip);
    let name = painter.layout_job(name);
    painter.galley(pos2(left, rect.top() + 11.0), name, TEXT);
    painter.text(
        pos2(left, rect.top() + 33.0),
        egui::Align2::LEFT_TOP,
        &game.store,
        FontId::proportional(14.0),
        MUTED,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            9,
            Stroke::new(2.0_f32, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(&game.name)
}

impl App {
    fn library(&mut self, ui: &mut egui::Ui) {
        ui.label(heading("Библиотека", 23.0));
        ui.add_space(2.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Найти игру…")
                .margin(egui::Margin::symmetric(10, 9))
                .desired_width(f32::INFINITY),
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("Добавить").clicked() {
                self.open_add(ui.ctx());
            }
            if ui
                .button("Обновить")
                .on_hover_text("Повторить поиск игр на ПК")
                .clicked()
            {
                self.rescan();
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

    fn demo_controls(&mut self, ui: &mut egui::Ui) {
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let friend = self.demo.profile == "friend";
            ui.label(heading(
                if friend {
                    "Тестовый профиль друга"
                } else {
                    "Тестовый профиль игрока"
                },
                22.0,
            ));
            ui.label(format!("Уровень в демоигре — {}", self.demo_level));
            muted(
                ui,
                "Повысь уровень и отправь сохранение. Затем восстанови версию и проверь уровень.",
            );
            ui.horizontal_wrapped(|ui| {
                if ui.button("Пройти уровень").clicked() {
                    match self.demo.advance() {
                        Ok(()) => self.refresh_versions(),
                        Err(e) => self.status = format!("Не удалось изменить демосохранение. {e}"),
                    }
                }
                if !friend {
                    let opened = self
                        .friend_window
                        .as_mut()
                        .is_some_and(|child| matches!(child.try_wait(), Ok(None)));
                    if ui
                        .add_enabled(
                            !opened,
                            egui::Button::new(if opened {
                                "Окно друга открыто"
                            } else {
                                "Открыть окно друга"
                            }),
                        )
                        .clicked()
                    {
                        match std::env::current_exe()
                            .and_then(|exe| std::process::Command::new(exe).arg("--friend").spawn())
                        {
                            Ok(child) => self.friend_window = Some(child),
                            Err(e) => self.status = format!("Не удалось открыть окно друга. {e}"),
                        }
                    }
                }
            });
        });
    }

    fn game_header(&mut self, ui: &mut egui::Ui, game: &Game) {
        let art = self.artwork.get(
            game,
            self.settings.covers.get(&game.id).map(PathBuf::as_path),
        );
        let width = ui.available_width();
        let art_width = (width * 0.50).min(510.0);
        let frame = egui::Frame::new()
            .fill(HERO)
            .corner_radius(14)
            .inner_margin(20);
        let response = frame.show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let left = ui.available_width() - art_width;
            ui.allocate_ui_with_layout(
                vec2(left, 136.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_max_width(left);
                    ui.label(
                        RichText::new(&game.store)
                            .font(semibold(15.0))
                            .color(ON_HERO),
                    );
                    ui.label(
                        RichText::new(&game.name)
                            .font(semibold(30.0))
                            .color(Color32::WHITE),
                    );
                    ui.add_space(4.0);
                    if ui
                        .button(if self.show_setup {
                            "Скрыть настройки"
                        } else {
                            "Настройки игры"
                        })
                        .clicked()
                    {
                        self.show_setup = !self.show_setup;
                    }
                    if ui.button("История версий").clicked() {
                        self.jump_to_history = true;
                    }
                },
            );
        });
        let rect = response.response.rect;
        let artwork = egui::Rect::from_min_max(
            pos2(rect.right() - art_width, rect.top()),
            rect.right_bottom(),
        );
        cover(ui, artwork, art.as_ref(), &game.name, 14);
    }

    fn group_controls(&mut self, ui: &mut egui::Ui, game: &Game) {
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(heading("Группа Google Диска", 22.0));
            if let Some(link) = self.settings.groups.get(&game.id).cloned() {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Скопировать приглашение").clicked() {
                        ui.ctx().copy_text(link);
                    }
                    if ui.button("Выйти из группы").clicked() {
                        self.change_group(game, None);
                    }
                });
            } else {
                let service = option_env!("GAMESAVES_GROUP_ORIGIN").filter(|s| !s.is_empty());
                if let Some(url) = service {
                    ui.hyperlink_to(
                        "Создать группу",
                        format!("{}/create", url.trim_end_matches('/')),
                    );
                    self.invite_field(ui, game);
                } else {
                    muted(ui, "В этой сборке сервис групп не подключён. Создать группу из папки Google Диска пока нельзя.");
                    ui.collapsing("У меня есть приглашение GameSaves", |ui| {
                        self.invite_field(ui, game);
                    });
                }
            }
            if !self.cloud_error.is_empty() {
                ui.label(&self.cloud_error);
            }
        });
    }
    fn invite_field(&mut self, ui: &mut egui::Ui, game: &Game) {
        ui.label("Приглашение GameSaves от владельца группы");
        ui.add(
            egui::TextEdit::singleline(&mut self.invitation)
                .hint_text("Вставь приглашение")
                .password(true),
        );
        let input = self.invitation.trim();
        let parsed = crate::group::Group::parse(input);
        if !input.is_empty() && parsed.is_err() {
            let drive = url::Url::parse(input)
                .ok()
                .is_some_and(|u| u.host_str() == Some("drive.google.com"));
            ui.label(egui::RichText::new(if drive {
                "Это ссылка на папку Google Диска. Для входа друг должен прислать приглашение из GameSaves."
            } else {
                "Скопируй приглашение целиком, включая часть после #."
            }).color(ALERT));
        }
        if ui
            .add_enabled(parsed.is_ok(), egui::Button::new("Войти в группу"))
            .clicked()
        {
            self.change_group(game, Some(input.to_string()));
        }
    }
    fn change_group(&mut self, game: &Game, link: Option<String>) {
        if self.config_error.is_some() {
            return;
        }
        let mut settings = self.settings.clone();
        if let Some(link) = link {
            settings.groups.insert(game.id.clone(), link);
        } else {
            settings.groups.remove(&game.id);
        }
        match settings.write(&self.settings_path) {
            Ok(()) => {
                self.settings = settings;
                self.select(game.id.clone());
            }
            Err(e) => self.status = format!("Не удалось записать настройки. {e}"),
        }
    }
    fn send_save(&mut self, ui: &mut egui::Ui, game: &Game) {
        let ready_save = self
            .settings
            .save_paths
            .get(&game.id)
            .filter(|path| path.is_file())
            .cloned();
        let world = self.world();
        let cloud = self.group();
        let ready = (world.is_some() || cloud.is_some()) && ready_save.is_some();
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(heading("Отправка сохранения", 22.0));
            if ready {
                muted(ui, "Сохрани прогресс и закрой игру перед отправкой.");
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
                    let author = self.settings.nickname.clone();
                    if let (Some(group), Some(source)) = (cloud, ready_save.as_ref()) {
                        let source = source.clone();
                        self.start_job(ui.ctx(), move || group.send(&source, &author));
                    } else if let (Some(world), Some(source)) = (world, ready_save) {
                        let sender = self.network.as_ref().map(peers::Network::sender);
                        self.start_job(ui.ctx(), move || {
                            peers::deliver(sender.as_ref(), &world, &source, &author)
                        });
                    }
                }
            });
        });
    }

    fn history(&mut self, ui: &mut egui::Ui, game: &Game) {
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let title = ui.label(heading("История версий", 22.0));
            if self.jump_to_history {
                self.jump_to_history = false;
                title.scroll_to_me(Some(egui::Align::TOP));
            }
            if self.versions.is_empty() {
                muted(ui, "Полученные и отправленные версии появятся здесь.");
                return;
            }
            for (index, (label, path)) in self.versions.iter().enumerate() {
                let selected = Some(path) == self.version.as_ref();
                let display = if index == 0 {
                    format!("{label}  ·  Последняя")
                } else {
                    label.clone()
                };
                let radio = egui::RadioButton::new(selected, RichText::new(display).color(TEXT));
                if ui
                    .add(radio)
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
            let destination = self
                .settings
                .save_paths
                .get(&game.id)
                .filter(|path| path.is_file());
            if ui
                .add_enabled(
                    destination.is_some() && self.version.is_some(),
                    egui::Button::new("Восстановить версию"),
                )
                .clicked()
            {
                self.restore_confirmation = self.version.clone().zip(destination.cloned());
            }
        });
    }

    fn setup(&mut self, ui: &mut egui::Ui, game: &Game) {
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(heading("Настройки игры", 22.0));
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
        let modal = egui::Modal::new(egui::Id::new("add-game"))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .inner_margin(24)
                    .corner_radius(14),
            )
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.heading("Добавить игру");
                muted(ui, "Укажи название и папку, куда установлена игра.");
                ui.label("Название");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_game_name)
                        .id(egui::Id::new("new-game-title"))
                        .margin(egui::Margin::symmetric(10, 8))
                        .desired_width(f32::INFINITY),
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
                .margin(egui::Margin::symmetric(10, 8))
                .desired_width(
                    (ui.available_width() - if cfg!(windows) { 195.0 } else { 20.0 })
                        .clamp(140.0, 440.0),
                )
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
        self.poll_group(ctx);
        let idle = self.job.is_none();
        if self.selected.is_some() && idle {
            let interval = Duration::from_secs(2);
            if self.last_refresh.elapsed() >= interval {
                self.refresh_versions();
            }
            ctx.request_repaint_after(interval.saturating_sub(self.last_refresh.elapsed()));
        }
        egui::TopBottomPanel::top("brand")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(24, 12)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(37.0, 37.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 10, ACCENT);
                    for shift in [3.0, -3.0] {
                        let tile = egui::Rect::from_center_size(
                            rect.center() + vec2(shift, -shift),
                            vec2(16.0, 12.0),
                        );
                        ui.painter().rect_filled(tile, 3, ACCENT);
                        ui.painter().rect_stroke(
                            tile,
                            3,
                            Stroke::new(1.8_f32, Color32::WHITE),
                            egui::StrokeKind::Inside,
                        );
                    }
                    ui.label(heading("GameSaves", 24.0));
                    ui.separator();
                    ui.label("Мой ник");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.nickname_input)
                            .char_limit(24)
                            .desired_width(155.0),
                    );
                    let name = self.nickname_input.trim();
                    if self.settings.nickname != name {
                        let valid = crate::snapshots::valid_nickname(name);
                        if ui
                            .add_enabled(
                                valid && self.config_error.is_none(),
                                egui::Button::new("Сохранить"),
                            )
                            .clicked()
                        {
                            let mut updated = self.settings.clone();
                            updated.nickname = name.to_owned();
                            match updated.write(&self.settings_path) {
                                Ok(()) => {
                                    self.settings = updated;
                                    self.status = "Ник сохранён.".into();
                                }
                                Err(e) => self.status = format!("Не удалось сохранить ник. {e}"),
                            }
                        }
                        if !valid {
                            ui.label("Буквы, цифры, пробел, точка, дефис или _");
                        }
                    }
                });
            });
        if !self.games.is_empty() {
            egui::SidePanel::left("game-list")
                .exact_width(269.0)
                .frame(egui::Frame::new().fill(SIDEBAR).inner_margin(18))
                .show(ctx, |ui| {
                    ui.add_enabled_ui(idle, |ui| self.library(ui));
                });
        }
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
                            ACCENT
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
                                self.group_controls(ui, &game);
                                if game.id == crate::demo::GAME_ID {
                                    self.demo_controls(ui);
                                }
                                ui.add_space(8.0);
                                if self.show_setup {
                                    self.setup(ui, &game);
                                    ui.add_space(7.0);
                                }
                                self.send_save(ui, &game);
                                ui.add_space(7.0);
                                self.history(ui, &game);
                            });
                        } else {
                            self.welcome(ui);
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
// A save disk drawn in logical points stays sharp at native Windows DPI.
fn save_illustration(ui: &egui::Ui, rect: egui::Rect) {
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 14, ACCENT);
    let center = rect.center();
    let scale = (rect.width() / 258.0).min(rect.height() / 212.0).min(1.2);
    let point = |x, y| center + vec2(x, y) * scale;
    let area = |x, y, w, h| egui::Rect::from_min_size(point(x, y), vec2(w, h) * scale);
    for col in 0..(rect.width() / 15.0) as usize {
        for row in 0..(rect.height() / 15.0) as usize {
            painter.circle_filled(
                rect.min + vec2(9.0 + 15.0 * col as f32, 9.0 + 15.0 * row as f32),
                1.1,
                Color32::from_rgb(98, 144, 247),
            );
        }
    }
    painter.circle_filled(center, 86.0 * scale, Color32::from_rgb(172, 202, 255));
    let edge = HERO;
    for (dx, dy, fill) in [(11.0, 10.0, ORANGE), (0.0, 0.0, ACCENT)] {
        let outline = [
            (-52.0, -59.0),
            (31.0, -59.0),
            (52.0, -37.0),
            (52.0, 59.0),
            (-52.0, 59.0),
        ]
        .map(|(x, y)| point(x + dx, y + dy))
        .to_vec();
        painter.add(egui::Shape::convex_polygon(
            outline,
            fill,
            Stroke::new(2.0 * scale, edge),
        ));
    }
    painter.rect_filled(area(-32.0, -58.0, 54.0, 38.0), 2, ON_HERO);
    painter.rect_filled(area(4.0, -51.0, 10.0, 23.0), 1, edge);
    painter.rect_filled(
        area(-38.0, 1.0, 76.0, 56.0),
        3,
        Color32::from_rgb(255, 246, 219),
    );
    painter.rect_filled(area(-38.0, 1.0, 76.0, 12.0), 2, ORANGE);
    for (y, end) in [(26.0, 26.0), (37.0, 13.0)] {
        painter.line_segment(
            [point(-25.0, y), point(end, y)],
            Stroke::new(2.0 * scale, edge),
        );
    }
}

impl App {
    fn open_add(&mut self, ctx: &egui::Context) {
        ctx.memory_mut(|mem| mem.request_focus(egui::Id::new("new-game-title")));
        self.show_add = true;
        self.status.clear();
    }

    fn rescan(&mut self) {
        self.games = catalog::discover(&self.settings.custom_games);
        self.artwork.rescan(&self.games);
        if let Some(game) = self
            .games
            .iter()
            .find(|game| Some(&game.id) == self.selected.as_ref())
        {
            self.candidates = catalog::save_candidates(game);
        } else if let Some(game) = self.games.first() {
            self.select(game.id.clone());
        } else {
            self.selected = None;
            self.candidates.clear();
        }
        self.status = if self.games.is_empty() {
            "Поиск завершён. Игры не найдены. Можно добавить игру вручную.".into()
        } else {
            format!("Поиск завершён. Игр в библиотеке — {}.", self.games.len())
        };
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width().min(900.0);
        ui.vertical_centered(|ui| {
            ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.add_space(4.0);
                ui.label(heading("Библиотека игр", 31.0));
                ui.add_space(8.0);
                egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(18).inner_margin(18).show(ui, |ui| {
                        let available = ui.available_width();
                        let horizontal = available >= 620.0;
                        let art_width = if horizontal { (available * 0.35).min(296.0) } else { available };
                        let art_height = if horizontal { 212.0 } else { 126.0 };
                        let layout = if horizontal { egui::Layout::left_to_right(egui::Align::Center) }
                                     else { egui::Layout::top_down(egui::Align::Min) };
                        ui.with_layout(layout, |ui| {
                            let (rect, _) = ui.allocate_exact_size(vec2(art_width, art_height), egui::Sense::hover());
                            save_illustration(ui, rect);
                            ui.add_space(if horizontal { 14.0 } else { 4.0 });
                            ui.vertical(|ui| {
                                ui.label(heading("Добавь первую игру", 29.0));
                                muted(ui, "Укажи папку установленной игры. Здесь появятся её обложка и история сохранений.");
                                ui.add_space(6.0);
                                ui.horizontal_wrapped(|ui| {
                                if ui.add(primary("Добавить игру")).clicked() {
                                    self.open_add(ui.ctx());
                                }
                                if ui.button("Повторить поиск на ПК").clicked() {
                                    self.rescan();
                                }
                                });
                            });
                        });
                    });
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_theme_changes_preserve_library_contrast_and_type() {
        let ctx = egui::Context::default();
        configure(&ctx);
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let input = egui::RawInput {
                system_theme: Some(theme),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                let style = ctx.style();
                assert_eq!(style.visuals.panel_fill, BACKGROUND);
                assert_eq!(style.visuals.override_text_color, Some(TEXT));
                assert_eq!(style.visuals.weak_text_color(), MUTED);
                assert!(style.text_styles[&egui::TextStyle::Body].size >= 17.0);
                assert!(style.spacing.interact_size.y >= 42.0);
            });
        }
    }
}
