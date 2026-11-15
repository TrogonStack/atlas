# Visual artifact integration requirements

Status: proposed visual integration contract. Main implements the
[artifact service](../how-to/manage-artifacts.md) for upload, stored-byte
integrity verification, immutable identity and retrieval using NATS Object Store
or S3-compatible storage. Visual captures, typed model attachments, raster
verification and authenticated Studio image delivery remain proposed work.

This reference defines the artifact boundary shared by the proposed
[visual state contract](visual-state-contracts.md) and
[email catalog contract](email-catalog-contracts.md). It does not introduce
wire fields, enable storage, execute captures or establish runtime acceptance.
It refines the artifact boundary in the merged
[visual product proposal](../explanation/visual-product-atlas.md) and
[delivery roadmap](../explanation/visual-product-atlas-roadmap.md).

## Separate authorities

| Concern | Authority and limit |
| --- | --- |
| Artifact bytes | The artifact service accepts immutable bytes after size, digest, persistence and publication checks. |
| Image interpretation | A bounded image verifier establishes a supported raster format and dimensions. Declared media metadata alone establishes neither. |
| Model attachment | The model backend accepts a typed reference through the existing guarded mutation path. Upload completion does not attach a file. |
| Capture meaning | A capture record binds accepted bytes to exact model subjects, profile, build and evidence. Byte integrity does not prove the captured scenario ran. |
| Freshness and baseline | The capture contract determines applicability and reviewed baseline selection. File age and successful retrieval do not establish freshness. |
| Delivery | An authenticated adapter checks current access before returning bytes. A cached image or temporary grant cannot establish model authority. |

Existing Specification `ContractArtifact` members retain source text inside a
Specification. They are separate from the binary artifact service and do not
become screenshot attachments through a media type label or external link.

## Typed reference

Propose an `ArtifactRef` with the following required values for every visual or
email capture attachment. This is a semantic shape; wire allocation remains
part of coordinated native implementation.

| Value | Rule |
| --- | --- |
| `namespace` | Exact artifact namespace. A model namespace does not implicitly select an artifact namespace. |
| `artifact_id` | Immutable ID returned by successful upload completion. No filename, object key, URL, latest-version lookup or digest fallback. |
| `expected_sha256` | Required lowercase hexadecimal SHA-256 digest of the accepted original bytes. Resolution checks equality with accepted artifact metadata. |

Namespace and immutable ID establish artifact identity. The required digest
pins integrity; equal digests do not merge identities or grant access to another
namespace. Provider, bucket, storage key, credentials and temporary URLs remain
outside model references.

The containing capture or attachment retains its own native identity and
referring-owner incarnation. Its exact UI, connection, scenario or email variant
selection describes what the image documents. Those selections do not replace
the artifact reference or make the image exclusive to one subject. Reusing an
image in another capture requires an explicit reference in that capture.

An attachment remains a reference when one of its documented subjects stops
resolving. The system must preserve that distinction for inspection and
retention. Owner recreation, changed connection identity and repeated flow
occurrences follow their native identity contracts; matching endpoints do not
retarget an attachment.

## Upload and guarded attachment

The implemented [artifact API](../../proto/trogonatlas/api/eventmodel/v1alpha1/artifacts.proto)
exposes storage discovery, begin, content transfer, complete, metadata lookup
and content retrieval. Control operations and Atlas-mediated bytes use gRPC;
MCP controls call those RPCs. Direct S3 byte transfer uses presigned HTTP PUT or
GET, with file-capable clients handling upload bytes. NATS has no direct HTTP
transfer or provider image URL. A remote MCP server cannot read an arbitrary
caller-local file from a path.

The integration must preserve this sequence:

1. Authenticate and authorize upload in the requested artifact namespace.
   Discover the active provider and server admission limits before transfer.
1. Begin a bounded upload and transfer bytes using the returned method.
   Keep upload instructions transient and separate from model content.
1. Complete the upload. Verify the declared size and digest, persisted final
   bytes and publication receipt before treating the artifact as accepted.
   Interrupted or ambiguous completion must recover the same accepted result
   or report failure; a client cannot promote pending bytes to Ready.
1. Verify image suitability before accepting an inline visual capture. Retain
   the verifier result against the exact original artifact ID and digest.
