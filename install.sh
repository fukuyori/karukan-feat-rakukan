#!/usr/bin/env bash
# Karukan feat. Rakukan — Linux (fcitx5) インストーラ
#
# ソースからビルドして fcitx5 アドオンをインストールする。
#   ./install.sh                # ユーザーローカル (~/.local、sudo 不要)
#   ./install.sh --system      # システム (/usr、sudo 必要)
#   ./install.sh --uninstall   # アンインストール (--system と組み合わせ可)
#
# オプション:
#   --no-dict     システム辞書 (dict.tgz) をダウンロードしない
#   --no-native   -C target-cpu=native を使わない(配布用ビルド)
#   --restart     インストール後に fcitx5 を再起動する
#   --purge       アンインストール時にユーザーデータ(辞書・学習・設定)も消す
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ADDON_DIR="$REPO_ROOT/karukan-im/fcitx5/fcitx5-addon"
DICT_URL="https://github.com/togatoga/karukan/releases/latest/download/dict.tgz"
DATA_DIR="$HOME/.local/share/karukan-im"
ENV_CONF="$HOME/.config/environment.d/fcitx5-karukan.conf"

MODE=user
UNINSTALL=0
WANT_DICT=1
NATIVE=ON
RESTART=0
PURGE=0

for arg in "$@"; do
    case "$arg" in
        --system) MODE=system ;;
        --user) MODE=user ;;
        --uninstall) UNINSTALL=1 ;;
        --no-dict) WANT_DICT=0 ;;
        --no-native) NATIVE=OFF ;;
        --restart) RESTART=1 ;;
        --purge) PURGE=1 ;;
        -h|--help)
            sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *) echo "不明なオプション: $arg (--help を参照)" >&2; exit 1 ;;
    esac
done

if [ "$MODE" = system ]; then
    PREFIX=/usr
    SUDO=sudo
else
    PREFIX="$HOME/.local"
    SUDO=""
fi

info() { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m警告:\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31mエラー:\033[0m %s\n' "$*" >&2; exit 1; }

# --- アンインストール ------------------------------------------------------

if [ "$UNINSTALL" = 1 ]; then
    info "アンインストールします (prefix: $PREFIX)"
    libdir="$PREFIX/lib"
    [ "$MODE" = system ] && [ -d /usr/lib/x86_64-linux-gnu/fcitx5 ] && libdir=/usr/lib/x86_64-linux-gnu
    files=(
        "$libdir/fcitx5/karukan.so"
        "$libdir/fcitx5/libkarukan_fcitx5.so"
        "$PREFIX/share/fcitx5/addon/karukan.conf"
        "$PREFIX/share/fcitx5/inputmethod/karukan.conf"
        "$PREFIX/share/metainfo/org.fcitx.Fcitx5.Addon.Karukan.metainfo.xml"
    )
    for size in 16 24 32 48 128; do
        files+=("$PREFIX/share/icons/hicolor/${size}x${size}/apps/fcitx-karukan.png")
    done
    for f in "${files[@]}"; do
        if [ -e "$f" ]; then
            $SUDO rm -v "$f"
        fi
    done
    if [ "$MODE" = user ] && [ -f "$ENV_CONF" ]; then
        rm -v "$ENV_CONF"
    fi
    if [ "$PURGE" = 1 ]; then
        warn "ユーザーデータを削除します: $DATA_DIR と ~/.config/karukan-im"
        rm -rf "$DATA_DIR" "$HOME/.config/karukan-im"
    else
        info "ユーザーデータ($DATA_DIR: 辞書・学習履歴)は残しています(--purge で削除)"
    fi
    info "完了。fcitx5 を再起動してください: fcitx5 -rd"
    exit 0
fi

# --- 依存チェック ----------------------------------------------------------

info "依存関係を確認しています"
missing=()
for cmd in cargo cmake make g++ pkg-config clang; do
    command -v "$cmd" >/dev/null 2>&1 || missing+=("$cmd")
done
if ! pkg-config --exists Fcitx5Core 2>/dev/null; then
    missing+=("fcitx5 開発ヘッダ (libfcitx5core-dev 等)")
fi
if [ "${#missing[@]}" -gt 0 ]; then
    warn "不足: ${missing[*]}"
    cat <<'EOS'
次でインストールできます (Debian/Ubuntu):

  sudo apt install fcitx5 fcitx5-modules-dev libfcitx5core-dev \
      libfcitx5config-dev libfcitx5utils-dev extra-cmake-modules \
      cmake make gcc g++ pkg-config \
      clang libclang-dev \
      libssl-dev libxkbcommon-dev

Rust: https://www.rust-lang.org/tools/install
EOS
    fail "依存関係を解決してから再実行してください"
fi

# --- ビルドとインストール --------------------------------------------------

info "ビルドします (prefix: $PREFIX, target-cpu=native: $NATIVE)"
cmake_args=(
    -B "$ADDON_DIR/build"
    -S "$ADDON_DIR"
    -DCMAKE_INSTALL_PREFIX="$PREFIX"
    -DKARUKAN_NATIVE="$NATIVE"
)
if [ "$MODE" = user ]; then
    cmake_args+=(-DCMAKE_INSTALL_LIBDIR=lib)
fi
cmake "${cmake_args[@]}"
cmake --build "$ADDON_DIR/build" -j"$(nproc)"

info "インストールします"
$SUDO cmake --install "$ADDON_DIR/build"

# Fail here with a useful message instead of reporting success when CMake's
# imported fcitx5 paths accidentally sent some files to another prefix.
install_libdir="$(sed -n 's/^CMAKE_INSTALL_LIBDIR:[^=]*=//p' "$ADDON_DIR/build/CMakeCache.txt" | head -1)"
[ -n "$install_libdir" ] || fail "CMake のライブラリ配置先を取得できません"
case "$install_libdir" in
    /*) installed_libdir="$install_libdir/fcitx5" ;;
    *) installed_libdir="$PREFIX/$install_libdir/fcitx5" ;;
esac
for installed_file in \
    "$installed_libdir/karukan.so" \
    "$installed_libdir/libkarukan_fcitx5.so" \
    "$PREFIX/share/fcitx5/addon/karukan.conf" \
    "$PREFIX/share/fcitx5/inputmethod/karukan.conf"
do
    [ -f "$installed_file" ] || fail "インストールされたファイルが見つかりません: $installed_file"
done

# --- ユーザーローカル: FCITX_ADDON_DIRS ------------------------------------

NEED_RELOGIN=0
if [ "$MODE" = user ]; then
    system_fcitx5_dir="$(pkg-config --variable=libdir Fcitx5Core)/fcitx5"
    want_line="FCITX_ADDON_DIRS=$HOME/.local/lib/fcitx5:$system_fcitx5_dir"
    # システムパスを含まない古い設定(fcitx5 の標準アドオンが見つからなくなる
    # 既知の問題)もここで上書きして直す。
    if [ ! -f "$ENV_CONF" ] || [ "$(cat "$ENV_CONF")" != "$want_line" ]; then
        mkdir -p "$(dirname "$ENV_CONF")"
        echo "$want_line" > "$ENV_CONF"
        info "FCITX_ADDON_DIRS を設定しました: $ENV_CONF"
    fi
    case "${FCITX_ADDON_DIRS:-}" in
        *"$HOME/.local/lib/fcitx5"*) : ;;
        *) NEED_RELOGIN=1 ;;
    esac
fi

# --- システム辞書 ----------------------------------------------------------

if [ "$WANT_DICT" = 1 ]; then
    if [ -f "$DATA_DIR/dict.bin" ]; then
        info "システム辞書は導入済みです ($DATA_DIR/dict.bin)"
    else
        info "システム辞書をダウンロードします"
        tmp="$(mktemp -d)"
        trap 'rm -rf "$tmp"' EXIT
        if command -v curl >/dev/null 2>&1; then
            curl -fL -o "$tmp/dict.tgz" "$DICT_URL"
        else
            wget -O "$tmp/dict.tgz" "$DICT_URL"
        fi
        tar xzf "$tmp/dict.tgz" -C "$tmp"
        mkdir -p "$DATA_DIR"
        cp "$tmp/dict.bin" "$DATA_DIR/"
        info "配置しました: $DATA_DIR/dict.bin"
    fi
fi

# --- 仕上げ ----------------------------------------------------------------

# バイナリ内の `karukan-version:<ver>:` マーカーを読む(末尾の `:` が
# 境界。strings の出力では隣接する文字列が連結されるため、素のバージョン
# 文字列を正規表現で切り出すと後続文字列を巻き込むことがある)
version="$(strings "$installed_libdir/libkarukan_fcitx5.so" 2>/dev/null \
    | grep -oE 'karukan-version:[^:]+:' | head -1 \
    | sed 's/^karukan-version://; s/:$//' || true)"
info "インストール完了${version:+ (バージョン: $version)}"

if [ "$RESTART" = 1 ]; then
    info "fcitx5 を新しい検索パスで再起動します"
    if [ "$MODE" = user ]; then
        FCITX_ADDON_DIRS="$HOME/.local/lib/fcitx5:$system_fcitx5_dir" fcitx5 -rd || true
    else
        fcitx5 -rd || true
    fi
elif [ "$NEED_RELOGIN" = 1 ]; then
    warn "初回のユーザーローカルインストールです。FCITX_ADDON_DIRS を反映するため、いったんログアウトして再ログインしてください。"
    echo "  再ログイン後、fcitx5 の設定(fcitx5-configtool)で入力メソッドに「Karukan」を追加してください。"
else
    echo "反映するには fcitx5 を再起動してください: fcitx5 -rd"
    echo "初回は fcitx5 の設定(fcitx5-configtool)で入力メソッドに「Karukan」を追加してください。"
fi
echo "初回起動時は変換モデルをバックグラウンドでダウンロードします(その間もかな入力と辞書変換は使えます)。"
