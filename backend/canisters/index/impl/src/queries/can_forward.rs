use crate::{read_state, RuntimeState};
use canister_api_macros::trace;
use ic_cdk_macros::query;
use index_canister::{
    can_forward::{Response::*, *},
    ProjectedAllowance,
};

#[query]
#[trace]
fn reference_counts(args: Args) -> Response {
    read_state(|state| reference_counts_impl(args, state))
}

fn reference_counts_impl(args: Args, runtime_state: &RuntimeState) -> Response {
    let user_id = runtime_state.env.caller();
    if let Some(user) = runtime_state.data.users.get(&user_id) {
        let user_owns_blob = runtime_state.data.blobs.user_owns_blob(&user_id, &args.file_hash);

        let bytes_used_after_operation = if user_owns_blob { user.bytes_used } else { user.bytes_used + args.file_size };

        let projected_allowance = ProjectedAllowance {
            bytes_used: user.bytes_used,
            byte_limit: user.byte_limit,
            bytes_used_after_upload: bytes_used_after_operation,
            bytes_used_after_operation,
        };

        if user.byte_limit >= bytes_used_after_operation {
            Success(projected_allowance)
        } else {
            AllowanceExceeded(projected_allowance)
        }
    } else {
        UserNotFound
    }
}
