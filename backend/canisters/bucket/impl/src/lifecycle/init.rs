use crate::lifecycle::{init_logger, init_state};
use crate::Data;
use bucket_canister::init::Args;
use canister_api_macros::trace;
use canister_logger::set_panic_hook;
use ic_cdk_macros::init;
use tracing::info;
use utils::env::canister::CanisterEnv;
use utils::env::Environment;

#[init]
#[trace]
fn init(args: Args) {
    set_panic_hook();
    init_logger(args.test_mode);

    let env = Box::new(CanisterEnv::new());

    let index_canister_id = env.caller();

    let data = Data::new(index_canister_id, env.now());

    init_state(env, data, args.wasm_version);

    info!("Initialization complete");
}
