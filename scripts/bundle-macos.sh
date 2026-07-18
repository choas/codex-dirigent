#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
app_dir="$project_root/dist/Codex Dirigent.app"
contents_dir="$app_dir/Contents"
iconset_dir="$project_root/dist/CodexDirigent.iconset"
source_png="$project_root/dist/CodexDirigent-1024.png"

cargo build --manifest-path "$project_root/Cargo.toml" --release

rm -rf "$app_dir" "$iconset_dir"
mkdir -p "$contents_dir/MacOS" "$contents_dir/Resources" "$iconset_dir"
cp "$project_root/target/release/codex-dirigent" "$contents_dir/MacOS/codex-dirigent"
cp "$project_root/packaging/Info.plist" "$contents_dir/Info.plist"

sips -s format png -z 1024 1024 "$project_root/assets/CodexDirigent.svg" \
  --out "$source_png" >/dev/null
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$source_png" \
    --out "$iconset_dir/icon_${size}x${size}.png" >/dev/null
  double_size=$((size * 2))
  sips -z "$double_size" "$double_size" "$source_png" \
    --out "$iconset_dir/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset_dir" -o "$contents_dir/Resources/CodexDirigent.icns"
rm -rf "$iconset_dir"
rm -f "$source_png"

codesign --force --deep --sign - "$app_dir"
echo "Created $app_dir"
