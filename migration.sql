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
