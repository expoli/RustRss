// 界面文案（zh-CN / en）。
//
// 约定：
// - 两份字典的 key 必须完全一致，启动时会比对并把结果上报到 stdout（可无人值守核对）；
// - 这里是界面文案的**唯一**出处：其他文件不得写死面向用户的中文/英文；
// - 动态文案用 t('key', { n: 3 })，值里的 {n} 会被替换。

// 以普通 <script> 加载（不用 ES module）：避免依赖自定义协议下的模块加载行为。
//
// 整个文件包在 IIFE 里：普通脚本的顶层函数声明会变成 window 的属性，
// 而 app.js 里又有同名的顶层 const —— WebKit 会直接报
// “Can't create duplicate variable that shadows a global property” 并整份拒绝解析。
// 所以这里只让 window.I18N 成为全局，其余都关在闭包里。
(function () {

const DICTS = {
  'zh-CN': {
    'app.name': 'RustRss',
    'toolbar.refresh': '刷新全部',
    'toolbar.addFeed': '添加订阅',
    'toolbar.settings': '设置',
    'toolbar.refreshTitle': '刷新全部订阅（r）',
    'toolbar.addFeedTitle': '添加订阅',
    'toolbar.settingsTitle': '设置',
    'search.placeholder': '搜索标题与正文（按 / 聚焦）',

    'sidebar.feeds': '订阅源',
    'sidebar.feedCount': '{n} 个',
    'sidebar.addPlaceholder': 'https://example.com/ 或 feed 地址',
    'sidebar.add': '添加',
    'sidebar.feedTooltipOk': '{url}\n双击刷新此源',
    'sidebar.feedTooltipFailed': '上次抓取：{status}｜{error}\n双击可重试',

    'list.unread': '全部未读',
    'list.starred': '星标',
    'list.all': '全部',
    'list.count': '{n} 篇',
    'list.emptyUnread': '没有未读文章',
    'list.empty': '这里还没有文章',
    'list.searchTitle': '搜索：{q}',
    'list.feedFallback': '订阅源',

    'reader.empty': '从中间列表选一篇文章。',
    'reader.shortcuts': '快捷键：j/k 上下 · Enter 打开 · u 未读切换 · s 星标 · l 稍后读 · r 刷新 · / 搜索 · Esc 清除',
    'reader.markRead': '标为已读',
    'reader.markUnread': '标为未读',
    'reader.addStar': '加星标',
    'reader.removeStar': '取消星标',
    'reader.markLater': '稍后读',
    'reader.removeLater': '取消稍后读',
    'list.later': '稍后读',
    'menu.moveTo': '移动到',
    'menu.moveToUngrouped': '移出到未分组',
    'menu.rename': '重命名',
    'menu.delete': '删除',
    'folder.newTitle': '新建文件夹',
    'folder.renameTitle': '重命名文件夹',
    'prompt.ok': '确定',
    'prompt.cancel': '取消',
    'settings.nav.rsshub': 'RSSHub',
    'settings.section.rsshubInstance': 'RSSHub 实例',
    'settings.rsshub.mirror': '实例地址',
    'settings.rsshub.mirrorHint': 'rsshub.app 的订阅将改用此实例抓取；留空用官方',
    'settings.rsshub.save': '保存',
    'settings.rsshub.test': '测试连接',
    'settings.rsshub.saved': '已保存：{url}',
    'settings.rsshub.testing': '正在测试连接…',
    'settings.rsshub.migrate': '迁移现有订阅',
    'settings.rsshub.migrateHint': '把已存在的 rsshub.app / rsshub:// 订阅改写为当前实例地址',
    'settings.rsshub.migrateConfirm': '将改写 {n} 条订阅的地址，确定执行？',
    'settings.rsshub.migrateDone': '已迁移 {n} 条订阅',
    'settings.rsshub.migrateNone': '没有需要迁移的订阅',
    'reader.openInBrowser': '浏览器打开',
    'reader.copyLink': '复制链接',
    'reader.summarize': 'AI 摘要',
    'reader.translate': 'AI 翻译',
    'reader.summarizeTitle': '用配置的 AI 生成摘要（结果会缓存，不重复花钱）',
    'reader.translateTitle': '翻译成设置里的目标语言',
    'reader.linkCopied': '链接已复制',
    'reader.copyFailed': '复制失败：{error}',

    'ai.panel.summary': 'AI 摘要',
    'ai.panel.translate': 'AI 翻译',
    'ai.panel.requesting': '请求中…',
    'ai.panel.regenerate': '重新生成',
    'ai.panel.close': '关闭',
    'ai.panel.fromCache': '来自缓存（未重复请求）',
    'ai.panel.fresh': '本次新请求',
    'ai.panel.truncated': '正文超长已截断',
    'ai.panel.failed': '失败',

    'settings.title': '设置',
    'settings.close': '关闭',

    'settings.nav.general': '通用',
    'settings.nav.reading': '阅读',
    'settings.nav.data': '数据',
    'settings.nav.ai': 'AI',
    'settings.nav.mcp': 'MCP',
    'settings.nav.about': '关于',

    'settings.section.appearance': '外观',
    'settings.section.storage': '存储',
    'settings.section.readBehaviour': '已读行为',
    'settings.section.bulk': '批量操作',
    'settings.section.opml': '订阅导入导出',
    'settings.section.provider': '服务商与模型',
    'settings.section.credentials': '凭据',
    'settings.section.sending': '发送行为',
    'settings.ai.confirmBeforeSend': '发送前确认要发什么',
    'settings.ai.confirmBeforeSendHint': '发出请求前先展示目标地址与内容（凭据已打码）；命中缓存时不会询问',
    'settings.section.server': '服务',
    'settings.section.client': '客户端接入',

    'settings.languageHint': '默认跟随系统语言',
    'settings.dbPath': '数据库位置',
    'settings.storageHintTitle': '备份方式',
    'settings.bulkTitle': '对当前视图生效',
    'settings.bulkHint': '只影响当前选中的视图（全部 / 某个订阅源）',
    'settings.opmlTitle': 'OPML 导入 / 导出',
    'settings.opmlHint': '导入会按 xmlUrl 去重，并把嵌套文件夹压平成「父/子」',
    'settings.mcp.portHint': '需 ≥ 1024；改端口会重启服务',
    'settings.mcp.snippetTitle': '一键客户端配置',
    'settings.mcp.snippetHint': '包含 token，粘贴后请自行保管',
    'settings.about.tagline': '本地优先的 RSS 阅读器：订阅与已读状态在本机，AI 用你自己的 key',
    'settings.about.license': '许可证',
    'settings.about.privacy': '隐私',
    'settings.about.privacyHint': '除订阅源与你配置的 AI 端点外，不产生其他外呼；无账号、无遥测',

    'ai.confirm.title': '这一步会把以下内容发出去',
    'ai.confirm.summary': '模型 {model}｜请求体 {chars} 字符',
    'ai.confirm.target': '目标地址',
    'ai.confirm.headers': '请求头（凭据已打码）',
    'ai.confirm.body': '请求体',
    'ai.confirm.clipped': '（仅展示前 {shown} / {total} 字符）',
    'ai.confirm.truncated': '⚠ 正文过长，提示词已被截断：发出去的内容不完整',
    'ai.confirm.dontAsk': '以后不再询问',
    'ai.confirm.cancel': '取消',
    'ai.confirm.send': '发送',

    'status.aiCancelled': '已取消发送',
    'status.aiCachedNoSend': '命中缓存：本次不会发送请求',
    'settings.markRead.title': 'j / k 浏览时顺便标记已读',
    'settings.markRead.hint':
      '关闭后：j/k 只移动选中项并把文章显示在右栏，只有 Enter、鼠标点击或 u 才会改变已读状态。默认开启（键盘阅读器的主流做法：扫一遍即已读）。',
    'settings.markAllRead': '当前视图全部已读',
    'settings.markAllUnread': '当前视图全部未读（撤销）',
    'settings.importOpml': '导入 OPML…',
    'settings.exportOpml': '导出 OPML…',
    'settings.language': '界面语言',
    'settings.languageAuto': '跟随系统',
    'settings.languageZh': '简体中文',
    'settings.languageEn': 'English',
    'settings.theme': '主题',
    'settings.themeHint': '深浅色跟随；手动固定后重启保持',
    'settings.themeSystem': '跟随系统',
    'settings.themeLight': '浅色',
    'settings.themeDark': '深色',
    'settings.closeAction': '关闭按钮行为',
    'settings.closeActionHint': '退出程序，或最小化到系统托盘（托盘不可用时始终退出）',
    'settings.closeExit': '退出程序',
    'settings.closeTray': '最小化到托盘',
    'win.minimize': '最小化',
    'win.maximize': '最大化 / 还原',
    'win.close': '关闭',
    'settings.storageHint': '设置存在同一个数据库里，与订阅、已读状态一起备份。',

    'settings.ai.title': 'AI（自带 API key）',
    'settings.ai.provider': '服务商',
    'settings.ai.providerOllama': 'Ollama（本地，无需 key）',
    'settings.ai.providerOpenai': 'OpenAI 兼容（含自建网关）',
    'settings.ai.providerAnthropic': 'Anthropic',
    'settings.ai.providerGemini': 'Gemini',
    'settings.ai.model': '模型',
    'settings.ai.modelPlaceholder': '例如 gpt-4o-mini / claude-sonnet-4 / llama3.2',
    'settings.ai.baseUrl': '端点',
    'settings.ai.baseUrlPlaceholder': '留空用默认端点',
    'settings.ai.target': '翻译目标',
    'settings.ai.targetPlaceholder': '中文',
    'settings.ai.key': 'API key',
    'settings.ai.keyPlaceholder': '留空＝不修改；保存后写入系统凭据库',
    'settings.ai.keySet': '已设置（{source}）；留空＝不修改',
    'settings.ai.keyUnset': '尚未设置；留空＝不修改',
    'settings.ai.save': '保存 AI 设置',
    'settings.ai.clearKey': '清除 key',
    'settings.ai.test': '测试连接',
    'settings.ai.status': '默认端点：{base}｜key：{key}',
    'settings.ai.statusSet': '已设置',
    'settings.ai.statusUnset': '未设置',
    'settings.ai.savedWithKey': '已保存，key 已写入系统凭据库',
    'settings.ai.savedNoKey': '已保存，但还没有 key',
    'settings.ai.saveFailed': '保存失败：{error}',
    'settings.ai.keyCleared': '已清除该服务商的 key',
    'settings.ai.clearFailed': '清除失败：{error}',
    'settings.ai.testing': '测试中…',
    'settings.ai.testOk': '连接可用。模型回复：{reply}',
    'settings.ai.testFailed': '连接失败：{error}',

    'common.yes': '是',
    'common.no': '否',

    'settings.mcp.title': 'MCP（让 agent 读你的订阅）',
    'settings.mcp.enabled': '启用 MCP HTTP 服务',
    'settings.mcp.hint': '仅监听回环地址（127.0.0.1），且每个请求都要带 token；关掉即停止服务并释放端口。',
    'settings.mcp.port': '端口',
    'settings.mcp.copySnippet': '复制客户端配置',
    'settings.mcp.rotate': '轮换 token（旧 token 立即失效）',
    'settings.mcp.statusRunning': '运行中：{url}｜仅回环：{loopback}｜token：{token}',
    'settings.mcp.statusStopped': '未运行（未启用或启动失败）｜token：{token}',
    'settings.mcp.copied': '客户端配置已复制（含 token，粘贴后请自行保管）',
    'settings.mcp.copyFailed': '复制失败：{error}',
    'settings.mcp.rotated': 'token 已轮换，旧 token 立即失效',
    'settings.mcp.failed': '操作失败：{error}',

    'status.refreshing': '正在刷新…',
    'status.refreshDone': '刷新完成：成功 {fetched}｜未修改 {notModified}｜新增 {inserted}｜失败 {failures}',
    'status.refreshFailed': '刷新失败：{error}',
    'status.refreshingOne': '正在刷新该源…',
    'status.refreshOneDone': '该源刷新完成：新增 {inserted}｜未修改 {notModified}｜失败 {failures}',
    'status.adding': '已添加，正在抓取…',
    'status.added': '已添加：新增 {inserted} 篇',
    'status.addFailed': '添加失败：{error}',
    'status.discovering': '正在识别 feed 地址…',
    'status.discoverFailed': '自动发现失败：{error}',
    'status.exported': '已导出到 {path}',
    'status.exportCancelled': '已取消导出',
    'status.exportFailed': '导出失败：{error}',
    'status.imported':
      '导入完成：新增 {added}｜已存在跳过 {skipped}｜新建文件夹 {folders}｜忽略大纲 {ignored}',
    'status.importCancelled': '已取消导入',
    'status.importFailed': '导入失败：{error}',
    'status.markedRead': '已标为已读 {n} 篇',
    'status.markedUnread': '已标为未读 {n} 篇',
    'status.markFailed': '操作失败：{error}',
    'status.markReadOn': '已开启「j/k 浏览时标记已读」',
    'status.markReadOff': '已关闭「j/k 浏览时标记已读」',
    'status.settingFailed': '保存设置失败：{error}',
    'status.bootFailed': '初始化失败：{error}',
  },

  en: {
    'app.name': 'RustRss',
    'toolbar.refresh': 'Refresh all',
    'toolbar.addFeed': 'Add feed',
    'toolbar.settings': 'Settings',
    'toolbar.refreshTitle': 'Refresh all feeds (r)',
    'toolbar.addFeedTitle': 'Add a feed',
    'toolbar.settingsTitle': 'Settings',
    'search.placeholder': 'Search titles and text (press / to focus)',

    'sidebar.feeds': 'Feeds',
    'sidebar.feedCount': '{n}',
    'sidebar.addPlaceholder': 'https://example.com/ or a feed URL',
    'sidebar.add': 'Add',
    'sidebar.feedTooltipOk': '{url}\nDouble-click to refresh this feed',
    'sidebar.feedTooltipFailed': 'Last fetch: {status} | {error}\nDouble-click to retry',

    'list.unread': 'All unread',
    'list.starred': 'Starred',
    'list.all': 'All',
    'list.count': '{n} articles',
    'list.emptyUnread': 'Nothing unread',
    'list.empty': 'No articles here yet',
    'list.searchTitle': 'Search: {q}',
    'list.feedFallback': 'Feed',

    'reader.empty': 'Pick an article from the list.',
    'reader.shortcuts':
      'Shortcuts: j/k move · Enter open · u toggle unread · s star · l read later · r refresh · / search · Esc clear',
    'reader.markRead': 'Mark read',
    'reader.markUnread': 'Mark unread',
    'reader.addStar': 'Star',
    'reader.removeStar': 'Unstar',
    'reader.markLater': 'Read later',
    'reader.removeLater': 'Remove from read later',
    'list.later': 'Read later',
    'menu.moveTo': 'Move to',
    'menu.moveToUngrouped': 'Move to ungrouped',
    'menu.rename': 'Rename',
    'menu.delete': 'Delete',
    'folder.newTitle': 'New folder',
    'folder.renameTitle': 'Rename folder',
    'prompt.ok': 'OK',
    'prompt.cancel': 'Cancel',
    'settings.nav.rsshub': 'RSSHub',
    'settings.section.rsshubInstance': 'RSSHub instance',
    'settings.rsshub.mirror': 'Instance URL',
    'settings.rsshub.mirrorHint': 'rsshub.app feeds will be fetched from this instance; leave empty for the official one',
    'settings.rsshub.save': 'Save',
    'settings.rsshub.test': 'Test connection',
    'settings.rsshub.saved': 'Saved: {url}',
    'settings.rsshub.testing': 'Testing connection…',
    'settings.rsshub.migrate': 'Migrate existing feeds',
    'settings.rsshub.migrateHint': 'Rewrite existing rsshub.app / rsshub:// feeds to the current instance URL',
    'settings.rsshub.migrateConfirm': 'This will rewrite {n} feed URLs. Continue?',
    'settings.rsshub.migrateDone': 'Migrated {n} feeds',
    'settings.rsshub.migrateNone': 'No feeds need migration',
    'reader.openInBrowser': 'Open in browser',
    'reader.copyLink': 'Copy link',
    'reader.summarize': 'AI summary',
    'reader.translate': 'AI translate',
    'reader.summarizeTitle': 'Summarise with your configured AI (cached, so you are not billed twice)',
    'reader.translateTitle': 'Translate into the target language from settings',
    'reader.linkCopied': 'Link copied',
    'reader.copyFailed': 'Copy failed: {error}',

    'ai.panel.summary': 'AI summary',
    'ai.panel.translate': 'AI translation',
    'ai.panel.requesting': 'Requesting…',
    'ai.panel.regenerate': 'Regenerate',
    'ai.panel.close': 'Close',
    'ai.panel.fromCache': 'from cache (no new request)',
    'ai.panel.fresh': 'new request',
    'ai.panel.truncated': 'body was truncated',
    'ai.panel.failed': 'failed',

    'settings.title': 'Settings',
    'settings.close': 'Close',

    'settings.nav.general': 'General',
    'settings.nav.reading': 'Reading',
    'settings.nav.data': 'Data',
    'settings.nav.ai': 'AI',
    'settings.nav.mcp': 'MCP',
    'settings.nav.about': 'About',

    'settings.section.appearance': 'Appearance',
    'settings.section.storage': 'Storage',
    'settings.section.readBehaviour': 'Read behaviour',
    'settings.section.bulk': 'Bulk actions',
    'settings.section.opml': 'Import and export',
    'settings.section.provider': 'Provider and model',
    'settings.section.credentials': 'Credentials',
    'settings.section.sending': 'Sending',
    'settings.ai.confirmBeforeSend': 'Confirm what gets sent',
    'settings.ai.confirmBeforeSendHint': 'Shows the destination and payload before sending (credentials masked); skipped when the answer is already cached',
    'settings.section.server': 'Server',
    'settings.section.client': 'Client setup',

    'settings.languageHint': 'Follows the system language by default',
    'settings.dbPath': 'Database location',
    'settings.storageHintTitle': 'Backups',
    'settings.bulkTitle': 'Applies to the current view',
    'settings.bulkHint': 'Only affects the view you are looking at (all, or one feed)',
    'settings.opmlTitle': 'OPML import / export',
    'settings.opmlHint': 'Import dedupes by xmlUrl and flattens nested folders to parent/child',
    'settings.mcp.portHint': 'Must be 1024 or higher; changing it restarts the server',
    'settings.mcp.snippetTitle': 'One-click client config',
    'settings.mcp.snippetHint': 'It contains the token, so keep it somewhere safe',
    'settings.about.tagline': 'A local-first RSS reader: subscriptions and read state stay on this machine, AI uses your own key',
    'settings.about.license': 'License',
    'settings.about.privacy': 'Privacy',
    'settings.about.privacyHint': 'No traffic beyond your feeds and the AI endpoint you configure; no account, no telemetry',

    'ai.confirm.title': 'This step sends the following',
    'ai.confirm.summary': 'Model {model}｜request body {chars} characters',
    'ai.confirm.target': 'Destination',
    'ai.confirm.headers': 'Headers (credentials masked)',
    'ai.confirm.body': 'Request body',
    'ai.confirm.clipped': '(showing the first {shown} of {total} characters)',
    'ai.confirm.truncated': 'Warning: the article is too long, so the prompt was cut short. The outgoing content is incomplete.',
    'ai.confirm.dontAsk': 'Do not ask again',
    'ai.confirm.cancel': 'Cancel',
    'ai.confirm.send': 'Send',

    'status.aiCancelled': 'Send cancelled',
    'status.aiCachedNoSend': 'Cached answer: nothing will be sent this time',
    'settings.markRead.title': 'Mark as read while browsing with j / k',
    'settings.markRead.hint':
      'When off, j/k only move the selection and show the article; only Enter, a mouse click or u change the read state. On by default (the usual behaviour of keyboard readers: sweep and it is read).',
    'settings.markAllRead': 'Mark this view read',
    'settings.markAllUnread': 'Mark this view unread (undo)',
    'settings.importOpml': 'Import OPML…',
    'settings.exportOpml': 'Export OPML…',
    'settings.language': 'Language',
    'settings.languageAuto': 'Follow system',
    'settings.theme': 'Theme',
    'settings.themeHint': 'Follows the system; a fixed choice persists across restarts',
    'settings.themeSystem': 'Follow system',
    'settings.themeLight': 'Light',
    'settings.themeDark': 'Dark',
    'settings.closeAction': 'Close button behavior',
    'settings.closeActionHint': 'Exit the app, or minimize to the system tray (always exits when the tray is unavailable)',
    'settings.closeExit': 'Exit',
    'settings.closeTray': 'Minimize to tray',
    'win.minimize': 'Minimize',
    'win.maximize': 'Maximize / restore',
    'win.close': 'Close',
    'settings.languageZh': '简体中文',
    'settings.languageEn': 'English',
    'settings.storageHint': 'Settings live in the same database, so they are backed up with your feeds.',

    'settings.ai.title': 'AI (bring your own key)',
    'settings.ai.provider': 'Provider',
    'settings.ai.providerOllama': 'Ollama (local, no key)',
    'settings.ai.providerOpenai': 'OpenAI-compatible (incl. your own gateway)',
    'settings.ai.providerAnthropic': 'Anthropic',
    'settings.ai.providerGemini': 'Gemini',
    'settings.ai.model': 'Model',
    'settings.ai.modelPlaceholder': 'e.g. gpt-4o-mini / claude-sonnet-4 / llama3.2',
    'settings.ai.baseUrl': 'Endpoint',
    'settings.ai.baseUrlPlaceholder': 'leave empty for the default endpoint',
    'settings.ai.target': 'Translate to',
    'settings.ai.targetPlaceholder': 'Chinese',
    'settings.ai.key': 'API key',
    'settings.ai.keyPlaceholder': 'empty = keep current; stored in the OS keychain',
    'settings.ai.keySet': 'Set ({source}); leave empty to keep it',
    'settings.ai.keyUnset': 'Not set; leave empty to keep it',
    'settings.ai.save': 'Save AI settings',
    'settings.ai.clearKey': 'Clear key',
    'settings.ai.test': 'Test connection',
    'settings.ai.status': 'Default endpoint: {base} | key: {key}',
    'settings.ai.statusSet': 'set',
    'settings.ai.statusUnset': 'not set',
    'settings.ai.savedWithKey': 'Saved; the key is now in the OS keychain',
    'settings.ai.savedNoKey': 'Saved, but there is still no key',
    'settings.ai.saveFailed': 'Save failed: {error}',
    'settings.ai.keyCleared': 'Key cleared for this provider',
    'settings.ai.clearFailed': 'Clear failed: {error}',
    'settings.ai.testing': 'Testing…',
    'settings.ai.testOk': 'Connection works. Model replied: {reply}',
    'settings.ai.testFailed': 'Connection failed: {error}',

    'common.yes': 'yes',
    'common.no': 'no',

    'settings.mcp.title': 'MCP (let an agent read your feeds)',
    'settings.mcp.enabled': 'Enable the MCP HTTP server',
    'settings.mcp.hint':
      'Loopback only (127.0.0.1) and every request must carry the token; switching it off stops the server and frees the port.',
    'settings.mcp.port': 'Port',
    'settings.mcp.copySnippet': 'Copy client config',
    'settings.mcp.rotate': 'Rotate token (old one stops working)',
    'settings.mcp.statusRunning': 'Running: {url} | loopback only: {loopback} | token: {token}',
    'settings.mcp.statusStopped': 'Not running (disabled or failed to start) | token: {token}',
    'settings.mcp.copied': 'Client config copied (it contains the token - keep it safe)',
    'settings.mcp.copyFailed': 'Copy failed: {error}',
    'settings.mcp.rotated': 'Token rotated; the old one stopped working',
    'settings.mcp.failed': 'Action failed: {error}',

    'status.refreshing': 'Refreshing…',
    'status.refreshDone':
      'Refreshed: {fetched} fetched | {notModified} not modified | {inserted} new | {failures} failed',
    'status.refreshFailed': 'Refresh failed: {error}',
    'status.refreshingOne': 'Refreshing this feed…',
    'status.refreshOneDone':
      'Feed refreshed: {inserted} new | {notModified} not modified | {failures} failed',
    'status.adding': 'Added, fetching now…',
    'status.added': 'Added: {inserted} new articles',
    'status.addFailed': 'Could not add: {error}',
    'status.discovering': 'Looking for the feed…',
    'status.discoverFailed': 'Could not find a feed: {error}',
    'status.exported': 'Exported to {path}',
    'status.exportCancelled': 'Export cancelled',
    'status.exportFailed': 'Export failed: {error}',
    'status.imported':
      'Imported: {added} added | {skipped} already subscribed | {folders} folders created | {ignored} outlines ignored',
    'status.importCancelled': 'Import cancelled',
    'status.importFailed': 'Import failed: {error}',
    'status.markedRead': 'Marked {n} articles read',
    'status.markedUnread': 'Marked {n} articles unread',
    'status.markFailed': 'Action failed: {error}',
    'status.markReadOn': 'Enabled "mark read while browsing with j/k"',
    'status.markReadOff': 'Disabled "mark read while browsing with j/k"',
    'status.settingFailed': 'Could not save the setting: {error}',
    'status.bootFailed': 'Startup failed: {error}',
  },
};

let current = 'zh-CN';

function resolveLocale(preference) {
  if (preference === 'zh-CN' || preference === 'en') return preference;
  const lang = (navigator.language || 'en').toLowerCase();
  return lang.startsWith('zh') ? 'zh-CN' : 'en';
}

function setLocale(preference) {
  current = resolveLocale(preference);
  document.documentElement.lang = current;
  return current;
}

function locale() {
  return current;
}

/** 取文案；缺 key 时返回 key 本身（比显示 undefined 更容易定位） */
function t(key, params) {
  const dict = DICTS[current] || DICTS['zh-CN'];
  const raw = dict[key];
  if (raw === undefined) return key;
  if (!params) return raw;
  return raw.replace(/\{(\w+)\}/g, (_, name) =>
    params[name] === undefined ? `{${name}}` : String(params[name])
  );
}

/** 把 index.html 里的静态文案替换掉：data-i18n / data-i18n-placeholder / data-i18n-title */
function applyStaticI18n(root = document) {
  for (const node of root.querySelectorAll('[data-i18n]')) {
    node.textContent = t(node.dataset.i18n);
  }
  for (const node of root.querySelectorAll('[data-i18n-placeholder]')) {
    node.placeholder = t(node.dataset.i18nPlaceholder);
  }
  for (const node of root.querySelectorAll('[data-i18n-title]')) {
    node.title = t(node.dataset.i18nTitle);
  }
}

/**
 * 自检：两份字典的 key 集合必须完全一致，且 DOM 里每个 data-i18n 都能取到文案。
 * 结果上报到日志，因此「缺 key 数为 0」这条验收点可以机械核对。
 */
function selfTest() {
  const problems = [];
  const zh = Object.keys(DICTS['zh-CN']);
  const en = Object.keys(DICTS.en);
  const missingInEn = zh.filter((k) => !(k in DICTS.en));
  const missingInZh = en.filter((k) => !(k in DICTS['zh-CN']));
  if (missingInEn.length) problems.push(`en 缺 ${missingInEn.length} 个 key: ${missingInEn.join(',')}`);
  if (missingInZh.length) problems.push(`zh 缺 ${missingInZh.length} 个 key: ${missingInZh.join(',')}`);
  if (zh.length !== en.length) problems.push(`key 数不一致: zh=${zh.length} en=${en.length}`);

  const attrs = ['data-i18n', 'data-i18n-placeholder', 'data-i18n-title'];
  for (const attr of attrs) {
    for (const node of document.querySelectorAll(`[${attr}]`)) {
      const key = node.getAttribute(attr);
      if (!(key in DICTS['zh-CN'])) problems.push(`${attr} 引用了不存在的 key: ${key}`);
    }
  }

  return { ok: problems.length === 0, keys: zh.length, problems };
}

window.I18N = { DICTS, t, applyStaticI18n, selfTest, setLocale, resolveLocale, locale };

})();
