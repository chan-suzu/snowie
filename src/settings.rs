//! Settings window (`snowie.exe --settings`). Runs as its own process so the overlay stays
//! small and never has to host a GUI toolkit; every change is saved and the overlay is told
//! to reload.

use crate::config::{self, parse_timer, Config};
use crate::tray;
use eframe::egui::{self, Align, Color32, CornerRadius, FontFamily, Layout, RichText, Stroke, Ui, Vec2};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};

const TITLE: &str = "Snowie — настройки";

// frosty night palette
const BG: Color32 = Color32::from_rgb(11, 17, 29);
const SIDEBAR: Color32 = Color32::from_rgb(15, 23, 38);
const CARD: Color32 = Color32::from_rgb(19, 29, 47);
const CARD_STROKE: Color32 = Color32::from_rgb(33, 48, 75);
const CONTROL: Color32 = Color32::from_rgb(31, 45, 70);
const CONTROL_HOVER: Color32 = Color32::from_rgb(40, 60, 94);
const ACCENT: Color32 = Color32::from_rgb(92, 178, 255);
const ACCENT_DIM: Color32 = Color32::from_rgb(52, 105, 170);
const TEXT: Color32 = Color32::from_rgb(222, 232, 250);
const WEAK: Color32 = Color32::from_rgb(132, 150, 182);

pub fn run() {
    unsafe {
        if let Ok(h) = FindWindowW(None, w!("Snowie — настройки")) {
            if !h.is_invalid() {
                let _ = ShowWindow(h, SW_RESTORE);
                let _ = SetForegroundWindow(h);
                return;
            }
        }
    }
    let icon = egui::IconData { rgba: tray::snowflake_rgba(32), width: 32, height: 32 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([680.0, 480.0])
            .with_icon(icon),
        ..Default::default()
    };
    let _ = eframe::run_native(TITLE, options, Box::new(|cc| Ok(Box::new(SettingsApp::new(cc)))));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Snow,
    Drifts,
    Sound,
    Plow,
    Fireworks,
    System,
}

const PAGES: [(Page, &str, &str); 6] = [
    (Page::Snow, "❄", "Снегопад"),
    (Page::Drifts, "▲", "Сугробы"),
    (Page::Sound, "♫", "Звук и музыка"),
    (Page::Plow, "⛟", "Снегоуборщик"),
    (Page::Fireworks, "✦", "Салют"),
    (Page::System, "⚙", "Система"),
];

struct SettingsApp {
    cfg: Config,
    saved: Config,
    last_save: Instant,
    last_sync: Instant,
    autostart: bool,
    new_timer: String,
    status: String,
    page: Page,
    logo: egui::TextureHandle,
}

/// Segoe UI from Windows for text, Segoe UI Symbol for the icons; egui's own fonts stay as fallback.
fn install_fonts(ctx: &egui::Context) {
    let dir = std::path::Path::new(&std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts");
    let mut fonts = egui::FontDefinitions::default();
    let mut add = |name: &str, file: &str| -> bool {
        match std::fs::read(dir.join(file)) {
            Ok(bytes) => {
                fonts.font_data.insert(name.to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
                true
            }
            Err(_) => false,
        }
    };
    let regular = add("segoe", "segoeui.ttf");
    let semibold = add("segoe-semibold", "seguisb.ttf");
    let symbols = add("segoe-symbol", "seguisym.ttf");
    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    let mut front = Vec::new();
    if regular {
        front.push("segoe".to_owned());
    }
    if symbols {
        front.push("segoe-symbol".to_owned());
    }
    prop.splice(0..0, front.clone());
    let mut heading = Vec::new();
    if semibold {
        heading.push("segoe-semibold".to_owned());
    }
    heading.extend(front);
    heading.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts.families.insert(FontFamily::Name("heading".into()), heading);
    ctx.set_fonts(fonts);
}

fn install_style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = Color32::from_rgb(9, 14, 24);
    v.faint_bg_color = CARD;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.slider_trailing_fill = true;
    v.hyperlink_color = ACCENT;
    let r = CornerRadius::same(8);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = r;
        w.bg_stroke = Stroke::NONE;
    }
    v.widgets.inactive.bg_fill = CONTROL;
    v.widgets.inactive.weak_bg_fill = CONTROL;
    v.widgets.hovered.bg_fill = CONTROL_HOVER;
    v.widgets.hovered.weak_bg_fill = CONTROL_HOVER;
    v.widgets.active.bg_fill = ACCENT_DIM;
    v.widgets.active.weak_bg_fill = ACCENT_DIM;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, CARD_STROKE);
    v.widgets.noninteractive.corner_radius = r;
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(10.0, 10.0);
        s.spacing.button_padding = Vec2::new(14.0, 7.0);
        s.spacing.slider_width = 220.0;
        s.spacing.interact_size.y = 26.0;
        for (style, size) in [
            (egui::TextStyle::Body, 15.0),
            (egui::TextStyle::Button, 15.0),
            (egui::TextStyle::Small, 12.5),
            (egui::TextStyle::Heading, 26.0),
        ] {
            if let Some(f) = s.text_styles.get_mut(&style) {
                f.size = size;
            }
        }
    });
}

