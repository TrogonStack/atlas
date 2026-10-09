//! Connection setup and the small slice of the API this workspace calls.

use std::time::Duration;

use tonic::{
    metadata::{Ascii, MetadataValue},
    service::{interceptor::InterceptedService, Interceptor},
    transport::{Channel, ClientTlsConfig, Endpoint},
    Request, Status,
};

use crate::{
    error::AuthzedError,
    object::{ObjectId, ObjectRef, ObjectType, PresharedKey, Relation, Revision, SubjectRef},
    relationship::{RelationshipFilter, RelationshipUpdate},
    v1::{
        check_permission_response::Permissionship, consistency::Requirement,
        delete_relationships_response::DeletionProgress,
        permissions_service_client::PermissionsServiceClient,
        schema_service_client::SchemaServiceClient, CheckPermissionRequest,
        Consistency as ConsistencyProto, Cursor, DeleteRelationshipsRequest, LookupPermissionship,
        LookupResourcesRequest, ReadSchemaRequest, WriteRelationshipsRequest, WriteSchemaRequest,
    },
};

/// How fresh an answer has to be.
///
/// The variant to reach for by default is [`AtLeastAsFresh`](Consistency::AtLeastAsFresh)
/// carrying the revision of the caller's own last write. [`MinimizeLatency`](Consistency::MinimizeLatency)
/// is the fastest and is what invites the new-enemy problem: a caller can
/// revoke access and then, on a replica that has not caught up, still be
/// granted it. [`FullyConsistent`](Consistency::FullyConsistent) rules that
/// out at the cost of a quorum read on every call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Consistency {
    MinimizeLatency,
    AtLeastAsFresh(Revision),
    AtExactSnapshot(Revision),
    FullyConsistent,
}

impl Consistency {
    fn to_proto(&self) -> ConsistencyProto {
        ConsistencyProto {
            requirement: Some(match self {
                Self::MinimizeLatency => Requirement::MinimizeLatency(true),
                Self::AtLeastAsFresh(r) => Requirement::AtLeastAsFresh(r.to_proto()),
                Self::AtExactSnapshot(r) => Requirement::AtExactSnapshot(r.to_proto()),
                Self::FullyConsistent => Requirement::FullyConsistent(true),
            }),
        }
    }
}

/// The answer to one permission check, with the revision it was answered at.
///
/// A caveated result is reported as [`Conditional`](Permission::Conditional)
/// and is *not* permission. This client never sends caveat context, so a
/// conditional answer means the schema asked a question nobody answered, and
/// the only safe reading of that is no.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckOutcome {
    pub permission: Permission,
    pub checked_at: Option<Revision>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Granted,
    Denied,
    Conditional,
}

impl CheckOutcome {
    #[must_use]
    pub fn is_granted(&self) -> bool {
        self.permission == Permission::Granted
    }
}

/// Every resource of one type that a subject may act on, and the revision the
/// listing was taken at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupPage {
    pub resources: Vec<ObjectId>,
    pub looked_up_at: Option<Revision>,
}

#[derive(Debug, Clone)]
pub struct SpiceDbConfig {
    /// `http://host:50051` or `https://host:443`. The scheme decides TLS.
    pub endpoint: String,
    pub preshared_key: PresharedKey,
    pub request_timeout: Duration,
    /// Resources fetched per `LookupResources` round trip.
    pub page_size: u32,
    /// Ceiling on a single lookup's total result set.
    ///
    /// A lookup answers "everything this subject can see", which is unbounded
    /// by nature. Without a ceiling one misconfigured grant turns into an
    /// unbounded allocation in the request path.
    pub max_resources: usize,
}

impl SpiceDbConfig {
    #[must_use]
    pub fn new(endpoint: String, preshared_key: PresharedKey) -> Self {
        Self {
            endpoint,
            preshared_key,
            request_timeout: Duration::from_secs(5),
            page_size: 1000,
            max_resources: 100_000,
        }
    }
}

#[derive(Clone)]
struct Bearer(MetadataValue<Ascii>);

impl Interceptor for Bearer {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        request
            .metadata_mut()
            .insert("authorization", self.0.clone());
        Ok(request)
    }
}

type Permissions = PermissionsServiceClient<InterceptedService<Channel, Bearer>>;
type Schema = SchemaServiceClient<InterceptedService<Channel, Bearer>>;

