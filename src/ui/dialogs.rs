//! Dialog windows. A dialog is open while it is listed in `View::dialogs`.

use std::time::Duration;

use egui::{Color32, ComboBox, Context, Grid, Key, RichText, ScrollArea, Slider, TextEdit, Ui, Window};

use crate::action::{BINDINGS, Dialog, key_text};
use crate::app::{App, Pick, Request};
use crate::audio_filters::{EQ_BANDS_HZ, EQ_PRESETS, EQ_RANGE_DB, StereoMode};
use crate::format;
use crate::mpv::Node;
use crate::player::PICTURE;
use crate::settings::{RenderProfile, Resume, Theme};

pub fn show(ctx: &Context, app: &mut App) {
    for dialog in app.view.dialogs.clone() {
        let (title, resizable) = match dialog {
            Dialog::OpenUrl => ("Open Network Stream", false),
            Dialog::JumpToTime => ("Jump to Time", false),
            Dialog::Effects => ("Effects and Filters", false),
            Dialog::MediaInfo => ("Media Information", true),
            Dialog::Messages => ("Messages", true),
            Dialog::Preferences => ("Preferences", false),
            Dialog::Shortcuts => ("Keyboard Shortcuts", true),
            Dialog::About => ("About VibeLC", false),
        };
        let mut open = true;
        let mut done = false;
        Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(resizable)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                done = match dialog {
                    Dialog::OpenUrl => open_url(ui, app),
                    Dialog::JumpToTime => jump_to_time(ui, app),
                    Dialog::Effects => effects(ui, app),
                    Dialog::MediaInfo => media_info(ui, app),
                    Dialog::Messages => messages(ui, app),
                    Dialog::Preferences => preferences(ui, app),
                    Dialog::Shortcuts => shortcuts(ui),
                    Dialog::About => about(ui, app),
                }
            });
        if !open || done {
            app.view.dialogs.retain(|d| *d != dialog);
            if dialog == Dialog::Preferences {
                app.save_settings();
            }
        }
    }
}

/// A text field that takes the keyboard focus when its dialog opens; returns (field, Enter pressed).
fn field(ui: &mut Ui, text: &mut String, width: f32, hint: &str, focus: &mut bool) -> bool {
    let r = ui.add(TextEdit::singleline(text).desired_width(width).hint_text(hint));
    if std::mem::take(focus) {
        r.request_focus();
    }
    r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
}

fn open_url(ui: &mut Ui, app: &mut App) -> bool {
    ui.label("Address of a stream (http, https, rtsp, ...) or a file:");
    let enter = field(ui, &mut app.view.url, 460.0, "https://", &mut app.view.focus);
    let (mut play, mut add, mut cancel) = (enter, false, false);
    ui.horizontal(|ui| {
        play |= ui.button("Play").clicked();
        add = ui.button("Add to Playlist").clicked();
        cancel = ui.button("Cancel").clicked();
    });
    let url = app.view.url.trim().to_string();
    if (play || add) && !url.is_empty() {
        app.open(vec![url.into()], add);
        app.view.url.clear();
        return true;
    }
    cancel
}

fn jump_to_time(ui: &mut Ui, app: &mut App) -> bool {
    ui.label("Time (h:mm:ss, mm:ss or seconds):");
    let enter = field(ui, &mut app.view.jump, 160.0, "0:00", &mut app.view.focus);
    let time = format::parse_time(&app.view.jump);
    if time.is_none() {
        ui.colored_label(Color32::from_rgb(230, 90, 80), "Not a valid time");
    }
    let (mut go, mut cancel) = (enter, false);
    ui.horizontal(|ui| {
        go |= ui.add_enabled(time.is_some(), egui::Button::new("Go")).clicked();
        cancel = ui.button("Cancel").clicked();
    });
    if go && let Some(t) = time {
        app.player.command(&["seek", &t.to_string(), "absolute"]);
        return true;
    }
    cancel
}

fn tabs(ui: &mut Ui, current: &mut usize, names: &[&str]) {
    ui.horizontal(|ui| {
        for (i, name) in names.iter().enumerate() {
            ui.selectable_value(current, i, *name);
        }
    });
    ui.separator();
}

