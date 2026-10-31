use super::hierarchy::Insertion;
use super::projection::text_block;
use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize};

fn day(value: &str) -> DiaryDay {
    DiaryDay::parse(value).unwrap()
}

#[test]
fn weekday_is_fixed_across_leap_and_year_boundaries() {
    for (value, expected) in [
        ("2024-02-29", "2024-02-29 (THU)"),
        ("2025-12-31", "2025-12-31 (WED)"),
        ("2026-01-01", "2026-01-01 (THU)"),
        ("2026-10-30", "2026-10-30 (FRI)"),
    ] {
        assert_eq!(render_notion(&day(value), "").unwrap().heading, expected);
    }
}

#[test]
fn supported_markdown_projects_without_losing_text_or_annotations() {
    let markdown = concat!(
        "# Heading\n\n",
        "soft\nbreak and **bold** *italic* ~~strike~~ `code` ",
        "[link](https://example.invalid).  \nHard.\n\n",
        "- one\n  - nested\n\n1. first\n\n> quote\n\n```rust\nfn main() {}\n```\n",
    );
    let plan = render_notion(&day("2026-10-30"), markdown).unwrap();
    plan.validate().unwrap();
    assert_eq!(plan.blocks[0].kind, BlockKind::Heading2);
    assert!(
        plan.blocks
            .iter()
            .any(|block| block.kind == BlockKind::Heading1)
    );
    assert!(
        plan.blocks
            .iter()
            .any(|block| block.kind == BlockKind::BulletedListItem)
    );
    assert!(
        plan.blocks
            .iter()
            .any(|block| block.kind == BlockKind::NumberedListItem)
    );
    assert!(
        plan.blocks
            .iter()
            .any(|block| block.kind == BlockKind::Quote)
    );
    assert!(
        plan.blocks
            .iter()
            .any(|block| block.kind == BlockKind::Code)
    );
    let rich: Vec<_> = plan
        .blocks
        .iter()
        .flat_map(|block| &block.rich_text)
        .collect();
    assert!(rich.iter().any(|item| item.annotations.bold));
    assert!(rich.iter().any(|item| item.annotations.italic));
    assert!(rich.iter().any(|item| item.annotations.strikethrough));
    assert!(rich.iter().any(|item| item.annotations.code));
    assert!(rich.iter().any(|item| item.link.is_some()));
}

#[test]
fn unicode_splitting_uses_scalar_boundaries_and_preserves_all_text() {
    let text = "한🙂".repeat(1_501);
    let plan = render_notion(&day("2026-10-30"), &text).unwrap();
    assert_eq!(plan.blocks[1].display_text(), text);
    assert!(
        plan.blocks[1]
            .rich_text
            .iter()
            .all(|item| item.text.chars().count() <= MAX_RICH_TEXT_CHARS)
    );
}

#[test]
fn unsupported_construct_is_preserved_as_literal_markdown() {
    let markdown = "before\n\n![alt](https://example.invalid/image.png)\n\nafter";
    let plan = render_notion(&day("2026-10-30"), markdown).unwrap();
    assert_eq!(plan.blocks.len(), 2);
    assert_eq!(plan.blocks[1].display_text(), markdown);
}

#[test]
fn hierarchy_allows_unrelated_but_fails_closed() {
    let unrelated = PageChild {
        id: "x".into(),
        kind: PageKind::Other,
        title: Some("notes".into()),
        trashed: false,
        accessible: true,
    };
    assert_eq!(
        resolve_child(std::slice::from_ref(&unrelated), "2026", true),
        Ok(None)
    );
    let canonical = PageChild {
        id: "year".into(),
        kind: PageKind::ChildPage,
        title: Some("2026".into()),
        trashed: false,
        accessible: true,
    };
    assert_eq!(
        resolve_child(&[unrelated, canonical.clone()], "2026", true),
        Ok(Some(NotionPage {
            id: "year".into(),
            title: "2026".into()
        }))
    );
    assert_eq!(
        resolve_child(&[canonical.clone(), canonical], "2026", true),
        Err(HierarchyError::Duplicate)
    );
    assert_eq!(
        resolve_child(&[], "2026", false),
        Err(HierarchyError::IncompleteListing)
    );
    for (mut child, expected) in [
        (
            PageChild {
                kind: PageKind::Other,
                ..PageChild {
                    id: "wrong".into(),
                    kind: PageKind::ChildPage,
                    title: Some("2026".into()),
                    trashed: false,
                    accessible: true,
                }
            },
            HierarchyError::WrongKind,
        ),
        (
            PageChild {
                id: "trashed".into(),
                kind: PageKind::ChildPage,
                title: Some("2026".into()),
                trashed: true,
                accessible: true,
            },
            HierarchyError::Trashed,
        ),
        (
            PageChild {
                id: "hidden".into(),
                kind: PageKind::ChildPage,
                title: Some("2026".into()),
                trashed: false,
                accessible: false,
            },
            HierarchyError::Inaccessible,
        ),
    ] {
        child.title = Some("2026".into());
        assert_eq!(resolve_child(&[child], "2026", true), Err(expected));
    }
}

#[test]
fn same_day_legacy_and_wrong_weekday_headings_conflict() {
    let plan = render_notion(&day("2026-10-30"), "body").unwrap();
    assert_eq!(classify_same_day(&[], &plan), SameDayState::Absent);
    assert_eq!(
        classify_same_day(
            &[text_block(BlockKind::Heading2, plan.heading.clone())],
            &plan
        ),
        SameDayState::Canonical { index: 0 }
    );
    for heading in ["2026-10-30", "2026-10-30 (THU)"] {
        assert!(matches!(
            classify_same_day(&[text_block(BlockKind::Heading2, heading.into())], &plan),
            SameDayState::Ambiguous { .. }
        ));
    }
}

#[test]
fn retry_policy_is_bounded_and_long_wait_becomes_pending() {
    let mut policy = RetryPolicy::default();
    assert_eq!(
        policy.classify(Some(429), Some(Duration::from_secs(61)), false),
        RetryAction::Pending
    );
    let mut write = RetryPolicy::default();
    assert_eq!(
        write.classify(Some(503), None, true),
        RetryAction::InspectAmbiguousWrite
    );
    let mut exhausted = RetryPolicy::default();
    for _ in 0..4 {
        assert!(matches!(
            exhausted.classify(Some(503), None, false),
            RetryAction::RetryAfter(_)
        ));
    }
    assert_eq!(
        exhausted.classify(Some(503), None, false),
        RetryAction::Exhausted
    );
}

#[test]
fn opaque_pagination_cursor_is_encoded_without_interpreting_it() {
    assert_eq!(encode_query_component("a/b+c=?"), "a%2Fb%2Bc%3D%3F");
}

#[test]
fn retry_wait_observes_cancellation_without_sleeping_the_delay() {
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        wait_cancellable(Duration::from_secs(60), &cancelled),
        Err(RemoteError::Pending)
    );
}

#[derive(Default)]
struct FakeState {
    children: BTreeMap<String, Vec<PageChild>>,
    blocks: BTreeMap<String, Vec<RemoteBlock>>,
    next_id: u64,
    lose_next_append_reply: bool,
    /// `(append call number, committed)`: that append's reply is lost,
    /// and the blocks are either stored (`true`) or never reach Notion.
    lose_append_reply_on_call: Option<(usize, bool)>,
    append_anchors: Vec<Option<String>>,
    /// Appends that asked for the start of the parent's children.
    start_inserts: usize,
    list_blocks_calls: usize,
    lose_next_create_reply: bool,
    fail_list_blocks_on_call: Option<(usize, RemoteError)>,
    list_errors: BTreeMap<String, RemoteError>,
    creates: usize,
    trashed: Vec<String>,
    /// Trash calls numbered from 0 at or past this fail without effect.
    fail_trash_from: Option<usize>,
    trash_calls: usize,
}

#[derive(Default)]
struct FakeRemote(Mutex<FakeState>);

impl FakeRemote {
    fn lose_next_create_reply(&self) {
        self.0.lock().unwrap().lose_next_create_reply = true;
    }

