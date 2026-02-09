// Mock Base Node HTTP Server for Integration Testing
//
// This module provides a lightweight mock HTTP server that simulates
// a Tari base node's REST API for testing wallet scanning and operations.

use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::task::JoinHandle;

/// Mock base node server state
#[derive(Clone)]
pub struct MockBaseNode {
    /// Current chain height
    pub tip_height: Arc<Mutex<u64>>,
    /// Stored blocks by height
    pub blocks: Arc<Mutex<HashMap<u64, Value>>>,
    /// Server handle for shutdown
    pub server_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Port the server is running on
    pub port: u16,
}

impl MockBaseNode {
    /// Create a new mock base node
    pub fn new(port: u16) -> Self {
        Self {
            tip_height: Arc::new(Mutex::new(0)),
            blocks: Arc::new(Mutex::new(HashMap::new())),
            server_handle: Arc::new(Mutex::new(None)),
            port,
        }
    }

    /// Start the mock server
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        let app = self.create_router();
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));

        let server = tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .expect("Failed to bind mock server");
            
            axum::serve(listener, app)
                .await
                .expect("Failed to run mock server");
        });

        *self.server_handle.lock().unwrap() = Some(server);

        // Wait a bit for server to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        Ok(())
    }

    /// Create the router with all endpoints
    fn create_router(&self) -> Router {
        let state = self.clone();

        Router::new()
            .route("/tip_info", get(tip_info_handler))
            .route("/base_node/blocks/:height", get(get_block_handler))
            .route("/headers/:height", get(get_header_handler))
            .with_state(state)
    }

    /// Add a test block at a specific height
    pub fn add_block(&self, height: u64, block_data: Value) {
        self.blocks.lock().unwrap().insert(height, block_data);
        
        // Update tip if this is higher
        let mut tip = self.tip_height.lock().unwrap();
        if height > *tip {
            *tip = height;
        }
    }

    /// Set the chain tip height
    pub fn set_tip_height(&self, height: u64) {
        *self.tip_height.lock().unwrap() = height;
    }

    /// Stop the mock server
    pub async fn stop(&self) {
        if let Some(handle) = self.server_handle.lock().unwrap().take() {
            handle.abort();
        }
    }
}

impl Drop for MockBaseNode {
    fn drop(&mut self) {
        if let Some(handle) = self.server_handle.lock().unwrap().take() {
            handle.abort();
        }
    }
}

/// Handler for /tip_info endpoint
async fn tip_info_handler(
    axum::extract::State(state): axum::extract::State<MockBaseNode>,
) -> impl IntoResponse {
    let tip_height = *state.tip_height.lock().unwrap();
    
    Json(json!({
        "height": tip_height,
        "hash": format!("0000000000000000000000000000000000000000000000000000000000000{:03}", tip_height),
        "timestamp": 1234567890 + (tip_height * 120),
    }))
}

/// Handler for /base_node/blocks/:height endpoint
async fn get_block_handler(
    Path(height): Path<u64>,
    axum::extract::State(state): axum::extract::State<MockBaseNode>,
) -> impl IntoResponse {
    let blocks = state.blocks.lock().unwrap();
    
    if let Some(block) = blocks.get(&height) {
        (StatusCode::OK, Json(block.clone()))
    } else {
        // Return empty block if not found
        (
            StatusCode::OK,
            Json(json!({
                "header": {
                    "height": height,
                    "prev_hash": format!("0000000000000000000000000000000000000000000000000000000000000{:03}", height.saturating_sub(1)),
                    "hash": format!("0000000000000000000000000000000000000000000000000000000000000{:03}", height),
                    "timestamp": 1234567890 + (height * 120),
                },
                "body": {
                    "outputs": [],
                    "kernels": [],
                },
            })),
        )
    }
}

/// Handler for /headers/:height endpoint  
async fn get_header_handler(
    Path(height): Path<u64>,
    axum::extract::State(_state): axum::extract::State<MockBaseNode>,
) -> impl IntoResponse {
    Json(json!({
        "height": height,
        "prev_hash": format!("0000000000000000000000000000000000000000000000000000000000000{:03}", height.saturating_sub(1)),
        "hash": format!("0000000000000000000000000000000000000000000000000000000000000{:03}", height),
        "timestamp": 1234567890 + (height * 120),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_server_starts() {
        let mock_node = MockBaseNode::new(18080);
        mock_node.start().await.unwrap();
        
        // Test that we can reach the server
        let client = reqwest::Client::new();
        let response = client
            .get("http://127.0.0.1:18080/tip_info")
            .send()
            .await
            .unwrap();
        
        assert_eq!(response.status(), 200);
        mock_node.stop().await;
    }

    #[tokio::test]
    async fn test_tip_info() {
        let mock_node = MockBaseNode::new(18081);
        mock_node.set_tip_height(100);
        mock_node.start().await.unwrap();
        
        let client = reqwest::Client::new();
        let response: Value = client
            .get("http://127.0.0.1:18081/tip_info")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        
        assert_eq!(response["height"], 100);
        mock_node.stop().await;
    }

    #[tokio::test]
    async fn test_get_block() {
        let mock_node = MockBaseNode::new(18082);
        
        // Add a test block
        mock_node.add_block(
            50,
            json!({
                "header": {"height": 50},
                "body": {"outputs": [{"value": 1000}]}
            }),
        );
        
        mock_node.start().await.unwrap();
        
        let client = reqwest::Client::new();
        let response: Value = client
            .get("http://127.0.0.1:18082/base_node/blocks/50")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        
        assert_eq!(response["header"]["height"], 50);
        assert_eq!(response["body"]["outputs"][0]["value"], 1000);
        
        mock_node.stop().await;
    }
}
