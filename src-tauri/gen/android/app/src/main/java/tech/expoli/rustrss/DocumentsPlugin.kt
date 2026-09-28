package tech.expoli.rustrss

import android.app.Activity
import android.net.Uri
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

/** 文档读写载荷：uri 来自系统文档选择器（SAF content://），content 仅 writeText 携带。 */
@InvokeArg
class TextDocArgs {
  lateinit var uri: String
  var content: String? = null
}

/** get 的响应：普通类（Jackson 按属性序列化 → {"text": …}）。 */
class TextDocOut {
  var text: String = ""
}

/**
 * OPML 导入/导出的文档读写：系统文档选择器返回 content:// URI，
 * `std::fs` 读不了，这里经 ContentResolver 流转发。
 */
@TauriPlugin
class DocumentsPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun readText(invoke: Invoke) {
    val args = invoke.parseArgs(TextDocArgs::class.java)
    val out = TextDocOut()
    activity.contentResolver
      .openInputStream(Uri.parse(args.uri))
      ?.use { stream -> out.text = stream.bufferedReader().readText() }
    invoke.resolveObject(out)
  }

  @Command
  fun writeText(invoke: Invoke) {
    val args = invoke.parseArgs(TextDocArgs::class.java)
    val stream =
      activity.contentResolver.openOutputStream(Uri.parse(args.uri))
        ?: return invoke.reject("无法打开输出文档")
    stream.use { it.write((args.content ?: "").toByteArray(Charsets.UTF_8)) }
    invoke.resolve()
  }
}
