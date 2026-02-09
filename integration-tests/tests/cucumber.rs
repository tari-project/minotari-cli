use cucumber::World;
use std::path::PathBuf;

mod steps;

#[tokio::main]
async fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let features_path = manifest_dir.join("features");

    steps::MinotariWorld::cucumber()
        .max_concurrent_scenarios(1)
        .run(features_path)
        .await;
}
