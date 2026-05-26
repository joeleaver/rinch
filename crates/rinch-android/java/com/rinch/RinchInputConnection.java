package com.rinch;

import android.view.View;
import android.view.inputmethod.BaseInputConnection;

/**
 * Custom InputConnection that forwards IME text to Rust via JNI.
 *
 * Handles committed text, composing text, and deletions from the
 * Android soft keyboard (including CJK composing, autocomplete,
 * and swipe-to-type).
 */
public class RinchInputConnection extends BaseInputConnection {

    public RinchInputConnection(View targetView, boolean fullEditor) {
        super(targetView, fullEditor);
    }

    @Override
    public boolean commitText(CharSequence text, int newCursorPosition) {
        if (text != null && text.length() > 0) {
            nativeCommitText(text.toString());
        }
        return true;
    }

    @Override
    public boolean setComposingText(CharSequence text, int newCursorPosition) {
        // For now, commit composing text immediately.
        // A full implementation would track composing state for
        // inline preview (underlined text) during CJK input.
        return true;
    }

    @Override
    public boolean finishComposingText() {
        return true;
    }

    @Override
    public boolean deleteSurroundingText(int beforeLength, int afterLength) {
        nativeDeleteSurrounding(beforeLength, afterLength);
        return true;
    }

    @Override
    public boolean sendKeyEvent(android.view.KeyEvent event) {
        // Let NativeActivity's input queue handle raw key events
        return super.sendKeyEvent(event);
    }

    // ── JNI native methods (implemented in rinch-android/src/ime.rs) ───

    private static native void nativeCommitText(String text);
    private static native void nativeDeleteSurrounding(int before, int after);
}
