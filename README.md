# lazydb

スタンドアロン TUI SQL クライアント。任意のターミナルで動作する Rust 製シングルバイナリ。

## 特徴

- **TUI**: 3ペインレイアウト（Schema Browser / Query Editor / Results）
- **マルチ DB**: PostgreSQL / MySQL 対応
- **複数タブ**: 接続ごとに独立した Editor / Results を複数タブで管理
- **vim キーバインド**: Editor は Normal/Insert モーダル編集、3ペイン共通の vim 風スクロール
- **SQL フォーマッタ**: Editor で `=` を押すとクエリ全体を整形
- **UPDATE 文生成**: Results の `cc` でカーソル行から UPDATE 文を自動生成し Editor へ追記（対象テーブルのカラム情報を自動フェッチ）
- **Schema Browser**: スキーマ動的取得、`/` でテーブル名インクリメンタル検索
- **接続方式**: Direct / SSH トンネル / AWS SSM トンネル
- **パスワード管理**: OS キーチェーン（macOS Keychain / Linux Secret Service）連携
- **オートコンプリート**: SQL キーワード・テーブル名・カラム名のサジェスト
- **クエリ履歴**: NDJSON で自動保存、インクリメンタル検索
- **エクスポート**: CSV / JSON ファイル出力
- **安全機能**: readonly 接続での書き込みブロック、自動 LIMIT 付与、設定ファイル/履歴のパーミッション 600 制限

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

### スクロール・画面移動（3ペイン共通）

| キー | 動作 |
|------|------|
| `j` / `k` / `↑` / `↓` | 縦1単位移動 |
| `h` / `l` / `←` / `→` | 横1単位移動 |
| `g` / `G` | 縦先頭 / 縦末尾 |
| `0` / `Home` | 横先頭 |
| `$` / `End` | 横末尾 |
| `PgDn` / `PgUp` | 縦20単位移動 |
| `Ctrl+D` / `Ctrl+U` | 縦20単位移動 |
| `H` / `L` | 横40単位移動 |
| `zz` | カーソル行を画面中央に寄せる |

### Schema Browser

| キー | 動作 |
|------|------|
| `Enter` | テーブル展開・折りたたみ |
| `s` | クイック `SELECT * FROM` を Editor へ挿入 |
| `y` | テーブル名をコピー |
| `r` | スキーマ再読み込み |
| `/` | テーブル名検索（`Enter` 確定 / `Esc` 取消） |
| `n` / `N` | 次/前の一致へ移動 |

### Editor Normal モード

| キー | 動作 |
|------|------|
| `i` / `a` / `A` / `o` / `O` | Insert モードへ |
| `w` / `b` / `e` | 単語移動 |
| `^` | 行の最初の非空白 |
| `x` / `dd` / `D` / `C` | 削除 |
| `u` / `Ctrl+R` | undo / redo |
| `=` | クエリ全体をフォーマット |

### Editor Insert モード

| キー | 動作 |
|------|------|
| `Esc` | Normal モードへ |
| `Tab` | サジェスト候補選択 |

### Results

| キー | 動作 |
|------|------|
| `y` | 行データをコピー |
| `cc` | カーソル行の UPDATE 文を生成し Editor へ追記 |

## 必要要件

- SSH トンネル使用時: `ssh` コマンド
- SSM トンネル使用時: AWS CLI (`aws`)

> **Note:** PostgreSQL / MySQL への接続はネイティブドライバ（sqlx）を使用するため、`psql` や `mysql` コマンドのインストールは不要です。

## ライセンス

MIT
