# P1-M6: Control API Server architecture plan

Status: Planned

This plan makes the control transport an inbound adapter around the P1-M4 use
cases and the P1-M5 source connectors.

## Affected modules and interfaces

- `diary-control-contract`: versioned JSON DTOs, safe problems, events, and
  schema/build identifiers.
- `diary-control`: REST routes, SSE fan-out, authentication, jobs, request
  bounds, and error translation.
- `apps/nd`: `serve` lifecycle, lock/runtime files, configuration, adapter
  registration, listener selection, signals, and shutdown.
- `diary-core`: no transport dependencies; only use-case calls from handlers.

## Approach and sequence

1. Freeze the first contract version around existing use cases and define
   explicit mappings between wire DTOs and core values.
2. Implement lifecycle primitives in documented startup order, writing runtime
   state only after the listener is ready and trusting it only while locked.
3. Add loopback HTTP with health/version negotiation and local authorization,
   then layer workflow reads and conditional mutations on top.
4. Run blocking SQLite, hashing, and helper work on bounded workers; represent
   long work as jobs with timeouts and cancellation requests.
5. Emit small SSE notifications after committed state changes and require
   clients to refetch authoritative state after reconnect or a missed revision.
6. Exercise multiple clients, stale state, duplicate commands, bad origins,
   crashes, and shutdown through transport-level tests.

The route table may expose only behavior implemented so far; later milestones
add model and publication behavior behind the already versioned resources.
