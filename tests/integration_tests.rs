use cucumber::World;

#[path = "cucumber/steps.rs"]
mod steps;

#[tokio::main]
async fn main() {
    steps::MinotariWorld::cucumber()
        .max_concurrent_scenarios(1)
        .run("tests/cucumber/features/")
        .await;
}
