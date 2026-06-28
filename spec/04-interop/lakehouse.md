# Lakehouse Integration Profile

## 50. Lakehouse Integration Profile

COVE is a file format, not a catalog or table format.
**COVE files MAY be managed by:**
- COVM native manifests,
- Iceberg,
- Delta,
- Hudi,
- engine-specific catalogs,
- object-store inventory systems,
- custom manifests.
**Lakehouse hints MAY include:**
- table schema fingerprint,
- partition values,
- source snapshot identifier,
- data file sequence number,
- delete/visibility overlay reference,
- catalog table identifier,
- source format provenance,
- conversion digest.
**Rules:**
- Lakehouse hints are optional.
- Lakehouse hints MUST NOT override COVE file semantics.
- Visibility/delete overlays are external in v2.
- COVE files remain immutable.
**Recommended usage:**
**Iceberg / Delta / Hudi:**
  May use COVE-T files as data files if the table engine has a COVE reader.

**COVM:**
  Native lightweight COVE dataset manifest for archive/object-store planning.

**COVX:**
  Optional acceleration sidecar for immutable COVE files.

### 50.1 COVM vs Lakehouse Catalogs

COVM is not a transaction log, table catalog, or lakehouse protocol. It is a COVE-native planning manifest for a set of immutable COVE files.
**Rules:**
- When COVE files are managed by Iceberg, Delta, Hudi, or another table format, that external catalog remains authoritative for table snapshot selection, transactions, schema evolution, deletes, and visibility.
- COVM MAY mirror catalog-derived file lists and pruning metadata for faster COVE-native planning, but COVM MUST NOT override the external catalog's selected snapshot or visibility rules.
- Standalone COVM datasets MAY use immutable COVM publication to identify a dataset state, but this is a lightweight archive/dataset mechanism, not a replacement for ACID table protocols.
- Lakehouse hints inside COVE are descriptive hints. They MUST be validated against the external catalog before being used as authoritative table metadata.

### 50.2 COVE as Data Files in Table Formats

COVE v2 intentionally does not define a COVE Table Layer with ACID commits, catalog state, snapshot isolation, schema evolution, partition evolution, or transaction logs. Those responsibilities belong to an external table format or catalog.

Official integration specifications MAY define how COVE-T files are used as data files inside Iceberg, Delta, Hudi, Hive-style catalogs, Unity-style catalogs, or engine-specific catalogs. Such adapter specifications MUST:
- keep .cove files immutable,
- identify data files by URI plus stable file_id, file_len, footer_crc32c, and digest where available,
- map external table schema fields to COVE table_id/column_id without changing the COVE file schema,
- apply the external catalog's snapshot, partition, delete, visibility, time-travel, and schema-evolution rules before returning rows,
- treat LAKEHOUSE_HINTS, COVM entries, and metadata JSON as hints unless the external catalog explicitly accepts them,
- reject or ignore any COVE hint that conflicts with the selected external snapshot.

A future COVE-native table protocol, if one is ever standardised, MUST be a separate companion specification with its own conformance level, commit protocol, and feature gates. It MUST NOT weaken the immutability or standalone readability of COVE data files.

### 50.3 External Delete and Visibility Overlay Semantics

External row-level deletes, deletion vectors, equality deletes, access filters, and visibility overlays are outside COVE-Core and COVE-T v2. They MAY be referenced by lakehouse hints or manifests, but their semantics are defined by the external table format, catalog, or application protocol.

