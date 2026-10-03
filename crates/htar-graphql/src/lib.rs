use async_graphql_parser::{parse_query, types::{ExecutableDocument, Selection}};
use futures::stream::{FuturesUnordered, StreamExt};
use reqwest::Client;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::warn;

pub struct GraphqlFederationEngine {
    client: Client,
    max_depth: usize,
    max_complexity: usize,
    // Using Arc-swap or an internal read-optimized map lets you drop `&mut self` 
    // so this engine can be safely shared across your web worker threads.
    mappings: Arc<HashMap<String, GraphqlFieldMapping>>,
}

#[derive(Clone, Debug)]
pub struct GraphqlFieldMapping {
    pub field_name: String,
    pub target_url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
}

impl GraphqlFederationEngine {
    /// Creates a new federation engine with custom depth, complexity limits, and field mappings.
    pub fn new(
        max_depth: usize,
        max_complexity: usize,
        mappings: HashMap<String, GraphqlFieldMapping>,
    ) -> Self {
        Self {
            client: Client::new(),
            max_depth,
            max_complexity,
            mappings: Arc::new(mappings),
        }
    }

    /// Creates a new federation engine with a custom reqwest client.
    pub fn with_client(
        client: Client,
        max_depth: usize,
        max_complexity: usize,
        mappings: HashMap<String, GraphqlFieldMapping>,
    ) -> Self {
        Self {
            client,
            max_depth,
            max_complexity,
            mappings: Arc::new(mappings),
        }
    }

    /// Maximum query AST depth allowed.
    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    /// Maximum query complexity (field count) allowed.
    pub fn max_complexity(&self) -> usize {
        self.max_complexity
    }

    /// Field mappings for downstream federation.
    pub fn mappings(&self) -> &Arc<HashMap<String, GraphqlFieldMapping>> {
        &self.mappings
    }

    /// TRUE AST DEPTH CHECKER
    /// Walks the parsed GraphQL document recursively instead of counting raw string braces.
    pub fn validate_query(&self, query_str: &str) -> Result<(), &'static str> {
        // Parse into a structural AST. If syntax is invalid, it fails here instantly.
        let ast: ExecutableDocument = parse_query(query_str).map_err(|_| "Invalid GraphQL syntax")?;
        
        let mut max_observed_depth = 0;
        let mut total_fields_count = 0;

        for (_name, op) in ast.operations.iter() {
            let (depth, complexity) = self.measure_selection_set(&op.node.selection_set.node.items);
            if depth > max_observed_depth {
                max_observed_depth = depth;
            }
            total_fields_count += complexity;
        }

        if max_observed_depth > self.max_depth {
            return Err("Query depth limit exceeded (threshold = 6)");
        }
        if total_fields_count > self.max_complexity {
            return Err("Query complexity threshold exceeded");
        }

        Ok(())
    }

    // Helper method to recursively calculate exact structural depth and field complexity weights
    fn measure_selection_set(&self, items: &[async_graphql_parser::Positioned<Selection>]) -> (usize, usize) {
        if items.is_empty() {
            return (0, 0);
        }

        let mut max_child_depth = 0;
        let mut total_fields = 0;

        for item in items {
            match &item.node {
                Selection::Field(field) => {
                    total_fields += 1; // Count every field requested
                    if !field.node.selection_set.node.items.is_empty() {
                        let (child_depth, child_complexity) =
                            self.measure_selection_set(&field.node.selection_set.node.items);
                        if child_depth > max_child_depth {
                            max_child_depth = child_depth;
                        }
                        total_fields += child_complexity;
                    }
                }
                Selection::InlineFragment(inline) => {
                    if !inline.node.selection_set.node.items.is_empty() {
                        let (child_depth, child_complexity) =
                            self.measure_selection_set(&inline.node.selection_set.node.items);
                        if child_depth > max_child_depth {
                            max_child_depth = child_depth;
                        }
                        total_fields += child_complexity;
                    }
                }
                Selection::FragmentSpread(_) => {
                    total_fields += 1;
                }
            }
        }
        // Current level + deepest nested child path
        (1 + max_child_depth, total_fields)
    }

    /// ZERO-OVERHEAD CONCURRENT EXECUTION
    /// Drops `tokio::spawn` completely. Drives I/O concurrently using an in-thread multiplexer.
    pub async fn execute_federated_query(
        &self,
        fields: Vec<(String, HashMap<String, String>)>,
    ) -> HashMap<String, JsonValue> {
        let mut workers = FuturesUnordered::new();

        for (field_name, args) in fields {
            if let Some(mapping) = self.mappings.get(&field_name) {
                let client = self.client.clone();
                let mapping = mapping.clone();

                // Move execution into an in-thread future pipeline instead of spawning an OS thread task
                workers.push(async move {
                    let mut url = mapping.target_url.clone();
                    for (k, v) in &args {
                        url = url.replace(&format!("{{{}}}", k), v);
                    }

                    let mut req = match mapping.method.as_str() {
                        "POST" => client.post(&url),
                        "PUT" => client.put(&url),
                        _ => client.get(&url),
                    };

                    for (hk, hv) in &mapping.headers {
                        req = req.header(hk, hv);
                    }

                    match req.send().await {
                        Ok(resp) => {
                            if let Ok(json) = resp.json::<JsonValue>().await {
                                (field_name, json)
                            } else {
                                (field_name, JsonValue::String("Invalid JSON response".to_string()))
                            }
                        }
                        Err(e) => {
                            warn!("Downstream REST hook error for field '{}': {}", field_name, e);
                            (field_name, JsonValue::String(format!("Upstream error: {}", e)))
                        }
                    }
                });
            }
        }

        // Stream responses out out of order as they finish downloading
        let mut response_data = HashMap::new();
        while let Some((field_name, json)) = workers.next().await {
            response_data.insert(field_name, json);
        }

        response_data
    }
}

