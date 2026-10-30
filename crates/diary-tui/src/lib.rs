//! Terminal-client boundary for Narihito's Diary.

#![forbid(unsafe_code)]

pub mod amendment_view;
pub mod assistant_view;
pub mod client;
pub mod command_line;
pub mod command_line_view;
pub mod compose_view;
pub mod date_display;
pub mod display;
pub mod editor;
pub mod events;
pub mod existing_entry_view;
pub mod focus;
pub mod help_overlay;
pub mod ime;
pub mod layout;
pub mod main_pane;
pub mod readiness_view;
pub mod resources;
pub mod review_view;
pub mod sanitize;
pub mod screen_body;
pub mod session;
pub mod session_runtime;
pub mod sidebar;
pub mod source_shortcuts;
pub mod sources_menu;
pub mod sources_view;
pub mod stage;
pub mod stage_tip;
pub mod start_view;
pub mod status_line;
pub mod terminal;
pub mod theme;
pub mod widgets;
pub mod work_status;
pub mod worker;
