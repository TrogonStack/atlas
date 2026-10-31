use trogon_atlas_types::TypeUrl;

/// A built-in annotation message, recognized only by its exact protobuf
/// full name so a look-alike type from another package never passes for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BuiltinAnnotation(&'static str);

impl BuiltinAnnotation {
    pub(crate) const ASSUMPTION: Self =
        Self("trogonatlas.annotation.v1alpha1.AssumptionAnnotation");
    pub(crate) const CODE_REF: Self = Self("trogonatlas.annotation.v1alpha1.CodeRefAnnotation");
    pub(crate) const DERIVED_FIELD: Self =
        Self("trogonatlas.eventmodel.v1alpha1.DerivedFieldAnnotation");
    pub(crate) const LIFECYCLE: Self = Self("trogonatlas.annotation.v1alpha1.LifecycleAnnotation");
    pub(crate) const MIXED_SURFACE: Self =
        Self("trogonatlas.eventmodel.v1alpha1.MixedSurfaceAnnotation");
    pub(crate) const OPEN_QUESTION: Self =
        Self("trogonatlas.annotation.v1alpha1.OpenQuestionAnnotation");
    pub(crate) const ORPHAN: Self = Self("trogonatlas.eventmodel.v1alpha1.OrphanAnnotation");
    pub(crate) const PERSONAL_DATA: Self =
        Self("trogonatlas.eventmodel.v1alpha1.PersonalDataAnnotation");
    pub(crate) const PINNED_REF: Self = Self("trogonatlas.eventmodel.v1alpha1.PinnedRefAnnotation");
    pub(crate) const SHARED_CONSUMERS: Self =
        Self("trogonatlas.eventmodel.v1alpha1.SharedConsumersAnnotation");
    pub(crate) const UUIDV5_IDENTITY: Self =
        Self("trogonatlas.eventmodel.v1alpha1.Uuidv5IdentityAnnotation");

    pub(crate) fn matches(self, any: &prost_types::Any) -> bool {
        TypeUrl::parse(&any.type_url).is_ok_and(|url| url.full_name() == self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn any(type_url: &str) -> prost_types::Any {
        prost_types::Any {
            type_url: type_url.into(),
            value: Vec::new(),
        }
    }

    #[test]
    fn matches_the_exact_message_under_any_host() {
        for url in [
            "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.OrphanAnnotation",
            "example.com/types/trogonatlas.eventmodel.v1alpha1.OrphanAnnotation",
        ] {
            assert!(BuiltinAnnotation::ORPHAN.matches(&any(url)), "{url}");
        }
    }

    #[test]
    fn rejects_names_that_only_share_a_suffix() {
        for url in [
            "type.googleapis.com/acme.trogonatlas.eventmodel.v1alpha1.OrphanAnnotation",
            "type.googleapis.com/acmetrogonatlas.eventmodel.v1alpha1.OrphanAnnotation",
            "trogonatlas.eventmodel.v1alpha1.OrphanAnnotation",
        ] {
            assert!(!BuiltinAnnotation::ORPHAN.matches(&any(url)), "{url}");
        }
    }
}
