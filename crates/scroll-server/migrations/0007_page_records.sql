-- The pages of the workspaces' records. A `pages` row without a record (content that arrived before its record,
-- or a page whose record was deleted) is not one; reads of records go through this view, so that rule is written
-- once. A plain view is expanded into each query before planning, so queries plan exactly as they would on
-- `pages`, partial indexes included.
CREATE VIEW page_records AS
    SELECT id, workspace_id, parent_id, row_of, record FROM pages WHERE record IS NOT NULL;
