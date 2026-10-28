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
| Inferred reflection | `inferred_reflection` | A model-suggested interpretation marked for acceptance, editing, or removal. |
| Entry review | `entry_review` | Review and confirmation of the complete edited diary entry. |
| Output preview | `output_preview` | Exact local and Notion payloads and destination paths before approval. |
| Publication status | `publication_status` | Per-destination success, pending, conflict, or verification state. |
| IME indicator | `ime_indicator` | Language-level macOS input-source label, including `IME: ?` when unknown. |

Use “source note” for input and “entry” for the finished diary. Use “confirm”
for the person's selection or text decision and “publish” for the Notion side
effect. A cloud-only note is never described as empty merely because it has
not downloaded. The [UX information architecture](ux-information-architecture.md)
places these terms in the screen flow.
