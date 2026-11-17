-- Audit #64: record who suspended a project so an account Owner/Admin cannot
-- undo an operator/system suspension via PATCH status=active.
--   'account' = suspended by the account itself (PATCH)  -> account may reactivate
--   'system'  = operator suspend / cap / billing         -> account may NOT reactivate
--   NULL      = active, or suspended before this migration -> treated as locked
--               (an operator can reactivate; safe default).
ALTER TABLE projects ADD COLUMN suspended_by TEXT;
