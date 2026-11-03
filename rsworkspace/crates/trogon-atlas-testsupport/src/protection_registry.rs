use futures::StreamExt as _;

/// The protection registry bucket behind a test store, written to directly
/// to produce states the server must report as unknown or incomplete.
pub struct ProtectionRegistry {
    kv: async_nats::jetstream::kv::Store,
}

impl ProtectionRegistry {
    pub(crate) async fn open(url: &str, bucket: &str) -> Self {
        let client = async_nats::connect(url)
            .await
            .unwrap_or_else(|e| panic!("connect to test NATS: {e}"));
        let kv = async_nats::jetstream::new(client)
            .get_key_value(bucket)
            .await
            .unwrap_or_else(|e| panic!("open protection bucket {bucket}: {e}"));
        Self { kv }
    }

    /// Overwrite every stored row with bytes no row decodes from, returning
    /// the revision each now sits at.
    pub async fn corrupt_every_row(&self) -> Vec<u64> {
        let keys: Vec<String> = self
            .kv
            .keys()
            .await
            .unwrap_or_else(|e| panic!("list protection keys: {e}"))
            .map(|key| key.unwrap_or_else(|e| panic!("read protection key: {e}")))
            .collect()
            .await;
        let mut revisions = Vec::with_capacity(keys.len());
        for key in keys {
            let revision = self
                .kv
                .put(&key, vec![0xff, 0xff, 0xff, 0xff].into())
                .await
                .unwrap_or_else(|e| panic!("overwrite protection row {key}: {e}"));
            revisions.push(revision);
        }
        revisions
    }

    /// Store a key that names no identity, so a listing cannot account for
    /// it and must report itself incomplete.
    pub async fn add_unparseable_key(&self) {
        self.kv
            .put("zz.unparseable.key", b"{}".to_vec().into())
            .await
            .unwrap_or_else(|e| panic!("add unparseable protection key: {e}"));
    }
}
