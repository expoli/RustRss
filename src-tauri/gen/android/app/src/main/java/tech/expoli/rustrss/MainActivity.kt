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
      // 值经 sessionStorage 跨导航持久：页面中途重载（真机实测会发生——用户
      // 看到「启动正常 → 闪一下 → 向上弹回重叠」正是重载把已修正的变量打回
      // 注册时的快照值）时，恢复的是**最新值**而不是注册时的快照。
      val script = "try{" +
        "var d=document.documentElement;" +
        "window.__applyInsets=function(t,b){d.style.setProperty('--inset-top',t+'px');" +
        "d.style.setProperty('--inset-bottom',b+'px');" +
        "try{sessionStorage.setItem('__insets',t+','+b);}catch(e){}};" +
        "var s=sessionStorage.getItem('__insets');" +
        "if(s){var p=s.split(',');window.__applyInsets(p[0],p[1]);}" +
        "else{window.__applyInsets($lastTop,$lastBottom);}" +
        "}catch(e){}"
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
  /// inset 是物理像素，CSS px 是 dp——必须除以 density（评审指出的单位错误，
  /// 否则修好覆盖后会 ~2.6 倍过度留白）。
  private fun pushInsets(topPx: Int, bottomPx: Int, force: Boolean = false) {
    if (!force && topPx == lastTop && bottomPx == lastBottom) return
    lastTop = topPx
    lastBottom = bottomPx
    val web = webView ?: return
    val d = resources.displayMetrics.density
    // 唯一的换算点：入参恒为物理 px，换算结果不回写状态位——重试传原始值
    // 时换算幂等（32 → 32dp 恒定，不再衰减）。
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
