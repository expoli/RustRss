# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile
# RustRss: Tauri mobile plugin 的参数/返回类经 Jackson 反射序列化
# （SecureStorePlugin/DocumentsPlugin 的 get/set），R8 不得重命名或去 getter。
-keep class tech.expoli.rustrss.SecureArgs { *; }
-keep class tech.expoli.rustrss.SecureOut { *; }
-keep class tech.expoli.rustrss.TextDocArgs { *; }
-keep class tech.expoli.rustrss.TextDocOut { *; }
