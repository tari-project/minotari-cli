// Base Node Step Definitions
//
// Step definitions for managing base node processes in tests.

use cucumber::{given, when};

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
        true,  // is_seed_node
        name.clone(),
        vec![],  // no seed nodes for the first seed
    ).await;
    
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
        false,  // not a seed node
        name.clone(),
        seed_nodes,
    ).await;
    
    world.base_nodes.insert(name, node);
}

#[given(expr = "I have a base node {word} connected to all seed nodes")]
#[when(expr = "I have a base node {word} connected to all seed nodes")]
async fn start_base_node_connected_to_seeds(world: &mut MinotariWorld, name: String) {
    start_base_node(world, name).await;
}
