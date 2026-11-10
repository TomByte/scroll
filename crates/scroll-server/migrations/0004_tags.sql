-- Tags: labels any page can carry. A tag's name is unique in its workspace by `validation::tag_name_key`, which is
-- stored to look the holder of a name up; core's checks decide.
CREATE TABLE tags (
    id           UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    name_key     TEXT NOT NULL,
    record       BYTEA NOT NULL
);
CREATE INDEX tags_workspace_id_name_key ON tags (workspace_id, name_key);

-- Each membership is its own record, so two people tagging one page at once both keep their tags.
CREATE TABLE page_tags (
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    page_id      UUID NOT NULL,
    tag_id       UUID NOT NULL,
    PRIMARY KEY (page_id, tag_id)
);
CREATE INDEX page_tags_tag_id ON page_tags (tag_id);
CREATE INDEX page_tags_workspace_id ON page_tags (workspace_id);
