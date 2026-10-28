# MVP operational constraints research

Status: Active

Research date: 2026-10-28

This document captures cross-cutting reliability and privacy findings that do
not belong solely to the Markdown, Notion, or TUI research. It is not a phase
or milestone plan.

## Two outputs are not one transaction

One completed diary produces:

1. A final Markdown file on the local filesystem.
2. A daily section in a Notion month subpage beneath the configured root and
   year pages.

The filesystem and Notion API cannot participate in one atomic transaction.
The application must expose partial success instead of claiming that the two
writes always succeed together.

The lowest-risk order is:

1. Let the user preview and confirm both outputs.
2. Write the local file to a temporary file in the destination directory.
3. Flush and atomically rename it to its final name on the same filesystem.
4. Resolve or create the target year and month subpages, then submit the
   Notion section to the month page.
5. Verify the Notion result and record the page and block IDs locally.

The local diary is then the durable source of truth. A Notion failure leaves a
complete local entry available for a later retry. Performing Notion first could
publish an entry even when the local source-of-truth write fails.

| Outcome | Required behavior |
|---|---|
| Local write fails | Do not call Notion; preserve the editor contents and show the filesystem error. |
| Local succeeds, Notion definitely fails | Keep the local file; mark Notion as pending and offer a later retry. |
| Local succeeds, Notion page creation or entry append is ambiguous | Read the relevant parent or month page and verify before any retry. |
| Both succeed and verify | Report both destinations and retain the Notion page and block IDs. |

The owner-confirmed Phase 1 locations and state contract are recorded below.

## Source data must remain read-only

Input files are synchronized by another application through iCloud. The TUI
should not rewrite, rename, move, tag, or delete them. This avoids sync
conflicts, preserves provenance, and keeps retry behavior simple.

If a source file changes during a run, compare its size and modification time
after reading or retain a content digest. Warn before producing output from a
stale snapshot. The owner requires a stop and successful reread/reconfirmation
when freshness cannot be verified at a workflow checkpoint.

## Sensitive-data boundaries

The copied examples demonstrate that the input can contain:

- Diary prose and voice recordings.
- Precise latitude and longitude.
- A human-readable place name.
- Timestamps, weather, tags, and transcription metadata.
- A Notion credential with permission to modify workspace content.

Required safeguards for later implementation:

- Default to no telemetry and no content-bearing logs.
- Redact request bodies, response page content, tokens, coordinates, and diary
  text from errors.
- Keep the Notion token out of process arguments, logs, and the SQLite database.
  The owner chose a plaintext, owner-only config file for this personal MVP:
  the repo-local, git-ignored `config/global_config.yaml`, which superseded an
  earlier per-user global location.
- Send only user-confirmed text to Notion.
- Never send source metadata unless that behavior is separately approved and
  visible to the user. Phase 1 never sends Ayumi audio or attachment paths.
- Use synthetic fixtures in tests.

The real `tmp/Ayumi/` fixture is covered by the `tmp/` entry in `.gitignore`.
It must not be staged or committed.

## Date selection affects both outputs

For Ayumi's Diary input, the owner decided that both the `YYYY/MM/DD` directory
date and the offset-aware Markdown-header `created_at` date propose membership
in a diary day. The TUI must expose disagreements and allow the user to include
or exclude discovered source notes regardless of either metadata date. The
confirmed selection is the only source set used for composition.

The diary day chosen in the workflow must still be one explicit value shared
by:

- Input filtering.
- The local output filename and any front matter.
- The Notion section heading.
- Duplicate detection and retry state.

Including a note from another metadata day does not relabel that note and does
not change the diary day's output identity. Choosing different day identities
for different sinks would make one diary appear under two days. See
[Markdown input format research](markdown-input-format.md).

## iCloud availability and permission failures

Input discovery can encounter a directory that does not exist, is not
authorized, is still synchronizing, or contains cloud-only placeholders. These
are different user actions and should not collapse into "no entries":

| Condition | User-facing distinction |
|---|---|
| iCloud Drive root absent | iCloud Drive is unavailable or not configured. |
| Configured directory absent | The configured directory name is wrong or not synchronized. |
| Permission denied | macOS privacy access must be granted to the terminal or app. |
| File not resident | Download from iCloud is pending or required. |
| Invalid Markdown/front matter | Identify the affected relative path without printing content. |

## Notion is an external side effect

The TUI should show the exact date and text that will be sent and require a
specific confirmation before the first write. A retry after a known pending
failure may reuse that confirmation only if the content is unchanged.

The application should never replace the configured root or a year/month page
as a fallback. It creates missing subpages and appends only to the identified
month page. See
[Notion daily diary section research](notion-diary-section.md).

## P1-M1 owner decisions for output and state

The user-visible diary root defaults to
`$HOME/Documents/narihitos-diary/entries/` and is configurable. An entry is
`YYYY/MM/YYYY-MM-DD.md`, containing YAML front matter with diary day and
writing language followed by the full confirmed Markdown body. A
same-path/same-digest entry resumes; different content is never overwritten
automatically and must be reopened for explicit resolution.

Private operational state lives in a per-user SQLite database under
`$HOME/Library/Application Support/narihitos-diary/`. It stores unfinished
drafts for recovery and versioned per-day publication receipts with local
content digest, selected-source IDs and digests, Notion status, confirmed
Notion payload digest, root/year/month page IDs, returned block IDs, and
timestamps. It stores no token,
coordinates, or attachment path. The Notion token lives in the repo-local,
git-ignored `config/global_config.yaml` with owner-only permissions.

Composition uses two-pass drafting through a hosted default adapter (provider
undecided) or an optional Apple on-device adapter, and always provides a
multiline manual editor. Readable note bodies, memories, drafts, and the edited
entry may leave the Mac; front matter, attachment paths, audio, and credentials
never do (owner decision, superseding the earlier on-device-only rule).
Notion receives the confirmed Markdown body without YAML front matter under a
`heading_2` ISO date. The configured root contains `YYYY` year subpages;
each year contains `YYYY-MM` month subpages. Missing pages are created during
publication. New daily sections are appended at the bottom of their month
page. A same-day section is
accepted only when the local receipt and a
read-back match; otherwise publication stops for user resolution.

The non-sandboxed Phase 1 TUI uses the configured iCloud path plus a native
Foundation availability/coordinated-read adapter. Folder pickers and
security-scoped bookmarks remain requirements only for a future sandboxed
package. See [P1-M1 validation evidence](p1-m1-validation-evidence.md).

