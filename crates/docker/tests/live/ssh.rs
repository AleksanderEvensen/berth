//! Read-only checks against a real engine over SSH. Nothing is created,
//! changed or run in a container, so any host you can `ssh` into will do.
//! Run: BERTH_TEST_SSH_HOST=ssh://my-host cargo test -p berth-docker --test live ssh -- --ignored

use berth_docker::{BollardEngine, ContainerState, Endpoint, Engine, LogOptions, parse_host};
use futures::{StreamExt as _, executor::block_on};

use super::TestResult;

#[test]
#[ignore = "requires BERTH_TEST_SSH_HOST and key or agent login to that host"]
fn ssh_engine_reads_like_a_local_one() -> TestResult<()> {
    let host = std::env::var("BERTH_TEST_SSH_HOST")?;
    let endpoint = parse_host(&host)?;
    let engine = BollardEngine::connect(&endpoint)?;

    let info = block_on(engine.info())?;
    let Endpoint::Ssh { hostname, .. } = &info.endpoint else {
        return Err(format!("not an SSH endpoint: {}", info.endpoint).into());
    };
    println!("{} {} at {host} ({hostname:?})", info.name, info.version);

    let containers = block_on(engine.list_containers())?;
    block_on(engine.list_images())?;
    block_on(engine.list_volumes())?;
    block_on(engine.list_networks())?;
    println!("{} containers", containers.len());

    // Streams: each holds its own ssh process next to the pooled ones.
    if let Some(running) = containers
        .iter()
        .find(|c| c.state == ContainerState::Running)
    {
        let options = LogOptions {
            follow: false,
            tail: Some(5),
            since: None,
        };
        let logs: Vec<_> = block_on(engine.logs(&running.id, options).collect());
        for line in logs {
            line?;
        }
        let sample = block_on(engine.stats(&running.id).next()).ok_or("no stats sample")?;
        sample?;
    }
    Ok(())
}
