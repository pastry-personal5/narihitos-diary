//! Notion HTTP transport, queueing, and remote JSON mapping.

use super::*;

/// First-in-first-out queue shared by every Notion request in the process.
/// A job that finishes one request goes to the back of the line, so a job
/// issuing back-to-back requests cannot starve a waiting one; a waiter that is
/// cancelled leaves the line without blocking those behind it.
pub(super) struct RequestQueue {
    pub(super) state: Mutex<QueueState>,
    pub(super) turn_changed: std::sync::Condvar,
}

#[derive(Default)]
pub(super) struct QueueState {
    next_ticket: u64,
    waiting: std::collections::VecDeque<u64>,
    busy: bool,
}

pub(super) struct QueueTurn<'a>(&'a RequestQueue);

pub(super) fn request_queue() -> &'static RequestQueue {
    static QUEUE: OnceLock<RequestQueue> = OnceLock::new();
    QUEUE.get_or_init(|| RequestQueue {
        state: Mutex::new(QueueState::default()),
        turn_changed: std::sync::Condvar::new(),
    })
}

impl RequestQueue {
    pub(super) fn enter(&self, cancelled: &AtomicBool) -> Result<QueueTurn<'_>, RemoteError> {
        let mut state = self.state.lock().map_err(|_| RemoteError::Ambiguous)?;
        let ticket = state.next_ticket;
        state.next_ticket = state.next_ticket.wrapping_add(1);
        state.waiting.push_back(ticket);
        loop {
            if cancelled.load(Ordering::Acquire) {
                state.waiting.retain(|waiting| *waiting != ticket);
                self.turn_changed.notify_all();
                return Err(RemoteError::Pending);
            }
            if !state.busy && state.waiting.front() == Some(&ticket) {
                state.waiting.pop_front();
                state.busy = true;
                return Ok(QueueTurn(self));
            }
            // Wake periodically so a cancelled waiter notices without a signal.
            state = self
                .turn_changed
                .wait_timeout(state, Duration::from_millis(10))
                .map_err(|_| RemoteError::Ambiguous)?
                .0;
        }
    }
}

impl Drop for QueueTurn<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.busy = false;
        }
        self.0.turn_changed.notify_all();
    }
}

type Waiter = Arc<dyn Fn(Duration, &AtomicBool) -> Result<(), RemoteError> + Send + Sync>;

pub(super) struct HttpRemote {
    token: String,
    agent: ureq::Agent,
    base_url: String,
    wait: Waiter,
}

impl HttpRemote {
    pub(super) fn new(token: String) -> Result<Self, NotionError> {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        Ok(Self {
            token,
            agent: config.into(),
            base_url: "https://api.notion.com/v1".to_owned(),
            wait: Arc::new(wait_cancellable),
        })
    }

    /// Points the adapter at a local stub with an injected wait, so the retry
    /// matrix runs without a network or real sleeps.
    #[cfg(test)]
    pub(super) fn with_stub(mut self, base_url: String, wait: Waiter) -> Self {
        self.base_url = base_url;
        self.wait = wait;
        self
    }

