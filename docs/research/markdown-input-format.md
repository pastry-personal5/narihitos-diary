# Markdown input format research

Status: Active

Research date: 2026-10-28

This document records the input layout and file format observed in the copied
`tmp/Ayumi/` examples. It describes evidence and implementation constraints;
it is not a phase or milestone plan. No diary prose or metadata values are
reproduced here.

## Confirmed input root

The MVP input root is:

```text
$HOME/Library/Mobile Documents/com~apple~CloudDocs/<Configured Directory Name>/
```

For the supplied examples, `<Configured Directory Name>` is `Ayumi` and the
equivalent repository fixture root is `tmp/Ayumi/`.

`<Configured Directory Name>` should be treated as one directory name, not an
arbitrary path. Reject empty names, `.` and `..`, path separators, absolute
paths, and names whose resolved path escapes the iCloud Drive root.

## Observed directory layout

Both supplied entries follow this shape:

```text
<Configured Directory Name>/
└── YYYY/
    └── MM/
        └── DD/
            ├── YYYY-MM-DD_HH-MM-SS-mmm.md
            └── YYYY-MM-DD_HH-MM-SS-mmm.assets/
                └── <identifier>.m4a
```

Findings from the two examples:

- Both Markdown files are under `2026/10/28/`.
- Each Markdown filename matches `YYYY-MM-DD_HH-MM-SS-mmm.md`.
- Each entry has a sibling `.assets` directory derived from the Markdown stem.
- Each Markdown file links to one existing `.m4a` file in that directory.
- `.DS_Store` files occur at several directory levels and are not input.
- Enumeration should include regular files ending in `.md`, ignore hidden
  metadata files, and not follow symbolic links by default.

This is a two-file sample, so the layout is observed rather than proven to be
universal. Discovery should report malformed or unexpected entries without
aborting every valid entry.

## Text and Markdown format

Both files are UTF-8, use LF line endings, and have no UTF-8 byte-order mark.
They contain:

1. YAML front matter delimited by `---` lines.
2. Plain paragraph lines separated by blank lines.
3. A normal relative Markdown link to an audio attachment.

The examples contain no headings, lists, tasks, fenced code, blockquotes, or
Markdown image syntax. A representative structure is:

```markdown
---
id: <string>
created_at: <ISO-8601 string>
updated_at: <ISO-8601 string>
tags: []
favorite: <boolean>
weather_summary: <string>
weather_temperature_celsius: <number>
location_latitude: <number>
location_longitude: <number>
location_place_name: <string>
transcription_model: <string>
transcription_prompt: <string>
---

<paragraph>

<paragraph>

[<label>](YYYY-MM-DD_HH-MM-SS-mmm.assets/<identifier>.m4a)
```

### Front-matter fields

| Field | Observed type | Present in both | Notes |
|---|---|---:|---|
| `id` | string | Yes | Treat as opaque; do not infer structure from it. |
| `created_at` | ISO-8601 string | Yes | Includes fractional seconds and a numeric UTC offset. |
| `updated_at` | ISO-8601 string | Yes | Same format as `created_at`; equal to it in both examples. |
| `tags` | array of strings | Yes | Empty in both examples. |
| `favorite` | boolean | Yes | YAML boolean. |
| `weather_summary` | string | Yes | Potentially personal context. |
| `weather_temperature_celsius` | number | Yes | Floating-point in both examples. |
| `location_latitude` | number | Yes | Precise location data. |
| `location_longitude` | number | Yes | Precise location data. |
| `location_place_name` | string | Yes | Human-readable location data. |
| `transcription_model` | string | Yes | Describes how body text was produced. |
| `transcription_prompt` | string | Yes | Describes transcription configuration. |

The format reader should use typed deserialization for known fields while
tolerating additional fields. A missing or invalid required field should become
a per-file diagnostic, not a panic. The raw body should remain available even
when optional metadata cannot be parsed.

## Time semantics

The two samples agree on a non-obvious time relationship:

- The front-matter timestamps carry offset `+09:00`.
- The date directories match the date portion of the filename.
- In both examples, `created_at` is exactly nine hours later than the filename
  timestamp when both are compared using the `+09:00` offset.

The best current inference is that filenames and directories encode UTC while
`created_at` records local time. Two examples are not enough to make that a
format guarantee. Around local midnight, the directory date may therefore
differ from the user's intended diary date. Preserve the original offset and
parse `created_at` as an offset-aware timestamp.

### Decided diary-day behavior

For Ayumi's Diary input, the `YYYY/MM/DD` directory date and the local calendar
date of the offset-aware front-matter `created_at` value are both day metadata.
The application uses both to propose source notes for the diary day; neither
silently overrides the other.

The TUI must show a date disagreement and let the user include or exclude any
discovered source note regardless of either metadata date. The confirmed
selection, not an automatic date rule, determines which source bodies are used
to compose the entry. Original metadata remains unchanged. Automatic discovery
uses `D-1`, `D`, and `D+1`; an explicit date jump loads one other day at a time,
as specified below.

