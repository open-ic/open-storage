use candid::CandidType;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use types::{CanisterId, Version};

#[derive(CandidType, Serialize, Deserialize)]
pub struct FailedUpgrade {
    pub canister_id: CanisterId,
    pub from_version: Version,
    pub to_version: Version,
}

#[derive(CandidType, Serialize, Deserialize, Default)]
pub struct CanistersRequiringUpgrade {
    pending: VecDeque<CanisterId>,
    in_progress: HashSet<CanisterId>,
    failed: VecDeque<FailedUpgrade>,
}

impl CanistersRequiringUpgrade {
    pub fn enqueue(&mut self, canister_id: CanisterId) {
        self.pending.push_back(canister_id);
    }

    pub fn try_take_next(&mut self) -> Option<CanisterId> {
        let canister_id = self.pending.pop_front()?;
        self.in_progress.insert(canister_id);
        Some(canister_id)
    }

    pub fn mark_success(&mut self, canister_id: &CanisterId) {
        self.in_progress.remove(canister_id);
    }

    pub fn mark_failure(&mut self, failed_upgrade: FailedUpgrade) {
        self.in_progress.remove(&failed_upgrade.canister_id);
        self.failed.push_back(failed_upgrade);
    }

    pub fn is_in_progress(&self, canister_id: &CanisterId) -> bool {
        self.in_progress.contains(canister_id)
    }

    pub fn count_in_progress(&self) -> u32 {
        self.in_progress.len() as u32
    }

    pub fn remove(&mut self, canister_id: &CanisterId) {
        self.pending.retain(|id| id != canister_id);
        self.in_progress.remove(canister_id);
        self.failed.retain(|pu| &pu.canister_id != canister_id);
    }

    pub fn metrics(&self) -> Metrics {
        let mut failed = Vec::new();
        for ((from_version, to_version), group) in &self.failed.iter().group_by(|f| (f.from_version, f.to_version)) {
            failed.push(FailedUpgradeCount {
                from_version,
                to_version,
                count: group.count(),
            })
        }
        failed.sort_unstable_by_key(|f| (f.from_version, f.to_version));

        Metrics {
            pending: self.pending.len(),
            in_progress: self.in_progress.len(),
            failed,
        }
    }
}

pub struct Metrics {
    pub pending: usize,
    pub in_progress: usize,
    pub failed: Vec<FailedUpgradeCount>,
}

#[derive(CandidType, Serialize, Debug)]
pub struct FailedUpgradeCount {
    pub from_version: Version,
    pub to_version: Version,
    pub count: usize,
}
