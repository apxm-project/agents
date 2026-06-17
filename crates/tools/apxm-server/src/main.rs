//! APXM Server binary entrypoint.

// SSE streaming creates cross-thread alloc/free patterns where mimalloc beats the system allocator.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> anyhow::Result<()> {
    apxm_server::run()
}
