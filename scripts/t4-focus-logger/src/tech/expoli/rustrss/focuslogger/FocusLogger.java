package tech.expoli.rustrss.focuslogger;
import android.accessibilityservice.AccessibilityService;
import android.util.Log;
import android.view.accessibility.AccessibilityEvent;
import android.view.accessibility.AccessibilityNodeInfo;
public class FocusLogger extends AccessibilityService {
  @Override public void onAccessibilityEvent(AccessibilityEvent event) {
    AccessibilityNodeInfo n = event.getSource();
    String id = n == null ? "null" : String.valueOf(n.getViewIdResourceName());
    String text = n == null ? "null" : String.valueOf(n.getText());
    String desc = n == null ? "null" : String.valueOf(n.getContentDescription());
    String focus = n == null ? "null" : String.valueOf(n.isAccessibilityFocused());
    Log.i("T4A11Y", "type="+AccessibilityEvent.eventTypeToString(event.getEventType())+" package="+event.getPackageName()+" id="+id+" text="+text+" desc="+desc+" a11yFocus="+focus);
  }
  @Override public void onInterrupt() { Log.i("T4A11Y", "INTERRUPT"); }
  @Override protected void onServiceConnected() { Log.i("T4A11Y", "CONNECTED"); }
}
