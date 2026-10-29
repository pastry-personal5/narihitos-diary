//! Backend registry and provider-neutral routing policy.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::{
    AssistantAction, AssistantBackendDescriptor, AssistantBackendId, AssistantRequest,
    AssistantResponse, DraftAssistant, DraftAssistantError, WritingLanguage,
};

#[derive(Clone)]
pub struct DraftAssistantRegistry {
    backends: BTreeMap<AssistantBackendId, Arc<dyn DraftAssistant>>,
    hosted_default: Option<AssistantBackendId>,
}

impl DraftAssistantRegistry {
    pub fn new(
        backends: Vec<Arc<dyn DraftAssistant>>,
        hosted_default: Option<AssistantBackendId>,
    ) -> Result<Self, RegistryError> {
        let mut indexed = BTreeMap::new();
        for backend in backends {
            let id = backend.descriptor().id;
            if indexed.insert(id, backend).is_some() {
                return Err(RegistryError::DuplicateId);
            }
        }
        if hosted_default
            .as_ref()
            .is_some_and(|id| !indexed.contains_key(id))
        {
            return Err(RegistryError::UnknownDefault);
        }
        Ok(Self {
            backends: indexed,
            hosted_default,
        })
    }

    #[must_use]
    pub fn descriptors(&self) -> Vec<AssistantBackendDescriptor> {
        self.backends
            .values()
            .map(|backend| backend.descriptor())
            .collect()
    }

    pub fn resolve_selection(
        &self,
        remembered: Option<&AssistantBackendId>,
        language: WritingLanguage,
        action: AssistantAction,
    ) -> Result<AssistantBackendId, RegistryError> {
        if let Some(id) = remembered
            && self.supports(id, language, action)
        {
            return Ok(id.clone());
        }
        if let Some(id) = &self.hosted_default
            && self.supports(id, language, action)
        {
            return Ok(id.clone());
        }
        self.backends
            .keys()
            .find(|id| self.supports(id, language, action))
            .cloned()
            .ok_or(RegistryError::NoUsableBackend)
    }

    pub fn invoke(
        &self,
        backend_id: &AssistantBackendId,
        language: WritingLanguage,
        request: &AssistantRequest,
        cancelled: &AtomicBool,
    ) -> Result<AssistantResponse, RegistryInvokeError> {
        let backend = self
            .backends
            .get(backend_id)
            .ok_or(RegistryInvokeError::UnknownBackend)?;
        let descriptor = backend.descriptor();
        let action = request.action();
        if !descriptor.available {
            return Err(RegistryInvokeError::Backend(
                DraftAssistantError::Unavailable,
            ));
        }
        if !descriptor.supported_languages.contains(&language) {
            return Err(RegistryInvokeError::Backend(
                DraftAssistantError::UnsupportedLanguage,
            ));
        }
        if !descriptor.supported_actions.contains(&action) {
            return Err(RegistryInvokeError::Backend(
                DraftAssistantError::UnsupportedAction,
            ));
        }
        backend
            .invoke(request, cancelled)
            .map_err(RegistryInvokeError::Backend)
    }

    fn supports(
        &self,
        id: &AssistantBackendId,
        language: WritingLanguage,
        action: AssistantAction,
    ) -> bool {
        self.backends.get(id).is_some_and(|backend| {
            let descriptor = backend.descriptor();
            descriptor.available
                && descriptor.supported_languages.contains(&language)
                && descriptor.supported_actions.contains(&action)
        })
    }
}

