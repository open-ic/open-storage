use bucket_canister::upload_chunk::Args as UploadChunkArgs;
use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;
use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::collections::{HashMap, HashSet};
use types::{AccessorId, BlobId, BlobReferenceAdded, BlobReferenceRemoved, Hash, TimestampMillis, UserId};
use utils::hasher::hash_bytes;

#[derive(Serialize, Deserialize, Default)]
pub struct Blobs {
    blob_references: HashMap<BlobId, BlobReference>,
    pending_blobs: HashMap<BlobId, PendingBlob>,
    reference_counts: ReferenceCounts,
    accessors_map: AccessorsMap,
    // TODO move this to stable memory
    data: HashMap<Hash, ByteBuf>,
}

#[derive(Serialize, Deserialize)]
pub struct BlobReference {
    pub uploaded_by: UserId,
    pub accessors: HashSet<AccessorId>,
    pub hash: Hash,
    pub created: TimestampMillis,
}

impl Blobs {
    pub fn uploaded_by(&self, blob_id: &BlobId) -> Option<UserId> {
        self.blob_references
            .get(blob_id)
            .map(|b| b.uploaded_by)
            .or_else(|| self.pending_blobs.get(blob_id).map(|b| b.uploaded_by))
    }

    pub fn put_chunk(&mut self, args: PutChunkArgs) -> PutChunkResult {
        if self.blob_references.contains_key(&args.blob_id) {
            return PutChunkResult::BlobAlreadyExists;
        }

        let blob_id = args.blob_id;
        let now = args.now;
        let mut blob_reference_added = None;

        let completed_blob: Option<PendingBlob> = match self.pending_blobs.entry(blob_id) {
            Vacant(e) => {
                blob_reference_added = Some(BlobReferenceAdded {
                    uploaded_by: args.uploaded_by,
                    blob_id,
                    blob_hash: args.hash,
                    blob_size: args.total_size,
                });
                let pending_blob: PendingBlob = args.into();
                if pending_blob.is_completed() {
                    Some(pending_blob)
                } else {
                    e.insert(pending_blob);
                    None
                }
            }
            Occupied(mut e) => {
                let pending_blob = e.get_mut();
                if !pending_blob.add_chunk(args.chunk_index, args.bytes) {
                    return PutChunkResult::ChunkAlreadyExists;
                }
                if pending_blob.is_completed() {
                    Some(e.remove())
                } else {
                    None
                }
            }
        };

        let mut blob_completed = false;
        if let Some(completed_blob) = completed_blob {
            let hash = hash_bytes(&completed_blob.bytes);
            if hash != completed_blob.hash {
                return PutChunkResult::HashMismatch(HashMismatch {
                    provided_hash: completed_blob.hash,
                    actual_hash: hash,
                });
            }
            self.insert_completed_blob(blob_id, completed_blob, now);
            blob_completed = true;
        }

        PutChunkResult::Success(PutChunkResultSuccess {
            blob_completed,
            blob_reference_added,
        })
    }

    pub fn remove_blob_reference(&mut self, uploaded_by: UserId, blob_id: BlobId) -> RemoveBlobReferenceResult {
        if let Occupied(e) = self.blob_references.entry(blob_id) {
            if e.get().uploaded_by != uploaded_by {
                RemoveBlobReferenceResult::NotAuthorized
            } else {
                let blob_reference = e.remove();
                for accessor_id in blob_reference.accessors.iter() {
                    self.accessors_map.unlink(*accessor_id, &blob_id);
                }

                let mut blob_deleted = false;
                if self.reference_counts.decr(blob_reference.hash) == 0 {
                    self.data.remove(&blob_reference.hash);
                    blob_deleted = true;
                }

                RemoveBlobReferenceResult::Success(BlobReferenceRemoved {
                    uploaded_by,
                    blob_hash: blob_reference.hash,
                    blob_deleted,
                })
            }
        } else {
            RemoveBlobReferenceResult::NotFound
        }
    }

    pub fn remove_pending_blob(&mut self, blob_id: &BlobId) -> bool {
        self.pending_blobs.remove(blob_id).is_some()
    }

    pub fn remove_accessor(&mut self, accessor_id: &AccessorId) -> Vec<BlobReferenceRemoved> {
        let mut blob_references_removed = Vec::new();

        if let Some(blob_ids) = self.accessors_map.remove(accessor_id) {
            for blob_id in blob_ids {
                if let Occupied(mut e) = self.blob_references.entry(blob_id) {
                    let blob_reference = e.get_mut();
                    blob_reference.accessors.remove(accessor_id);
                    if blob_reference.accessors.is_empty() {
                        let delete_blob = self.reference_counts.decr(blob_reference.hash) == 0;
                        if delete_blob {
                            self.data.remove(&blob_reference.hash);
                        }
                        let blob_reference = e.remove();
                        blob_references_removed.push(BlobReferenceRemoved {
                            uploaded_by: blob_reference.uploaded_by,
                            blob_hash: blob_reference.hash,
                            blob_deleted: delete_blob,
                        });
                    }
                }
            }
        }

        blob_references_removed
    }

    pub fn contains_hash(&self, hash: &Hash) -> bool {
        self.data.contains_key(hash)
    }

