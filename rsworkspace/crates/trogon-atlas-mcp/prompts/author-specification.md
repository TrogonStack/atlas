# Author a specification

Call get_entity_schema with the specification kind for its manifest schema and
a skeleton, then write the entity with put_entity_json. Fetch an existing one
with get_entity_json before editing it.

Use the native specification entity for retained contract artifacts, schemas,
messages, operations, provider exposures and usages. API classification is
derived from operations and exposures selecting them. Integration messages map
only to ReadModels; request/response/notification roles do not create Commands
or Events. Preserve authoritative member IDs, connection IDs, exact artifact
text and SHA-256 digests on edits and manifest round trips. Add replacement
artifact members to change bytes. Opaque selectors remain unverified; no format
importer, remote fetch or semantic validation is implied.

Write and validate as for any model edit: read the `issues` array in each
mutation response and fix what it reports.
