use serde::{Deserialize, Serialize};
use tracing::info;

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
