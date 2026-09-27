# Shared helpers for fixture builders. History is part of the logic, so every fixture
# is a real git repository.
set -euo pipefail

export GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.com
export GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.com

init() {
  git init -q -b main "$1"
  cd "$1"
}

commit() {
  git add -A
  git commit -q -m "$1"
}

write() {
  mkdir -p "$(dirname "$1")"
  cat > "$1"
}
