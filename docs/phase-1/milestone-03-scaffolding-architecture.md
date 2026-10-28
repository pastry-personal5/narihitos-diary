# P1-M3: Scaffolding and tools architecture plan

Status: Planned

This plan turns the proposed module boundaries into buildable shells while
leaving behavior to later milestones. Dependency and build-tool changes remain
owner approval points under [AGENTS.md](../../AGENTS.md).

## Affected modules and interfaces

- Workspace members for `diary-core`, connectors, the control contract and
  server, the TUI, and the `apps/nd` composition root.
- `helpers/apple-bridge` as the single Swift package and executable boundary.
- Repository configuration templates, ignore rules, and contributor tooling.
- Minimal command parsing for `nd` and `nd serve`.

## Approach and sequence

1. Propose the smallest dependency set and Swift toolchain version, verify
   Apache-2.0 compatibility, and obtain owner approval before editing manifests.
2. Create crate and package shells with dependency direction pointing inward;
   keep contract DTOs separate from core types and Apple code macOS-gated.
3. Wire one binary composition root whose default mode and `serve` subcommand
   return explicit not-yet-implemented outcomes rather than fake success.
4. Define only the helper protocol envelope needed for a fake request/response
   test; defer iCloud and model commands to their owning milestones.
5. Add safe configuration examples and ignore rules, then document the actual
   toolchain and validation workflow.
6. Run and record the complete Rust and Swift gate as the scaffold baseline.

Later milestones may refine outer-crate grouping, but they must preserve the
independent core and the single `nd` binary. Broad restructuring is not part of
this milestone.
