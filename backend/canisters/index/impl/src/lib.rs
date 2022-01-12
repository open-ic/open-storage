use crate::model::blob_buckets::BlobBuckets;
use crate::model::buckets::Buckets;
use candid::{CandidType, Principal};
use canister_logger::LogMessagesWrapper;
use canister_state_macros::state_operations;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use types::{
    BlobReferenceAdded, BlobReferenceRejected, BlobReferenceRejectedReason, BlobReferenceRemoved, CanisterId, CanisterWasm,
    Cycles, Timestamped, UserId, Version,
};
use utils::canister::CanistersRequiringUpgrade;
use utils::env::Environment;

mod guards;
mod lifecycle;
mod model;
mod queries;
mod updates;

const DEFAULT_CHUNK_SIZE_BYTES: u32 = 1 << 19; // 1/2 Mb
const MAX_EVENTS_TO_SYNC_PER_BATCH: usize = 10000;
const STATE_VERSION: StateVersion = StateVersion::V1;
const MIN_CYCLES_BALANCE: Cycles = 10_000_000_000_000; // 10T
const BUCKET_CANISTER_TOP_UP_AMOUNT: Cycles = 1_000_000_000_000; // 1T

#[derive(CandidType, Serialize, Deserialize)]
enum StateVersion {
    V1,
}

thread_local! {
    static RUNTIME_STATE: RefCell<Option<RuntimeState>> = RefCell::default();
    static LOG_MESSAGES: RefCell<LogMessagesWrapper> = RefCell::default();
    static WASM_VERSION: RefCell<Timestamped<Version>> = RefCell::default();
}

state_operations!();

struct RuntimeState {
    pub env: Box<dyn Environment>,
    pub data: Data,
}

impl RuntimeState {
    pub fn new(env: Box<dyn Environment>, data: Data) -> RuntimeState {
        RuntimeState { env, data }
    }

    pub fn is_caller_service_principal(&self) -> bool {
        let caller = self.env.caller();
        self.data.service_principals.contains(&caller)
    }

    pub fn is_caller_bucket(&self) -> bool {
        let caller = self.env.caller();
        self.data.buckets.get(&caller).is_some()
    }
}

#[derive(Serialize, Deserialize)]
struct Data {
    pub service_principals: HashSet<Principal>,
    pub bucket_canister_wasm: CanisterWasm,
    pub users: HashMap<UserId, UserRecord>,
    pub blob_buckets: BlobBuckets,
    pub buckets: Buckets,
    pub canisters_requiring_upgrade: CanistersRequiringUpgrade,
    #[serde(default)]
    pub total_cycles_spent_on_canisters: Cycles,
    pub test_mode: bool,
}

impl Data {
    fn new(service_principals: Vec<Principal>, bucket_canister_wasm: CanisterWasm, test_mode: bool) -> Data {
        Data {
            service_principals: service_principals.into_iter().collect(),
            bucket_canister_wasm,
            users: HashMap::new(),
            blob_buckets: BlobBuckets::default(),
            buckets: Buckets::default(),
            canisters_requiring_upgrade: CanistersRequiringUpgrade::default(),
            total_cycles_spent_on_canisters: 0,
            test_mode,
        }
    }

    pub fn add_blob_reference(
        &mut self,
        bucket: CanisterId,
        br_added: BlobReferenceAdded,
    ) -> Result<(), BlobReferenceRejected> {
        if let Some(user) = self.users.get_mut(&br_added.uploaded_by) {
            if user.bytes_used + br_added.blob_size > user.byte_limit {
                return Err(BlobReferenceRejected {
                    blob_id: br_added.blob_id,
                    reason: BlobReferenceRejectedReason::AllowanceReached,
                });
            } else {
                user.bytes_used += br_added.blob_size;
            }
        } else {
            return Err(BlobReferenceRejected {
                blob_id: br_added.blob_id,
                reason: BlobReferenceRejectedReason::UserNotFound,
            });
        }

        self.blob_buckets.add(br_added.blob_hash, br_added.blob_size, bucket);
        Ok(())
    }

    pub fn remove_blob_reference(&mut self, bucket: CanisterId, br_removed: BlobReferenceRemoved) {
        let blob_size = if br_removed.blob_deleted {
            self.blob_buckets.remove(br_removed.blob_hash, bucket)
        } else {
            self.blob_buckets.get(&br_removed.blob_hash).map(|r| r.size)
        };

        if let Some(blob_size) = blob_size {
            if let Some(user) = self.users.get_mut(&br_removed.uploaded_by) {
                user.bytes_used -= blob_size;
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct UserRecord {
    pub byte_limit: u64,
    pub bytes_used: u64,
}
