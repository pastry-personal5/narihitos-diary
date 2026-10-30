# P1-M1 validation evidence

Status: Active

Research date: 2026-10-28

This note records the synthetic scenarios and disposable platform probes used
to close P1-M1. It contains no real diary prose, location, attachment path,
credential, Notion page identifier, or other private fixture value.

## Test environment

| Item | Observed value |
|---|---|
| macOS | 26.7.1 (25G241), Apple Silicon |
| Terminal | Apple Terminal for the live input-source probe |
| Xcode | 27.1 (27A9275) |
| Rust used for probes | 1.98.0 |
| Declared minimum tested by the disposable Cargo fixture | 1.89 |

The results are evidence for the selected baseline, not a claim that every
future macOS update behaves identically.

## Source and selection scenarios

The following invented cases were reviewed against the selection contract.
`D` is the diary day chosen by the user.

| Case | Directory date | Header date | Expected state and behavior |
|---|---|---|---|
| Exact match | `D` | `D` | Proposed, shown as matching, initially included. |
| UTC/local boundary | `D-1` | `D` | Proposed, disagreement shown, initially included. |
| Reverse boundary | `D` | `D+1` | Proposed, disagreement shown, initially included. |
| Missing header | `D` | Missing | Proposed from the directory date with a metadata diagnostic. |
| Extra front matter | `D` | `D` | Proposed; unknown keys are ignored and never sent to composition. |
| Malformed front matter | `D` | Invalid | Listed with a diagnostic; readable body may be manually included using the directory date. |
| Manual other day | `D-7` | `D-7` | Not loaded automatically; a direct date jump loads only that date and permits inclusion. |
| Attachment variants | `D` | `D` | Zero, one, and several standalone audio links produce the same eligibility; link nodes and targets are omitted. |
| Changes during read | `D` | `D` | A changed size, modification time, or digest invalidates the snapshot and requires reread and reconfirmation. |

Automatic discovery is limited to the three directories for `D-1`, `D`, and
`D+1`. This covers a calendar-date shift caused by an offset-aware timestamp
without scanning the archive. A date picker can load one additional explicit
day at a time. Discovery accepts regular `.md` files only, ignores hidden files
and symlinks, and reports per-file errors without discarding valid neighbors.

The accepted text envelope is UTF-8, with an optional UTF-8 BOM and LF or CRLF
line endings. YAML front matter is optional for eligibility. When present,
`created_at` must be an offset-aware timestamp to contribute a header date;
unknown keys and missing non-date fields are tolerated. The body is retained as
Markdown text, but only supported standalone audio-link nodes are removed; an
unsupported or ambiguous link shape is diagnosed instead of risking disclosure
of an attachment target.

The confirmed source IDs and their content digests form an immutable selection
snapshot. Any later inclusion change, file change, or availability failure
invalidates the generated draft and both output previews.

## Composition comparison

All prompts used fictional notes about a walk, a workshop proposal, and
finishing an outline. No repository fixture or personal prose was used.

| Prototype | Observed result | Rubric outcome |
|---|---|---|
| Manual editor | The three facts can be copied and joined without inference. | Grounded and private, but requires all drafting work from the user. |
| Apple on-device, one-pass English | Returned all three events in order in about 1.27 seconds. | Grounded on this run; added only a generic heading. |
| Apple on-device, one-pass Japanese | Returned in about 2.39 seconds but invented a date, season, scenery, importance, another person's reaction, and a future plan. | Failed groundedness and unwanted-addition criteria. |
| Apple on-device, guided two-pass Japanese | Pass A reproduced one fact per source; Pass B used only those facts. Total about 2.09 seconds. | Passed the synthetic groundedness, must-keep, chronology, and privacy checks on this run. |

The model reported available on this Mac. A sandboxed generation failed with a
model-service error, while the same user-context executable worked outside that
research sandbox; the product must treat runtime generation errors as ordinary
fallback conditions. Apple's documented 4,096-token session limit requires
fresh, bounded extraction sessions and forbids silent truncation.

The evidence supports optional, on-device two-pass drafting with structured
extraction and a full manual editor. It does not support one-pass generation or
automatic acceptance. These probes ran on-device only; the owner later made a
hosted adapter the default, so hosted quality is not evidenced here.

## Output and Notion fixture scenarios

An equivalent in-memory page fixture was used because no Notion credential or
owner diary-page content was available to this research session. It modeled
paginated top-level blocks, canonical and conflicting headings, committed writes
whose responses were lost, content limits, and rate-limit responses.

The original fixture modeled a single destination page. After the owner chose
a root/year/month hierarchy, a separate disposable Python fixture modeled
paginated `child_page` results, `POST /v1/pages` with a page parent and title,
page identity read-back, and a response lost before or after a create commit.
It used only invented IDs and titles. Its twelve scenarios passed on 2026-10-28:

