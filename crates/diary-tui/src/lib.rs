//! Terminal-client boundary for Narihito's Diary.

#![forbid(unsafe_code)]

pub mod client;
pub mod command_line;
pub mod compose_view;
pub mod date_display;
pub mod editor;
pub mod ime;
pub mod layout;
pub mod main_pane;
pub mod resources;
pub mod review_view;
pub mod sanitize;
pub mod sidebar;
pub mod sources_view;
pub mod stage;
pub mod start_view;
pub mod status_line;
pub mod terminal;
pub mod work_status;
pub mod worker;
