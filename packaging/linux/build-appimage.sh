#!/bin/sh
# Wrap the built program in an AppImage: one file that runs on any recent Linux.
#   build-appimage.sh <path to bilby> <path to appimagetool> <output file>
set -eu
here=$(cd "$(dirname "$0")" && pwd)
binary=$1
tool=$2
out=$3
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
app="$dir/Bilby.AppDir"
mkdir -p "$app/usr/bin" "$app/usr/share/applications" "$app/usr/share/icons/hicolor/512x512/apps"
cp "$binary" "$app/usr/bin/bilby"
chmod 755 "$app/usr/bin/bilby"
cp "$here/bilby.desktop" "$app/bilby.desktop"
cp "$here/bilby.desktop" "$app/usr/share/applications/bilby.desktop"
cp "$here/../bilby.png" "$app/bilby.png"
cp "$here/../bilby.png" "$app/usr/share/icons/hicolor/512x512/apps/bilby.png"
ln -s bilby.png "$app/.DirIcon"
cat > "$app/AppRun" <<'RUN'
#!/bin/sh
exec "$(dirname "$(readlink -f "$0")")/usr/bin/bilby" "$@"
RUN
chmod 755 "$app/AppRun"
# The tool is itself an AppImage; unpacking it first works where FUSE is missing.
ARCH=x86_64 "$tool" --appimage-extract-and-run --no-appstream "$app" "$out"
chmod 755 "$out"