    fn fail_list_blocks_on_call(&self, call: usize, error: RemoteError) {
        self.0.lock().unwrap().fail_list_blocks_on_call = Some((call, error));
    }

    fn fail_children_of(&self, parent: &str, error: RemoteError) {
        self.0
            .lock()
            .unwrap()
            .list_errors
            .insert(parent.to_owned(), error);
    }

    fn seed_children(&self, parent: &str, children: Vec<PageChild>) {
        self.0
            .lock()
            .unwrap()
            .children
            .insert(parent.to_owned(), children);
    }

    fn seed_blocks(&self, parent: &str, blocks: Vec<RemoteBlock>) {
        self.0
            .lock()
            .unwrap()
            .blocks
            .insert(parent.to_owned(), blocks);
    }

    fn lose_next_append_reply(&self) {
        self.0.lock().unwrap().lose_next_append_reply = true;
    }

    fn fail_trash_from(&self, call: Option<usize>) {
        let mut state = self.0.lock().unwrap();
        state.fail_trash_from = call;
        state.trash_calls = 0;
    }

    fn month_ids(&self, parent: &str) -> Vec<String> {
        self.0.lock().unwrap().blocks[parent]
            .iter()
            .map(|item| item.id.clone())
            .collect()
    }

    fn month_texts(&self, parent: &str) -> Vec<String> {
        self.0.lock().unwrap().blocks[parent]
            .iter()
            .map(|item| item.block.display_text())
            .collect()
    }

    fn lose_append_reply_on_call(&self, call: usize, committed: bool) {
        self.0.lock().unwrap().lose_append_reply_on_call = Some((call, committed));
    }
}

impl NotionRemote for FakeRemote {
    fn list_children(
        &self,
        parent_id: &str,
        _: &AtomicBool,
    ) -> Result<Vec<PageChild>, RemoteError> {
        let state = self.0.lock().unwrap();
        if let Some(error) = state.list_errors.get(parent_id) {
            return Err(*error);
        }
        Ok(state.children.get(parent_id).cloned().unwrap_or_default())
    }

    fn list_blocks(
        &self,
        parent_id: &str,
        _: &AtomicBool,
    ) -> Result<Vec<RemoteBlock>, RemoteError> {
        let mut state = self.0.lock().unwrap();
        state.list_blocks_calls += 1;
        if let Some((call, error)) = state.fail_list_blocks_on_call
            && call == state.list_blocks_calls
        {
            state.fail_list_blocks_on_call = None;
            return Err(error);
        }
        Ok(state.blocks.get(parent_id).cloned().unwrap_or_default())
    }

    fn create_page(
        &self,
        parent_id: &str,
        title: &str,
        _: &AtomicBool,
    ) -> Result<String, RemoteError> {
        let mut state = self.0.lock().unwrap();
        state.creates += 1;
        state.next_id += 1;
        let id = format!("page-{}", state.next_id);
        state
            .children
            .entry(parent_id.to_owned())
            .or_default()
            .push(PageChild {
                id: id.clone(),
                kind: PageKind::ChildPage,
                title: Some(title.to_owned()),
                trashed: false,
                accessible: true,
            });
        if state.lose_next_create_reply {
            state.lose_next_create_reply = false;
            return Err(RemoteError::Ambiguous);
        }
        Ok(id)
    }

    fn append_blocks(
        &self,
        parent_id: &str,
        blocks: &[NotionBlock],
        place: &Placement,
        _: &AtomicBool,
    ) -> Result<Vec<RemoteBlock>, RemoteError> {
        let mut state = self.0.lock().unwrap();
        state.append_anchors.push(match place {
            Placement::After(id) => Some(id.clone()),
            Placement::Start | Placement::End => None,
        });
        if *place == Placement::Start {
            state.start_inserts += 1;
        }
        let call = state.append_anchors.len();
        let lost = state
            .lose_append_reply_on_call
            .filter(|(lost_call, _)| *lost_call == call);
        if let Some((_, false)) = lost {
            state.lose_append_reply_on_call = None;
            return Err(RemoteError::Ambiguous);
        }
        let mut appended = Vec::new();
        for block in blocks {
            state.next_id += 1;
            appended.push(RemoteBlock {
                id: format!("block-{}", state.next_id),
                block: block.clone(),
            });
        }
        let existing = state.blocks.entry(parent_id.to_owned()).or_default();
        let insertion = match place {
            Placement::End => existing.len(),
            Placement::Start => 0,
            Placement::After(id) => existing
                .iter()
                .position(|item| &item.id == id)
                .map_or(existing.len(), |index| index + 1),
        };
        existing.splice(insertion..insertion, appended.clone());
        if lost.is_some() {
            state.lose_append_reply_on_call = None;
            return Err(RemoteError::Ambiguous);
        }
        if state.lose_next_append_reply {
            state.lose_next_append_reply = false;
            return Err(RemoteError::Ambiguous);
        }
        Ok(appended)
    }

    fn trash_block(&self, block_id: &str, _: &AtomicBool) -> Result<(), RemoteError> {
        let mut state = self.0.lock().unwrap();
        let call = state.trash_calls;
        state.trash_calls += 1;
        if state.fail_trash_from.is_some_and(|from| call >= from) {
            return Err(RemoteError::Ambiguous);
        }
        for blocks in state.blocks.values_mut() {
            blocks.retain(|item| item.id != block_id);
        }
        state.trashed.push(block_id.to_owned());
        Ok(())
    }
}

struct Published {
    client: NotionClient,
    remote: Arc<FakeRemote>,
    month: String,
    ids: Vec<String>,
    digest: Digest,
    day: DiaryDay,
}

/// Publishes `markdown` for 2026-10-30 with an unrelated section on each side.
fn published(markdown: &str) -> Published {
    let remote = Arc::new(FakeRemote::default());
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let diary_day = day("2026-10-30");
    let preflight = client.preflight(&diary_day, markdown).unwrap();
    let request = NotionRequest {
        day: diary_day.clone(),
        markdown: markdown.into(),
        expected_target_identity: preflight.target_identity,
        expected_payload_digest: preflight.payload_digest.clone(),
        expected_state_digest: preflight.observed_state_digest,
    };
    let WriteOutcome::Verified {
        receipt_id,
        returned_ids,
    } = client.write(&request, &AtomicBool::new(false))
    else {
        panic!("initial publication must verify");
    };
    // An unrelated section after the owned one, written by someone else.
    remote
        .append_blocks(
            &receipt_id,
            &[
                text_block(BlockKind::Heading2, "2026-10-31 (SAT)".into()),
                text_block(BlockKind::Paragraph, "Someone else's day.".into()),
            ],
            &Placement::End,
            &AtomicBool::new(false),
        )
        .unwrap();
    Published {
        client,
        remote,
        month: receipt_id,
        ids: returned_ids,
        digest: preflight.payload_digest,
        day: diary_day,
    }
}

impl Published {
    fn request(&self, markdown: &str) -> NotionReplaceRequest {
        NotionReplaceRequest {
            day: self.day.clone(),
            markdown: markdown.into(),
            expected_target_identity: ROOT.into(),
            expected_payload_digest: Digest::parse(
                render_notion(&self.day, markdown).unwrap().payload_digest,
            )
            .unwrap(),
            owned_month_id: self.month.clone(),
            owned_block_ids: self.ids.clone(),
            owned_payload_digest: self.digest.clone(),
        }
    }
}

