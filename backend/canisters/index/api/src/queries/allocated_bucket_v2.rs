use crate::allocated_bucket;
use crate::ProjectedAllowance;
use candid::CandidType;
use serde::Deserialize;
use types::{CanisterId, Hash};

#[derive(CandidType, Deserialize, Debug)]
pub struct Args {
    pub file_hash: Hash,
    pub file_size: u64,
}

#[derive(CandidType, Deserialize, Debug)]
pub enum Response {
    Success(SuccessResult),
    AllowanceExceeded(ProjectedAllowance),
    UserNotFound,
    BucketUnavailable,
}

#[derive(CandidType, Deserialize, Debug)]
pub struct SuccessResult {
    pub canister_id: CanisterId,
    pub chunk_size: u32,
    pub byte_limit: u64,
    pub bytes_used: u64,
    pub bytes_used_after_upload: u64,
    pub projected_allowance: ProjectedAllowance,
}

impl From<allocated_bucket::Response> for Response {
    fn from(response: allocated_bucket::Response) -> Self {
        match response {
            allocated_bucket::Response::AllowanceExceeded(pa) => Response::AllowanceExceeded(pa),
            allocated_bucket::Response::BucketUnavailable => Response::BucketUnavailable,
            allocated_bucket::Response::UserNotFound => Response::UserNotFound,
            allocated_bucket::Response::Success(sr) => Response::Success(SuccessResult {
                canister_id: sr.canister_id,
                chunk_size: sr.chunk_size,
                byte_limit: sr.projected_allowance.byte_limit,
                bytes_used: sr.projected_allowance.bytes_used,
                bytes_used_after_upload: sr.projected_allowance.bytes_used_after_operation,
                projected_allowance: sr.projected_allowance,
            }),
        }
    }
}
