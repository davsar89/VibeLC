//! The menu bar and the video's right-click menu, built from the same functions.

use egui::{Button, Color32, Ui};

use super::{icon, icon_text};
use crate::action::{self, Action, Dialog};
use crate::app::{App, aspect_index};
use crate::audio_filters::StereoMode;
use crate::media::display_name;
use crate::player::{ASPECTS, track_label};
use crate::settings::{LoopMode, Theme};

type Menu = fn(&mut Ui, &mut App);

const MENUS: [(&str, Menu); 8] = [
    ("Media", media),
    ("Playback", playback),
    ("Audio", audio),
    ("Video", video),
    ("Subtitle", subtitle),
    ("Tools", tools),
    ("View", view),
    ("Help", help),
];

pub fn bar(ui: &mut Ui, app: &mut App) {
    egui::MenuBar::new().ui(ui, |ui| {
        for (title, menu) in MENUS {
            ui.menu_button(title, |ui| menu(ui, app));
        }
    });
}

pub fn context(ui: &mut Ui, app: &mut App) {
    let s = &app.player.state;
    let label = if !s.idle && !s.paused { "Pause" } else { "Play" };
    act(ui, app, label, None, true, Action::PlayPause);
    act(ui, app, "Stop", None, true, Action::Stop);
    act(ui, app, "Previous", None, true, Action::Previous);
    act(ui, app, "Next", None, true, Action::Next);
    ui.separator();
    act(ui, app, "Fullscreen", Some(app.win.fullscreen), true, Action::Fullscreen);
    act(ui, app, "Minimal Interface", Some(app.view.minimal), true, Action::Minimal);
    ui.separator();
    for (title, menu) in MENUS {
        ui.menu_button(title, |ui| menu(ui, app));
    }
}

/// A menu entry: optional check mark (space is kept for it, so labels line up), the shortcut,
/// and whether it can be used now. Returns whether it was clicked.
fn entry(ui: &mut Ui, label: &str, checked: Option<bool>, enabled: bool, hint: Option<Action>) -> bool {
    let color = if checked == Some(true) { ui.visuals().text_color() } else { Color32::TRANSPARENT };
    let mut button = Button::new((icon_text(icon::CHECK, 12.0).color(color), label));
    if let Some(key) = hint.and_then(action::hint) {
        button = button.shortcut_text(key);
    }
    ui.add_enabled(enabled, button).clicked()
}

fn act(ui: &mut Ui, app: &mut App, label: &str, checked: Option<bool>, enabled: bool, action: Action) {
    if entry(ui, label, checked, enabled, Some(action)) {
        app.actions.push(action);
    }
}

fn media(ui: &mut Ui, app: &mut App) {
    act(ui, app, "Open Files...", None, true, Action::OpenFiles);
    act(ui, app, "Add Files to Playlist...", None, true, Action::AddFiles);
    act(ui, app, "Open Folder...", None, true, Action::OpenFolder);
    act(ui, app, "Open Network Stream...", None, true, Action::Show(Dialog::OpenUrl));
    act(ui, app, "Open Location from Clipboard", None, true, Action::OpenClipboard);
    let recent = app.settings.recent.clone();
    ui.add_enabled_ui(!recent.is_empty(), |ui| {
        ui.menu_button("Open Recent", |ui| {
            for (i, location) in recent.iter().enumerate() {
                act(ui, app, &display_name(location), None, true, Action::OpenRecent(i));
            }
            ui.separator();
            act(ui, app, "Clear List", None, true, Action::ClearRecent);
        });
    });
    ui.separator();
    let has_list = !app.player.state.playlist.items().is_empty();
    act(ui, app, "Save Playlist...", None, has_list, Action::SavePlaylist);
    act(ui, app, "Quit at End of Playlist", Some(app.quit_at_end), true, Action::QuitAtEnd);
    ui.separator();
    act(ui, app, "Quit", None, true, Action::Quit);
}

