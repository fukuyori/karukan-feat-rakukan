#!/usr/bin/env bash
# Debian/Ubuntu 配布用パッケージ (.deb) のビルド
#
#   scripts/build-deb.sh          # dist/karukan-fcitx5_<ver>_<arch>.deb を生成
#
# - 配布用に -C target-cpu=native を使わずビルドする (KARUKAN_NATIVE=OFF)
# - 辞書 (dict.bin) は同梱しない。初回は install.sh か docs/dictionary.md の
#   手順で配置する(辞書なしでもかな入力・モデル変換は動作する)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ADDON_DIR="$REPO_ROOT/karukan-im/fcitx5/fcitx5-addon"
BUILD_DIR="$ADDON_DIR/build-deb"
STAGE="$REPO_ROOT/dist/stage"
DIST="$REPO_ROOT/dist"

for cmd in dpkg-deb dpkg cmake cargo strip; do
    command -v "$cmd" >/dev/null 2>&1 || { echo "エラー: $cmd がありません" >&2; exit 1; }
done

ARCH="$(dpkg --print-architecture)"
HASH="$(git -C "$REPO_ROOT" rev-parse --short HEAD)"
DATE="$(git -C "$REPO_ROOT" show -s --format=%cd --date=format:%Y%m%d HEAD)"
# 日付を含めるのはアップグレード時のバージョン比較を単調にするため
# (ハッシュだけでは辞書順が時系列にならない)
VERSION="0.1.0+${DATE}.g${HASH}"
PKG="karukan-fcitx5_${VERSION}_${ARCH}"

echo "==> 配布用ビルド (KARUKAN_NATIVE=OFF, version: $VERSION)"
cmake -B "$BUILD_DIR" -S "$ADDON_DIR" \
    -DCMAKE_INSTALL_PREFIX=/usr -DKARUKAN_NATIVE=OFF
cmake --build "$BUILD_DIR" -j"$(nproc)"

echo "==> ステージング"
rm -rf "$STAGE"
DESTDIR="$STAGE" cmake --install "$BUILD_DIR"

echo "==> バイナリを strip"
find "$STAGE" -name '*.so' -exec strip --strip-unneeded {} +

echo "==> ドキュメントと権限"
DOC="$STAGE/usr/share/doc/karukan-fcitx5"
mkdir -p "$DOC"
cat > "$DOC/copyright" <<EOF
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: karukan-feat-rakukan
Source: https://github.com/fukuyori/karukan-feat-rakukan

Files: *
Copyright: 2026 togatoga, fukuyori
License: MIT or Apache-2.0
 デュアルライセンス。全文はソースリポジトリの LICENSE-MIT および
 LICENSE-APACHE を参照。
 .
 On Debian systems, the complete text of the Apache License 2.0 can be
 found in /usr/share/common-licenses/Apache-2.0.

Files: karukan-engine/data/*
Copyright: Google Inc. (Mozc project)
License: BSD-3-Clause
 Mozc 由来のデータ。ライセンス全文と各ファイルの由来はソースリポジトリの
 THIRD_PARTY_LICENSES を参照。
EOF
cat > "$DOC/changelog" <<EOF
karukan-fcitx5 ($VERSION) unstable; urgency=medium

  * Build from git $HASH.
    See https://github.com/fukuyori/karukan-feat-rakukan/commits/main

 -- fukuyori <fukuyori.n@gmail.com>  $(git -C "$REPO_ROOT" show -s --format=%cD HEAD)
EOF
gzip -9n "$DOC/changelog"
chmod 644 "$DOC/copyright" "$DOC/changelog.gz"
find "$STAGE" -type d -exec chmod 755 {} +
# lintian 備考: embedded-library libyaml は Rust 側の静的リンク
# (serde_yaml 経由) によるもので許容している。

echo "==> パッケージメタデータ"
mkdir -p "$STAGE/DEBIAN"
INSTALLED_SIZE="$(du -sk "$STAGE" --exclude=DEBIAN | cut -f1)"
cat > "$STAGE/DEBIAN/control" <<EOF
Package: karukan-fcitx5
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Depends: fcitx5, libfcitx5core7 | libfcitx5core8, libfcitx5config6, libfcitx5utils2, libxkbcommon0, libuuid1, libgomp1, libstdc++6, libc6
Installed-Size: $INSTALLED_SIZE
Maintainer: fukuyori <fukuyori.n@gmail.com>
Homepage: https://github.com/fukuyori/karukan-feat-rakukan
Description: Japanese IME for fcitx5 with neural kana-kanji conversion
 Karukan feat. Rakukan - ニューラルかな漢字変換エンジンを持つ
 fcitx5 向け日本語入力メソッド。ライブ変換・変換学習・
 F6-F10 変換・範囲指定変換に対応。
 .
 変換モデルは初回起動時に Hugging Face から自動ダウンロードされる。
 システム辞書 (dict.bin) は同梱しない。導入手順は
 https://github.com/fukuyori/karukan-feat-rakukan/blob/main/docs/dictionary.md
EOF

# fcitx5 reads addon and input-method metadata when the daemon starts. A
# maintainer script runs as root and cannot safely restart an arbitrary user's
# desktop-session daemon, so give an explicit instruction instead of silently
# leaving the newly installed IM absent from fcitx5-configtool.
cat > "$STAGE/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e

if [ "$1" = configure ]; then
    echo "Karukan: fcitx5 を実行中のユーザーとして再起動してください: fcitx5 -rd"
    echo "Karukan: その後 fcitx5-configtool を開き直し、Karukan を追加してください。"
fi
EOF
chmod 755 "$STAGE/DEBIAN/postinst"

# Without the input-method metadata the libraries install successfully, but
# Karukan never appears in the available-input-method list.
for packaged_file in \
    "$STAGE/usr/share/fcitx5/addon/karukan.conf" \
    "$STAGE/usr/share/fcitx5/inputmethod/karukan.conf"
do
    [ -f "$packaged_file" ] || {
        echo "エラー: パッケージに必要なファイルがありません: ${packaged_file#"$STAGE"}" >&2
        exit 1
    }
done
find "$STAGE/usr/lib" -path '*/fcitx5/karukan.so' -type f -print -quit | grep -q . || {
    echo "エラー: パッケージに karukan.so がありません" >&2
    exit 1
}
find "$STAGE/usr/lib" -path '*/fcitx5/libkarukan_fcitx5.so' -type f -print -quit | grep -q . || {
    echo "エラー: パッケージに libkarukan_fcitx5.so がありません" >&2
    exit 1
}

# md5sums (DEBIAN 以下を除く全ファイル)
(cd "$STAGE" && find . -type f -not -path './DEBIAN/*' -printf '%P\n' \
    | sort | xargs md5sum > DEBIAN/md5sums)

echo "==> dpkg-deb"
mkdir -p "$DIST"
dpkg-deb --build --root-owner-group "$STAGE" "$DIST/$PKG.deb"
rm -rf "$STAGE"

echo "==> 生成物"
ls -lh "$DIST/$PKG.deb"
dpkg-deb --info "$DIST/$PKG.deb" | sed -n '1,14p'
echo
echo "内容確認:  dpkg -c $DIST/$PKG.deb"
echo "インストール: sudo apt install $DIST/$PKG.deb"
if command -v lintian >/dev/null 2>&1; then
    echo "==> lintian(参考)"
    lintian --no-tag-display-limit "$DIST/$PKG.deb" || true
fi
