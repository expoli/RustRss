package tech.expoli.rustrss

import android.app.Activity
import android.content.Intent
import android.net.Uri
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

/** Reader external-link payload (passed through the open_external command). */
@InvokeArg
class OpenUrlArgs {
  lateinit var url: String
}

/**
 * Device-browser handoff for reader external links (ACTION_VIEW).
 *
 * The Rust side validates the URL before dispatching (http/https only, no
 * whitespace/control/metacharacters — validate_external_url). This side
 * re-checks the scheme so the plugin alone can never be talked into firing
 * another intent type, and reports back a rejection instead of crashing if
 * no activity can handle the URL.
 */
@TauriPlugin
class LinkPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun openUrl(invoke: Invoke) {
    val args = invoke.parseArgs(OpenUrlArgs::class.java)
    if (!args.url.startsWith("http://") && !args.url.startsWith("https://")) {
      invoke.reject("only http/https URLs are openable, got: ${args.url}")
      return
    }
    val view = Intent(Intent.ACTION_VIEW, Uri.parse(args.url))
      .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    try {
      activity.startActivity(view)
      invoke.resolve()
    } catch (e: Exception) {
      invoke.reject("no activity can open ${args.url}: ${e.message}")
    }
  }
}
