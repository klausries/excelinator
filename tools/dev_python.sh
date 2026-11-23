#!/bin/sh
# Work on the Python package in place: build the extension, put it next to the sources and
# let the installed package point at the source tree (edits take effect without reinstalling).
set -e
cd "$(dirname "$0")/../bindings/python"
maturin build --release >/dev/null
site=$(python3 -c "import sysconfig; print(sysconfig.get_paths()['purelib'])")
tmp=$(mktemp -d)
unzip -q -o ../../target/wheels/xlcells-*.whl -d "$tmp"
cp "$tmp"/xlcells/_xlcells*.so python/xlcells/
rm -rf "$tmp"
if [ ! -L "$site/xlcells" ]; then
  rm -rf "$site/xlcells"
  ln -s "$(pwd)/python/xlcells" "$site/xlcells"
fi
echo "xlcells in place: $site/xlcells -> $(pwd)/python/xlcells"
# The engine (calculation, dataframes, regions, stores, looks) is a package of its own, in
# two builds: on the Polars of the environment, and with Polars built in.
for pair in python-engine:xlcells_engine python-engine-bundled:xlcells_engine_bundled; do
  dir=${pair%%:*}
  module=${pair##*:}
  cd "../$dir"
  maturin build --release >/dev/null
  tmp=$(mktemp -d)
  unzip -q -o ../../target/wheels/"$module"-0*.whl -d "$tmp"
  cp "$tmp/$module"/_engine*.so "python/$module/"
  rm -rf "$tmp"
  if [ ! -L "$site/$module" ]; then
    rm -rf "$site/$module"
    ln -s "$(pwd)/python/$module" "$site/$module"
  fi
  echo "$module in place: $site/$module -> $(pwd)/python/$module"
done