#[derive(Clone)]
pub struct SpiceDb {
    permissions: Permissions,
    schema: Schema,
    page_size: u32,
    max_resources: usize,
}

impl std::fmt::Debug for SpiceDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpiceDb")
            .field("page_size", &self.page_size)
            .field("max_resources", &self.max_resources)
            .finish_non_exhaustive()
    }
}

impl SpiceDb {
    /// Connect lazily: this does not touch the network, so a SpiceDB that is
    /// slow to come up does not hold the server's startup hostage. The first
    /// failure surfaces on the first call.
    ///
    /// # Errors
    /// When the endpoint is not a usable URI, or TLS cannot be configured for
    /// an `https` endpoint.
    pub fn connect(config: &SpiceDbConfig) -> Result<Self, AuthzedError> {
        let mut endpoint = Endpoint::from_shared(config.endpoint.clone())
            .map_err(|e| AuthzedError::malformed("endpoint", e))?
            .timeout(config.request_timeout)
            .connect_timeout(config.request_timeout);
        if config.endpoint.starts_with("https://") {
            endpoint = endpoint
                .tls_config(ClientTlsConfig::new().with_native_roots())
                .map_err(|source| AuthzedError::Connect {
                    endpoint: config.endpoint.clone(),
                    source,
                })?;
        }
        let bearer = Bearer(
            MetadataValue::try_from(config.preshared_key.header_value())
                .map_err(|e| AuthzedError::malformed("preshared key", e))?,
        );
        let channel = endpoint.connect_lazy();
        Ok(Self {
            permissions: PermissionsServiceClient::with_interceptor(
                channel.clone(),
                bearer.clone(),
            ),
            schema: SchemaServiceClient::with_interceptor(channel, bearer),
            page_size: config.page_size,
            max_resources: config.max_resources,
        })
    }

    /// # Errors
    /// When SpiceDB is unreachable, refuses the request, or answers with a
    /// permissionship the API does not define.
    pub async fn check_permission(
        &self,
        resource: &ObjectRef,
        permission: &Relation,
        subject: &SubjectRef,
        consistency: &Consistency,
    ) -> Result<CheckOutcome, AuthzedError> {
        let response = self
            .permissions
            .clone()
            .check_permission(CheckPermissionRequest {
                consistency: Some(consistency.to_proto()),
                resource: Some(resource.to_proto()),
                permission: permission.as_str().to_string(),
                subject: Some(subject.to_proto()),
                context: None,
                with_tracing: false,
            })
            .await
            .map_err(|e| AuthzedError::rpc("CheckPermission", e))?
            .into_inner();

        let permission = match response.permissionship() {
            Permissionship::HasPermission => Permission::Granted,
            Permissionship::NoPermission => Permission::Denied,
            Permissionship::ConditionalPermission => Permission::Conditional,
            Permissionship::Unspecified => {
                return Err(AuthzedError::protocol(
                    "CheckPermission",
                    "permissionship was UNSPECIFIED, which the API says servers never return",
                ))
            }
        };
        Ok(CheckOutcome {
            permission,
            checked_at: Revision::from_proto(response.checked_at),
        })
    }

    /// Every resource of `resource_type` on which `subject` holds
    /// `permission`.
    ///
    /// Paged internally, and pinned: page one's revision becomes an exact
    /// snapshot for the pages after it. Without the pin, a write landing
    /// mid-listing can shift the underlying order and make the cursor skip or
    /// repeat resources, which for an authorization filter means silently
    /// dropping something the caller owns.
    ///
    /// # Errors
    /// When SpiceDB is unreachable or refuses, or when the result set exceeds
    /// the configured ceiling.
    pub async fn lookup_resources(
        &self,
        resource_type: &ObjectType,
        permission: &Relation,
        subject: &SubjectRef,
        consistency: &Consistency,
    ) -> Result<LookupPage, AuthzedError> {
        let mut resources = Vec::new();
        let mut cursor: Option<Cursor> = None;
        let mut snapshot: Option<Revision> = None;

        loop {
            let effective = match &snapshot {
                Some(revision) => Consistency::AtExactSnapshot(revision.clone()),
                None => consistency.clone(),
            };
            let mut stream = self
                .permissions
                .clone()
                .lookup_resources(LookupResourcesRequest {
                    consistency: Some(effective.to_proto()),
                    resource_object_type: resource_type.as_str().to_string(),
                    permission: permission.as_str().to_string(),
                    subject: Some(subject.to_proto()),
                    context: None,
                    optional_limit: self.page_size,
                    optional_cursor: cursor.take(),
                    with_debug: false,
                })
                .await
                .map_err(|e| AuthzedError::rpc("LookupResources", e))?
                .into_inner();

            let mut in_page = 0_u32;
            while let Some(item) = stream
                .message()
                .await
                .map_err(|e| AuthzedError::rpc("LookupResources", e))?
            {
                in_page += 1;
                if snapshot.is_none() {
                    snapshot = Revision::from_proto(item.looked_up_at.clone());
                }
                cursor.clone_from(&item.after_result_cursor);
                // A caveated resource is not a permitted resource; see
                // `CheckOutcome`.
                if item.permissionship() != LookupPermissionship::HasPermission {
                    continue;
                }
                if resources.len() >= self.max_resources {
                    return Err(AuthzedError::Overflow {
                        limit: self.max_resources,
                    });
                }
                resources.push(ObjectId::parse(&item.resource_object_id)?);
            }

            if in_page < self.page_size || cursor.is_none() {
                break;
            }
        }

        Ok(LookupPage {
            resources,
            looked_up_at: snapshot,
        })
    }

