//! Compilation Service stdio/Unix child. The product CLI never constructs this
//! handler in-process; it supervises this binary over JSONL.

use std::io::{self, BufReader};

use apxm_compilation_service::{
    CompilationService, UnixEndpoint, capture_confinement_readiness, serve_stdio, serve_unix,
};

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
    // The service compiles submitted source by evaluating it inside an
    // authoring frontend. What the kernel confines that evaluation to is the
    // first thing an operator needs to know, so it is reported before the first
    // request rather than inferred from a refused compile. Standard output
    // carries the protocol, so readiness goes to standard error.
    match serde_json::to_string(&capture_confinement_readiness()) {
        Ok(readiness) => eprintln!("{readiness}"),
        Err(error) => {
            eprintln!("the capture confinement readiness could not be reported: {error}");
            std::process::exit(1);
        }
    }

    let service = CompilationService::from_env();
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
