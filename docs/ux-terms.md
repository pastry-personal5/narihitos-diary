# UX terms

Status: Proposal

These proposed terms give the terminal client consistent labels for its
workflow and components. They use the language of the existing product
decisions; code-facing names are suggestions for later implementation.

## Proposed visible terms

| UI term | Code-facing name | Meaning |
|---|---|---|
| Diary day | `diary_day` | The single date used for discovery, output naming, and Notion heading. |
| Source note | `source_note` | One discovered, selectable Markdown note. |
| Date disagreement | `date_disagreement` | Directory and valid header dates differ. Neither date overrides the person's selection. |
| Source selection | `source_selection` | The included notes and their confirmed snapshots. |
| Download needed | `download_needed` | A selected iCloud note needs a local copy before it can be read. |
| Memory review | `memory_review` | Optional review of model-extracted, source-linked memories. |
| Draft editor | `draft_editor` | Multiline editor used for both manual and model-assisted drafts. |
| Inferred reflection | `inferred_reflection` | A model-suggested interpretation shown in the editor between reserved delimiters. The person accepts it by removing the delimiters, edits it, or removes it. |
| Entry review | `entry_review` | Review and confirmation of the complete edited diary entry. Refused while any reserved reflection delimiter character remains. |
| Output preview | `output_preview` | Exact local and Notion payloads and destination paths before approval, including any destination that is blocked. |
| Publication status | `publication_status` | Per-destination state: verified, pending, blocked, or conflict. |
| Conflict view | `conflict_view` | Existing destination content beside the planned payload, with the resolutions the person may choose. |
| Amendment | `amendment` | A new revision of a Completed day, seeded from the saved local file. After approval it replaces only content the app itself wrote and receipted. |
| Draft proposal | `draft_proposal` | Memories or entry text written by an agent client, shown in a Banner for the person to accept or reject. |
| Composition path | `composition_path` | Hosted (default) or Apple on-device. It is labeled in the UI and is never a consent step. |
| Setup check | `setup_check` | Read-only readiness list on the Start screen: configuration, Ayumi directory, entries root, composition path, and Notion. |
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
places these terms in the screen flow.

## Proposed chrome terms

The terminal client's screen chrome is five zones: a left Workflow Sidebar, a
central Main Pane, a right Resources Sidebar, a bottom Command Line, and a
bottom-most Status Line.

| UI term | Code-facing name | Meaning |
|---|---|---|
| Workflow Sidebar | `workflow_sidebar` | Left chrome pane listing Standard Layout stages and their step state for jump-to-step navigation. |
| Resources Sidebar | `resources_sidebar` | Right chrome pane listing context-relevant sources, inputs, or outputs with explicit resource state; it may render an indented tree. |
| Step state | `step_state` | A Workflow Sidebar marker: done, current, available, locked, or stale. Always a text label or symbol, never color alone. |
| Resource state | `resource_state` | A Resources Sidebar marker: ready, selected, preparing, unavailable, warning, blocked, conflict, or stale. Always a text label or symbol, never color alone. |
| Main Pane | `main_pane` | Central chrome pane; renders the active screen's content. |
| Banner | `banner` | Top region of the Main Pane for a condition that needs a decision, such as a conflict or a stale source, with its explanation. |
| Command Line | `command_line` | Bottom single-line input for typed commands; not a text filter. Opens with a chord, takes ASCII tokens, and maps Hangul typed on the 2-Set Korean layout back to them. |
| Status Line | `status_line` | Bottom-most ambient bar: diary day, workflow state, connection, IME indicator, progress, a short error summary, and action hints truncated by priority. |

Status Line is the single realization of both the "App status" and
"Action/status area" content named in the
[UX information architecture](ux-information-architecture.md#chrome-layout).
Keys follow "Keybindings" in [AGENTS.md](../AGENTS.md).
