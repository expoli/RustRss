# Tech Design: 一键备份/恢复

- 模块: rss-reader / backup-restore
- 日期: 2026-09-21（rev2）

## 机制（rev2：在线快照 + 启动时替换 + 崩溃容忍）

```
备份：rusqlite backup API（feature "backup"）在线快照 → dest/RustRss-backup-<ts>.sqlite
      （对活跃读写一致；产物独立干净，无 -wal/-shm 依赖；不做 checkpoint）
恢复：validate（只读打开 + user_version ≤ 当前）→ 确认 → fs::copy(备份, data_dir/pending-restore.sqlite)
      → 提示重启（不做退出时替换：MCP 第二连接存活 + Windows 不能 rename 打开文件）
启动：main() 最前、AppState::open 与 MCP runtime 启动之前：
      apply_pending_restore(db_path)（任何连接打开前执行 → 无边车、无锁）
```

## apply_pending_restore(db_path) 算法（崩溃容忍）

```
pending = db_dir/"pending-restore.sqlite"
若 pending 不存在：
    若 db 缺失 && 存在 bak-*：rename(bak 最新一份 → db)（崩溃回滚）→ 返回 true
    否则返回 false（无事可做，不动现库；正常启动路径——绝不触碰 -wal/-shm，
                     未 checkpoint 的已提交事务在 Store::open 时正常回放）
若 pending 存在：
    若 db 存在：rename(db → db.bak-<ts>)；删除更旧的 bak-*（只留最新 1 份）
    （db 缺失不报错：上次崩溃在 rename 之间，替换意图优先，继续）
    删除 db-wal、db-shm（R3 修订：必须在 rename(pending→db) **之前**——此时旧库已挪
     bak，边车属于旧库，删除安全；若先 rename 后删边车，两步之间崩溃会留下
     stale WAL，且下次启动 pending 已不在 → apply no-op → Store::open 回放旧帧
     污染恢复库，静默损坏）
    rename(pending → db)
    返回 true
```

**不变量**：边车删除严格先于 `rename(pending → db)`，故「pending 已消费 + db 已替换 +
stale 边车残留」这一崩溃态在构造上不可达。评审备选「apply 入口无条件删边车」被否：
正常启动（无 pending）下它会在 Store::open 前丢掉 WAL 中未 checkpoint 的已提交事务，
属数据丢失；仅 pending 分支内删除是安全的。

- Windows 安全：启动时所有连接未打开，rename/delete 均可执行。
- 保底回滚：bak 文件保留最近 1 份，用户可手工恢复。

## 分层

- `rustrss-core/src/store/backup.rs`：
  - `export_backup(store: &Store, dest_dir: &Path) -> Result<PathBuf>`（backup API 快照；时间戳命名）
  - `validate_backup(path: &Path, current_version: i64) -> Result<()>`（`OpenFlags::SQLITE_OPEN_READ_ONLY` 打开）
  - `stage_restore(backup: &Path, data_dir: &Path) -> Result<()>`（fs::copy → pending-restore.sqlite）
  - `apply_pending_restore(db_path: &Path) -> Result<bool>`（上述算法；纯文件操作，无连接）
- Cargo：rustrss-core 加 rusqlite feature `backup`。
- `src-tauri/commands.rs`：`backup_db` / `restore_db`（dialog + validate + stage；成功提示重启）。
- `main.rs`：`main()` 最前调 `apply_pending_restore(&resolve_db_path())`（在 AppState::open 之前；MCP 随后）。
- UI：设置页「备份」「恢复」两按钮 + 恢复确认（tauri dialog confirm）。

## 测试（core 层，tests/backup.rs）

1. 建库写入 feeds/entries/read/starred/settings → 开第二个只读连接 → export_backup → 新 Store 打开备份，全量一致（含 starred）。
2. validate_backup：合法 ok；文本文件 err；高 user_version err。
3. stage → apply：现库被替换为备份内容；pending 清理；**预置非空 db-wal/db-shm 后 apply，边车被删且打开 db 内容即备份内容**；幂等（再次 apply false）。
4. 崩溃态 A（db 缺失 + pending 在）→ apply 完成替换；崩溃态 B（db 缺失 + bak 在 + 无 pending）→ 回滚 bak；崩溃态 C（**db 已挪 bak、边车已删、pending 尚未 rename 的窗口**——构造 db 缺失 + stale 边车 + pending 在）→ apply 完成替换且边车不存在、内容为 pending。
5. 无 pending 且 db 正常 → apply 不动现库。
