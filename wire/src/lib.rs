//! The client ↔ backend wire contract: `proto/exilence.proto`, a verbatim
//! copy of `docs/spec/exilence.proto` in `exilence-ce/migration`, compiled to
//! prost messages and tonic clients and servers. Shared with
//! `exilence-ce-ng-backend`, which pins this crate by a client release tag.

pub mod v1 {
    tonic::include_proto!("exilence.v1");
}

impl v1::ItemDefinition {
    /// The prost encoding with `id` left 0, so field 1 is absent: the bytes
    /// [`Self::content_id`] hashes and that storage keeps next to the id.
    pub fn content_bytes(&self) -> Vec<u8> {
        let mut bytes = prost::Message::encode_to_vec(self);
        // prost writes fields in number order, so a set `id` is the first nine
        // bytes: the fixed64 tag byte and its eight value bytes.
        if self.id != 0 {
            bytes.drain(..9);
        }
        bytes
    }

    /// xxh3-64 of [`Self::content_bytes`]: the id every definition is known
    /// by, whatever its `id` field holds.
    pub fn content_id(&self) -> u64 {
        xxhash_rust::xxh3::xxh3_64(&self.content_bytes())
    }

    /// Both [`Self::content_bytes`] and [`Self::content_id`] from a single encode.
    pub fn content_bytes_and_id(&self) -> (Vec<u8>, u64) {
        let bytes = self.content_bytes();
        let id = xxhash_rust::xxh3::xxh3_64(&bytes);
        (bytes, id)
    }
}