## Audio attachments

The linked attachments are M4A containers with mono AAC audio at 44.1 kHz. The
two samples are approximately 4.9 seconds and 33.7 seconds long. The Markdown
body already includes transcription-related metadata and prose, so audio
decoding is not demonstrably required for the MVP.

Phase 1 ignores audio attachments while reading and composing. It does not
play, copy, upload, decode, transcribe, or use audio for composition. The one
write-side exception is explicit source deletion: safely resolved linked audio
is deleted by path together with its Ayumi note. Audio attachment paths are
also excluded from composition input and final output. An entry remains eligible
whether it has no attachment, one attachment, or multiple attachments; the
application can recognize an audio link in order to omit it but must not open
the linked file.

## iCloud Drive behavior on macOS

The literal `Mobile Documents/com~apple~CloudDocs` path is an MVP convention,
not Apple's public container-discovery API. A non-sandboxed TUI can attempt a
normal read there, but three macOS behaviors matter:

- Optimize Mac Storage can leave some iCloud Drive files without a resident
  local copy. Apple exposes download status and an explicit download request
  through Foundation's ubiquitous-item APIs.
- macOS privacy controls protect iCloud Drive. The terminal host or packaged
  application may need user consent under Privacy & Security > Files and
  Folders.
- A future sandboxed macOS application should obtain user-selected directory
  access and persist a security-scoped bookmark instead of relying on this raw
  path.

For the initial TUI, a clear "file is not downloaded or permission was denied"
diagnostic is preferable to silently skipping an entry. Foundation integration
to request downloads can be evaluated only if ordinary reads prove unreliable.

## Privacy and fixture handling

These files contain diary prose, precise coordinates, a place name, weather,
transcription settings, and voice recordings. Treat the whole input tree as
sensitive:

- Do not log file contents, front-matter values, coordinates, tokens, or audio.
- Logs may contain a redacted relative path and a stable error category.
- Keep source files read-only; the MVP output belongs elsewhere.
- Never commit `tmp/Ayumi/` or derived fixtures containing real content.
- Tests should use synthetic entries with invented locations and recordings.

The repository currently does not ignore `tmp/`; add an ignore rule before any
commit that might otherwise include the copied examples.

## P1-M1 owner decisions

The owner confirmed that automatic discovery reads only the `D-1`, `D`, and
`D+1` date directories for the selected diary day. A date picker can load one
other explicit day at a time; the application never scans the whole archive
merely to populate selection.

The owner confirmed that missing or malformed front matter does not make a note
ineligible when its Markdown body is readable. The TUI keeps it selectable,
shows a warning, and uses only valid date metadata for proposals.

The owner also confirmed that unknown front-matter fields are ignored and
missing non-date fields are tolerated. Only a valid offset-aware `created_at`
contributes the header date; otherwise the TUI warns and retains the directory
date.

The owner confirmed that UTF-8 is the only supported source encoding, with or
without a BOM and with LF or CRLF line endings. A file that cannot be decoded as
UTF-8 receives a warning and is not treated as having a readable body. Body
Markdown is otherwise preserved apart from supported standalone audio-link
nodes; ambiguous attachment syntax is diagnosed rather than passed through.

Discovery ignores hidden entries and symlinks and accepts regular `.md` files.
The owner confirmed best-effort freshness checks rather than continuous
monitoring: compare file identity, size, modification time, and a retained
content digest at workflow checkpoints. A detected change invalidates the
confirmed selection snapshot, generated draft, and output previews until the
source is reread and reconfirmed. If freshness cannot be verified, output stops
until a successful reread and reconfirmation.

Ordinary reads alone do not distinguish permission loss, a cloud-only item,
and a malformed file well enough. Phase 1 therefore uses a small Foundation
adapter to query ubiquitous-item status, request a selected cloud-only item's
download, and coordinate the read. It never treats an unavailable file as an
empty note. Synthetic cases and the complete state contract are in
[P1-M1 validation evidence](p1-m1-validation-evidence.md).

## Sources

- Local inspection of the two files and two audio attachments under
  `tmp/Ayumi/` on 2026-10-28.
- [Apple: Store files in iCloud Drive on Mac](https://support.apple.com/guide/mac-help/store-files-in-icloud-drive-mchle5a61431/mac)
- [Apple: Start downloading a ubiquitous item](https://developer.apple.com/documentation/foundation/filemanager/startdownloadingubiquitousitem(at:))
- [Apple: Ubiquitous-item download status](https://developer.apple.com/documentation/foundation/urlresourcevalues/ubiquitousitemdownloadingstatus)
- [Apple: Controlling app access to files in macOS](https://support.apple.com/guide/security/controlling-app-access-to-files-secddd1d86a6/web)
- [Apple: Accessing files from the macOS App Sandbox](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox)

