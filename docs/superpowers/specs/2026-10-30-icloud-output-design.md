# iCloud output connector design

Status: Implemented (P1-M14). No SQLite migration was needed; amendment replacement of the mirror file is deferred to P1-M11.

Adds an iCloud Drive Markdown output that mirrors the local entry file. It is written automatically and quietly when configured, and it never blocks or changes the local or Notion outputs.

## Intent

- **Outcome:** each confirmed diary day also appears as a plain Markdown file in a configured iCloud Drive folder, so it syncs to the person's other devices without any extra step.
- **Owner decisions (2026-10-30):** file outputs are all Markdown; Notion is kept unchanged; the iCloud folder is a configured path written as plain files (no CloudKit, no Swift helper); the write is automatic and quiet when configured.
- **Success:** with `icloud_output` configured, approving the local plan produces a verified iCloud file with byte-identical content and no additional prompt. Without the setting, nothing changes.

## Design

### Destination model

- Add `DestinationKind::Mirror` (`"mirror"`) beside `Local` and `Remote` in `crates/diary-core/src/types.rs`.
- `Local` stays the durable source of truth. `Mirror` and `Remote` are "later destinations" that retry from the verified local file, as Notion does today.
- The destination is created only when `paths.icloud_entries_root` is configured; otherwise no `Mirror` target exists in any plan.

### Adapter

- Reuse `LocalOutput` (`crates/diary-connectors/src/local_output.rs`) with a different root. Rendering (`render_local_markdown`: YAML `date` and `language`, then body), layout (`YYYY/MM/YYYY-MM-DD.md`), the temp-file, flush and no-clobber finalization, identical-content resume, conflict, and read-back stay exactly as in P1-M9. The bytes equal the local file's.
- Add an iCloud preflight, run read-only before any write:
  - the root must resolve under `~/Library/Mobile Documents/` (for example `com~apple~CloudDocs/...`), with no `..` components and no symlink escape;
  - an iCloud placeholder (`.<name>.icloud`) for the target means the file is evicted or not yet downloaded: report `blocked` with a reason token and never treat it as missing or write over it;
  - an unreadable or missing root is `blocked`, not created silently outside iCloud.
- Different existing content at the target is a conflict and is never overwritten, as for local. An approved amendment replaces only the app's own receipted mirror file, guarded by the mirror's own receipt digest (as P1-M11 does for local).

### Approval and quiet behavior

- The mirror adds no approval step. The existing exact-plan approval covers it because its bytes equal the local plan's; the plan digest includes the mirror target so a changed config forces a new preview.
- Publication runs the mirror automatically after the local write is verified, in the same publication job, without a separate command or confirmation.
- Quiet means no extra UI when it works. The Output preview lists the mirror path in one line. Publication status shows a mirror row only when it is `pending`, `blocked`, or in `conflict`. A verified mirror collapses into the local row's "also saved to iCloud" note. Failures never raise a modal and never fail the local publication.

### Workflow and recovery

- The mirror gets its own intent, digest, receipt, and status. Gating, `Partial` and `Completed` computation, retries, cancellation, and startup reconciliation treat it like `Remote`: a retry re-enters `Publishing` for unfinished destinations only, and an uncertain mirror write is reconciled by inspecting the target before any retry.
- A mirror failure yields `Partial` for that day only; local and Notion proceed independently, and other days are unaffected.
- Logs and diagnostics stay content-free: a reason token and keyed tags only, never paths or diary text, per AGENTS.md "Control API diagnostics".

### Config

```yaml
paths:
  entries_root: "~/Documents/narihitos-diary/entries"
  # Optional. Omit to disable. Must be under ~/Library/Mobile Documents/.
  icloud_entries_root: "~/Library/Mobile Documents/com~apple~CloudDocs/narihitos-diary/entries"
```

Startup validation rejects a value that fails the preflight path rules, with a content-free reason.

### Persistence and contract

- SQLite migration for the destination-kind value `mirror`, with a round-trip test (`DestinationKind::parse`/`as_str`).
- Control API `/v1` adds the `mirror` destination kind. This is an additive, versioned contract change documented in `docs/control-api-v1.md`; clients that predate it must treat an unknown kind as non-blocking.

## Testing

Synthetic fixtures only, covering: byte-identical mirror and local output; identical-content resume; conflict on different content; evicted placeholder and unreadable root as `blocked`; path rules (outside iCloud, `..`, symlink escape); crash between write and read-back with startup reconciliation; mirror failure leaving local and Notion `Completed`; amendment replacement guarded by the mirror receipt; the quiet UI states (no row when verified, row when blocked or in conflict); config absent means no change in any existing test.

## Out of scope

- CloudKit, the Swift helper, and `NSFileCoordinator`.
- Making iCloud the source of truth, or any other peer-destination model.
- Changing the Notion format, the local layout, or the YAML keys.

## Docs to update with the code

`docs/product-behavior.md`, `docs/architecture.md` (Adding connectors, Publication and recovery), `docs/control-api-v1.md`, `docs/ux-tui.md`, `docs/user-guide-tui.md`, `README.md` (config), `docs/phase-1/changelog.md`, `docs/roadmap.md`, `docs/README.md`, and the AGENTS.md "Decided" index. A new milestone, P1-M14: iCloud output, carries the verifiable completion checklist.
