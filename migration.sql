-- Migration: Add project-level sharing functionality
-- Simple project sharing - if shared, user has full access to that specific project

CREATE TABLE project_shares (
  project_id  UUID          NOT NULL,
  user_id     UUID          NOT NULL,
  created_at  TIMESTAMPTZ   NOT NULL DEFAULT now(),

  PRIMARY KEY (project_id, user_id),
  FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE ON UPDATE CASCADE,
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE ON UPDATE CASCADE
);
