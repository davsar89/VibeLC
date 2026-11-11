//! Everything the user can ask for, and the keyboard shortcuts. Menus, buttons, keys and the
//! context menu all produce an [`Action`], so each behaves the same from every path.

use egui::{Event, Key, Modifiers};

use crate::audio_filters::StereoMode;
use crate::settings::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    OpenUrl,
    JumpToTime,
    Effects,
    MediaInfo,
    Messages,
    Preferences,
    Shortcuts,
    About,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    OpenFiles,
    AddFiles,
    OpenFolder,
    OpenClipboard,
    OpenRecent(usize),
    ClearRecent,
    SavePlaylist,
    QuitAtEnd,
    Quit,
    PlayPause,
    Stop,
    Previous,
    Next,
    /// Relative seek in seconds.
    Seek(i32),
    FrameStep,
    AbLoop,
    PreviousChapter,
    NextChapter,
    Chapter(usize),
    /// Speed change in steps of 0.1.
    SpeedStep(i8),
    /// Speed in percent.
    SetSpeed(u16),
    CycleLoop,
    Shuffle,
    /// mpv track id; 0 disables the track type.
    AudioTrack(i64),
    CycleAudio,
    AudioDevice(usize),
    Stereo(StereoMode),
    /// Volume change in steps of 5%.
    Volume(i8),
    Mute,
    /// Delay change in milliseconds.
    AudioDelay(i32),
    VideoTrack(i64),
    Fullscreen,
    LeaveFullscreen,
    OnTop,
    /// Window size in percent of the video size.
    Zoom(u16),
    Fill,
    Aspect(usize),
    CycleAspect,
    Rotate(u16),
    Deinterlace,
    Snapshot,
    AddSubtitle,
    SubTrack(i64),
    CycleSub,
    SubDelay(i32),
    /// Subtitle size change in percent.
    SubSize(i32),
    Show(Dialog),
    Playlist,
    StatusBar,
    Minimal,
    Theme(Theme),
    ShowTime,
    PlayIndex(usize),
    RemoveIndex(usize),
    ClearPlaylist,
}

impl Action {
    /// Whether holding the key repeats the action.
    fn repeats(self) -> bool {
        use Action::*;
        matches!(self, Seek(_) | Volume(_) | SpeedStep(_) | AudioDelay(_) | SubDelay(_) | SubSize(_) | FrameStep)
    }
}

pub struct Binding {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub action: Action,
    pub what: &'static str,
}

const CTRL: u8 = 1;
const SHIFT: u8 = 2;
const ALT: u8 = 4;

const fn b(key: Key, mods: u8, action: Action, what: &'static str) -> Binding {
    Binding { key, ctrl: mods & CTRL != 0, shift: mods & SHIFT != 0, alt: mods & ALT != 0, action, what }
}

/// The shortcut table, in the order the shortcuts window lists it.
pub const BINDINGS: &[Binding] = {
    use Action::*;
    use Key::*;
    &[
        b(Space, 0, PlayPause, "Play or pause"),
        b(S, 0, Stop, "Stop"),
        b(P, 0, Previous, "Previous in playlist"),
        b(N, 0, Next, "Next in playlist"),
        b(ArrowLeft, 0, Seek(-10), "Back 10 seconds"),
        b(ArrowRight, 0, Seek(10), "Forward 10 seconds"),
        b(ArrowLeft, SHIFT, Seek(-3), "Back 3 seconds"),
        b(ArrowRight, SHIFT, Seek(3), "Forward 3 seconds"),
        b(ArrowLeft, CTRL, Seek(-60), "Back 1 minute"),
        b(ArrowRight, CTRL, Seek(60), "Forward 1 minute"),
        b(ArrowLeft, CTRL | ALT, Seek(-300), "Back 5 minutes"),
        b(ArrowRight, CTRL | ALT, Seek(300), "Forward 5 minutes"),
        b(E, 0, FrameStep, "Next frame"),
        b(OpenBracket, 0, SpeedStep(-1), "Slower"),
        b(CloseBracket, 0, SpeedStep(1), "Faster"),
        b(Equals, 0, SetSpeed(100), "Normal speed"),
        b(L, 0, CycleLoop, "Loop: off, all, one"),
        b(R, 0, Shuffle, "Shuffle"),
        b(T, 0, ShowTime, "Show time"),
        b(ArrowUp, 0, Volume(1), "Volume up"),
        b(ArrowDown, 0, Volume(-1), "Volume down"),
        b(ArrowUp, CTRL, Volume(1), "Volume up"),
        b(ArrowDown, CTRL, Volume(-1), "Volume down"),
        b(M, 0, Mute, "Mute"),
        b(B, 0, CycleAudio, "Next audio track"),
        b(J, 0, AudioDelay(-50), "Audio delay -50 ms"),
        b(K, 0, AudioDelay(50), "Audio delay +50 ms"),
        b(V, 0, CycleSub, "Next subtitle track"),
        b(G, 0, SubDelay(-50), "Subtitle delay -50 ms"),
        b(H, 0, SubDelay(50), "Subtitle delay +50 ms"),
        b(F, 0, Fullscreen, "Fullscreen"),
        b(F11, 0, Fullscreen, "Fullscreen"),
        b(Escape, 0, LeaveFullscreen, "Leave fullscreen"),
        b(A, 0, CycleAspect, "Next aspect ratio"),
        b(S, SHIFT, Snapshot, "Take snapshot"),
        b(O, CTRL, OpenFiles, "Open files"),
        b(O, CTRL | SHIFT, AddFiles, "Add files to playlist"),
        b(F, CTRL, OpenFolder, "Open folder"),
        b(N, CTRL, Show(Dialog::OpenUrl), "Open network stream"),
        b(V, CTRL, OpenClipboard, "Open location from clipboard"),
        b(T, CTRL, Show(Dialog::JumpToTime), "Jump to time"),
        b(L, CTRL, Playlist, "Show playlist"),
        b(H, CTRL, Minimal, "Minimal interface"),
        b(E, CTRL, Show(Dialog::Effects), "Effects and filters"),
        b(I, CTRL, Show(Dialog::MediaInfo), "Media information"),
        b(M, CTRL, Show(Dialog::Messages), "Messages"),
        b(P, CTRL, Show(Dialog::Preferences), "Preferences"),
        b(F1, 0, Show(Dialog::Shortcuts), "Keyboard shortcuts"),
        b(Q, CTRL, Quit, "Quit"),
    ]
};

