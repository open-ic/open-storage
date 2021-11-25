use crate::guards::caller_is_service_principal;
use crate::model::bucket_sync_state::EventToSync;
use crate::UserRecord;
use crate::{RuntimeState, RUNTIME_STATE};
use canister_api_macros::trace;
use ic_cdk_macros::update;
use index_canister::add_user::*;

#[update(guard = "caller_is_service_principal")]
#[trace]
fn add_user(args: Args) -> Response {
    RUNTIME_STATE.with(|state| add_user_impl(args, state.borrow_mut().as_mut().unwrap()))
}

fn add_user_impl(args: Args, runtime_state: &mut RuntimeState) -> Response {
    if runtime_state.data.users.contains_key(&args.user_id) {
        return Response::UserAlreadyExists;
    }

    runtime_state.data.users.insert(
        args.user_id,
        UserRecord {
            byte_limit: args.byte_limit,
            bytes_used: 0,
        },
    );

    runtime_state.data.buckets.sync_event(EventToSync::UserAdded(args.user_id));
    Response::Success
}