    /// # Errors
    /// When SpiceDB is unreachable or refuses the write, or answers without
    /// the revision it wrote at.
    pub async fn write_relationships(
        &self,
        updates: &[RelationshipUpdate],
    ) -> Result<Revision, AuthzedError> {
        let response = self
            .permissions
            .clone()
            .write_relationships(WriteRelationshipsRequest {
                updates: updates.iter().map(RelationshipUpdate::to_proto).collect(),
                optional_preconditions: Vec::new(),
                optional_transaction_metadata: None,
            })
            .await
            .map_err(|e| AuthzedError::rpc("WriteRelationships", e))?
            .into_inner();
        Revision::from_proto(response.written_at).ok_or_else(|| {
            AuthzedError::protocol("WriteRelationships", "response carried no ZedToken")
        })
    }

    /// Delete every relationship the filter matches, in one transaction.
    ///
    /// No limit is set, so SpiceDB either deletes the whole matching set or
    /// fails. A partial delete would leave an authorization state that is
    /// neither the old one nor the new one, which is worse than an error.
    ///
    /// # Errors
    /// When SpiceDB is unreachable or refuses, or answers without the
    /// revision it deleted at.
    pub async fn delete_relationships(
        &self,
        filter: &RelationshipFilter,
    ) -> Result<Revision, AuthzedError> {
        let response = self
            .permissions
            .clone()
            .delete_relationships(DeleteRelationshipsRequest {
                relationship_filter: Some(filter.to_proto()),
                optional_preconditions: Vec::new(),
                optional_limit: 0,
                optional_allow_partial_deletions: false,
                optional_transaction_metadata: None,
                optional_cursor: None,
            })
            .await
            .map_err(|e| AuthzedError::rpc("DeleteRelationships", e))?
            .into_inner();
        if response.deletion_progress() != DeletionProgress::Complete {
            return Err(AuthzedError::protocol(
                "DeleteRelationships",
                "deletion reported as incomplete despite no limit being set",
            ));
        }
        Revision::from_proto(response.deleted_at).ok_or_else(|| {
            AuthzedError::protocol("DeleteRelationships", "response carried no ZedToken")
        })
    }

    /// # Errors
    /// When SpiceDB is unreachable, or rejects the schema.
    pub async fn write_schema(&self, schema: &str) -> Result<Revision, AuthzedError> {
        let response = self
            .schema
            .clone()
            .write_schema(WriteSchemaRequest {
                schema: schema.to_string(),
            })
            .await
            .map_err(|e| AuthzedError::rpc("WriteSchema", e))?
            .into_inner();
        Revision::from_proto(response.written_at)
            .ok_or_else(|| AuthzedError::protocol("WriteSchema", "response carried no ZedToken"))
    }

    /// # Errors
    /// When SpiceDB is unreachable or refuses.
    pub async fn read_schema(&self) -> Result<String, AuthzedError> {
        Ok(self
            .schema
            .clone()
            .read_schema(ReadSchemaRequest {})
            .await
            .map_err(|e| AuthzedError::rpc("ReadSchema", e))?
            .into_inner()
            .schema_text)
    }
}