fn effects(ui: &mut Ui, app: &mut App) -> bool {
    tabs(ui, &mut app.view.effects_tab, &["Audio", "Video", "Synchronization"]);
    match app.view.effects_tab {
        0 => audio_effects(ui, app),
        1 => {
            let mut picture = app.player.state.picture;
            for (i, name) in PICTURE.iter().enumerate() {
                let label = format!("{}{}", name[..1].to_uppercase(), &name[1..]);
                if ui.add(Slider::new(&mut picture[i], -100..=100).text(label)).changed() {
                    app.player.set(name, picture[i]);
                }
            }
            if ui.button("Reset").clicked() {
                for name in PICTURE {
                    app.player.set(name, 0i64);
                }
            }
        }
        _ => {
            let s = &app.player.state;
            let (mut audio_ms, mut sub_ms, mut size) =
                (s.audio_delay * 1000.0, s.sub_delay * 1000.0, s.sub_scale * 100.0);
            if ui.add(Slider::new(&mut audio_ms, -5000.0..=5000.0).text("Audio delay (ms)").step_by(10.0)).changed() {
                app.player.set("audio-delay", audio_ms / 1000.0);
            }
            if ui.add(Slider::new(&mut sub_ms, -10000.0..=10000.0).text("Subtitle delay (ms)").step_by(10.0)).changed()
            {
                app.player.set("sub-delay", sub_ms / 1000.0);
            }
            if ui.add(Slider::new(&mut size, 10.0..=500.0).text("Subtitle size (%)").step_by(5.0)).changed() {
                app.player.set("sub-scale", size / 100.0);
                app.settings.subtitle_scale = size / 100.0;
            }
            if ui.button("Reset").clicked() {
                app.player.set("audio-delay", 0.0);
                app.player.set("sub-delay", 0.0);
            }
        }
    }
    false
}

fn audio_effects(ui: &mut Ui, app: &mut App) {
    let settings = &mut app.settings;
    let eq = &mut settings.equalizer;
    let mut apply = ui.checkbox(&mut eq.enabled, "Equalizer").changed();
    let preset = EQ_PRESETS.iter().find(|p| p.preamp_db == eq.preamp_db && p.gains_db == eq.gains_db);
    ComboBox::from_label("Preset").selected_text(preset.map_or("Custom", |p| p.name)).show_ui(ui, |ui| {
        for p in EQ_PRESETS {
            if ui.selectable_label(preset.is_some_and(|x| x.name == p.name), p.name).clicked() {
                (eq.enabled, eq.preamp_db, eq.gains_db) = (true, p.preamp_db, p.gains_db);
                apply = true;
            }
        }
    });
    ui.add_enabled_ui(eq.enabled, |ui| {
        ui.horizontal(|ui| {
            let labels = EQ_BANDS_HZ.map(|hz| if hz >= 1000 { format!("{}K", hz / 1000) } else { hz.to_string() });
            let bands = std::iter::once(("Preamp".to_string(), &mut eq.preamp_db))
                .chain(labels.into_iter().zip(eq.gains_db.iter_mut()));
            for (label, value) in bands {
                ui.vertical(|ui| {
                    let r = ui
                        .add(Slider::new(value, -EQ_RANGE_DB..=EQ_RANGE_DB).vertical().show_value(false).step_by(0.5));
                    // Rebuilding the filter interrupts the sound briefly, so apply on release.
                    apply |= r.drag_stopped() || (r.changed() && !r.dragged());
                    ui.small(format!("{value:+.1}"));
                    ui.small(label);
                });
            }
        });
    });
    ComboBox::from_label("Stereo mode").selected_text(settings.stereo_mode.label()).show_ui(ui, |ui| {
        for mode in StereoMode::ALL {
            apply |= ui.selectable_value(&mut settings.stereo_mode, mode, mode.label()).changed();
        }
    });
    apply |= ui.checkbox(&mut settings.normalize_volume, "Normalize volume").changed();
    if apply {
        app.player.apply_audio_filters(&app.settings);
    }
}

fn grid(ui: &mut Ui, id: &str, rows: &[(String, String)]) {
    Grid::new(id).num_columns(2).striped(true).show(ui, |ui| {
        for (k, v) in rows {
            ui.label(RichText::new(k).strong());
            ui.label(v);
            ui.end_row();
        }
    });
}

