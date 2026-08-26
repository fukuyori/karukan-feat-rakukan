#!/usr/bin/env bash
# GitHub Release の作成(タグ付け → .deb ビルド → アップロードまで)
#
#   scripts/release.sh                    # 次の v0.1.0-rakukan.<N+1> でリリース
#   scripts/release.sh v0.2.0-rakukan.1   # タグ名を指定
#   scripts/release.sh --dry-run          # タグ名と変更点だけ表示して終了
#   scripts/release.sh --skip-tests       # cargo test を省略
#
# バージョン表記 (git describe --tags) がタグ名そのものになるよう、
# タグを **ビルドの前に** ローカルへ作成するのがこのスクリプトの要。
# 逆順(ビルド → gh release create でリモートだけにタグ)だと、配布物の
# バイナリは v...-rakukan.N-1-g<hash> のような開発ビルド表記のままになる。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DRY_RUN=0
SKIP_TESTS=0
TAG=""
for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=1 ;;
        --skip-tests) SKIP_TESTS=1 ;;
        --help|-h)
            sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        v*) TAG="$arg" ;;
        *) echo "エラー: 不明な引数 $arg (--help 参照)" >&2; exit 1 ;;
    esac
done

for cmd in git gh cargo dpkg-deb sha256sum; do
    command -v "$cmd" >/dev/null 2>&1 || { echo "エラー: $cmd がありません" >&2; exit 1; }
done

# --- 前提チェック -----------------------------------------------------------

[ "$(git branch --show-current)" = main ] || { echo "エラー: main ブランチで実行してください" >&2; exit 1; }
[ -z "$(git status --porcelain --untracked-files=no)" ] || {
    echo "エラー: 未コミットの変更があります (-dirty がバイナリに入ります)" >&2; exit 1; }

echo "==> タグと origin/main を取得"
git fetch --tags origin
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || {
    echo "エラー: HEAD が origin/main と一致しません (push または pull してください)" >&2; exit 1; }

# --- タグ名の決定 -----------------------------------------------------------

LAST="$(git tag -l 'v*-rakukan.*' --sort=-v:refname | head -1)"
if [ -z "$TAG" ]; then
    [ -n "$LAST" ] || { echo "エラー: 既存の v*-rakukan.* タグが無いためタグ名を自動決定できません。引数で指定してください" >&2; exit 1; }
    TAG="${LAST%.*}.$(( ${LAST##*.} + 1 ))"
fi
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && {
    echo "エラー: タグ $TAG は既に存在します" >&2; exit 1; }

echo "==> リリース: $TAG (直前のリリース: ${LAST:-なし})"
echo "==> 変更点 (${LAST:+$LAST..}HEAD):"
git log --format='  - %s (%h)' ${LAST:+$LAST..}HEAD

if [ "$DRY_RUN" = 1 ]; then
    echo "==> --dry-run のためここで終了 (タグ作成・ビルド・アップロードなし)"
    exit 0
fi

# --- テスト -----------------------------------------------------------------

if [ "$SKIP_TESTS" = 0 ]; then
    echo "==> cargo test --workspace"
    cargo test --workspace --quiet
fi

# --- タグ付けとビルド -------------------------------------------------------

echo "==> タグを作成: $TAG"
git tag "$TAG"
# 以降の失敗でローカルタグを残さない (push 済みになった時点で解除)
TAG_PUSHED=0
cleanup() {
    if [ "$TAG_PUSHED" = 0 ]; then
        echo "==> 失敗したためローカルタグ $TAG を削除" >&2
        git tag -d "$TAG" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

echo "==> .deb をビルド"
scripts/build-deb.sh

DEB="$(ls -t dist/karukan-fcitx5_*.deb | head -1)"

# 配布物のバイナリがタグ名そのものを名乗ることを確認
EMBEDDED="$(dpkg-deb --fsys-tarfile "$DEB" \
    | tar -xO ./usr/lib/*/fcitx5/libkarukan_fcitx5.so \
    | strings | grep -oE 'karukan-version:[^:]+:' | head -1 \
    | sed 's/^karukan-version://; s/:$//')"
[ "$EMBEDDED" = "$TAG" ] || {
    echo "エラー: バイナリのバージョン表記 ($EMBEDDED) がタグ ($TAG) と一致しません" >&2; exit 1; }
echo "==> バイナリのバージョン表記: $EMBEDDED"

echo "==> SHA256SUMS"
(cd dist && sha256sum "$(basename "$DEB")" > SHA256SUMS && cat SHA256SUMS)

# --- 公開 -------------------------------------------------------------------

echo "==> タグを push"
git push origin "$TAG"
TAG_PUSHED=1

NOTES="## 変更点${LAST:+ ($LAST から)}

$(git log --format='- %s (%h)' ${LAST:+$LAST..}HEAD)

## インストール

\`\`\`bash
sudo apt install ./$(basename "$DEB")
\`\`\`

- 対応: Debian/Ubuntu 系 (amd64)、fcitx5
- 変換モデルは初回起動時に Hugging Face から自動ダウンロード
- システム辞書 (dict.bin) は同梱しない。導入手順は [docs/dictionary.md](https://github.com/fukuyori/karukan-feat-rakukan/blob/main/docs/dictionary.md)
- CPU 固有命令なしの汎用ビルド

## 検証

\`\`\`bash
sha256sum -c SHA256SUMS
\`\`\`"

echo "==> gh release create $TAG"
gh release create "$TAG" "$DEB" dist/SHA256SUMS --title "$TAG" --notes "$NOTES"

echo "==> 完了: https://github.com/fukuyori/karukan-feat-rakukan/releases/tag/$TAG"
echo "リリースノートの追記: gh release edit $TAG --notes-file <file>"