1. Submit the capture and its `ArtifactRef` through a guarded native model
   mutation. Recheck current authoring authority, artifact readability, exact
   subject resolution, image-verification result and required owner guards.
1. Return the accepted model result and its identities. A failed attachment
   leaves an accepted, recoverable artifact; it does not delete the bytes or
   imply that the model change landed.

Cross-namespace attachment requires authority for both the model mutation and
the referenced artifact. It does not copy artifact ownership into the model
namespace. Read projections must enforce both boundaries without disclosing
hidden artifact metadata or inventing a readable replacement.

Ordinary model mutation and artifact publication use separate authorities.
This contract does not claim an atomic transaction over them. While accepted
artifacts cannot be collected, admission can validate the durable accepted
record conservatively. Any future deletion capability must introduce the
coordinated attachment holds described under
[retention](#retention-and-collection).

Branch, etag, incarnation and operation guards apply to model writes as they do
elsewhere. Image upload completion does not override stale guards. Concurrent
subject edits must yield a stale or unresolved capture when applicability
cannot be proved; newer metadata cannot relabel older bytes as a new capture.

## Image verification and inert rendering

The implemented artifact service accepts a declared media type and verifies
size and digest against persisted bytes. Its acceptance does not establish
image decoding, dimensions, scenario execution, sanitization or freshness.
Existing accepted files must pass the proposed image verification before inline
visual display.

The initial inline image profile accepts static PNG, JPEG and WebP only.
Verification must inspect actual bytes and reject a mismatching declared image
type, malformed or truncated content, unsupported encoding and animation.
SVG, HTML, PDF and other generic files remain outside this inline profile.
Retaining a generic file never authorizes embedding executable content.

Image verification must bound input bytes, dimensions, decoded pixel count,
decoded working memory, processing time and concurrent jobs before allocating
expensive decode buffers. It must use a maintained decoder and record the
verified format, dimensions, original digest and verification implementation
version. A timeout, unsupported decoder feature or exhausted budget produces
an explicit unavailable or failed result. It cannot become successful
verification or a fresh capture.

Verification must not rewrite the original artifact. Re-encoding, redaction or
thumbnail generation produces a separate derived artifact with its own ID and
digest and a recorded exact source reference. Retain that relationship and the
transformation version. A thumbnail cannot substitute for original capture
bytes in an evidence receipt.

Capture producers sanitize credentials, personal data and environment-specific
content before upload. Successful raster verification does not certify privacy
or redact pixels. Capture metadata must contain fixture and role labels without
tokens, cookies, passwords or signed URLs. Image interpretation by an agent is
a separate operation from storing or displaying evidence.

## Browser delivery

| Implemented provider behavior | Proposed browser integration requirement |
| --- | --- |
| NATS Object Store | Authenticated `GetArtifactContent` returns bytes through gRPC. There is no provider HTTP image URL. Studio needs an adapter. |
| S3-compatible storage | Authenticated content RPC remains available. A download RPC can issue an expiring presigned HTTP GET grant. The grant is separate from artifact identity. |
| Presigned grants | A grant can remain usable until expiry after Atlas access changes. Issuing one is not a guarantee that every later byte read rechecks current authorization. |

The proposed default for Studio is a same-origin authenticated image bridge
using the content RPC. Each request must check the caller's current artifact
namespace access and the authorized capture or attachment context before
fetching bytes. Provider credentials stay on the server. The bridge must not
accept arbitrary storage keys, remote URLs or caller-selected upstreams.
This refines the proposal's unsettled browser-delivery decision; no bridge or
HTTP image route is implemented by these documents.

Return only verified supported raster content with its verified content type,
`X-Content-Type-Options: nosniff` and an inline disposition without unsafe
filename interpolation. Error responses must preserve failure status and
must not masquerade as an empty successful image. Apply the same rules to
originals, thumbnails, downloads and image-detail views.

Bound request concurrency, source bytes, response buffering, decode work and
delivery time across the whole process. A per-image limit alone does not bound
a gallery of concurrent requests. Avoid base64 image data in model JSON and
fetch metadata before image bytes. Catalog pagination and viewport loading must
not prefetch every full-resolution capture.

Default protected delivery responses must prevent shared or persistent
unauthorized caching. Any later private cache must partition by deployment,
caller and authorization context, and recheck current access before reusing
bytes. Immutable digests help detect changes; they do not authorize cache reads
or guarantee revocation of bytes a reader already obtained.

A derivative or persisted export that combines inputs retains their complete
exact identity set. Every delivery checks current authority for all required
transitive inputs as well as the derivative's own namespace. Moving a thumbnail,
redaction or PDF into a readable namespace cannot make restricted source bytes
public. Input revisions are frozen for generation; a same-key replacement is
not an implicit new source. Missing input provenance prevents delivery under
this policy rather than granting access based only on the output artifact.

Direct presigned browser delivery would be a separate opt-in deployment policy
with explicit grant lifetime, browser reachability, CORS and revocation limits.
It is not a portable NATS capability or the default current-authorization
guarantee. Persist no signed URL in a capture, export or model document.

## Read context and completeness

The model selects the relevant branch or historical scope. The artifact
catalog is not a historical model snapshot. Resolving an old capture means
loading its exact recorded artifact identity under current read authority;
it does not rewind the artifact service or recover deleted bytes implicitly.

Catalog and flow responses must retain their model scope, completeness and
applicability information. If a projection cannot read the required model
owners consistently, it must report the observed revisions and any uncertainty
rather than claim an atomic snapshot across owners and artifact stores.
Paginated counts must use the same authorized scope and report incompleteness.

Hidden captures, artifact metadata, related subjects and thumbnails must not
leak through filter counts, grouping, image dimensions, generated URLs or
distinguishable resolution errors. Operational failure is distinct from an
empty catalog, missing evidence or stale capture. The UI must preserve those
states and avoid converting retrieval failure into absence or success.

## Retention and collection

Confirmed artifacts remain retained independently of upload-session expiry.
Removing an attachment, entity, model or branch does not delete its images.
Supported historical records, other branches and explicit reuse may still
reference them. No automatic final-object collection is enabled by this
contract, and generic bucket TTL must not expire accepted bytes.

Before any confirmed-file collection is introduced, implement all of the
following as an authoritative artifact lifecycle:

- Complete typed-reference traversal across live entities, retained branches,
  historical revisions and explicit retention pins. Search results and capped
  impact queries are insufficient reference inventories.
- Durable attachment holds coordinated with final-deletion claims. Register a
  hold before a model attachment can commit; uncertainty or crashes retain
  bytes conservatively. A deletion claim must prevent new attachment admission
  instead of allowing a reference to disappearing bytes.
- Release holds only after durable model and history changes. Reconciliation
  must identify its coverage and mutation watermark and fail closed on missing
  pages, decode failures, read errors or incomplete history coverage.
- A configured recovery period after the last confirmed reference and hold
  disappear. Reattachment cancels eligibility before deletion is claimed.
- Retained cleanup evidence, repeated handling of late provider writes and
  separate policies for abandoned staging, unaccepted final candidates and
  accepted final objects. A missing catalog lookup never authorizes deletion.

Temporary-upload cleanup and confirmed-file retention have different admission
criteria. Integrating a staging cleaner does not establish reference accounting
or authorize collecting accepted files. Namespace teardown and provider
migration require separate procedures that preserve accepted bytes and catalog
authority together.

## Required limits and verification

Implementation must expose concrete effective deployment limits for upload
bytes and lifetime, accepted image dimensions and pixel count, total decode
memory, verification time, concurrent transfers, concurrent decoding,
thumbnail jobs, response bytes and request deadlines. Apply the lowest relevant
artifact, transport, model-entity and storage limit. Reject excessive input
atomically without truncating bytes, references or evidence.

Capture ingestion must also bound metadata lengths, profile and build labels,
subject selections, capture records per owner and paginated result size.
Backend completeness must survive client limits. Resource exhaustion is a
retryable refusal where appropriate, never successful partial verification.

Connected acceptance must cover upload, completion, guarded attachment,
reference discovery, export/reapply and authenticated Studio retrieval through
public seams against each supported provider. Include mismatched digests,
unaccepted artifacts, revoked namespace access, stale guards, owner recreation,
branch and historical reads, malformed images, decompression expansion,
concurrent gallery requests and thumbnail/original authorization parity.
Add collection-race and full-history retention receipts before enabling
confirmed-file collection. Schema tests, mocked delivery and artifact byte
round trips alone do not establish this visual integration.
