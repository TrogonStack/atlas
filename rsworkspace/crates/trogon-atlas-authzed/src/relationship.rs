//! The relationship tuple, and the three things that can be done to one.

use crate::{
    object::{ObjectId, ObjectRef, ObjectType, Relation, SubjectRef},
    v1,
};

/// `resource#relation@subject`, the only fact SpiceDB stores.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Relationship {
    pub resource: ObjectRef,
    pub relation: Relation,
    pub subject: SubjectRef,
}

impl Relationship {
    #[must_use]
    pub fn new(resource: ObjectRef, relation: Relation, subject: SubjectRef) -> Self {
        Self {
            resource,
            relation,
            subject,
        }
    }

    pub(crate) fn to_proto(&self) -> v1::Relationship {
        v1::Relationship {
            resource: Some(self.resource.to_proto()),
            relation: self.relation.as_str().to_string(),
            subject: Some(self.subject.to_proto()),
            optional_caveat: None,
            optional_expires_at: None,
        }
    }
}

impl std::fmt::Display for Relationship {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{}@{}", self.resource, self.relation, self.subject)
    }
}

/// `Create` errors if the relationship exists, `Touch` upserts, `Delete`
/// no-ops if it is already gone.
///
/// The distinction is load-bearing for anything that must not clobber: a
/// registration racing another registration wants `Create` so the loser is
/// told, while a repair pass wants `Touch` so re-running it is free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationshipOp {
    Create,
    Touch,
    Delete,
}

impl RelationshipOp {
    fn to_proto(self) -> v1::relationship_update::Operation {
        match self {
            Self::Create => v1::relationship_update::Operation::Create,
            Self::Touch => v1::relationship_update::Operation::Touch,
            Self::Delete => v1::relationship_update::Operation::Delete,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipUpdate {
    pub op: RelationshipOp,
    pub relationship: Relationship,
}

impl RelationshipUpdate {
    #[must_use]
    pub fn new(op: RelationshipOp, relationship: Relationship) -> Self {
        Self { op, relationship }
    }

    pub(crate) fn to_proto(&self) -> v1::RelationshipUpdate {
        v1::RelationshipUpdate {
            operation: self.op.to_proto() as i32,
            relationship: Some(self.relationship.to_proto()),
        }
    }
}

/// Which relationships a bulk delete matches.
///
/// Everything left unset widens the match, so the resource type alone would
/// delete every relationship of that type. Narrow deliberately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipFilter {
    resource_type: ObjectType,
    resource_id: Option<ObjectId>,
    relation: Option<Relation>,
    subject: Option<SubjectRef>,
}

impl RelationshipFilter {
    #[must_use]
    pub fn new(resource_type: ObjectType) -> Self {
        Self {
            resource_type,
            resource_id: None,
            relation: None,
            subject: None,
        }
    }

    #[must_use]
    pub fn on(mut self, resource_id: ObjectId) -> Self {
        self.resource_id = Some(resource_id);
        self
    }

    #[must_use]
    pub fn with_relation(mut self, relation: Relation) -> Self {
        self.relation = Some(relation);
        self
    }

    #[must_use]
    pub fn with_subject(mut self, subject: SubjectRef) -> Self {
        self.subject = Some(subject);
        self
    }

    pub(crate) fn to_proto(&self) -> v1::RelationshipFilter {
        v1::RelationshipFilter {
            resource_type: self.resource_type.as_str().to_string(),
            optional_resource_id: self
                .resource_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            optional_resource_id_prefix: String::new(),
            optional_relation: self
                .relation
                .as_ref()
                .map(|r| r.as_str().to_string())
                .unwrap_or_default(),
            optional_subject_filter: self.subject.as_ref().map(|subject| v1::SubjectFilter {
                subject_type: subject.object.object_type.as_str().to_string(),
                optional_subject_id: subject.object.object_id.as_str().to_string(),
                optional_relation: Some(v1::subject_filter::RelationFilter {
                    relation: subject
                        .relation
                        .as_ref()
                        .map(|relation| relation.as_str().to_string())
                        .unwrap_or_default(),
                }),
            }),
        }
    }
}

impl std::fmt::Display for RelationshipFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:", self.resource_type)?;
        match &self.resource_id {
            Some(id) => write!(f, "{id}")?,
            None => f.write_str("*")?,
        }
        match &self.relation {
            Some(relation) => write!(f, "#{relation}")?,
            None => f.write_str("#*")?,
        }
        match &self.subject {
            Some(subject) => write!(f, "@{subject}"),
            None => f.write_str("@*"),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn alice() -> ObjectRef {
        ObjectRef::new(
            ObjectType::parse("user").unwrap(),
            ObjectId::parse("alice").unwrap(),
        )
    }

    fn subject_relation_of(filter: &RelationshipFilter) -> Option<String> {
        filter
            .to_proto()
            .optional_subject_filter
            .unwrap()
            .optional_relation
            .map(|relation| relation.relation)
    }

    #[test]
    fn a_direct_subject_filter_matches_only_the_direct_subject() {
        let filter = RelationshipFilter::new(ObjectType::parse("organization").unwrap())
            .with_subject(SubjectRef::new(alice()));

        assert_eq!(subject_relation_of(&filter), Some(String::new()));
    }

    #[test]
    fn a_subject_set_filter_matches_that_relation() {
        let filter =
            RelationshipFilter::new(ObjectType::parse("organization").unwrap()).with_subject(
                SubjectRef::with_relation(alice(), Relation::parse("member").unwrap()),
            );

        assert_eq!(subject_relation_of(&filter), Some("member".to_string()));
    }
}