    fn insert_completed_blob(&mut self, blob_id: BlobId, completed_blob: PendingBlob, now: TimestampMillis) {
        for accessor_id in completed_blob.accessors.iter() {
            self.accessors_map.link(*accessor_id, blob_id);
        }

        self.blob_references.insert(
            blob_id,
            BlobReference {
                uploaded_by: completed_blob.uploaded_by,
                accessors: completed_blob.accessors,
                hash: completed_blob.hash,
                created: now,
            },
        );
        self.reference_counts.incr(completed_blob.hash);
        self.data.entry(completed_blob.hash).or_insert(completed_blob.bytes);
    }
}

#[derive(Serialize, Deserialize, Default)]
struct ReferenceCounts {
    counts: HashMap<Hash, u32>,
}

impl ReferenceCounts {
    pub fn incr(&mut self, hash: Hash) -> u32 {
        *self
            .counts
            .entry(hash)
            .and_modify(|c| {
                *c += 1;
            })
            .or_insert(1)
    }

    pub fn decr(&mut self, hash: Hash) -> u32 {
        if let Occupied(mut e) = self.counts.entry(hash) {
            let count = e.get_mut();
            if *count > 1 {
                *count -= 1;
                return *count;
            } else {
                e.remove();
            }
        }
        0
    }
}

#[derive(Serialize, Deserialize, Default)]
struct AccessorsMap {
    map: HashMap<AccessorId, HashSet<BlobId>>,
}

impl AccessorsMap {
    pub fn link(&mut self, accessor_id: AccessorId, blob_id: BlobId) {
        self.map.entry(accessor_id).or_default().insert(blob_id);
    }

    pub fn unlink(&mut self, accessor_id: AccessorId, blob_id: &BlobId) {
        if let Occupied(mut e) = self.map.entry(accessor_id) {
            let entry = e.get_mut();
            entry.remove(blob_id);
            if entry.is_empty() {
                e.remove();
            }
        }
    }

    pub fn remove(&mut self, accessor_id: &AccessorId) -> Option<HashSet<BlobId>> {
        self.map.remove(accessor_id)
    }
}

#[derive(Serialize, Deserialize)]
struct PendingBlob {
    uploaded_by: UserId,
    created: TimestampMillis,
    hash: Hash,
    mime_type: String,
    accessors: HashSet<AccessorId>,
    chunk_size: u32,
    total_size: u64,
    remaining_chunks: HashSet<u32>,
    bytes: ByteBuf,
}

impl PendingBlob {
    pub fn add_chunk(&mut self, chunk_index: u32, bytes: ByteBuf) -> bool {
        if self.remaining_chunks.remove(&chunk_index) {
            let start_index = self.chunk_size * chunk_index;
            for (index, byte) in bytes.into_iter().enumerate().map(|(i, b)| (i + start_index as usize, b)) {
                self.bytes[index] = byte;
            }
            true
        } else {
            false
        }
    }

    pub fn is_completed(&self) -> bool {
        self.remaining_chunks.is_empty()
    }
}

pub struct PutChunkArgs {
    uploaded_by: UserId,
    blob_id: BlobId,
    hash: Hash,
    mime_type: String,
    accessors: Vec<AccessorId>,
    chunk_index: u32,
    chunk_size: u32,
    total_size: u64,
    bytes: ByteBuf,
    now: TimestampMillis,
}

impl PutChunkArgs {
    pub fn new(uploaded_by: UserId, now: TimestampMillis, upload_chunk_args: UploadChunkArgs) -> Self {
        Self {
            uploaded_by,
            blob_id: upload_chunk_args.blob_id,
            hash: upload_chunk_args.hash,
            mime_type: upload_chunk_args.mime_type,
            accessors: upload_chunk_args.accessors,
            chunk_index: upload_chunk_args.chunk_index,
            chunk_size: upload_chunk_args.chunk_size,
            total_size: upload_chunk_args.total_size,
            bytes: upload_chunk_args.bytes,
            now,
        }
    }
}

impl From<PutChunkArgs> for PendingBlob {
    fn from(args: PutChunkArgs) -> Self {
        let chunk_count = (((args.total_size - 1) / (args.chunk_size as u64)) + 1) as u32;

        let mut pending_blob = Self {
            uploaded_by: args.uploaded_by,
            created: args.now,
            hash: args.hash,
            mime_type: args.mime_type,
            accessors: args.accessors.into_iter().collect(),
            chunk_size: args.chunk_size,
            total_size: args.total_size,
            remaining_chunks: (0..chunk_count).into_iter().collect(),
            bytes: ByteBuf::with_capacity(args.total_size as usize),
        };
        pending_blob.add_chunk(args.chunk_index, args.bytes);
        pending_blob
    }
}

pub enum PutChunkResult {
    Success(PutChunkResultSuccess),
    BlobAlreadyExists,
    ChunkAlreadyExists,
    HashMismatch(HashMismatch),
}

pub struct PutChunkResultSuccess {
    pub blob_completed: bool,
    pub blob_reference_added: Option<BlobReferenceAdded>,
}

pub enum RemoveBlobReferenceResult {
    Success(BlobReferenceRemoved),
    NotAuthorized,
    NotFound,
}

pub struct HashMismatch {
    pub provided_hash: Hash,
    pub actual_hash: Hash,
}