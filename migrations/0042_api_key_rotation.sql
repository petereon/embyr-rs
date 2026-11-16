-- api-key-rotation slice 1 (ADR-085), expand-only.
-- NULL expiry with a non-NULL api_key_hash_previous = the previous key is dead (fail closed).
ALTER TABLE projects ADD COLUMN api_key_previous_expires_at TIMESTAMPTZ;
-- Display only; NULL = never rotated.
ALTER TABLE projects ADD COLUMN api_key_rotated_at TIMESTAMPTZ;