/// The action bound to `key` with exactly these modifiers. Punctuation ignores the Shift or
/// AltGr (Ctrl+Alt on Windows) that some keyboard layouts need to type it.
pub fn lookup(key: Key, m: Modifiers) -> Option<Action> {
    let punctuation = matches!(key, Key::OpenBracket | Key::CloseBracket | Key::Equals);
    let (ctrl, shift, alt) =
        if punctuation { (m.command && !m.alt, false, false) } else { (m.command, m.shift, m.alt) };
    BINDINGS.iter().find(|b| b.key == key && (b.ctrl, b.shift, b.alt) == (ctrl, shift, alt)).map(|b| b.action)
}

/// Text such as "Ctrl+Shift+O" for a binding.
pub fn key_text(b: &Binding) -> String {
    let mut s = String::new();
    for (on, name) in [(b.ctrl, "Ctrl+"), (b.alt, "Alt+"), (b.shift, "Shift+")] {
        if on {
            s.push_str(name);
        }
    }
    let arrows = matches!(b.key, Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown);
    s.push_str(if arrows { b.key.name() } else { b.key.symbol_or_name() });
    s
}

/// The first shortcut for `action`, for menu hints and tooltips.
pub fn hint(action: Action) -> Option<String> {
    BINDINGS.iter().find(|b| b.action == action).map(key_text)
}

/// Removes this frame's shortcut key presses from egui's input and returns their actions, plus
/// pasted text (egui turns Ctrl+V into a paste event). Text fields keep all keys.
pub fn take_shortcuts(ctx: &egui::Context) -> (Vec<Action>, Option<String>) {
    if ctx.egui_wants_keyboard_input() {
        return (Vec::new(), None);
    }
    let (mut actions, mut pasted) = (Vec::new(), None);
    ctx.input_mut(|i| {
        i.events.retain(|e| match e {
            Event::Key { key, pressed: true, repeat, modifiers, .. } => match lookup(*key, *modifiers) {
                Some(a) => {
                    if !repeat || a.repeats() {
                        actions.push(a);
                    }
                    false
                }
                None => true,
            },
            Event::Paste(text) => {
                pasted = Some(text.clone());
                false
            }
            _ => true,
        })
    });
    (actions, pasted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_is_bound_twice() {
        for (i, x) in BINDINGS.iter().enumerate() {
            for y in &BINDINGS[i + 1..] {
                assert!(
                    (x.key, x.ctrl, x.shift, x.alt) != (y.key, y.ctrl, y.shift, y.alt),
                    "{} is bound twice",
                    key_text(x)
                );
            }
        }
    }

    #[test]
    fn modifiers_must_match_exactly() {
        let shift = Modifiers { shift: true, ..Modifiers::NONE };
        let ctrl_alt = Modifiers { ctrl: true, command: true, alt: true, ..Modifiers::NONE };
        assert_eq!(lookup(Key::ArrowRight, Modifiers::NONE), Some(Action::Seek(10)));
        assert_eq!(lookup(Key::ArrowRight, shift), Some(Action::Seek(3)));
        assert_eq!(lookup(Key::ArrowRight, ctrl_alt), Some(Action::Seek(300)));
        assert_eq!(lookup(Key::S, shift), Some(Action::Snapshot));
        assert_eq!(lookup(Key::S, Modifiers::COMMAND), None);
        assert_eq!(lookup(Key::Q, Modifiers::NONE), None);
        // "[" typed with AltGr on Windows, "=" typed with Shift on Nordic layouts.
        assert_eq!(lookup(Key::OpenBracket, ctrl_alt), Some(Action::SpeedStep(-1)));
        assert_eq!(lookup(Key::Equals, shift), Some(Action::SetSpeed(100)));
    }

    #[test]
    fn hints_read_naturally() {
        assert_eq!(hint(Action::AddFiles).as_deref(), Some("Ctrl+Shift+O"));
        assert_eq!(hint(Action::Seek(-300)).as_deref(), Some("Ctrl+Alt+Left"));
        assert_eq!(hint(Action::SpeedStep(1)).as_deref(), Some("]"));
        assert_eq!(hint(Action::Fullscreen).as_deref(), Some("F"));
        assert_eq!(hint(Action::ClearRecent), None);
    }
}
