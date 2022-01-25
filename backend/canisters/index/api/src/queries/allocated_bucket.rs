use candid::CandidType;
use serde::Deserialize;
use types::{CanisterId, Hash};

#[derive(CandidType, Deserialize, Debug)]
pub struct Args {
    pub blob_hash: Hash,
    pub blob_size: u64,
}

#[derive(CandidType, Deserialize, Debug)]
pub enum Response {
    Success(Result),
    AllowanceReached,
    UserNotFound,
    BucketUnavailable,
}

#[derive(CandidType, Deserialize, Debug)]
pub struct Result {
    pub canister_id: CanisterId,
    pub chunk_size: u32,
}

use crate::allocated_bucket_v2 as v2;

impl From<Args> for v2::Args {
    fn from(args: Args) -> Self {
        Self {
            file_hash: args.blob_hash,
            file_size: args.blob_size,
        }
    }
}
