//! The controller: seek bar with times, playback buttons and volume.

use egui::{Align, Button, Color32, Label, Layout, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

use super::{ACCENT, icon, icon_text};
use crate::action::{self, Action, Dialog};
use crate::app::App;
use crate::format;
use crate::settings::LoopMode;

pub fn show(ui: &mut Ui, app: &mut App) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let s = &app.player.state;
        let time_width = ui.fonts_mut(|f| {
            f.layout_no_wrap("0:00:00".into(), egui::FontId::proportional(14.0), Color32::WHITE).size().x
        }) + 4.0;
        let shown = app.view.seek_drag.or(s.time);
        ui.add_sized([time_width, 18.0], Label::new(format::time(shown.or(if s.idle { None } else { Some(0.0) }))));
        let right = match (app.view.show_remaining, s.duration, shown) {
            (true, Some(d), Some(t)) => format!("-{}", format::time(Some((d - t).max(0.0)))),
            _ => format::time(s.duration),
        };
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = ui.add_sized([time_width, 18.0], Label::new(right).sense(Sense::click()));
            if label.on_hover_text("Click to switch between total and remaining time").clicked() {
                app.view.show_remaining ^= true;
            }
            seek_bar(ui, app);
        });
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let s = &app.player.state;
        let playing = !s.idle && !s.paused;
        let (glyph, tip) = if playing { (icon::PAUSE, "Pause") } else { (icon::PLAY, "Play") };
        button(ui, app, glyph, 22.0, false, tip, Action::PlayPause);
        ui.add_space(6.0);
        button(ui, app, icon::PREVIOUS, 16.0, false, "Previous", Action::Previous);
        button(ui, app, icon::STOP, 16.0, false, "Stop", Action::Stop);
        button(ui, app, icon::NEXT, 16.0, false, "Next", Action::Next);
        ui.add_space(6.0);
        let full = app.win.fullscreen;
        let glyph = if full { icon::LEAVE_FULLSCREEN } else { icon::FULLSCREEN };
        button(ui, app, glyph, 16.0, false, "Fullscreen", Action::Fullscreen);
        let effects = app.view.dialogs.contains(&Dialog::Effects);
        button(ui, app, icon::EFFECTS, 16.0, effects, "Effects and filters", Action::Show(Dialog::Effects));
        if !full {
            button(ui, app, icon::PLAYLIST, 16.0, app.settings.show_playlist, "Playlist", Action::Playlist);
        }
        let (glyph, tip) = match app.settings.loop_mode {
            LoopMode::One => (icon::LOOP_ONE, "Loop one (click for off)"),
            LoopMode::All => (icon::LOOP, "Loop all (click for loop one)"),
            LoopMode::Off => (icon::LOOP, "Loop off (click for loop all)"),
        };
        button(ui, app, glyph, 16.0, app.settings.loop_mode != LoopMode::Off, tip, Action::CycleLoop);
        button(ui, app, icon::SHUFFLE, 16.0, app.shuffled, "Shuffle", Action::Shuffle);

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let s = &app.player.state;
            ui.add_sized([40.0, 18.0], Label::new(format!("{:.0}%", s.volume)));
            volume_bar(ui, app);
            let s = &app.player.state;
            let glyph = match s.volume {
                _ if s.muted => icon::MUTED,
                v if v <= 0.0 => icon::VOLUME_OFF,
                v if v < 60.0 => icon::VOLUME_LOW,
                _ => icon::VOLUME_HIGH,
            };
            button(ui, app, glyph, 16.0, false, if s.muted { "Unmute" } else { "Mute" }, Action::Mute);
        });
    });
    ui.add_space(2.0);
}

fn button(ui: &mut Ui, app: &mut App, glyph: char, size: f32, on: bool, tip: &str, action: Action) {
    let tip = match action::hint(action) {
        Some(key) => format!("{tip} ({key})"),
        None => tip.to_string(),
    };
    let side = size + 12.0;
    let b = Button::new(icon_text(glyph, size)).selected(on).frame_when_inactive(on).min_size(vec2(side, side));
    if ui.add(b).on_hover_text(tip).clicked() {
        app.actions.push(action);
    }
}

