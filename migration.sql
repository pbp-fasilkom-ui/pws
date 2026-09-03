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
