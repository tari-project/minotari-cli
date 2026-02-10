// Base Node Step Definitions
//
// Step definitions for managing base node processes in tests.

use cucumber::{given, then, when};

use super::common::MinotariWorld;

// =============================
// Base Node Steps
// =============================

#[given(expr = "I have a seed node {word}")]
#[when(expr = "I have a seed node {word}")]
async fn start_seed_node(world: &mut MinotariWorld, name: String) {
    // Dynamically load and use the spawn function
    use super::common::test_support;

    let base_dir = world.current_base_dir.as_ref().expect("Base dir not set").clone();

    let node = test_support::spawn_base_node(
        &base_dir,
        &mut world.assigned_ports,
        &mut world.base_nodes,
        true, // is_seed_node
        name.clone(),
        vec![], // no seed nodes for the first seed
    )
    .await;

    world.base_nodes.insert(name.clone(), node);
    world.seed_nodes.push(name);
}

#[given(expr = "I have a base node {word}")]
#[when(expr = "I have a base node {word}")]
async fn start_base_node(world: &mut MinotariWorld, name: String) {
    use super::common::test_support;

    let base_dir = world.current_base_dir.as_ref().expect("Base dir not set").clone();
    let seed_nodes = world.all_seed_nodes().to_vec();

    let node = test_support::spawn_base_node(
        &base_dir,
        &mut world.assigned_ports,
        &mut world.base_nodes,
        false, // not a seed node
        name.clone(),
        seed_nodes,
    )
    .await;

    world.base_nodes.insert(name, node);
}

#[given(expr = "I have a base node {word} connected to all seed nodes")]
#[when(expr = "I have a base node {word} connected to all seed nodes")]
async fn start_base_node_connected_to_seeds(world: &mut MinotariWorld, name: String) {
    start_base_node(world, name).await;
}

#[then("the base node should be running")]
async fn base_node_is_running(world: &mut MinotariWorld) {
    // Verify at least one base node is running
    assert!(!world.base_nodes.is_empty(), "No base nodes are running");

    // Verify the first node has valid ports
    if let Some((_, node)) = world.base_nodes.iter().next() {
        assert!(node.port > 0, "Invalid P2P port");
        assert!(node.grpc_port > 0, "Invalid GRPC port");
        assert!(node.http_port > 0, "Invalid HTTP port");
        println!(
            "Base node is running on ports - P2P: {}, GRPC: {}, HTTP: {}",
            node.port, node.grpc_port, node.http_port
        );
    }
}

#[when(expr = "I mine {int} blocks on {word}")]
async fn mine_blocks_on_node(world: &mut MinotariWorld, num_blocks: u64, node_name: String) {
    let node = world
        .base_nodes
        .get(&node_name)
        .expect(&format!("Node {} not found", node_name));

    // Use the wallet's address for mining rewards
    let wallet_address = world.wallet.get_comms_public_key().to_hex();

    node.mine_blocks(num_blocks, &wallet_address)
        .await
        .expect("Failed to mine blocks");

    // Store the current height for later verification
    let height = node.get_tip_height().await.expect("Failed to get tip height");
    world.last_command_output = Some(format!("Mined {} blocks, current height: {}", num_blocks, height));
    println!("Mined {} blocks on {}, current height: {}", num_blocks, node_name, height);
}

#[then(expr = "the chain height should be {int}")]
async fn chain_height_should_be(world: &mut MinotariWorld, expected_height: u64) {
    // Check the first base node
    if let Some((_, node)) = world.base_nodes.iter().next() {
        let actual_height = node.get_tip_height().await.expect("Failed to get tip height");
        assert_eq!(
            actual_height, expected_height,
            "Expected height {} but got {}",
            expected_height, actual_height
        );
        println!("Chain height verified: {}", actual_height);
    } else {
        panic!("No base nodes available");
    }
}

#[then(expr = "{word} should be at height {int}")]
async fn node_at_height(world: &mut MinotariWorld, node_name: String, expected_height: u64) {
    let node = world
        .base_nodes
        .get(&node_name)
        .expect(&format!("Node {} not found", node_name));

    node.wait_for_height(expected_height, 30)
        .await
        .expect(&format!(
            "Node {} failed to reach height {}",
            node_name, expected_height
        ));

    let actual_height = node.get_tip_height().await.expect("Failed to get tip height");
    println!("Node {} is at height {}", node_name, actual_height);
}
