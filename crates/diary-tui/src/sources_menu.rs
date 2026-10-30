//! The flat Sources-stage menu contains the items that apply to the current
//! state, in a fixed order. A menu item and its letter shortcut resolve to the
//! same [`SourcesAction`], so both share the same availability rules.

use crate::source_shortcuts::SourceShortcut;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourcesAction {
    Include,
    Exclude,
    ToggleInclude,
    View,
    Hide,
    ShowHidden,
    Refine,
    Accept,
    Reject,
    Edit,
    Delete,
    ConfirmSourceList,
    Refresh,
    AddDate,
    LoadMore,
    Help,
    BackToStart,
}

/// A menu row. `ToggleInclude` is the single Include/Exclude row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourcesMenuItem {
    ToggleInclude,
    View,
    Hide,
    ShowHidden,
    Refine,
    Accept,
    Reject,
    Edit,
    Delete,
    ConfirmSourceList,
    Refresh,
    AddDate,
    LoadMore,
    Help,
    BackToStart,
}

/// The state the menu depends on. Built by `SourcesViewModel::menu_state`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourcesMenuState {
    pub candidate_focused: bool,
    /// The focused note is resident or cloud-only, so it can be read.
    pub focused_readable: bool,
    pub focused_included: bool,
    /// The focused note's text is displayed in the Main Pane.
    pub focused_shown: bool,
    pub hidden_count: usize,
    pub assistant_backend_selected: bool,
    /// A refinement suggestion for the focused note is displayed.
    pub suggestion_shown: bool,
    pub has_more: bool,
    pub included_count: usize,
}

#[must_use]
pub fn visible_items(state: &SourcesMenuState) -> Vec<SourcesMenuItem> {
    use SourcesMenuItem as Item;
    let mut items = Vec::with_capacity(15);
    if state.candidate_focused && (state.focused_included || state.focused_readable) {
        items.push(Item::ToggleInclude);
    }
    if state.candidate_focused && state.focused_readable && !state.focused_shown {
        items.push(Item::View);
    }
    if state.candidate_focused {
        items.push(Item::Hide);
    }
    if state.hidden_count > 0 {
        items.push(Item::ShowHidden);
    }
    if state.focused_shown && state.assistant_backend_selected {
        items.push(Item::Refine);
    }
    if state.suggestion_shown {
        items.push(Item::Accept);
        items.push(Item::Reject);
    }
    if state.focused_shown {
        items.push(Item::Edit);
        items.push(Item::Delete);
    }
    items.push(Item::ConfirmSourceList);
    items.push(Item::Refresh);
    items.push(Item::AddDate);
    if state.has_more {
        items.push(Item::LoadMore);
    }
    items.push(Item::Help);
    items.push(Item::BackToStart);
    items
}

#[must_use]
pub fn label(item: SourcesMenuItem, state: &SourcesMenuState) -> String {
    match item {
        SourcesMenuItem::ToggleInclude if state.focused_included => "Exclude this note".to_owned(),
        SourcesMenuItem::ToggleInclude => "Include this note".to_owned(),
        SourcesMenuItem::View => "View content".to_owned(),
        SourcesMenuItem::Hide => "Hide note".to_owned(),
        SourcesMenuItem::ShowHidden => format!("Show hidden notes ({})", state.hidden_count),
        SourcesMenuItem::Refine => "Refine with assistant".to_owned(),
        SourcesMenuItem::Accept => "Accept refinement (writes the note)".to_owned(),
        SourcesMenuItem::Reject => "Reject refinement".to_owned(),
        SourcesMenuItem::Edit => "Edit note (Ctrl-S saves)".to_owned(),
        SourcesMenuItem::Delete => "Delete note and linked audio".to_owned(),
        SourcesMenuItem::ConfirmSourceList if state.included_count == 0 => {
            "Continue without notes".to_owned()
        }
        SourcesMenuItem::ConfirmSourceList
            if state.included_count > diary_control_contract::MAX_SELECTED_SOURCES =>
        {
            "Confirm Source List (disabled: exclude notes to reach the 64-note limit)".to_owned()
        }
        SourcesMenuItem::ConfirmSourceList => "Confirm Source List".to_owned(),
        SourcesMenuItem::Refresh => "Refresh notes".to_owned(),
        SourcesMenuItem::AddDate => "Add a date".to_owned(),
        SourcesMenuItem::LoadMore => "Load more".to_owned(),
        SourcesMenuItem::Help => "Shortcuts help".to_owned(),
        SourcesMenuItem::BackToStart => "Back to Start".to_owned(),
    }
}