/// iOS-style switch.
fn toggle(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let size = Vec2::new(44.0, 24.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool(response.id, *on);
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
        let bg =
            Color32::from_rgb(mix(CONTROL.r(), ACCENT.r()), mix(CONTROL.g(), ACCENT.g()), mix(CONTROL.b(), ACCENT.b()));
        let radius = rect.height() / 2.0;
        ui.painter().rect_filled(rect, radius, bg);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        ui.painter().circle_filled(egui::pos2(x, rect.center().y), radius - 3.0, Color32::WHITE);
    }
    response
}

/// A rounded card holding a few setting rows.
fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, CARD_STROKE))
        .corner_radius(14)
        .inner_margin(egui::Margin::symmetric(18, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    ui.add_space(4.0);
}

/// One setting: title and hint on the left, the control on the right.
fn row(ui: &mut Ui, title: &str, hint: &str, control: impl FnOnce(&mut Ui)) {
    // fixed split: text on the left, controls right-aligned in the rest
    let w = ui.available_width();
    let text_w = (w * 0.5).max(180.0);
    ui.allocate_ui_with_layout(Vec2::new(w, 0.0), Layout::left_to_right(Align::Center), |ui| {
        ui.allocate_ui_with_layout(Vec2::new(text_w, 0.0), Layout::top_down(Align::LEFT), |ui| {
            ui.set_width(text_w);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(egui::Label::new(RichText::new(title).size(15.5).color(TEXT)).wrap());
            if !hint.is_empty() {
                ui.add(egui::Label::new(RichText::new(hint).size(12.5).color(WEAK)).wrap());
            }
        });
        let rest = ui.available_width();
        ui.allocate_ui_with_layout(Vec2::new(rest, 0.0), Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().slider_width = (rest - 110.0).clamp(80.0, 220.0);
            control(ui);
        });
    });
}

fn divider(ui: &mut Ui) {
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(r.x_range(), r.top() + 1.0, Stroke::new(1.0, CARD_STROKE));
    ui.add_space(6.0);
}

fn slider(ui: &mut Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, decimals: usize) {
    ui.add(egui::Slider::new(v, range).suffix(suffix).fixed_decimals(decimals));
}

fn percent(ui: &mut Ui, v: &mut f32) {
    let mut p = *v * 100.0;
    if ui.add(egui::Slider::new(&mut p, 0.0..=100.0).suffix(" %").fixed_decimals(0)).changed() {
        *v = p / 100.0;
    }
}

fn accent_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(Color32::from_rgb(8, 20, 38)).strong())
            .fill(ACCENT)
            .corner_radius(10),
    )
}

fn heading(ui: &mut Ui, title: &str, sub: &str) {
    ui.label(RichText::new(title).family(FontFamily::Name("heading".into())).size(28.0).color(Color32::WHITE));
    ui.label(RichText::new(sub).size(14.0).color(WEAK));
    ui.add_space(8.0);
}

