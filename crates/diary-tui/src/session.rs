//! Connection authority for the terminal session. Events never mutate a
//! workflow directly: they disable mutations and request canonical reads.

use diary_control_contract::EventDto;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryCause {
    EventReset,
    UnknownEvent,
    DecodeFailure,
    ResultOverflow,
    TransportLoss,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionAction {
    None,
    Reload {
        workflow_id: Option<String>,
        job_id: Option<String>,
        cause: Option<RecoveryCause>,
    },
    Reattach,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionAuthority {
    instance_id: String,
    mutations_enabled: bool,
    workflow_list_reload_pending: bool,
    workflow_reload_pending: bool,
    job_reload_pending: bool,
}

impl SessionAuthority {
    #[must_use]
    pub fn connected(instance_id: String) -> Self {
        Self {
            instance_id,
            mutations_enabled: true,
            workflow_list_reload_pending: false,
            workflow_reload_pending: false,
            job_reload_pending: false,
        }
    }

    #[must_use]
    pub const fn mutations_enabled(&self) -> bool {
        self.mutations_enabled
    }

    pub fn observe_event(&mut self, event: &EventDto) -> SessionAction {
        if event.instance_id != self.instance_id {
            self.disable_for_reload(false, false, false);
            return SessionAction::Reattach;
        }
        match event.event_type.as_str() {
            "workflow.changed" | "entry.changed" => {
                self.disable_for_reload(
                    event.workflow_id.is_none(),
                    event.workflow_id.is_some(),
                    false,
                );
                SessionAction::Reload {
                    workflow_id: event.workflow_id.clone(),
                    job_id: None,
                    cause: None,
                }
            }
            "job.changed" => {
                self.disable_for_reload(
                    event.workflow_id.is_none() && event.job_id.is_none(),
                    event.workflow_id.is_some(),
                    event.job_id.is_some(),
                );
                SessionAction::Reload {
                    workflow_id: event.workflow_id.clone(),
                    job_id: event.job_id.clone(),
                    cause: None,
                }
            }
            "reset" => self.recover(
                event.workflow_id.clone(),
                event.job_id.clone(),
                RecoveryCause::EventReset,
            ),
            _ => self.recover(
                event.workflow_id.clone(),
                event.job_id.clone(),
                RecoveryCause::UnknownEvent,
            ),
        }
    }

    pub fn recover(
        &mut self,
        workflow_id: Option<String>,
        job_id: Option<String>,
        cause: RecoveryCause,
    ) -> SessionAction {
        let workflow_list = workflow_id.is_none() && job_id.is_none();
        self.disable_for_reload(workflow_list, workflow_id.is_some(), job_id.is_some());
        SessionAction::Reload {
            workflow_id,
            job_id,
            cause: Some(cause),
        }
    }

    pub fn canonical_workflow_list_loaded(&mut self) {
        self.workflow_list_reload_pending = false;
        self.update_mutation_gate();
    }

    pub fn canonical_workflow_loaded(&mut self) {
        self.workflow_reload_pending = false;
        self.update_mutation_gate();
    }

    pub fn canonical_job_loaded(&mut self) {
        self.job_reload_pending = false;
        self.update_mutation_gate();
    }

    fn disable_for_reload(&mut self, workflow_list: bool, workflow: bool, job: bool) {
        self.mutations_enabled = false;
        self.workflow_list_reload_pending |= workflow_list;
        self.workflow_reload_pending |= workflow;
        self.job_reload_pending |= job;
    }

    fn update_mutation_gate(&mut self) {
        self.mutations_enabled = !self.workflow_list_reload_pending
            && !self.workflow_reload_pending
            && !self.job_reload_pending;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(event_type: &str) -> EventDto {
        EventDto {
            id: 3,
            instance_id: "instance_a".to_owned(),
            event_type: event_type.to_owned(),
            workflow_id: Some("wf_20261029".to_owned()),
            revision: Some(4),
            job_id: None,
            progress_percent: None,
        }
    }

    #[test]
    fn workflow_event_disables_mutations_until_canonical_reload() {
        let mut authority = SessionAuthority::connected("instance_a".to_owned());
        assert!(matches!(
            authority.observe_event(&event("workflow.changed")),
            SessionAction::Reload { .. }
        ));
        assert!(!authority.mutations_enabled());
        authority.canonical_workflow_loaded();
        assert!(authority.mutations_enabled());
    }

    #[test]
    fn job_event_requires_both_job_and_workflow_reads() {
        let mut authority = SessionAuthority::connected("instance_a".to_owned());
        let mut changed = event("job.changed");
        changed.job_id = Some("job_1".to_owned());
        let action = authority.observe_event(&changed);
        assert!(matches!(
            action,
            SessionAction::Reload {
                workflow_id: Some(_),
                job_id: Some(_),
                ..
            }
        ));
        authority.canonical_workflow_loaded();
        assert!(!authority.mutations_enabled());
        authority.canonical_job_loaded();
        assert!(authority.mutations_enabled());
    }

    #[test]
    fn unknown_and_reset_events_fail_closed() {
        for event_type in ["future.event", "reset"] {
            let mut authority = SessionAuthority::connected("instance_a".to_owned());
            let action = authority.observe_event(&event(event_type));
            assert!(matches!(
                action,
                SessionAction::Reload { cause: Some(_), .. }
            ));
            assert!(!authority.mutations_enabled());
        }
    }

    #[test]
    fn a_different_instance_requires_full_reattachment() {
        let mut authority = SessionAuthority::connected("instance_a".to_owned());
        let mut changed = event("workflow.changed");
        changed.instance_id = "instance_b".to_owned();
        assert_eq!(authority.observe_event(&changed), SessionAction::Reattach);
        assert!(!authority.mutations_enabled());
    }

    #[test]
    fn result_overflow_disables_mutations_until_explicit_recovery_reads_finish() {
        let mut authority = SessionAuthority::connected("instance_a".to_owned());
        let action = authority.recover(
            Some("wf_20261029".to_owned()),
            Some("job_1".to_owned()),
            RecoveryCause::ResultOverflow,
        );
        assert!(matches!(
            action,
            SessionAction::Reload {
                cause: Some(RecoveryCause::ResultOverflow),
                ..
            }
        ));
        authority.canonical_job_loaded();
        assert!(!authority.mutations_enabled());
        authority.canonical_workflow_loaded();
        assert!(authority.mutations_enabled());
    }
}
