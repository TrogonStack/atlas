# Upload and retrieve artifacts

Use the artifact service to upload an image or other file, verify its bytes,
and obtain its namespace and immutable artifact ID. The server can
store the bytes in NATS JetStream Object Store or an S3-compatible bucket.
Artifact metadata and upload sessions remain in NATS KV with either backend.

Atlas artifact operations use gRPC, including creating upload grants,
completion, metadata lookup and byte retrieval. MCP tools call those RPCs.
Only direct S3 byte transfers use presigned HTTP PUT or GET requests.

Artifacts and upload sessions are scoped to a namespace and are independent of
model branches. Creating or removing a model branch does not create or expire
its file bytes. Typed attachments from model entities remain separate work.

Configure storage on the server deployment. CLI and MCP clients discover that
configuration; they do not need NATS or S3 credentials. Artifact storage is
disabled by default.

With Helm, select `artifacts.backend.kind` and put that provider's settings
in `artifacts.backend.spec`. Use `kind: nats` with a NATS Object Store
`spec.bucket`, or `kind: s3` with an existing S3 `spec.bucket` and optional
S3 settings. `artifacts.catalog.natsKvBucket` always names the separate NATS
KV bucket for metadata and upload sessions with either backend. See the
[chart's backend settings and examples](../../devops/helm/charts/trogon-atlas-server/README.md#artifacts).

## Enable NATS Object Store

Set these variables in the environment that starts `trogon-atlas-server`:

```sh
export TROGON_ATLAS_ARTIFACT_BACKEND=nats
export TROGON_ATLAS_ARTIFACT_NATS_BUCKET=atlas_artifacts
export TROGON_ATLAS_ARTIFACT_CATALOG_BUCKET=atlas_artifact_catalog
export TROGON_ATLAS_ARTIFACT_MAX_BYTES=16777216
export TROGON_ATLAS_ARTIFACT_UPLOAD_TTL_SECS=900
export TROGON_ATLAS_MAX_MESSAGE_BYTES=37748736
```

The CLI uses its configured Atlas gRPC endpoint for all NATS artifact
operations. Artifact storage needs no separate public origin, HTTP upload
route or content route. Use TLS for remote connections.

The service uses the existing `TROGON_ATLAS_NATS_URL` connection. NATS must
have JetStream enabled, and the server's NATS identity needs access to the
artifact Object Store and catalog KV bucket. Provision file storage, replicas,
capacity, and backups for both buckets according to the deployment's
durability requirements. The server creates missing buckets at startup when
its NATS identity permits it, and reuses existing compatible buckets. New
buckets use the service's provider defaults; pre-provision them when a
different replication policy is required.

An artifact can be larger than a NATS message because Object Store splits its
bytes into chunks. This does not change the existing entity-size contract or
the NATS transport capacity required by Atlas's model store.

## Enable S3 or an S3-compatible provider

Create a private bucket, then configure the server:

```sh
export TROGON_ATLAS_ARTIFACT_BACKEND=s3
export TROGON_ATLAS_ARTIFACT_CATALOG_BUCKET=atlas_artifact_catalog
export TROGON_ATLAS_ARTIFACT_S3_BUCKET=product-artifacts
export TROGON_ATLAS_ARTIFACT_S3_REGION=us-east-1
export TROGON_ATLAS_MAX_MESSAGE_BYTES=37748736
```

Supply credentials to the server through the deployment's secret manager or
supported workload identity. Standard credential variables include
`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and, for temporary credentials,
`AWS_SESSION_TOKEN`. Keep their values out of manifests, command arguments,
logs, and client configuration. The Helm chart can reference an existing
Secret through `extraEnv`; see its
[artifact configuration](../../devops/helm/charts/trogon-atlas-server/README.md#artifacts).

The server needs permission to put, get, and delete artifact objects in its
bucket's staging and final object prefixes. Bucket access remains private;
the server issues bounded presigned requests instead of granting clients
bucket credentials. Restrict the deployment's storage identity to its artifact
bucket and object prefixes.

For MinIO or another S3-compatible provider, set
`TROGON_ATLAS_ARTIFACT_S3_ENDPOINT` to the provider's endpoint and enable
`TROGON_ATLAS_ARTIFACT_S3_FORCE_PATH_STYLE` if it requires path-style requests.
The explicit Atlas endpoint takes precedence over inherited AWS endpoint
variables. Inherited endpoints also pass Atlas's URL validation when used.
For a local MinIO instance reachable at port 9000 by both a native server and
the CLI:

```sh
export TROGON_ATLAS_ARTIFACT_S3_ENDPOINT=http://127.0.0.1:9000
export TROGON_ATLAS_ARTIFACT_S3_FORCE_PATH_STYLE=true
export TROGON_ATLAS_ARTIFACT_S3_ALLOW_HTTP=true
```

Allow HTTP only for isolated local development. The configured endpoint is
used in signed URLs, so it must be reachable by upload clients as well as the
server. A container-only hostname will not work for a CLI running outside
that network. Configure bucket CORS if a browser will upload directly to S3;
CLI uploads are not subject to browser CORS.

The S3 backend still requires NATS for the artifact catalog and the rest of
the Atlas service. Configuring S3 does not replace the model store.

Changing the configured backend, bucket, or endpoint does not migrate existing
artifacts. Preserve the matching catalog and blobs, or migrate them together
before changing the deployment configuration.

## Discover the active backend

Configure the CLI's Atlas endpoint and provide its bearer credential through
the existing `TROGON_ATLAS_AUTH_TOKEN` secret environment variable. Then run:

```sh
trogon-atlas artifacts info
```

The result reports whether storage is enabled, the backend, maximum file size,
upload lifetime, and whether direct upload is available. It does not expose
provider credentials.

`TROGON_ATLAS_ARTIFACT_MAX_BYTES` defaults to 16 MiB and cannot exceed 32 MiB.
The gRPC message limit must be at least this value plus 65,536 bytes for
request metadata. `TROGON_ATLAS_ARTIFACT_UPLOAD_TTL_SECS` defaults to 900 seconds
and must be between 1 and 3,600 seconds. Configure these limits in deployment,
not independently in each client.

## Upload with the CLI

Use the namespace's ID, as returned by namespace discovery:

```sh
trogon-atlas artifacts upload \
  --namespace NAMESPACE_ID \
  --file ./login-error.png \
  --media-type image/png
```

The CLI calculates the size and SHA-256 digest, begins an upload, transfers the
bytes, and requests completion. For NATS, it transfers through the
authenticated Atlas RPC. For S3, it puts the bytes directly to the returned
presigned URL with the required headers, without forwarding the Atlas bearer
credential or following redirects.

Completion verifies the staged bytes against the declared size and digest,
then stores the verified bytes under a separate immutable final key. The
returned artifact includes its namespace, opaque artifact ID, filename, media
type, size and digest. Keep the namespace and artifact ID together for later
RPC reads. Uploading does not publish a model
change or attach the file to an entity.

Treat media type as declared presentation metadata. Successful completion
establishes byte integrity; it does not certify screenshot contents, image
dimensions, execution of a scenario, or the freshness of a capture.

## Upload from MCP

Use a writer-capable key and an appropriate authoring tool profile. The
control tools are:

- `get_artifact_storage_info` to discover the active backend and limits.
- `begin_artifact_upload` with the namespace, filename, media type, size, and
  SHA-256 digest.
- `complete_artifact_upload` with the namespace and returned upload ID.
- `get_artifact` for accepted metadata.
- `get_artifact_download` to create a direct S3 GET grant.

The begin response identifies the transfer method. For NATS,
`directUpload` is false or omitted in proto3 JSON, and no upload URL or headers
are returned.
Transfer the bytes with `UploadArtifactContent`, then call
`CompleteArtifactUpload`. For S3, `directUpload` is true and the response
contains a presigned PUT URL, required headers and expiry. Transfer the bytes
with that PUT, then complete through the RPC. Creating the URL is an Atlas
RPC operation; only the actual S3 transfer is an HTTP request.

Keep signed URLs transient; do not put them into model documents or logs.

A remote MCP server cannot read a file merely because a caller supplies a
local path. A caller with binary gRPC support can use
`UploadArtifactContent` for a NATS upload. Otherwise, run the Atlas CLI where
the file is available. For direct S3 uploads, use a file-capable HTTP helper
or the CLI. Preserve the returned namespace and artifact ID for subsequent
tool calls.

## Retrieve a file

Inspect metadata or download accepted bytes:

```sh
trogon-atlas artifacts get --namespace NAMESPACE_ID --id ARTIFACT_ID
trogon-atlas artifacts download \
  --namespace NAMESPACE_ID \
  --id ARTIFACT_ID \
  --output ./login-error.png
```

The CLI retrieves bytes through `GetArtifactContent` for either backend and
checks the downloaded size and digest before publishing the output file.
Reads require access to the artifact's namespace. Use `GetArtifact`
for metadata and `GetArtifactContent` for bytes, passing the namespace and
artifact ID. Each RPC checks current Atlas authorization.

`get_artifact_download` can return a short-lived S3 GET URL. A signed URL is a
temporary capability and remains usable until it expires, even if the
caller's Atlas key is revoked in the meantime. Use the authenticated
`GetArtifactContent` RPC when every retrieval must recheck current
authorization. The `GetArtifactDownload` RPC creates the signed S3 grant;
fetching its bytes is the subsequent HTTP GET.

For NATS storage, `GetArtifactDownload` returns `FAILED_PRECONDITION`
instructing the caller to use `GetArtifactContent`. NATS byte retrieval has
no download URL. The CLI handles authenticated retrieval through the RPC.

## Preserve an artifact for model attachment

Keep the returned namespace and artifact ID with the intended model subject.
The intended visual-model workflow associates artifacts with a UI, slice,
scenario or exact [connection occurrence](../reference/connection-identity.md),
including several named visual states for one interaction. Equal UI and
Command endpoints do not identify the same connection occurrence.

The artifact service currently provides storage, verification and RPC
retrieval. It does not add typed artifact fields to model entities, a
VisualState entity or a Studio state gallery. Model attachment requires a
separate typed contract and guarded mutation workflow; an accepted upload
does not create that association. Do not substitute an expiring signed URL
in a Document for a durable artifact attachment.

## Retain and clean up stored objects

Use a fresh immutable final key for each accepted artifact. NATS Object Store
overwrites an object's current contents when a name is reused; it does not
provide a capture revision history for that name. S3 bucket versioning is also
separate from the Atlas artifact identity.

Upload completion and model attachment are separate operations. A failed model
write can leave an accepted but unattached artifact, and a crashed upload can
leave staging data. The service currently has no artifact deletion or garbage
collection API or provider lifecycle tags. Monitor bucket capacity and manage
abandoned staging objects through a reviewed operator procedure that preserves
their catalog records. A signed PUT can start before expiry and finish after
it; expiry alone does not prove that a deleted staging key cannot reappear.
Retain cleanup evidence and revisit temporary objects after uncertain writes.

The proposed [artifact lifecycle and cleanup contract](../reference/artifact-lifecycle.md)
defines temporary markers, durable write receipts, cleanup claims, NATS
partial-chunk recovery and future reference-aware retention. Its automatic
worker and model-reference accounting are not implemented yet. Confirmation
continues to mean successful byte verification and catalog publication,
independently of model attachment.

Do not apply a blanket storage TTL to finalized bytes or remove final objects
without checking model, branch, and retained-history references. Back up the
artifact catalog together with the selected blob store. Presigned URL expiry
does not expire an accepted artifact's identity or bytes.