COVE predicate metadata and indexes describe the physical rows present in the immutable COVE file before external visibility filtering. When an external overlay is active:
- PredicateZoneOutcome::DefinitelyNo remains safe for pruning because no physical row in the zone satisfies the predicate.
- PredicateZoneOutcome::DefinitelyYes remains safe only as a claim that every remaining visible row from that physical zone satisfies the predicate; it does not prove that any visible row remains.
- Unknown remains Unknown.
- Exact sets, blooms, ColumnDomain ranges, and zone stats MAY be used to reject impossible predicates over the physical file, but they MUST NOT be interpreted as exact visible-table domains unless the overlay is proven empty or overlay-aware metadata is available.
- Lookup indexes and inverted morsel indexes return physical row candidates. Readers MUST apply the external visibility/delete overlay before returning rows.
- Aggregate synopses over a COVE file are exact only for the physical COVE rows. They MUST NOT answer visible-table aggregate queries when a non-empty external overlay is active unless an overlay-aware correction or proof is applied.

External overlays that reference physical positions SHOULD identify the target COVE file by file_id plus file length, footer CRC, and cryptographic digest where available. Rewritten or compacted COVE files receive new physical row references; overlays for old files MUST NOT be silently applied to rewritten files.

In v2 `LAKEHOUSE_HINTS` may reference an external visibility overlay by setting
hint flag bit 2. The overlay reference is encoded after `conversion_digest` as
`overlay_kind: u8`, `fingerprint_flags: u8`, optional fingerprint fields in
flag order (`file_id`, `file_len`, `footer_crc32c`, `digest`), then
`reference_len: u16` and UTF-8 `reference` bytes. This reference is descriptive;
the external table format or catalog remains authoritative for overlay
semantics.

### 50.4 Append, Streaming, CDC, and Compaction Boundary

**The accepted mutable-data pattern for COVE v2 is immutable-file publication:**
- append by writing additional complete COVE files, or by writing immutable COVE-O `.covedelta` artifacts and publishing a new COVM state or external table snapshot that selects the resulting base-plus-delta chain,
- update/delete by external table-format overlays or by rewriting affected data into new COVE files,
- update/delete for COVE-O by publishing selected `.covedelta` artifacts whose temporal records, tombstones, and summaries are part of the selected dataset snapshot,
- compact by writing replacement COVE files and publishing a new manifest/catalog state,
- ingest streams by buffering or micro-batching into temporary writer state, then finalising complete COVE files.

**Rules:**
- A .cove file MUST NOT be appended in place after finalisation.
- A `.covedelta` file MUST NOT be appended in place after finalisation.
- A partially written object MUST NOT be treated as a valid COVE file.
- A partially written `.covedelta` artifact MUST NOT be treated as a valid selected delta.
- Patch, delta, CDC, or operation-log files MAY be represented as ordinary COVE-T data files when an external protocol defines their meaning, but COVE-Core/COVE-T readers MUST treat them as ordinary data unless that external protocol is explicitly in scope. COVE-O `.covedelta` artifacts are a COVE-O temporal profile surface, not ordinary `.cove` files.
- COVM readers MUST select one published dataset state. They MUST NOT merge multiple COVM generations as an implicit transaction log unless a separate protocol says to do so.
- COVM or the governing external catalog is authoritative for selecting the base artifact and ordered `.covedelta` chain. Readers MUST NOT discover visible deltas by filename scanning, directory ordering, object-store listing, wall-clock time, or "latest file" heuristics.
- A delta-bearing snapshot selected by COVM MUST be read as the selected base-plus-delta chain. A reader that cannot validate the required delta profile, chain digest, required summaries, or required features MUST fail closed for that selected snapshot rather than silently returning base-only object state.
- A user or API MAY explicitly open the base `.cove` artifact directly. That direct-file read is not the same operation as reading the delta-bearing dataset snapshot.
- Generic COVE-Core and COVE-T readers that do not claim COVE-O delta support MUST treat `.covedelta` artifacts as out of scope. Ordinary direct reads of finalised `.cove` files remain unaffected by unrelated `.covedelta` files.
- Compaction MUST preserve logical table semantics according to the governing manifest or catalog; it MUST NOT mutate the replaced COVE files.
- COVE-O delta compaction materialises a selected base-plus-delta snapshot into a new self-contained `.cove` file and publishes a new manifest/catalog state; it MUST NOT mutate the replaced base or delta artifacts.

---
