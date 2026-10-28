# P1-M6: Control API Server overview

Status: Planned

This milestone exposes the core through the loopback Control API Server and
makes `nd serve` a secure, recoverable foreground process. It supplies the
transport needed by the TUI without moving policy into handlers.

## Goal

Provide the versioned REST, SSE, job, lifecycle, and local authorization
contract through which every Phase 1 client uses diary workflows.

## In scope

- Startup/shutdown order, single-instance lock, owner-only runtime file,
  configuration permission checks, migrations, purge, and reconciliation.
- Loopback-only binding, bounded port fallback, control token, capability
  checks, `Host`/`Origin` validation, safe problem details, and redacted logs.
- Versioned workflow resources, ETags, entry-buffer conditional writes, jobs,
  cancellation, and SSE notifications.
- Typed contract DTOs and an in-process test client for later TUI work.

## Out of scope

- `/mcp`, remote access, a browser client, and agent tokens.
- Production terminal screens, model adapters, and output connector behavior.
- Broadening the listener beyond loopback.

## Completion checklist

- [ ] `nd serve` follows the documented lifecycle and safely handles a stale
  runtime file, duplicate process, migrations, purge, background reconciliation,
  and bounded shutdown.
- [ ] The listener is loopback-only, uses the approved port sequence, writes
  owner-only runtime data, and rejects unsafe config permissions.
- [ ] Contract tests cover health/version negotiation, bearer capabilities,
  `Host` and `Origin`, request bounds, no-store responses, safe RFC 9457 errors,
  and content-free logs.
- [ ] Workflow resources map all implemented use cases without placing source,
  confirmation, approval, or publication policy in route handlers.
- [ ] ETag tests cover stale workflow mutations and independent entry-buffer
  writes across multiple clients.
- [ ] Job and SSE tests cover accepted work, progress, cancellation, reconnect,
  missed events, and canonical state refetch.
- [ ] The API schema and startup documentation are current, and `/mcp` remains
  disabled and out of Phase 1.
- [ ] The standard Rust and Swift milestone gate passes and acceptance evidence
  is recorded.
