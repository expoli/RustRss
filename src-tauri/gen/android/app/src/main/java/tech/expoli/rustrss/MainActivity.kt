package tech.expoli.rustrss

import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  // TauriActivity disables Wry's default history navigation. The phone UI
  // pushes reader/settings entries, so both button and gesture Back must use it.
  override val handleBackNavigation: Boolean = true

  /// WebView 引用与最新 insets：注入 CSS 变量用（onWebViewCreate 后才可用）。
  private var webView: WebView? = null
  private var lastTop = -1
  private var lastBottom = -1

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
    // 首次注入要落在真实页面（tauri:// 加载完成后）才不被后续导航冲掉；
    // 页面是本地资产，加载毫秒级，三段重试覆盖时序竞态，脚本本身幂等。
    webView.postDelayed({ pushInsets(lastTop, lastBottom) }, 0)
    webView.postDelayed({ pushInsets(lastTop, lastBottom) }, 400)
    webView.postDelayed({ pushInsets(lastTop, lastBottom) }, 1200)
  }

  /// 把上下系统栏 inset 写进 CSS 变量。同值短路：insets 事件里重复触发零注入。
  private fun pushInsets(top: Int, bottom: Int) {
    if (top == lastTop && bottom == lastBottom) return
    lastTop = top
    lastBottom = bottom
    val web = webView ?: return
    val script =
      "document.documentElement.style.setProperty('--inset-top','${top}px');" +
        "document.documentElement.style.setProperty('--inset-bottom','${bottom}px');"
    web.post { web.evaluateJavascript(script, null) }
  }
}
