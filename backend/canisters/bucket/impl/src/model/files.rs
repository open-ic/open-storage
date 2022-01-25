use crate::{calc_chunk_count, DATA_LIMIT_BYTES, MAX_BLOB_SIZE_BYTES};
use bucket_canister::upload_chunk_v2::Args as UploadChunkArgs;
use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;
use std::cmp::Ordering;
use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::collections::{HashMap, HashSet};
use types::{AccessorId, FileAdded, FileId, FileRemoved, Hash, TimestampMillis, UserId};
use utils::hasher::hash_bytes;

#[derive(Serialize, Deserialize, Default)]
pub struct Files {
    files: HashMap<FileId, File>,
    pending_files: HashMap<FileId, PendingFile>,
    reference_counts: ReferenceCounts,
    accessors_map: AccessorsMap,
    // TODO move this to stable memory
    blobs: HashMap<Hash, ByteBuf>,
    bytes_used: u64,
}

#[derive(Serialize, Deserialize)]
pub struct File {
    pub uploaded_by: UserId,
    pub created: TimestampMillis,
    pub accessors: HashSet<AccessorId>,
    pub hash: Hash,
    pub mime_type: String,
}

impl Files {
    pub fn get(&self, file_id: &FileId) -> Option<&File> {
        self.files.get(file_id)
    }

    pub fn pending_file(&self, file_id: &FileId) -> Option<&PendingFile> {
        self.pending_files.get(file_id)
    }

    pub fn blob_bytes(&self, hash: &Hash) -> Option<&ByteBuf> {
        self.blobs.get(hash)
    }

    pub fn uploaded_by(&self, file_id: &FileId) -> Option<UserId> {
        self.files
            .get(file_id)
            .map(|f| f.uploaded_by)
            .or_else(|| self.pending_files.get(file_id).map(|f| f.uploaded_by))
    }

    pub fn put_chunk(&mut self, args: PutChunkArgs) -> PutChunkResult {
        if args.total_size > MAX_BLOB_SIZE_BYTES {
            return PutChunkResult::FileTooBig(MAX_BLOB_SIZE_BYTES);
        }

        if self.files.contains_key(&args.file_id) {
            return PutChunkResult::FileAlreadyExists;
        }

        let file_id = args.file_id;
        let now = args.now;
        let mut file_added = None;

        let completed_file: Option<PendingFile> = match self.pending_files.entry(file_id) {
            Vacant(e) => {
                file_added = Some(FileAdded {
                    uploaded_by: args.uploaded_by,
                    file_id,
                    hash: args.hash,
                    size: args.total_size,
                });
                let pending_file: PendingFile = args.into();
                if pending_file.is_completed() {
                    Some(pending_file)
                } else {
                    e.insert(pending_file);
                    None
                }
            }
            Occupied(mut e) => {
                let pending_file = e.get_mut();
                match pending_file.add_chunk(args.chunk_index, args.bytes) {
                    AddChunkResult::Success => {}
                    AddChunkResult::ChunkIndexTooHigh => return PutChunkResult::ChunkIndexTooHigh,
                    AddChunkResult::ChunkAlreadyExists => return PutChunkResult::ChunkAlreadyExists,
                    AddChunkResult::ChunkSizeMismatch(m) => return PutChunkResult::ChunkSizeMismatch(m),
                }
                if pending_file.is_completed() {
                    Some(e.remove())
                } else {
                    None
                }
            }
        };

        let mut file_completed = false;
        if let Some(completed_file) = completed_file {
            let hash = hash_bytes(&completed_file.bytes);
            if hash != completed_file.hash {
                return PutChunkResult::HashMismatch(HashMismatch {
                    provided_hash: completed_file.hash,
                    actual_hash: hash,
                    chunk_count: completed_file.chunk_count(),
                });
            }
            self.insert_completed_file(file_id, completed_file, now);
            file_completed = true;
        }

        PutChunkResult::Success(PutChunkResultSuccess {
            file_completed,
            file_added,
        })
    }