fn media_info(ui: &mut Ui, app: &mut App) -> bool {
    tabs(ui, &mut app.view.info_tab, &["General", "Codec", "Statistics"]);
    let s = &app.player.state;
    let text = |n: &Node| match n {
        Node::String(v) => v.clone(),
        Node::Int(v) => v.to_string(),
        Node::Double(v) => format!("{v:.3}"),
        Node::Flag(v) => if *v { "yes" } else { "no" }.to_string(),
        _ => String::new(),
    };
    ScrollArea::vertical().max_height(420.0).show(ui, |ui| match app.view.info_tab {
        0 => {
            let mut rows = vec![
                ("Title".to_string(), s.title.clone().unwrap_or_default()),
                ("Location".to_string(), s.path.clone().unwrap_or_default()),
                ("Duration".to_string(), format::time(s.duration)),
            ];
            rows.extend(s.file_format.clone().map(|f| ("Format".to_string(), f)));
            rows.extend(s.file_size.map(|b| ("Size".to_string(), format::bytes(b))));
            if let Node::Map(tags) = &s.metadata {
                rows.extend(
                    tags.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("title")).map(|(k, v)| (k.clone(), text(v))),
                );
            }
            grid(ui, "general", &rows);
        }
        1 => {
            const FIELDS: [(&str, &str); 10] = [
                ("codec", "Codec"),
                ("title", "Title"),
                ("lang", "Language"),
                ("demux-w", "Width"),
                ("demux-h", "Height"),
                ("demux-fps", "Frame rate"),
                ("demux-channel-count", "Channels"),
                ("demux-samplerate", "Sample rate"),
                ("demux-bitrate", "Bitrate"),
                ("external-filename", "File"),
            ];
            for t in s.tracks.items() {
                let kind = match t.str_of("type") {
                    Some("video") => "Video",
                    Some("audio") => "Audio",
                    _ => "Subtitle",
                };
                ui.strong(format!("{kind} track {}", t.i64_of("id").unwrap_or(0)));
                let rows: Vec<(String, String)> =
                    FIELDS.iter().filter_map(|(key, label)| t.get(key).map(|v| (label.to_string(), text(v)))).collect();
                grid(ui, &format!("track{}{kind}", t.i64_of("id").unwrap_or(0)), &rows);
                ui.add_space(6.0);
            }
        }
        _ => {
            let polled = |name| text(s.polled(name));
            let mut rows = vec![
                ("Decoding".to_string(), s.hwdec.clone().map_or("software".to_string(), |h| format!("hardware ({h})"))),
                ("Displayed frame rate".to_string(), polled("estimated-vf-fps")),
                ("Dropped frames (output)".to_string(), polled("frame-drop-count")),
                ("Dropped frames (decoder)".to_string(), polled("decoder-frame-drop-count")),
                ("Audio/video offset (s)".to_string(), polled("avsync")),
            ];
            for (name, label) in [("video-bitrate", "Video bitrate"), ("audio-bitrate", "Audio bitrate")] {
                if let Some(b) = s.polled(name).as_f64() {
                    rows.push((label.to_string(), format!("{:.0} kb/s", b / 1000.0)));
                }
            }
            if let Some(cache) = s.polled("demuxer-cache-state").f64_of("cache-duration") {
                rows.push(("Buffered".to_string(), format!("{cache:.1} s")));
            }
            grid(ui, "stats", &rows);
        }
    });
    false
}

fn messages(ui: &mut Ui, app: &mut App) -> bool {
    const LEVELS: [(&str, log::Level); 4] = [
        ("Errors", log::Level::Error),
        ("Warnings", log::Level::Warn),
        ("Information", log::Level::Info),
        ("Debug", log::Level::Debug),
    ];
    let mut copy = false;
    ui.horizontal(|ui| {
        let level = &mut app.view.log_level;
        ComboBox::from_label("Show").selected_text(LEVELS[(*level).min(3)].0).show_ui(ui, |ui| {
            for (i, (name, _)) in LEVELS.iter().enumerate() {
                ui.selectable_value(level, i, *name);
            }
        });
        copy = ui.button("Copy All").clicked();
        if ui.button("Clear").clicked() {
            crate::logging::with_lines(|lines| lines.clear());
        }
    });
    let max = LEVELS[app.view.log_level.min(3)].1;
    let lines: Vec<(log::Level, String)> = crate::logging::with_lines(|lines| {
        lines.iter().filter(|l| l.level <= max).map(|l| (l.level, l.text.clone())).collect()
    })
    .unwrap_or_default();
    if copy {
        ui.ctx().copy_text(lines.iter().map(|l| l.1.as_str()).collect::<Vec<_>>().join("\n"));
    }
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    ScrollArea::both().stick_to_bottom(true).auto_shrink(false).max_height(400.0).show_rows(
        ui,
        row_height,
        lines.len(),
        |ui, range| {
            for (level, text) in &lines[range] {
                let color = match level {
                    log::Level::Error => Color32::from_rgb(230, 90, 80),
                    log::Level::Warn => Color32::from_rgb(220, 170, 60),
                    _ => ui.visuals().text_color(),
                };
                ui.label(RichText::new(text).monospace().color(color));
            }
        },
    );
    ui.ctx().request_repaint_after(Duration::from_secs(1));
    false
}

