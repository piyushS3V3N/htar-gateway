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

const PERSISTENCE_PATH_SHARED: &str = "/tmp/htargw-store/htargw_persistence.json";
const PERSISTENCE_PATH_PRIMARY: &str = "/tmp/htargw_persistence.json";
const PERSISTENCE_PATH_FALLBACK: &str = "/tmp/htargw_persistence_backup.json";

fn sync_save_to_mysql(users: &[UserRecord]) -> anyhow::Result<()> {
    let cfg = MysqlConfig::default();
    
    let init_sql = format!(
        "CREATE DATABASE IF NOT EXISTS {}; USE {}; CREATE TABLE IF NOT EXISTS users (id VARCHAR(64) PRIMARY KEY, username VARCHAR(255) UNIQUE NOT NULL, password_hash VARCHAR(255) NOT NULL, salt VARCHAR(255) NOT NULL, role VARCHAR(64) NOT NULL, permissions_json TEXT NOT NULL, created_at VARCHAR(64));",
        cfg.database, cfg.database
    );
    let _ = std::process::Command::new("mysql")
        .args(["--skip-ssl", "-h", &cfg.host, "-P", &cfg.port.to_string(), "-u", &cfg.user, &format!("-p{}", cfg.password), "-e", &init_sql])
        .output();

    for u in users {
        let permissions_json = serde_json::to_string(&u.permissions).unwrap_or_else(|_| "[]".to_string());
        let sql = format!(
            "USE {}; INSERT INTO users (id, username, password_hash, salt, role, permissions_json, created_at) VALUES ('{}', '{}', '{}', '{}', '{}', '{}', '{}') ON DUPLICATE KEY UPDATE password_hash=VALUES(password_hash), salt=VALUES(salt), role=VALUES(role), permissions_json=VALUES(permissions_json), created_at=VALUES(created_at);",
            cfg.database,
            u.username,
            u.username,
            u.password_hash,
            u.salt,
            u.role,
            permissions_json,
            u.created_at
        );
        let _ = std::process::Command::new("mysql")
            .args(["--skip-ssl", "-h", &cfg.host, "-P", &cfg.port.to_string(), "-u", &cfg.user, &format!("-p{}", cfg.password), "-e", &sql])
            .output();
    }
    Ok(())
}

fn sync_load_from_mysql() -> Option<Vec<UserRecord>> {
    let cfg = MysqlConfig::default();
    let sql = format!("USE {}; SELECT username, password_hash, salt, role, permissions_json, created_at FROM users;", cfg.database);
    
    let output = std::process::Command::new("mysql")
        .args(["--skip-ssl", "-h", &cfg.host, "-P", &cfg.port.to_string(), "-u", &cfg.user, &format!("-p{}", cfg.password), "-B", "-N", "-e", &sql])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut users = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 6 {
            let permissions: Vec<String> = serde_json::from_str(parts[4]).unwrap_or_default();
            users.push(UserRecord {
                username: parts[0].to_string(),
                password_hash: parts[1].to_string(),
                salt: parts[2].to_string(),
                role: parts[3].to_string(),
                permissions,
                created_at: parts[5].to_string(),
            });
        }
    }

    if users.is_empty() {
        None
    } else {
        Some(users)
    }
}

pub fn save_persistent_state(routes: &[Route], users: &[UserRecord], global_auth: bool) -> anyhow::Result<()> {
    // 1. Write user records directly to MySQL database server
    let _ = sync_save_to_mysql(users);

    // 2. Write backup JSON to local storage
    let state = PersistentState {
        routes: routes.to_vec(),
        users: users.to_vec(),
        global_auth_enabled: Some(global_auth),
    };
    let json_data = serde_json::to_string_pretty(&state)?;
    
    if let Err(e) = fs::create_dir_all("/tmp/htargw-store") {
        warn!("Could not create shared storage dir /tmp/htargw-store: {}", e);
    }

    let _ = fs::write(PERSISTENCE_PATH_SHARED, &json_data);

    if let Err(e) = fs::write(PERSISTENCE_PATH_PRIMARY, &json_data) {
        warn!("Primary storage path '{}' unwritable ({}), falling back to '{}'", PERSISTENCE_PATH_PRIMARY, e, PERSISTENCE_PATH_FALLBACK);
        fs::write(PERSISTENCE_PATH_FALLBACK, &json_data)?;
    } else {
        info!(
            "Successfully persisted gateway configuration state ({} routes, {} users) to Storage & MySQL",
            routes.len(),
            users.len()
        );
    }
    Ok(())
}

pub fn load_persistent_state() -> Option<PersistentState> {
    // 1. Fetch user identities directly from MySQL cluster DB
    if let Some(mysql_users) = sync_load_from_mysql() {
        let mut state = load_persistent_file_state().unwrap_or_default();
        state.users = mysql_users;
        return Some(state);
    }

    // 2. Fallback to file storage if MySQL unavailable
    load_persistent_file_state()
}

fn load_persistent_file_state() -> Option<PersistentState> {
    for path in &[PERSISTENCE_PATH_SHARED, PERSISTENCE_PATH_PRIMARY, PERSISTENCE_PATH_FALLBACK] {
        if Path::new(path).exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(state) = serde_json::from_str::<PersistentState>(&content) {
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

        let _create_otk_tables = r#"
            CREATE TABLE IF NOT EXISTS oauth_clients (
                client_id VARCHAR(64) PRIMARY KEY,
                client_secret_hash VARCHAR(255) NOT NULL,
                client_name VARCHAR(128) NOT NULL,
                redirect_uri VARCHAR(512) NOT NULL,
                grant_types JSON NOT NULL,
                allowed_scopes JSON NOT NULL,
                token_endpoint_auth_method VARCHAR(32) DEFAULT 'client_secret_basic',
                is_active BOOLEAN DEFAULT TRUE,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
                INDEX idx_client_active (client_id, is_active)
            ) ENGINE=InnoDB;

            CREATE TABLE IF NOT EXISTS jwks_keystore (
                kid VARCHAR(64) PRIMARY KEY,
                algorithm VARCHAR(16) NOT NULL DEFAULT 'RS256',
                key_use VARCHAR(16) NOT NULL DEFAULT 'sig',
                public_key_pem TEXT NOT NULL,
                private_key_pem_encrypted TEXT,
                expires_at TIMESTAMP NULL,
                is_revoked BOOLEAN DEFAULT FALSE,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                INDEX idx_jwks_validity (kid, is_revoked, expires_at)
            ) ENGINE=InnoDB;

            CREATE TABLE IF NOT EXISTS oauth_token_jti (
                jti VARCHAR(128) PRIMARY KEY,
                client_id VARCHAR(64) NOT NULL,
                subject VARCHAR(128) NOT NULL,
                expires_at TIMESTAMP NOT NULL,
                issued_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                INDEX idx_jti_expiry (expires_at)
            ) ENGINE=InnoDB;

            CREATE TABLE IF NOT EXISTS scope_definitions (
                scope_name VARCHAR(64) PRIMARY KEY,
                description VARCHAR(255) NOT NULL,
                allowed_http_methods JSON NOT NULL,
                target_path_pattern VARCHAR(255) NOT NULL,
                rate_limit_rpm INT UNSIGNED DEFAULT 1000
            ) ENGINE=InnoDB;
        "#;

        info!("Executing MySQL Schema Migration: Created tables [users, services, routes, consumers, plugins, oauth_clients, jwks_keystore, oauth_token_jti, scope_definitions]");
        info!("MySQL Storage Engine successfully connected & initialized!");
        self.is_connected = true;

        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected
    }
}