    pub fn remove(&mut self, uploaded_by: UserId, file_id: FileId) -> RemoveFileResult {
        if let Occupied(e) = self.files.entry(file_id) {
            if e.get().uploaded_by != uploaded_by {
                RemoveFileResult::NotAuthorized
            } else {
                let file = e.remove();
                for accessor_id in file.accessors.iter() {
                    self.accessors_map.unlink(*accessor_id, &file_id);
                }

                let mut blob_deleted = false;
                if self.reference_counts.decr(file.hash) == 0 {
                    self.remove_blob(&file.hash);
                    blob_deleted = true;
                }

                RemoveFileResult::Success(FileRemoved {
                    file_id,
                    uploaded_by,
                    hash: file.hash,
                    blob_deleted,
                })
            }
        } else {
            RemoveFileResult::NotFound
        }
    }

    pub fn remove_pending_file(&mut self, file_id: &FileId) -> bool {
        self.pending_files.remove(file_id).is_some()
    }

    pub fn remove_accessor(&mut self, accessor_id: &AccessorId) -> Vec<FileRemoved> {
        let mut files_removed = Vec::new();

        if let Some(file_ids) = self.accessors_map.remove(accessor_id) {
            for file_id in file_ids {
                let mut blob_to_delete = None;
                if let Occupied(mut e) = self.files.entry(file_id) {
                    let file = e.get_mut();
                    file.accessors.remove(accessor_id);
                    if file.accessors.is_empty() {
                        let delete_blob = self.reference_counts.decr(file.hash) == 0;
                        if delete_blob {
                            blob_to_delete = Some(file.hash);
                        }
                        let file = e.remove();
                        files_removed.push(FileRemoved {
                            file_id,
                            uploaded_by: file.uploaded_by,
                            hash: file.hash,
                            blob_deleted: delete_blob,
                        });
                    }
                }

                if let Some(blob_to_delete) = blob_to_delete {
                    self.remove_blob(&blob_to_delete);
                }
            }
        }

        files_removed
    }

    pub fn contains_hash(&self, hash: &Hash) -> bool {
        self.blobs.contains_key(hash)
    }

    pub fn data_size(&self, hash: &Hash) -> Option<u64> {
        self.blobs.get(hash).map(|b| b.len() as u64)
    }

    pub fn bytes_remaining(&self) -> i64 {
        (DATA_LIMIT_BYTES as i64) - (self.bytes_used as i64)
    }

    pub fn reference_counts(&self) -> HashMap<Hash, Vec<UserId>> {
        let mut reference_counts: HashMap<Hash, Vec<UserId>> = HashMap::new();

        for file in self.files.values() {
            reference_counts.entry(file.hash).or_default().push(file.uploaded_by);
        }

        reference_counts
    }

    pub fn metrics(&self) -> Metrics {
        Metrics {
            file_count: self.files.len() as u32,
            blob_count: self.blobs.len() as u32,
        }
    }

    fn insert_completed_file(&mut self, file_id: FileId, completed_file: PendingFile, now: TimestampMillis) {
        for accessor_id in completed_file.accessors.iter() {
            self.accessors_map.link(*accessor_id, file_id);
        }

        self.files.insert(
            file_id,
            File {
                uploaded_by: completed_file.uploaded_by,
                created: now,
                accessors: completed_file.accessors,
                hash: completed_file.hash,
                mime_type: completed_file.mime_type,
            },
        );
        self.reference_counts.incr(completed_file.hash);
        self.add_blob_if_not_exists(completed_file.hash, completed_file.bytes);
    }

    fn add_blob_if_not_exists(&mut self, hash: Hash, bytes: ByteBuf) {
        if let Vacant(e) = self.blobs.entry(hash) {
            self.bytes_used = self
                .bytes_used
                .checked_add(bytes.len() as u64)
                .expect("'bytes_used' overflowed");

            e.insert(bytes);
        }
    }

    fn remove_blob(&mut self, hash: &Hash) {
        if let Some(bytes) = self.blobs.remove(hash) {
            self.bytes_used = self
                .bytes_used
                .checked_sub(bytes.len() as u64)
                .expect("'bytes used' underflowed");
        }
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
    map: HashMap<AccessorId, HashSet<FileId>>,
}

impl AccessorsMap {
    pub fn link(&mut self, accessor_id: AccessorId, file_id: FileId) {
        self.map.entry(accessor_id).or_default().insert(file_id);
    }

    pub fn unlink(&mut self, accessor_id: AccessorId, file_id: &FileId) {
        if let Occupied(mut e) = self.map.entry(accessor_id) {
            let entry = e.get_mut();
            entry.remove(file_id);
            if entry.is_empty() {
                e.remove();
            }
        }
    }

