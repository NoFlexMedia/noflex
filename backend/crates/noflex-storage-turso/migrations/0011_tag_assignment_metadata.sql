-- SPDX-License-Identifier: AGPL-3.0-only
--
-- What's true of a tag on one target rather than of the tag: a person's role and
-- character belong to the work, not to the person.

ALTER TABLE tag_assignments ADD COLUMN metadata TEXT NOT NULL DEFAULT '{}';
