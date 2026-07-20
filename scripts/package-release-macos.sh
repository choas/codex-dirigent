#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
app_dir="$project_root/dist/Codex Dirigent.app"
version=$(plutil -extract CFBundleShortVersionString raw "$project_root/packaging/Info.plist")
archive="$project_root/dist/Codex-Dirigent-$version-macos-universal.zip"
checksum="$archive.sha256"
sign_identity=${CODE_SIGN_IDENTITY:?Set CODE_SIGN_IDENTITY to a Developer ID Application identity}

CODE_SIGN_IDENTITY="$sign_identity" MACOS_UNIVERSAL=1 "$project_root/scripts/bundle-macos.sh"
codesign --verify --deep --strict --verbose=2 "$app_dir"

if [ -n "${NOTARY_PROFILE:-}" ]; then
  notarization_archive="$project_root/dist/Codex-Dirigent-$version-notarization.zip"
  rm -f "$notarization_archive"
  ditto -c -k --sequesterRsrc --keepParent "$app_dir" "$notarization_archive"
  xcrun notarytool submit "$notarization_archive" \
    --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$app_dir"
  xcrun stapler validate "$app_dir"
  rm -f "$notarization_archive"
else
  echo "NOTARY_PROFILE is unset; the archive will be Developer ID signed but not notarized." >&2
fi

rm -f "$archive" "$checksum"
ditto -c -k --sequesterRsrc --keepParent "$app_dir" "$archive"
(CDPATH= cd -- "$project_root/dist" && \
  shasum -a 256 "$(basename "$archive")" >"$(basename "$checksum")")

echo "Created $archive"
echo "Created $checksum"