| Hierarchy scenario | Observed fixture result |
|---|---|
| No year or month page | Created one `2026` child under the root and one `2026-02` child under that year; a second resolution reused both without another create. |
| Existing year and month beyond the first child-list page | Paginated both parents and reused the existing page IDs without a create. |
| Existing year, missing month | Created only the month under the existing year. |
| Duplicate year or duplicate month title | Stopped before a create; no path was chosen arbitrarily. |
| Same-name child database or archived/mismatched page identity | Stopped before publication instead of treating it as the expected page. |
| Create committed but response lost | Reread the parent, found the unique child, and reused it without replaying the create. |
| Create did not commit and response was lost | Reread found no child, but publication stopped as unresolved; no immediate create was replayed. A later retry requires a fresh parent read. |
| Parent child-list read failed | Stopped before any create. |

The fixture validates the decision logic, not live Notion behavior. The
documented API shape supports creating a page with `parent.page_id` and a
`title` property; listing a page's block children returns `child_page` blocks
and can require pagination. Connector acceptance must still validate the
request/response mapping against a disposable Notion hierarchy and execute
the configured root's read-only preflight before any real publication.

| Scenario | Research fixture result; owner decision where applicable |
|---|---|
| Local path absent | Atomically create `YYYY/MM/YYYY-MM-DD.md`, then mark Notion pending. |
| Same local path, same digest | Resume the recorded state; do not rewrite or republish. |
| Same local path, different content | Refuse automatic overwrite; reopen for explicit user resolution. |
| No same-day Notion heading in the target month page | Append one `heading_2` and the confirmed body at the month page's end. P1-M10 later fixed the heading as `YYYY-MM-DD (DDD)` and selected a lossless native-block projection. |
| Same heading and recorded block IDs/digest | Treat a read-back match as already published. |
| Same heading without a matching receipt | Stop before writing and require user resolution; do not append a duplicate. |
| Timeout or ambiguous `5xx` | Paginate and read before retrying; match heading and intended text. |
| `429` or `529` | Respect `Retry-After`, bounded backoff, and cancellation. |
| Rich text over 2,000 characters | Split on Unicode boundaries without truncation. |
| More than 100 child blocks in a request | Chunk requests, retaining pending state until every chunk verifies. |

The actual configured root page's title and top-level shape remain deliberately
unobserved by owner decision, reaffirmed on 2026-10-28. First-run read-only
preflight must paginate the root and any existing target year/month pages,
show the full destination path, and refuse publication if access or same-day
state is ambiguous. No implementation may claim compatibility with an
uninspected page hierarchy.

## Platform probes

### TUI and toolchain

A disposable Cargo project set `rust-version = "1.89"` and combined Ratatui
0.30.2, Crossterm 0.29.0, `ratatui-textarea` 0.9.3, and
`security-framework` 3.7.0. Its two in-memory tests exercised Japanese
multiline paste, key-press filtering, and rendering. All targets compiled and
the resolved graph contained one Crossterm version. The fixture was not added
to this repository.

Terminal restoration still needs an interactive acceptance test during
implementation because an in-memory backend cannot prove restoration after a
real process panic. P1-M1 establishes that such tests need no diary fixture.

### Keychain

The macOS-specific `security-framework` API compiled in the disposable Rust
fixture. A uniquely named generic-password item was created, read back, and
removed in the user's login context; the probe refused to overwrite a
pre-existing item and verified absence after deletion. No real credential was
used or logged.

### Input-language status

The disposable Rust probe linked only Carbon/HIToolbox and Core Foundation. It
used `TISCopyCurrentKeyboardInputSource`,
`kTISPropertyInputSourceLanguages`, and the selected-source change
notification. It reported `en` for the active source and `ja` for installed
Japanese sources. Because Japanese was not enabled, the reversible live test
used the already-enabled Korean source and observed `en → ko → en`; both the
notification and 100 ms polling paths saw both changes without restarting.

Notifications arrived in the same 100 ms sampling interval as each change. A
30.82-second idle run of the 100 ms polling upper-bound used 0.03 seconds of
user CPU and 0.07 seconds of system CPU. No Accessibility, Input Monitoring, or
Automation permission was requested. A terminal-hosted AppKit probe returned
no current `NSTextInputContext`, making AppKit unsuitable as the primary source.

## Sources

- [Markdown input format research](markdown-input-format.md)
- [Agent-assisted diary composition research](agent-assisted-diary-composition.md)
- [Notion daily diary section research](notion-diary-section.md)
- [Rust TUI stack research](rust-tui-stack.md)
- [Apple: SystemLanguageModel](https://developer.apple.com/documentation/foundationmodels/systemlanguagemodel)
- [Apple: generating with Foundation Models](https://developer.apple.com/documentation/FoundationModels/generating-content-and-performing-tasks-with-foundation-models)
- [Apple: NSTextInputContext](https://developer.apple.com/documentation/appkit/nstextinputcontext)
- [Apple: iCloud download status](https://developer.apple.com/documentation/foundation/urlresourcekey/ubiquitousitemdownloadingstatuskey)
- [Apple: generic password items](https://developer.apple.com/documentation/security/ksecclassgenericpassword)
- [Notion: retrieve block children](https://developers.notion.com/reference/get-block-children)
- [Notion: working with page content and page-parent creation](https://developers.notion.com/guides/data-apis/working-with-page-content)
- [Notion: request limits](https://developers.notion.com/reference/request-limits)
- [Notion: versioning](https://developers.notion.com/reference/versioning)
- [Rust release notes](https://doc.rust-lang.org/releases.html#version-1890-2025-08-07)