#[test]
fn replace_swaps_only_the_owned_body_and_keeps_the_heading_and_neighbors() {
    let owned = published("Old one.\n\nOld two.");
    let before = owned.remote.month_ids(&owned.month);
    let request = owned.request("New only.");
    let WriteOutcome::Verified {
        receipt_id,
        returned_ids,
    } = owned
        .client
        .replace_section(&request, &AtomicBool::new(false))
    else {
        panic!("an untouched owned section must be replaced");
    };
    assert_eq!(receipt_id, owned.month);
    assert_eq!(returned_ids[0], owned.ids[0], "heading ID is kept");
    assert_eq!(returned_ids.len(), 2);
    assert_eq!(
        owned.remote.month_texts(&owned.month),
        vec![
            "2026-10-30 (FRI)",
            "New only.",
            "2026-10-31 (SAT)",
            "Someone else's day."
        ]
    );
    let after = owned.remote.month_ids(&owned.month);
    assert_eq!(&after[after.len() - 2..], &before[before.len() - 2..]);
    // A replay is a no-op: nothing is inserted or trashed again.
    let state = owned.remote.0.lock().unwrap().trashed.len();
    assert_eq!(state, 2);
    assert!(matches!(
        owned
            .client
            .replace_section(&request, &AtomicBool::new(false)),
        WriteOutcome::Verified { .. }
    ));
    assert_eq!(owned.remote.0.lock().unwrap().trashed.len(), 2);
}

#[test]
fn replace_refuses_edited_extended_or_missing_sections_without_writing() {
    for tamper in 0..3 {
        let owned = published("Old one.");
        {
            let mut state = owned.remote.0.lock().unwrap();
            let blocks = state.blocks.get_mut(&owned.month).unwrap();
            match tamper {
                0 => blocks[1].block = text_block(BlockKind::Paragraph, "Edited.".into()),
                1 => blocks.insert(
                    2,
                    RemoteBlock {
                        id: "extra".into(),
                        block: text_block(BlockKind::Paragraph, "Added by hand.".into()),
                    },
                ),
                _ => {
                    blocks.remove(1);
                }
            }
        }
        let appends = owned.remote.0.lock().unwrap().append_anchors.len();
        let request = owned.request("New.");
        assert_eq!(
            owned
                .client
                .replace_section(&request, &AtomicBool::new(false)),
            WriteOutcome::Conflict,
            "tamper {tamper}"
        );
        assert_eq!(
            owned.client.reconcile_replace(&request),
            InspectionOutcome::Conflict
        );
        let preflight = owned.client.preflight_replace(&request).unwrap();
        assert_eq!(preflight.preflight.status, NotionPreflightStatus::Existing);
        let state = owned.remote.0.lock().unwrap();
        assert_eq!(state.append_anchors.len(), appends, "tamper {tamper}");
        assert!(state.trashed.is_empty(), "tamper {tamper}");
    }
}

