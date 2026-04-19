# lazydb

スタンドアロン TUI SQL クライアント。任意のターミナルで動作する Rust 製シングルバイナリ。

## 特徴

- **TUI**: 3ペインレイアウト（Schema Browser / Query Editor / Results）
- **マルチ DB**: PostgreSQL / MySQL 対応
- **vim キーバインド**: Editor は Normal/Insert モーダル編集
- **接続方式**: Direct / SSH トンネル / AWS SSM トンネル
- **オートコンプリート**: SQL キーワード・テーブル名・カラム名のサジェスト
- **クエリ履歴**: NDJSON で自動保存、インクリメンタル検索
- **エクスポート**: CSV / JSON ファイル出力
- **安全機能**: readonly 接続での書き込みブロック、自動 LIMIT 付与

## インストール

```bash
brew install takuyatechexpert/tap/lazydb
```

## 使い方

### TUI モード

```bash
# 接続ピッカーから選択して起動
lazydb

# 接続名を指定して起動
lazydb --connection staging

# 設定ファイルを指定
lazydb --config ~/my-config.yml --connections ~/my-connections.yml
```

### CLI モード（非インタラクティブ）

```bash
# SQL を直接実行
lazydb exec -c local -q "SELECT * FROM users LIMIT 10"

# SQL ファイルを実行
lazydb exec -c local -f query.sql --format json

# 接続一覧を表示
lazydb list-connections

# パスワードを OS キーチェーンに保存
lazydb set-password local

# パスワードを OS キーチェーンから削除
lazydb delete-password local
```

## 設定

### `~/.config/lazydb/connections.yml`

```yaml
# PostgreSQL（db_type 省略時のデフォルト）
- type: direct
  name: local-pg
  label: local
  host: localhost
  port: 5432
  database: mydb
  user: postgres
  password: "keychain:local-pg"  # keychain:NAME / env:VAR / prompt / 平文

# MySQL
- type: direct
  name: local-mysql
  label: local
  db_type: mysql
  host: localhost
  port: 3306
  database: mydb
  user: root
  password: "env:MYSQL_PWD"

- type: ssh
  name: staging-db
  label: stg
  readonly: true
  ssh_host: bastion-stg        # ~/.ssh/config の Host エイリアス可
  # ssh_user: ec2-user         # 省略時は SSH config に委譲
  remote_db_host: db.example.internal
  local_port: 15432
  database: mydb
  user: postgres

- type: ssm
  name: prod-db
  label: prd
  readonly: true
  instance_id: i-XXXXXXXXXXXXXXXXX
  ssh_user: ec2-user
  ssh_key: ~/.ssh/id_rsa
  aws_profile: production
  remote_db_host: db.example.internal
  local_port: 15433
  database: mydb
  user: postgres
```

### `~/.config/lazydb/config.yml`

```yaml
default_limit: 100          # 自動 LIMIT 件数（0: 無効）
auto_connect: false         # true: 起動時に default_connection へ自動接続
default_connection: local   # auto_connect 時の接続名
```

## キーバインド

### グローバル

| キー | 動作 |
|------|------|
| `Tab` / `Shift+Tab` | パネル移動 |
| `Ctrl+E` | クエリ実行 |
| `Ctrl+H` | 履歴ピッカー |
| `Ctrl+X` | エクスポート |
| `Ctrl+C` | 接続切り替え |
| `Ctrl+Q` | 終了 |
| `?` | ヘルプ |

### タブ操作

| キー | 動作 |
|------|------|
| `Ctrl+T` | 新規タブ追加 |
| `Ctrl+W` | アクティブタブを閉じる |
| `Ctrl+N` | 次のタブへ |
| `Ctrl+P` | 前のタブへ |

### Editor Normal モード

| キー | 動作 |
|------|------|
| `i` / `a` / `A` / `o` / `O` | Insert モードへ |
| `h` / `j` / `k` / `l` | カーソル移動 |
| `w` / `b` / `e` | 単語移動 |
| `0` / `$` / `^` | 行頭 / 行末 / 非空白 |
| `gg` / `G` | 先頭 / 末尾 |
| `x` / `dd` / `D` / `C` | 削除 |
| `u` / `Ctrl+R` | undo / redo |

### Editor Insert モード

| キー | 動作 |
|------|------|
| `Esc` | Normal モードへ |
| `Tab` | サジェスト候補選択 |

### Results

| キー | 動作 |
|------|------|
| `j` / `k` | 行移動 |
| `h` / `l` | 横スクロール |
| `g` / `G` | 先頭 / 末尾 |
| `Ctrl+D` / `Ctrl+U` | ページ移動 |
| `y` | 行データコピー |

## 必要要件

- Rust 1.70+
- SSH トンネル使用時: `ssh` コマンド
- SSM トンネル使用時: AWS CLI (`aws`)

> **Note:** PostgreSQL / MySQL への接続はネイティブドライバ（sqlx）を使用するため、`psql` や `mysql` コマンドのインストールは不要です。

## ライセンス

MIT