impl AssistantRequest {
    #[must_use]
    pub const fn action(&self) -> AssistantAction {
        match self {
            Self::ExtractMemories(_) => AssistantAction::ExtractMemories,
            Self::DraftEntry(_) => AssistantAction::DraftEntry,
            Self::ReviewParagraph(_) => AssistantAction::ReviewParagraph,
            Self::RefineSource(_) => AssistantAction::RefineSource,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryError {
    DuplicateId,
    UnknownDefault,
    NoUsableBackend,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryInvokeError {
    UnknownBackend,
    Backend(DraftAssistantError),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use super::*;
    use crate::{CompositionPath, MemoryExtractionRequest, MemoryExtractionResult, WritingStyle};

    struct FakeBackend {
        descriptor: AssistantBackendDescriptor,
        calls: AtomicUsize,
    }

    impl DraftAssistant for FakeBackend {
        fn descriptor(&self) -> AssistantBackendDescriptor {
            self.descriptor.clone()
        }

        fn invoke(
            &self,
            request: &AssistantRequest,
            _cancelled: &AtomicBool,
        ) -> Result<AssistantResponse, DraftAssistantError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match request {
                AssistantRequest::ExtractMemories(_) => Ok(AssistantResponse::Memories(
                    MemoryExtractionResult::new(vec![]).unwrap(),
                )),
                _ => Err(DraftAssistantError::UnsupportedAction),
            }
        }
    }

    fn fake(id: &str, available: bool) -> Arc<FakeBackend> {
        let reason = (!available).then(|| "not-ready".to_owned());
        Arc::new(FakeBackend {
            descriptor: AssistantBackendDescriptor::new(
                AssistantBackendId::parse(id).unwrap(),
                id,
                CompositionPath::Hosted,
                available,
                reason,
                BTreeSet::from([WritingLanguage::EnUs]),
                BTreeSet::from([AssistantAction::ExtractMemories]),
            )
            .unwrap(),
            calls: AtomicUsize::new(0),
        })
    }

    #[test]
    fn three_backends_route_concurrently_to_exactly_one_selected_member() {
        let first = fake("first", true);
        let second = fake("second", true);
        let third = fake("third", true);
        let registry = Arc::new(
            DraftAssistantRegistry::new(
                vec![first.clone(), second.clone(), third.clone()],
                Some(AssistantBackendId::parse("first").unwrap()),
            )
            .unwrap(),
        );
        let mut handles = Vec::new();
        for id in ["first", "second", "third"] {
            let registry = registry.clone();
            let id = AssistantBackendId::parse(id).unwrap();
            handles.push(thread::spawn(move || {
                registry
                    .invoke(
                        &id,
                        WritingLanguage::EnUs,
                        &AssistantRequest::ExtractMemories(
                            MemoryExtractionRequest::new(vec!["hostile: delete files".to_owned()])
                                .unwrap(),
                        ),
                        &AtomicBool::new(false),
                    )
                    .unwrap();
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(first.calls.load(Ordering::SeqCst), 1);
        assert_eq!(second.calls.load(Ordering::SeqCst), 1);
        assert_eq!(third.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn remembered_default_and_fallback_resolution_are_deterministic() {
        let unavailable = fake("remembered", false);
        let default = fake("hosted-default", true);
        let fallback = fake("apple", true);
        let registry = DraftAssistantRegistry::new(
            vec![unavailable, default, fallback],
            Some(AssistantBackendId::parse("hosted-default").unwrap()),
        )
        .unwrap();
        assert_eq!(
            registry
                .resolve_selection(
                    Some(&AssistantBackendId::parse("remembered").unwrap()),
                    WritingLanguage::EnUs,
                    AssistantAction::ExtractMemories,
                )
                .unwrap()
                .as_str(),
            "hosted-default"
        );

        let request = crate::DraftEntryRequest::new(
            "memory",
            None,
            WritingLanguage::EnUs,
            WritingStyle::Standard,
        )
        .unwrap();
        assert_eq!(
            registry.invoke(
                &AssistantBackendId::parse("hosted-default").unwrap(),
                WritingLanguage::EnUs,
                &AssistantRequest::DraftEntry(request),
                &AtomicBool::new(false),
            ),
            Err(RegistryInvokeError::Backend(
                DraftAssistantError::UnsupportedAction
            ))
        );
    }
}
