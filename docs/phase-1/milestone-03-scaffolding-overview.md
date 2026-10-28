# P1-M3: Scaffolding and tools overview

Status: Planned

This milestone creates the buildable project skeleton and verification baseline
used by every later Phase 1 milestone. It introduces structure and test seams,
not diary behavior.

## Goal

Establish the approved Rust workspace, `nd` binary, Swift helper package, and
development checks without prematurely implementing product workflows.

## In scope

- Confirm the initial dependency and tool choices with the owner before changing
  manifests or build tooling.
- Create the Rust workspace boundaries from the architecture, preserving an
  independently testable inward-facing core and one canonical `nd` binary.
- Provide `nd` and `nd serve` command shells with no claimed workflow behavior.
- Create `helpers/apple-bridge` with a versioned protocol shell and fake-based
  test target; choose and document its Swift toolchain version.
- Add repository-local configuration examples and ignore rules without adding
  real credentials or private diary data.
- Establish unit-test, formatting, lint, and build commands for both languages.

## Out of scope

- Diary workflow, HTTP endpoints, terminal screens, connector behavior, and
  external writes.
- Selecting the hosted provider, its credential storage, or its HTTP client.
- Reading the configured Notion root or private Ayumi notes.

## Completion checklist

- [ ] The owner has approved every new dependency and build-tool choice before
  its manifest or configuration is added.
- [ ] The workspace reflects the architecture boundaries, and `diary-core` can
  compile and test without terminal, HTTP, SQLite, Notion, or Apple frameworks.
- [ ] One `nd` binary exposes non-functional `nd` and `nd serve` command shells
  with clear placeholder behavior.
- [ ] `helpers/apple-bridge` builds and tests with a fake protocol exchange, and
  its selected Swift toolchain version is documented.
- [ ] Configuration templates, private-path defaults, and ignore rules contain
  no credential or diary content and match the files-and-configuration plan.
- [ ] Contributor documentation names the actual setup and validation commands.
- [ ] The standard milestone gate passes, including Swift build and test now
  that the helper exists, and the results are recorded as acceptance evidence.
