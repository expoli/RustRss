package tech.expoli.rustrss

import android.app.Activity
import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** 读写载荷：service + account 定位条目，password 仅 set 携带。 */
@InvokeArg
class SecureArgs {
  lateinit var service: String
  lateinit var account: String
  var password: String? = null
}

/** get 的响应：普通类（Jackson 按属性序列化 → {"password": …}）。 */
class SecureOut {
  var password: String = ""
}

/**
 * AI 凭据的安全存储：secret 用 Android Keystore 的 AES/GCM 密钥加密，
 * 密文（Base64(iv‖ct)）存进应用私有的 SharedPreferences。
 *
 * 明文只存在于调用参数与解密瞬间：不进 SQLite、不进偏好文件的明文、不进日志
 * （本类没有任何 Log 调用）。Keystore 密钥不出安全硬件（TEE/StrongBox），
 * 卸载应用即随数据一起消失。
 */
@TauriPlugin
class SecureStorePlugin(private val activity: Activity) : Plugin(activity) {
  private val prefs by lazy {
    activity.getSharedPreferences("secure_store", Context.MODE_PRIVATE)
  }

  private fun slot(service: String, account: String) = "$service::$account"

  private fun alias(service: String) =
    "rustrss_" + service.filter { it.isLetterOrDigit() || it == '_' || it == '-' }.lowercase()

  private fun secretKey(alias: String): SecretKey {
    val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    (ks.getEntry(alias, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
    val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
    generator.init(
      KeyGenParameterSpec.Builder(
        alias,
        KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
      )
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(256)
        .build()
    )
    return generator.generateKey()
  }

  private fun encrypt(alias: String, plain: ByteArray): ByteArray {
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.ENCRYPT_MODE, secretKey(alias))
    // GCM 推荐 96 位 IV：init 不传参时由 cipher 生成，cipher.iv 即可取回
    return cipher.iv + cipher.doFinal(plain)
  }

  private fun decrypt(alias: String, blob: ByteArray): ByteArray {
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.DECRYPT_MODE, secretKey(alias), GCMParameterSpec(128, blob, 0, 12))
    return cipher.doFinal(blob, 12, blob.size - 12)
  }

  @Command
  fun set(invoke: Invoke) {
    val args = invoke.parseArgs(SecureArgs::class.java)
    val password =
      args.password ?: return invoke.reject("password is required")
    val alias = alias(args.service)
    val blob =
      Base64.encodeToString(
        encrypt(alias, password.toByteArray(Charsets.UTF_8)),
        Base64.NO_WRAP,
      )
    prefs.edit().putString(slot(args.service, args.account), blob).apply()
    invoke.resolve()
  }

  @Command
  fun get(invoke: Invoke) {
    val args = invoke.parseArgs(SecureArgs::class.java)
    val out = SecureOut()
    val stored = prefs.getString(slot(args.service, args.account), null)
    // 空串 = 条目不存在（Rust 侧与桌面 NoEntry 同语义）
    if (stored != null) {
      out.password = String(decrypt(alias(args.service), Base64.decode(stored, Base64.NO_WRAP)), Charsets.UTF_8)
    }
    // 注意：resolveObject 的参数必须是普通 Kotlin 类——传 JSObject 会被 Jackson
    // 按 org.json 内部字段序列化成 {"nameValuePairs": …}（实测踩过）。
    invoke.resolveObject(out)
  }

  @Command
  fun delete(invoke: Invoke) {
    val args = invoke.parseArgs(SecureArgs::class.java)
    prefs.edit().remove(slot(args.service, args.account)).apply()
    invoke.resolve()
  }
}
