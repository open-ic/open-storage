use crate::guards::caller_is_index_canister;
use crate::model::files::RemoveFileResult;
use crate::model::index_sync_state::EventToSync;
use crate::{mutate_state, RuntimeState, MAX_EVENTS_TO_SYNC_PER_BATCH};
use bucket_canister::c2c_sync_index::{Response::*, *};
use canister_api_macros::trace;
use ic_cdk_macros::update;
use types::FileRemoved;

#[update(guard = "caller_is_index_canister")]
#[trace]
fn c2c_sync_index(args: Args) -> Response {
    mutate_state(|state| c2c_sync_index_impl(args, state))
}

fn c2c_sync_index_impl(args: Args, runtime_state: &mut RuntimeState) -> Response {
    for user_id in args.users_added {
        runtime_state.data.users.add(user_id);
    }

    let mut files_removed: Vec<FileRemoved> = Vec::new();

    for user_id in args.users_removed {
        if let Some(user) = runtime_state.data.users.remove(user_id) {
            for file_id in user.files_owned() {
                if let RemoveFileResult::Success(b) = runtime_state.data.files.remove(user_id, file_id) {
                    files_removed.push(b)
                }
            }
        }
    }

    for accessor_id in args.accessors_removed {
        files_removed.extend(runtime_state.data.files.remove_accessor(&accessor_id));
    }

    if files_removed.len() > MAX_EVENTS_TO_SYNC_PER_BATCH {
        // If there are too many events to sync in a single batch, queue the excess events to be
        // synced later via heartbeat
        let excess = files_removed.split_off(MAX_EVENTS_TO_SYNC_PER_BATCH);

        for removed in excess {
            runtime_state.data.index_sync_state.enqueue(EventToSync::FileRemoved(removed));
        }
    }

    Success(SuccessResult { files_removed })
}
