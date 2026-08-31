//! Runtime Service stdio/Unix child. The product CLI never constructs this
//! handler in-process; it supervises this binary over JSONL.

use std::io::{self, BufReader};
use std::sync::Arc;

use apxm_commit_local::ReadAuthorizationBinding;
use apxm_runtime_service::{RuntimeService, UnixEndpoint, serve_stdio, serve_unix};

fn main() {
    let mut socket = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => {
                socket = Some(args.next().unwrap_or_default());
            }
            value if value.starts_with("--socket=") => {
                socket = Some(value[9..].to_owned());
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }
    let service = match RuntimeService::try_from_env() {
        Ok(service) => service,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    // A standalone child remains deny-by-default.  An owner-composed caller
    // may inject an exact typed read binding through the transport environment;
    // this does not make APXM a product authorization authority.
    let service = match ReadAuthorizationBinding::from_env() {
        Ok(Some(binding)) => {
            let scope_ref = binding.scope_ref.as_str().to_owned();
            service
                .with_read_access_hook(Arc::new(binding))
                .with_output_access_scope_ref(scope_ref)
        }
        Ok(None) => service,
        Err(error) => {
            eprintln!("invalid runtime read authorization binding: {error}");
            std::process::exit(1);
        }
    };
    let result = if let Some(path) = socket {
        if let Err(error) = UnixEndpoint::new(path.clone()) {
            eprintln!("{error}");
            std::process::exit(2);
        }
        serve_unix(&path, service)
    } else {
        serve_stdio(BufReader::new(io::stdin()), io::stdout(), service)
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
