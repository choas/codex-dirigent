#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
app_dir="$project_root/dist/Codex Dirigent.app"
contents_dir="$app_dir/Contents"
iconset_dir="$project_root/dist/CodexDirigent.iconset"
source_png="$project_root/dist/CodexDirigent-1024.png"
sign_identity=${CODE_SIGN_IDENTITY:--}

if [ "${MACOS_UNIVERSAL:-0}" = "1" ]; then
  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    if ! rustup target list --installed | grep -qx "$target"; then
      echo "Missing Rust target: $target" >&2
      echo "Install it with: rustup target add $target" >&2
      exit 1
    fi
    cargo build --manifest-path "$project_root/Cargo.toml" --release --locked --target "$target"
  done
  binary_path="$project_root/dist/codex-dirigent-universal"
  mkdir -p "$project_root/dist"
  lipo -create \
    "$project_root/target/aarch64-apple-darwin/release/codex-dirigent" \
    "$project_root/target/x86_64-apple-darwin/release/codex-dirigent" \
    -output "$binary_path"
else
  cargo build --manifest-path "$project_root/Cargo.toml" --release --locked
  binary_path="$project_root/target/release/codex-dirigent"
fi

rm -rf "$app_dir" "$iconset_dir"
mkdir -p "$contents_dir/MacOS" "$contents_dir/Resources" "$iconset_dir"
cp "$binary_path" "$contents_dir/MacOS/codex-dirigent"
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
rm -f "$source_png" "$project_root/dist/codex-dirigent-universal"

if [ "$sign_identity" = "-" ]; then
  codesign --force --deep --sign - "$app_dir"
else
  codesign --force --deep --options runtime --timestamp --sign "$sign_identity" "$app_dir"
fi
echo "Created $app_dir"
