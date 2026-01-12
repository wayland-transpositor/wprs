use wprs::server;
use wprs::server::backends::mock;
use wprs::prelude::*;
use wprs::protocols::wprs::Event;
use wprs::protocols::wprs::Endpoint;
use wprs::protocols::wprs::Request;
use wprs::protocols::wprs::Serializer;

fn main() -> Result<()> {
    let opts = mock::MockOptions::parse("wprsd_demo");

    let serializer = Serializer::<Request, Event>::new_server_endpoint(Endpoint::Unix {
        path: opts.socket_path.clone(),
    })
    .location(loc!())?;

    eprintln!("wprsd_demo listening on: {}", opts.socket_path.display());
    eprintln!("Connect with:");
    eprintln!(
        "  cargo run --bin wprsc -- --socket {}",
        opts.socket_path.display()
    );

    let backend = mock::MockBackend::new(opts);
    let tick_interval = backend.tick_interval();
    server::runtime::run_loop::run(backend, serializer, tick_interval).location(loc!())
}
