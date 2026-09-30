# 阅读体验改版：逐动作迁移台账

来源：Chorus Proposal `b5585be7-a1d3-4311-8c1b-ce458ad649d6` 的《菜单盘点》M01–M20（OpenSpec slug `refine-rustrss-reading-experience`）。本表的 `ID` 是迁移核对 ID；T2 的运行时 action descriptor 使用此 ID 或记录映射。`新入口` 是批准后的目标，未完成前不代表已经实现。状态栏由责任任务回填 `待验 / 通过 + 证据路径 / 不适用 + 平台理由`，T7 逐行关闭。下面的操作均在隔离 fixture 中进行；危险项先取消，再经确认执行并回读。

| ID | 动作 | 旧入口 → 新入口 | 平台 | 状态或副作用；核验步骤 | 责任 | 证据 |
|---|---|---|---|---|---|---|
| M01.articles | 文章 | 侧栏/底栏 → 四主导航文章 | 双端 | 保持当前视图；切换并返回 | T3/T6 | Android 与 Linux 最终产物均完成文章→阅读→返回；[T7 Android 三旅程](evidence/t7-final/android-journeys-round2/android-journeys.json)、[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M01.subscriptions | 订阅 | 侧栏/底栏 → 四主导航订阅 | 双端 | 保留选中源；切换并返回 | T2/T6 | Android 与 Linux 最终产物新增/移动源并回读 ID；选中源跨页保留见旧 [T2](evidence/t2-menu/record.md)；[T7 Android 三旅程](evidence/t7-final/android-journeys-round2/android-journeys.json)、[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M01.saved | 收藏 | 侧栏/底栏 → 四主导航收藏 | 双端 | 保留星标/稍后读子视图；切换 | T3/T6 | Android 最终产物星标/稍后读后进入收藏并回读；Linux 最终组合未单独复测子视图；[T7 Android 三旅程](evidence/t7-final/android-journeys-round2/android-journeys.json) |
| M01.settings | 设置 | 工具栏/底栏 → 四主导航设置 | 双端 | 草稿离开规则；进入再返回 | T5/T6 | Android 草稿/预览/保存/取消与 Linux 模态保存均在最终产物复核；[T7 Android 三旅程](evidence/t7-final/android-journeys-round2/android-journeys.json)、[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M01.unread | 未读视图 | 智能视图/页内切换 → 标题旁筛选 | 双端 | 未读数同 core；切换 | T3 | Android 最终夹具 15 未读条目，切换行为沿用 [T3](evidence/t3-list/record.md)；Linux 最终组合未单独复测计数；[T7 Android 三旅程](evidence/t7-final/android-journeys-round2/android-journeys.json) |
| M01.all | 全部视图 | 智能视图/页内切换 → 标题旁筛选 | 双端 | 已读仍可见；切换 | T3 | Android 与 Linux 最终产物 All 打开文章；[T7 Android 三旅程](evidence/t7-final/android-journeys-round2/android-journeys.json)、[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M01.starred | 星标视图 | 智能视图/页内切换 → 收藏内筛选 | 双端 | 仅星标；切换 | T3 | Android 通过、Linux 最终组合未单独复测：[T3 证据](evidence/t3-list/record.md) |
| M01.later | 稍后读视图 | 智能视图/页内切换 → 收藏内筛选 | 双端 | 仅稍后读；切换 | T3 | Android 通过、Linux 最终组合未单独复测：[T3 证据](evidence/t3-list/record.md) |
| M02.search | 标题及正文搜索 | 顶部输入 → 搜索按钮展开输入 | 双端 | 手机明确全库 scope；查长标题/正文 | T3 | Linux 最终产物本地新增文章可搜索；[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M02.clear | 清除及取消搜索 | 输入清除/Esc → 展开区取消 | 双端 | 恢复前一筛选和锚点；搜索后取消 | T3 | 桌面搜索取消返回列表通过，原筛选/锚点未单独测量；[T6 原生记录](evidence/t6-convergence/record.md) |
| M02.sort.newest | 最新排序 | 列表排序菜单 → 列表更多 | 双端 | 存储 list.sort；切换并重启 | T3 | Android 通过、Linux 最终组合未单独复测：[T3 证据](evidence/t3-list/record.md) |
| M02.sort.oldest | 最旧排序 | 列表排序菜单 → 列表更多 | 双端 | 存储 list.sort；切换并重启 | T3 | Linux 最终产物最旧排序 SQLite 回读；[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M02.sort.unread | 未读优先 | 列表排序菜单 → 列表更多 | 双端 | 存储 list.sort；切换并重启 | T3 | Android 通过、Linux 最终组合未单独复测：[T3 证据](evidence/t3-list/record.md) |
| M02.hide-read | 隐藏已读 | 列表排序菜单/U → 列表更多/U | 双端 | 存储 list.hide_read；切换 | T3/T6 | Android 通过；桌面可信 U 两次及 SQLite true→false 通过：[T6 第二轮](evidence/t6-convergence/review-desktop-results.json) |
| M02.bulk.read | 当前作用域全标已读 | 列表批量菜单/A → 列表更多/A | 双端 | 确认 scope；执行并查库 | T3/T6 | Android 通过；桌面可信 A 将 All 范围 30 篇标已读并查库：[T6 第二轮](evidence/t6-convergence/review-desktop-results.json) |
| M02.bulk.unread | 当前作用域全标未读 | 列表批量菜单 → 列表更多 | 双端 | 确认 scope；执行并查库 | T3 | Android 通过、Linux 最终组合未单独复测：[T3 证据](evidence/t3-list/record.md) |
| M02.pagination | 续页、加载数、总数 | 列表尾部/头部 → 原位 | 双端 | 30/200行，M/N 正确；续页无重复 | T3 | Android 通过、Linux 最终组合未单独复测：[T3 证据](evidence/t3-list/record.md) |
| M02.retry | 续页重试 | 失败尾行 → 原位 | 双端 | 失败后重试保留筛选 | T3 | Android 通过、Linux 最终组合未单独复测：[T3 证据](evidence/t3-list/record.md) |
| M03.refresh-all | 刷新全部 | 工具栏/r → 顶部显式刷新/r | 双端 | 单 flight；观察开始/结束/失败 | T3/T6 | Android 通过；桌面可信 r 触发刷新中反馈，单 flight/完成未由此测试证明：[T6 第二轮](evidence/t6-convergence/review-desktop-results.json) |
| M03.refresh-feed | 单源刷新 | 源右键 → 源更多/右键 | 双端 | 单 flight；对象 ID 不漂移 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M03.status | 刷新状态 | 固定状态栏 → 稳定状态位 | 双端 | 不跳布局；触发和失败 | T3 | Android 通过、Linux 最终组合未单独复测：[T3 证据](evidence/t3-list/record.md) |
| M04.url | URL 或网站发现 | 添加订阅输入 → 订阅页明确添加 | 双端 | 候选发现；输入测试 URL | T2 | 桌面本地 HTTP RSS 直连新增与抓取通过，候选发现沿用 [T2 证据](evidence/t2-menu/record.md)；[T6 原生记录](evidence/t6-convergence/record.md) |
| M04.candidate | 候选源选择 | 发现结果 → 输入页选择 | 双端 | 选择目标源后新增 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M04.rsshub | RSSHub 入口 | 添加订阅 → 输入页 | 双端 | 输入完整 `rsshub://t2/menu-fixture`；本地镜像命中并回读源 URL/ID/成功状态 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M04.result | 添加反馈 | 原状态位 → 稳定反馈位 | 双端 | 成功/失败，输入保留 | T2/T5 | 桌面新增成功落库通过，失败反馈沿用 [T2 证据](evidence/t2-menu/record.md)；[T6 原生记录](evidence/t6-convergence/record.md) |
| M05.select | 选择源、未读数与错误 | 侧栏行 → 可见行与状态 | 双端 | 选中/错误/计数；切换源 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.move-up | 源上移 | 右键 → 更多/右键整理组 | 双端 | 排序落库；执行并重启 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.move-down | 源下移 | 右键 → 更多/右键整理组 | 双端 | 排序落库；执行并重启 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.refresh | 立即刷新源 | 右键 → 更多/右键更新组 | 双端 | 单源刷新；观察结果 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.edit | 编辑源 | 右键 → 更多/右键 | 双端 | 进入原表单；取消/保存 | T2/T5 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.interval | 源刷新间隔 | 级联菜单 → 面板内选择页/桌面级联 | 双端 | 当前值、覆盖优先；切换并回读 | T2/T6 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.folder | 移动源至文件夹/未分组 | 级联菜单 → 面板内选择页/桌面级联 | 双端 | 当前值、对象 ID；移动并回读 | T2/T6 | Android 与 Linux 最终产物移动源到文件夹并回读对象 ID；[T7 Android 三旅程](evidence/t7-final/android-journeys-round2/android-journeys.json)、[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M05.unsubscribe | 取消订阅 | 右键 → 更多危险组 | 双端 | 显示影响、取消无写且焦点回触发项；确认级联且焦点回存活行/标题 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M06.name | 源自定义名称 | 编辑对话框 → 对象编辑页 | 双端 | 保存/取消并回读 | T2/T5 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M06.folder | 编辑源文件夹 | 编辑对话框 → 对象编辑页 | 双端 | 未分组也可选；回读 | T2/T5 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M06.interval | 编辑源刷新间隔 | 编辑对话框 → 对象编辑页 | 双端 | 继承/覆盖；回读 | T2/T5 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M06.url | 只读源 URL | 编辑对话框 → 对象编辑页 | 双端 | URL 不可被编辑；查看 | T2/T5 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M07.create | 新建文件夹 | 侧栏加号 → 订阅页添加 | 双端 | 新建并回读 | T2 | Linux 最终产物新建文件夹 ID 回读；[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M07.aggregate | 文件夹聚合列表 | 侧栏选择 → 文件夹行选择 | 双端 | 作用域计数；打开 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M07.collapse | 折叠/展开文件夹 | 侧栏箭头 → 可见行控制 | 双端 | 子源显隐，状态保留 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M07.rename | 重命名文件夹 | 右键 → 行更多/右键 | 双端 | 保存/取消并回读 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M07.delete | 删除文件夹 | 右键 → 更多危险组 | 双端 | 说明源保留；取消/确认查库并检查焦点去向 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M08.view | 标签视图、颜色、未读数 | 标签侧栏 → 可见标签行 | 双端 | 筛选、计数、颜色；打开 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M08.collapse | 折叠标签区 | 标签标题 → 可见控制 | 双端 | 切换并重启 | T2 | 通过：[T2 设备与重启证据](evidence/t2-menu/record.md) |
| M08.rename | 重命名标签 | 右键 → 行更多/右键 | 双端 | 保存/取消并回读 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M08.color | 标签颜色/默认色 | 右键 → 面板内选择页/桌面级联 | 双端 | 当前值、恢复默认；回读 | T2/T6 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M08.pin | 置顶/取消置顶 | 右键 → 行更多/右键 | 双端 | 顺序变化；切换并回读 | T2 | 通过：[T2 设备与重启证据](evidence/t2-menu/record.md) |
| M08.sort | 标签上移/下移、拖拽 | 拖拽/右键 → 更多排序/桌面拖拽 | 双端 | 无触屏拖拽依赖；移动并回读 | T2/T6 | 手机菜单排序沿用 [T2](evidence/t2-menu/record.md)；Linux 最终产物可信指针拖拽标签/源并回读排序；[T7 Linux 操作](evidence/t7-final/linux-actions-round2/results.json) |
| M08.delete | 删除标签 | 右键 → 更多危险组 | 双端 | 影响数量与确认；取消/确认及焦点去向 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M09.search | 搜索标签 | 标签选择器/t → 阅读更多/选择器 | 双端 | 键入中英；结果筛选 | T2/T4 | 通过：[T2 中英搜索与选择证据](evidence/t2-menu/record.md)、[T4 Android](evidence/t4-reader/record.md) |
| M09.create | 新建标签 | 标签选择器 → 阅读更多/选择器 | 双端 | 新建并附加；回读 | T2/T4 | Linux 最终产物新建标签落库；[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M09.attach | 附加标签 | 标签选择器 → 阅读更多/选择器 | 双端 | 文章标签关系；回读 | T2/T4 | Linux 最终产物标签关系回读；[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M09.remove | 移除标签 | 标签 chip → chip/更多 | 双端 | 关系删除，标签仍在 | T2/T4 | 通过：[T2 证据](evidence/t2-menu/record.md)、[T4 Android](evidence/t4-reader/record.md) |
| M09.open | 跳转标签视图 | 标签 chip → 原位 | 双端 | 跳转并可返回 | T2/T4 | 通过：[T2 证据](evidence/t2-menu/record.md)、[T4 Android](evidence/t4-reader/record.md) |
| M10.aa | Aa 阅读设置 | 阅读工具条 → 阅读页头 | 双端 | 字号/行高草稿；打开 | T4 | Android 最终产物阅读 Aa 正文/代码字体保存、锚点不移与冷重启通过；Linux 最终 Aa 组合未单独复测；[T7 Android 阅读副作用](evidence/t7-final/android-reading-effects-round2/reading-effects.json) |
| M10.read | 已读/未读 | 阅读工具条/u → 阅读更多/u | 双端 | 改状态；回读 | T4/T6 | Android 主路径通过：[T4](evidence/t4-reader/record.md)；桌面可信 u 后 SQLite 状态翻转：[T6 第二轮](evidence/t6-convergence/review-desktop-results.json) |
| M10.star | 星标/取消 | 阅读工具条/s → 阅读底部/s | 双端 | 星标独立；回读 | T4/T6 | 桌面同进程与可信 s 后 SQLite 翻转通过；[T6 第二轮](evidence/t6-convergence/review-desktop-results.json) |
| M10.later | 稍后读/取消 | 阅读工具条 → 阅读底部 | 双端 | 待读独立；回读 | T4 | 桌面同进程与可信 l 后 SQLite 翻转通过；[T6 第二轮](evidence/t6-convergence/review-desktop-results.json) |
| M10.open | 浏览器打开 | 阅读工具条 → 阅读更多 | 双端 | 外链交给系统；点击 | T4 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M10.copy | 复制链接 | 阅读工具条 → 阅读更多 | 双端 | 剪贴板含原 URL；点击 | T4 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M10.share | 原生分享 | 阅读工具条 → 阅读更多 | Android | 系统分享表；点击/取消 | T4 | Android 原生通过：[T4](evidence/t4-reader/record.md) |
| M10.fulltext | 按需全文 | 阅读工具条 → 阅读更多 | 双端 | 不提前请求；触发/失败/重试 | T4 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M11.summary | AI 摘要 | 阅读工具条 → 更多 AI 组 | 双端 | 经原确认；取消不请求 | T4 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M11.translate | AI 翻译 | 阅读工具条 → 更多 AI 组 | 双端 | 经原确认；取消不请求 | T4 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M11.result | 结果类型与元信息 | AI 面板 → 原位 | 双端 | 摘要/翻译来源可辨；打开 | T4 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M11.retry | 重新生成 | AI 面板按钮 → 原位 | 双端 | 再确认/替换结果；点击 | T4 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M11.close | 关闭结果 | AI 面板按钮 → 原位 | 双端 | 不影响正文；关闭 | T4 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M11.error | AI 成功/失败/取消 | AI 面板 → 稳定反馈位 | 双端 | 三态明确；模拟响应 | T4/T5 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M12.target | 发送目标地址 | AI 确认 → 原确认 | 双端 | 地址真实；查看 | T4/T5 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M12.headers | 脱敏 headers | AI 确认 → 原确认 | 双端 | 无密钥明文；查看 | T4/T5 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M12.body | 正文与大小 | AI 确认 → 原确认 | 双端 | 范围正确；查看 | T4/T5 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M12.cancel | 取消发送 | AI 确认 → 原确认 | 双端 | 零网络；取消并查日志 | T4/T5 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M12.send | 确认发送 | AI 确认 → 原确认 | 双端 | 仅确认后请求；执行 | T4/T5 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M12.remember | 不再询问 | AI 确认 → 原确认 | 双端 | 保持既有设置语义；切换 | T4/T5 | Android 通过：[T4](evidence/t4-reader/record.md)；Linux 最终组合未单独复测 |
| M13.mode | 系统/浅/深 | 外观设置 → 外观常用 | 双端 | 三态、系统切换无写；切换 | T1/T5 | 通过：系统/浅/深逐项保存回读；[Android 手工读回](evidence/t5-settings/android-manual-results.json)；最终 Linux 视觉/主题读回 [T7 Linux 12 格视觉读回](evidence/t7-final/visual-after/visual-results.json) |
| M13.presets | 浅深 Clear/Paper/Slate | 外观设置 → 外观常用 | 双端 | 独立预设，旧 ID 不变；六格查看 | T1/T5 | 通过：浅深各三预设逐项回读；[Android 手工读回](evidence/t5-settings/android-manual-results.json)；最终 Linux 视觉/主题读回 [T7 Linux 12 格视觉读回](evidence/t7-final/visual-after/visual-results.json) |
| M13.override.clear | 清除全部覆盖 | 外观设置 → 外观高级 | 双端 | 预设值恢复；取消/保存回读 | T1/T5 | 通过：取消零写、保存覆盖树为空；[Android 手工读回](evidence/t5-settings/android-manual-results.json)；Linux 最终效果未单独复测 |
| M13.override.inherit | 单项继承 | 外观设置 → 外观高级 | 双端 | 其它覆盖不丢；保存回读 | T1/T5 | 通过：仅密度继承，摘要与颜色覆盖保留；[Android 手工读回](evidence/t5-settings/android-manual-results.json)；Linux 最终效果未单独复测 |
| M13.override.color | 颜色覆盖 | 外观设置 → 外观高级 | 双端 | 仅编辑目标色；预览/保存 | T1/T5 | 通过：浅色强调色预览/保存回读；[Android 手工读回](evidence/t5-settings/android-manual-results.json)；最终 Linux 视觉/主题读回 [T7 Linux 12 格视觉读回](evidence/t7-final/visual-after/visual-results.json) |
| M13.type.family | UI 字体 | 外观设置 → 外观常用/高级 | 双端 | 用户字体优先；设置并重启 | T1/T5 | Android 最终 APK UI serif 保存并冷重启，实际标题 computed font=serif；[T7 Android 外观副作用](evidence/t7-final/android-appearance-effects-round2/appearance-effects.json)；Linux 最终效果未单独复测 |
| M13.type.size | UI 字号 | 外观设置 → 外观常用/高级 | 双端 | 旧值不重置；设置并重启 | T1/T5 | 通过：字号 20 保存，Activity 重建后放大截图；[Android 手工读回](evidence/t5-settings/android-manual-results.json)；Linux 最终效果未单独复测 |
| M13.list.density | 列表密度 | 外观设置 → 外观常用 | 双端 | 行高变化；保存并重启 | T1/T5 | Android 最终 APK comfortable→compact 行高 167.55→147.95 px、内边距 9→5 且重启保留；[T7 Android 外观副作用](evidence/t7-final/android-appearance-effects-round2/appearance-effects.json)；Linux 最终效果未单独复测 |
| M13.list.summary | 摘要行数 | 外观设置 → 外观常用 | 双端 | 摘要可关；保存并重启 | T1/T5 | 通过：摘要 0 行保存回读；[Android 手工读回](evidence/t5-settings/android-manual-results.json)；Linux 最终效果未单独复测 |
| M13.list.thumbnail | 缩略图开关 | 外观设置 → 外观常用 | 双端 | 不额外抓图；保存并重启 | T1/T5 | Android 最终 APK本地图片服务器：关闭后冷重启请求 0，开启后冷重启请求 1；[T7 Android 外观副作用](evidence/t7-final/android-appearance-effects-round2/appearance-effects.json)；Linux 最终效果未单独复测 |
| M13.list.radius | 圆角 | 外观设置 → 外观高级 | 双端 | 旧值保留；保存并重启 | T1/T5 | 通过：圆角 6 保存回读；[Android 手工读回](evidence/t5-settings/android-manual-results.json)；Linux 最终效果未单独复测 |
| M13.preview.start | 草稿预览 | 外观设置 → 原草稿链路 | 双端 | 临时零持久化；打开 | T1/T5 | 最终产物 Android 预览零写、Linux 同进程预览零写；[T7 Android 三旅程](evidence/t7-final/android-journeys-round2/android-journeys.json)、[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M13.preview.save | 保存主题 | 外观设置 → 原草稿链路 | 双端 | CAS 修订；保存回读 | T1/T5 | 最终产物 Android/Linux 保存修订回读，Android stale CAS 拒写；[T7 Android 三旅程](evidence/t7-final/android-journeys-round2/android-journeys.json)、[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json)、[T7 Android 设置回归](evidence/t7-final/android-settings-followup-round2/android-followup-results.json) |
| M13.preview.cancel | 丢弃草稿 | 外观设置 → 原草稿链路 | 双端 | 零写、恢复原样；取消 | T1/T5 | 最终产物 Android/Linux 丢弃草稿零写；[T7 Android 三旅程](evidence/t7-final/android-journeys-round2/android-journeys.json)、[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M13.history.refresh | 刷新历史 | 外观设置 → 外观高级 | 双端 | 修订列表正确；打开 | T1/T5 | 最终 Android APK 历史列表刷新含旧修订；[T7 Android 设置回归](evidence/t7-final/android-settings-followup-round2/android-followup-results.json)；Linux 最终效果未单独复测 |
| M13.history.restore | 恢复历史 | 外观设置 → 外观高级 | 双端 | 新修订、CAS；执行回读 | T1/T5 | 最终 Android APK 恢复新修订、陈旧 CAS 拒写；[T7 Android 设置回归](evidence/t7-final/android-settings-followup-round2/android-followup-results.json)；Linux 最终效果未单独复测 |
| M14.type.family | 正文字体 | 阅读设置/Aa → 阅读常用 | 双端 | 正文锚点；调节并回读 | T4/T5 | Android 最终 APK长文 Aa 保存 serif，正文锚点差 0px，重启 computed style 保留；[T7 Android 阅读副作用](evidence/t7-final/android-reading-effects-round2/reading-effects.json)；Linux 最终效果未单独复测 |
| M14.type.size | 正文字号 | 阅读设置/Aa → 阅读常用 | 双端 | 正文锚点；调节并回读 | T4/T5 | 通过：正文字号保存回读；[Android 补充](evidence/t5-settings/android-followup-results.json)；Linux 最终效果未单独复测 |
| M14.type.line | 正文行高 | 阅读设置/Aa → 阅读常用 | 双端 | 正文锚点；调节并回读 | T4/T5 | 通过：行高保存回读；[Android 补充](evidence/t5-settings/android-followup-results.json)；Linux 最终效果未单独复测 |
| M14.type.paragraph | 段落间距 | 阅读设置/Aa → 阅读高级 | 双端 | 正文锚点；调节并回读 | T4/T5 | 通过：段距保存回读；[Android 补充](evidence/t5-settings/android-followup-results.json)；Linux 最终效果未单独复测 |
| M14.code.family | 等宽字体 | 阅读设置 → 阅读高级 | 双端 | 代码块；调节并查看 | T4/T5 | Android 最终 APK真实 code 块 computed monospace 保存及重启保留；[T7 Android 阅读副作用](evidence/t7-final/android-reading-effects-round2/reading-effects.json)；Linux 最终效果未单独复测 |
| M14.code.size | 等宽字号 | 阅读设置 → 阅读高级 | 双端 | 代码块；调节并查看 | T4/T5 | Android 最终 APK真实 code 块 computed 20px 保存及重启保留；[T7 Android 阅读副作用](evidence/t7-final/android-reading-effects-round2/reading-effects.json)；Linux 最终效果未单独复测 |
| M14.mark-read | 导航时标记已读 | 阅读设置 → 阅读行为 | 双端 | 按原保存语义；切换并读文章 | T4/T5 | Android 最终 APK可信 WebView j 导航同一文章：关闭 read=false、开启 read=true，并查 get_entry；[T7 Android 阅读副作用](evidence/t7-final/android-reading-effects-round2/reading-effects.json)；Linux 最终效果未单独复测 |
| M14.width | 阅读宽度 | 阅读设置 → 阅读高级 | 桌面 | 布局存储；调节 | T5/T6 | Linux 最终二进制同进程 760px 保存及正文锚点保留；[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M14.layout | 三栏/聚焦阅读 | 阅读设置 → 阅读高级 | 桌面 | 布局不丢；切换 | T5/T6 | Linux 最终二进制聚焦阅读布局保留正文节点与选中行；[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M15.proxy | 环境/直连/自定义代理 | 订阅设置 → 连接组 | 双端 | 保持原保存/错误；切换 | T5 | 通过：自定义与直连保存回读；[Android 原生副作用](evidence/t5-settings/android-required-results.json) |
| M15.proxy-url | 代理 URL | 订阅设置 → 连接组 | 双端 | 输入不丢、校验错误；填写 | T5 | 通过：非法 URL 输入保留、字段错误，数据库未写；[Android](evidence/t5-settings/android-results.json)、[Android 手工读回](evidence/t5-settings/android-manual-results.json) |
| M15.proxy-bypass | 代理绕过列表 | 订阅设置 → 连接组 | 双端 | 输入不丢；填写并保存 | T5 | 通过：绕过列表保存回读；[Android 原生副作用](evidence/t5-settings/android-required-results.json) |
| M15.proxy-save | 保存代理 | 订阅设置 → 连接组 | 双端 | 保持原反馈；保存并回读 | T5 | 通过：失败零写后修正保存回读；[Android](evidence/t5-settings/android-results.json)、[Android 原生副作用](evidence/t5-settings/android-required-results.json) |
| M15.refresh.interval | 全局刷新间隔 | 订阅设置 → 更新组 | 双端 | 覆盖优先语义；设置并重启 | T5 | Android 最终 APK：预置 30 分钟前抓取时间，真实 60s 调度 tick 后全局 15m 源请求 1 次、单源覆盖 120m 源 0 次；[T7 Android M15 原生探针](evidence/t7-final/m15-android/results.json) |
| M15.refresh.concurrent | 刷新并发 | 订阅设置 → 更新组 | 双端 | 存储与上限；设置并重启 | T5 | Android 最终 APK：可信原生 Refresh all 点击，13 个本地端点最大同时请求数精确为 12；[T7 Android M15 原生探针](evidence/t7-final/m15-android/results.json) |
| M15.refresh.startup | 启动刷新 | 订阅设置 → 更新组 | 双端 | 旧默认与重启；切换 | T5 | Android 最终 APK：冷启动关闭 13s 零请求、开启在 am start 后 11.754s 请求 1 次；[T7 Android M15 原生探针](evidence/t7-final/m15-android/results.json) |
| M15.notify | 新文章通知 | 订阅设置 → 更新组 | 双端 | 平台能力；切换 | T5 | Android 最终 APK：后台恢复新增未读 0→1，系统通知及 shade 可见；手动刷新未读 1→2 无重复通知；[T7 Android M15 原生探针](evidence/t7-final/m15-android/results.json) |
| M15.rsshub.save | RSSHub 实例保存 | 订阅设置 → RSSHub 组 | 双端 | 地址回读；保存 | T5 | 通过：镜像 URL 保存回读；[Android 原生副作用](evidence/t5-settings/android-required-results.json) |
| M15.rsshub.test | RSSHub 实例测试 | 订阅设置 → RSSHub 组 | 双端 | 成功/失败反馈；测试 | T5 | Android 最终 APK：本地成功探针见 [设置回归](evidence/t7-final/android-settings-required-round2/android-required-results.json)；四个路由均 503 时原生 Test connection 显示失败；[T7 Android M15 原生探针](evidence/t7-final/m15-android/results.json) |
| M15.rsshub.migrate | RSSHub 实例迁移 | 订阅设置 → RSSHub 组 | 双端 | 原确认与范围；隔离库执行 | T5 | 通过：非冲突旧域候选预览 1、取消零写、确认归一为 rsshub:// 且 ID 不变、再次预览 0；[迁移回读](evidence/t5-settings/android-rsshub-migration.json) |
| M16.provider | Ollama/OpenAI/Anthropic/Gemini | AI 设置 → 供应商区 | 双端 | 四供应商可选；切换 | T5 | 通过：四供应商逐个保存回读；[Android 补充](evidence/t5-settings/android-followup-results.json) |
| M16.model | 模型选择 | AI 设置 → 供应商区 | 双端 | 当前值；设置并回读 | T5 | 通过：四供应商合成模型逐个保存回读；[Android 补充](evidence/t5-settings/android-followup-results.json) |
| M16.endpoint | 端点 | AI 设置 → 供应商区 | 双端 | Android 地址说明；设置 | T5 | 通过：Android 可达提示、端点保存回读；[Android 原生副作用](evidence/t5-settings/android-required-results.json) |
| M16.language | 目标语言 | AI 设置 → 供应商区 | 双端 | 设置并回读 | T5 | 通过：目标语言保存回读；[Android 原生副作用](evidence/t5-settings/android-required-results.json) |
| M16.max-tokens | 最大 tokens | AI 设置 → 高级区 | 双端 | 真实值可见可编辑；保存 | T5 | 通过：1024 保存回读；[Android 原生副作用](evidence/t5-settings/android-required-results.json) |
| M16.reasoning | reasoning effort | AI 设置 → 高级区 | 双端 | 真实值可见可编辑；保存 | T5 | 通过：high 保存回读；[Android 原生副作用](evidence/t5-settings/android-required-results.json) |
| M16.key-status | 密钥状态 | AI 设置 → 密钥区 | 双端 | 不显明文；查看/重启 | T5 | 部分：Android 最终 APK Keystore 状态回读；Linux 私有 Secret Service 在 CreateCollection 需交互解锁，写入被拒；[T7 Android 设置回归](evidence/t7-final/android-settings-followup-round2/android-followup-results.json)、[T7 Linux 操作](evidence/t7-final/linux-actions-round2/results.json) |
| M16.key-save | 保存密钥 | AI 设置 → 密钥区 | 双端 | 安全存储；保存/重启 | T5 | 部分：Android 最终 APK 合成密钥保存回读；Linux 私有 Secret Service 在 CreateCollection 需交互解锁，未通过；[T7 Android 设置回归](evidence/t7-final/android-settings-followup-round2/android-followup-results.json)、[T7 Linux 操作](evidence/t7-final/linux-actions-round2/results.json) |
| M16.key-clear | 清除密钥 | AI 设置 → 密钥区 | 双端 | 清除后不可用；执行/重启 | T5 | 部分：Android 最终 APK Keystore 清除回读；Linux 私有 Secret Service 在 CreateCollection 需交互解锁，未通过；[T7 Android 设置回归](evidence/t7-final/android-settings-followup-round2/android-followup-results.json)、[T7 Linux 操作](evidence/t7-final/linux-actions-round2/results.json) |
| M16.test | 测试连接 | AI 设置 → 供应商区 | 双端 | 成功/失败反馈；点击 | T5 | Android 最终 APK 本地成功；Linux 最终二进制隔离失败端点返回明确网络错误；[T7 Android 设置必验](evidence/t7-final/android-settings-required-round2/android-required-results.json)、[T7 Linux 操作](evidence/t7-final/linux-actions-round2/results.json) |
| M16.confirm | 发送确认开关 | AI 设置 → 高级区 | 双端 | 不改变默认；切换 | T5 | 通过：确认开关保存并恢复回读；[Android 原生副作用](evidence/t5-settings/android-required-results.json) |
| M17.path | 数据目录说明 | 数据设置 → 原位 | 双端 | 路径可见；打开 | T5 | 通过：Android/Linux 数据目录可见；[Android](evidence/t5-settings/android-results.json)、[Linux 原生](evidence/t5-settings/desktop-results.json) |
| M17.opml-import | OPML 导入 | 数据设置 → 原位 | 双端 | Android SAF/MIME；导入隔离文件 | T5 | 通过：Android SAF 导入合成源 ID 2；[Android SAF](evidence/t5-settings/android-saf-results.json) |
| M17.opml-export | OPML 导出 | 数据设置 → 原位 | 双端 | 仅订阅；导出并检查 | T5 | Linux 最终二进制同进程 GTK Save File 选择器导出 XML，含新源与文件夹；[T7 Linux 同进程 18 项](evidence/t7-final/desktop-journey-round2/journey-results.json) |
| M17.db-backup | 完整数据库备份 | 数据设置 → 原位 | 桌面 | 旧流程与路径；隔离库备份 | T5/T6 | 通过：Linux GTK 选目录生成 SQLite 快照、integrity_check ok；[Linux 原生](evidence/t5-settings/desktop-results.json)；最终二进制复核 [T7 Linux 设置 49 项](evidence/t7-final/desktop-settings-round2/desktop-results.json) |
| M17.db-restore | 完整数据库恢复 | 数据设置 → 原位 | 桌面 | 旧确认与回滚；隔离库恢复 | T5/T6 | 通过：Linux 取消零暂存、确认暂存、重启恢复及 .bak 回滚；[Linux 原生](evidence/t5-settings/desktop-results.json)；最终二进制复核 [T7 Linux 设置 49 项](evidence/t7-final/desktop-settings-round2/desktop-results.json) |
| M18.locale | 自动/中文/英文 | 通用设置 → 原位 | 双端 | 两套 key、切换即时；操作 | T5 | 通过：中文/英文即时切换与持久回读，浅深截图；[Android 原生副作用](evidence/t5-settings/android-required-results.json)、[Android 手工读回](evidence/t5-settings/android-manual-results.json) |
| M18.about | 关于 | 通用设置 → 原位 | 双端 | 内容可读；打开 | T5 | Android 最终 APK 原生关于面板内容可读，含协议、隐私与源码说明；[T7 Android 设置必验](evidence/t7-final/android-settings-required-round2/android-required-results.json) |
| M18.agpl | AGPL | 通用设置 → 原位 | 双端 | 协议可达；打开 | T5 | Android 最终 APK 关于面板内联 AGPL-3.0-or-later 文本可见；此项为内联内容，无独立系统外链；[T7 Android 设置必验](evidence/t7-final/android-settings-required-round2/android-required-results.json) |
| M18.privacy | 隐私 | 通用设置 → 原位 | 双端 | 文档可达；打开 | T5 | Android 最终 APK 关于面板内联隐私说明可见；此项为内联内容，无独立系统外链；[T7 Android 设置必验](evidence/t7-final/android-settings-required-round2/android-required-results.json) |
| M18.source | 源码 | 通用设置 → 原位 | 双端 | URL 正确；打开 | T5 | 部分：Android 源码入口可见；Linux 最终二进制隔离 xdg-open 收到正确 GitHub URL，图形浏览器未验证；[T7 Linux 操作](evidence/t7-final/linux-actions-round2/results.json) |
| M18.log-level | 日志级别 | 通用设置 → 原位 | 双端 | info/debug 即时生效；切换 | T5 | Linux 最终二进制 info→debug SQLite 回读，DEBUG 实际日志行仅在 debug 出现；Android 设置回读；[T7 Linux 操作](evidence/t7-final/linux-actions-round2/results.json)、[T7 Android 设置必验](evidence/t7-final/android-settings-required-round2/android-required-results.json) |
| M18.exit | 关闭/退出行为 | 通用设置 → 原位 | 桌面 | 保持平台行为；操作 | T5/T6 | Linux 最终二进制 exit 设置后窗口关闭进程退出码 0；[T7 Linux 操作](evidence/t7-final/linux-actions-round2/results.json) |
| M18.tray | 托盘行为 | 通用设置 → 原位 | 桌面 | 原切换语义；操作 | T5/T6 | Linux 最终二进制私有 SNI host 注册，close→窗口 unmap、托盘菜单恢复为 viewable；面板图标像素未测；[T7 Linux 操作](evidence/t7-final/linux-actions-round2/results.json) |
| M18.log-folder | 打开日志目录 | 通用设置 → 原位 | 桌面 | 系统打开；点击 | T5/T6 | Linux 最终二进制隔离 xdg-open 收到存在的日志目录；真实图形文件管理器未测；[T7 Linux 操作](evidence/t7-final/linux-actions-round2/results.json) |
| M19.enabled | MCP 开关 | MCP 设置 → 第七类 | 桌面 | 回环监听；切换 | T5/T6 | 通过：Linux 本地服务开关与回环监听；[Linux 原生](evidence/t5-settings/desktop-results.json)；最终二进制复核 [T7 Linux 设置 49 项](evidence/t7-final/desktop-settings-round2/desktop-results.json) |
| M19.port | MCP 端口 | MCP 设置 → 第七类 | 桌面 | 端口校验/回读；设置 | T5/T6 | 通过：UI 与 Rust IPC 无效端口拒写，合法端口回读；[Linux 原生](evidence/t5-settings/desktop-results.json)；最终二进制复核 [T7 Linux 设置 49 项](evidence/t7-final/desktop-settings-round2/desktop-results.json) |
| M19.status | MCP 状态 | MCP 设置 → 第七类 | 桌面 | 监听状态正确；查看 | T5/T6 | 通过：服务健康/状态回读；[Linux 原生](evidence/t5-settings/desktop-results.json)；最终二进制复核 [T7 Linux 设置 49 项](evidence/t7-final/desktop-settings-round2/desktop-results.json) |
| M19.snippet | 客户端片段复制 | MCP 设置 → 第七类 | 桌面 | 复制并检查内容；点击 | T5/T6 | 通过：配置片段剪贴板读回；[Linux 原生](evidence/t5-settings/desktop-results.json)；最终二进制复核 [T7 Linux 设置 49 项](evidence/t7-final/desktop-settings-round2/desktop-results.json) |
| M19.read-token | 读 token 轮换 | MCP 设置 → 第七类 | 桌面 | 旧值失效；隔离服务操作 | T5/T6 | 通过：读 token 轮换后旧值 HTTP 失效；[Linux 原生](evidence/t5-settings/desktop-results.json)；最终二进制复核 [T7 Linux 设置 49 项](evidence/t7-final/desktop-settings-round2/desktop-results.json) |
| M19.write-toggle | 写工具开关 | MCP 设置 → 第七类 | 桌面 | 默认关/分权；切换 | T5/T6 | 通过：写权限开关回读；[Linux 原生](evidence/t5-settings/desktop-results.json)；最终二进制复核 [T7 Linux 设置 49 项](evidence/t7-final/desktop-settings-round2/desktop-results.json) |
| M19.danger-toggle | 危险工具开关 | MCP 设置 → 第七类 | 桌面 | 默认关、确认；切换 | T5/T6 | 通过：危险权限取消零写、确认回读；[Linux 原生](evidence/t5-settings/desktop-results.json)；最终二进制复核 [T7 Linux 设置 49 项](evidence/t7-final/desktop-settings-round2/desktop-results.json) |
| M19.write-token.generate | 生成写 token | MCP 设置 → 第七类 | 桌面 | 生成后工具可用；隔离服务 | T5/T6 | 通过：合成写 token 生成与权限回读；[Linux 原生](evidence/t5-settings/desktop-results.json)；最终二进制复核 [T7 Linux 设置 49 项](evidence/t7-final/desktop-settings-round2/desktop-results.json) |
| M19.write-token.copy | 复制写 token | MCP 设置 → 第七类 | 桌面 | 剪贴板核验；隔离服务 | T5/T6 | 通过：写 token 剪贴板读回；[Linux 原生](evidence/t5-settings/desktop-results.json)；最终二进制复核 [T7 Linux 设置 49 项](evidence/t7-final/desktop-settings-round2/desktop-results.json) |
| M19.write-token.rotate | 轮换写 token | MCP 设置 → 第七类 | 桌面 | 旧值立即失效；隔离服务 | T5/T6 | 通过：轮换后旧 token 失效；[Linux 原生](evidence/t5-settings/desktop-results.json)；最终二进制复核 [T7 Linux 设置 49 项](evidence/t7-final/desktop-settings-round2/desktop-results.json) |
| M19.write-token.clear | 清除写 token | MCP 设置 → 第七类 | 桌面 | 写工具不可用；隔离服务 | T5/T6 | 通过：清除后写 token 不可用；[Linux 原生](evidence/t5-settings/desktop-results.json)；最终二进制复核 [T7 Linux 设置 49 项](evidence/t7-final/desktop-settings-round2/desktop-results.json) |
| M20.window.min | 窗口最小化 | 标题栏 → 原位 | 桌面 | 原生行为；点击 | T6 | Linux 主链通过：私有 Xvfb 承载嵌套 KWin，同一 Wayland 原生进程可信指针点击真实按钮，compositor minimized=false→true、画面消失、PID 守卫恢复 false/active=true；真实 KWin 两后端状态另证，任务栏 shell 点击未测：[T6 第三轮](evidence/t6-convergence/nested-window-results.json) |
| M20.window.max | 窗口最大化 | 标题栏 → 原位 | 桌面 | 原生行为；点击 | T6 | 通过：真实 Wayland 最大化/还原尺寸与状态读回；[T6 原生记录](evidence/t6-convergence/record.md) |
| M20.window.close | 窗口关闭 | 标题栏 → 原位 | 桌面 | 原生行为；点击 | T6 | 通过：真实 Wayland 标题栏关闭，进程退出码 0；[T6 原生记录](evidence/t6-convergence/record.md) |
| M20.shortcuts | 快捷键帮助 | ? → 原位 | 桌面 | 列表准确；打开/关闭 | T6 | 通过：原生 ? 打开帮助、Esc 关闭；[T6 原生记录](evidence/t6-convergence/record.md) |
| M20.confirm | 危险确认与输入 prompt | 弹层 → 统一视觉弹层 | 双端 | 取消零写、确认回读 | T2/T5 | 通过：T2 危险确认及 T5 MCP 取消零写、确认回读；[Linux 原生](evidence/t5-settings/desktop-results.json) |
| M20.legacy-db.refuse | 旧库拒绝 | 启动拒绝屏 → 统一反馈 | 双端 | 原库只读；隔离旧库 | T5/T6 | 通过：隔离 v13 库触发拒绝屏，库哈希不变且无 WAL；[Linux 旧库原生](evidence/t5-settings/legacy-refusal-runtime-results.json) |
| M20.legacy-db.export | 只读导出 OPML | 启动拒绝屏 → 统一反馈 | 双端 | 原库不变；导出并检查 | T5/T6 | 通过：GTK 原生保存选择器导出 302 字节 OPML 含旧源，原库字节不变；[Linux 旧库原生](evidence/t5-settings/legacy-refusal-runtime-results.json) |
| M20.legacy-db.quit | 退出 | 启动拒绝屏 → 统一反馈 | 双端 | 原库不变；点击 | T5/T6 | 通过：拒绝屏 Quit 退出应用，原库未写；[Linux 旧库原生](evidence/t5-settings/legacy-refusal-runtime-results.json) |
| M20.empty.feeds | 空订阅 | 空态 → 稳定空态 | 双端 | 添加入口可见；隔离 fixture | T5 | 最终 Android APK 空库原生可访问树/焦点及 Add feed 操作 11 项通过；[T7 Android 空态](evidence/t7-final/android-empty-round2/android-empty-results.json) |
| M20.empty.search | 空搜索 | 空态 → 稳定空态 | 双端 | 清除/重试可见；隔离 fixture | T5 | 最终 Android APK 无结果 Clear/Retry 文案、48px、原生焦点和动作 11 项通过；[T7 Android 空态](evidence/t7-final/android-empty-round2/android-empty-results.json) |
| M20.offline | 离线 | 状态区 → 稳定反馈位 | 双端 | 缓存可读；断网打开 | T5 | 最终 Android APK 本地源断服后 connection_error，缓存列表和正文仍可读；[T7 Android 离线重试](evidence/t7-final/android-offline-round2/offline-results.json) |
| M20.failure | 请求失败 | 状态区 → 稳定反馈位 | 双端 | 反馈不跳布局；模拟失败 | T5 | 最终 Android APK 本地源断服 connection_error，缓存正文与无溢出；代理字段错误另见 [T5](evidence/t5-settings/android-results.json)；[T7 Android 离线重试](evidence/t7-final/android-offline-round2/offline-results.json) |
| M20.retry | 重试 | 状态区 → 稳定反馈位 | 双端 | 输入保留；失败后重试 | T5 | 最终 Android APK 本地服务恢复后重试 fetched=1、failures=[]、状态 ok、缓存文章不重复；[T7 Android 离线重试](evidence/t7-final/android-offline-round2/offline-results.json) |

不适用判定按运行平台能力，不能仅按窄屏宽度；Android M19 及桌面专属项应隐藏，窄桌面仍应可用。旧入口基线取自 `ui/index.html`、`ui/app.js`、`ui/mobile.js` 在 `bc680d5155b7c6afcee355fe6aa9bc74ef456031` 的实现。动作副作用必须由执行后的 store/系统回读证明；打开菜单本身不得执行动作。

T2 的「通过」对应 [任务设备与桌面记录](evidence/t2-menu/record.md)中的执行路径，包含 Android 37 项及重启复核、Linux WebKitGTK 菜单键盘 11 项与隔离库回读。T7 已用最终 Android APK 和 Linux 二进制复核上述明确引用的组合路径；其余仅有前任务证据的行保留平台与范围说明。M08 桌面拖拽由最终 Linux 二进制可信指针和库排序读回验证。