fn playback(ui: &mut Ui, app: &mut App) {
    let s = &app.player.state;
    let (loaded, playing) = (!s.idle, !s.idle && !s.paused);
    let ab = match s.ab_loop {
        (None, _) => "Set Loop Start (A)",
        (Some(_), None) => "Set Loop End (B)",
        _ => "Clear A-B Loop",
    };
    let chapters: Vec<(String, bool)> = s
        .chapters
        .items()
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let time = crate::format::time(c.f64_of("time"));
            let title = c.str_of("title").map_or_else(|| format!("Chapter {}", i + 1), str::to_owned);
            (format!("{title}  ({time})"), s.chapter == Some(i as i64))
        })
        .collect();
    let speed = (s.speed * 100.0).round() as u16;

    act(ui, app, if playing { "Pause" } else { "Play" }, None, true, Action::PlayPause);
    act(ui, app, "Stop", None, loaded, Action::Stop);
    act(ui, app, "Previous", None, true, Action::Previous);
    act(ui, app, "Next", None, true, Action::Next);
    ui.separator();
    act(ui, app, "Jump Forward", None, loaded, Action::Seek(10));
    act(ui, app, "Jump Backward", None, loaded, Action::Seek(-10));
    act(ui, app, "Jump to Time...", None, loaded, Action::Show(Dialog::JumpToTime));
    act(ui, app, "Next Frame", None, loaded, Action::FrameStep);
    act(ui, app, ab, None, loaded, Action::AbLoop);
    ui.separator();
    ui.menu_button("Speed", |ui| {
        act(ui, app, "Faster", None, true, Action::SpeedStep(1));
        act(ui, app, "Slower", None, true, Action::SpeedStep(-1));
        ui.separator();
        for percent in [25, 50, 75, 100, 125, 150, 200, 300] {
            let label = format!("{:.2}x", f64::from(percent) / 100.0);
            act(ui, app, &label, Some(speed == percent), true, Action::SetSpeed(percent));
        }
    });
    ui.add_enabled_ui(!chapters.is_empty(), |ui| {
        ui.menu_button("Chapter", |ui| {
            act(ui, app, "Previous Chapter", None, true, Action::PreviousChapter);
            act(ui, app, "Next Chapter", None, true, Action::NextChapter);
            ui.separator();
            for (i, (label, current)) in chapters.iter().enumerate() {
                act(ui, app, label, Some(*current), true, Action::Chapter(i));
            }
        });
    });
    ui.separator();
    for (label, mode) in [("Loop Off", LoopMode::Off), ("Loop All", LoopMode::All), ("Loop One", LoopMode::One)] {
        if entry(ui, label, Some(app.settings.loop_mode == mode), true, None) {
            app.set_loop(mode);
        }
    }
    act(ui, app, "Shuffle", Some(app.shuffled), true, Action::Shuffle);
}

/// "Disable" plus every track of a type, with the selected one checked.
fn tracks(ui: &mut Ui, app: &mut App, kind: &str, select: fn(i64) -> Action) {
    let s = &app.player.state;
    let selected = s.selected(kind);
    let list: Vec<(i64, String)> = s.tracks_of(kind).filter_map(|t| Some((t.i64_of("id")?, track_label(t)))).collect();
    act(ui, app, "Disable", Some(selected.is_none()), !list.is_empty(), select(0));
    for (id, label) in list {
        act(ui, app, &label, Some(selected == Some(id)), true, select(id));
    }
}

fn audio(ui: &mut Ui, app: &mut App) {
    ui.menu_button("Audio Track", |ui| tracks(ui, app, "audio", Action::AudioTrack));
    ui.menu_button("Audio Device", |ui| {
        let s = &app.player.state;
        let devices: Vec<(String, bool)> = s
            .audio_devices
            .items()
            .iter()
            .map(|d| {
                (
                    d.str_of("description").or(d.str_of("name")).unwrap_or("?").to_string(),
                    d.str_of("name") == Some(&s.audio_device),
                )
            })
            .collect();
        for (i, (label, current)) in devices.iter().enumerate() {
            act(ui, app, label, Some(*current), true, Action::AudioDevice(i));
        }
    });
    ui.menu_button("Stereo Mode", |ui| {
        for mode in StereoMode::ALL {
            act(ui, app, mode.label(), Some(app.settings.stereo_mode == mode), true, Action::Stereo(mode));
        }
    });
    ui.separator();
    act(ui, app, "Increase Volume", None, true, Action::Volume(1));
    act(ui, app, "Decrease Volume", None, true, Action::Volume(-1));
    act(ui, app, "Mute", Some(app.player.state.muted), true, Action::Mute);
}

