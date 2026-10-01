// ==============================================================================
// HTAR-GATEWAY GRAPHQL-OVER-REST AGGREGATION & FEDERATION ENGINE
// Target: crates/htar-graphql/src/lib.rs
// Implements: Query Depth Checking (Limit = 6), Complexity Guards, Tokio Parallelism
// ==============================================================================

use futures::future::join_all;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphqlFieldMapping {
    pub field_name: String,
    pub target_url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
}

#[derive(Clone)]
pub struct GraphqlFederationEngine {
    client: Client,
    max_depth: usize,
    max_complexity: usize,
    mappings: Arc<HashMap<String, GraphqlFieldMapping>>,
}

impl Default for GraphqlFederationEngine {
    fn default() -> Self {
        Self::new(6, 200).expect("Failed to create default GraphqlFederationEngine")
    }
}

impl GraphqlFederationEngine {
    pub fn new(max_depth: usize, max_complexity: usize) -> anyhow::Result<Self> {
        let client = Client::builder()
            .pool_max_idle_per_host(100)
            .tcp_nodelay(true)
            .build()?;

        let mut mappings = HashMap::new();
        // Default seed mapping from Layer 7 migrated service
        mappings.insert(
            "paymentTransaction".to_string(),
            GraphqlFieldMapping {
                field_name: "paymentTransaction".to_string(),
                target_url: "http://payments-backend.internal:8080/api/v1/payments/{id}".to_string(),
                method: "GET".to_string(),
                headers: HashMap::from([
                    ("Accept".to_string(), "application/json".to_string()),
                    ("X-Gateway-Route".to_string(), "payments-service-v1".to_string()),
                ]),
            },
        );
        mappings.insert(
            "accountLedger".to_string(),
            GraphqlFieldMapping {
                field_name: "accountLedger".to_string(),
                target_url: "http://payments-backend.internal:8080/api/v1/accounts/{accountId}/ledger".to_string(),
                method: "GET".to_string(),
                headers: HashMap::from([
                    ("Accept".to_string(), "application/json".to_string()),
                ]),
            },
        );

        Ok(Self {
            client,
            max_depth,
            max_complexity,
            mappings: Arc::new(mappings),
        })
    }

    pub fn register_mapping(&mut self, mapping: GraphqlFieldMapping) {
        if let Some(map) = Arc::get_mut(&mut self.mappings) {
            map.insert(mapping.field_name.clone(), mapping);
        } else {
            let mut new_map = (*self.mappings).clone();
            new_map.insert(mapping.field_name.clone(), mapping);
            self.mappings = Arc::new(new_map);
        }
    }

    /// Calculate syntactic nesting depth to protect against performance exhaustion attacks
    pub fn validate_query_depth(&self, query: &str) -> Result<usize, &'static str> {
        let mut current_depth = 0usize;
        let mut max_observed = 0usize;

        for ch in query.chars() {
            match ch {
                '{' => {
                    current_depth += 1;
                    if current_depth > max_observed {
                        max_observed = current_depth;
                    }
                    if current_depth > self.max_depth {
                        return Err("Query depth limit exceeded (threshold = 6)");
                    }
                }
                '}' => {
                    current_depth = current_depth.saturating_sub(1);
                }
                _ => {}
            }
        }

        Ok(max_observed)
    }

    /// Estimate AST query complexity based on selection sets
    pub fn validate_query_complexity(&self, query: &str) -> Result<usize, &'static str> {
        let token_count = query.split_whitespace().count();
        if token_count > self.max_complexity {
            return Err("Query complexity threshold exceeded");
        }
        Ok(token_count)
    }

    /// Execute parallel async REST hooks for parsed GraphQL query fields
    pub async fn execute_federated_query(
        &self,
        fields: Vec<(String, HashMap<String, String>)>,
    ) -> HashMap<String, JsonValue> {
        let mut tasks = Vec::new();

        for (field_name, args) in fields {
            if let Some(mapping) = self.mappings.get(&field_name) {
                let client = self.client.clone();
                let mapping = mapping.clone();
                tasks.push(tokio::spawn(async move {
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
                }));
            }
        }

        let results = join_all(tasks).await;
        let mut response_data = HashMap::new();
        for res in results {
            if let Ok((name, val)) = res {
                response_data.insert(name, val);
            }
        }
        response_data
    }

    /// Export GraphQL Schema Definition Language (SDL)
    pub fn export_sdl(&self) -> String {
        r#"# GraphQL-over-REST Federated SDL
directive @rest(
  endpoint: String!
  method: String! = "GET"
  headers: [HeaderInput!]
  timeoutMs: Int = 3000
) on FIELD_DEFINITION

input HeaderInput {
  key: String!
  value: String!
}

type Query {
  paymentTransaction(id: ID!): PaymentRecord
    @rest(
      endpoint: "http://payments-backend.internal:8080/api/v1/payments/{args.id}"
      method: "GET"
    )
  accountLedger(accountId: ID!): AccountLedger
    @rest(
      endpoint: "http://payments-backend.internal:8080/api/v1/accounts/{args.accountId}/ledger"
      method: "GET"
    )
}

type PaymentRecord {
  id: ID!
  amount: Float!
  currency: String!
  status: String!
}

type AccountLedger {
  accountId: ID!
  availableBalance: Float!
  currency: String!
}
"#.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_depth_checker_pass() {
        let engine = GraphqlFederationEngine::default();
        let q = "{ paymentTransaction(id: \"1\") { id amount account { accountId } } }";
        let res = engine.validate_query_depth(q);
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 3);
    }

    #[test]
    fn test_depth_checker_exceeded() {
        let engine = GraphqlFederationEngine::default();
        let q = "{ a { b { c { d { e { f { g { h } } } } } } } }";
        let res = engine.validate_query_depth(q);
        assert!(res.is_err());
    }
}
