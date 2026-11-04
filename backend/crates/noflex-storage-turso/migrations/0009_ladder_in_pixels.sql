-- SPDX-License-Identifier: AGPL-3.0-only
--
-- Encode plans now carry an output size rather than a height ceiling. Unstarted
-- jobs hold plans in the old shape; the next planning pass queues them again.
-- Finished versions don't read their plan.

DELETE FROM optimized_versions WHERE status <> 'ready';
