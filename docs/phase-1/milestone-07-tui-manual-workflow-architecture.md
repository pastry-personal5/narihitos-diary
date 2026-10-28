# P1-M7: TUI and manual workflow architecture plan

Status: Planned

This plan connects the approved terminal information architecture to the
Control API without creating a second workflow authority in the client.

## Affected modules and interfaces

- `diary-tui`: terminal guard, typed API client, SSE worker, ViewModels, views,
  editor buffer, Command Line mapping, and IME adapter.
- `diary-control-contract`: representations needed by the manual workflow.
- `apps/nd`: default TUI startup and terminal error handling.
- `diary-control`: any missing manual-flow route wiring, not policy.

## Approach and sequence

1. Establish terminal ownership and restoration, then attach through the
   runtime file and negotiate contract/build versions before sending a token.
2. Implement the Standard Layout shell, connection states, Status Line, focus
   model, terminal-size fallback, and non-blocking API/event workers.
3. Add Start and source-selection ViewModels over authoritative workflow DTOs,
   including selected-only jobs and explicit empty-selection confirmation.
4. Add the multiline editor with in-memory unsent text, debounced conditional
   saves, recovery, full entry review, and confirmation guards.
5. Implement accessible focus actions, safe Ctrl chords, Hangul-to-ASCII command
   mapping, display-width handling, and the TIS run-loop/polling adapter.
6. Test ViewModels and rendering with fakes, then record the required real
   Terminal and Korean IME acceptance evidence.

The event/render loop never performs HTTP, filesystem, SQLite, helper, or model
work. On reconnect it reloads server state before re-enabling mutations.
