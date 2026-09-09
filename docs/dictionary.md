# Dictionary

karukan はモデル推論に加えて、システム辞書・ユーザー辞書からの変換候補を提供します。辞書の構築・管理ツールについては [karukan-cli の README](../karukan-cli/README.md) を参照してください。

> [!NOTE]
> モデル推論だけでは語彙が限られるため、システム辞書の併用を強く推奨します。配布用 `.deb` にはシステム辞書とライセンス文書を同梱しています。ソースから導入する場合は `install.sh` が辞書を取得します。

## System Dictionary

double-array trieベースのシステム辞書です。

- デフォルトパス: `~/.local/share/karukan-im/dict.bin`（macOS: `~/Library/Application Support/com.karukan.karukan-im/dict.bin`）
- `dict_path` で任意のパスを指定可能（[Configuration](configuration.md) 参照）
- Linux の `.deb` 同梱辞書: `/usr/share/karukan-im/dict.bin`
- 読み込み優先順位: 明示的な `dict_path` → ユーザーのデフォルトパス → `.deb` 同梱辞書
- `dict_path` を指定した場合はそのファイルのみを使用。読み込み失敗時には警告を記録
- 同梱辞書は全ユーザーで共有し、パッケージ更新時に更新。個人の辞書ファイルは上書きしない
- 辞書が存在しない場合は辞書なしで動作

同梱辞書は SudachiDict 由来で、ライセンス・第三者の権利表示・加工内容は
`/usr/share/doc/karukan-fcitx5/dictionary/` に収録しています。

`.deb` を使わず手動で配置する場合は、以下からダウンロードできます:

```bash
# Linux
wget https://github.com/togatoga/karukan/releases/latest/download/dict.tgz
tar xzf dict.tgz
mkdir -p ~/.local/share/karukan-im
cp dict.bin ~/.local/share/karukan-im/

# macOS
curl -LO https://github.com/togatoga/karukan/releases/latest/download/dict.tgz
tar xzf dict.tgz
mkdir -p ~/Library/"Application Support"/com.karukan.karukan-im
cp dict.bin ~/Library/"Application Support"/com.karukan.karukan-im/
```

自分でビルドする場合は [karukan-cli の README](../karukan-cli/README.md) を参照してください。

## User Dictionary

ユーザー辞書ディレクトリにファイルを配置すると、ユーザー辞書として読み込まれます。対応形式と登録方法の詳細は [user-dictionary.md](user-dictionary.md) を参照してください。

- デフォルトパス: `~/.local/share/karukan-im/user_dicts/`（macOS: `~/Library/Application Support/com.karukan.karukan-im/user_dicts/`）
- ディレクトリ内のファイルはすべて自動で読み込み（KRKNバイナリ・Mozc TSV を自動判定）
- ディレクトリが存在しない場合はユーザー辞書なしで動作

## 変換候補の優先順位

1. 📝 学習キャッシュ
2. 👤 ユーザー辞書
3. 🤖 モデル推論
4. 📚 システム辞書（スコア順）
5. ひらがな / カタカナ
6. 🔄 Rewriter（半角カタカナ・英字全角半角・記号バリアント）