/// The action a menu row performs when activated.
#[must_use]
pub const fn item_action(item: SourcesMenuItem) -> SourcesAction {
    match item {
        SourcesMenuItem::ToggleInclude => SourcesAction::ToggleInclude,
        SourcesMenuItem::View => SourcesAction::View,
        SourcesMenuItem::Hide => SourcesAction::Hide,
        SourcesMenuItem::ShowHidden => SourcesAction::ShowHidden,
        SourcesMenuItem::Refine => SourcesAction::Refine,
        SourcesMenuItem::Accept => SourcesAction::Accept,
        SourcesMenuItem::Reject => SourcesAction::Reject,
        SourcesMenuItem::Edit => SourcesAction::Edit,
        SourcesMenuItem::Delete => SourcesAction::Delete,
        SourcesMenuItem::ConfirmSourceList => SourcesAction::ConfirmSourceList,
        SourcesMenuItem::Refresh => SourcesAction::Refresh,
        SourcesMenuItem::AddDate => SourcesAction::AddDate,
        SourcesMenuItem::LoadMore => SourcesAction::LoadMore,
        SourcesMenuItem::Help => SourcesAction::Help,
        SourcesMenuItem::BackToStart => SourcesAction::BackToStart,
    }
}

/// The action a letter shortcut performs.
#[must_use]
pub const fn shortcut_action(shortcut: SourceShortcut) -> SourcesAction {
    match shortcut {
        SourceShortcut::Include => SourcesAction::Include,
        SourceShortcut::Exclude => SourcesAction::Exclude,
        SourceShortcut::View => SourcesAction::View,
        SourceShortcut::Hide => SourcesAction::Hide,
        SourceShortcut::ShowHidden => SourcesAction::ShowHidden,
        SourceShortcut::Refine => SourcesAction::Refine,
        SourceShortcut::Reject => SourcesAction::Reject,
        SourceShortcut::Edit => SourcesAction::Edit,
        SourceShortcut::Delete => SourcesAction::Delete,
        SourceShortcut::Refresh => SourcesAction::Refresh,
        SourceShortcut::AddDate => SourcesAction::AddDate,
        SourceShortcut::LoadMore => SourcesAction::LoadMore,
        SourceShortcut::Help => SourcesAction::Help,
    }
}

