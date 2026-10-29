# UX terms

Status: Active

These terms give the terminal client consistent labels for its
workflow, stages, and components. They use the language of the existing product
decisions; code-facing names are suggestions for later implementation.

## Canonical workflow stages

The Workflow Sidebar always uses these labels, in this order. They are
user-facing navigation stages, not aliases for the finer-grained server
workflow states.

| Stage | Code-facing name | Contains |
|---|---|---|
| Start | `start` | Server and setup status, unfinished workflows, and diary-day choice. |
| Sources | `sources` | Discovery, source-note inclusion, readable-text inspection, selected-only download, and source-set confirmation. |
| Compose | `compose` | Assistant selection, optional memory review, draft creation, manual editing, and paragraph review. The manual editor is always available. |
| Review | `review` | Review and confirmation of the complete edited entry. |
| Preview | `preview` | Exact local and Notion output plans and approval. |
| Publish | `publish` | Per-destination progress, verification, retry, and conflict resolution. |

Use these six labels wherever the complete journey is shown. An optional step
inside a stage can be unavailable without removing or renaming the stage. Each
stage has a canonical-state/capability-derived progress state, and exactly one has the
presentation-only `current` marker because its resource is open in the Main
Pane. The current marker can coexist with `done`, `available`, or `stale` and
does not mutate the server's authoritative domain state.

## Proposed visible terms

| UI term | Code-facing name | Meaning |
|---|---|---|
| Diary day | `diary_day` | The single date used for discovery, output naming, and Notion heading. |
| Source note | `source_note` | One discovered, selectable Markdown note. |
| Active resource | `active_resource` | The item whose content is shown in the Main Pane. Activating a source note for inspection does not include or exclude it. |
| Date disagreement | `date_disagreement` | Directory and valid header dates differ. Neither date overrides the person's selection. |
| Source selection | `source_selection` | The included notes and their confirmed snapshots. |
| Download needed | `download_needed` | A selected iCloud note needs a local copy before it can be read. |
| Memory review | `memory_review` | Optional review of assistant-extracted, source-linked memories. |
| Draft editor | `draft_editor` | Multiline editor used for both manual and assistant-created drafts. |
| Assistant backend | `assistant_backend` | One selectable implementation of the shared `DraftAssistant` contract, identified independently from its execution boundary. Examples include an Apple on-device backend and a hosted backend; availability and supported actions are shown at runtime. |
| Paragraph review | `paragraph_review` | An assistant suggestion for the paragraph focused at the editor cursor. The suggestion never changes the entry until the person accepts it; rejecting it leaves the entry unchanged. |
| Inferred reflection | `inferred_reflection` | A model-suggested interpretation shown in the editor between reserved delimiters. The person accepts it by removing the delimiters, edits it, or removes it. |
| Entry review | `entry_review` | Review and confirmation of the complete edited diary entry. Refused while any reserved reflection delimiter character remains. |
| Output preview | `output_preview` | Exact local and Notion payloads and destination paths before approval, including any destination that is blocked. |
| Publication status | `publication_status` | Per-destination state: verified, pending, blocked, or conflict. |
| Conflict view | `conflict_view` | Existing destination content beside the planned payload, with the resolutions the person may choose. |
| Amendment | `amendment` | A new revision of a Completed day, seeded from the saved local file. After approval it replaces only content the app itself wrote and receipted. |
| Draft proposal | `draft_proposal` | Memories or entry text written by an agent client, shown in a Banner for the person to accept or reject. |
| Composition path | `composition_path` | The selected assistant backend's execution boundary: hosted (default) or Apple on-device. It is labeled in the UI and is never a consent step. Manual editing is always available and is not an assistant backend. |
| Setup check | `setup_check` | Read-only readiness list on the Start stage: configuration, Ayumi directory, entries root, assistant backends, and Notion. |
| Server connection | `server_connection` | The client's link to the manually started server: not running, connecting, connected, reconnecting, or version mismatch. |
| IME indicator | `ime_indicator` | Language-level macOS input-source label: `IME: EN`, `IME: KO`, or `IME: ?` for any other or unknown language. |

Use “source note” for input and “entry” for the finished diary. Use “confirm”
for the person's selection or text decision, “approve” for the exact output
plan, and “publish” for running the approved plan, which writes the local file
and then publishes to Notion. A screen that reports one destination says
“write” for the local file and “publish to Notion” for Notion. “Blocked” means
a destination cannot be attempted yet, for example because Notion is not
configured; it is neither a failure nor a conflict. A cloud-only note is never
described as empty merely because it has not downloaded. A `ko-KR` entry drafted
from en-US notes is labeled a translation. The [UX information architecture](ux-information-architecture.md)
places these terms in the stage flow.

## Proposed chrome terms

The terminal client's screen chrome is five zones, ordered from left to right as
the Workflow Sidebar, Resources Sidebar, and Main Pane, followed by a bottom
Command Line and bottom-most Status Line.

| UI term | Code-facing name | Meaning |
|---|---|---|
| Workflow Sidebar | `workflow_sidebar` | Left chrome pane listing the six canonical stages and their stage state for jump-to-stage navigation. |
| Resources Sidebar | `resources_sidebar` | Middle chrome pane listing context-relevant sources, inputs, assistant suggestions, or outputs with explicit resource state; activating an item shows its content in the Main Pane, and it may render an indented tree. |
| Stage state | `stage_state` | A Workflow Sidebar progress marker: done, available, locked, or stale, plus a separate current marker for the stage open in the Main Pane. Always use text or symbols, never color alone. |
| Resource state | `resource_state` | One or more Resources Sidebar markers: ready, active, included, preparing, unavailable, warning, blocked, conflict, or stale. `active` controls the Main Pane; `included` controls the prospective source set. Always use text or symbols, never color alone. |
| Main Pane | `main_pane` | Right chrome pane; renders the active stage and active resource. |
| Banner | `banner` | Top region of the Main Pane for a condition that needs a decision, such as a conflict or a stale source, with its explanation. |
| Command Line | `command_line` | Bottom single-line input for typed commands; not a text filter. Opens with a chord, takes ASCII tokens, and maps Hangul typed on the 2-Set Korean layout back to them. |
| Status Line | `status_line` | Bottom-most ambient bar: diary day, workflow state, connection, IME indicator, progress, a short error summary, and action hints truncated by priority. |

Status Line is the single realization of both the "App status" and
"Action/status area" content named in the
[UX information architecture](ux-information-architecture.md#chrome-layout).
Keys follow "Keybindings" in [AGENTS.md](../AGENTS.md).
