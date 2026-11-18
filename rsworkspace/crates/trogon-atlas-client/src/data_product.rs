use anyhow::{Context, Result};
use serde_json::Value;
use trogon_atlas_proto as pb;

use crate::{
    client::Client,
    tenant_types::{self, TypeScope},
};

pub async fn inspect(client: &mut Client, id: pb::Id) -> Result<Value> {
    let mut scope = TypeScope::default();
    scope.add_namespace(id.namespace.clone());
    let response = client
        .inspect_data_product(pb::InspectDataProductRequest {
            product: Some(pb::DataProductRef { id: Some(id) }),
        })
        .await
        .context("InspectDataProduct failed")?
        .into_inner();
    tenant_types::transcode_pool(client, &scope)
        .await?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.InspectDataProductResponse",
            &response,
        )
        .context("encode DataProduct inspection")
}