fn seek_bar(ui: &mut Ui, app: &mut App) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click_and_drag());
    app.view.seek_width_px = f64::from(rect.width()) * app.win.scale;
    let s = &app.player.state;
    let duration = s.duration.filter(|d| *d > 0.0 && s.seekable);
    let fraction_at = |x: f32| f64::from(((x - rect.left()) / rect.width()).clamp(0.0, 1.0));

    if let (Some(d), Some(pointer)) = (duration, response.interact_pointer_pos()) {
        let t = fraction_at(pointer.x) * d;
        if response.drag_stopped() || response.clicked() {
            app.player.command(&["seek", &t.to_string(), "absolute"]);
            app.view.seek_drag = None;
        } else if response.dragged() && app.view.seek_drag != Some(t) {
            // Keyframe seeks follow the pointer quickly; the release seeks exactly.
            app.player.command(&["seek", &t.to_string(), "absolute+keyframes"]);
            app.view.seek_drag = Some(t);
        }
    }

    let s = &app.player.state;
    let visuals = ui.visuals();
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 5.0));
    let painter = ui.painter();
    painter.rect_filled(track, 2.5, visuals.widgets.inactive.bg_fill);
    let Some(d) = duration else { return };
    let x_of = |t: f64| track.left() + track.width() * (t / d).clamp(0.0, 1.0) as f32;
    // Buffered parts of a network stream.
    for range in s.polled("demuxer-cache-state").get("seekable-ranges").map(|r| r.items()).unwrap_or_default() {
        if let (Some(a), Some(b)) = (range.f64_of("start"), range.f64_of("end")) {
            let r = Rect::from_x_y_ranges(x_of(a)..=x_of(b), track.y_range());
            painter.rect_filled(r, 2.5, visuals.widgets.hovered.bg_fill);
        }
    }
    let position = app.view.seek_drag.or(s.time).unwrap_or(0.0);
    painter.rect_filled(Rect::from_x_y_ranges(track.left()..=x_of(position), track.y_range()), 2.5, ACCENT);
    for chapter in s.chapters.items() {
        if let Some(t) = chapter.f64_of("time").filter(|t| *t > 0.0) {
            let x = x_of(t);
            painter.line_segment(
                [pos2(x, track.top() - 3.0), pos2(x, track.bottom() + 3.0)],
                Stroke::new(1.5, visuals.text_color()),
            );
        }
    }
    if response.hovered() || response.dragged() {
        painter.circle_filled(pos2(x_of(position), track.center().y), 7.0, ACCENT);
        if let Some(pointer) = response.hover_pos() {
            let t = fraction_at(pointer.x) * d;
            let chapter = s.chapters.items().iter().rev().find(|c| c.f64_of("time").is_some_and(|ct| ct <= t));
            let mut text = format::time(Some(t));
            if let Some(title) = chapter.and_then(|c| c.str_of("title")) {
                text = format!("{text}  {title}");
            }
            response.on_hover_ui_at_pointer(|ui| {
                ui.label(RichText::new(text));
            });
        }
    }
}

fn volume_bar(ui: &mut Ui, app: &mut App) {
    const MAX: f64 = 125.0;
    let (rect, response) = ui.allocate_exact_size(vec2(110.0, 18.0), Sense::click_and_drag());
    let response = response.on_hover_text("Volume (mouse wheel, Up and Down keys)");
    if let Some(pointer) = response.interact_pointer_pos() {
        let v = (f64::from((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * MAX).round();
        if v != app.player.state.volume {
            app.player.set("volume", v);
            app.player.state.volume = v;
        }
    }
    if response.hovered() {
        app.view.wheel_volume(ui, &mut app.actions);
    }
    let s = &app.player.state;
    let visuals = ui.visuals();
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 5.0));
    let painter = ui.painter();
    painter.rect_filled(track, 2.5, visuals.widgets.inactive.bg_fill);
    let x = track.left() + track.width() * (s.volume / MAX).clamp(0.0, 1.0) as f32;
    let fill = if s.muted { visuals.weak_text_color() } else { ACCENT };
    painter.rect_filled(Rect::from_x_y_ranges(track.left()..=x, track.y_range()), 2.5, fill);
    // A tick at 100%: above it mpv amplifies.
    let x100 = track.left() + track.width() * (100.0 / MAX) as f32;
    painter.line_segment(
        [pos2(x100, track.top() - 3.0), pos2(x100, track.bottom() + 3.0)],
        Stroke::new(1.0, visuals.text_color()),
    );
    if response.hovered() || response.dragged() {
        painter.circle_filled(pos2(x, track.center().y), 6.0, fill);
    }
}
