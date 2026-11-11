//! The playlist panel. The list itself is mpv's `playlist` property.

use egui::{Align2, FontId, Key, ScrollArea, Sense, TextStyle, Ui, text::LayoutJob, vec2};

use super::{ACCENT, icon, icon_text};
use crate::action::Action;
use crate::app::App;
use crate::format;
use crate::media::display_name;

pub fn show(ui: &mut Ui, app: &mut App) {
    let s = &app.player.state;
    let entries = s.playlist.items();
    // Title and duration of each entry: the playlist's own title, else the file's tag, else the
    // file name; the playing file's duration comes from the player.
    let rows: Vec<(String, Option<f64>, bool)> = entries
        .iter()
        .map(|e| {
            let file = e.str_of("filename").unwrap_or_default();
            let info = app.probed.get(file);
            let current = e.bool_of("current") == Some(true);
            let title = e
                .str_of("title")
                .or(info.and_then(|i| i.title.as_deref()))
                .map_or_else(|| display_name(file), str::to_owned);
            let duration = if current && s.duration.is_some() { s.duration } else { info.and_then(|i| i.duration) };
            (title, duration, current)
        })
        .collect();

    ui.horizontal(|ui| {
        ui.strong("Playlist");
        let total: f64 = rows.iter().filter_map(|r| r.1).sum();
        ui.weak(format!("{} items, {}", rows.len(), format::time(Some(total))));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new(icon_text(icon::CLEAR, 14.0)).frame_when_inactive(false))
                .on_hover_text("Clear the playlist")
                .clicked()
            {
                app.actions.push(Action::ClearPlaylist);
            }
            if ui
                .add(egui::Button::new(icon_text(icon::ADD, 14.0)).frame_when_inactive(false))
                .on_hover_text("Add files")
                .clicked()
            {
                app.actions.push(Action::AddFiles);
            }
        });
    });
    ui.separator();
    if rows.is_empty() {
        ui.weak("Drop media here, or use Media > Open Files.");
        return;
    }

    let row_height = ui.text_style_height(&TextStyle::Body) + 8.0;
    ScrollArea::vertical().auto_shrink(false).show_rows(ui, row_height, rows.len(), |ui, range| {
        for i in range {
            let (title, duration, current) = &rows[i];
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
            let selected = app.view.playlist_selected == Some(i);
            let visuals = ui.visuals();
            if selected {
                ui.painter().rect_filled(rect, 3.0, visuals.selection.bg_fill.gamma_multiply(0.35));
            } else if response.hovered() {
                ui.painter().rect_filled(rect, 3.0, visuals.widgets.hovered.weak_bg_fill);
            }
            let color = if *current { ACCENT } else { visuals.text_color() };
            let duration = format::time(*duration);
            let font = FontId::proportional(14.0);
            let duration_width = ui.painter().layout_no_wrap(duration.clone(), font.clone(), color).size().x;
            let mut job = LayoutJob::simple_singleline(format!("{}. {title}", i + 1), font.clone(), color);
            job.wrap.max_width = rect.width() - duration_width - 16.0;
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(rect.left_center() + vec2(4.0, -galley.size().y / 2.0), galley, color);
            ui.painter().text(
                rect.right_center() - vec2(4.0, 0.0),
                Align2::RIGHT_CENTER,
                duration,
                font,
                color.gamma_multiply(0.8),
            );
            if response.clicked() {
                app.view.playlist_selected = Some(i);
            }
            if response.double_clicked() {
                app.actions.push(Action::PlayIndex(i));
            }
            response.context_menu(|ui| {
                if ui.button("Play").clicked() {
                    app.actions.push(Action::PlayIndex(i));
                }
                if ui.button("Remove").clicked() {
                    app.actions.push(Action::RemoveIndex(i));
                }
                ui.separator();
                if ui.button("Clear the Playlist").clicked() {
                    app.actions.push(Action::ClearPlaylist);
                }
            });
        }
    });
    let typing = ui.ctx().egui_wants_keyboard_input();
    if let Some(i) = app.view.playlist_selected.filter(|_| !typing) {
        if ui.input(|inp| inp.key_pressed(Key::Delete)) {
            app.actions.push(Action::RemoveIndex(i));
            app.view.playlist_selected = None;
        } else if ui.input(|inp| inp.key_pressed(Key::Enter)) {
            app.actions.push(Action::PlayIndex(i));
        }
    }
}