fn preferences(ui: &mut Ui, app: &mut App) -> bool {
    let mut done = false;
    let s = &mut app.settings;
    Grid::new("prefs").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
        ui.label("Theme");
        ComboBox::from_id_salt("theme").selected_text(format!("{:?}", s.theme)).show_ui(ui, |ui| {
            for theme in [Theme::System, Theme::Light, Theme::Dark] {
                ui.selectable_value(&mut s.theme, theme, format!("{theme:?}"));
            }
        });
        ui.end_row();

        ui.label("Video quality");
        let before = s.profile;
        ComboBox::from_id_salt("profile").selected_text(s.profile.label()).show_ui(ui, |ui| {
            for p in RenderProfile::ALL {
                ui.selectable_value(&mut s.profile, p, p.label());
            }
        });
        if s.profile != before {
            app.player.apply_profile(s);
        }
        ui.end_row();

        ui.label("Hardware decoding");
        if ui.checkbox(&mut s.hardware_decoding, "Use the graphics card to decode video").changed() {
            app.player.set("hwdec", s.hwdec());
        }
        ui.end_row();

        ui.label("Window");
        ui.checkbox(&mut s.resize_to_video, "Resize to fit the video");
        ui.end_row();

        ui.label("Continue playback");
        ComboBox::from_id_salt("resume").selected_text(format!("{:?}", s.resume)).show_ui(ui, |ui| {
            for (mode, label) in [(Resume::Ask, "Ask"), (Resume::Always, "Always"), (Resume::Never, "Never")] {
                ui.selectable_value(&mut s.resume, mode, label);
            }
        });
        ui.end_row();

        ui.label("Opening from other programs");
        ui.checkbox(&mut s.single_instance, "Use the window that is already open");
        ui.end_row();

        for (label, value, property) in [
            ("Audio languages", &mut s.audio_languages, "alang"),
            ("Subtitle languages", &mut s.subtitle_languages, "slang"),
        ] {
            ui.label(label);
            let r = ui.add(TextEdit::singleline(value).hint_text("e.g. nor,eng").desired_width(200.0));
            if r.lost_focus() {
                app.player.set(property, value.as_str());
            }
            ui.end_row();
        }

        ui.label("Snapshots");
        ui.horizontal(|ui| {
            let folder =
                s.screenshot_folder.as_ref().map_or("Pictures folder".to_string(), |p| p.display().to_string());
            ui.label(folder);
            if ui.button("Change...").clicked() {
                app.requests.push(Request::Pick(Pick::SnapshotFolder));
            }
        });
        ui.end_row();
    });
    ui.separator();
    if ui.button("Close").clicked() {
        done = true;
    }
    done
}

fn shortcuts(ui: &mut Ui) -> bool {
    ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
        Grid::new("keys").num_columns(2).striped(true).show(ui, |ui| {
            let mut i = 0;
            while i < BINDINGS.len() {
                // Consecutive bindings of one action share a row.
                let same = BINDINGS[i..].iter().take_while(|b| b.action == BINDINGS[i].action).count();
                let keys: Vec<String> = BINDINGS[i..i + same].iter().map(key_text).collect();
                ui.label(RichText::new(keys.join(", ")).monospace());
                ui.label(BINDINGS[i].what);
                ui.end_row();
                i += same;
            }
        });
    });
    false
}

fn about(ui: &mut Ui, app: &mut App) -> bool {
    ui.heading(format!("VibeLC {}", env!("CARGO_PKG_VERSION")));
    ui.label("Fully vibe coded lightweight vlc like video player.");
    ui.add_space(6.0);
    ui.label(format!("Playback engine: {}", app.player.state.mpv_version));
    ui.label("Licence: GNU GPL 3.0 or later.");
    ui.label("Icons: Lucide (ISC licence). Font: Inter (SIL Open Font Licence).");
    if let Some(dir) = crate::settings::config_dir() {
        ui.label(format!("Settings and log: {}", dir.display()));
    }
    ui.add_space(6.0);
    ui.button("Close").clicked()
}
