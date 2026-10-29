# P1-M5: Source ingestion and Apple bridge architecture plan

Status: Done

This plan implements `SourceCatalog` and `SourceReader` against Ayumi files and
the P1-M3 Swift-helper envelope. P1-M4 keeps selection, confirmation, and
invalidation policy; the adapters provide evidence and fresh, sanitized bytes.

## Implemented shape

- `diary-core` exposes connector-neutral `SourceCatalog` and `SourceReader`
  ports, typed availability/read/access outcomes, and a checkpoint runner that
  revalidates the exact confirmed snapshots through the existing invalidation
  policy. SQLite schema v3 persists candidate availability and upgrades both
  prior M4 schema versions forward.
- `AyumiCatalog` validates one configured directory component, discovers only
  the requested date set, parses the shared Markdown envelope, and never reads
  a cloud-only candidate body. `AyumiSource` combines it with the helper client
  for selected download, coordinated reads, new date evidence, and freshness.
- `AppleBridgeClient` keeps the v1 64 KiB frame, one-process-per-request model,
  request correlation, deterministic lookup, finite timeout/cancellation, and
  a configurable concurrency cap. It accepts only an owned mode-0600 regular
  file in an isolated mode-0700 request directory beneath the app's mode-0700
  handoff root and cleans successful, failed, cancelled, timed-out, and orphaned
  handoffs without racing concurrent reads.
- The Swift helper implements availability, bounded download wait, and
  `NSFileCoordinator` reads against iCloud Drive. It revalidates containment
  and regular-file identity, writes an opaque private handoff, and returns only
  content-free typed errors and source metadata. Protocol tests inject a fake
  service; Rust tests use fake executables and synthetic trees.

## Boundaries and deliverables

| Boundary | P1-M5 responsibility |
|---|---|
| `diary-connectors` | Ayumi catalog, Markdown envelope and audio filtering, source stamps, freshness checks, and a bounded helper client. |
| `helpers/apple-bridge` | Versioned iCloud availability, download, and coordinated-read commands, plus fake-based Swift tests. |
| `diary-core` | Narrow source outcomes or use-case wiring needed to consume the real adapters; no filesystem, Swift, or Markdown parser types. |
| `apps/nd` | Register source connectors and validate the configured directory name; production server startup remains P1-M6. |

