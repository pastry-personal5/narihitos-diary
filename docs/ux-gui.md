# UX GUI

Status: Proposal

This document separates approved terminal behavior from proposed presentation
details. Product scope is in [product-behavior.md](product-behavior.md), and
component vocabulary is in [ux-terms.md](ux-terms.md).

## Current interface

There is no production TUI yet. This document describes the intended client
experience; it does not claim an implemented screen or keybinding.

## Approved behavior to represent

- Show directory and header dates together when choosing source notes, visibly
  mark disagreements and malformed metadata, and allow inclusion or exclusion
  regardless of either date.
- Request downloads only for selected cloud-only notes. Keep unavailable and
  unreadable states distinct from an empty note.
- Offer a multiline manual editor at all times after source confirmation,
  including when Apple model assistance is unavailable or fails. Optional
  assistance uses a source-linked memory review before drafting.
- Offer US English and South Korean Korean per entry, remember the last choice,
  and offer concise, standard, and reflective styles with concise as default.
  Mark model-inferred reflections for human review.
- Require confirmation of the complete edited entry and then show the exact
  local and Notion outputs before the first write. Display the root/year/month
  path and any subpages that would be created. Show conflicts and partial
  success plainly.
- Display the macOS input language through Text Input Source Services
  notifications, use a 500 ms polling fallback when notification registration
  fails, and show `IME: ?` when the language is unknown.

## Proposed presentation details

The source list should place inclusion state, note identity, directory date,
header date, availability, and warning status in a stable row. A detail area
can show the readable text without audio links. The chosen diary day stays
visible while inspecting another date. A selected cloud-only row offers a
download action and progress state; the person confirms the selection only
after its readable snapshot has been reviewed.

The editor should keep language and style choices visible and allow paste,
multiline editing, and review of source-linked memories. An inferred reflection
needs a visible marker and an explicit accept, edit, or remove choice. The
entry review should show the entire edited text, not a summary. If freshness
fails or another client changes the workflow, the screen should explain what
must be reread and preserve recoverable edits without treating them as
confirmed.

The output preview should show the local file path and rendered Markdown,
followed by the Notion destination, heading, and exact body. It should separate
read-only preflight results from actions that create pages or append text. The
publication screen should distinguish local success, Notion pending, verified
success, and a conflict that needs user resolution. Connection loss should
disable publication until the client reloads the server's current workflow.

Ratatui views render the ViewModel; the client sends actions to the Control
API Server and receives state changes through REST and events. Layout and
keybindings remain open implementation details. See the
[information architecture](ux-information-architecture.md) for screen order.

