use crate::lifecycle::{init_logger, init_state, BUFFER_SIZE};
use crate::{Data, LOG_MESSAGES};
use bucket_canister::post_upgrade::Args;
use canister_api_macros::trace;
use canister_logger::{set_panic_hook, LogMessage, LogMessagesWrapper};
use ic_cdk::api::stable::BufferedStableReader;
use ic_cdk_macros::post_upgrade;
use tracing::info;
use utils::env::canister::CanisterEnv;

#[post_upgrade]
#[trace]
fn post_upgrade(args: Args) {
    set_panic_hook();

    let env = Box::new(CanisterEnv::new());
    let reader = BufferedStableReader::new(BUFFER_SIZE);

    let (data, log_messages, trace_messages): (Data, Vec<LogMessage>, Vec<LogMessage>) =
        serializer::deserialize(reader).unwrap();

    init_logger(data.test_mode);
    init_state(env, data, args.wasm_version);

    if !log_messages.is_empty() || !trace_messages.is_empty() {
        LOG_MESSAGES.with(|l| rehydrate_log_messages(log_messages, trace_messages, &l.borrow()))
    }

    info!(version = %args.wasm_version, "Post-upgrade complete");
}

fn rehydrate_log_messages(
    log_messages: Vec<LogMessage>,
    trace_messages: Vec<LogMessage>,
    messages_container: &LogMessagesWrapper,
) {
    for message in log_messages {
        messages_container.logs.push(message);
    }

    for message in trace_messages {
        messages_container.traces.push(message);
    }
}
