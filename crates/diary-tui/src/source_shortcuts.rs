//! Sources-stage source-list letter shortcuts. Bare letters are
//! allowed in exactly one place, the Resources Sidebar in the Sources stage,
//! and only while the IME indicator reads `IME: EN`: a Korean IME composes bare
//! letters, so under `IME: KO` or `IME: ?` every key is ignored. The gate and
//! the key map are pure so they are tested without a terminal, and the Help
//! Overlay text is generated from the same table that routes the keys.

use crate::focus::FocusZone;
use crate::ime::ImeState;
use crate::stage::Stage;

/// What a shortcut asks for. Each maps to the same action as its menu item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceShortcut {
    Include,
    Exclude,
    View,
    Hide,
    ShowHidden,
    Refine,
    Reject,
    Edit,
    Delete,
    Refresh,
    AddDate,
    LoadMore,
    Help,
}

impl SourceShortcut {
    pub const ALL: [Self; 13] = [
        Self::Include,
        Self::Exclude,
        Self::View,
        Self::Hide,
        Self::ShowHidden,
        Self::Refine,
        Self::Reject,
        Self::Edit,
        Self::Delete,
        Self::Refresh,
        Self::AddDate,
        Self::LoadMore,
        Self::Help,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutGroup {
    List,
    Note,
    Assistant,
}

impl ShortcutGroup {
    const fn title(self) -> &'static str {
        match self {
            Self::List => "List",
            Self::Note => "Note",
            Self::Assistant => "Assistant",
        }
    }
}

/// One row of the shortcut table: the only place a key is bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShortcutDef {
    pub shortcut: SourceShortcut,
    pub key: char,
    pub group: ShortcutGroup,
    pub label: &'static str,
}

pub const SHORTCUTS: [ShortcutDef; 13] = [
    def(
        SourceShortcut::Include,
        'i',
        ShortcutGroup::List,
        "Include the note",
    ),
    def(
        SourceShortcut::Exclude,
        'x',
        ShortcutGroup::List,
        "Exclude the note",
    ),
    def(
        SourceShortcut::Hide,
        'h',
        ShortcutGroup::List,
        "Hide the note from the list",
    ),
    def(
        SourceShortcut::ShowHidden,
        'u',
        ShortcutGroup::List,
        "Show hidden notes",
    ),
    def(
        SourceShortcut::Refresh,
        'f',
        ShortcutGroup::List,
        "Refresh notes",
    ),
    def(
        SourceShortcut::AddDate,
        'a',
        ShortcutGroup::List,
        "Add a date",
    ),
    def(
        SourceShortcut::LoadMore,
        'm',
        ShortcutGroup::List,
        "Load more notes",
    ),
    def(
        SourceShortcut::View,
        'v',
        ShortcutGroup::Note,
        "View the note's content",
    ),
    def(
        SourceShortcut::Edit,
        'e',
        ShortcutGroup::Note,
        "Edit the shown note",
    ),
    def(
        SourceShortcut::Delete,
        'd',
        ShortcutGroup::Note,
        "Delete the shown note (asks to confirm)",
    ),
    def(
        SourceShortcut::Refine,
        'r',
        ShortcutGroup::Assistant,
        "Refine with the assistant",
    ),
    def(
        SourceShortcut::Reject,
        'n',
        ShortcutGroup::Assistant,
        "Reject the shown refinement",
    ),
    def(
        SourceShortcut::Help,
        '?',
        ShortcutGroup::List,
        "Open this help",
    ),
];

const fn def(
    shortcut: SourceShortcut,
    key: char,
    group: ShortcutGroup,
    label: &'static str,
) -> ShortcutDef {
    ShortcutDef {
        shortcut,
        key,
        group,
        label,
    }
}

/// Everything the gate depends on. Built each frame from the session state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShortcutContext {
    pub zone: FocusZone,
    pub stage: Stage,
    pub ime: ImeState,
    /// A text editor, the Command Line, or a date field has the keyboard.
    pub text_input_active: bool,
    /// An Enter/Esc confirmation prompt is open.
    pub confirmation_active: bool,
    pub help_overlay_open: bool,
}

/// Whether letters may be interpreted at all. Evaluated before any key.
#[must_use]
pub fn gate_open(context: &ShortcutContext) -> bool {
    context.zone == FocusZone::Resources
        && context.stage == Stage::Sources
        && context.ime == ImeState::English
        && !context.text_input_active
        && !context.confirmation_active
        && !context.help_overlay_open
}

/// Maps a plain character to its shortcut. Returns `None` when the gate is
/// closed or the character is not bound, so the caller ignores the key.
/// Only ASCII letters and `?` count, in either case.
#[must_use]
pub fn shortcut_for(context: &ShortcutContext, ch: char) -> Option<SourceShortcut> {
    if !gate_open(context) || !(ch.is_ascii_alphabetic() || ch == '?') {
        return None;
    }
    let key = ch.to_ascii_lowercase();
    SHORTCUTS
        .iter()
        .find(|entry| entry.key == key)
        .map(|entry| entry.shortcut)
}

