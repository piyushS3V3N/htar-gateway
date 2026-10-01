-- ==============================================================================
-- HTAR-GATEWAY OTK STATE & OAUTH2 METADATA STORE (MySQL 8.x)
-- ==============================================================================

CREATE DATABASE IF NOT EXISTS htargw_db
    CHARACTER SET utf8mb4
    COLLATE utf8mb4_unicode_ci;

USE htargw_db;

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

INSERT INTO oauth_clients (client_id, client_secret_hash, client_name, redirect_uri, grant_types, allowed_scopes)
VALUES ('consumer_payments_service_v1_id', '$2b$12$e6y9gG0Q9pY7q8L9o0.abcdefgh1234567890', 'l7_migrated_client', 'https://oauth.internal/callback', JSON_ARRAY('client_credentials'), JSON_ARRAY('payments:read', 'payments:write'))
ON DUPLICATE KEY UPDATE updated_at = CURRENT_TIMESTAMP;

