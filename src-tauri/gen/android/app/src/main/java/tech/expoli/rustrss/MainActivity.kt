package tech.expoli.rustrss

import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature

class MainActivity : TauriActivity() {
  // TauriActivity disables Wry's default history navigation. The phone UI
  // pushes reader/settings entries, so both button and gesture Back must use it.
  override val handleBackNavigation: Boolean = true

  /// WebView 引用与最新 insets：注入 CSS 变量用（onWebViewCreate 后才可用）。
  /// lastTop/lastBottom 恒存**原始物理像素**——密度换算只在 pushInsets 内部
  /// 做一次（评审 P0：此前把换算后的 dp 存回状态位，verify 重试把它当物理
  /// 像素再除一次密度，32→10→3→1→0 指数衰减，600ms 一档——正是真机
  /// 「启动正常→弹回重叠」的确定性回归）。
  private var webView: WebView? = null
  private var lastTop = 0
  private var lastBottom = 0
  private var documentStartRegistered = false

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // 模板默认 edge-to-edge（targetSdk 36+ 还会强制生效）：系统栏透明、应用内容
    // 画到状态栏/手势条底下。与桌面报头一致的手机视觉 = 「视觉延伸」：报头背景
    // 延伸到状态栏后面、底部导航面板延伸到手势条后面，而不是把整个 WebView
    // 缩在系统栏之间露出窗口底色（那正是用户报的「上下白边」）。
    // 因此这里只在视图层垫**横向刘海**与**键盘**；上下 inset 经 CSS 变量
    // (--inset-top / --inset-bottom) 交给网页，由报头/底部导航自己吃掉。
    // （WebView 对 env(safe-area-inset-*) 的支持不可靠，且它只映射刘海不映射
    // 系统栏——所以由原生注入。）
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      )
      val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
      // 横向：刘海在左右两侧的场景直接由容器避让（横屏窄边，无视觉边界问题）；
      // 底部：只垫键盘。手势条 inset 走 CSS（底部导航面板延伸到手势条后面，
      // 按钮行本身贴底——用户选择）。
      view.setPadding(bars.left, 0, bars.right, ime.bottom)
      pushInsets(bars.top, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
    ViewCompat.requestApplyInsets(content)
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    this.webView = webView
    // 注册快照用 dp（状态位是物理 px，__applyInsets 消费 dp——单位必须一致，
    // 评审指出的「脚本插入物理 px」回归）。脚本结构：
    //   1) 定义 __applyInsets（documentElement 每次重取；存储写失败不外抛）
    //   2) 无条件应用注册快照——存储禁用/损坏也不会让 insets 完全丢空
    //   3) 再尝试用 sessionStorage 里的最新值覆盖（页面中途重载后恢复的是
    //      最新值而不是注册快照；恢复失败只意味着退回快照）
    val d = resources.displayMetrics.density
    val topDp = (lastTop / d).toInt()
    val bottomDp = (lastBottom / d).toInt()
    if (WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
      // document-start：脚本在**每次页面导航**的文档起点自动执行——根除
      // 「注入早于文档就绪、随后被导航冲掉」的时序竞态（真机冷启动 >1s 时
      // 延迟注入全部落空，状态栏时间与报头重叠）。
      val script = "window.__applyInsets=function(t,b){" +
        "var d=document.documentElement;if(!d)return;" +
        "d.style.setProperty('--inset-top',t+'px');" +
        "d.style.setProperty('--inset-bottom',b+'px');" +
        "try{sessionStorage.setItem('__insets',t+','+b);}catch(e){}};" +
        "window.__applyInsets($topDp,$bottomDp);" +
        "try{var s=sessionStorage.getItem('__insets');" +
        "if(s){var p=s.split(',');window.__applyInsets(Number(p[0]),Number(p[1]));}}catch(e){}"
      WebViewCompat.addDocumentStartJavaScript(webView, script, setOf("*"))
      documentStartRegistered = true
    } else {
      // 老 WebView 无 document-start 支持：退回延迟注入（尽力而为）
      webView.postDelayed({ pushInsets(lastTop, lastBottom, force = true) }, 0)
      webView.postDelayed({ pushInsets(lastTop, lastBottom, force = true) }, 800)
      webView.postDelayed({ pushInsets(lastTop, lastBottom, force = true) }, 2000)
    }
    // 慢设备/老 WebView 兜底：确认 CSS 变量真的算进去了，没生效就重试。
    // 两个分支共用（评审：fallback 分支此前无人重试）。上限 10 次 × 600ms。
    verifyApplied(0)
  }

  /// 读回计算样式验证 --inset-top 已生效；未生效则重注入并重试（原生侧闭环）。
  /// 期望值与注入值走同一换算（原始 px → dp），判据不因单位漂移而误判。
  private fun verifyApplied(attempt: Int) {
    val web = webView ?: return
    val expected = "${(lastTop / resources.displayMetrics.density).toInt()}px"
    web.post {
      web.evaluateJavascript(
        "getComputedStyle(document.documentElement).getPropertyValue('--inset-top').trim()",
      ) { result ->
        val applied = result?.trim()?.trim('"') == expected
        if (!applied && attempt < 10) {
          web.postDelayed({
            pushInsets(lastTop, lastBottom, force = true) // 原始值重传：换算幂等
            verifyApplied(attempt + 1)
          }, 600)
        }
      }
    }
  }

  /// 把上下系统栏 inset 写进 CSS 变量。同值短路：insets 事件里重复触发零注入。
  /// inset 是物理像素，CSS px 是 dp——换算只在函数内做一次，结果不回写状态位
  /// （重试传原始值时换算幂等，32 恒为 32dp，不再衰减）。
  private fun pushInsets(topPx: Int, bottomPx: Int, force: Boolean = false) {
    if (!force && topPx == lastTop && bottomPx == lastBottom) return
    lastTop = topPx
    lastBottom = bottomPx
    val web = webView ?: return
    val d = resources.displayMetrics.density
    val topDp = (topPx / d).toInt()
    val bottomDp = (bottomPx / d).toInt()
    val script = if (documentStartRegistered) {
      "window.__applyInsets&&window.__applyInsets($topDp,$bottomDp);"
    } else {
      "document.documentElement.style.setProperty('--inset-top','${topDp}px');" +
        "document.documentElement.style.setProperty('--inset-bottom','${bottomDp}px');"
    }
    web.post { web.evaluateJavascript(script, null) }
  }
}
