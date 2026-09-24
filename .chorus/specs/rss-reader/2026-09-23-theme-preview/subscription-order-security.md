# 订阅排序、正文安全与持久性验收

## 实现

v14新增feeds.position，NULL保持原名称排序；组内相对移动在core事务内读取当前顺序并写入。不存在的目标/不同文件夹会拒绝，自身移动无改动，新增订阅排在已排列项之后。桌面拖拽与上移/下移菜单共用move_feed；侧栏沿用keyed reconcile，不重建文章正文。跨组移动使用既有编辑菜单。

## 运行证据

结果：[subscription-order-security-results.json](subscription-order-security-results.json)。

- 重建桌面资源后，Xvfb英文浅色22项、中文深色23项通过。每档继承基础空态/失败/恢复检查，新增三源排序、重启保留、取消删除零修改、确认仅删除选中源。
- 排序实际使用xdotool原生指针事件，没有直接调用move_feed或合成drop。最终事件记录含accepted dragover/drop；顺序[1,2,3]→[3,1,2]，重启仍相同。
- 早期探针起拖后未收到drop，不计通过。最终使用windowraise/windowfocus、屏幕绝对坐标分步移动、目标行顶部向内8像素落点；两档均成功。没有增加“dragend即保存”之类产品补偿。
- 删除菜单打开与确认/取消按钮由DOM事件驱动，SQLite回读3→3→2；这部分不声称原生指针验证。
- 中文档将script、img onerror、javascript链接、svg onload和iframe srcdoc写入隔离库，重启后经真实文章行打开。WebKit事件正对照触发1次；正文executed=0、无活动脚本/事件属性/危险URL，安全段落仍可读。
- 此处是X11/Xvfb，不代替原生Wayland、GNOME或其它平台拖拽/安全验收。

## 自动测试

- feed_order三条：移动前后/错误输入原子性/重启、组边界及新订阅追加、v13升级保留排序与限流期限。
- Node新增两条：越组/失效/自身移动不发IPC，正确目标参数；删除取消不调用后端，确认只传指定ID。Node共51条通过。
- durability三条（含子进程入口）：逐版本v1至v13升级当前v14，检查订阅、文章、已读/星标、抓取时间和ETag；子进程保持数据库打开，确认已提交且WAL非空后kill，再打开验证同一批状态。仅测试隔离库。

## 复现

```bash
df -h .
cargo build -p rustrss-core --example theme_fixture
cargo build -p rustrss-desktop
/usr/bin/python3 scripts/verify-empty-error-states.py --organize
/usr/bin/python3 scripts/verify-empty-error-states.py --organize --locale zh-CN --theme dark
cargo test -p rustrss-core --test feed_order --test durability
node --test scripts/tests/*.test.cjs
```

## 限制

未验证跨平台原生拖拽；逐版本升级夹具覆盖需求中的基础行与标志，不代表每个历史AI/标签/主题配置组合都已穷举。强杀覆盖应用进程终止，不等于磁盘断电/文件系统损坏恢复。

最终全量回归：Rust 439 passed / 0 failed（30段）、Node51/0、clippy仅原有3条警告。退出码均0。
