# Fixtures

These responses are written by hand, not recorded: the API answers only
with an API key, and the project has no shared one to record with. Each
follows the response shapes of OpenSubtitles' REST API v1, as its
documentation (opensubtitles.stoplight.io) and other clients' published
recordings show them, with made-up ids, names and tokens:

| File                  | Answer to                                         |
| --------------------- | ------------------------------------------------- |
| `search_movie.json`   | `GET /api/v1/subtitles` for a film, by hash and id |
| `search_episode.json` | `GET /api/v1/subtitles` for an episode             |
| `download.json`       | `POST /api/v1/download`                            |
| `download_quota.json` | `POST /api/v1/download` answered `406`, quota used |
| `login.json`          | `POST /api/v1/login` for a VIP account             |

Re-record them against the live API, with a key, when the API changes.
