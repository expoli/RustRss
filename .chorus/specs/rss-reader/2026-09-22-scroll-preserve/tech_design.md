# Tech Design: 后台刷新后保持列表深度滚动位置

- 模块: rss-reader / scroll-preserve
- 日期: 2026-09-22（rev2：评审 B1 条件修复 + scrollTop=0 决策 + readSessionIds 全路径）

## 实现（纯前端，ui/app.js）

`initRefreshEvents` 的 `refresh:done` 处理器改造（loadAll({reader:false}) 拆为两支）：

```
if (state.entries.length > PAGE_SIZE && !paging.loading && !paging.error) {
    // prepend 路径（rev2：去掉 !exhausted——已耗尽的多页视图同样要保持位置）
    const first = state.entries[0];                    // 已加载首行
    const rows = await invoke('list_entries', {…当前筛选, limit: PAGE_SIZE});
    const fresh = rows.filter(r => (r.sortkey > first.sortkey)
                                || (r.sortkey === first.sortkey && r.id > first.id))
                     .filter(r => !state.readSessionIds.has(r.id));  // 会话已读不回插
    if (fresh.length) {
        const list = el('entries');
        const atTop = list.scrollTop === 0;            // rev2：在顶部不补偿——
        const before = list.scrollHeight;              // 用户正看着顶部，新条目立即可见
        const anchor = list.firstChild;
        for (const e of fresh.reverse()) {             // 旧→新顺序插入，新的在最上
            state.entries.unshift(e);
            list.insertBefore(buildEntryRow(e), anchor);
        }
        if (!atTop) list.scrollTop += list.scrollHeight - before;  // 滚动态才做视口补偿
        el('list-count').textContent = t('list.count', { n: state.entries.length });
    }
    refreshCounts();  // 现有 debounced 路径（侧栏/计数照旧）
} else {
    // 现有 reset 路径不变：loadAll({reader:false})
}
```

要点：
- **prepend 条件（rev2 评审 B1）**：`length > PAGE_SIZE && !paging.loading && !paging.error`——exhausted 不参与（小库/星标等已耗尽的多页视图同样保持位置）；loading/error 时走 reset 防与 loadMore 竞态。
- **scrollTop=0 决策（rev2 NOTE）**：在顶部不补偿，新条目立即可见；仅滚动态（scrollTop>0）补偿 scrollHeight 增量。
- **readSessionIds（rev2 NOTE）**：`state.readSessionIds = new Set()`；**所有 set_read 成功点**统一维护（openEntry 标读、toggleRead、markAll 类），视图切换 reset 时清空——任何路径读过的 id 刷新后都不回插。
- 游标不动物权：append 游标取「已取末行」，prepend 只动头部；exhausted 不变（总数只会更多）；哨兵在尾部不受影响。
- sortkey 比较用后端直出字段（P0-3 契约）。

## 测试

- headless（Xvfb + xdotool + 本地 fixture feed）：
  1. 加载 2 页 → 滚到中部 → 后台刷新（fixture 返回新条目）→ scrollTop/selectedId 前后一致 + 顶部出现新行；
  2. 滚回顶部（scrollTop=0）→ 刷新 → 不补偿、新行直接可见；
  3. 无新条目 → scrollTop 逐像素不变、零 DOM 写入；
  4. 未读视图：会话读 1 篇（行已删）→ 刷新 → 该 id 不回插（openEntry 路径与 toggleRead 路径各验一次）。
- 手动清单补 §11；spec.md 回填（渐进加载条目下补滚动保持）。