    pub(super) fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
        is_write: bool,
        cancelled: &AtomicBool,
    ) -> Result<serde_json::Value, RemoteError> {
        let _turn = request_queue().enter(cancelled)?;
        let url = format!("{}{path}", self.base_url);
        let mut retry = RetryPolicy::default();
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(RemoteError::Pending);
            }
            let authorization = format!("Bearer {}", self.token);
            let encoded = body
                .map(serde_json::to_vec)
                .transpose()
                .map_err(|_| RemoteError::Invalid)?;
            let result = match (method, encoded) {
                ("GET", None) => self
                    .agent
                    .get(&url)
                    .header("Authorization", &authorization)
                    .header("Notion-Version", API_VERSION)
                    .header("Content-Type", "application/json")
                    .call(),
                ("POST", Some(encoded)) => self
                    .agent
                    .post(&url)
                    .header("Authorization", &authorization)
                    .header("Notion-Version", API_VERSION)
                    .header("Content-Type", "application/json")
                    .send(encoded),
                ("DELETE", None) => self
                    .agent
                    .delete(&url)
                    .header("Authorization", &authorization)
                    .header("Notion-Version", API_VERSION)
                    .call(),
                ("PATCH", Some(encoded)) => self
                    .agent
                    .patch(&url)
                    .header("Authorization", &authorization)
                    .header("Notion-Version", API_VERSION)
                    .header("Content-Type", "application/json")
                    .send(encoded),
                _ => return Err(RemoteError::Invalid),
            };
            let mut response = match result {
                Ok(response) => response,
                Err(_) if is_write => return Err(RemoteError::Ambiguous),
                Err(_) => match retry.classify(None, None, false) {
                    RetryAction::RetryAfter(delay) => {
                        (self.wait)(delay, cancelled)?;
                        continue;
                    }
                    _ => return Err(RemoteError::Blocked(SetupReason::RemoteUnavailable)),
                },
            };
            let status = response.status().as_u16();
            let retry_after = response
                .headers()
                .get("Retry-After")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(Duration::from_secs);
            if !(200..300).contains(&status) {
                match retry.classify(Some(status), retry_after, is_write) {
                    RetryAction::RetryAfter(delay) => {
                        (self.wait)(delay, cancelled)?;
                        continue;
                    }
                    RetryAction::InspectAmbiguousWrite => return Err(RemoteError::Ambiguous),
                    RetryAction::Pending | RetryAction::Exhausted => {
                        return Err(RemoteError::Pending);
                    }
                    RetryAction::Changed => return Err(RemoteError::Invalid),
                    _ => return Err(RemoteError::Blocked(content_free_reason(Some(status)))),
                }
            }
            let raw = response
                .body_mut()
                .with_config()
                .limit(2 * 1024 * 1024)
                .read_to_string()
                .map_err(|_| {
                    if is_write {
                        RemoteError::Ambiguous
                    } else {
                        RemoteError::Invalid
                    }
                })?;
            return serde_json::from_str(&raw).map_err(|_| RemoteError::Invalid);
        }
    }

    fn list_all(
        &self,
        parent_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<Vec<serde_json::Value>, RemoteError> {
        let mut cursor: Option<String> = None;
        let mut all = Vec::new();
        loop {
            let encoded_cursor = cursor
                .as_deref()
                .map(|value| format!("&start_cursor={}", encode_query_component(value)))
                .unwrap_or_default();
            let response = self.request(
                "GET",
                &format!("/blocks/{parent_id}/children?page_size=100{encoded_cursor}"),
                None,
                false,
                cancelled,
            )?;
            let page = response.as_object().ok_or(RemoteError::Invalid)?;
            let results = page
                .get("results")
                .and_then(serde_json::Value::as_array)
                .ok_or(RemoteError::Invalid)?;
            all.extend(results.iter().cloned());
            let has_more = page
                .get("has_more")
                .and_then(serde_json::Value::as_bool)
                .ok_or(RemoteError::Invalid)?;
            if !has_more {
                if page
                    .get("next_cursor")
                    .is_some_and(|value| !value.is_null())
                {
                    return Err(RemoteError::Invalid);
                }
                break;
            }
            cursor = Some(
                page.get("next_cursor")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .ok_or(RemoteError::Invalid)?
                    .to_owned(),
            );
        }
        Ok(all)
    }

    fn parse_block_tree(
        &self,
        value: &serde_json::Value,
        cancelled: &AtomicBool,
    ) -> Result<RemoteBlock, RemoteError> {
        let mut parsed = parse_remote_block(value)?;
        let has_children = value
            .get("has_children")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if has_children {
            parsed.block.children = self
                .list_all(&parsed.id, cancelled)?
                .iter()
                .map(|child| {
                    self.parse_block_tree(child, cancelled)
                        .map(|item| item.block)
                })
                .collect::<Result<Vec<_>, _>>()?;
        }
        Ok(parsed)
    }
}

pub(super) fn encode_query_component(value: &str) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(&mut encoded, "%{byte:02X}");
        }
    }
    encoded
}

