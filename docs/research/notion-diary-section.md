# Notion daily diary section research

Status: Active

Research date: 2026-10-28

This document evaluates how the macOS TUI can add one daily diary section to
a month subpage beneath the configured Notion root page. It records API
behavior and the owner publication contract, not an implementation plan.

## Target page

Configured URL:

```text
[REDACTED NOTION PAGE URL]
```

The 32 hexadecimal characters normalize to page UUID:

```text
[REDACTED NOTION PAGE ID]
```

Notion documents that page URLs can change format while record UUIDs remain
stable. Accepting the supplied link once and persisting the normalized page ID
is safer than depending on the URL shape for every request.

The root page itself was not inspected: the public URL was not accessible without
Notion authorization, and no credential was available or used. Its current
block layout, title, permissions, and existing daily sections remain unknown.
This is retained as an explicit compatibility unknown, not filled in from
guesswork. First-run read-only preflight must inspect and paginate the root
and any existing target year/month pages before enabling publication.

## Owner-confirmed page hierarchy

The configured page is a root, not the daily-entry destination. For diary day
`2026-02-14`, the path is:

```text
<configured root page>
└── 2026
    └── 2026-02
        └── heading_2: 2026-02-14
            confirmed entry body follows as paragraph blocks
```

Use the workflow's diary day to choose the `YYYY` year and `YYYY-MM` month.
Create a missing year page under the root and a missing month page under that
year when publishing. Reuse an existing unambiguous path. Append each new
daily section at the bottom of the month page; check for the same-day heading
there. A duplicate or ambiguous year/month page title requires resolution
before a write. The TUI previews the root, year, and month destination and
any pages that publication will create.

## Authentication for a personal TUI

As of this research date, Notion explicitly recommends a personal access token
(PAT) for a local command-line tool that acts as one user in that user's own
workspace. A PAT uses the creator's existing page permissions and does not
require adding a bot connection to each page.

For this personal MVP:

- Create a dedicated PAT with the Notion API capability and an expiry.
- Store the token in the repo-local, git-ignored `config/global_config.yaml`,
  which must be owner-only, as the owner chose for this personal MVP. Never
  commit that file or copy the token into logs, process arguments, SQLite, or TUI dumps.
- Keep the page ID in ordinary configuration; it is an identifier, not a
  secret.
- Surface `401` as an expired/revoked token and `403` as insufficient
  capability. A `404` can also mean that the credential cannot access the page.

An internal connection remains a valid alternative for a team-owned workflow,
but it is a separate bot identity, must be granted page access, and requires a
workspace owner to create it. OAuth is unnecessary unless this becomes a
multi-user product.

All REST requests must include a pinned `Notion-Version` header. The current
latest version is `2026-03-11`; pin it rather than silently adopting a future
breaking version.

## Two APIs can modify page content

### Block API

Notion represents a page body as child blocks. The relevant operations are:

- `GET /v1/blocks/{page_id}/children` to inspect top-level children. Results are
  paginated, with at most 100 per response.
- `PATCH /v1/blocks/{page_id}/children` to append a heading and paragraph
  blocks. In API version `2026-03-11`, `position: {"type":"end"}` explicitly
  appends at the end.

A daily section can be represented by one `heading_2` block containing a
canonical local date, followed by one or more paragraph blocks. This closely
matches the requested "section" and changes only newly appended blocks.

### Markdown content API

Notion also supports reading and updating a page as Markdown:

- `GET /v1/pages/{page_id}/markdown`
- `PATCH /v1/pages/{page_id}/markdown`

The endpoint can perform exact search-and-replace with `update_content` or
replace the complete body with `replace_content`. Its older `insert_content`
command can append Markdown at the end, but Notion now labels that command
legacy and recommends the more precise update commands for new connections.

For this use case, `update_content` would require a unique, permanent anchor in
the page, while `replace_content` rewrites too much and risks disturbing manual
edits, child pages, or databases. The Markdown endpoint is attractive if the
page is deliberately prepared with a diary insertion marker, but that page
structure has not been confirmed.

## Researched API direction and owner ordering requirement

The block API is the stronger current candidate:

1. Read and paginate the root's child pages, then the target year's child
   pages. Reuse an unambiguous year/month path or create missing pages.
2. Read and paginate the month page's blocks for the diary-date heading.
3. If absent, append the heading and paragraph blocks at the month page's end.
4. Read back the created blocks before reporting success.

### Amending an owned section

An amendment replaces a section the app published and receipted. The block API
(checked against the current reference, 2026-10-28) supports it as follows:

- `PATCH /v1/blocks/{block_id}/children` accepts `position` with type `after_block`
  (and `start` or `end`) to insert after a given block; the older `after`
  parameter is deprecated, and the two cannot be combined. A request takes at
  most 100 children, and appended blocks cannot be moved afterward.
- `DELETE /v1/blocks/{block_id}` sets `in_trash`, which moves the block to the
  trash where it can be restored through the update-block or update-page
  endpoints. It does not destroy content.
