#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# The cla workflow's check. Records anyone who has signed CLA.md by commenting on
# the pull request, then sets the `cla` status on its head commit, passing once
# every commit author has signed the current version. It reads the pull request
# through the API only: nothing from it is checked out or run.
#
# Every run rescans the pull request's comments, so a run cancelled behind a newer
# one loses no signature. CLA_DRY_RUN=1 prints what it would write instead.
#
#   GH_TOKEN=… GITHUB_REPOSITORY=owner/repo GITHUB_EVENT_PATH=event.json scripts/ci/cla.sh

set -euo pipefail
shopt -s inherit_errexit

root=$(cd "$(dirname "$0")/../.." && pwd)
repo=${GITHUB_REPOSITORY:?}
branch='cla-signatures'
file=signatures.json
phrase='I have read the NoFlex CLA and I agree to its terms.'
marker='<!-- noflex-cla -->'
# GitHub user ids that don't sign: whoever the CLA is with (GodTamIt), and the
# dependency bots (dependabot[bot], renovate[bot]). Any other bot's commits fail:
# an agent's work is signed for by the person who directed it, as its author.
exempt='[3161130, 49699333, 29139614]'

version=$(sed -n 's/^Version \([0-9][0-9.]*\).*/\1/p' "$root/CLA.md" | head -n 1)
[ -n "$version" ] || {
  echo "CLA.md names no version" >&2
  exit 1
}
cla=https://github.com/$repo/blob/HEAD/CLA.md

write() {
  if [ -n "${CLA_DRY_RUN:-}" ]; then
    echo "would write: $*" >&2
  else
    gh api "$@" >/dev/null
  fi
}

pr=$(jq -r '.pull_request.number // .issue.number' "${GITHUB_EVENT_PATH:?}")
head=$(gh api "repos/$repo/pulls/$pr" --jq .head.sha)

# {sha, signatures}, where sha is null before the first signature.
load() {
  local out
  if out=$(gh api "repos/$repo/contents/$file?ref=$branch" 2>&1); then
    jq '{sha, signatures: (.content | gsub("\n"; "") | @base64d | fromjson)}' <<<"$out"
  elif grep -q 'HTTP 404' <<<"$out"; then
    echo '{"sha": null, "signatures": []}'
  else
    echo "$out" >&2
    return 1
  fi
}

# The branch starts at the default branch's head, so making it adds no commit;
# every commit on it is then the API's, which GitHub signs.
ensure_branch() {
  gh api "repos/$repo/git/ref/heads/$branch" >/dev/null 2>&1 && return
  local base sha
  base=$(gh api "repos/$repo" --jq .default_branch)
  sha=$(gh api "repos/$repo/git/ref/heads/$base" --jq .object.sha)
  write -X POST "repos/$repo/git/refs" -f ref="refs/heads/$branch" -f sha="$sha"
}

comments=$(gh api --paginate "repos/$repo/issues/$pr/comments" \
  --jq '.[] | {comment: .id, id: .user.id, login: .user.login, type: .user.type, body, url: .html_url, at: .created_at}' | jq -s .)

