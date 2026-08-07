-- Fix: admin_api_keys.service_account_id FK should be ON DELETE SET NULL
-- so that deleting a service_account preserves the key audit record
-- (with service_account_id cleared) rather than blocking the DELETE.
-- The handler revokes keys (sets revoked_at) before deleting the SA,
-- so the audit record is retained with revoked_at set and service_account_id NULL.
ALTER TABLE admin_api_keys
    DROP CONSTRAINT IF EXISTS admin_api_keys_service_account_id_fkey;

ALTER TABLE admin_api_keys
    ADD CONSTRAINT admin_api_keys_service_account_id_fkey
    FOREIGN KEY (service_account_id)
    REFERENCES service_accounts(id)
    ON DELETE SET NULL;
