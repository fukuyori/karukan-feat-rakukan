#!/usr/bin/env bash
# Debian/Ubuntu 配布用パッケージ (.deb) のビルド
#
#   scripts/build-deb.sh          # dist/karukan-fcitx5_<ver>_<arch>.deb を生成
#
# - 配布用に -C target-cpu=native を使わずビルドする (KARUKAN_NATIVE=OFF)
# - 固定版のシステム辞書とライセンス文書を同梱する
# - KARUKAN_DICT_ARCHIVE で取得済みの同一アーカイブを指定可能
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ADDON_DIR="$REPO_ROOT/karukan-im/fcitx5/fcitx5-addon"
BUILD_DIR="$ADDON_DIR/build-deb"
STAGE="$REPO_ROOT/dist/stage"
DIST="$REPO_ROOT/dist"

for cmd in dpkg-deb dpkg cmake cargo strip curl tar sha256sum; do
    command -v "$cmd" >/dev/null 2>&1 || { echo "エラー: $cmd がありません" >&2; exit 1; }
done

if ! dpkg-query -W -f='${db:Status-Abbrev}' extra-cmake-modules 2>/dev/null \
    | grep -q '^ii '; then
    cat >&2 <<'EOF'
エラー: CMake の ECM (extra-cmake-modules) がありません。

Debian/Ubuntuでは、ビルド依存関係を次のコマンドで導入してください:

  sudo apt install extra-cmake-modules fcitx5-modules-dev \
      libfcitx5core-dev libfcitx5config-dev libfcitx5utils-dev \
      libxkbcommon-dev clang libclang-dev libssl-dev
EOF
    exit 1
fi

# Pin both the release and digest so a release cannot silently change dictionaries.
DICT_URL="https://github.com/togatoga/karukan/releases/download/v0.1.0/dict.tgz"
DICT_SHA256="f194d2526bf826622bc5baf7c07e7525ad9bde2dc63896cd929045283a4aeccd"
DICT_ARCHIVE="${KARUKAN_DICT_ARCHIVE:-$DIST/cache/dict-$DICT_SHA256.tgz}"
if [ ! -f "$DICT_ARCHIVE" ]; then
    if [ -n "${KARUKAN_DICT_ARCHIVE:-}" ]; then
        echo "エラー: 辞書アーカイブがありません: $DICT_ARCHIVE" >&2
        exit 1
    fi
    mkdir -p "$(dirname "$DICT_ARCHIVE")"
    curl -fL --retry 3 -o "$DICT_ARCHIVE.part" "$DICT_URL"
    mv "$DICT_ARCHIVE.part" "$DICT_ARCHIVE"
fi
printf '%s  %s\n' "$DICT_SHA256" "$DICT_ARCHIVE" | sha256sum --check
DICT_TMP="$(mktemp -d)"
trap 'rm -rf "$DICT_TMP"' EXIT
# Extract only the payload and its original attribution/license documents.
tar xzf "$DICT_ARCHIVE" -C "$DICT_TMP" --no-same-owner --no-same-permissions \
    dict.bin docs/LEGAL docs/LICENSE-2.0.txt docs/README.md
for file in dict.bin docs/LEGAL docs/LICENSE-2.0.txt docs/README.md; do
    [ -s "$DICT_TMP/$file" ] || { echo "エラー: 辞書の必須ファイルがありません: $file" >&2; exit 1; }
done

ARCH="$(dpkg --print-architecture)"
HASH="$(git -C "$REPO_ROOT" rev-parse --short HEAD)"
DATE="$(git -C "$REPO_ROOT" show -s --format=%cd --date=format:%Y%m%d HEAD)"
# 日付を含めるのはアップグレード時のバージョン比較を単調にするため
# (ハッシュだけでは辞書順が時系列にならない)
VERSION="0.1.0+${DATE}.g${HASH}"
if [ -n "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=no)" ]; then
    VERSION="${VERSION}.dirty"
fi
PKG="karukan-fcitx5_${VERSION}_${ARCH}"

# Keep the Debian package version and the version reported by the binary in
# sync. github-release.sh overrides this with the release tag for official builds.
export KARUKAN_BUILD_VERSION="${KARUKAN_BUILD_VERSION:-$VERSION}"

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
mkdir -p "$DOC/dictionary" "$STAGE/usr/share/karukan-im"
install -m 644 "$DICT_TMP/dict.bin" "$STAGE/usr/share/karukan-im/dict.bin"
install -m 644 "$DICT_TMP"/docs/{LEGAL,LICENSE-2.0.txt,README.md} "$DOC/dictionary/"
printf 'Source: %s\nSHA256: %s\n' "$DICT_URL" "$DICT_SHA256" > "$DOC/dictionary/SOURCE"
install -m 644 "$REPO_ROOT"/{LICENSE-MIT,LICENSE-APACHE,THIRD_PARTY_LICENSES} "$DOC/"
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

Files: usr/share/karukan-im/dict.bin
Copyright: Works Applications Co., Ltd.
           2011-2013 The UniDic Consortium
           2015-2019 Toshinori Sato
License: Apache-2.0 and BSD-3-Clause
 SudachiDict 由来の加工済み辞書。出典と加工内容は dictionary/README.md、
 第三者の著作権表示・条件・免責事項は dictionary/LEGAL、
 Apache License 全文は dictionary/LICENSE-2.0.txt を参照。
EOF
cat > "$DOC/changelog" <<EOF
karukan-fcitx5 ($VERSION) unstable; urgency=medium

  * Build from git $HASH.
    See https://github.com/fukuyori/karukan-feat-rakukan/commits/develop

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
 システム辞書 (SudachiDict 由来) とライセンス文書を同梱。
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
    "$STAGE/usr/share/karukan-im/dict.bin" \
    "$DOC/dictionary/LEGAL" \
    "$DOC/dictionary/LICENSE-2.0.txt" \
    "$DOC/dictionary/README.md" \
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