impl SettingsApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_fonts(&cc.egui_ctx);
        install_style(&cc.egui_ctx);
        let logo = cc.egui_ctx.load_texture(
            "logo",
            egui::ColorImage::from_rgba_unmultiplied([64, 64], &tray::snowflake_rgba(64)),
            egui::TextureOptions::LINEAR,
        );
        let cfg = Config::load();
        SettingsApp {
            saved: cfg.clone(),
            cfg,
            last_save: Instant::now(),
            last_sync: Instant::now(),
            autostart: tray::autostart_enabled(),
            new_timer: String::new(),
            status: String::new(),
            page: Page::Snow,
            logo,
        }
    }

    fn command(&mut self, cmd: usize, done: &str) {
        self.status = if config::send(cmd) { done.to_string() } else { "Snowie не запущен".to_string() };
    }

    fn persist(&mut self, ctx: &egui::Context) {
        if self.cfg != self.saved {
            // throttle saves while a slider is being dragged
            if self.last_save.elapsed() > Duration::from_millis(150) {
                if self.cfg.save().is_ok() {
                    config::send(config::CMD_RELOAD);
                    self.saved = self.cfg.clone();
                }
                self.last_save = Instant::now();
            } else {
                ctx.request_repaint_after(Duration::from_millis(160));
            }
        } else if self.last_sync.elapsed() > Duration::from_secs(1) {
            // pick up changes made from the tray menu
            self.last_sync = Instant::now();
            let disk = Config::load();
            if disk != self.cfg {
                self.cfg = disk.clone();
                self.saved = disk;
            }
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.add(egui::Image::new(egui::load::SizedTexture::new(self.logo.id(), [38.0, 38.0])));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(
                    RichText::new("Snowie").family(FontFamily::Name("heading".into())).size(22.0).color(Color32::WHITE),
                );
                ui.label(RichText::new("снег на рабочем столе").size(12.0).color(WEAK));
            });
        });
        ui.add_space(22.0);
        for (page, icon, name) in PAGES {
            let selected = self.page == page;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), egui::Sense::click());
            let bg = if selected {
                ACCENT_DIM.gamma_multiply(0.55)
            } else if resp.hovered() {
                CONTROL.gamma_multiply(0.8)
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, 10.0, bg);
            if selected {
                let bar = egui::Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 9.0), Vec2::new(3.0, 22.0));
                ui.painter().rect_filled(bar, 2.0, ACCENT);
            }
            let color = if selected { Color32::WHITE } else { TEXT };
            let font = egui::FontId::proportional(16.0);
            ui.painter().text(
                rect.left_center() + Vec2::new(16.0, 0.0),
                egui::Align2::LEFT_CENTER,
                icon,
                font.clone(),
                if selected { ACCENT } else { WEAK },
            );
            ui.painter().text(rect.left_center() + Vec2::new(44.0, 0.0), egui::Align2::LEFT_CENTER, name, font, color);
            if resp.clicked() {
                self.page = page;
            }
            ui.add_space(2.0);
        }
        // running state at the bottom
        ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
            ui.add_space(16.0);
            let running = config::overlay_running();
            if running {
                if ui.button("Выключить").clicked() {
                    self.command(config::CMD_EXIT, "Snowie выключен");
                }
            } else if accent_button(ui, "Запустить Snowie").clicked() {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).spawn();
                    self.status = "Запускаю…".into();
                }
            }
            let (dot, text) = if running {
                (Color32::from_rgb(90, 220, 140), "Работает")
            } else {
                (WEAK, "Выключен")
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").color(dot));
                ui.label(RichText::new(text).color(TEXT));
            });
        });
    }

    fn page_snow(&mut self, ui: &mut Ui) {
        heading(ui, "Снегопад", "Как падает снег. Всё меняется сразу, перезапуск не нужен.");
        card(ui, |ui| {
            row(ui, "Пауза", "Спрятать снег и остановить всё", |ui| {
                toggle(ui, &mut self.cfg.paused);
            });
            divider(ui);
            row(ui, "Сила снегопада", "Сколько снежинок на экране", |ui| {
                slider(ui, &mut self.cfg.intensity, 0.1..=3.0, "×", 2)
            });
            divider(ui);
            row(ui, "Размер снежинок", "", |ui| slider(ui, &mut self.cfg.flake_size, 0.5..=2.0, "×", 2));
            divider(ui);
            row(ui, "Ветер", "Как сильно сносит снежинки", |ui| {
                slider(ui, &mut self.cfg.wind, 0.0..=4.0, "×", 2)
            });
        });
        card(ui, |ui| {
            row(
                ui,
                "Убрать весь снег",
                "Сугробы, снежки, снеговики и следы",
                |ui| {
                    if ui.button("Убрать").clicked() {
                        self.command(config::CMD_CLEAR, "Снег убран");
                    }
                },
            );
        });
    }

    fn page_drifts(&mut self, ui: &mut Ui) {
        heading(ui, "Сугробы", "Снег копится на окнах и на панели задач.");
        card(ui, |ui| {
            row(ui, "Снег на окнах", "Сугробы на верхних краях окон", |ui| {
                toggle(ui, &mut self.cfg.snow_on_windows);
            });
            divider(ui);
            row(ui, "Скорость накопления", "Как быстро растут сугробы", |ui| {
                slider(ui, &mut self.cfg.accumulation, 0.0..=4.0, "×", 2)
            });
            divider(ui);
            row(
                ui,
                "Высота на окнах",
                "Больше снег не держится — осыпается",
                |ui| slider(ui, &mut self.cfg.window_drift, 4.0..=120.0, " px", 0),
            );
            divider(ui);
            row(ui, "Высота на панели задач", "", |ui| {
                slider(ui, &mut self.cfg.floor_drift, 4.0..=200.0, " px", 0)
            });
        });
    }

    fn page_sound(&mut self, ui: &mut Ui) {
        heading(ui, "Звук и музыка", "Новогодние мелодии и звуки снега. Всё синтезируется программой.");
        card(ui, |ui| {
            row(
                ui,
                "Звуки",
                "Хруст снега, броски, шлепки, салют, снегоуборщик",
                |ui| {
                    toggle(ui, &mut self.cfg.sounds);
                },
            );
            divider(ui);
            row(ui, "Громкость звуков", "", |ui| percent(ui, &mut self.cfg.sound_volume));
        });
        card(ui, |ui| {
            row(
                ui,
                "Новогодняя музыка",
                "«Jingle Bells» и «We Wish You a Merry Christmas»",
                |ui| {
                    toggle(ui, &mut self.cfg.music);
                },
            );
            divider(ui);
            row(ui, "Громкость музыки", "", |ui| percent(ui, &mut self.cfg.volume));
        });
        ui.label(
            RichText::new(
                "Обе мелодии — общественное достояние. Записей в программе нет: ноты синтезируются при запуске, \
                 поэтому никаких лицензий на аудио не нужно.",
            )
            .size(12.5)
            .color(WEAK),
        );
    }

    fn page_plow(&mut self, ui: &mut Ui) {
        heading(ui, "Снегоуборщик", "Проезжает по панели задач и расчищает снег. Снеговиков объезжает.");
        card(ui, |ui| {
            row(ui, "Приезжает сам", "", |ui| {
                toggle(ui, &mut self.cfg.plow);
            });
            divider(ui);
            row(ui, "Как часто", "", |ui| slider(ui, &mut self.cfg.plow_minutes, 1.0..=60.0, " мин", 0));
        });
        card(ui, |ui| {
            row(ui, "Вызвать сейчас", "", |ui| {
                if accent_button(ui, "Вызвать").clicked() {
                    self.command(config::CMD_PLOW, "Снегоуборщик выехал");
                }
            });
        });
    }

    fn page_fireworks(&mut self, ui: &mut Ui) {
        heading(ui, "Салют", "Коробка с фейерверками на панели задач. Щелчок по ней — салют.");
        card(ui, |ui| {
            row(ui, "Коробка с салютами", "", |ui| {
                toggle(ui, &mut self.cfg.fireworks_box);
            });
            divider(ui);
            row(ui, "Место на панели задач", "Слева направо", |ui| {
                percent(ui, &mut self.cfg.box_pos)
            });
            divider(ui);
            row(ui, "Отсчёт до Нового года", "Надпись над коробкой", |ui| {
                toggle(ui, &mut self.cfg.countdown);
            });
            divider(ui);
            row(
                ui,
                "Салют в полночь",
                "Три минуты фейерверков и поздравление 1 января",
                |ui| {
                    toggle(ui, &mut self.cfg.new_year_show);
                },
            );
        });
        card(ui, |ui| {
            row(ui, "Таймеры", "Салют каждый день в заданное время", |ui| {
                let enter;
                {
                    let add = ui.button("Добавить");
                    let edit =
                        ui.add(egui::TextEdit::singleline(&mut self.new_timer).hint_text("23:30").desired_width(64.0));
                    enter = add.clicked() || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                }
                if enter {
                    match parse_timer(&self.new_timer) {
                        Some((h, m)) => {
                            let t = format!("{h:02}:{m:02}");
                            if !self.cfg.timers.contains(&t) {
                                self.cfg.timers.push(t);
                                self.cfg.timers.sort();
                            }
                            self.new_timer.clear();
                        }
                        None => self.status = "Время в формате ЧЧ:ММ, например 23:30".into(),
                    }
                }
            });
            if !self.cfg.timers.is_empty() {
                ui.add_space(4.0);
                let mut remove = None;
                ui.horizontal_wrapped(|ui| {
                    for (i, t) in self.cfg.timers.iter().enumerate() {
                        let chip = egui::Button::new(RichText::new(format!("{t}   ✕")).color(TEXT))
                            .fill(CONTROL)
                            .corner_radius(14);
                        if ui.add(chip).on_hover_text("Удалить").clicked() {
                            remove = Some(i);
                        }
                    }
                });
                if let Some(i) = remove {
                    self.cfg.timers.remove(i);
                }
            }
        });
        card(ui, |ui| {
            row(ui, "Запустить салют", "", |ui| {
                if accent_button(ui, "Бабах!").clicked() {
                    self.command(config::CMD_FIREWORKS, "Салют!");
                }
            });
        });
    }

    fn page_system(&mut self, ui: &mut Ui) {
        heading(ui, "Система", "Запуск, производительность и отладка.");
        card(ui, |ui| {
            row(ui, "Запускать вместе с Windows", "", |ui| {
                if toggle(ui, &mut self.autostart).changed() {
                    tray::set_autostart(self.autostart);
                }
            });
            divider(ui);
            row(
                ui,
                "Прятать в полноэкранных приложениях",
                "Игры, видео, презентации",
                |ui| {
                    toggle(ui, &mut self.cfg.hide_fullscreen);
                },
            );
        });
        card(ui, |ui| {
            row(
                ui,
                "Частота кадров",
                "Меньше — экономнее; при движении окон всегда плавно",
                |ui| {
                    for f in [144u32, 60, 30] {
                        let sel = self.cfg.fps == f;
                        let b = egui::Button::new(RichText::new(f.to_string()).color(if sel {
                            Color32::from_rgb(8, 20, 38)
                        } else {
                            TEXT
                        }))
                        .fill(if sel { ACCENT } else { CONTROL })
                        .corner_radius(8);
                        if ui.add(b).clicked() {
                            self.cfg.fps = f;
                        }
                    }
                },
            );
            divider(ui);
            row(
                ui,
                "Профайлер на экране",
                "Время кадра, GPU, память — в углу экрана",
                |ui| {
                    toggle(ui, &mut self.cfg.profiler);
                },
            );
        });
        card(ui, |ui| {
            row(
                ui,
                "Сбросить настройки",
                "Вернуть всё как было после установки",
                |ui| {
                    if ui.button("Сбросить").clicked() {
                        self.cfg = Config::default();
                        self.status = "Настройки сброшены".into();
                    }
                },
            );
        });
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Panel::left("nav")
            .exact_size(220.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(SIDEBAR).inner_margin(egui::Margin::symmetric(12, 0)))
            .show(ui, |ui| self.sidebar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin { left: 28, right: 28, top: 22, bottom: 12 }))
            .show(ui, |ui| {
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    // status line under the page
                    ui.label(RichText::new(&self.status).size(13.0).color(WEAK));
                    ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            // keep a little room for the scrollbar
                            let w = (ui.available_width() - 12.0).min(680.0);
                            ui.set_max_width(w);
                            ui.set_min_width(w);
                            match self.page {
                                Page::Snow => self.page_snow(ui),
                                Page::Drifts => self.page_drifts(ui),
                                Page::Sound => self.page_sound(ui),
                                Page::Plow => self.page_plow(ui),
                                Page::Fireworks => self.page_fireworks(ui),
                                Page::System => self.page_system(ui),
                            }
                        });
                    });
                });
            });
        self.persist(&ctx);
    }
}