/// The Help Overlay body, generated from [`SHORTCUTS`]. `ime` is the current
/// indicator state so the person sees why letters may be inactive.
#[must_use]
pub fn help_lines(ime: ImeState) -> Vec<String> {
    let mut lines = vec![
        format!(
            "Letters work in the note list (Resources Sidebar) only while the IME reads IME: EN. Now: {}.",
            ime.status_label()
        ),
        "Either letter case. Every action is also in the menu (Tab to Main, Enter).".to_owned(),
        String::new(),
    ];
    for group in [
        ShortcutGroup::List,
        ShortcutGroup::Note,
        ShortcutGroup::Assistant,
    ] {
        lines.push(group.title().to_owned());
        for entry in SHORTCUTS.iter().filter(|entry| entry.group == group) {
            lines.push(format!("  {}  {}", entry.key, entry.label));
        }
        lines.push(String::new());
    }
    lines.extend([
        "No letter (focus plus Enter only)".to_owned(),
        "  Accept refinement   writes the real note".to_owned(),
        "  Confirm Source List confirms the selection".to_owned(),
        String::new(),
        "Always available".to_owned(),
        "  Tab / Shift-Tab, arrows   move focus".to_owned(),
        "  Enter                     activate; on a note, view it".to_owned(),
        "  Ctrl-G                    Command Line".to_owned(),
        "  Ctrl-P / Ctrl-N           previous / next stage".to_owned(),
        "  Ctrl-S                    save an open source edit".to_owned(),
        "  Ctrl-C twice              quit".to_owned(),
        String::new(),
        "Esc, Enter, or ? closes this help. Up/Down, PageUp/PageDown scroll.".to_owned(),
    ]);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_context() -> ShortcutContext {
        ShortcutContext {
            zone: FocusZone::Resources,
            stage: Stage::Sources,
            ime: ImeState::English,
            text_input_active: false,
            confirmation_active: false,
            help_overlay_open: false,
        }
    }

    #[test]
    fn every_shortcut_has_exactly_one_distinct_key() {
        for shortcut in SourceShortcut::ALL {
            let count = SHORTCUTS
                .iter()
                .filter(|entry| entry.shortcut == shortcut)
                .count();
            assert_eq!(count, 1, "{shortcut:?}");
        }
        let mut keys: Vec<char> = SHORTCUTS.iter().map(|entry| entry.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), SHORTCUTS.len());
    }

    #[test]
    fn every_bound_key_maps_in_either_case_when_the_gate_is_open() {
        let context = open_context();
        for entry in SHORTCUTS {
            assert_eq!(shortcut_for(&context, entry.key), Some(entry.shortcut));
            assert_eq!(
                shortcut_for(&context, entry.key.to_ascii_uppercase()),
                Some(entry.shortcut)
            );
        }
    }

    #[test]
    fn accept_and_confirm_have_no_letter() {
        // Neither action exists in the shortcut enum, so no key can reach it.
        for entry in SHORTCUTS {
            let label = entry.label.to_ascii_lowercase();
            assert!(!label.contains("accept"), "{label}");
            assert!(!label.contains("confirm source"), "{label}");
        }
    }

    #[test]
    fn no_key_is_interpreted_unless_the_ime_reads_english() {
        for ime in [ImeState::Korean, ImeState::Unknown] {
            let context = ShortcutContext {
                ime,
                ..open_context()
            };
            for entry in SHORTCUTS {
                assert_eq!(shortcut_for(&context, entry.key), None, "{ime:?}");
            }
        }
    }

    #[test]
    fn no_key_is_interpreted_outside_the_resources_zone_or_sources_stage() {
        for zone in [FocusZone::Workflow, FocusZone::Main, FocusZone::Utility] {
            let context = ShortcutContext {
                zone,
                ..open_context()
            };
            assert_eq!(shortcut_for(&context, 'i'), None, "{zone:?}");
        }
        for stage in [
            Stage::Start,
            Stage::Compose,
            Stage::Review,
            Stage::Preview,
            Stage::PublishLocal,
            Stage::PublishRemote,
        ] {
            let context = ShortcutContext {
                stage,
                ..open_context()
            };
            assert_eq!(shortcut_for(&context, 'i'), None, "{stage:?}");
        }
    }

    #[test]
    fn text_input_confirmations_and_the_overlay_close_the_gate() {
        for context in [
            ShortcutContext {
                text_input_active: true,
                ..open_context()
            },
            ShortcutContext {
                confirmation_active: true,
                ..open_context()
            },
            ShortcutContext {
                help_overlay_open: true,
                ..open_context()
            },
        ] {
            assert!(!gate_open(&context));
            assert_eq!(shortcut_for(&context, 'd'), None);
            assert_eq!(shortcut_for(&context, '?'), None);
        }
    }

    #[test]
    fn unbound_and_non_ascii_characters_are_ignored() {
        let context = open_context();
        for ch in ['z', 'c', 'g', 'p', '1', ' ', ':', 'ㅇ', 'é', '한'] {
            assert_eq!(shortcut_for(&context, ch), None, "{ch:?}");
        }
    }

    #[test]
    fn help_text_lists_every_shortcut_and_the_letterless_actions() {
        let text = help_lines(ImeState::Korean).join("\n");
        for entry in SHORTCUTS {
            assert!(
                text.contains(&format!("  {}  {}", entry.key, entry.label)),
                "help is missing {:?}",
                entry.shortcut
            );
        }
        assert!(text.contains("Accept refinement"));
        assert!(text.contains("Confirm Source List"));
        assert!(text.contains("Now: IME: KO."));
        assert!(text.contains("Ctrl-C twice"));
    }
}
