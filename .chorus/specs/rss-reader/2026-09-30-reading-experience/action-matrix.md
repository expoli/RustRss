# 阅读体验改版：逐动作迁移台账

来源：Chorus Proposal `b5585be7-a1d3-4311-8c1b-ce458ad649d6` 的《菜单盘点》M01–M20（OpenSpec slug `refine-rustrss-reading-experience`）。本表的 `ID` 是迁移核对 ID；T2 的运行时 action descriptor 使用此 ID 或记录映射。`新入口` 是批准后的目标，未完成前不代表已经实现。状态栏由责任任务回填 `待验 / 通过 + 证据路径 / 不适用 + 平台理由`，T7 逐行关闭。下面的操作均在隔离 fixture 中进行；危险项先取消，再经确认执行并回读。

| ID | 动作 | 旧入口 → 新入口 | 平台 | 状态或副作用；核验步骤 | 责任 | 证据 |
|---|---|---|---|---|---|---|
| M01.articles | 文章 | 侧栏/底栏 → 四主导航文章 | 双端 | 保持当前视图；切换并返回 | T3/T6 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M01.subscriptions | 订阅 | 侧栏/底栏 → 四主导航订阅 | 双端 | 保留选中源；切换并返回 | T2/T6 | Android 导航/激活通过、选中源与桌面组合待 T6：[T3 证据](evidence/t3-list/record.md) |
| M01.saved | 收藏 | 侧栏/底栏 → 四主导航收藏 | 双端 | 保留星标/稍后读子视图；切换 | T3/T6 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M01.settings | 设置 | 工具栏/底栏 → 四主导航设置 | 双端 | 草稿离开规则；进入再返回 | T5/T6 | Android 导航/Back/焦点通过，草稿规则待 T5、桌面组合待 T6：[T3 证据](evidence/t3-list/record.md) |
| M01.unread | 未读视图 | 智能视图/页内切换 → 标题旁筛选 | 双端 | 未读数同 core；切换 | T3 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M01.all | 全部视图 | 智能视图/页内切换 → 标题旁筛选 | 双端 | 已读仍可见；切换 | T3 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M01.starred | 星标视图 | 智能视图/页内切换 → 收藏内筛选 | 双端 | 仅星标；切换 | T3 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M01.later | 稍后读视图 | 智能视图/页内切换 → 收藏内筛选 | 双端 | 仅稍后读；切换 | T3 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M02.search | 标题及正文搜索 | 顶部输入 → 搜索按钮展开输入 | 双端 | 手机明确全库 scope；查长标题/正文 | T3 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M02.clear | 清除及取消搜索 | 输入清除/Esc → 展开区取消 | 双端 | 恢复前一筛选和锚点；搜索后取消 | T3 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M02.sort.newest | 最新排序 | 列表排序菜单 → 列表更多 | 双端 | 存储 list.sort；切换并重启 | T3 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M02.sort.oldest | 最旧排序 | 列表排序菜单 → 列表更多 | 双端 | 存储 list.sort；切换并重启 | T3 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M02.sort.unread | 未读优先 | 列表排序菜单 → 列表更多 | 双端 | 存储 list.sort；切换并重启 | T3 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M02.hide-read | 隐藏已读 | 列表排序菜单/U → 列表更多/U | 双端 | 存储 list.hide_read；切换 | T3/T6 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M02.bulk.read | 当前作用域全标已读 | 列表批量菜单/A → 列表更多/A | 双端 | 确认 scope；执行并查库 | T3/T6 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M02.bulk.unread | 当前作用域全标未读 | 列表批量菜单 → 列表更多 | 双端 | 确认 scope；执行并查库 | T3 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M02.pagination | 续页、加载数、总数 | 列表尾部/头部 → 原位 | 双端 | 30/200行，M/N 正确；续页无重复 | T3 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M02.retry | 续页重试 | 失败尾行 → 原位 | 双端 | 失败后重试保留筛选 | T3 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M03.refresh-all | 刷新全部 | 工具栏/r → 顶部显式刷新/r | 双端 | 单 flight；观察开始/结束/失败 | T3/T6 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M03.refresh-feed | 单源刷新 | 源右键 → 源更多/右键 | 双端 | 单 flight；对象 ID 不漂移 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M03.status | 刷新状态 | 固定状态栏 → 稳定状态位 | 双端 | 不跳布局；触发和失败 | T3 | Android 通过、桌面待 T6：[T3 证据](evidence/t3-list/record.md) |
| M04.url | URL 或网站发现 | 添加订阅输入 → 订阅页明确添加 | 双端 | 候选发现；输入测试 URL | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M04.candidate | 候选源选择 | 发现结果 → 输入页选择 | 双端 | 选择目标源后新增 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M04.rsshub | RSSHub 入口 | 添加订阅 → 输入页 | 双端 | 输入完整 `rsshub://t2/menu-fixture`；本地镜像命中并回读源 URL/ID/成功状态 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M04.result | 添加反馈 | 原状态位 → 稳定反馈位 | 双端 | 成功/失败，输入保留 | T2/T5 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.select | 选择源、未读数与错误 | 侧栏行 → 可见行与状态 | 双端 | 选中/错误/计数；切换源 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.move-up | 源上移 | 右键 → 更多/右键整理组 | 双端 | 排序落库；执行并重启 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.move-down | 源下移 | 右键 → 更多/右键整理组 | 双端 | 排序落库；执行并重启 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.refresh | 立即刷新源 | 右键 → 更多/右键更新组 | 双端 | 单源刷新；观察结果 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.edit | 编辑源 | 右键 → 更多/右键 | 双端 | 进入原表单；取消/保存 | T2/T5 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.interval | 源刷新间隔 | 级联菜单 → 面板内选择页/桌面级联 | 双端 | 当前值、覆盖优先；切换并回读 | T2/T6 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.folder | 移动源至文件夹/未分组 | 级联菜单 → 面板内选择页/桌面级联 | 双端 | 当前值、对象 ID；移动并回读 | T2/T6 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M05.unsubscribe | 取消订阅 | 右键 → 更多危险组 | 双端 | 显示影响、取消无写且焦点回触发项；确认级联且焦点回存活行/标题 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M06.name | 源自定义名称 | 编辑对话框 → 对象编辑页 | 双端 | 保存/取消并回读 | T2/T5 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M06.folder | 编辑源文件夹 | 编辑对话框 → 对象编辑页 | 双端 | 未分组也可选；回读 | T2/T5 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M06.interval | 编辑源刷新间隔 | 编辑对话框 → 对象编辑页 | 双端 | 继承/覆盖；回读 | T2/T5 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M06.url | 只读源 URL | 编辑对话框 → 对象编辑页 | 双端 | URL 不可被编辑；查看 | T2/T5 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M07.create | 新建文件夹 | 侧栏加号 → 订阅页添加 | 双端 | 新建并回读 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M07.aggregate | 文件夹聚合列表 | 侧栏选择 → 文件夹行选择 | 双端 | 作用域计数；打开 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M07.collapse | 折叠/展开文件夹 | 侧栏箭头 → 可见行控制 | 双端 | 子源显隐，状态保留 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M07.rename | 重命名文件夹 | 右键 → 行更多/右键 | 双端 | 保存/取消并回读 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M07.delete | 删除文件夹 | 右键 → 更多危险组 | 双端 | 说明源保留；取消/确认查库并检查焦点去向 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M08.view | 标签视图、颜色、未读数 | 标签侧栏 → 可见标签行 | 双端 | 筛选、计数、颜色；打开 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M08.collapse | 折叠标签区 | 标签标题 → 可见控制 | 双端 | 切换并重启 | T2 | 通过：[T2 设备与重启证据](evidence/t2-menu/record.md) |
| M08.rename | 重命名标签 | 右键 → 行更多/右键 | 双端 | 保存/取消并回读 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M08.color | 标签颜色/默认色 | 右键 → 面板内选择页/桌面级联 | 双端 | 当前值、恢复默认；回读 | T2/T6 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M08.pin | 置顶/取消置顶 | 右键 → 行更多/右键 | 双端 | 顺序变化；切换并回读 | T2 | 通过：[T2 设备与重启证据](evidence/t2-menu/record.md) |
| M08.sort | 标签上移/下移、拖拽 | 拖拽/右键 → 更多排序/桌面拖拽 | 双端 | 无触屏拖拽依赖；移动并回读 | T2/T6 | 待验（T2 手机更多菜单上移/下移已通过；既有桌面拖拽的最终组合复核归 T6；[T2 证据](evidence/t2-menu/record.md)） |
| M08.delete | 删除标签 | 右键 → 更多危险组 | 双端 | 影响数量与确认；取消/确认及焦点去向 | T2 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M09.search | 搜索标签 | 标签选择器/t → 阅读更多/选择器 | 双端 | 键入中英；结果筛选 | T2/T4 | 通过：[T2 中英搜索与选择证据](evidence/t2-menu/record.md) |
| M09.create | 新建标签 | 标签选择器 → 阅读更多/选择器 | 双端 | 新建并附加；回读 | T2/T4 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M09.attach | 附加标签 | 标签选择器 → 阅读更多/选择器 | 双端 | 文章标签关系；回读 | T2/T4 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M09.remove | 移除标签 | 标签 chip → chip/更多 | 双端 | 关系删除，标签仍在 | T2/T4 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M09.open | 跳转标签视图 | 标签 chip → 原位 | 双端 | 跳转并可返回 | T2/T4 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M10.aa | Aa 阅读设置 | 阅读工具条 → 阅读页头 | 双端 | 字号/行高草稿；打开 | T4 | 待验 |
| M10.read | 已读/未读 | 阅读工具条/u → 阅读更多/u | 双端 | 改状态；回读 | T4/T6 | 待验 |
| M10.star | 星标/取消 | 阅读工具条/s → 阅读底部/s | 双端 | 星标独立；回读 | T4/T6 | 待验 |
| M10.later | 稍后读/取消 | 阅读工具条 → 阅读底部 | 双端 | 待读独立；回读 | T4 | 待验 |
| M10.open | 浏览器打开 | 阅读工具条 → 阅读更多 | 双端 | 外链交给系统；点击 | T4 | 待验 |
| M10.copy | 复制链接 | 阅读工具条 → 阅读更多 | 双端 | 剪贴板含原 URL；点击 | T4 | 待验 |
| M10.share | 原生分享 | 阅读工具条 → 阅读更多 | Android | 系统分享表；点击/取消 | T4 | 待验 |
| M10.fulltext | 按需全文 | 阅读工具条 → 阅读更多 | 双端 | 不提前请求；触发/失败/重试 | T4 | 待验 |
| M11.summary | AI 摘要 | 阅读工具条 → 更多 AI 组 | 双端 | 经原确认；取消不请求 | T4 | 待验 |
| M11.translate | AI 翻译 | 阅读工具条 → 更多 AI 组 | 双端 | 经原确认；取消不请求 | T4 | 待验 |
| M11.result | 结果类型与元信息 | AI 面板 → 原位 | 双端 | 摘要/翻译来源可辨；打开 | T4 | 待验 |
| M11.retry | 重新生成 | AI 面板按钮 → 原位 | 双端 | 再确认/替换结果；点击 | T4 | 待验 |
| M11.close | 关闭结果 | AI 面板按钮 → 原位 | 双端 | 不影响正文；关闭 | T4 | 待验 |
| M11.error | AI 成功/失败/取消 | AI 面板 → 稳定反馈位 | 双端 | 三态明确；模拟响应 | T4/T5 | 待验 |
| M12.target | 发送目标地址 | AI 确认 → 原确认 | 双端 | 地址真实；查看 | T4/T5 | 待验 |
| M12.headers | 脱敏 headers | AI 确认 → 原确认 | 双端 | 无密钥明文；查看 | T4/T5 | 待验 |
| M12.body | 正文与大小 | AI 确认 → 原确认 | 双端 | 范围正确；查看 | T4/T5 | 待验 |
| M12.cancel | 取消发送 | AI 确认 → 原确认 | 双端 | 零网络；取消并查日志 | T4/T5 | 待验 |
| M12.send | 确认发送 | AI 确认 → 原确认 | 双端 | 仅确认后请求；执行 | T4/T5 | 待验 |
| M12.remember | 不再询问 | AI 确认 → 原确认 | 双端 | 保持既有设置语义；切换 | T4/T5 | 待验 |
| M13.mode | 系统/浅/深 | 外观设置 → 外观常用 | 双端 | 三态、系统切换无写；切换 | T1/T5 | 待验 |
| M13.presets | 浅深 Clear/Paper/Slate | 外观设置 → 外观常用 | 双端 | 独立预设，旧 ID 不变；六格查看 | T1/T5 | 待验 |
| M13.override.clear | 清除全部覆盖 | 外观设置 → 外观高级 | 双端 | 预设值恢复；取消/保存回读 | T1/T5 | 待验 |
| M13.override.inherit | 单项继承 | 外观设置 → 外观高级 | 双端 | 其它覆盖不丢；保存回读 | T1/T5 | 待验 |
| M13.override.color | 颜色覆盖 | 外观设置 → 外观高级 | 双端 | 仅编辑目标色；预览/保存 | T1/T5 | 待验 |
| M13.type.family | UI 字体 | 外观设置 → 外观常用/高级 | 双端 | 用户字体优先；设置并重启 | T1/T5 | 待验 |
| M13.type.size | UI 字号 | 外观设置 → 外观常用/高级 | 双端 | 旧值不重置；设置并重启 | T1/T5 | 待验 |
| M13.list.density | 列表密度 | 外观设置 → 外观常用 | 双端 | 行高变化；保存并重启 | T1/T5 | 待验 |
| M13.list.summary | 摘要行数 | 外观设置 → 外观常用 | 双端 | 摘要可关；保存并重启 | T1/T5 | 待验 |
| M13.list.thumbnail | 缩略图开关 | 外观设置 → 外观常用 | 双端 | 不额外抓图；保存并重启 | T1/T5 | 待验 |
| M13.list.radius | 圆角 | 外观设置 → 外观高级 | 双端 | 旧值保留；保存并重启 | T1/T5 | 待验 |
| M13.preview.start | 草稿预览 | 外观设置 → 原草稿链路 | 双端 | 临时零持久化；打开 | T1/T5 | 待验 |
| M13.preview.save | 保存主题 | 外观设置 → 原草稿链路 | 双端 | CAS 修订；保存回读 | T1/T5 | 待验 |
| M13.preview.cancel | 丢弃草稿 | 外观设置 → 原草稿链路 | 双端 | 零写、恢复原样；取消 | T1/T5 | 待验 |
| M13.history.refresh | 刷新历史 | 外观设置 → 外观高级 | 双端 | 修订列表正确；打开 | T1/T5 | 待验 |
| M13.history.restore | 恢复历史 | 外观设置 → 外观高级 | 双端 | 新修订、CAS；执行回读 | T1/T5 | 待验 |
| M14.type.family | 正文字体 | 阅读设置/Aa → 阅读常用 | 双端 | 正文锚点；调节并回读 | T4/T5 | 待验 |
| M14.type.size | 正文字号 | 阅读设置/Aa → 阅读常用 | 双端 | 正文锚点；调节并回读 | T4/T5 | 待验 |
| M14.type.line | 正文行高 | 阅读设置/Aa → 阅读常用 | 双端 | 正文锚点；调节并回读 | T4/T5 | 待验 |
| M14.type.paragraph | 段落间距 | 阅读设置/Aa → 阅读高级 | 双端 | 正文锚点；调节并回读 | T4/T5 | 待验 |
| M14.code.family | 等宽字体 | 阅读设置 → 阅读高级 | 双端 | 代码块；调节并查看 | T4/T5 | 待验 |
| M14.code.size | 等宽字号 | 阅读设置 → 阅读高级 | 双端 | 代码块；调节并查看 | T4/T5 | 待验 |
| M14.mark-read | 导航时标记已读 | 阅读设置 → 阅读行为 | 双端 | 按原保存语义；切换并读文章 | T4/T5 | 待验 |
| M14.width | 阅读宽度 | 阅读设置 → 阅读高级 | 桌面 | 布局存储；调节 | T5/T6 | 待验 |
| M14.layout | 三栏/聚焦阅读 | 阅读设置 → 阅读高级 | 桌面 | 布局不丢；切换 | T5/T6 | 待验 |
| M15.proxy | 环境/直连/自定义代理 | 订阅设置 → 连接组 | 双端 | 保持原保存/错误；切换 | T5 | 待验 |
| M15.proxy-url | 代理 URL | 订阅设置 → 连接组 | 双端 | 输入不丢、校验错误；填写 | T5 | 待验 |
| M15.proxy-bypass | 代理绕过列表 | 订阅设置 → 连接组 | 双端 | 输入不丢；填写并保存 | T5 | 待验 |
| M15.proxy-save | 保存代理 | 订阅设置 → 连接组 | 双端 | 保持原反馈；保存并回读 | T5 | 待验 |
| M15.refresh.interval | 全局刷新间隔 | 订阅设置 → 更新组 | 双端 | 覆盖优先语义；设置并重启 | T5 | 待验 |
| M15.refresh.concurrent | 刷新并发 | 订阅设置 → 更新组 | 双端 | 存储与上限；设置并重启 | T5 | 待验 |
| M15.refresh.startup | 启动刷新 | 订阅设置 → 更新组 | 双端 | 旧默认与重启；切换 | T5 | 待验 |
| M15.notify | 新文章通知 | 订阅设置 → 更新组 | 双端 | 平台能力；切换 | T5 | 待验 |
| M15.rsshub.save | RSSHub 实例保存 | 订阅设置 → RSSHub 组 | 双端 | 地址回读；保存 | T5 | 待验 |
| M15.rsshub.test | RSSHub 实例测试 | 订阅设置 → RSSHub 组 | 双端 | 成功/失败反馈；测试 | T5 | 待验 |
| M15.rsshub.migrate | RSSHub 实例迁移 | 订阅设置 → RSSHub 组 | 双端 | 原确认与范围；隔离库执行 | T5 | 待验 |
| M16.provider | Ollama/OpenAI/Anthropic/Gemini | AI 设置 → 供应商区 | 双端 | 四供应商可选；切换 | T5 | 待验 |
| M16.model | 模型选择 | AI 设置 → 供应商区 | 双端 | 当前值；设置并回读 | T5 | 待验 |
| M16.endpoint | 端点 | AI 设置 → 供应商区 | 双端 | Android 地址说明；设置 | T5 | 待验 |
| M16.language | 目标语言 | AI 设置 → 供应商区 | 双端 | 设置并回读 | T5 | 待验 |
| M16.max-tokens | 最大 tokens | AI 设置 → 高级区 | 双端 | 真实值可见可编辑；保存 | T5 | 待验 |
| M16.reasoning | reasoning effort | AI 设置 → 高级区 | 双端 | 真实值可见可编辑；保存 | T5 | 待验 |
| M16.key-status | 密钥状态 | AI 设置 → 密钥区 | 双端 | 不显明文；查看/重启 | T5 | 待验 |
| M16.key-save | 保存密钥 | AI 设置 → 密钥区 | 双端 | 安全存储；保存/重启 | T5 | 待验 |
| M16.key-clear | 清除密钥 | AI 设置 → 密钥区 | 双端 | 清除后不可用；执行/重启 | T5 | 待验 |
| M16.test | 测试连接 | AI 设置 → 供应商区 | 双端 | 成功/失败反馈；点击 | T5 | 待验 |
| M16.confirm | 发送确认开关 | AI 设置 → 高级区 | 双端 | 不改变默认；切换 | T5 | 待验 |
| M17.path | 数据目录说明 | 数据设置 → 原位 | 双端 | 路径可见；打开 | T5 | 待验 |
| M17.opml-import | OPML 导入 | 数据设置 → 原位 | 双端 | Android SAF/MIME；导入隔离文件 | T5 | 待验 |
| M17.opml-export | OPML 导出 | 数据设置 → 原位 | 双端 | 仅订阅；导出并检查 | T5 | 待验 |
| M17.db-backup | 完整数据库备份 | 数据设置 → 原位 | 桌面 | 旧流程与路径；隔离库备份 | T5/T6 | 待验 |
| M17.db-restore | 完整数据库恢复 | 数据设置 → 原位 | 桌面 | 旧确认与回滚；隔离库恢复 | T5/T6 | 待验 |
| M18.locale | 自动/中文/英文 | 通用设置 → 原位 | 双端 | 两套 key、切换即时；操作 | T5 | 待验 |
| M18.about | 关于 | 通用设置 → 原位 | 双端 | 内容可读；打开 | T5 | 待验 |
| M18.agpl | AGPL | 通用设置 → 原位 | 双端 | 协议可达；打开 | T5 | 待验 |
| M18.privacy | 隐私 | 通用设置 → 原位 | 双端 | 文档可达；打开 | T5 | 待验 |
| M18.source | 源码 | 通用设置 → 原位 | 双端 | URL 正确；打开 | T5 | 待验 |
| M18.log-level | 日志级别 | 通用设置 → 原位 | 双端 | info/debug 即时生效；切换 | T5 | 待验 |
| M18.exit | 关闭/退出行为 | 通用设置 → 原位 | 桌面 | 保持平台行为；操作 | T5/T6 | 待验 |
| M18.tray | 托盘行为 | 通用设置 → 原位 | 桌面 | 原切换语义；操作 | T5/T6 | 待验 |
| M18.log-folder | 打开日志目录 | 通用设置 → 原位 | 桌面 | 系统打开；点击 | T5/T6 | 待验 |
| M19.enabled | MCP 开关 | MCP 设置 → 第七类 | 桌面 | 回环监听；切换 | T5/T6 | 待验 |
| M19.port | MCP 端口 | MCP 设置 → 第七类 | 桌面 | 端口校验/回读；设置 | T5/T6 | 待验 |
| M19.status | MCP 状态 | MCP 设置 → 第七类 | 桌面 | 监听状态正确；查看 | T5/T6 | 待验 |
| M19.snippet | 客户端片段复制 | MCP 设置 → 第七类 | 桌面 | 复制并检查内容；点击 | T5/T6 | 待验 |
| M19.read-token | 读 token 轮换 | MCP 设置 → 第七类 | 桌面 | 旧值失效；隔离服务操作 | T5/T6 | 待验 |
| M19.write-toggle | 写工具开关 | MCP 设置 → 第七类 | 桌面 | 默认关/分权；切换 | T5/T6 | 待验 |
| M19.danger-toggle | 危险工具开关 | MCP 设置 → 第七类 | 桌面 | 默认关、确认；切换 | T5/T6 | 待验 |
| M19.write-token.generate | 生成写 token | MCP 设置 → 第七类 | 桌面 | 生成后工具可用；隔离服务 | T5/T6 | 待验 |
| M19.write-token.copy | 复制写 token | MCP 设置 → 第七类 | 桌面 | 剪贴板核验；隔离服务 | T5/T6 | 待验 |
| M19.write-token.rotate | 轮换写 token | MCP 设置 → 第七类 | 桌面 | 旧值立即失效；隔离服务 | T5/T6 | 待验 |
| M19.write-token.clear | 清除写 token | MCP 设置 → 第七类 | 桌面 | 写工具不可用；隔离服务 | T5/T6 | 待验 |
| M20.window.min | 窗口最小化 | 标题栏 → 原位 | 桌面 | 原生行为；点击 | T6 | 待验 |
| M20.window.max | 窗口最大化 | 标题栏 → 原位 | 桌面 | 原生行为；点击 | T6 | 待验 |
| M20.window.close | 窗口关闭 | 标题栏 → 原位 | 桌面 | 原生行为；点击 | T6 | 待验 |
| M20.shortcuts | 快捷键帮助 | ? → 原位 | 桌面 | 列表准确；打开/关闭 | T6 | 待验 |
| M20.confirm | 危险确认与输入 prompt | 弹层 → 统一视觉弹层 | 双端 | 取消零写、确认回读 | T2/T5 | 通过：[T2 证据](evidence/t2-menu/record.md) |
| M20.legacy-db.refuse | 旧库拒绝 | 启动拒绝屏 → 统一反馈 | 双端 | 原库只读；隔离旧库 | T5/T6 | 待验 |
| M20.legacy-db.export | 只读导出 OPML | 启动拒绝屏 → 统一反馈 | 双端 | 原库不变；导出并检查 | T5/T6 | 待验 |
| M20.legacy-db.quit | 退出 | 启动拒绝屏 → 统一反馈 | 双端 | 原库不变；点击 | T5/T6 | 待验 |
| M20.empty.feeds | 空订阅 | 空态 → 稳定空态 | 双端 | 添加入口可见；隔离 fixture | T5 | 待验 |
| M20.empty.search | 空搜索 | 空态 → 稳定空态 | 双端 | 清除/重试可见；隔离 fixture | T5 | 待验 |
| M20.offline | 离线 | 状态区 → 稳定反馈位 | 双端 | 缓存可读；断网打开 | T5 | 待验 |
| M20.failure | 请求失败 | 状态区 → 稳定反馈位 | 双端 | 反馈不跳布局；模拟失败 | T5 | 待验 |
| M20.retry | 重试 | 状态区 → 稳定反馈位 | 双端 | 输入保留；失败后重试 | T5 | 待验 |

不适用判定按运行平台能力，不能仅按窄屏宽度；Android M19 及桌面专属项应隐藏，窄桌面仍应可用。旧入口基线取自 `ui/index.html`、`ui/app.js`、`ui/mobile.js` 在 `bc680d5155b7c6afcee355fe6aa9bc74ef456031` 的实现。动作副作用必须由执行后的 store/系统回读证明；打开菜单本身不得执行动作。

T2 的「通过」对应 [任务设备与桌面记录](evidence/t2-menu/record.md)中的执行路径，包含 Android 37 项及重启复核、Linux WebKitGTK 菜单键盘 11 项与隔离库回读；仍需 T7 对最终组合产物复核跨平台和重启保持。M08 桌面旧拖拽的最终组合复核归 T6，保留「待验」，没有用代码存在代替运行结果。
