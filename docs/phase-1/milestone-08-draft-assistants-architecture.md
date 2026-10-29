# P1-M8: Draft assistants architecture plan

Status: Planned

This plan fills the `DraftAssistant` port with an extensible backend registry
only after its undecided hosted dependencies are owner-approved. It reserves a
safe, collection-shaped configuration for multiple Codex profiles without
prematurely choosing Codex as the production hosted adapter.

## Affected modules and interfaces

- `diary-core`: backend descriptors and selection, extraction, memory review,
  drafting, paragraph-review suggestions, validation, preferences, and
  freshness-check use cases bounded by `DraftAssistant`.
- `diary-connectors`: chosen hosted adapter and Swift helper client adapter.
- `diary-control`: strict global-configuration loading for zero or more named
  Codex profiles, without loading or copying credentials from those homes.
- `helpers/apple-bridge`: versioned Foundation Models command group.
- `diary-control` and contract: collection-shaped backend capability state,
  extraction/draft/review jobs, and versioned suggestion accept/reject.
- `diary-tui`: backend selection, Memory review, assisted-editor, source
  inspection, and paragraph-suggestion states inside Compose.

## Backend and suggestion boundaries

Startup registers `Arc<dyn DraftAssistant>` implementations by stable backend
ID. Core and transport consume a list of descriptors and resolve an ID for each
assistant command; they never switch on a provider enum. A descriptor reports
its display label, `hosted` or `apple-on-device` composition path, availability,
supported languages, and `extract-memories`, `draft-entry`, and
`review-paragraph` capabilities. Manual composition is a workflow action, not a
registry member.

Extraction and drafting receive only server-owned confirmed source snapshots.
The TUI submits neither source bodies nor arbitrary paths, and the active source
in the Resources Sidebar has no effect on the request. The server performs the
freshness checkpoint, resolves exactly one backend ID, constructs the allowed
request type, and records the backend ID on the result for display and recovery.
Source text is untrusted. The port offers no tools or callback by which an
adapter can read another source, run a shell command, call an output connector,
or perform a workflow transition. Agent-style adapters must disable those
capabilities in their own process/API configuration as well as omit them from
the Rust port.

A paragraph target is the current entry-buffer version, the UTF-8 byte range of
the Markdown paragraph containing the editor cursor (blank-line delimited and
shown before submission), and a digest of those exact bytes. The assistant
returns a proposed replacement plus an optional bounded explanation. The server
stores it as an `AssistantSuggestion`; accept conditionally applies it and
advances the buffer version, while reject marks or removes it without mutating
the entry. Any edit that changes the target makes the suggestion stale. Neither
operation confirms the complete entry.

P1-M12 already makes the breaking pre-client contract change: capabilities
report `manual_workflow_available` separately from `assistant_backends[]`,
writing settings carry an optional backend ID, and the backend descriptor owns
the hosted/on-device path. Its database migration preserves workflows and
language/style settings while mapping the old path preference to no selected
backend. P1-M8 populates the descriptor collection and adds assistant action
resources; it does not reintroduce manual as a path. Mixed-version
client/server pairs stop at version mismatch.

## Codex profile configuration

The planned global configuration shape is a collection, not a singleton:

```yaml
assistants:
  codex_profiles:
    - alias: "pp7"
      run_command: "CODEX_HOME=/Users/user1/.codex-pp7 codex"
    - alias: "work"
      run_command: "CODEX_HOME=/Users/user1/.codex-work codex"
```

`alias` is chosen by the user, shown in the TUI, and used to construct a stable
backend ID such as `codex.pp7`; it therefore follows the existing backend-ID
grammar and must be unique. Renaming an alias creates a new backend identity and
requires reselection. Config order is display order. `run_command` records the
corresponding Codex launcher, including its profile-specific `CODEX_HOME`.
Duplicate or invalid aliases and malformed commands fail strict configuration
parsing. A syntactically valid profile whose directory, executable, or Codex
authentication is unavailable remains a typed unavailable backend so the
manual editor and other backends still work.

