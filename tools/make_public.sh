#!/bin/sh
# Makes the repository as it goes to GitHub: a new git repository with one commit that
# holds the files of HEAD, without the history (see docs/RELEASE.md for why). Nothing is
# pushed. Run from anywhere; the working tree must be committed.
#
#   PUBLIC_NAME="Name" PUBLIC_EMAIL="name@users.noreply.github.com" tools/make_public.sh [DIR]
#
# DIR defaults to out/public/excelinator. Without PUBLIC_NAME and PUBLIC_EMAIL the files
# are prepared and checked, but not committed.
set -e
root=$(cd "$(dirname "$0")/.." && pwd)
dest=${1:-$root/out/public/excelinator}
if [ -n "$(git -C "$root" status --porcelain --untracked-files=no)" ]; then
  echo "the working tree has uncommitted changes: commit them first" >&2
  exit 1
fi
rm -rf "$dest"
mkdir -p "$dest"
git -C "$root" archive HEAD | tar -x -C "$dest"

# Checks: what must not go public.
fail=0
big=$(find "$dest" -type f -size +5M)
if [ -n "$big" ]; then echo "files larger than 5 MB:" >&2; echo "$big" >&2; fail=1; fi
# This machine's home folder and name, the network it is in, keys and tokens.
host=$(hostname | sed 's/\..*//')
for pattern in "$HOME/" "$host" '\.fritz\.box' '\.local>' 'BEGIN [A-Z ]*PRIVATE KEY' 'ghp_[A-Za-z0-9]' 'sk-[A-Za-z0-9]\{20\}'; do
  hits=$(grep -rIl -e "$pattern" "$dest" 2>/dev/null | grep -v '/tools/make_public.sh$' || true)
  if [ -n "$hits" ]; then echo "\"$pattern\" found in:" >&2; echo "$hits" >&2; fail=1; fi
done
for d in corpus/src-enron corpus/lo-enron corpus/lo-oracle out target dist .venv; do
  if [ -e "$dest/$d" ]; then echo "$d must not be there" >&2; fail=1; fi
done
if [ "$fail" -ne 0 ]; then
  echo "not ready to go public (see above)" >&2
  exit 1
fi

cd "$dest"
git init -q -b main
git add -A
echo "prepared: $dest ($(git ls-files | wc -l | tr -d ' ') files, $(du -sh . | cut -f1))"
if [ -n "$PUBLIC_NAME" ] && [ -n "$PUBLIC_EMAIL" ]; then
  git -c user.name="$PUBLIC_NAME" -c user.email="$PUBLIC_EMAIL" commit -q -m "excelinator $(cat packaging/VERSION) (pre-alpha)"
  echo "committed as $PUBLIC_NAME <$PUBLIC_EMAIL>; next: git remote add origin …; git push -u origin main; git tag v$(cat packaging/VERSION); git push origin --tags"
else
  echo "not committed: set PUBLIC_NAME and PUBLIC_EMAIL (the author shown on GitHub) and run it again"
fi