The [source-port contract](../planned-mvp-architecture.md#domain-and-application-contracts)
and the [P1-M4 policy](milestone-04-core-workflow-persistence-architecture.md#core-contract)
are the inward boundary. Keep raw front matter, absolute paths, helper frames,
audio references, and platform errors out of core source bodies and assistant
requests. The TUI in P1-M7 renders the evidence and warnings returned here.

## Catalog and identity

1. Accept the configured Ayumi directory as one directory name under the
   documented iCloud root. Reject empty names, `.`, `..`, separators, absolute
   names, symlinks, and resolved escapes. Revalidate containment when a source
   reference is used; a stale or forged reference cannot select an arbitrary
   file.
2. For automatic discovery, enumerate only the three calendar directories
   `D-1`, `D`, and `D+1`. An explicit date request enumerates exactly that
   additional day; it is never an invitation to recursively scan the archive.
   Handle month and year boundaries, missing directories, duplicate requests,
   hidden entries, non-regular files, symlinks, and malformed filenames with
   deterministic results and per-file diagnostics. Accept regular `.md` files
   without treating the observed filename timestamp as a date authority.
3. Derive a stable connector-scoped `SourceRef` from the root-relative source
   location, independently of front-matter `id`, file content, or discovery
   order. Resolve it only inside the connector after root and file checks; do
   not expose the absolute path as a core or assistant field.
   P1-M12 later persists that already-sanitized relative path separately as the
   local source-note name used by the terminal client; the opaque command ID
   and assistant-data boundary remain unchanged.
4. Return the directory date for each candidate. For locally readable files,
   inspect bounded metadata to obtain a valid offset-aware `created_at` date,
   using the calendar date in that timestamp's own offset, not a conversion to
   the Mac time zone. Return both dates and a disagreement flag. Missing,
   malformed, or offset-free values produce a warning and no header date.
   Unknown fields and missing non-date fields do not disqualify the note.
5. Query availability without downloading or coordinating a body read for an
   unselected cloud-only note. Its header date can remain unknown until it is
   selected and read. If new date evidence appears after that read, return it
   for review before the person confirms the selection. Neither date silently
   decides inclusion; P1-M4 records the person's exact selected set.

## Markdown and data boundary

Read selected files as bounded original bytes. Compute the digest over those
bytes before removing a BOM, normalizing line endings for parsing, or removing
attachment links; a change to any byte is then visible to freshness checks.
Accept only valid UTF-8, with optional BOM and either LF or CRLF. Invalid UTF-8
is a warned unreadable outcome, never an empty body.

Parse front matter only when a leading `---` delimiter is present. A valid
closing delimiter separates metadata from body even if the YAML content is
malformed; keep the body and warn. With no front matter, the entire document is
body. If an opening delimiter has no trustworthy body boundary, keep the
candidate visible but do not pass possible metadata to composition as prose;
report an unreadable selected-source outcome until a safe reread or correction.
Likewise, malformed known date fields never become valid date evidence. The
catalog and reader should use the same parser so a post-selection read cannot
silently reinterpret a candidate.

Filter the readable Markdown body before it reaches a core snapshot. Remove
supported standalone links to local audio attachments without opening their
targets. Preserve surrounding prose and unrelated Markdown links. If syntax
could contain an attachment path but cannot be classified safely—including a
multi-link line, image, or reference definition—warn and withhold that selected
body rather than sending a path to composition. Test the filtered body as the
only text in source snapshots, persistence, and future assistant-shaped input;
the original-byte digest remains a separate stamp. Do not log front-matter
values, note content, audio paths, or private absolute paths. Tests use invented
prose and metadata only.

## Swift iCloud command group and byte handoff

Keep the P1-M3 one-request/one-response JSON envelope, version and request ID,
128-byte request-ID cap, 64 KiB frame cap, protocol-only standard output,
per-request helper lifetime, and deterministic helper lookup. Add availability,
request-download, and coordinated-read commands. The Rust client passes only a
validated requested source; the helper rechecks containment and file type at
use time. Availability may be queried for candidates; download and coordinated
read require selection. The download request must wait for a bounded,
observable resident outcome before the read, or report that the note still
needs download.

A note can exceed one JSON frame. For coordinated reads, use a private
temporary byte handoff in an isolated mode-0700 request directory beneath a
mode-0700 app-controlled root: the helper writes one bounded mode-0600 snapshot
and returns an opaque basename plus file identity, size, and modification time
in its JSON response. The Rust client accepts only a file in that request
directory, checks ownership, mode, link count, size, and opened-file identity,
reads bounded bytes, computes the digest, and removes the request directory
afterward. Isolation lets concurrent reads clean their own success, failure,
cancellation, or timeout without deleting another request's handoff. The helper
removes partial files on failure; the client removes recognized orphaned
handoffs on its next start. Choose and document a finite source-byte limit
before implementation; exceeding it is an explicit unreadable outcome. The
handoff is an implementation detail and never becomes a core source ref or
persisted path. This preserves the approved JSON envelope while allowing notes
larger than its frame cap.

The selected source-byte limit is 8 MiB. The direct synthetic-tree reader, the
Swift coordinated reader, and the Rust handoff client all enforce it.

Bound command input and output, read size, wait time, and concurrent helper
processes. Kill a timed-out or cancelled helper and clean its handoff. Map
missing binary, unsupported version, malformed or extra frame, crash, timeout,
cancellation, permission denial, cloud-only state, and oversize to distinct
content-free adapter outcomes. Translate those to existing core outcomes such
as `NeedsDownload`, `PermissionDenied`, `Unreadable`, and `Unverifiable` without
conflating failure with an empty readable note. Swift tests use fake file and
iCloud operations; Rust tests use a fake helper executable. Neither test opens
the private iCloud tree.

## Snapshots and freshness

For each selected note, retain its `SourceRef`, sanitized body, date evidence,
and a stamp of file identity, size, modification time, and original-byte
digest. After the coordinated read, check that identity and metadata still
describe the requested source. A mismatch yields `Changed`; an impossible
check yields `Unverifiable`. A note not yet resident is `NeedsDownload`, not a
zero-length snapshot. An empty but readable selected body remains distinct
from the zero-selected-sources path and from any read failure.

At the post-read, pre-drafting-pass, pre-entry-confirmation, pre-preview, and
pre-first-write checkpoints, revalidate the root-relative reference and obtain
a fresh coordinated read to compare all stamp fields. Same-size or same-mtime
edits still change the digest. Removal, permission loss, helper failure, or an
incomplete check yields `Unverifiable` unless a definite identity or content
change yields `Changed`; an adapter access error is applied as unverifiable
before its typed error is returned. The P1-M4 state machine then voids the
confirmed selection and downstream draft, preview, and approval, retaining user
edits as unconfirmed. Only a successful reread followed by human reconfirmation
restores a usable source snapshot. An amendment uses P1-M11's separate path and
does not perform these source checks.

## Implementation and verification sequence

1. Confirm P1-M3 and P1-M4 completion and inspect their implemented ports,
   protocol, and approved dependencies. Obtain approval before a new dependency,
   build tool, or change to the helper envelope.
2. Implement root validation, bounded catalog queries, stable references,
   availability, and date evidence. Test synthetic trees including year
   boundaries, unexpected entries, symlinks, and cloud-only candidates.
3. Implement the shared Markdown parser and body filter. Test malformed and
   absent front matter, invalid bytes, offset semantics, and attachment paths
   at the core-readable and assistant-shaped boundaries.
4. Add the Swift iCloud commands, private byte handoff, and Rust helper client.
   Test selected-only acquisition, helper protocol failures, cancellation,
   limits, and cleanup through Swift fakes and a fake executable.
5. Connect selected snapshots and freshness outcomes to the P1-M4 use cases.
   Test every checkpoint, changed versus unverifiable outcomes, invalidation,
   and reread plus reconfirmation without an HTTP server or TUI.
6. Update affected docs and the overview's acceptance table. Run focused tests
   while iterating and the full Rust and Swift gate once at the end. Keep
   P1-M5 `Planned` until milestone order permits implementation, and mark it
   `Done` only after the checklist and gate have evidence.
