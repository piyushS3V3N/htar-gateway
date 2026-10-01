-- ==============================================================================
-- HTAR-GATEWAY OTK STATE & OAUTH2 METADATA STORE (MySQL 8.x)
-- CA Layer 7 OAuth Toolkit (OTK) Relational Persistence Migration
-- ==============================================================================

CREATE DATABASE IF NOT EXISTS htargw_db
    CHARACTER SET utf8mb4
    COLLATE utf8mb4_unicode_ci;

USE htargw_db;

-- 1. OAuth2 Registered Client Definitions
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

-- 2. Asymmetric Cryptographic Keys & JWKS Key Vault
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

-- 3. JTI (JWT ID) Replay Attack Prevention Buffer
CREATE TABLE IF NOT EXISTS oauth_token_jti (
    jti VARCHAR(128) PRIMARY KEY,
    client_id VARCHAR(64) NOT NULL,
    subject VARCHAR(128) NOT NULL,
    expires_at TIMESTAMP NOT NULL,
    issued_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    INDEX idx_jti_expiry (expires_at),
    CONSTRAINT fk_jti_client FOREIGN KEY (client_id) REFERENCES oauth_clients(client_id) ON DELETE CASCADE
) ENGINE=InnoDB;

-- 4. Dynamic Scope Authorization Matrix
CREATE TABLE IF NOT EXISTS scope_definitions (
    scope_name VARCHAR(64) PRIMARY KEY,
    description VARCHAR(255) NOT NULL,
    allowed_http_methods JSON NOT NULL,
    target_path_pattern VARCHAR(255) NOT NULL,
    rate_limit_rpm INT UNSIGNED DEFAULT 1000
) ENGINE=InnoDB;

-- Seed Migration Data for Ingested Layer 7 Policies
INSERT INTO oauth_clients (client_id, client_secret_hash, client_name, redirect_uri, grant_types, allowed_scopes)
VALUES (
    'l7-client-app',
    '$2b$12$K8y4X4Wf/f/0g54d8l6X4.1i8h8Q4N1z7vTz9oFkHqKxU4J5y3Z.',
    'Payments Internal Microservice',
    'https://payments-backend.internal:8080/oauth/callback',
    JSON_ARRAY('authorization_code', 'client_credentials', 'refresh_token'),
    JSON_ARRAY('payments:read', 'payments:write', 'payments:admin')
) ON DUPLICATE KEY UPDATE updated_at = CURRENT_TIMESTAMP;

INSERT INTO scope_definitions (scope_name, description, allowed_http_methods, target_path_pattern, rate_limit_rpm)
VALUES 
    ('payments:read', 'Read financial transaction ledgers', JSON_ARRAY('GET'), '/api/v1/payments/*', 5000),
    ('payments:write', 'Commit transactional mutations', JSON_ARRAY('POST', 'PUT'), '/api/v1/payments/*', 2000)
ON DUPLICATE KEY UPDATE rate_limit_rpm = VALUES(rate_limit_rpm);