#[test]
fn every_interruption_boundary_keeps_content_and_resumes_without_replaying() {
    // (label, trash failures from call, lose insert reply on call (committed))
    type Case = (&'static str, Option<usize>, Option<(usize, bool)>);
    let cases: [Case; 6] = [
        ("insert never reaches notion", None, Some((3, false))),
        ("insert committed, reply lost", None, Some((3, true))),
        ("read-back done, first trash fails", Some(0), None),
        ("partway through trashing", Some(1), None),
        ("all trashed, nothing left to do", None, None),
        ("second chunk insert lost", None, Some((4, true))),
    ];
    for (label, fail_trash, lose_insert) in cases {
        let long = (0..120)
            .map(|n| format!("Paragraph {n}."))
            .collect::<Vec<_>>()
            .join("\n\n");
        let markdown = if label.starts_with("second chunk") {
            long.as_str()
        } else {
            "Old one.\n\nOld two."
        };
        let owned = published(markdown);
        let replacement = if label.starts_with("second chunk") {
            long.replace("Paragraph", "Rewritten")
        } else {
            "New one.\n\nNew two.\n\nNew three.".to_owned()
        };
        let request = owned.request(&replacement);
        owned.remote.fail_trash_from(fail_trash);
        // Appends so far: the initial publication plus the neighbor section.
        let base = owned.remote.0.lock().unwrap().append_anchors.len();
        if let Some((call, committed)) = lose_insert {
            let offset = if label.starts_with("second chunk") {
                2
            } else {
                1
            };
            owned
                .remote
                .lose_append_reply_on_call(base + offset, committed);
            let _ = call;
        }
        let first = owned
            .client
            .replace_section(&request, &AtomicBool::new(false));
        let interrupted = !matches!(first, WriteOutcome::Verified { .. });
        assert_eq!(
            interrupted,
            fail_trash.is_some() || lose_insert.is_some(),
            "{label}"
        );

        // No loss at the crash: until the old body is gone, it is still all there.
        let texts = owned.remote.month_texts(&owned.month);
        assert!(texts.contains(&"Someone else's day.".to_owned()), "{label}");
        assert!(texts.contains(&"2026-10-30 (FRI)".to_owned()), "{label}");

        // Recovery: the destination is healthy again; reconcile, then resume.
        owned.remote.fail_trash_from(None);
        let inserted_before = owned.remote.0.lock().unwrap().append_anchors.len();
        let mut outcome = owned.client.reconcile_replace(&request);
        if !matches!(outcome, InspectionOutcome::Matching { .. }) {
            assert!(
                matches!(
                    outcome,
                    InspectionOutcome::Absent | InspectionOutcome::Unavailable
                ),
                "{label}: {outcome:?}"
            );
            let WriteOutcome::Verified {
                receipt_id,
                returned_ids,
            } = owned
                .client
                .replace_section(&request, &AtomicBool::new(false))
            else {
                panic!("{label}: resume must finish");
            };
            outcome = InspectionOutcome::Matching {
                receipt_id,
                returned_ids,
            };
        }
        let InspectionOutcome::Matching { returned_ids, .. } = outcome else {
            unreachable!()
        };
        let final_texts = owned.remote.month_texts(&owned.month);
        let expected_body = render_notion(&owned.day, &replacement)
            .unwrap()
            .blocks
            .len()
            - 1;
        assert_eq!(returned_ids.len(), expected_body + 1, "{label}");
        assert_eq!(returned_ids[0], owned.ids[0], "{label}");
        assert_eq!(
            final_texts.len(),
            expected_body + 1 + 2,
            "{label}: {final_texts:?}"
        );
        assert_eq!(
            &final_texts[final_texts.len() - 2..],
            ["2026-10-31 (SAT)", "Someone else's day."]
        );
        let state = owned.remote.0.lock().unwrap();
        // Old body trashed exactly once each; nothing else was ever trashed.
        let mut trashed = state.trashed.clone();
        trashed.sort();
        let mut old_body = owned.ids[1..].to_vec();
        old_body.sort();
        assert_eq!(trashed, old_body, "{label}");
        // Committed inserts are never repeated: a recovery that finds the
        // body already inserted adds no append.
        if label.contains("committed") || label.contains("trash") || label.contains("nothing") {
            assert_eq!(state.append_anchors.len(), inserted_before, "{label}");
        }
    }
}

#[test]
fn lost_append_reply_is_inspected_and_never_appended_twice() {
    let root = "0123456789abcdef0123456789abcdef";
    let remote = Arc::new(FakeRemote::default());
    let client = NotionClient::scripted(remote.clone(), root);
    let diary_day = day("2026-10-30");
    let preflight = client.preflight(&diary_day, "body").unwrap();
    assert_eq!(preflight.status, NotionPreflightStatus::Ready);
    assert!(!preflight.year_exists);
    assert!(!preflight.month_exists);
    let request = NotionRequest {
        day: diary_day,
        markdown: "body".into(),
        expected_target_identity: preflight.target_identity,
        expected_payload_digest: preflight.payload_digest,
        expected_state_digest: preflight.observed_state_digest,
    };
    remote.lose_next_append_reply();
    assert_eq!(
        client.write(&request, &AtomicBool::new(false)),
        WriteOutcome::AmbiguousWrite
    );
    let matching = client.inspect(&request);
    let InspectionOutcome::Matching {
        ref returned_ids, ..
    } = matching
    else {
        panic!("lost response must reconcile to the exact committed section");
    };
    assert_eq!(returned_ids.len(), 2);
    assert_eq!(client.inspect(&request), matching);
}

#[test]
fn later_chunk_uses_last_read_back_verified_block_as_its_anchor() {
    let root = "0123456789abcdef0123456789abcdef";
    let remote = Arc::new(FakeRemote::default());
    let client = NotionClient::scripted(remote.clone(), root);
    let diary_day = day("2026-10-30");
    let markdown = (0..120)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let preflight = client.preflight(&diary_day, &markdown).unwrap();
    let request = NotionRequest {
        day: diary_day,
        markdown,
        expected_target_identity: preflight.target_identity,
        expected_payload_digest: preflight.payload_digest,
        expected_state_digest: preflight.observed_state_digest,
    };
    assert!(matches!(
        client.write(&request, &AtomicBool::new(false)),
        WriteOutcome::Verified { .. }
    ));
    let state = remote.0.lock().unwrap();
    assert_eq!(state.append_anchors.len(), 2);
    assert!(state.append_anchors[0].is_none());
    assert!(state.append_anchors[1].is_some());
    assert!(state.list_blocks_calls >= 4);
}

const ROOT: &str = "0123456789abcdef0123456789abcdef";

fn long_markdown() -> String {
    (0..120)
        .map(|index| format!("paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn request_for(client: &NotionClient, value: &str, markdown: &str) -> NotionRequest {
    let diary_day = day(value);
    let preflight = client.preflight(&diary_day, markdown).unwrap();
    NotionRequest {
        day: diary_day,
        markdown: markdown.into(),
        expected_target_identity: preflight.target_identity,
        expected_payload_digest: preflight.payload_digest,
        expected_state_digest: preflight.observed_state_digest,
    }
}

fn page(id: &str, title: &str) -> PageChild {
    PageChild {
        id: id.into(),
        kind: PageKind::ChildPage,
        title: Some(title.into()),
        trashed: false,
        accessible: true,
    }
}

fn remote_heading(id: &str, text: &str) -> RemoteBlock {
    RemoteBlock {
        id: id.into(),
        block: text_block(BlockKind::Heading2, text.into()),
    }
}

#[test]
fn lost_reply_on_a_later_chunk_that_committed_is_adopted_without_another_append() {
    let remote = Arc::new(FakeRemote::default());
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let request = request_for(&client, "2026-10-30", &long_markdown());
    remote.lose_append_reply_on_call(2, true);
    assert_eq!(
        client.write(&request, &AtomicBool::new(false)),
        WriteOutcome::AmbiguousWrite
    );
    assert_eq!(remote.0.lock().unwrap().append_anchors.len(), 2);
    let InspectionOutcome::Matching { returned_ids, .. } = client.inspect(&request) else {
        panic!("a committed later chunk must reconcile to the exact section");
    };
    assert_eq!(returned_ids.len(), 121);
    // Reconciling never appends; a new write sees the section and refuses.
    assert_eq!(
        client.write(&request, &AtomicBool::new(false)),
        WriteOutcome::Conflict
    );
    assert_eq!(remote.0.lock().unwrap().append_anchors.len(), 2);
}

#[test]
fn lost_reply_on_a_later_chunk_that_never_landed_is_a_conflict_not_a_silent_completion() {
    let remote = Arc::new(FakeRemote::default());
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let request = request_for(&client, "2026-10-30", &long_markdown());
    remote.lose_append_reply_on_call(2, false);
    assert_eq!(
        client.write(&request, &AtomicBool::new(false)),
        WriteOutcome::AmbiguousWrite
    );
    // Only the first chunk is present: inspection must not adopt it, and
    // nothing may append the missing remainder on its own.
    assert_eq!(client.inspect(&request), InspectionOutcome::Conflict);
    assert_eq!(remote.0.lock().unwrap().append_anchors.len(), 2);
    let blocks = remote
        .0
        .lock()
        .unwrap()
        .blocks
        .values()
        .map(Vec::len)
        .sum::<usize>();
    assert_eq!(blocks, 100);
}

#[derive(Clone, Copy)]
enum Reply {
    Ok,
    Status(u16, Option<&'static str>),
    Drop,
}

struct Stub {
    base_url: String,
    requests: Arc<AtomicUsize>,
    waits: Arc<Mutex<Vec<Duration>>>,
}

impl Stub {
    fn serving(script: Vec<Reply>) -> Self {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!(
            "http://127.0.0.1:{}/v1",
            listener.local_addr().unwrap().port()
        );
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        std::thread::spawn(move || {
            for reply in script {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut received = Vec::new();
                let mut chunk = [0_u8; 1024];
                while !received.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => received.extend_from_slice(&chunk[..count]),
                    }
                }
                let head = String::from_utf8_lossy(&received).to_ascii_lowercase();
                let length = head
                    .split("content-length:")
                    .nth(1)
                    .and_then(|rest| rest.split("\r\n").next())
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let body_start = received
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map_or(received.len(), |index| index + 4);
                let mut have = received.len().saturating_sub(body_start);
                while have < length {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => have += count,
                    }
                }
                counter.fetch_add(1, Ordering::SeqCst);
                let (code, retry_after, body) = match reply {
                    Reply::Drop => continue,
                    Reply::Ok => (
                        200,
                        None,
                        r#"{"results":[],"has_more":false,"next_cursor":null}"#,
                    ),
                    Reply::Status(code, retry_after) => (code, retry_after, "{}"),
                };
                let retry_after = retry_after
                    .map(|value| format!("Retry-After: {value}\r\n"))
                    .unwrap_or_default();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\n{retry_after}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self {
            base_url,
            requests,
            waits: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn remote(&self) -> HttpRemote {
        let waits = Arc::clone(&self.waits);
        HttpRemote::new("secret".to_owned()).unwrap().with_stub(
            self.base_url.clone(),
            Arc::new(move |delay, _| {
                waits.lock().unwrap().push(delay);
                Ok(())
            }),
        )
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    fn waits(&self) -> Vec<Duration> {
        self.waits.lock().unwrap().clone()
    }
}

fn read(remote: &HttpRemote) -> Result<serde_json::Value, RemoteError> {
    remote.request(
        "GET",
        "/blocks/x/children",
        None,
        false,
        &AtomicBool::new(false),
    )
}

fn write(remote: &HttpRemote) -> Result<serde_json::Value, RemoteError> {
    remote.request(
        "PATCH",
        "/blocks/x/children",
        Some(&serde_json::json!({})),
        true,
        &AtomicBool::new(false),
    )
}

fn secs(values: &[u64]) -> Vec<Duration> {
    values
        .iter()
        .map(|value| Duration::from_secs(*value))
        .collect()
}

#[test]
fn rate_limits_honor_retry_after_up_to_sixty_seconds_then_become_pending() {
    for (status, header, expected) in [
        (429, Some("2"), secs(&[2])),
        (529, Some("2"), secs(&[2])),
        (429, Some("60"), secs(&[60])),
        (429, Some("0"), secs(&[0])),
        // No valid header falls back to the exponential schedule.
        (429, None, secs(&[1])),
        (529, None, secs(&[1])),
        (429, Some("soon"), secs(&[1])),
    ] {
        let stub = Stub::serving(vec![Reply::Status(status, header), Reply::Ok]);
        assert!(read(&stub.remote()).is_ok(), "{status} {header:?}");
        assert_eq!(stub.requests(), 2, "{status} {header:?}");
        assert_eq!(stub.waits(), expected, "{status} {header:?}");
    }
    for status in [429, 529] {
        let stub = Stub::serving(vec![Reply::Status(status, Some("61"))]);
        assert_eq!(read(&stub.remote()), Err(RemoteError::Pending));
        assert_eq!(stub.requests(), 1);
        assert!(stub.waits().is_empty(), "a long wait must not be slept");
    }
}

#[test]
fn repeated_rate_limits_back_off_exponentially_within_the_attempt_bound() {
    let stub = Stub::serving(vec![
        Reply::Status(429, None),
        Reply::Status(529, None),
        Reply::Status(429, None),
        Reply::Ok,
    ]);
    assert!(read(&stub.remote()).is_ok());
    assert_eq!(stub.waits(), secs(&[1, 2, 4]));
}

#[test]
fn idempotent_reads_retry_5xx_and_transport_errors_and_stop_at_five_attempts() {
    for status in [500, 502, 503, 504] {
        let stub = Stub::serving(vec![Reply::Status(status, None), Reply::Ok]);
        assert!(read(&stub.remote()).is_ok(), "{status}");
        assert_eq!(stub.requests(), 2);
        assert_eq!(stub.waits(), secs(&[1]));
    }
    let stub = Stub::serving(vec![Reply::Drop, Reply::Ok]);
    assert!(read(&stub.remote()).is_ok());
    assert_eq!(stub.requests(), 2);
    assert_eq!(stub.waits(), secs(&[1]));

    let exhausted = Stub::serving(vec![Reply::Status(503, None); 5]);
    assert_eq!(read(&exhausted.remote()), Err(RemoteError::Pending));
    assert_eq!(exhausted.requests(), 5, "five total attempts, no sixth");
    assert_eq!(exhausted.waits(), secs(&[1, 2, 4, 8]));
}

#[test]
fn writes_are_never_blindly_retried_after_5xx_or_a_lost_response() {
    for status in [500, 502, 503, 504] {
        let stub = Stub::serving(vec![Reply::Status(status, None), Reply::Ok]);
        assert_eq!(
            write(&stub.remote()),
            Err(RemoteError::Ambiguous),
            "{status}"
        );
        assert_eq!(stub.requests(), 1, "{status}");
        assert!(stub.waits().is_empty());
    }
    let stub = Stub::serving(vec![Reply::Drop, Reply::Ok]);
    assert_eq!(write(&stub.remote()), Err(RemoteError::Ambiguous));
    assert_eq!(stub.requests(), 1);
    // A rate limit means the write was refused, so it may be retried.
    let limited = Stub::serving(vec![Reply::Status(429, Some("1")), Reply::Ok]);
    assert!(write(&limited.remote()).is_ok());
    assert_eq!(limited.requests(), 2);
}

#[test]
fn auth_permission_and_validation_errors_are_not_retried() {
    for (status, expected) in [
        (401, RemoteError::Blocked(SetupReason::CredentialInvalid)),
        (403, RemoteError::Blocked(SetupReason::PermissionDenied)),
        (404, RemoteError::Blocked(SetupReason::RootUnverifiable)),
        (400, RemoteError::Blocked(SetupReason::ResponseInvalid)),
        (409, RemoteError::Invalid),
        (418, RemoteError::Blocked(SetupReason::RemoteUnavailable)),
    ] {
        for is_write in [false, true] {
            let stub = Stub::serving(vec![Reply::Status(status, None), Reply::Ok]);
            let remote = stub.remote();
            let result = if is_write {
                write(&remote)
            } else {
                read(&remote)
            };
            assert_eq!(result, Err(expected), "{status} write={is_write}");
            assert_eq!(stub.requests(), 1, "{status} write={is_write}");
            assert!(stub.waits().is_empty());
        }
    }
}

#[test]
fn cancellation_stops_before_a_request_and_during_a_retry_wait() {
    let stub = Stub::serving(Vec::new());
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        stub.remote()
            .request("GET", "/blocks/x/children", None, false, &cancelled),
        Err(RemoteError::Pending)
    );
    assert_eq!(stub.requests(), 0);

    let stub = Stub::serving(vec![Reply::Status(503, None), Reply::Ok]);
    let remote = HttpRemote::new("secret".to_owned()).unwrap().with_stub(
        stub.base_url.clone(),
        Arc::new(|_, cancelled| {
            cancelled.store(true, Ordering::Release);
            Err(RemoteError::Pending)
        }),
    );
    assert_eq!(read(&remote), Err(RemoteError::Pending));
    assert_eq!(stub.requests(), 1, "no request after the wait is cancelled");
}

#[test]
fn a_job_issuing_back_to_back_requests_cannot_starve_a_waiting_job() {
    const BURST: usize = 60;
    let stub = Stub::serving(vec![Reply::Ok; BURST + 1]);
    let busy = Arc::new(stub.remote());
    let waiting = Arc::new(stub.remote());
    let finished = Arc::new(AtomicUsize::new(0));
    let waiter_saw = Arc::new(AtomicUsize::new(usize::MAX));

    let burst = {
        let (busy, finished) = (Arc::clone(&busy), Arc::clone(&finished));
        std::thread::spawn(move || {
            for _ in 0..BURST {
                read(&busy).unwrap();
                finished.fetch_add(1, Ordering::SeqCst);
            }
        })
    };
    while finished.load(Ordering::SeqCst) < 3 {
        std::thread::sleep(Duration::from_millis(1));
    }
    let started_after = finished.load(Ordering::SeqCst);
    let waiter = {
        let (waiting, finished, saw) = (
            Arc::clone(&waiting),
            Arc::clone(&finished),
            Arc::clone(&waiter_saw),
        );
        std::thread::spawn(move || {
            read(&waiting).unwrap();
            saw.store(finished.load(Ordering::SeqCst), Ordering::SeqCst);
        })
    };
    burst.join().unwrap();
    waiter.join().unwrap();
    let served_after = waiter_saw.load(Ordering::SeqCst) - started_after;
    assert!(
        served_after <= 3,
        "the waiting job was served only after {served_after} more requests from the busy job"
    );
}

#[test]
fn a_cancelled_waiter_leaves_the_line_without_blocking_those_behind_it() {
    let queue = Arc::new(RequestQueue {
        state: Mutex::new(QueueState::default()),
        turn_changed: std::sync::Condvar::new(),
    });
    let first = queue.enter(&AtomicBool::new(false)).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_waiter = {
        let (queue, cancelled) = (Arc::clone(&queue), Arc::clone(&cancelled));
        std::thread::spawn(move || queue.enter(&cancelled).is_err())
    };
    std::thread::sleep(Duration::from_millis(30));
    let patient_waiter = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || queue.enter(&AtomicBool::new(false)).is_ok())
    };
    std::thread::sleep(Duration::from_millis(30));
    cancelled.store(true, Ordering::Release);
    assert!(
        cancelled_waiter.join().unwrap(),
        "cancellation ends the wait"
    );
    drop(first);
    assert!(
        patient_waiter.join().unwrap(),
        "the next waiter still gets its turn"
    );
}

#[test]
fn hierarchy_failures_fail_closed_at_every_level() {
    type Breaker = fn(&mut PageChild);
    let broken: [(&str, Breaker); 4] = [
        ("wrong kind", |child| child.kind = PageKind::Other),
        ("trashed", |child| child.trashed = true),
        ("inaccessible", |child| child.accessible = false),
        ("duplicate", |_| {}),
    ];
    for level in ["year", "month"] {
        for (label, break_child) in broken {
            let remote = Arc::new(FakeRemote::default());
            let client = NotionClient::scripted(remote.clone(), ROOT);
            let mut bad = page("bad", if level == "year" { "2026" } else { "2026-10" });
            break_child(&mut bad);
            let mut listing = vec![bad.clone()];
            if label == "duplicate" {
                listing.push(bad);
            }
            if level == "year" {
                remote.seed_children(ROOT, listing);
            } else {
                remote.seed_children(ROOT, vec![page("year", "2026")]);
                remote.seed_children("year", listing);
            }
            assert_eq!(
                client.preflight(&day("2026-10-30"), "body").unwrap_err(),
                "notion-hierarchy-ambiguous",
                "{level} {label}"
            );
            let request = NotionRequest {
                day: day("2026-10-30"),
                markdown: "body".into(),
                expected_target_identity: ROOT.into(),
                expected_payload_digest: Digest::parse(
                    render_notion(&day("2026-10-30"), "body")
                        .unwrap()
                        .payload_digest,
                )
                .unwrap(),
                expected_state_digest: Digest::of_bytes(b"stale"),
            };
            assert_eq!(
                client.write(&request, &AtomicBool::new(false)),
                WriteOutcome::Unavailable,
                "{level} {label}"
            );
            assert_eq!(remote.0.lock().unwrap().creates, 0, "{level} {label}");
        }
    }
}

#[test]
fn remote_errors_map_to_content_free_reasons_at_every_level() {
    for (error, reason) in [
        (
            RemoteError::Blocked(SetupReason::CredentialInvalid),
            "notion-credential-invalid",
        ),
        (
            RemoteError::Blocked(SetupReason::PermissionDenied),
            "notion-permission-denied",
        ),
        (
            RemoteError::Blocked(SetupReason::RootUnverifiable),
            "notion-root-unverifiable",
        ),
        (
            RemoteError::Blocked(SetupReason::RemoteUnavailable),
            "notion-remote-unavailable",
        ),
        (RemoteError::Pending, "notion-retry-deferred"),
        (RemoteError::Invalid, "notion-response-invalid"),
    ] {
        for parent in [ROOT, "year"] {
            let remote = Arc::new(FakeRemote::default());
            remote.seed_children(ROOT, vec![page("year", "2026")]);
            remote.fail_children_of(parent, error);
            let client = NotionClient::scripted(remote, ROOT);
            assert_eq!(
                client.preflight(&day("2026-10-30"), "body").unwrap_err(),
                reason
            );
        }
    }
}

#[test]
fn unrelated_siblings_do_not_block_publication() {
    let remote = Arc::new(FakeRemote::default());
    remote.seed_children(ROOT, vec![page("misc", "Notes"), page("year", "2026")]);
    remote.seed_children(
        "year",
        vec![page("other", "Trips"), page("month", "2026-10")],
    );
    remote.seed_blocks(
        "month",
        vec![
            remote_heading("old", "2026-10-29 (THU)"),
            remote_heading("other", "scratch"),
        ],
    );
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let request = request_for(&client, "2026-10-30", "body");
    assert!(matches!(
        client.write(&request, &AtomicBool::new(false)),
        WriteOutcome::Verified { .. }
    ));
    let state = remote.0.lock().unwrap();
    assert_eq!(state.creates, 0);
    assert_eq!(state.blocks["month"].len(), 4);
    assert_eq!(state.blocks["month"][0].id, "old");
}

#[test]
fn lost_year_or_month_create_reply_is_inspected_before_any_further_write() {
    for lost_on in [1, 2] {
        let remote = Arc::new(FakeRemote::default());
        let client = NotionClient::scripted(remote.clone(), ROOT);
        if lost_on == 2 {
            // The year already exists; only the month reply is lost.
            remote.seed_children(ROOT, vec![page("year", "2026")]);
        }
        let request = request_for(&client, "2026-10-30", "body");
        remote.lose_next_create_reply();
        assert_eq!(
            client.write(&request, &AtomicBool::new(false)),
            WriteOutcome::AmbiguousWrite,
            "lost create #{lost_on}"
        );
        let creates_after_loss = remote.0.lock().unwrap().creates;
        assert_eq!(creates_after_loss, 1);
        assert!(remote.0.lock().unwrap().blocks.is_empty());
        // The committed page is adopted by a fresh inspection, never recreated.
        let retry = request_for(&client, "2026-10-30", "body");
        assert!(matches!(
            client.write(&retry, &AtomicBool::new(false)),
            WriteOutcome::Verified { .. }
        ));
        let state = remote.0.lock().unwrap();
        let pages: usize = state.children.values().map(Vec::len).sum();
        assert_eq!(pages, 2, "exactly one year and one month page");
    }
}

#[test]
fn lost_read_back_reply_reconciles_without_a_second_append() {
    let remote = Arc::new(FakeRemote::default());
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let request = request_for(&client, "2026-10-30", "body");
    // Block listings during a write: same-day scan (1), read-back after append (2).
    remote.fail_list_blocks_on_call(2, RemoteError::Ambiguous);
    assert_eq!(
        client.write(&request, &AtomicBool::new(false)),
        WriteOutcome::AmbiguousWrite
    );
    assert_eq!(remote.0.lock().unwrap().append_anchors.len(), 1);
    assert!(matches!(
        client.inspect(&request),
        InspectionOutcome::Matching { .. }
    ));
    assert_eq!(remote.0.lock().unwrap().append_anchors.len(), 1);
}

#[test]
fn unowned_same_day_heading_is_a_conflict_and_nothing_is_written() {
    let remote = Arc::new(FakeRemote::default());
    remote.seed_children(ROOT, vec![page("year", "2026")]);
    remote.seed_children("year", vec![page("month", "2026-10")]);
    remote.seed_blocks("month", vec![remote_heading("theirs", "2026-10-30 (FRI)")]);
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let preflight = client.preflight(&day("2026-10-30"), "body").unwrap();
    assert_eq!(preflight.status, NotionPreflightStatus::Existing);
    let request = NotionRequest {
        day: day("2026-10-30"),
        markdown: "body".into(),
        expected_target_identity: preflight.target_identity,
        expected_payload_digest: preflight.payload_digest,
        expected_state_digest: preflight.observed_state_digest,
    };
    assert_eq!(
        client.write(&request, &AtomicBool::new(false)),
        WriteOutcome::Conflict
    );
    assert!(remote.0.lock().unwrap().append_anchors.is_empty());
    assert_eq!(remote.0.lock().unwrap().blocks["month"].len(), 1);
}

#[test]
fn changed_remote_state_or_cancellation_voids_the_write() {
    let remote = Arc::new(FakeRemote::default());
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let request = request_for(&client, "2026-10-30", "body");
    assert_eq!(
        client.write(&request, &AtomicBool::new(true)),
        WriteOutcome::DefiniteNoEffect
    );
    remote.seed_children(ROOT, vec![page("year", "2026")]);
    assert_eq!(
        client.write(&request, &AtomicBool::new(false)),
        WriteOutcome::Conflict
    );
    let state = remote.0.lock().unwrap();
    assert_eq!(state.creates, 0);
    assert!(state.append_anchors.is_empty());
}

#[test]
fn verified_section_is_adopted_again_without_another_write() {
    let remote = Arc::new(FakeRemote::default());
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let request = request_for(&client, "2026-10-30", "body");
    let WriteOutcome::Verified { returned_ids, .. } =
        client.write(&request, &AtomicBool::new(false))
    else {
        panic!("first write verifies");
    };
    let InspectionOutcome::Matching {
        returned_ids: again,
        ..
    } = client.inspect(&request)
    else {
        panic!("restart inspection matches the receipt");
    };
    assert_eq!(returned_ids, again);
    assert_eq!(remote.0.lock().unwrap().append_anchors.len(), 1);
}

fn dated(value: &str) -> NotionBlock {
    text_block(
        BlockKind::Heading2,
        render_notion(&day(value), "").unwrap().heading,
    )
}

fn para(text: &str) -> NotionBlock {
    text_block(BlockKind::Paragraph, text.into())
}

fn insertion(blocks: &[NotionBlock], value: &str) -> Insertion {
    plan_insertion(blocks, &render_notion(&day(value), "body").unwrap())
}

#[test]
fn insertion_goes_after_the_greatest_earlier_section() {
    let blocks = [
        dated("2026-10-28"),
        para("a"),
        para("a2"),
        dated("2026-10-30"),
        para("c"),
    ];
    let found = insertion(&blocks, "2026-10-29");
    assert_eq!(found.point, InsertionPoint::After(2));
    assert_eq!(found.label, "after 2026-10-28");
}

#[test]
fn a_later_day_goes_last_and_an_empty_page_appends() {
    let blocks = [dated("2026-10-28"), para("a")];
    let found = insertion(&blocks, "2026-10-30");
    assert_eq!(found.point, InsertionPoint::End);
    assert_eq!(found.label, "after 2026-10-28");
    let empty = insertion(&[], "2026-10-30");
    assert_eq!(empty.point, InsertionPoint::End);
    assert_eq!(empty.label, "end");
    // A page with only unrelated content also appends.
    let unrelated = insertion(&[para("notes")], "2026-10-30");
    assert_eq!(unrelated.point, InsertionPoint::End);
    assert_eq!(unrelated.label, "end");
}

#[test]
fn an_earlier_day_goes_before_the_first_later_section() {
    let blocks = [dated("2026-10-28"), para("a")];
    let first = insertion(&blocks, "2026-10-27");
    assert_eq!(first.point, InsertionPoint::Start);
    assert_eq!(first.label, "before 2026-10-28");
    let with_intro = [para("intro"), dated("2026-10-28"), para("a")];
    let after_intro = insertion(&with_intro, "2026-10-27");
    assert_eq!(after_intro.point, InsertionPoint::After(0));
    assert_eq!(after_intro.label, "before 2026-10-28");
}

#[test]
fn an_unsorted_page_is_never_made_less_sorted() {
    // 30 sits before 28; publishing 29 follows the greatest earlier date (28).
    let blocks = [
        dated("2026-10-30"),
        para("c"),
        dated("2026-10-28"),
        para("a"),
        dated("2026-10-31"),
    ];
    let found = insertion(&blocks, "2026-10-29");
    assert_eq!(found.point, InsertionPoint::After(3));
    assert_eq!(found.label, "after 2026-10-28");
}

#[test]
fn only_canonical_dated_headings_count_and_any_heading_ends_a_section() {
    let blocks = [
        dated("2026-10-28"),
        para("a"),
        text_block(BlockKind::Heading2, "2026-10-29".into()),
        text_block(BlockKind::Heading2, "2026-10-29 (wed)".into()),
        text_block(BlockKind::Heading1, "Notes".into()),
        para("n"),
    ];
    // The ISO-only and lower-case headings are not dated sections, but they
    // still end the 28th section, so the insertion follows its body.
    let found = insertion(&blocks, "2026-10-30");
    assert_eq!(found.label, "after 2026-10-28");
    assert_eq!(found.point, InsertionPoint::After(1));
}

fn seeded_month(days: &[&str]) -> Arc<FakeRemote> {
    let remote = Arc::new(FakeRemote::default());
    remote.seed_children(ROOT, vec![page("year", "2026")]);
    remote.seed_children("year", vec![page("month", "2026-10")]);
    let mut blocks = Vec::new();
    for value in days {
        blocks.push(RemoteBlock {
            id: format!("h-{value}"),
            block: dated(value),
        });
        blocks.push(RemoteBlock {
            id: format!("b-{value}"),
            block: para(&format!("body {value}")),
        });
    }
    remote.seed_blocks("month", blocks);
    remote
}

fn publish_day(remote: &Arc<FakeRemote>, value: &str, markdown: &str) -> WriteOutcome {
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let request = request_for(&client, value, markdown);
    client.write(&request, &AtomicBool::new(false))
}

fn headings(remote: &FakeRemote) -> Vec<String> {
    remote
        .month_texts("month")
        .into_iter()
        .filter(|text| text.starts_with("2026-10-"))
        .collect()
}

#[test]
fn a_new_day_is_inserted_between_existing_days() {
    let remote = seeded_month(&["2026-10-28", "2026-10-30"]);
    assert!(matches!(
        publish_day(&remote, "2026-10-29", "mine"),
        WriteOutcome::Verified { .. }
    ));
    assert_eq!(
        remote.month_texts("month"),
        vec![
            "2026-10-28 (WED)",
            "body 2026-10-28",
            "2026-10-29 (THU)",
            "mine",
            "2026-10-30 (FRI)",
            "body 2026-10-30",
        ]
    );
}

#[test]
fn an_earlier_day_is_inserted_first_and_a_later_day_last() {
    let remote = seeded_month(&["2026-10-28", "2026-10-30"]);
    assert!(matches!(
        publish_day(&remote, "2026-10-27", "first"),
        WriteOutcome::Verified { .. }
    ));
    assert_eq!(remote.0.lock().unwrap().start_inserts, 1);
    assert!(matches!(
        publish_day(&remote, "2026-10-31", "last"),
        WriteOutcome::Verified { .. }
    ));
    assert_eq!(
        headings(&remote),
        vec![
            "2026-10-27 (TUE)",
            "2026-10-28 (WED)",
            "2026-10-30 (FRI)",
            "2026-10-31 (SAT)"
        ]
    );
    // The append of the later day needed no anchor.
    let state = remote.0.lock().unwrap();
    assert_eq!(state.append_anchors.last(), Some(&None));
}

#[test]
fn a_section_over_one_hundred_blocks_inserts_mid_page_and_reads_back_verified() {
    let remote = seeded_month(&["2026-10-28", "2026-10-30"]);
    assert!(matches!(
        publish_day(&remote, "2026-10-29", &long_markdown()),
        WriteOutcome::Verified { .. }
    ));
    let texts = remote.month_texts("month");
    assert_eq!(texts.len(), 4 + 121);
    assert_eq!(texts[2], "2026-10-29 (THU)");
    assert_eq!(texts[3], "paragraph 0");
    assert_eq!(texts[122], "paragraph 119");
    assert_eq!(texts[123], "2026-10-30 (FRI)");
    // The second chunk chains after the last verified block of the first.
    let state = remote.0.lock().unwrap();
    assert_eq!(state.append_anchors.len(), 2);
    assert_eq!(state.append_anchors[0].as_deref(), Some("b-2026-10-28"));
    assert!(state.append_anchors[1].is_some());
}

#[test]
fn an_unsorted_page_keeps_its_order_and_the_new_day_never_worsens_it() {
    let remote = seeded_month(&["2026-10-30", "2026-10-28"]);
    assert!(matches!(
        publish_day(&remote, "2026-10-29", "mine"),
        WriteOutcome::Verified { .. }
    ));
    assert_eq!(
        headings(&remote),
        vec!["2026-10-30 (FRI)", "2026-10-28 (WED)", "2026-10-29 (THU)"]
    );
}

#[test]
fn preflight_reports_the_placement_and_a_moved_anchor_voids_the_plan() {
    let remote = seeded_month(&["2026-10-28", "2026-10-30"]);
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let preflight = client.preflight(&day("2026-10-29"), "mine").unwrap();
    assert_eq!(preflight.placement, "after 2026-10-28");
    let request = NotionRequest {
        day: day("2026-10-29"),
        markdown: "mine".into(),
        expected_target_identity: preflight.target_identity,
        expected_payload_digest: preflight.payload_digest,
        expected_state_digest: preflight.observed_state_digest,
    };
    // Someone adds a section in the meantime: the plan must be re-previewed.
    remote
        .append_blocks(
            "month",
            &[dated("2026-10-29")],
            &Placement::End,
            &AtomicBool::new(false),
        )
        .unwrap();
    let before = remote.month_texts("month");
    assert_eq!(
        client.write(&request, &AtomicBool::new(false)),
        WriteOutcome::Conflict
    );
    assert_eq!(remote.month_texts("month"), before);
    // A month page that does not exist yet appends.
    let fresh = Arc::new(FakeRemote::default());
    let fresh_client = NotionClient::scripted(fresh, ROOT);
    assert_eq!(
        fresh_client
            .preflight(&day("2026-10-29"), "mine")
            .unwrap()
            .placement,
        "end"
    );
}

#[test]
fn a_lost_reply_mid_page_is_adopted_without_a_second_insert() {
    let remote = seeded_month(&["2026-10-28", "2026-10-30"]);
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let request = request_for(&client, "2026-10-29", "mine");
    remote.lose_next_append_reply();
    assert_eq!(
        client.write(&request, &AtomicBool::new(false)),
        WriteOutcome::AmbiguousWrite
    );
    let InspectionOutcome::Matching { returned_ids, .. } = client.inspect(&request) else {
        panic!("the committed mid-page section must reconcile");
    };
    assert_eq!(returned_ids.len(), 2);
    assert_eq!(remote.0.lock().unwrap().append_anchors.len(), 1);
    assert_eq!(headings(&remote).len(), 3);
}

fn extend_request(
    client: &NotionClient,
    value: &str,
    markdown: &str,
    resolved: bool,
) -> NotionExtendRequest {
    let existing = client.observe_existing(&day(value)).unwrap().unwrap();
    let mut request = NotionExtendRequest {
        day: day(value),
        markdown: markdown.into(),
        appended_at: "2026-10-30 21:14".into(),
        expected_target_identity: ROOT.into(),
        expected_result_digest: None,
        owned_month_id: existing.month_id,
        owned_block_ids: existing.block_ids,
        owned_section_digest: existing.section_digest,
    };
    if resolved {
        request.expected_result_digest = Some(
            client
                .preflight_extend(&request)
                .unwrap()
                .preflight
                .payload_digest,
        );
    }
    request
}

#[test]
fn an_existing_canonical_section_is_observed_and_anything_else_is_not_offered() {
    let remote = seeded_month(&["2026-10-27", "2026-10-28"]);
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let section = client
        .observe_existing(&day("2026-10-28"))
        .unwrap()
        .unwrap();
    assert_eq!(section.month_id, "month");
    assert_eq!(section.block_ids, vec!["h-2026-10-28", "b-2026-10-28"]);
    assert_eq!(section.blocks.len(), 2);
    assert_eq!(client.observe_existing(&day("2026-10-29")), Ok(None));
    // A heading that is not the canonical form is a plain conflict.
    remote
        .append_blocks(
            "month",
            &[text_block(BlockKind::Heading2, "2026-10-29".into())],
            &Placement::End,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(
        client.observe_existing(&day("2026-10-29")),
        Err("notion-same-day-conflict".to_owned())
    );
}

#[test]
fn append_adds_a_separator_dated_line_and_body_after_the_last_block_of_the_section() {
    let remote = seeded_month(&["2026-10-27", "2026-10-28", "2026-10-29"]);
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let request = extend_request(&client, "2026-10-28", "More.", true);
    let WriteOutcome::Verified {
        receipt_id,
        returned_ids,
    } = client.extend_section(&request, &AtomicBool::new(false))
    else {
        panic!("an untouched section must be extended");
    };
    assert_eq!(receipt_id, "month");
    assert_eq!(returned_ids.len(), 5);
    assert_eq!(
        remote.month_texts("month"),
        vec![
            "2026-10-27 (TUE)",
            "body 2026-10-27",
            "2026-10-28 (WED)",
            "body 2026-10-28",
            "---",
            "Appended 2026-10-30 21:14",
            "More.",
            "2026-10-29 (THU)",
            "body 2026-10-29",
        ]
    );
    // The dated line is italic; the rule is a literal paragraph.
    let state = remote.0.lock().unwrap();
    let dated = &state.blocks["month"][5].block;
    assert!(dated.rich_text.iter().all(|item| item.annotations.italic));
    drop(state);
    // A replay is a no-op: nothing is inserted twice.
    let appends = remote.0.lock().unwrap().append_anchors.len();
    assert!(matches!(
        client.extend_section(&request, &AtomicBool::new(false)),
        WriteOutcome::Verified { .. }
    ));
    assert_eq!(remote.0.lock().unwrap().append_anchors.len(), appends);
}

#[test]
fn blocked_same_day_page_has_a_direct_month_url() {
    let remote = Arc::new(FakeRemote::default());
    let month_id = "01234567-89ab-cdef-0123-456789abcdef";
    remote.seed_children(ROOT, vec![page("year", "2026")]);
    remote.seed_children("year", vec![page(month_id, "2026-10")]);
    remote.seed_blocks(
        month_id,
        vec![
            RemoteBlock {
                id: "h1".into(),
                block: dated("2026-10-29"),
            },
            RemoteBlock {
                id: "h2".into(),
                block: dated("2026-10-29"),
            },
        ],
    );
    let client = NotionClient::scripted(remote, ROOT);
    let preflight = client.preflight(&day("2026-10-29"), "Today").unwrap();
    assert_eq!(preflight.status, NotionPreflightStatus::Existing);
    assert_eq!(
        client.observe_existing(&day("2026-10-29")),
        Err("notion-same-day-conflict".to_owned())
    );
    assert_eq!(
        client.month_page_url(&day("2026-10-29")),
        Some("https://www.notion.so/0123456789abcdef0123456789abcdef".to_owned())
    );
}

#[test]
fn the_previewed_result_digest_matches_the_section_after_the_append() {
    let remote = seeded_month(&["2026-10-28"]);
    let client = NotionClient::scripted(remote, ROOT);
    let request = extend_request(&client, "2026-10-28", "More.", true);
    assert!(matches!(
        client.extend_section(&request, &AtomicBool::new(false)),
        WriteOutcome::Verified { .. }
    ));
    let after = client
        .observe_existing(&day("2026-10-28"))
        .unwrap()
        .unwrap();
    assert_eq!(Some(after.section_digest), request.expected_result_digest);
}

#[test]
fn append_refuses_a_section_edited_since_the_preview_and_writes_nothing() {
    let remote = seeded_month(&["2026-10-28", "2026-10-29"]);
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let request = extend_request(&client, "2026-10-28", "More.", true);
    // Someone edits the section after the preview.
    remote
        .append_blocks(
            "month",
            &[para("added by hand")],
            &Placement::After("b-2026-10-28".into()),
            &AtomicBool::new(false),
        )
        .unwrap();
    let before = remote.0.lock().unwrap().append_anchors.len();
    assert_eq!(
        client.extend_section(&request, &AtomicBool::new(false)),
        WriteOutcome::Conflict
    );
    assert_eq!(remote.0.lock().unwrap().append_anchors.len(), before);
    let preflight = client.preflight_extend(&request).unwrap();
    assert_eq!(preflight.preflight.status, NotionPreflightStatus::Existing);
}

#[test]
fn a_long_append_chunks_and_an_interrupted_one_resumes_without_duplicates() {
    let remote = seeded_month(&["2026-10-28", "2026-10-29"]);
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let request = extend_request(&client, "2026-10-28", &long_markdown(), true);
    let base = remote.0.lock().unwrap().append_anchors.len();
    // The first chunk lands but its reply is lost.
    remote.lose_append_reply_on_call(base + 1, true);
    assert_eq!(
        client.extend_section(&request, &AtomicBool::new(false)),
        WriteOutcome::AmbiguousWrite
    );
    assert!(matches!(
        client.extend_section(&request, &AtomicBool::new(false)),
        WriteOutcome::Verified { .. }
    ));
    let texts = remote.month_texts("month");
    // heading + body + rule + dated line + 120 paragraphs, then the next day.
    assert_eq!(texts.len(), 2 + 2 + 120 + 2);
    assert_eq!(texts[4], "paragraph 0");
    assert_eq!(texts[123], "paragraph 119");
    assert_eq!(texts[124], "2026-10-29 (THU)");
}

#[test]
fn overwrite_replaces_a_section_the_app_did_not_write_and_keeps_its_neighbors() {
    let remote = seeded_month(&["2026-10-27", "2026-10-28", "2026-10-29"]);
    let client = NotionClient::scripted(remote.clone(), ROOT);
    let existing = client
        .observe_existing(&day("2026-10-28"))
        .unwrap()
        .unwrap();
    let new = render_notion(&day("2026-10-28"), "Fresh.").unwrap();
    let request = NotionReplaceRequest {
        day: day("2026-10-28"),
        markdown: "Fresh.".into(),
        expected_target_identity: ROOT.into(),
        expected_payload_digest: Digest::parse(new.payload_digest).unwrap(),
        owned_month_id: existing.month_id,
        owned_block_ids: existing.block_ids.clone(),
        owned_payload_digest: existing.section_digest,
    };
    let preview = client.preflight_replace(&request).unwrap();
    assert_eq!(preview.preflight.status, NotionPreflightStatus::Ready);
    assert_eq!(preview.current_blocks.len(), 2);
    let WriteOutcome::Verified { returned_ids, .. } =
        client.replace_section(&request, &AtomicBool::new(false))
    else {
        panic!("an untouched foreign section must be replaced");
    };
    assert_eq!(returned_ids[0], existing.block_ids[0], "heading is kept");
    assert_eq!(
        remote.month_texts("month"),
        vec![
            "2026-10-27 (TUE)",
            "body 2026-10-27",
            "2026-10-28 (WED)",
            "Fresh.",
            "2026-10-29 (THU)",
            "body 2026-10-29",
        ]
    );
    assert_eq!(remote.0.lock().unwrap().trashed, vec!["b-2026-10-28"]);
}
