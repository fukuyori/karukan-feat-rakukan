#!/usr/bin/env bash
# README 用デモ GIF の生成
#
#   scripts/make-demo-gif.sh [録画ファイル.webm]
#
# 引数を省略すると ~/Videos/Screencasts/ の最新の録画を使う
# (GNOME の画面録画: Ctrl+Alt+Shift+R で開始/停止)。
# 2パス(パレット生成 → 適用)で images/demo.gif に出力する。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$REPO_ROOT/images/demo.gif"

command -v ffmpeg >/dev/null 2>&1 || {
    echo "ffmpeg が必要です: sudo apt install ffmpeg" >&2
    exit 1
}

INPUT="${1:-}"
if [ -z "$INPUT" ]; then
    INPUT="$(ls -t "$HOME/Videos/Screencasts"/*.webm 2>/dev/null | head -1 || true)"
    [ -n "$INPUT" ] || { echo "録画が見つかりません (~/Videos/Screencasts/*.webm)" >&2; exit 1; }
fi
echo "==> 入力: $INPUT"

# 幅800px・12fps。パレットを作ってから適用すると GIF の画質が大きく上がる。
FILTERS="fps=12,scale=800:-1:flags=lanczos"
PALETTE="$(mktemp --suffix=.png)"
trap 'rm -f "$PALETTE"' EXIT
ffmpeg -v warning -i "$INPUT" -vf "$FILTERS,palettegen=stats_mode=diff" -y "$PALETTE"
ffmpeg -v warning -i "$INPUT" -i "$PALETTE" \
    -lavfi "$FILTERS [x]; [x][1:v] paletteuse=dither=bayer:bayer_scale=4" \
    -y "$OUT"

echo "==> 出力: $OUT ($(du -h "$OUT" | cut -f1))"
echo "確認してよければ: git add images/demo.gif && git commit"
