-- Add deleted_at column to projects for soft-delete tracking
ALTER TABLE projects ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
