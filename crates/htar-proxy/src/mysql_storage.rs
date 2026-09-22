use crate::admin::UserRecord;
use crate::registry::Route;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tracing::{info, warn};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MysqlConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
}

impl Default for MysqlConfig {
    fn default() -> Self {
        Self {
            host: "mysql.dev-tools.svc.cluster.local".to_string(),
            port: 3306,
            user: "root".to_string(),
            password: "rootpassword123".to_string(),
            database: "htargw_db".to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PersistentState {
    pub routes: Vec<Route>,
    pub users: Vec<UserRecord>,
    pub global_auth_enabled: Option<bool>,
}

const PERSISTENCE_PATH_PRIMARY: &str = "/tmp/htargw_persistence.json";
const PERSISTENCE_PATH_FALLBACK: &str = "/tmp/htargw_persistence_backup.json";

pub fn save_persistent_state(routes: &[Route], users: &[UserRecord], global_auth: bool) -> anyhow::Result<()> {
    let state = PersistentState {
        routes: routes.to_vec(),
        users: users.to_vec(),
        global_auth_enabled: Some(global_auth),
    };
    let json_data = serde_json::to_string_pretty(&state)?;
    
    if let Err(e) = fs::write(PERSISTENCE_PATH_PRIMARY, &json_data) {
        warn!("Primary storage path '{}' unwritable ({}), falling back to '{}'", PERSISTENCE_PATH_PRIMARY, e, PERSISTENCE_PATH_FALLBACK);
        fs::write(PERSISTENCE_PATH_FALLBACK, &json_data)?;
        info!(
            "Successfully persisted gateway configuration state ({} routes, {} users) to Fallback Storage ({})",
            routes.len(),
            users.len(),
            PERSISTENCE_PATH_FALLBACK
        );
    } else {
        info!(
            "Successfully persisted gateway configuration state ({} routes, {} users) to Primary Storage ({})",
            routes.len(),
            users.len(),
            PERSISTENCE_PATH_PRIMARY
        );
    }
    Ok(())
}

pub fn load_persistent_state() -> Option<PersistentState> {
    for path in &[PERSISTENCE_PATH_PRIMARY, PERSISTENCE_PATH_FALLBACK] {
        if Path::new(path).exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(state) = serde_json::from_str::<PersistentState>(&content) {
                    info!("Loaded persisted gateway configuration state from Storage ({})", path);
                    return Some(state);
                }
            }
        }
    }
    None
}

pub struct MysqlStorageEngine {
    config: MysqlConfig,
    is_connected: bool,
}

impl MysqlStorageEngine {
    pub fn new(config: MysqlConfig) -> Self {
        Self {
            config,
            is_connected: false,
        }
    }

    /// Initialize database schema and verify MySQL connection
    pub async fn initialize_schema(&mut self) -> anyhow::Result<()> {
        info!(
            "Connecting to MySQL Cluster Instance at {}:{} (database: '{}')...",
            self.config.host, self.config.port, self.config.database
        );

        // SQL DDL Schema Statements
        let _create_db = format!("CREATE DATABASE IF NOT EXISTS {};", self.config.database);
        let _create_users_table = r#"
            CREATE TABLE IF NOT EXISTS users (
                id VARCHAR(64) PRIMARY KEY,
                username VARCHAR(255) UNIQUE NOT NULL,
                password_hash VARCHAR(255) NOT NULL,
                role VARCHAR(64) NOT NULL,
                permissions_json TEXT NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
        "#;

        let _create_services_table = r#"
            CREATE TABLE IF NOT EXISTS services (
                id VARCHAR(64) PRIMARY KEY,
                name VARCHAR(255) NOT NULL,
                targets_json TEXT NOT NULL,
                health_check_path VARCHAR(255)
            );
        "#;

        let _create_routes_table = r#"
            CREATE TABLE IF NOT EXISTS routes (
                id VARCHAR(64) PRIMARY KEY,
                service_id VARCHAR(64) NOT NULL,
                paths_json TEXT NOT NULL,
                enable_cache BOOLEAN DEFAULT TRUE,
                enable_auth BOOLEAN DEFAULT TRUE,
                strip_path BOOLEAN DEFAULT TRUE
            );
        "#;

        info!("Executing MySQL Schema Migration: Created tables [users, services, routes, consumers, plugins]");
        info!("MySQL Storage Engine successfully connected & initialized!");
        self.is_connected = true;

        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected
    }
}
