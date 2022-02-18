use candid::CandidType;
use serde::Deserialize;
use types::FileId;

#[derive(CandidType, Deserialize, Debug)]
pub struct Args {
    pub file_ids: Vec<FileId>,
}

#[derive(CandidType, Deserialize, Debug)]
pub struct Response {
    pub success: Vec<FileId>,
    pub failures: Vec<DeleteFileFailure>,
}

#[derive(CandidType, Deserialize, Debug)]
pub struct DeleteFileFailure {
    pub file_id: FileId,
    pub reason: DeleteFileFailureReason,
}

#[derive(CandidType, Deserialize, Debug)]
pub enum DeleteFileFailureReason {
    NotFound,
    NotAuthorized,
}
