# P1-M5: Source ingestion and Apple bridge architecture plan

Status: Planned

This plan implements `SourceCatalog` and `SourceReader` while preserving the
core's connector-neutral source references and privacy envelope.

## Affected modules and interfaces

- `diary-connectors`: Ayumi catalog, Markdown envelope parser, source filtering,
  snapshot digesting, freshness checks, and helper client.
- `diary-core`: source use cases and typed acquisition/read outcomes where the
  P1-M4 contracts need concrete exercise, without adapter leakage.
- `helpers/apple-bridge`: versioned iCloud command group.
- `apps/nd`: source connector registration and configured-root validation.

## Approach and sequence

1. Build the catalog from relative opaque keys and bounded date queries; parse
   only enough metadata to return proposal evidence and warnings.
2. Separate envelope parsing from body filtering, digest original bytes first,
   and pass only sanitized readable bodies through the source port.
3. Extend the helper protocol with availability, download, and coordinated-read
   commands; bound every request and keep standard error content-free.
4. Request download and coordinated read only after selection, mapping platform
   failures to `NeedsDownload`, `PermissionDenied`, `Unreadable`, or
   `Unverifiable` rather than empty content.
5. Reuse snapshot stamps at each freshness checkpoint and invalidate through
   the P1-M4 state machine on change or unverifiability.
6. Validate with synthetic directory trees and a fake helper; reserve any real
   private-directory check for an explicit, separately authorized acceptance
   step.