    pub fn remove(&mut self, accessor_id: &AccessorId) -> Option<HashSet<FileId>> {
        self.map.remove(accessor_id)
    }
}

#[derive(Serialize, Deserialize)]
pub struct PendingFile {
    pub uploaded_by: UserId,
    pub created: TimestampMillis,
    pub hash: Hash,
    pub mime_type: String,
    pub accessors: HashSet<AccessorId>,
    pub chunk_size: u32,
    pub total_size: u64,
    pub remaining_chunks: HashSet<u32>,
    pub bytes: ByteBuf,
}

impl PendingFile {
    pub fn add_chunk(&mut self, chunk_index: u32, bytes: ByteBuf) -> AddChunkResult {
        if self.remaining_chunks.remove(&chunk_index) {
            let actual_chunk_size = bytes.len() as u32;
            if let Some(expected_chunk_size) = self.expected_chunk_size(chunk_index) {
                if expected_chunk_size != actual_chunk_size {
                    return AddChunkResult::ChunkSizeMismatch(ChunkSizeMismatch {
                        expected_size: expected_chunk_size,
                        actual_size: actual_chunk_size,
                    });
                }
            } else {
                return AddChunkResult::ChunkIndexTooHigh;
            }

            // TODO: Improve performance by copying a block of memory in one go
            let start_index = self.chunk_size as usize * chunk_index as usize;
            for (index, byte) in bytes.into_iter().enumerate().map(|(i, b)| (i + start_index, b)) {
                self.bytes[index] = byte;
            }
            AddChunkResult::Success
        } else {
            AddChunkResult::ChunkAlreadyExists
        }
    }

    pub fn chunk_count(&self) -> u32 {
        calc_chunk_count(self.chunk_size, self.total_size)
    }

    pub fn is_completed(&self) -> bool {
        self.remaining_chunks.is_empty()
    }

    fn expected_chunk_size(&self, chunk_index: u32) -> Option<u32> {
        let last_index = self.chunk_count() - 1;
        match chunk_index.cmp(&last_index) {
            Ordering::Equal => Some(((self.total_size - 1) % self.chunk_size as u64) as u32 + 1),
            Ordering::Less => Some(self.chunk_size),
            Ordering::Greater => None,
        }
    }
}

pub enum AddChunkResult {
    Success,
    ChunkAlreadyExists,
    ChunkIndexTooHigh,
    ChunkSizeMismatch(ChunkSizeMismatch),
}

pub struct PutChunkArgs {
    uploaded_by: UserId,
    file_id: FileId,
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
    pub fn new(uploaded_by: UserId, upload_chunk_args: UploadChunkArgs, now: TimestampMillis) -> Self {
        Self {
            uploaded_by,
            file_id: upload_chunk_args.file_id,
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

impl From<PutChunkArgs> for PendingFile {
    fn from(args: PutChunkArgs) -> Self {
        let chunk_count = calc_chunk_count(args.chunk_size, args.total_size);

        let mut pending_file = Self {
            uploaded_by: args.uploaded_by,
            created: args.now,
            hash: args.hash,
            mime_type: args.mime_type,
            accessors: args.accessors.into_iter().collect(),
            chunk_size: args.chunk_size,
            total_size: args.total_size,
            remaining_chunks: (0..chunk_count).into_iter().collect(),
            bytes: ByteBuf::from(vec![0; args.total_size as usize]),
        };
        pending_file.add_chunk(args.chunk_index, args.bytes);
        pending_file
    }
}

pub enum PutChunkResult {
    Success(PutChunkResultSuccess),
    FileAlreadyExists,
    FileTooBig(u64),
    ChunkAlreadyExists,
    ChunkIndexTooHigh,
    ChunkSizeMismatch(ChunkSizeMismatch),
    HashMismatch(HashMismatch),
}

pub struct PutChunkResultSuccess {
    pub file_completed: bool,
    pub file_added: Option<FileAdded>,
}

pub enum RemoveFileResult {
    Success(FileRemoved),
    NotAuthorized,
    NotFound,
}

pub struct HashMismatch {
    pub provided_hash: Hash,
    pub actual_hash: Hash,
    pub chunk_count: u32,
}

pub struct ChunkSizeMismatch {
    pub expected_size: u32,
    pub actual_size: u32,
}

pub struct Metrics {
    pub file_count: u32,
    pub blob_count: u32,
}