fn video(ui: &mut Ui, app: &mut App) {
    let s = &app.player.state;
    let (has_video, aspect, rotate, fill, deinterlace) =
        (s.has_video, aspect_index(s.aspect), s.rotate, s.panscan > 0.0, s.deinterlace);
    ui.menu_button("Video Track", |ui| tracks(ui, app, "video", Action::VideoTrack));
    act(ui, app, "Fullscreen", Some(app.win.fullscreen), true, Action::Fullscreen);
    act(ui, app, "Always on Top", Some(app.view.on_top), true, Action::OnTop);
    ui.add_enabled_ui(has_video && !app.win.fullscreen, |ui| {
        ui.menu_button("Zoom", |ui| {
            act(ui, app, "Half Size", None, true, Action::Zoom(50));
            act(ui, app, "Original Size", None, true, Action::Zoom(100));
            act(ui, app, "Double Size", None, true, Action::Zoom(200));
        });
    });
    act(ui, app, "Fill Window (Crop)", Some(fill), true, Action::Fill);
    ui.menu_button("Aspect Ratio", |ui| {
        act(ui, app, "Default", Some(aspect == 0), true, Action::Aspect(0));
        for (i, label) in ASPECTS.iter().enumerate() {
            act(ui, app, label, Some(aspect == i + 1), true, Action::Aspect(i + 1));
        }
    });
    ui.menu_button("Rotate", |ui| {
        for degrees in [0, 90, 180, 270] {
            act(ui, app, &format!("{degrees}°"), Some(rotate == i64::from(degrees)), true, Action::Rotate(degrees));
        }
    });
    act(ui, app, "Deinterlace", Some(deinterlace), true, Action::Deinterlace);
    ui.separator();
    act(ui, app, "Take Snapshot", None, has_video, Action::Snapshot);
}

fn subtitle(ui: &mut Ui, app: &mut App) {
    let loaded = !app.player.state.idle;
    act(ui, app, "Add Subtitle File...", None, loaded, Action::AddSubtitle);
    ui.menu_button("Subtitle Track", |ui| tracks(ui, app, "sub", Action::SubTrack));
    ui.separator();
    act(ui, app, "Delay +50 ms", None, loaded, Action::SubDelay(50));
    act(ui, app, "Delay -50 ms", None, loaded, Action::SubDelay(-50));
    act(ui, app, "Larger Text", None, true, Action::SubSize(10));
    act(ui, app, "Smaller Text", None, true, Action::SubSize(-10));
}

fn tools(ui: &mut Ui, app: &mut App) {
    act(ui, app, "Effects and Filters", None, true, Action::Show(Dialog::Effects));
    act(ui, app, "Media Information", None, true, Action::Show(Dialog::MediaInfo));
    act(ui, app, "Messages", None, true, Action::Show(Dialog::Messages));
    ui.separator();
    act(ui, app, "Preferences", None, true, Action::Show(Dialog::Preferences));
}

fn view(ui: &mut Ui, app: &mut App) {
    act(ui, app, "Playlist", Some(app.settings.show_playlist), true, Action::Playlist);
    act(ui, app, "Status Bar", Some(app.settings.show_status_bar), true, Action::StatusBar);
    act(ui, app, "Minimal Interface", Some(app.view.minimal), true, Action::Minimal);
    ui.menu_button("Theme", |ui| {
        for (label, theme) in [("System", Theme::System), ("Light", Theme::Light), ("Dark", Theme::Dark)] {
            act(ui, app, label, Some(app.settings.theme == theme), true, Action::Theme(theme));
        }
    });
}

fn help(ui: &mut Ui, app: &mut App) {
    act(ui, app, "Keyboard Shortcuts", None, true, Action::Show(Dialog::Shortcuts));
    act(ui, app, "About VibeLC", None, true, Action::Show(Dialog::About));
}
