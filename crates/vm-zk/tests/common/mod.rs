use std::sync::Once;

use tracing_subscriber::{EnvFilter, fmt};

static INIT_TRACING: Once = Once::new();

pub fn init_tracing() {
    INIT_TRACING.call_once(|| {
        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("mpz_vm_zk=info"));
        let _ = fmt()
            .with_env_filter(filter)
            .with_test_writer()
            .with_target(true)
            .try_init();
    });
}