# Earliest signing comment per person. Pasted from Markdown, it may keep its backticks.
signers=$(jq --arg phrase "$phrase" '
  def norm: gsub("^[`\\s]+|[`\\s]+$"; "") | ascii_downcase | rtrimstr(".");
  map(select(.type == "User" and (.body | norm) == ($phrase | norm))) | unique_by(.id)' <<<"$comments")

# Prints every signature once this pull request's are recorded.
record() {
  local state new sha logins
  local -a previous
  for _ in 1 2 3; do
    state=$(load)
    new=$(jq --argjson signers "$signers" --arg version "$version" --argjson pr "$pr" '
      .signatures as $have
      | [$signers[]
         | select(.id as $id | $have | any(.id == $id and .version == $version) | not)
         | {id, login, version: $version, signed_at: .at, pull_request: $pr, comment: .url}]' <<<"$state")
    if [ "$(jq length <<<"$new")" = 0 ]; then
      jq .signatures <<<"$state"
      return
    fi
    logins=$(jq -r 'map(.login) | join(", ")' <<<"$new")
    sha=$(jq -r '.sha // empty' <<<"$state")
    previous=()
    [ -n "$sha" ] && previous=(-f "sha=$sha")
    ensure_branch
    # A conflict means another pull request's run wrote first; reload and merge again.
    if write -X PUT "repos/$repo/contents/$file" -f branch="$branch" \
      -f message="cla: record $logins for version $version" "${previous[@]}" \
      -f content="$(jq --argjson new "$new" '.signatures + $new | sort_by(.login | ascii_downcase)' <<<"$state" | base64 -w0)"; then
      jq --argjson new "$new" '.signatures + $new' <<<"$state"
      return
    fi
  done
  echo "couldn't record signatures for $logins" >&2
  return 1
}
signatures=$(record)

commits=$(gh api --paginate "repos/$repo/pulls/$pr/commits" \
  --jq '.[] | {sha: .sha[0:7], user: .author, name: .commit.author.name}' | jq -s .)
missing=$(jq -r --argjson signed "$signatures" --arg version "$version" --argjson exempt "$exempt" '
  [.[].user | select(. != null and .type == "User")]
  | unique_by(.id)
  | map(select(.id as $id | ($exempt | any(. == $id) | not) and ($signed | any(.id == $id and .version == $version) | not)))
  | .[] | "- @\(.login)"' <<<"$commits")
unmatched=$(jq -r '.[] | select(.user == null) | "- `\(.sha)`, by \(.name)"' <<<"$commits")
bots=$(jq -r --argjson exempt "$exempt" '
  .[] | select(.user != null and .user.type != "User" and (.user.id as $id | $exempt | any(. == $id) | not))
  | "- `\(.sha)`, by \(.user.login)"' <<<"$commits")
existing=$(jq -r --arg marker "$marker" \
  'map(select(.login == "github-actions[bot]" and (.body | contains($marker)))) | first | .comment // empty' <<<"$comments")

status() {
  write -X POST "repos/$repo/statuses/$head" -f state="$1" -f context=cla -f description="$2" -f target_url="$cla"
}

if [ -z "$missing$unmatched$bots" ]; then
  status success "Everyone on this pull request has signed the CLA."
  [ -n "$existing" ] && write -X PATCH "repos/$repo/issues/comments/$existing" \
    -f body="$marker
Everyone on this pull request has signed the [CLA]($cla), version $version. Thank you!"
  exit 0
fi

body="$marker
Thanks for the pull request! Before it can merge, everyone who wrote its commits signs NoFlex's [Contributor License Agreement]($cla), once. You keep your copyright, and your contribution stays available under the AGPL; [CONTRIBUTING.md](https://github.com/$repo/blob/HEAD/CONTRIBUTING.md#licensing-and-the-cla) explains why NoFlex asks."
if [ -n "$missing" ]; then
  body="$body

Still to sign version $version:

$missing

To sign, comment on this pull request:

\`\`\`
$phrase
\`\`\`"
fi
if [ -n "$unmatched" ]; then
  body="$body

These commits' emails don't belong to a GitHub account, so the check can't tell who wrote them. Add the email to your account under Settings → Emails, or amend the commits' author, and push again:

$unmatched"
fi
if [ -n "$bots" ]; then
  body="$body

These commits were written by a bot, which can't sign. Whoever directed it signs instead: make them the commits' author, and push again:

$bots"
fi
body="$body

Comment \`recheck\` to run the check again."

status failure "Waiting for the CLA to be signed: see the comment on this pull request."
if [ -n "$existing" ]; then
  write -X PATCH "repos/$repo/issues/comments/$existing" -f body="$body"
else
  write -X POST "repos/$repo/issues/$pr/comments" -f body="$body"
fi