The run command is configuration data but not an invitation to execute a shell.
M8 accepts the strict launcher form `CODEX_HOME=<absolute-directory> codex`,
parses it into an environment assignment and executable, and starts it through
the OS process API. Thus this pair:

```sh
alias: pp7
run command: CODEX_HOME=/Users/user1/.codex-pp7 codex
```

becomes a child `codex` process with only its `CODEX_HOME` overridden. Shell
operators, substitutions, redirections, additional environment assignments,
and alternate executables are rejected rather than evaluated. The production
adapter will add only the fixed arguments required by the separately approved
non-interactive Codex surface.

The adapter must not mutate the parent `nd` environment, infer a profile from
the server's own `CODEX_HOME`, enumerate unconfigured Codex homes, or read/copy
authentication and configuration files from a home. The raw run command and
absolute home path stay inside configuration validation and process setup:
APIs, persisted preferences, TUI text, logs, errors, prompts, and model input
use only the alias-derived backend ID, while the TUI displays the alias.

This contract settles profile multiplicity and configuration location only.
Choosing `codex exec`, app-server, or another supported non-interactive surface;
pinning and discovering the executable; account eligibility; provider-side data
use and retention; sandbox/tool denial; cancellation; concurrency; and upgrade
compatibility remain owner decision-gate items before a production Codex
adapter may send diary content.

## Approach and sequence

1. Record the approved multi-profile Codex configuration contract, then resolve
   and document the remaining provider, credential/account, HTTP or Codex
   process/API, executable/version, privacy, and lifecycle choices before
   dependency changes or real assistant calls.
2. Extend strict global-config parsing with the alias/run-command collection
   above. Parse the supported command form into a child environment and argv
   without a shell. Keep raw commands and profile paths out of runtime/API DTOs,
   and use fake directories and process launchers to prove zero-, one-, and
   multi-profile behavior without reading a real Codex home.
3. Verify P1-M12's separated manual-availability and optional-backend-ID model,
   then version only the new assistant action/result resources that M8 needs.
   Add mixed-client/server tests before assistant calls exist.
4. Define backend descriptors and a registry keyed by stable ID. Prove with at
   least three fake adapters that capabilities, selection, persistence, and
   routing are collection-shaped rather than a binary hosted/Apple branch. When
   Codex is approved, instantiate one descriptor per configured profile without
   adding provider-specific workflow or TUI branches.
5. Define extraction, drafting, and paragraph-review request types constructible
   only from the approved data classes, plus versioned suggestion acceptance;
   centralize size, timeout, cancellation, response validation, and the
   no-tools/no-side-effects adapter boundary.
6. Implement hosted availability and calls without logging text or exposing its
   credential through core, database, API, process arguments, or errors.
   If this is an approved Codex adapter, use only the selected alias's parsed
   command, set its child process's `CODEX_HOME`, and never interpolate it into
   a shell command or return the raw command or path.
7. Add the helper's on-device command group with the same structured outcomes
   and fake all model behavior in tests.
8. Wire extraction, draft, and paragraph-review jobs through freshness
   checkpoints and human memory/entry review. Route one command to one backend,
   retain manual text on failure, and refuse stale paragraph acceptance.
9. Extend the existing Compose ViewModel: show all registered backends, keep
   confirmed sources and the draft in the Resources Sidebar, render suggestions
   in the Main Pane, keep the selected backend and path visible in Work Status
   and the compact Status Line, and expose explicit accept/reject actions.
10. Prove the production adapter contracts and a three-fake registry through
   synthetic contract tests and fake-backed vertical flows.

The hosted default is a preference, not a consent screen; the selected backend
and active path stay visible. No adapter receives confirmation, approval, or
publication capabilities. A future Codex-backed adapter uses the same registry,
with one instance per configured profile, only after its remaining operational
choices are approved; a Codex MCP client remains a separate future inbound
client.