- `PATCH /v1/blocks/{block_id}` can edit a text block's content in place, but
  it cannot change a block's children and does not help when the paragraph count
  changes.

The chosen method inserts the new paragraphs after the receipted heading block,
reads them back, and only then trashes the old paragraph blocks, so an
interruption duplicates content instead of losing it. See
[Amendments](../planned-mvp-architecture.md#amendments) and the references
[append block children](https://developers.notion.com/reference/patch-block-children),
[delete a block](https://developers.notion.com/reference/delete-a-block), and
[update a block](https://developers.notion.com/reference/update-a-block).

Reasons:

- It does not rewrite existing page content.
- It directly models a heading plus paragraphs.
- It supports bottom-of-month placement with an end append.
- It provides block IDs that can be retained for later verification.

The block API remains a research candidate. Creating year/month subpages must
be validated with the chosen Notion page API during connector design. An
uncertain page creation requires rereading the parent before retrying, to avoid
duplicate subpages. This does not approve a Rust dependency.

## Duplicate prevention and ambiguous writes

Notion writes are not generally safe to repeat blindly. A connection can time
out after a write was committed, and retrying an append can create duplicates.
Notion's current error guidance says to inspect `additional_data` on a `503`;
for committed block appends it can include the parent and created child IDs.

A duplicate-aware write should therefore:

- Use one canonical date heading, based on the diary's decided local date.
- Check for that heading in the target month page before appending.
- If the heading exists without a matching local receipt and read-back, stop
  for user resolution; do not silently update or duplicate it.
- After a network timeout or ambiguous `5xx`, read the page before retrying.
- Compare both the date heading and intended paragraph content when confirming
  whether the write landed.
- Persist returned block IDs locally after a confirmed success.

For `429` and `529`, respect `Retry-After`, apply bounded backoff with jitter,
and keep the TUI responsive. Do not retry authentication, permission, or
validation errors without user action.

## Content and size limits

The current API limits relevant to diary text include:

- 2,000 characters per rich-text `text.content` value.
- 100 elements in an array of block or rich-text objects.
- 1,000 block elements and 500 KB for an entire request payload.
- 100 child blocks per page of list results.

Long diary paragraphs must be split on Unicode character boundaries into
multiple rich-text objects or paragraph blocks. Do not truncate silently.

Each month page receives only its own daily sections. This bounds the number
of daily headings per page while preserving the configured root as the stable
entry point.

## Phase 1 owner-confirmed publication contract

Notion receives the confirmed Markdown body after local YAML front matter is
removed. The TUI shows the root/year/month path, any subpages to create, diary
day, heading, and paragraphs before the first write. There is no separate
sentence-selection mode in Phase 1.

The section representation is:

```text
heading_2: YYYY-MM-DD
paragraph: <sentence or paragraph>
paragraph: <sentence or paragraph>
...
```

The heading is the workflow's canonical local diary day in ISO form. New
daily sections are appended at the bottom of the target month page. The body is
mapped to paragraph blocks,
splitting rich text and requests at documented limits without truncation.

A matching local receipt plus matching month page ID, block IDs, heading, and content is
already published. A same-day heading without that match is ambiguous: Phase 1
stops and requires user resolution instead of updating or duplicating it. After
an ambiguous write, paginate and read before retrying. `429` and `529` respect
`Retry-After`; non-idempotent writes are never blindly replayed.

The owner deferred inspection of the configured root to the runtime preflight.
Publication remains disabled if permissions, page hierarchy, or same-day
content cannot be reconciled. Equivalent-fixture cases are
recorded in [P1-M1 validation evidence](p1-m1-validation-evidence.md).

The hierarchy fixture covers missing and existing year/month pages, pagination,
duplicate or incompatible paths, and lost page-creation responses. It is
synthetic evidence, not proof of the configured page's compatibility or of a
live create request. The runtime preflight remains mandatory, and connector
acceptance must exercise page creation on a disposable Notion hierarchy.

## Sources

- [Notion: Personal access tokens](https://developers.notion.com/guides/get-started/personal-access-tokens)
- [Notion: Internal connections](https://developers.notion.com/guides/get-started/internal-connections)
- [Notion: API versioning](https://developers.notion.com/reference/versioning)
- [Notion: Working with page content](https://developers.notion.com/guides/data-apis/working-with-page-content)
- [Notion: Create a page](https://developers.notion.com/reference/post-page)
- [Notion: Create a subpage](https://www.notion.com/help/create-a-subpage)
- [Notion: Append block children](https://developers.notion.com/reference/patch-block-children)
- [Notion: Working with Markdown content](https://developers.notion.com/guides/data-apis/working-with-markdown-content)
- [Notion: Update page content as Markdown](https://developers.notion.com/reference/update-page-markdown)
- [Notion: Request limits and retry behavior](https://developers.notion.com/reference/request-limits)
- [Notion: Connection capabilities](https://developers.notion.com/reference/capabilities)
- [Notion: Secure API credentials](https://developers.notion.com/guides/get-started/handling-api-keys)
- [Apple: Keychain services](https://developer.apple.com/documentation/security/keychain-services)