pub(super) fn wait_cancellable(delay: Duration, cancelled: &AtomicBool) -> Result<(), RemoteError> {
    let started = std::time::Instant::now();
    while started.elapsed() < delay {
        if cancelled.load(Ordering::Acquire) {
            return Err(RemoteError::Pending);
        }
        let remaining = delay.saturating_sub(started.elapsed());
        std::thread::sleep(remaining.min(Duration::from_millis(50)));
    }
    Ok(())
}

impl NotionRemote for HttpRemote {
    fn list_children(
        &self,
        parent_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<Vec<PageChild>, RemoteError> {
        self.list_all(parent_id, cancelled)?
            .into_iter()
            .map(|value| parse_page_child(&value))
            .collect()
    }

    fn list_blocks(
        &self,
        parent_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<Vec<RemoteBlock>, RemoteError> {
        self.list_all(parent_id, cancelled)?
            .iter()
            .map(|value| self.parse_block_tree(value, cancelled))
            .collect()
    }

    fn create_page(
        &self,
        parent_id: &str,
        title: &str,
        cancelled: &AtomicBool,
    ) -> Result<String, RemoteError> {
        let body = serde_json::json!({
            "parent": { "type": "page_id", "page_id": parent_id },
            "properties": {
                "title": {
                    "type": "title",
                    "title": [{ "type": "text", "text": { "content": title } }]
                }
            }
        });
        self.request("POST", "/pages", Some(&body), true, cancelled)?
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or(RemoteError::Invalid)
    }

    fn append_blocks(
        &self,
        parent_id: &str,
        blocks: &[NotionBlock],
        place: &Placement,
        cancelled: &AtomicBool,
    ) -> Result<Vec<RemoteBlock>, RemoteError> {
        let children: Vec<_> = blocks.iter().map(block_json).collect();
        let position = match place {
            Placement::End => serde_json::json!({ "type": "end" }),
            Placement::Start => serde_json::json!({ "type": "start" }),
            Placement::After(id) => {
                serde_json::json!({ "type": "after_block", "after_block": { "id": id } })
            }
        };
        let body = serde_json::json!({ "children": children, "position": position });
        let response = self.request(
            "PATCH",
            &format!("/blocks/{parent_id}/children"),
            Some(&body),
            true,
            cancelled,
        )?;
        response
            .get("results")
            .and_then(serde_json::Value::as_array)
            .ok_or(RemoteError::Invalid)?
            .iter()
            .map(|value| self.parse_block_tree(value, cancelled))
            .collect()
    }

    fn trash_block(&self, block_id: &str, cancelled: &AtomicBool) -> Result<(), RemoteError> {
        self.request(
            "DELETE",
            &format!("/blocks/{block_id}"),
            None,
            true,
            cancelled,
        )
        .map(|_| ())
    }
}

pub(super) fn parse_page_child(value: &serde_json::Value) -> Result<PageChild, RemoteError> {
    let object = value.as_object().ok_or(RemoteError::Invalid)?;
    let id = object
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or(RemoteError::Invalid)?
        .to_owned();
    let block_type = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or(RemoteError::Invalid)?;
    let title = if block_type == "child_page" {
        object
            .get("child_page")
            .and_then(|value| value.get("title"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    } else {
        parse_rich_text(object.get(block_type))
            .ok()
            .map(|items| items.into_iter().map(|item| item.text).collect::<String>())
    };
    Ok(PageChild {
        id,
        kind: if block_type == "child_page" {
            PageKind::ChildPage
        } else {
            PageKind::Other
        },
        title,
        trashed: object
            .get("in_trash")
            .or_else(|| object.get("archived"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        accessible: true,
    })
}

pub(super) fn parse_remote_block(value: &serde_json::Value) -> Result<RemoteBlock, RemoteError> {
    let object = value.as_object().ok_or(RemoteError::Invalid)?;
    let id = object
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or(RemoteError::Invalid)?
        .to_owned();
    let block_type = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or(RemoteError::Invalid)?;
    let kind = match block_type {
        "paragraph" => BlockKind::Paragraph,
        "heading_1" => BlockKind::Heading1,
        "heading_2" => BlockKind::Heading2,
        "heading_3" => BlockKind::Heading3,
        "bulleted_list_item" => BlockKind::BulletedListItem,
        "numbered_list_item" => BlockKind::NumberedListItem,
        "quote" => BlockKind::Quote,
        "code" => BlockKind::Code,
        _ => return Err(RemoteError::Invalid),
    };
    let fields = object.get(block_type).ok_or(RemoteError::Invalid)?;
    let rich_text = parse_rich_text(Some(fields))?;
    let language = fields
        .get("language")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let children = fields
        .get("children")
        .and_then(serde_json::Value::as_array)
        .map(|children| {
            children
                .iter()
                .map(parse_remote_block)
                .map(|result| result.map(|item| item.block))
                .collect()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(RemoteBlock {
        id,
        block: NotionBlock {
            kind,
            rich_text,
            language,
            children,
        },
    })
}

pub(super) fn parse_rich_text(
    value: Option<&serde_json::Value>,
) -> Result<Vec<NotionRichText>, RemoteError> {
    value
        .and_then(|value| value.get("rich_text"))
        .and_then(serde_json::Value::as_array)
        .ok_or(RemoteError::Invalid)?
        .iter()
        .map(|item| {
            let object = item.as_object().ok_or(RemoteError::Invalid)?;
            let text = object
                .get("plain_text")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    object
                        .get("text")
                        .and_then(|text| text.get("content"))
                        .and_then(serde_json::Value::as_str)
                })
                .ok_or(RemoteError::Invalid)?
                .to_owned();
            let annotations = object.get("annotations");
            let flag = |name| {
                annotations
                    .and_then(|value| value.get(name))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
            };
            let link = object
                .get("href")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    object
                        .get("text")
                        .and_then(|text| text.get("link"))
                        .and_then(|link| link.get("url"))
                        .and_then(serde_json::Value::as_str)
                })
                .map(str::to_owned);
            Ok(NotionRichText {
                text,
                annotations: NotionTextAnnotations {
                    bold: flag("bold"),
                    italic: flag("italic"),
                    strikethrough: flag("strikethrough"),
                    code: flag("code"),
                },
                link,
            })
        })
        .collect()
}

pub(super) fn block_json(block: &NotionBlock) -> serde_json::Value {
    let rich_text: Vec<_> = block
        .rich_text
        .iter()
        .map(|item| {
            serde_json::json!({
                "type": "text",
                "text": {
                    "content": item.text,
                    "link": item.link.as_ref().map(|url| serde_json::json!({ "url": url }))
                },
                "annotations": {
                    "bold": item.annotations.bold,
                    "italic": item.annotations.italic,
                    "strikethrough": item.annotations.strikethrough,
                    "underline": false,
                    "code": item.annotations.code,
                    "color": "default"
                }
            })
        })
        .collect();
    let children: Vec<_> = block.children.iter().map(block_json).collect();
    let mut fields = serde_json::json!({ "rich_text": rich_text });
    if !children.is_empty() {
        fields["children"] = serde_json::Value::Array(children);
    }
    if let Some(language) = &block.language {
        fields["language"] = serde_json::Value::String(language.clone());
    }
    let mut object = serde_json::Map::new();
    object.insert("object".into(), serde_json::Value::String("block".into()));
    object.insert(
        "type".into(),
        serde_json::Value::String(block.kind.as_str().into()),
    );
    object.insert(block.kind.as_str().into(), fields);
    serde_json::Value::Object(object)
}

#[allow(dead_code)]
pub(super) fn canonical_object_for_block(
    block: &NotionBlock,
) -> BTreeMap<&'static str, serde_json::Value> {
    let mut value = BTreeMap::new();
    value.insert(
        "type",
        serde_json::Value::String(block.kind.as_str().into()),
    );
    value.insert(
        "rich_text",
        serde_json::to_value(&block.rich_text).unwrap_or(serde_json::Value::Null),
    );
    value
}