impl Default for GraphqlFederationEngine {
    fn default() -> Self {
        Self::new(6, 100, HashMap::new())
    }
}

impl GraphqlFieldMapping {
    pub fn new(
        field_name: impl Into<String>,
        target_url: impl Into<String>,
        method: impl Into<String>,
    ) -> Self {
        Self {
            field_name: field_name.into(),
            target_url: target_url.into(),
            method: method.into(),
            headers: HashMap::new(),
        }
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU16, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    static PORT_COUNTER: AtomicU16 = AtomicU16::new(18500);

    #[test]
    fn test_ast_depth_validation_pass() {
        let engine = GraphqlFederationEngine::default();

        // 1-level query
        assert!(engine.validate_query("query { hello }").is_ok());

        // 6-level nested query (exactly at threshold)
        let query_depth_6 = r#"
            query DeepQuery {
                level1 {
                    level2 {
                        level3 {
                            level4 {
                                level5 {
                                    level6
                                }
                            }
                        }
                    }
                }
            }
        "#;
        assert!(engine.validate_query(query_depth_6).is_ok());
    }

    #[test]
    fn test_ast_depth_validation_exceeded() {
        let engine = GraphqlFederationEngine::default();

        // 7-level nested query (exceeds threshold = 6)
        let query_depth_7 = r#"
            query TooDeep {
                level1 {
                    level2 {
                        level3 {
                            level4 {
                                level5 {
                                    level6 {
                                        level7
                                    }
                                }
                            }
                        }
                    }
                }
            }
        "#;
        let res = engine.validate_query(query_depth_7);
        assert_eq!(res, Err("Query depth limit exceeded (threshold = 6)"));
    }

    #[test]
    fn test_ast_complexity_exceeded() {
        let engine = GraphqlFederationEngine::new(10, 4, HashMap::new());

        // 5 fields queried when max complexity is 4
        let wide_query = r#"
            query Wide {
                fieldA
                fieldB
                fieldC
                fieldD
                fieldE
            }
        "#;
        let res = engine.validate_query(wide_query);
        assert_eq!(res, Err("Query complexity threshold exceeded"));
    }

    #[test]
    fn test_invalid_graphql_syntax() {
        let engine = GraphqlFederationEngine::default();
        let malformed = "query { unclosed_field";
        let res = engine.validate_query(malformed);
        assert_eq!(res, Err("Invalid GraphQL syntax"));
    }

    #[test]
    fn test_ast_inline_fragment() {
        let engine = GraphqlFederationEngine::default();
        let query_with_fragment = r#"
            query {
                node {
                    ... on User {
                        id
                        name
                    }
                }
            }
        "#;
        assert!(engine.validate_query(query_with_fragment).is_ok());
    }

    #[tokio::test]
    async fn test_federated_query_concurrent_execution() {
        // Spin up a lightweight local mock HTTP server
        let port = PORT_COUNTER.fetch_add(1, Ordering::SeqCst);
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .expect("Failed to bind test listener");

        // Spawn mock server loop
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let req_str = String::from_utf8_lossy(&buf[..n]);

                    let (status, body) = if req_str.contains("/users/42") {
                        ("200 OK", r#"{"id": 42, "name": "Alice"}"#)
                    } else if req_str.contains("/orders/42") {
                        ("200 OK", r#"{"order_id": 999, "total": 149.99}"#)
                    } else if req_str.contains("/error") {
                        ("500 Internal Server Error", r#"{"error": "db_fail"}"#)
                    } else {
                        ("404 Not Found", r#"{"error": "not_found"}"#)
                    };

                    let response = format!(
                        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        status,
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });

        // Set up field mappings
        let mut mappings = HashMap::new();
        mappings.insert(
            "user".to_string(),
            GraphqlFieldMapping::new("user", format!("http://127.0.0.1:{}/users/{{userId}}", port), "GET")
                .with_header("X-Custom-Auth", "bearer-token-123"),
        );
        mappings.insert(
            "orders".to_string(),
            GraphqlFieldMapping::new("orders", format!("http://127.0.0.1:{}/orders/{{userId}}", port), "GET"),
        );

        let engine = GraphqlFederationEngine::new(6, 100, mappings);

        // Execute federated query concurrently
        let mut user_args = HashMap::new();
        user_args.insert("userId".to_string(), "42".to_string());

        let mut orders_args = HashMap::new();
        orders_args.insert("userId".to_string(), "42".to_string());

        let fields = vec![
            ("user".to_string(), user_args),
            ("orders".to_string(), orders_args),
        ];

        let results = engine.execute_federated_query(fields).await;

        assert_eq!(results.len(), 2);
        assert_eq!(results["user"]["id"], 42);
        assert_eq!(results["user"]["name"], "Alice");
        assert_eq!(results["orders"]["order_id"], 999);
        assert_eq!(results["orders"]["total"], 149.99);
    }

    #[tokio::test]
    async fn test_federated_query_network_error_resilience() {
        // Point to an unavailable endpoint
        let mut mappings = HashMap::new();
        mappings.insert(
            "offline_service".to_string(),
            GraphqlFieldMapping::new("offline_service", "http://127.0.0.1:19999/down", "GET"),
        );

        let engine = GraphqlFederationEngine::new(6, 100, mappings);
        let fields = vec![("offline_service".to_string(), HashMap::new())];

        let results = engine.execute_federated_query(fields).await;
        assert_eq!(results.len(), 1);
        let val_str = results["offline_service"].as_str().unwrap();
        assert!(val_str.starts_with("Upstream error:"));
    }
}