/// The menu row an action belongs to, so a shortcut for an item that is not
/// currently shown can be refused. `Help` is always available.
#[must_use]
pub const fn action_item(action: SourcesAction) -> SourcesMenuItem {
    match action {
        SourcesAction::Include | SourcesAction::Exclude | SourcesAction::ToggleInclude => {
            SourcesMenuItem::ToggleInclude
        }
        SourcesAction::View => SourcesMenuItem::View,
        SourcesAction::Hide => SourcesMenuItem::Hide,
        SourcesAction::ShowHidden => SourcesMenuItem::ShowHidden,
        SourcesAction::Refine => SourcesMenuItem::Refine,
        SourcesAction::Accept => SourcesMenuItem::Accept,
        SourcesAction::Reject => SourcesMenuItem::Reject,
        SourcesAction::Edit => SourcesMenuItem::Edit,
        SourcesAction::Delete => SourcesMenuItem::Delete,
        SourcesAction::ConfirmSourceList => SourcesMenuItem::ConfirmSourceList,
        SourcesAction::Refresh => SourcesMenuItem::Refresh,
        SourcesAction::AddDate => SourcesMenuItem::AddDate,
        SourcesAction::LoadMore => SourcesMenuItem::LoadMore,
        SourcesAction::Help => SourcesMenuItem::Help,
        SourcesAction::BackToStart => SourcesMenuItem::BackToStart,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_shortcuts::SourceShortcut;

    fn base() -> SourcesMenuState {
        SourcesMenuState {
            candidate_focused: true,
            focused_readable: true,
            ..SourcesMenuState::default()
        }
    }

    #[test]
    fn fresh_readable_note_offers_include_view_hide_and_the_always_items() {
        let items = visible_items(&base());
        assert_eq!(
            items,
            vec![
                SourcesMenuItem::ToggleInclude,
                SourcesMenuItem::View,
                SourcesMenuItem::Hide,
                SourcesMenuItem::ConfirmSourceList,
                SourcesMenuItem::Refresh,
                SourcesMenuItem::AddDate,
                SourcesMenuItem::Help,
                SourcesMenuItem::BackToStart,
            ]
        );
    }

    #[test]
    fn shown_note_adds_edit_delete_and_refine_only_with_a_backend() {
        let shown = SourcesMenuState {
            focused_shown: true,
            ..base()
        };
        let items = visible_items(&shown);
        assert!(!items.contains(&SourcesMenuItem::View));
        assert!(items.contains(&SourcesMenuItem::Edit));
        assert!(items.contains(&SourcesMenuItem::Delete));
        assert!(!items.contains(&SourcesMenuItem::Refine));
        let with_backend = SourcesMenuState {
            assistant_backend_selected: true,
            ..shown
        };
        assert!(visible_items(&with_backend).contains(&SourcesMenuItem::Refine));
    }

    #[test]
    fn accept_and_reject_appear_only_while_a_suggestion_is_shown() {
        assert!(!visible_items(&base()).contains(&SourcesMenuItem::Accept));
        let suggested = SourcesMenuState {
            focused_shown: true,
            suggestion_shown: true,
            ..base()
        };
        let items = visible_items(&suggested);
        assert!(items.contains(&SourcesMenuItem::Accept));
        assert!(items.contains(&SourcesMenuItem::Reject));
    }

    #[test]
    fn unreadable_unincluded_note_cannot_be_included_or_viewed_but_can_be_hidden() {
        let state = SourcesMenuState {
            focused_readable: false,
            ..base()
        };
        let items = visible_items(&state);
        assert!(!items.contains(&SourcesMenuItem::ToggleInclude));
        assert!(!items.contains(&SourcesMenuItem::View));
        assert!(items.contains(&SourcesMenuItem::Hide));
        let included = SourcesMenuState {
            focused_included: true,
            ..state
        };
        assert!(visible_items(&included).contains(&SourcesMenuItem::ToggleInclude));
    }

    #[test]
    fn no_focused_note_hides_every_per_note_item() {
        let items = visible_items(&SourcesMenuState::default());
        assert_eq!(
            items,
            vec![
                SourcesMenuItem::ConfirmSourceList,
                SourcesMenuItem::Refresh,
                SourcesMenuItem::AddDate,
                SourcesMenuItem::Help,
                SourcesMenuItem::BackToStart,
            ]
        );
    }

    #[test]
    fn show_hidden_and_load_more_follow_their_conditions() {
        let state = SourcesMenuState {
            hidden_count: 2,
            has_more: true,
            ..base()
        };
        let items = visible_items(&state);
        assert!(items.contains(&SourcesMenuItem::ShowHidden));
        assert!(items.contains(&SourcesMenuItem::LoadMore));
        assert_eq!(
            label(SourcesMenuItem::ShowHidden, &state),
            "Show hidden notes (2)"
        );
    }

    #[test]
    fn confirm_reads_continue_without_notes_when_nothing_is_included() {
        let none = base();
        assert_eq!(
            label(SourcesMenuItem::ConfirmSourceList, &none),
            "Continue without notes"
        );
        let some = SourcesMenuState {
            included_count: 1,
            ..none
        };
        assert_eq!(
            label(SourcesMenuItem::ConfirmSourceList, &some),
            "Confirm Source List"
        );
        let included = SourcesMenuState {
            focused_included: true,
            ..none
        };
        assert_eq!(
            label(SourcesMenuItem::ToggleInclude, &included),
            "Exclude this note"
        );
    }

    #[test]
    fn over_limit_confirmation_explains_why_it_is_disabled() {
        let state = SourcesMenuState {
            included_count: diary_control_contract::MAX_SELECTED_SOURCES + 1,
            ..base()
        };
        let text = label(SourcesMenuItem::ConfirmSourceList, &state);
        assert!(text.contains("disabled"));
        assert!(text.contains("64-note limit"));
    }

    #[test]
    fn shortcuts_resolve_to_the_same_item_as_their_menu_row() {
        for shortcut in SourceShortcut::ALL {
            let action = shortcut_action(shortcut);
            assert_eq!(
                item_action(action_item(action)),
                match action {
                    SourcesAction::Include | SourcesAction::Exclude => SourcesAction::ToggleInclude,
                    other => other,
                }
            );
        }
    }

    #[test]
    fn no_shortcut_reaches_accept_or_confirm() {
        for shortcut in SourceShortcut::ALL {
            let action = shortcut_action(shortcut);
            assert_ne!(action, SourcesAction::Accept);
            assert_ne!(action, SourcesAction::ConfirmSourceList);
        }
    }
}
