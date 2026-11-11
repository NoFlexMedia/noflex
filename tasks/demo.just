# SPDX-License-Identifier: AGPL-3.0-only

set positional-arguments

script := justfile_directory() / "scripts/tasks/demo.sh"

# List demo recipes and their descriptions.
default:
    @just --list demo

# Fetch, build and start the demo, with the embedding model search uses. Options: --servers N, --port P, --bind HOST, --advertise HOST, --release. Changing them starts afresh.
up *args:
    @"{{ script }}" up "$@"

# Stop every demo server, keeping their data.
down:
    @"{{ script }}" down

# Show each demo server's address and whether it's running.
status:
    @"{{ script }}" status

# Follow one server's log (default: the first).
logs server='1':
    @"{{ script }}" logs "$1"

# Download the sample films without starting anything (about 2 GB, resumable).
fetch:
    @"{{ script }}" fetch

# Stop the demo and delete its accounts, watch state and caches; keeps the films.
reset:
    @"{{ script }}" reset

# Stop the demo and delete everything, the downloaded films and model included.
clean:
    @"{{ script }}" clean
