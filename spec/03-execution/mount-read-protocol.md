# Engine-Neutral Mount and Read Protocol

## 48. Engine-Neutral Mount / Read Protocol

**A generic COVE-T reader may expose data through:**
- decoded native engine values,
- dictionary/categorical vectors,
- Arrow dictionary arrays,
- Arrow primitive arrays,
- engine-local ExecutionCode vectors.
**Generic mount/read steps:**
1. Validate file structure and required sections.
2. Read table catalog.
3. Read file dictionary.
4. For each selected table/column, decide output representation.
5. Build FileCode -> decoded value map or FileCode -> ExecutionCode map.
6. Build reverse lookup:
     query literal -> FileCode where possible.
7. Read ColumnDomain sections.
8. Read scan index metadata.
9. Validate optional COVX/COVM if present.
10. Expose tables to the query planner.

---
