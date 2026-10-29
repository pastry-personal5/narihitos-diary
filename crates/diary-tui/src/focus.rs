//! Shared focus state for the Standard Layout. Focus is presentation-only and
//! never changes the server-owned workflow or resource inclusion.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusZone {
    Workflow,
    Resources,
    Main,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SidebarKind {
    Workflow,
    Resources,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusModel {
    pub zone: FocusZone,
    pub workflow_index: usize,
    pub resource_index: usize,
    pub main_control_index: usize,
}

impl Default for FocusModel {
    fn default() -> Self {
        Self {
            zone: FocusZone::Main,
            workflow_index: 0,
            resource_index: 0,
            main_control_index: 0,
        }
    }
}

impl FocusModel {
    pub fn next_zone(&mut self) {
        self.zone = match self.zone {
            FocusZone::Workflow => FocusZone::Resources,
            FocusZone::Resources => FocusZone::Main,
            FocusZone::Main => FocusZone::Workflow,
        };
    }

    pub fn previous_zone(&mut self) {
        self.zone = match self.zone {
            FocusZone::Workflow => FocusZone::Main,
            FocusZone::Resources => FocusZone::Workflow,
            FocusZone::Main => FocusZone::Resources,
        };
    }

    pub fn move_previous(&mut self, item_count: usize) {
        let index = self.active_index_mut();
        if item_count > 0 {
            *index = index.saturating_sub(1).min(item_count - 1);
        } else {
            *index = 0;
        }
    }

    pub fn move_next(&mut self, item_count: usize) {
        let index = self.active_index_mut();
        if item_count > 0 {
            *index = index.saturating_add(1).min(item_count - 1);
        } else {
            *index = 0;
        }
    }

    pub fn clamp(&mut self, workflow_count: usize, resource_count: usize, control_count: usize) {
        self.workflow_index = clamp_index(self.workflow_index, workflow_count);
        self.resource_index = clamp_index(self.resource_index, resource_count);
        self.main_control_index = clamp_index(self.main_control_index, control_count);
    }

    fn active_index_mut(&mut self) -> &mut usize {
        match self.zone {
            FocusZone::Workflow => &mut self.workflow_index,
            FocusZone::Resources => &mut self.resource_index,
            FocusZone::Main => &mut self.main_control_index,
        }
    }
}

fn clamp_index(index: usize, count: usize) -> usize {
    if count == 0 { 0 } else { index.min(count - 1) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_order_cycles_through_the_three_interactive_zones() {
        let mut focus = FocusModel::default();
        assert_eq!(focus.zone, FocusZone::Main);
        focus.next_zone();
        assert_eq!(focus.zone, FocusZone::Workflow);
        focus.next_zone();
        assert_eq!(focus.zone, FocusZone::Resources);
        focus.next_zone();
        assert_eq!(focus.zone, FocusZone::Main);
        focus.previous_zone();
        assert_eq!(focus.zone, FocusZone::Resources);
    }

    #[test]
    fn arrows_clamp_the_index_for_the_focused_zone() {
        let mut focus = FocusModel {
            zone: FocusZone::Resources,
            ..FocusModel::default()
        };
        focus.move_next(2);
        focus.move_next(2);
        assert_eq!(focus.resource_index, 1);
        focus.move_previous(2);
        focus.move_previous(2);
        assert_eq!(focus.resource_index, 0);
    }

    #[test]
    fn reload_clamps_each_index_without_changing_focus() {
        let mut focus = FocusModel {
            zone: FocusZone::Resources,
            workflow_index: 5,
            resource_index: 5,
            main_control_index: 5,
        };
        focus.clamp(2, 0, 3);
        assert_eq!(focus.workflow_index, 1);
        assert_eq!(focus.resource_index, 0);
        assert_eq!(focus.main_control_index, 2);
        assert_eq!(focus.zone, FocusZone::Resources);
    }
}
