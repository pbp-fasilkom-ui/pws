-- Migration: Add project-level sharing functionality
-- Simple project sharing - if shared, user has full access to that specific project

CREATE TABLE IF NOT EXISTS project_shares (
  project_id  UUID          NOT NULL,
  user_id     UUID          NOT NULL,
  created_at  TIMESTAMPTZ   NOT NULL DEFAULT now(),

  PRIMARY KEY (project_id, user_id),
  FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE ON UPDATE CASCADE,
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE ON UPDATE CASCADE
);

-- Migration: Track the source branch and exact commit for each deployment build.
-- Nullable columns preserve compatibility with build history created before this migration.
ALTER TABLE builds
  ADD COLUMN IF NOT EXISTS branch TEXT,
  ADD COLUMN IF NOT EXISTS commit_sha TEXT;

-- Migration: Prevent owner-namespace squatting.
-- project_owners.name had no uniqueness constraint, so the check-then-insert in
-- create_project_owner was a TOCTOU, and register_user rejects a username that
-- collides with an existing owner row -- meaning pre-created rows could block
-- legitimate registrations.
-- Remove any duplicate rows that are not referenced before adding the index.
DELETE FROM project_owners a
  USING project_owners b
  WHERE a.ctid > b.ctid
    AND a.name = b.name
    AND NOT EXISTS (SELECT 1 FROM projects WHERE projects.owner_id = a.id)
    AND NOT EXISTS (SELECT 1 FROM users_owners WHERE users_owners.owner_id = a.id);

CREATE UNIQUE INDEX IF NOT EXISTS project_owners_name_key ON project_owners (name);

-- Migration: per-user git push tokens.
-- api_token held exactly one row per project, so every collaborator shared one
-- credential: regenerating it locked out everyone else, and there was no way to
-- give a collaborator a credential without rotating the owner's.
-- user_id NULL marks the pre-existing project-wide token, which keeps already
-- configured git remotes working; new tokens are always per user.
ALTER TABLE api_token
  ADD COLUMN IF NOT EXISTS user_id UUID REFERENCES users(id) ON DELETE CASCADE;

-- At most one legacy project-wide token per project...
CREATE UNIQUE INDEX IF NOT EXISTS api_token_project_legacy_key
  ON api_token (project_id) WHERE user_id IS NULL;

-- ...and at most one token per (project, user).
CREATE UNIQUE INDEX IF NOT EXISTS api_token_project_user_key
  ON api_token (project_id, user_id) WHERE user_id IS NOT NULL;
