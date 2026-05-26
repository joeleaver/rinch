package com.rinch;

import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.os.Bundle;
import android.os.Vibrator;
import android.view.ViewGroup;
import android.view.inputmethod.InputMethodManager;

/**
 * Rinch Activity — extends NativeActivity with platform service methods
 * callable from Rust via JNI (clipboard, IME, haptics, etc.).
 *
 * The NativeActivity base class handles the native library loading,
 * surface management, and lifecycle forwarding to the android-activity crate.
 */
public class RinchActivity extends NativeActivity {

    private InputMethodManager imm;
    private ClipboardManager clipboardManager;
    private Vibrator vibrator;
    private RinchInputView inputView;



    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
        clipboardManager = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
        vibrator = (Vibrator) getSystemService(Context.VIBRATOR_SERVICE);

        // Add input proxy for proper InputConnection (CJK, autocomplete, etc.)
        // Must have non-zero size and be visible for IME to connect.
        inputView = new RinchInputView(this);
        ViewGroup.LayoutParams lp = new ViewGroup.LayoutParams(1, 1);
        addContentView(inputView, lp);
    }

    // ── IME ─────────────────────────────────────────────────────────────

    public void showKeyboard() {
        runOnUiThread(() -> {
            if (imm != null && inputView != null) {
                inputView.setVisibility(android.view.View.VISIBLE);
                inputView.requestFocus();
                imm.restartInput(inputView);
                imm.showSoftInput(inputView, InputMethodManager.SHOW_FORCED);
            }
        });
    }

    public void hideKeyboard() {
        runOnUiThread(() -> {
            if (imm != null && inputView != null) {
                imm.hideSoftInputFromWindow(inputView.getWindowToken(), 0);
            }
        });
    }

    // ── Clipboard ───────────────────────────────────────────────────────

    public void copyToClipboard(String text) {
        if (clipboardManager != null) {
            ClipData clip = ClipData.newPlainText("rinch", text);
            clipboardManager.setPrimaryClip(clip);
        }
    }

    public String pasteFromClipboard() {
        if (clipboardManager == null || !clipboardManager.hasPrimaryClip()) {
            return null;
        }
        ClipData clip = clipboardManager.getPrimaryClip();
        if (clip == null || clip.getItemCount() == 0) {
            return null;
        }
        CharSequence text = clip.getItemAt(0).getText();
        return text != null ? text.toString() : null;
    }

    public boolean hasClipboardText() {
        return clipboardManager != null && clipboardManager.hasPrimaryClip();
    }

    // ── Haptics ─────────────────────────────────────────────────────────

    @SuppressWarnings("deprecation")
    public void vibrate(long ms) {
        if (vibrator != null && vibrator.hasVibrator()) {
            vibrator.vibrate(ms);
        }
    }
}
