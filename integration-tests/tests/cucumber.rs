use cucumber::World;

mod steps;

#[tokio::test]
async fn run_cucumber_tests() {
    steps::MinotariWorld::cucumber()
        .max_concurrent_scenarios(1)
        .run("features/")
        .await;
}
