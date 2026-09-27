# Shared by every benchmark repository builder. It is the fixture helpers from
# tests/fixtures/_common.sh, with one change: commits get fixed dates, one day apart, so
# the same script builds the same commit ids on every machine and a run manifest can
# name the starting commit of each case.
source "$(dirname "${BASH_SOURCE[0]}")/../../tests/fixtures/_common.sh"

commit() {
  local n
  n=$(( $(git rev-list --count HEAD 2>/dev/null || echo 0) + 1 ))
  export GIT_AUTHOR_DATE="$(( 1767225600 + n * 86400 )) +0000"
  export GIT_COMMITTER_DATE="$GIT_AUTHOR_DATE"
  git add -A
  git commit -q -m "$1"
}
