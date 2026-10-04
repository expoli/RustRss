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
  private var webView: WebView? = null
  private var lastTop = 0
  private var lastBottom = 0
  private var documentStartRegistered = false

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // 模板默认 edge-to-edge（targetSdk 36+ 还会强制生效）：系统栏透明、应用内容
    // 画到状态栏/手势条底下。与桌面报头一致的手机视觉 = 「视觉延伸」：报头背景
    // 与 2px 墨线延伸到状态栏后面、底部导航面板延伸到手势条后面，而不是把整个
    // WebView 缩在系统栏之间露出窗口底色（那正是用户报的「上下白边」）。
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
      // 底部：只垫键盘。手势条 inset 走 CSS（底部导航面板延伸到手势条后面）。
      view.setPadding(bars.left, 0, bars.right, ime.bottom)
      pushInsets(bars.top, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
    ViewCompat.requestApplyInsets(content)
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    this.webView = webView
    if (WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
      // document-start：脚本在**每次页面导航**的文档起点自动执行——根除
      // 「注入早于文档就绪、随后被导航冲掉」的时序竞态（真机冷启动 >1s 时
      // 延迟注入全部落空，状态栏时间与报头重叠）。函数定义 + 立即应用当前值。
      val script = "window.__applyInsets=function(t,b){var d=document.documentElement;" +
        "d.style.setProperty('--inset-top',t+'px');" +
        "d.style.setProperty('--inset-bottom',b+'px');};" +
        "window.__applyInsets($lastTop,$lastBottom);"
      WebViewCompat.addDocumentStartJavaScript(webView, script, setOf("*"))
      documentStartRegistered = true
      // 慢设备/老 WebView 兜底：确认 CSS 变量真的算进去了，没生效就重试。
      // （document-start 覆盖绝大多数场景；此循环收敛其余：fallback 注入晚于
      //   页面加载完成、WebView 版本差异等。上限 10 次 × 600ms 后放弃。）
      verifyApplied(0)
    }
  }

  /// 读回计算样式验证 --inset-top 已生效；未生效则重注入并重试（原生侧闭环）。
  private fun verifyApplied(attempt: Int) {
    val web = webView ?: return
    web.post {
      web.evaluateJavascript(
        "getComputedStyle(document.documentElement).getPropertyValue('--inset-top').trim()",
      ) { result ->
        val applied = result?.trim()?.trim('"') == "${lastTop}px"
        if (!applied && attempt < 10) {
          web.postDelayed({
            pushInsets(lastTop, lastBottom, force = true)
            verifyApplied(attempt + 1)
          }, 600)
        }
      }
    }
  }

  /// 把上下系统栏 inset 写进 CSS 变量。同值短路：insets 事件里重复触发零注入。
  /// inset 是物理像素，CSS px 是 dp——必须除以 density（评审指出的单位错误，
  /// 否则修好覆盖后会 ~2.6 倍过度留白）。
  private fun pushInsets(topPx: Int, bottomPx: Int, force: Boolean = false) {
    val d = resources.displayMetrics.density
    val top = (topPx / d).toInt()
    val bottom = (bottomPx / d).toInt()
    if (!force && top == lastTop && bottom == lastBottom) return
    lastTop = top
    lastBottom = bottom
    val web = webView ?: return
    val script = if (documentStartRegistered) {
      "window.__applyInsets&&window.__applyInsets($top,$bottom);"
    } else {
      "document.documentElement.style.setProperty('--inset-top','${top}px');" +
        "document.documentElement.style.setProperty('--inset-bottom','${bottom}px');"
    }
    web.post { web.evaluateJavascript(script, null) }
  }
}
