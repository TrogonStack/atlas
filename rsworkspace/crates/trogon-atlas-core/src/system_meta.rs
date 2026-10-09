//! Server-owned incarnation identity for entities (the `SystemMeta`
//! block). Modeled after Kubernetes' `metadata.uid`: assigned on create,
//! preserved across updates, regenerated on delete + recreate.
//!
//! Lives in `trogon-atlas-core` rather than `trogon-atlas-store` so any
//! third-party storage backend can apply the same policy by calling
//! `stamp_system` before persisting; the rule is shape-of-the-entity,
//! not storage-mechanism.

use trogon_atlas_proto::{Entity, SystemMeta};

/// Returns a clone of `entity` with the `system` block set:
///
/// * On update (a `prev` exists with a non-empty (after trim) `uid`), the prior
///   incarnation's identity is preserved verbatim.
/// * Otherwise a fresh `UUIDv7` + ISO-8601 `created_at` is minted.
///
/// Client-provided `system` blocks are always overwritten; the server
/// (and the storage layer that delegates to it) owns this field.
#[must_use]
pub fn stamp_system(prev: Option<&Entity>, entity: &Entity) -> Entity {
    let mut e = entity.clone();
    e.system = Some(match prev.and_then(|p| p.system.as_ref()) {
        Some(s) if !s.uid.trim().is_empty() => s.clone(),
        _ => SystemMeta {
            uid: uuid::Uuid::now_v7().to_string(),
            created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        },
    });
    e
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn bare_entity() -> Entity {
        Entity {
            system: None,
            kind: None,
        }
    }

    #[test]
    fn create_path_mints_non_empty_uid_and_parseable_created_at() {
        let result = stamp_system(None, &bare_entity());
        let sys = result.system.unwrap();
        assert!(!sys.uid.is_empty(), "uid must be non-empty on create");
        chrono::DateTime::parse_from_rfc3339(&sys.created_at)
            .expect("created_at must be valid RFC3339");
    }

    #[test]
    fn update_path_preserves_prior_uid() {
        let prev_sys = SystemMeta {
            uid: "prior-uid-abc".into(),
            created_at: "2025-01-01T00:00:00Z".into(),
        };
        let prev = Entity {
            system: Some(prev_sys.clone()),
            kind: None,
        };
        let result = stamp_system(Some(&prev), &bare_entity());
        let sys = result.system.unwrap();
        assert_eq!(sys.uid, "prior-uid-abc");
        assert_eq!(sys.created_at, "2025-01-01T00:00:00Z");
    }

    #[test]
    fn update_with_empty_uid_mints_new() {
        let prev_sys = SystemMeta {
            uid: String::new(),
            created_at: String::new(),
        };
        let prev = Entity {
            system: Some(prev_sys),
            kind: None,
        };
        let result = stamp_system(Some(&prev), &bare_entity());
        let sys = result.system.unwrap();
        assert!(!sys.uid.is_empty(), "empty prev uid must trigger new mint");
        chrono::DateTime::parse_from_rfc3339(&sys.created_at).expect("created_at must be RFC3339");
    }

    #[test]
    fn client_provided_system_is_overwritten() {
        let client_sys = SystemMeta {
            uid: "client-uid".into(),
            created_at: "client-ts".into(),
        };
        let entity = Entity {
            system: Some(client_sys),
            kind: None,
        };
        let result = stamp_system(None, &entity);
        let sys = result.system.unwrap();
        assert_ne!(
            sys.uid, "client-uid",
            "client-provided system must be overwritten"
        );
    }

    #[test]
    fn bug_whitespace_uid_must_mint_new() {
        // Empty uid already mints; whitespace-only is equally non-identity and
        // must not be preserved as a "valid" incarnation id.
        let prev = Entity {
            system: Some(SystemMeta {
                uid: "   ".into(),
                created_at: "2025-01-01T00:00:00Z".into(),
            }),
            kind: None,
        };
        let result = stamp_system(Some(&prev), &bare_entity());
        let sys = result.system.unwrap();
        assert_ne!(sys.uid, "   ", "whitespace uid must not be preserved");
        assert_ne!(sys.uid.trim(), "");
    }

    #[test]
    fn update_overwrites_client_system_with_prev() {
        let prev = Entity {
            system: Some(SystemMeta {
                uid: "prior-uid".into(),
                created_at: "2025-01-01T00:00:00Z".into(),
            }),
            kind: None,
        };
        let client = Entity {
            system: Some(SystemMeta {
                uid: "client-uid".into(),
                created_at: "client-ts".into(),
            }),
            kind: None,
        };
        let result = stamp_system(Some(&prev), &client);
        let sys = result.system.unwrap();
        assert_eq!(sys.uid, "prior-uid");
        assert_ne!(sys.uid, "client-uid");
    }
}
