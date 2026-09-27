package tech.expoli.rustrss

import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // 模板默认 edge-to-edge（targetSdk 36+ 还会强制生效）：系统栏透明、应用内容
    // 画到状态栏/手势条底下。把系统栏与刘海的 insets 落成根容器的 padding，
    // 让 WebView 内容永远在安全区内（tech_design：支持安全区/status bar insets）。
    // 只在视图层垫一次，WebView 内部不必各自适配 env()（WebView 对 safe-area
    // 的支持并不可靠）。
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      )
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }
}
