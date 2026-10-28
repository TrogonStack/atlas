#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use std::sync::Arc;
use support::two_tenants::{authed,make_registry,serve_with_registry,ROOT_TOKEN};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics,service::EventModelServiceImpl};
use tonic::{Code,transport::Channel};
type Client=pb::event_model_service_client::EventModelServiceClient<Channel>;
fn id(ns:&str,s:&str)->pb::Id{pb::Id{namespace:ns.into(),slug:s.into(),version:1}}
fn owner(ns:&str,kind:pb::EntityKind,message:pb::ContractMemberRef)->pb::Entity{
 let claims=Some(pb::MessageClaims{claims:vec![pb::MessageClaim{direction:pb::CommunicationDirection::Receive as i32,message:Some(message),..Default::default()}],..Default::default()});
 pb::Entity{kind:Some(match kind{pb::EntityKind::Component=>pb::entity::Kind::Component(pb::Component{id:Some(id(ns,"owner")),title:"Owner".into(),message_claims:claims,..Default::default()}),pb::EntityKind::ExternalSystem=>pb::entity::Kind::ExternalSystem(pb::ExternalSystem{id:Some(id(ns,"owner")),title:"Owner".into(),message_claims:claims,..Default::default()}),_=>pb::entity::Kind::Domain(pb::Domain{id:Some(id(ns,"owner")),title:"Owner".into(),message_claims:claims,..Default::default()})}),..Default::default()}
}
async fn fixture()->(Client,String){
 let service=EventModelServiceImpl::try_new(Arc::new(trogon_atlas_testsupport::shared().await.store().await),ServerMetrics::default()).unwrap();
 let mut client=serve_with_registry(service,make_registry()).await;
 let ns=client.register_namespace(authed(pb::RegisterNamespaceRequest{name:"claims-acceptance".into(),parent:"commerce".into()},ROOT_TOKEN)).await.unwrap().into_inner().namespace.unwrap().id;
 (client,ns)
}
async fn put(client:&mut Client,e:pb::Entity,etag:&str)->Result<pb::PutEntityResponse,tonic::Status>{client.put_entity(authed(pb::PutEntityRequest{entity:Some(e),if_match:etag.into(),..Default::default()},ROOT_TOKEN)).await.map(tonic::Response::into_inner)}
fn unavailable_message(ns:&str)->pb::ContractMemberRef{pb::ContractMemberRef{specification:Some(pb::SpecificationRef{id:Some(id(ns,"missing"))}),owner_uid:"01930000-0000-7000-8000-000000000001".into(),kind:pb::ContractMemberKind::Message as i32,member_id:Some(pb::ContractMemberId{value:"01930000-0000-7000-8000-000000000002".into()}),expected_digest:None}}
#[tokio::test(flavor="multi_thread")]
async fn every_native_claim_owner_rejects_unavailable_exact_message(){
 let (mut c,ns)=fixture().await;
 for kind in [pb::EntityKind::Component,pb::EntityKind::ExternalSystem,pb::EntityKind::Domain]{
 let result=put(&mut c,owner(&ns,kind,unavailable_message(&ns)),"").await;
 assert!(result.is_err(),"{kind:?} accepted unavailable claim target");assert_eq!(result.unwrap_err().code(),Code::InvalidArgument);
 }
}
