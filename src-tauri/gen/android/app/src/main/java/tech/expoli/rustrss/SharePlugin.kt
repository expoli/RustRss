package tech.expoli.rustrss

import android.app.Activity
import android.content.Intent
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

/** Reader "Share" payload (passed through the share_url command). */
@InvokeArg
class ShareArgs {
  lateinit var title: String
  lateinit var url: String
}

/** System share panel (ACTION_SEND chooser): the landing point for the mobile side of the reader's Share button. */
@TauriPlugin
class SharePlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun shareUrl(invoke: Invoke) {
    val args = invoke.parseArgs(ShareArgs::class.java)
    val send = Intent(Intent.ACTION_SEND).apply {
      type = "text/plain"
      putExtra(Intent.EXTRA_TITLE, args.title)
      putExtra(Intent.EXTRA_TEXT, args.url)
    }
    activity.startActivity(Intent.createChooser(send, null))
    invoke.resolve()
  }
}
