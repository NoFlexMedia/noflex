-- SPDX-License-Identifier: AGPL-3.0-only
--
-- An edition is a cut, and can hold several files with different pictures and
-- sound, so facts about a file can't live on it. They're in `media_streams`.

DROP INDEX IF EXISTS editions_codec_video;
DROP INDEX IF EXISTS editions_codec_audio;
ALTER TABLE editions DROP COLUMN codec_video;
ALTER TABLE editions DROP COLUMN codec_audio;
ALTER TABLE editions DROP COLUMN width;
ALTER TABLE editions DROP COLUMN height;
ALTER TABLE editions DROP COLUMN bit_depth;
ALTER TABLE editions DROP COLUMN hdr_format;
ALTER TABLE editions DROP COLUMN audio_tracks_json;
ALTER TABLE editions DROP COLUMN subtitle_tracks_json;
ALTER TABLE editions DROP COLUMN video_codec;
ALTER TABLE editions DROP COLUMN video_profile;
ALTER TABLE editions DROP COLUMN video_level;
ALTER TABLE editions DROP COLUMN pixel_format;
ALTER TABLE editions DROP COLUMN color_primaries;
ALTER TABLE editions DROP COLUMN transfer_characteristics;
ALTER TABLE editions DROP COLUMN matrix_coefficients;
ALTER TABLE editions DROP COLUMN dovi_profile;
ALTER TABLE editions DROP COLUMN max_cll;
ALTER TABLE editions DROP COLUMN max_fall;
ALTER TABLE editions DROP COLUMN mastering_primary_r_x;
ALTER TABLE editions DROP COLUMN mastering_primary_r_y;
ALTER TABLE editions DROP COLUMN mastering_primary_g_x;
ALTER TABLE editions DROP COLUMN mastering_primary_g_y;
ALTER TABLE editions DROP COLUMN mastering_primary_b_x;
ALTER TABLE editions DROP COLUMN mastering_primary_b_y;
ALTER TABLE editions DROP COLUMN mastering_white_x;
ALTER TABLE editions DROP COLUMN mastering_white_y;
ALTER TABLE editions DROP COLUMN mastering_min_lum;
ALTER TABLE editions DROP COLUMN mastering_max_lum;
ALTER TABLE editions DROP COLUMN frame_rate;
